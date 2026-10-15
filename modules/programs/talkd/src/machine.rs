//! The push-to-talk cycle: one state machine, fed every event in order, that
//! acts only through [`World`]. No I/O here.
//!
//! A press inverts the default source's mute and owes a restore; the
//! obligation (server, node, previous mute) is saved before the mic is
//! touched and cleared only once the restore is seen to have happened. A
//! release 250 ms or more after its press restores, a quicker one latches,
//! and the press ending a latch restores, unless the mute was put back
//! outside talkd meanwhile, in which case that press starts a fresh cycle.
//! While the session is locked (or its lock state is unknown) a press may
//! only end a cycle, never start one.

use std::time::Duration;

/// A release this soon after its press latches.
pub const LATCH: Duration = Duration::from_millis(250);

/// How long a mute change has to show up in the node's params.
pub const CONFIRM: Duration = Duration::from_secs(2);

/// A node of one PipeWire server's lifetime: the server's core cookie (a
/// random u32 drawn at its start) and the node's object.serial, which that
/// server never reuses but a restarted one counts again from zero.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pin {
    pub server: u32,
    pub serial: u64,
}

/// What a cycle owes: putting `pin`'s mute back to `was_muted`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Obligation {
    pub pin: Pin,
    pub was_muted: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lock {
    Unlocked,
    Locked,
    /// No word from the compositor: treated as locked.
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// F13 down, at this CLOCK_BOOTTIME time.
    Press(Duration),
    Release(Duration),
    /// The keyboard went away; a hold ends as if released, never latched.
    DeviceLost,
    Lock(Lock),
    /// Connected and synced to the server with this cookie.
    ServerUp(u32),
    ServerGone,
    /// A node's mute, as the mixer reads it, changed to this.
    Observed(Pin, bool),
    NodeGone(Pin),
    /// A confirmation deadline set through [`World::arm`] passed.
    Deadline(u64),
    /// Shutting down: settle what is owed.
    Stop,
}

/// What went wrong. [`Problem::visible`] ones are worth a notification.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Problem {
    RestoreFailed(String),
    RestoreUnconfirmed,
    NodeGone,
    ServerGone,
    /// A saved obligation that cannot be honoured at start.
    Stale(&'static str),
    EngageFailed(String),
    EngageUnconfirmed,
    NoDefaultSource,
    Store(String),
    Locked(Lock),
    NotReady,
}

impl Problem {
    /// The ones that can leave the mic other than the user left it.
    pub fn visible(&self) -> bool {
        matches!(
            self,
            Problem::RestoreFailed(_)
                | Problem::RestoreUnconfirmed
                | Problem::NodeGone
                | Problem::ServerGone
                | Problem::Stale(_)
        )
    }

    pub fn message(&self) -> String {
        match self {
            Problem::RestoreFailed(why) => format!("could not restore the mic's mute: {why}"),
            Problem::RestoreUnconfirmed => {
                "the mic's mute restore was not confirmed; check the mic".to_owned()
            }
            Problem::NodeGone => {
                "the mic talkd flipped is gone; its mute was not restored".to_owned()
            }
            Problem::ServerGone => {
                "PipeWire went away mid-cycle; the mic's mute was not restored".to_owned()
            }
            Problem::Stale(why) => format!("dropped the restore saved by a previous run: {why}"),
            Problem::EngageFailed(why) => format!("could not flip the mic's mute: {why}"),
            Problem::EngageUnconfirmed => "the mic's mute flip was not confirmed".to_owned(),
            Problem::NoDefaultSource => "no default source with a mute to flip".to_owned(),
            Problem::Store(why) => format!("restore record: {why}"),
            Problem::Locked(Lock::Unknown) => {
                "press ignored: the session's lock state is unknown".to_owned()
            }
            Problem::Locked(_) => "press ignored: the session is locked".to_owned(),
            Problem::NotReady => "press ignored: not connected to PipeWire yet".to_owned(),
        }
    }
}

/// Everything the machine does to the outside.
pub trait World {
    /// The default source and its mute.
    fn default_source(&self) -> Option<(Pin, bool)>;
    /// A node's mute; None once it is gone (or no longer known).
    fn muted(&self, pin: Pin) -> Option<bool>;
    /// Sends a mute change. Err only if it certainly was not sent.
    fn set_mute(&mut self, pin: Pin, muted: bool) -> Result<(), String>;
    fn save(&mut self, obligation: &Obligation) -> Result<(), String>;
    fn clear(&mut self) -> Result<(), String>;
    /// Delivers [`Event::Deadline`]`(token)` after `after`.
    fn arm(&mut self, token: u64, after: Duration);
    fn problem(&mut self, problem: Problem);
    /// The mic is back where the user left it.
    fn recovered(&mut self);
}

/// A mute change sent and not yet seen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Pending {
    token: u64,
    pin: Pin,
    target: bool,
    restore: bool,
}

#[derive(Debug)]
pub struct Machine {
    obligation: Option<Obligation>,
    /// When the press that engaged went down, while it is still down.
    hold: Option<Duration>,
    pending: Option<Pending>,
    /// A restore was sent: seeing `was_muted` completes the cycle, even
    /// after its deadline.
    restoring: bool,
    /// The engage's change has shown, or can no longer be in flight. Until
    /// then a mute still at `was_muted` proves nothing: the flip may yet land.
    engage_seen: bool,
    server: Option<u32>,
    lock: Lock,
    /// Saved by a previous run, settled at the first [`Event::ServerUp`].
    recovering: Option<Obligation>,
    tokens: u64,
    stopping: bool,
}

impl Machine {
    pub fn new(saved: Option<Obligation>) -> Self {
        Machine {
            obligation: None,
            hold: None,
            pending: None,
            restoring: false,
            engage_seen: true,
            server: None,
            lock: Lock::Unknown,
            recovering: saved,
            tokens: 0,
            stopping: false,
        }
    }

    /// Stopping, and nothing more to wait for: no restore owed, or its
    /// outcome known (confirmed, failed, or unconfirmed past its deadline).
    pub fn settled(&self) -> bool {
        self.stopping
            && (self.obligation.is_none() || self.server.is_none() || self.pending.is_none())
    }

    pub fn handle(&mut self, event: Event, world: &mut impl World) {
        match event {
            Event::Press(at) => self.press(at, world),
            Event::Release(at) => self.release(Some(at), world),
            Event::DeviceLost => self.release(None, world),
            Event::Lock(lock) => self.lock = lock,
            Event::ServerUp(server) => {
                self.server = Some(server);
                if let Some(saved) = self.recovering.take() {
                    self.recover(saved, server, world);
                }
            }
            Event::ServerGone => {
                self.server = None;
                if self.obligation.is_some() {
                    self.void(Problem::ServerGone, world);
                }
            }
            Event::Observed(pin, muted) => self.observed(pin, muted, world),
            Event::NodeGone(pin) => {
                if self.obligation.is_some_and(|owed| owed.pin == pin) {
                    self.void(Problem::NodeGone, world);
                }
            }
            Event::Deadline(token) => {
                if let Some(pending) = self.pending.filter(|pending| pending.token == token) {
                    self.pending = None;
                    if pending.restore {
                        // sent after the engage, so that one is done too
                        self.engage_seen = true;
                        world.problem(Problem::RestoreUnconfirmed);
                    } else {
                        world.problem(Problem::EngageUnconfirmed);
                    }
                }
            }
            Event::Stop => {
                self.stopping = true;
                self.hold = None;
                let restoring = self.pending.is_some_and(|pending| pending.restore);
                if self.obligation.is_some() && self.server.is_some() && !restoring {
                    self.restore(world);
                }
            }
        }
    }

    fn press(&mut self, at: Duration, world: &mut impl World) {
        if self.stopping {
            return;
        }
        if self.server.is_none() || self.recovering.is_some() {
            world.problem(Problem::NotReady);
            return;
        }
        // a press while another is held (its release was lost) settles it too
        self.hold = None;
        let Some(owed) = self.obligation else {
            return self.engage(at, world);
        };
        match world.muted(owed.pin) {
            None => self.void(Problem::NodeGone, world),
            // put back outside talkd: this press starts afresh
            Some(muted) if muted == owed.was_muted && self.engage_seen => {
                self.finish(world);
                self.engage(at, world);
            }
            Some(_) => self.restore(world),
        }
    }

    /// The engaging press's release (`at`), or the keyboard lost (None).
    fn release(&mut self, at: Option<Duration>, world: &mut impl World) {
        let Some(since) = self.hold.take() else {
            return; // not the engaging press's: ignored
        };
        let tap = at.is_some_and(|at| at.checked_sub(since).is_none_or(|held| held < LATCH));
        if !tap {
            self.restore(world);
        }
    }

    fn engage(&mut self, at: Duration, world: &mut impl World) {
        if self.lock != Lock::Unlocked {
            return world.problem(Problem::Locked(self.lock));
        }
        let Some((pin, muted)) = world.default_source() else {
            return world.problem(Problem::NoDefaultSource);
        };
        let owed = Obligation {
            pin,
            was_muted: muted,
        };
        if let Err(why) = world.save(&owed) {
            // never touch the mic without a record of how to put it back
            return world.problem(Problem::Store(why));
        }
        self.obligation = Some(owed);
        self.restoring = false;
        self.engage_seen = false;
        match world.set_mute(pin, !muted) {
            Ok(()) => {
                self.hold = Some(at);
                self.expect(pin, !muted, false, world);
            }
            Err(why) => {
                // certainly not sent, so nothing is owed
                self.end();
                if let Err(why) = world.clear() {
                    world.problem(Problem::Store(why));
                }
                world.problem(Problem::EngageFailed(why));
            }
        }
    }

    fn restore(&mut self, world: &mut impl World) {
        let Some(owed) = self.obligation else {
            return;
        };
        self.hold = None;
        match world.muted(owed.pin) {
            None => self.void(Problem::NodeGone, world),
            Some(muted) if muted == owed.was_muted && self.engage_seen => self.finish(world),
            Some(_) => {
                self.restoring = true;
                match world.set_mute(owed.pin, owed.was_muted) {
                    Ok(()) => self.expect(owed.pin, owed.was_muted, true, world),
                    // still owed: the next press, or a late confirmation, ends it
                    Err(why) => world.problem(Problem::RestoreFailed(why)),
                }
            }
        }
    }

    fn observed(&mut self, pin: Pin, muted: bool, world: &mut impl World) {
        if self
            .obligation
            .is_some_and(|owed| owed.pin == pin && owed.was_muted != muted)
        {
            self.engage_seen = true;
        }
        if self
            .pending
            .is_some_and(|pending| pending.pin == pin && pending.target == muted)
        {
            self.pending = None;
        }
        if self.restoring
            && self
                .obligation
                .is_some_and(|owed| owed.pin == pin && owed.was_muted == muted)
        {
            self.finish(world);
        }
    }

    fn recover(&mut self, saved: Obligation, server: u32, world: &mut impl World) {
        // its run's connection is closed: nothing of it is still in flight
        self.engage_seen = true;
        if saved.pin.server != server {
            return self.void(Problem::Stale("PipeWire restarted since"), world);
        }
        match world.muted(saved.pin) {
            None => self.void(Problem::Stale("its mic is gone"), world),
            Some(muted) if muted == saved.was_muted => {
                self.obligation = Some(saved);
                self.finish(world);
            }
            Some(_) => {
                self.obligation = Some(saved);
                self.restore(world);
            }
        }
    }

    /// The cycle is over and the mic is where the user left it.
    fn finish(&mut self, world: &mut impl World) {
        self.end();
        if let Err(why) = world.clear() {
            world.problem(Problem::Store(why));
        }
        world.recovered();
    }

    /// The cycle can no longer be honoured: drop it, and say so.
    fn void(&mut self, problem: Problem, world: &mut impl World) {
        self.end();
        if let Err(why) = world.clear() {
            world.problem(Problem::Store(why));
        }
        world.problem(problem);
    }

    fn end(&mut self) {
        self.obligation = None;
        self.hold = None;
        self.pending = None;
        self.restoring = false;
        self.engage_seen = true;
    }

    fn expect(&mut self, pin: Pin, target: bool, restore: bool, world: &mut impl World) {
        self.tokens += 1;
        self.pending = Some(Pending {
            token: self.tokens,
            pin,
            target,
            restore,
        });
        world.arm(self.tokens, CONFIRM);
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    pub const SERVER: u32 = 0xC0FFEE;
    pub const MIC: Pin = Pin {
        server: SERVER,
        serial: 57,
    };
    pub const HEADSET: Pin = Pin {
        server: SERVER,
        serial: 84,
    };

    /// What [`set_mute`] does to the fake server.
    #[derive(Clone, Copy, Debug, PartialEq)]
    pub enum Apply {
        /// Applies at once; the change, if any, shows up at [`Fake::settle`].
        /// As in PipeWire, setting what already is shows nothing.
        Normally,
        /// Applies only at [`Fake::settle`], in order, each change showing
        /// as it lands: a message still on its way.
        Later,
        /// Not sent at all.
        Refuse,
        /// Applies, and the change never shows up (a stalled server).
        Silently,
        /// Not applied, and nothing says so.
        Drop,
    }

    /// A fake PipeWire, store, clock and notifier.
    pub struct Fake {
        pub nodes: Vec<(Pin, bool)>,
        pub default: Pin,
        pub saved: Option<Obligation>,
        pub save_fails: bool,
        pub apply: Apply,
        /// Changes applied but not yet observed.
        pub unseen: Vec<(Pin, bool)>,
        /// Changes sent but not yet applied.
        pub sent: Vec<(Pin, bool)>,
        pub armed: Vec<u64>,
        pub problems: Vec<Problem>,
        pub recoveries: usize,
        /// The saved record at each set_mute, for ordering checks.
        pub saved_at_set: Vec<Option<Obligation>>,
    }

    impl Fake {
        pub fn new(muted: bool) -> Self {
            Fake {
                nodes: vec![(MIC, muted)],
                default: MIC,
                saved: None,
                save_fails: false,
                apply: Apply::Normally,
                unseen: Vec::new(),
                sent: Vec::new(),
                armed: Vec::new(),
                problems: Vec::new(),
                recoveries: 0,
                saved_at_set: Vec::new(),
            }
        }

        pub fn get(&self, pin: Pin) -> Option<bool> {
            self.nodes
                .iter()
                .find_map(|&(node, muted)| (node == pin).then_some(muted))
        }

        fn put(&mut self, pin: Pin, muted: bool) {
            if let Some(entry) = self.nodes.iter_mut().find(|(node, _)| *node == pin) {
                entry.1 = muted;
            }
        }

        /// The mute key, or any client but talkd: changed and observed.
        pub fn external(&mut self, machine: &mut Machine, pin: Pin, muted: bool) {
            self.put(pin, muted);
            machine.handle(Event::Observed(pin, muted), self);
        }

        /// Delivers what is on its way: param events of applied changes, then
        /// the changes still to land.
        pub fn settle(&mut self, machine: &mut Machine) {
            for (pin, muted) in std::mem::take(&mut self.unseen) {
                machine.handle(Event::Observed(pin, muted), self);
            }
            for (pin, muted) in std::mem::take(&mut self.sent) {
                if self.get(pin).is_some_and(|now| now != muted) {
                    self.put(pin, muted);
                    machine.handle(Event::Observed(pin, muted), self);
                }
            }
        }

        /// Fires the latest deadline.
        pub fn expire(&mut self, machine: &mut Machine) {
            let token = *self.armed.last().expect("a deadline armed");
            machine.handle(Event::Deadline(token), self);
        }
    }

    impl World for Fake {
        fn default_source(&self) -> Option<(Pin, bool)> {
            self.get(self.default).map(|muted| (self.default, muted))
        }

        fn muted(&self, pin: Pin) -> Option<bool> {
            self.get(pin)
        }

        fn set_mute(&mut self, pin: Pin, muted: bool) -> Result<(), String> {
            self.saved_at_set.push(self.saved);
            match self.apply {
                Apply::Refuse => return Err("not connected".to_owned()),
                Apply::Drop => {}
                Apply::Normally => {
                    if self.get(pin).is_some_and(|now| now != muted) {
                        self.put(pin, muted);
                        self.unseen.push((pin, muted));
                    }
                }
                Apply::Later => self.sent.push((pin, muted)),
                Apply::Silently => self.put(pin, muted),
            }
            Ok(())
        }

        fn save(&mut self, obligation: &Obligation) -> Result<(), String> {
            if self.save_fails {
                return Err("disk full".to_owned());
            }
            self.saved = Some(*obligation);
            Ok(())
        }

        fn clear(&mut self) -> Result<(), String> {
            self.saved = None;
            Ok(())
        }

        fn arm(&mut self, token: u64, _after: Duration) {
            self.armed.push(token);
        }

        fn problem(&mut self, problem: Problem) {
            self.problems.push(problem);
        }

        fn recovered(&mut self) {
            self.recoveries += 1;
        }
    }

    const MUTED: bool = true;
    const LIVE: bool = false;

    fn ms(ms: u64) -> Duration {
        Duration::from_millis(ms)
    }

    /// A machine connected and unlocked, over a fake mic muted `muted`.
    fn ready(muted: bool) -> (Machine, Fake) {
        let mut fake = Fake::new(muted);
        let mut machine = Machine::new(None);
        machine.handle(Event::ServerUp(SERVER), &mut fake);
        machine.handle(Event::Lock(Lock::Unlocked), &mut fake);
        (machine, fake)
    }

    /// Runs `events`, letting every applied change show up after each.
    fn run(machine: &mut Machine, fake: &mut Fake, events: &[Event]) {
        for &event in events {
            machine.handle(event, fake);
            fake.settle(machine);
        }
    }

    #[test]
    fn a_hold_talks_through_a_muted_mic_and_mutes_a_live_one() {
        for start in [MUTED, LIVE] {
            let (mut machine, mut fake) = ready(start);
            run(&mut machine, &mut fake, &[Event::Press(ms(1_000))]);
            assert_eq!(fake.get(MIC), Some(!start));
            assert_eq!(
                fake.saved,
                Some(Obligation {
                    pin: MIC,
                    was_muted: start
                })
            );
            run(&mut machine, &mut fake, &[Event::Release(ms(3_000))]);
            assert_eq!(fake.get(MIC), Some(start));
            assert_eq!(fake.saved, None, "cleared once confirmed");
            assert!(fake.problems.is_empty(), "{:?}", fake.problems);
        }
    }

    #[test]
    fn a_tap_latches_until_the_next_press_whose_release_is_ignored() {
        let (mut machine, mut fake) = ready(MUTED);
        run(
            &mut machine,
            &mut fake,
            &[Event::Press(ms(1_000)), Event::Release(ms(1_249))],
        );
        assert_eq!(fake.get(MIC), Some(LIVE), "latched");
        run(&mut machine, &mut fake, &[Event::Press(ms(9_000))]);
        assert_eq!(fake.get(MIC), Some(MUTED));
        assert_eq!(fake.saved, None);
        run(&mut machine, &mut fake, &[Event::Release(ms(20_000))]);
        assert_eq!(fake.get(MIC), Some(MUTED), "that release does nothing");
    }

    #[test]
    fn the_latch_threshold_is_250_ms() {
        for (held, latched) in [(249, true), (250, false)] {
            let (mut machine, mut fake) = ready(MUTED);
            run(
                &mut machine,
                &mut fake,
                &[Event::Press(ms(5_000)), Event::Release(ms(5_000 + held))],
            );
            assert_eq!(
                fake.get(MIC),
                Some(if latched { LIVE } else { MUTED }),
                "{held}"
            );
        }
    }

    #[test]
    fn losing_the_keyboard_mid_hold_restores_even_within_a_tap() {
        let (mut machine, mut fake) = ready(MUTED);
        run(
            &mut machine,
            &mut fake,
            &[Event::Press(ms(1_000)), Event::DeviceLost],
        );
        assert_eq!(fake.get(MIC), Some(MUTED));
        assert_eq!(fake.saved, None);
        // and while latched it changes nothing: the next press still ends it
        run(
            &mut machine,
            &mut fake,
            &[
                Event::Press(ms(2_000)),
                Event::Release(ms(2_100)),
                Event::DeviceLost,
            ],
        );
        assert_eq!(fake.get(MIC), Some(LIVE));
    }

    #[test]
    fn a_press_ending_a_latch_undone_outside_starts_a_fresh_cycle() {
        for start in [MUTED, LIVE] {
            let (mut machine, mut fake) = ready(start);
            run(
                &mut machine,
                &mut fake,
                &[Event::Press(ms(1_000)), Event::Release(ms(1_100))],
            );
            fake.external(&mut machine, MIC, start); // the mic mute key
            assert_eq!(
                fake.saved.map(|owed| owed.was_muted),
                Some(start),
                "still latched"
            );
            run(&mut machine, &mut fake, &[Event::Press(ms(9_000))]);
            assert_eq!(fake.get(MIC), Some(!start), "a fresh hold, from {start}");
            run(&mut machine, &mut fake, &[Event::Release(ms(12_000))]);
            assert_eq!(fake.get(MIC), Some(start));
            assert_eq!(fake.saved, None);
        }
    }

    #[test]
    fn a_hold_undone_outside_is_not_restored_over() {
        let (mut machine, mut fake) = ready(MUTED);
        run(&mut machine, &mut fake, &[Event::Press(ms(1_000))]);
        fake.external(&mut machine, MIC, MUTED);
        let sets = fake.saved_at_set.len();
        run(&mut machine, &mut fake, &[Event::Release(ms(3_000))]);
        assert_eq!(fake.saved_at_set.len(), sets, "no set-mute");
        assert_eq!(fake.saved, None);
    }

    #[test]
    fn a_cycle_stays_on_its_node_and_the_next_one_takes_the_new_default() {
        let (mut machine, mut fake) = ready(MUTED);
        fake.nodes.push((HEADSET, MUTED));
        run(
            &mut machine,
            &mut fake,
            &[Event::Press(ms(1_000)), Event::Release(ms(1_100))],
        );
        fake.default = HEADSET; // plugged in while latched
        run(&mut machine, &mut fake, &[Event::Press(ms(5_000))]);
        assert_eq!(fake.nodes, [(MIC, MUTED), (HEADSET, MUTED)], "MIC restored");
        run(
            &mut machine,
            &mut fake,
            &[Event::Release(ms(6_000)), Event::Press(ms(7_000))],
        );
        assert_eq!(
            fake.nodes,
            [(MIC, MUTED), (HEADSET, LIVE)],
            "the new default"
        );
        assert_eq!(fake.saved.map(|owed| owed.pin), Some(HEADSET));
    }

    #[test]
    fn a_node_gone_or_the_server_gone_voids_the_cycle_and_says_so() {
        let (mut machine, mut fake) = ready(MUTED);
        run(&mut machine, &mut fake, &[Event::Press(ms(1_000))]);
        fake.nodes.clear();
        run(&mut machine, &mut fake, &[Event::NodeGone(MIC)]);
        assert_eq!(fake.saved, None);
        assert_eq!(fake.problems, [Problem::NodeGone]);
        // its release finds nothing owed
        run(&mut machine, &mut fake, &[Event::Release(ms(3_000))]);
        assert_eq!(fake.problems.len(), 1);

        let (mut machine, mut fake) = ready(MUTED);
        run(&mut machine, &mut fake, &[Event::Press(ms(1_000))]);
        run(&mut machine, &mut fake, &[Event::ServerGone]);
        assert_eq!(fake.saved, None);
        assert_eq!(fake.problems, [Problem::ServerGone]);
        run(&mut machine, &mut fake, &[Event::Press(ms(2_000))]);
        assert_eq!(fake.problems.last(), Some(&Problem::NotReady));
        // reconnected (a new server): presses work again
        run(&mut machine, &mut fake, &[Event::ServerUp(SERVER + 1)]);
        fake.nodes = vec![(
            Pin {
                server: SERVER + 1,
                serial: 3,
            },
            MUTED,
        )];
        fake.default = fake.nodes[0].0;
        run(&mut machine, &mut fake, &[Event::Press(ms(3_000))]);
        assert_eq!(fake.nodes[0].1, LIVE);
    }

    #[test]
    fn a_locked_session_ends_cycles_but_never_starts_one() {
        let (mut machine, mut fake) = ready(MUTED);
        run(
            &mut machine,
            &mut fake,
            &[
                Event::Press(ms(1_000)),
                Event::Release(ms(1_100)),
                Event::Lock(Lock::Locked),
            ],
        );
        assert_eq!(fake.get(MIC), Some(LIVE), "latched before the lock");
        run(&mut machine, &mut fake, &[Event::Press(ms(5_000))]);
        assert_eq!(fake.get(MIC), Some(MUTED), "ended while locked");
        run(
            &mut machine,
            &mut fake,
            &[Event::Release(ms(5_500)), Event::Press(ms(6_000))],
        );
        assert_eq!(fake.get(MIC), Some(MUTED), "but not started");
        assert_eq!(fake.problems, [Problem::Locked(Lock::Locked)]);
        // a latch put back outside while locked: the press ends it, no more
        run(&mut machine, &mut fake, &[Event::Lock(Lock::Unlocked)]);
        run(
            &mut machine,
            &mut fake,
            &[Event::Press(ms(7_000)), Event::Release(ms(7_100))],
        );
        run(&mut machine, &mut fake, &[Event::Lock(Lock::Unknown)]);
        fake.external(&mut machine, MIC, MUTED);
        run(&mut machine, &mut fake, &[Event::Press(ms(8_000))]);
        assert_eq!(fake.get(MIC), Some(MUTED));
        assert_eq!(fake.saved, None);
        assert_eq!(fake.problems.last(), Some(&Problem::Locked(Lock::Unknown)));
    }

    #[test]
    fn the_record_is_saved_before_the_mic_changes_and_kept_until_confirmed() {
        let (mut machine, mut fake) = ready(MUTED);
        let owed = Some(Obligation {
            pin: MIC,
            was_muted: MUTED,
        });
        machine.handle(Event::Press(ms(1_000)), &mut fake);
        assert_eq!(fake.saved_at_set, [owed], "saved first");
        fake.settle(&mut machine);
        machine.handle(Event::Release(ms(3_000)), &mut fake);
        assert_eq!(fake.saved, owed, "restore sent, not yet seen");
        fake.settle(&mut machine);
        assert_eq!(fake.saved, None);
        // a save that fails leaves the mic alone
        fake.save_fails = true;
        machine.handle(Event::Press(ms(5_000)), &mut fake);
        assert_eq!(fake.get(MIC), Some(MUTED));
        assert_eq!(fake.problems, [Problem::Store("disk full".to_owned())]);
    }

    #[test]
    fn an_engage_not_sent_owes_nothing() {
        let (mut machine, mut fake) = ready(MUTED);
        fake.apply = Apply::Refuse;
        run(&mut machine, &mut fake, &[Event::Press(ms(1_000))]);
        assert_eq!(fake.saved, None);
        assert_eq!(
            fake.problems,
            [Problem::EngageFailed("not connected".to_owned())]
        );
        run(&mut machine, &mut fake, &[Event::Release(ms(3_000))]);
        assert_eq!(fake.problems.len(), 1, "nothing to restore");
    }

    #[test]
    fn an_unconfirmed_engage_keeps_the_restore_owed() {
        // applied, but the server never says so
        let (mut machine, mut fake) = ready(MUTED);
        fake.apply = Apply::Silently;
        run(&mut machine, &mut fake, &[Event::Press(ms(1_000))]);
        fake.expire(&mut machine);
        assert_eq!(fake.problems, [Problem::EngageUnconfirmed]);
        assert!(fake.saved.is_some());
        fake.apply = Apply::Normally;
        run(&mut machine, &mut fake, &[Event::Release(ms(3_000))]);
        assert_eq!(fake.get(MIC), Some(MUTED), "restored anyway");
        assert_eq!(fake.saved, None);
    }

    #[test]
    fn an_unconfirmed_restore_stays_owed_and_reported_until_seen() {
        let (mut machine, mut fake) = ready(MUTED);
        run(&mut machine, &mut fake, &[Event::Press(ms(1_000))]);
        // sent, never applied, and nothing says so
        fake.apply = Apply::Drop;
        run(&mut machine, &mut fake, &[Event::Release(ms(3_000))]);
        fake.expire(&mut machine);
        assert_eq!(fake.problems, [Problem::RestoreUnconfirmed]);
        assert!(fake.saved.is_some(), "still owed");
        assert_eq!(fake.recoveries, 0);
        // the next press retries it
        fake.apply = Apply::Normally;
        run(&mut machine, &mut fake, &[Event::Press(ms(9_000))]);
        assert_eq!(fake.get(MIC), Some(MUTED));
        assert_eq!(fake.saved, None);
        assert_eq!(fake.recoveries, 1);
    }

    #[test]
    fn a_late_confirmation_still_completes_the_restore() {
        let (mut machine, mut fake) = ready(MUTED);
        run(&mut machine, &mut fake, &[Event::Press(ms(1_000))]);
        fake.apply = Apply::Silently;
        run(&mut machine, &mut fake, &[Event::Release(ms(3_000))]);
        fake.expire(&mut machine);
        assert!(fake.saved.is_some());
        machine.handle(Event::Observed(MIC, MUTED), &mut fake);
        assert_eq!(fake.saved, None);
        assert_eq!(fake.recoveries, 1);
    }

    #[test]
    fn a_restore_not_sent_stays_owed() {
        let (mut machine, mut fake) = ready(MUTED);
        run(&mut machine, &mut fake, &[Event::Press(ms(1_000))]);
        fake.apply = Apply::Refuse;
        run(&mut machine, &mut fake, &[Event::Release(ms(3_000))]);
        assert_eq!(
            fake.problems,
            [Problem::RestoreFailed("not connected".to_owned())]
        );
        assert!(fake.saved.is_some());
        fake.apply = Apply::Normally;
        run(&mut machine, &mut fake, &[Event::Press(ms(4_000))]);
        assert_eq!(fake.get(MIC), Some(MUTED));
        assert_eq!(fake.saved, None);
    }

    #[test]
    fn startup_restores_a_matching_record_and_drops_others() {
        let owed = Obligation {
            pin: MIC,
            was_muted: MUTED,
        };
        // same server, node there, still flipped: restored
        let mut fake = Fake::new(LIVE);
        fake.saved = Some(owed);
        let mut machine = Machine::new(Some(owed));
        machine.handle(Event::Press(ms(1)), &mut fake);
        assert_eq!(fake.problems, [Problem::NotReady], "not before the server");
        run(&mut machine, &mut fake, &[Event::ServerUp(SERVER)]);
        assert_eq!(fake.get(MIC), Some(MUTED));
        assert_eq!(fake.saved, None);

        // already put back: just cleared
        let mut fake = Fake::new(MUTED);
        fake.saved = Some(owed);
        let mut machine = Machine::new(Some(owed));
        run(&mut machine, &mut fake, &[Event::ServerUp(SERVER)]);
        assert_eq!((fake.saved, fake.saved_at_set.len()), (None, 0));

        // another server, or the node gone: dropped, with a report
        for (server, nodes) in [(SERVER + 1, vec![(MIC, LIVE)]), (SERVER, Vec::new())] {
            let mut fake = Fake::new(LIVE);
            fake.nodes = nodes.clone();
            fake.saved = Some(owed);
            let mut machine = Machine::new(Some(owed));
            run(&mut machine, &mut fake, &[Event::ServerUp(server)]);
            assert_eq!(fake.saved, None);
            assert_eq!(fake.saved_at_set.len(), 0, "never touched");
            assert!(matches!(fake.problems[..], [Problem::Stale(_)]));
        }
    }

    #[test]
    fn stopping_restores_what_is_owed_then_settles() {
        let (mut machine, mut fake) = ready(MUTED);
        run(
            &mut machine,
            &mut fake,
            &[Event::Press(ms(1_000)), Event::Release(ms(1_100))],
        );
        machine.handle(Event::Stop, &mut fake);
        assert!(!machine.settled(), "waiting for the restore");
        fake.settle(&mut machine);
        assert!(machine.settled());
        assert_eq!((fake.get(MIC), fake.saved), (Some(MUTED), None));
        // an unconfirmed one settles at its deadline, still saved for the next start
        let (mut machine, mut fake) = ready(MUTED);
        run(&mut machine, &mut fake, &[Event::Press(ms(1_000))]);
        fake.apply = Apply::Drop;
        machine.handle(Event::Stop, &mut fake);
        fake.expire(&mut machine);
        assert!(machine.settled());
        assert!(fake.saved.is_some());
    }

    #[test]
    fn a_release_before_its_engage_shows_still_restores() {
        // the flip is on its way when the keyboard goes: the mute still reads
        // as it was, which must not pass for "nothing owed"
        let (mut machine, mut fake) = ready(MUTED);
        fake.apply = Apply::Later;
        machine.handle(Event::Press(ms(1_000)), &mut fake);
        machine.handle(Event::DeviceLost, &mut fake);
        assert_eq!(
            fake.sent,
            [(MIC, LIVE), (MIC, MUTED)],
            "restore sent behind it"
        );
        assert!(fake.saved.is_some());
        fake.settle(&mut machine);
        assert_eq!((fake.get(MIC), fake.saved), (Some(MUTED), None));
        // nor for "put back outside" at the press after a latch
        let (mut machine, mut fake) = ready(MUTED);
        fake.apply = Apply::Later;
        machine.handle(Event::Press(ms(1_000)), &mut fake);
        machine.handle(Event::Release(ms(1_100)), &mut fake);
        machine.handle(Event::Press(ms(1_200)), &mut fake);
        assert_eq!(
            fake.sent,
            [(MIC, LIVE), (MIC, MUTED)],
            "a restore, not a fresh flip"
        );
        fake.settle(&mut machine);
        assert_eq!((fake.get(MIC), fake.saved), (Some(MUTED), None));
    }

    #[test]
    fn a_dropped_engage_is_restored_anyway_reported_then_released() {
        let (mut machine, mut fake) = ready(MUTED);
        fake.apply = Apply::Drop;
        run(&mut machine, &mut fake, &[Event::Press(ms(1_000))]);
        fake.expire(&mut machine);
        fake.apply = Apply::Normally;
        run(&mut machine, &mut fake, &[Event::Release(ms(3_000))]);
        assert_eq!(fake.saved_at_set.len(), 2, "the restore is sent");
        fake.expire(&mut machine); // nothing changes, so nothing confirms it
        assert_eq!(
            fake.problems,
            [Problem::EngageUnconfirmed, Problem::RestoreUnconfirmed]
        );
        assert!(fake.saved.is_some(), "kept, reported");
        // the next press sees it settled and starts afresh
        run(&mut machine, &mut fake, &[Event::Press(ms(9_000))]);
        assert_eq!(fake.get(MIC), Some(LIVE));
        assert_eq!(fake.recoveries, 1);
    }

    #[test]
    fn stopping_mid_engage_restores_behind_it() {
        let (mut machine, mut fake) = ready(MUTED);
        fake.apply = Apply::Later;
        machine.handle(Event::Press(ms(1_000)), &mut fake);
        machine.handle(Event::Stop, &mut fake);
        assert!(!machine.settled());
        fake.settle(&mut machine);
        assert!(machine.settled());
        assert_eq!((fake.get(MIC), fake.saved), (Some(MUTED), None));
    }

    #[test]
    fn only_restore_troubles_are_visible() {
        assert!(Problem::RestoreUnconfirmed.visible());
        assert!(Problem::NodeGone.visible());
        assert!(Problem::ServerGone.visible());
        assert!(!Problem::Locked(Lock::Locked).visible());
        assert!(!Problem::EngageUnconfirmed.visible());
    }
}
