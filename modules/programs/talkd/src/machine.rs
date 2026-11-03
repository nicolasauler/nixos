//! The push-to-talk cycle: one state machine, fed every event in order, that
//! acts only through [`World`]. No I/O here.
//!
//! A press inverts the default source's mute and owes a restore; the
//! obligation (server, node, node.name, previous mute) is saved before the
//! mic is touched and cleared only once the restore is seen to have
//! happened. A release 250 ms or more after its press restores, a quicker
//! one latches, and the press ending a latch restores, unless the mute was
//! put back outside talkd meanwhile, in which case that press starts a fresh
//! cycle. While the session is locked (or its lock state is unknown) a press
//! may only end a cycle, never start one.
//!
//! A mic that goes away mid-cycle (a headset dropping, PipeWire restarting)
//! ends its cycle, but not what it owes: the restore turns dormant, saved by
//! node.name, and is carried out when a source of that name is back. New
//! presses never drop one; a press on such a mic before its restore starts
//! from the mute it is owed, not from the flip it came back with.
//!
//! WirePlumber is told every restore owed (node.name to the mute it goes
//! back to), from before a flip until that restore is seen to be done, so it
//! can put a returning mic back to that mute too, whichever lands first.

use std::collections::BTreeMap;
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

/// What a cycle owes: putting `pin`'s mute back to `was_muted`. `name` is
/// the node's node.name, which outlives the node; records written before
/// talkd kept it have none. `description` is its node.description, to name
/// it in notices.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Obligation {
    pub pin: Pin,
    pub name: Option<String>,
    pub description: Option<String>,
    pub was_muted: bool,
}

/// A restore whose mic went away: due to the next source named `name`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dormant {
    pub name: String,
    pub description: Option<String>,
    pub was_muted: bool,
}

impl Dormant {
    /// What notices call the mic.
    pub fn label(&self) -> &str {
        self.description.as_deref().unwrap_or(&self.name)
    }

    fn back(&self) -> &'static str {
        if self.was_muted { "muted" } else { "live" }
    }
}

/// A source as the machine sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Source {
    pub pin: Pin,
    pub name: String,
    pub description: Option<String>,
    pub muted: bool,
}

/// Writes of a dormant restore per appearance of its mic.
pub const TRIES: u32 = 3;

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
    /// Connected and synced to the server with this cookie.
    ServerUp(u32),
    ServerGone,
    /// A request-scoped readback, taken after the server processed the
    /// corresponding write. Unsolicited/cached mute values cannot confirm it.
    Confirmed(u64, Pin, bool),
    /// The mute this write went to moved (node Props to device Route or
    /// back) before it was seen, and its new place does not hold the
    /// write's value: the request is over, unconfirmed.
    Relocated(u64, Pin),
    NodeGone(Pin),
    /// Sources came or their params settled: a dormant mic may be back.
    SourcesChanged,
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
    /// The cycle's mic, or its server, went away; its restore waits for the
    /// mic to come back.
    Awaiting {
        owed: Dormant,
        server_gone: bool,
    },
    /// A dormant restore's write to its returned mic was not sent.
    ReturnFailed {
        owed: Dormant,
        why: String,
    },
    /// A dormant restore's write to its returned mic was not seen in time.
    ReturnUnconfirmed(Dormant),
    /// A dormant restore was done, but dropping it from the saved list
    /// failed: a restart before that is saved would write it again.
    NotRetired(Dormant),
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
                | Problem::Awaiting { .. }
                | Problem::ReturnFailed { .. }
                | Problem::ReturnUnconfirmed(_)
                | Problem::NotRetired(_)
                | Problem::Stale(_)
        )
    }

    /// The dormant restore (by node.name) this is about; None for the
    /// active cycle and the rest. Each restore's notices stand on their own.
    pub fn subject(&self) -> Option<&str> {
        match self {
            Problem::Awaiting { owed, .. }
            | Problem::ReturnFailed { owed, .. }
            | Problem::ReturnUnconfirmed(owed)
            | Problem::NotRetired(owed) => Some(&owed.name),
            _ => None,
        }
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
            Problem::Awaiting { owed, server_gone } => {
                let (mic, back) = (owed.label(), owed.back());
                if *server_gone {
                    format!(
                        "PipeWire went away mid-cycle; {mic} goes back to {back} when it returns"
                    )
                } else {
                    format!(
                        "{mic}, which talkd flipped, is gone; it goes back to {back} when it returns"
                    )
                }
            }
            Problem::ReturnFailed { owed, why } => format!(
                "could not put {}'s mute back to {}: {why}",
                owed.label(),
                owed.back()
            ),
            Problem::ReturnUnconfirmed(owed) => format!(
                "putting {}'s mute back to {} was not confirmed; check the mic",
                owed.label(),
                owed.back()
            ),
            Problem::NotRetired(owed) => format!(
                "{}'s mute is back to {}, but talkd could not record that; a restart before it can may put it back to {} again",
                owed.label(),
                owed.back(),
                owed.back()
            ),
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
    /// The default source.
    fn default_source(&self) -> Option<Source>;
    /// The newest source named `name` and its settled mute; None while there
    /// is none, or its mute is refreshing or unavailable.
    fn find(&self, name: &str) -> Option<(Pin, bool)>;
    /// The current lock state, queried only before a new engagement.
    fn lock_state(&mut self) -> Lock;
    /// Whether the bound node still exists, independently of parameter refresh.
    fn exists(&self, pin: Pin) -> bool;
    /// A settled mute snapshot; None while refreshing or if unavailable.
    fn muted(&self, pin: Pin) -> Option<bool>;
    /// Sends a mute change identified by `token`. Err only if certainly not sent.
    fn set_mute(&mut self, pin: Pin, muted: bool, token: u64) -> Result<(), String>;
    fn save(&mut self, obligation: &Obligation) -> Result<(), String>;
    fn clear(&mut self) -> Result<(), String>;
    /// Replaces the saved dormant restores with `dormant`.
    fn save_dormant(&mut self, dormant: &[Dormant]) -> Result<(), String>;
    /// Delivers [`Event::Deadline`]`(token)` after `after`.
    fn arm(&mut self, token: u64, after: Duration);
    fn problem(&mut self, problem: Problem);
    /// The mic is back where the user left it: the active cycle's (None),
    /// or the dormant restore's of this node.name.
    fn recovered(&mut self, dormant: Option<&str>);
    /// Tells WirePlumber every restore owed, node.name to the mute it goes
    /// back to (none: nothing owed). Called on changes only, and before the
    /// write that flips the mic.
    fn publish(&mut self, owed: &BTreeMap<String, bool>);
}

/// A mute change sent and not yet seen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Pending {
    token: u64,
    pin: Pin,
    target: bool,
    restore: bool,
    expired: bool,
}

impl Pending {
    fn is(&self, token: u64, pin: Pin) -> bool {
        self.token == token && self.pin == pin
    }
}

/// A dormant restore, and its writes once the mic is back.
#[derive(Clone, Debug)]
struct Waiting {
    owed: Dormant,
    /// The latest write; once expired, a late readback still completes it.
    sent: Option<Pending>,
    /// The mic's current appearance, and the writes sent to it.
    at: Option<Pin>,
    tries: u32,
}

impl Waiting {
    fn new(owed: Dormant) -> Self {
        Waiting {
            owed,
            sent: None,
            at: None,
            tries: 0,
        }
    }

    /// The mic went away, or its server: the next appearance starts over.
    fn reset(&mut self) {
        self.sent = None;
        self.at = None;
        self.tries = 0;
    }
}

#[derive(Debug)]
pub struct Machine {
    obligation: Option<Obligation>,
    /// When the press that engaged went down, while it is still down.
    hold: Option<Duration>,
    pending: Option<Pending>,
    /// A restore was sent; only its request-scoped confirmation may finish it.
    restoring: bool,
    /// The engage's request has been confirmed. Until then the original
    /// cached mute proves nothing: the flip may still land.
    engage_seen: bool,
    server: Option<u32>,
    /// Saved by a previous run, settled at the first [`Event::ServerUp`].
    recovering: Option<Obligation>,
    /// Restores owed to mics that went away, at most one per name, never
    /// the active obligation's.
    dormant: Vec<Waiting>,
    /// `dormant` differs from its saved list: saving it failed.
    dirty: bool,
    /// The obligation record holds a restore that is dormant now, kept until
    /// the dormant list holding it is saved.
    fallback: bool,
    /// What [`World::publish`] last said (or, at start, what the caller
    /// published from the saved records: [`Machine::owed`]).
    published: BTreeMap<String, bool>,
    tokens: u64,
    stopping: bool,
}

impl Machine {
    pub fn new(saved: Option<Obligation>, dormant: Vec<Dormant>) -> Self {
        let mut machine = Machine {
            obligation: None,
            hold: None,
            pending: None,
            restoring: false,
            engage_seen: true,
            server: None,
            recovering: saved,
            dormant: dormant.into_iter().map(Waiting::new).collect(),
            dirty: false,
            fallback: false,
            published: BTreeMap::new(),
            tokens: 0,
            stopping: false,
        };
        machine.published = machine.owed();
        machine
    }

    /// Every restore owed, by node.name: the active cycle's (or the one a
    /// previous run saved) until it is done, and each dormant one until its
    /// write is confirmed. A record without a name has none to give.
    pub fn owed(&self) -> BTreeMap<String, bool> {
        // the obligation last: for one name it outranks a dormant restore,
        // as at recovery
        let active = self.obligation.iter().chain(&self.recovering);
        self.dormant
            .iter()
            .map(|waiting| (waiting.owed.name.clone(), waiting.owed.was_muted))
            .chain(active.filter_map(|owed| Some((owed.name.clone()?, owed.was_muted))))
            .collect()
    }

    /// Stopping, and nothing more to wait for: no restore owed, or its
    /// outcome known (confirmed, failed, or unconfirmed past its deadline).
    /// Dormant restores stay saved for the next start.
    pub fn settled(&self) -> bool {
        self.stopping
            && !self.dirty
            && (self.obligation.is_none()
                || self.server.is_none()
                || self.pending.is_none_or(|pending| pending.expired))
    }

    pub fn handle(&mut self, event: Event, world: &mut impl World) {
        self.event(event, world);
        self.announce(world);
    }

    fn event(&mut self, event: Event, world: &mut impl World) {
        match event {
            Event::Press(at) => self.press(at, world),
            Event::Release(at) => self.release(Some(at), world),
            Event::DeviceLost => self.release(None, world),
            Event::ServerUp(server) => {
                self.server = Some(server);
                if let Some(saved) = self.recovering.take() {
                    self.recover(saved, server, world);
                }
                self.revive(world);
            }
            Event::ServerGone => {
                self.server = None;
                // writes on the old connection can no longer be seen; the
                // mic's next appearance gets new ones
                for waiting in &mut self.dormant {
                    waiting.reset();
                }
                if self.obligation.is_some() {
                    self.sleep(true, world);
                }
            }
            Event::Confirmed(token, pin, muted) => self.confirmed(token, pin, muted, world),
            Event::Relocated(token, pin) => self.relocated(token, pin, world),
            Event::NodeGone(pin) => {
                for waiting in &mut self.dormant {
                    if waiting.at == Some(pin) {
                        waiting.reset();
                    }
                }
                if self.obligation.as_ref().is_some_and(|owed| owed.pin == pin) {
                    self.sleep(false, world);
                }
                // a replacement may have settled while that write was out,
                // and nothing else need come to say so
                self.revive(world);
            }
            Event::SourcesChanged => self.revive(world),
            Event::Deadline(token) => {
                if let Some(pending) = self
                    .pending
                    .as_mut()
                    .filter(|pending| pending.token == token)
                {
                    pending.expired = true;
                    world.problem(if pending.restore {
                        Problem::RestoreUnconfirmed
                    } else {
                        Problem::EngageUnconfirmed
                    });
                    // A deadline changes only waiting/reporting, never what
                    // the server is known to have done. Keep the request id.
                } else if let Some(waiting) = self.dormant.iter_mut().find(|waiting| {
                    waiting
                        .sent
                        .is_some_and(|sent| sent.token == token && !sent.expired)
                }) {
                    // Kept, so a late readback still completes it; but nothing
                    // may come (a write the server failed), so write again.
                    if let Some(sent) = waiting.sent.as_mut() {
                        sent.expired = true;
                    }
                    world.problem(Problem::ReturnUnconfirmed(waiting.owed.clone()));
                    self.revive(world);
                }
            }
            Event::Stop => {
                self.stopping = true;
                self.hold = None;
                // a restore done but still saved would be done again at the
                // next start, over whatever the mic is set to by then
                if self.dirty {
                    let _ = self.persist(world);
                }
                let restoring = self
                    .pending
                    .is_some_and(|pending| pending.restore && !pending.expired);
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
        let Some((pin, was_muted)) = self
            .obligation
            .as_ref()
            .map(|owed| (owed.pin, owed.was_muted))
        else {
            return self.engage(at, world);
        };
        match world.muted(pin) {
            // Put back outside talkd after a confirmed flip, with no restore
            // in flight: this press may start afresh.
            Some(muted) if muted == was_muted && self.engage_seen && !self.restoring => {
                self.finish(world);
                self.engage(at, world);
            }
            _ => self.restore(world),
        }
    }

    /// The engaging press's release (`at`), or the keyboard lost (None).
    fn release(&mut self, at: Option<Duration>, world: &mut impl World) {
        let Some(since) = self.hold.take() else {
            return; // not the engaging press's: ignored
        };
        let tap = at.is_some_and(|at| at.checked_sub(since).is_some_and(|held| held < LATCH));
        if !tap {
            self.restore(world);
        }
    }

    fn engage(&mut self, at: Duration, world: &mut impl World) {
        let lock = world.lock_state();
        if lock != Lock::Unlocked {
            return world.problem(Problem::Locked(lock));
        }
        let Some(source) = world.default_source() else {
            return world.problem(Problem::NoDefaultSource);
        };
        // The obligation record may be all that holds a dormant restore.
        if self.dirty && self.persist(world).is_err() {
            return;
        }
        // A mic back from going away mid-cycle and not restored yet still
        // holds the flip: its cycle starts from the mute it is owed.
        let dormant = self
            .dormant
            .iter()
            .position(|waiting| waiting.owed.name == source.name);
        let was_muted = dormant.map_or(source.muted, |index| self.dormant[index].owed.was_muted);
        let pin = source.pin;
        let owed = Obligation {
            pin,
            name: Some(source.name),
            description: source.description,
            was_muted,
        };
        if let Err(why) = world.save(&owed) {
            // never touch the mic without a record of how to put it back
            return world.problem(Problem::Store(why));
        }
        self.fallback = false;
        // The obligation now carries that restore. Saved before the flip; if
        // not, the list stays dirty and is saved again later.
        let taken = dormant.map(|index| self.dormant.remove(index));
        if taken.is_some() {
            let _ = self.persist(world);
        }
        self.obligation = Some(owed);
        self.restoring = false;
        self.engage_seen = false;
        // WirePlumber learns the baseline before it can see the flip
        self.announce(world);
        self.tokens += 1;
        let token = self.tokens;
        match world.set_mute(pin, !was_muted, token) {
            Ok(()) => {
                self.hold = Some(at);
                self.expect(pin, !was_muted, false, token, world);
            }
            Err(why) => {
                // certainly not sent, so nothing is owed but what was before
                self.end();
                match taken {
                    Some(waiting) => {
                        // the obligation record holds it until the list does
                        self.fallback = true;
                        self.dormant.push(Waiting::new(waiting.owed));
                        let _ = self.persist(world);
                    }
                    None => {
                        if let Err(why) = world.clear() {
                            world.problem(Problem::Store(why));
                        }
                    }
                }
                world.problem(Problem::EngageFailed(why));
            }
        }
    }

    fn restore(&mut self, world: &mut impl World) {
        let Some((pin, was_muted)) = self
            .obligation
            .as_ref()
            .map(|owed| (owed.pin, owed.was_muted))
        else {
            return;
        };
        self.hold = None;
        if !world.exists(pin) {
            return self.sleep(false, world);
        }
        if world.muted(pin) == Some(was_muted) && self.engage_seen && !self.restoring {
            return self.finish(world);
        }
        self.tokens += 1;
        let token = self.tokens;
        match world.set_mute(pin, was_muted, token) {
            Ok(()) => {
                self.restoring = true;
                self.expect(pin, was_muted, true, token, world);
            }
            // Still owed, including when params are temporarily unavailable.
            // Do not discard a previous in-flight request on a failed retry.
            Err(why) => world.problem(Problem::RestoreFailed(why)),
        }
    }

    fn confirmed(&mut self, token: u64, pin: Pin, muted: bool, world: &mut impl World) {
        if let Some(pending) = self
            .pending
            .filter(|pending| pending.is(token, pin) && pending.target == muted)
        {
            self.pending = None;
            if pending.restore {
                self.finish(world);
            } else {
                self.engage_seen = true;
            }
            return;
        }
        if let Some(index) = self.dormant.iter().position(|waiting| {
            waiting
                .sent
                .is_some_and(|sent| sent.is(token, pin) && sent.target == muted)
        }) {
            let waiting = self.dormant.remove(index);
            let saved = self.persist(world);
            world.recovered(Some(&waiting.owed.name));
            if saved.is_err() {
                // stays dirty, saved again later (at the latest, at Stop)
                world.problem(Problem::NotRetired(waiting.owed));
            }
        }
    }

    /// The mute moved before the write was seen: send it again, to where
    /// the mute is now, so a hold is not left without effect.
    fn relocated(&mut self, token: u64, pin: Pin, world: &mut impl World) {
        if let Some(pending) = self.pending.filter(|pending| pending.is(token, pin)) {
            self.tokens += 1;
            let token = self.tokens;
            match world.set_mute(pin, pending.target, token) {
                Ok(()) => self.expect(pin, pending.target, pending.restore, token, world),
                // its deadline still reports it unconfirmed
                Err(why) if pending.restore => world.problem(Problem::RestoreFailed(why)),
                Err(why) => world.problem(Problem::EngageFailed(why)),
            }
            return;
        }
        let Some(index) = self
            .dormant
            .iter()
            .position(|waiting| waiting.sent.is_some_and(|sent| sent.is(token, pin)))
        else {
            return;
        };
        // a resend is one of its TRIES writes like any other
        let waiting = &mut self.dormant[index];
        if waiting.tries >= TRIES {
            waiting.sent = None; // until its next appearance
            return world.problem(Problem::ReturnUnconfirmed(waiting.owed.clone()));
        }
        waiting.tries += 1;
        self.tokens += 1;
        let token = self.tokens;
        let target = self.dormant[index].owed.was_muted;
        self.dormant[index].sent = match world.set_mute(pin, target, token) {
            Ok(()) => {
                world.arm(token, CONFIRM);
                Some(Pending {
                    token,
                    pin,
                    target,
                    restore: true,
                    expired: false,
                })
            }
            Err(why) => {
                let owed = self.dormant[index].owed.clone();
                world.problem(Problem::ReturnFailed { owed, why });
                None // tried again when sources change
            }
        };
    }

    fn recover(&mut self, saved: Obligation, server: u32, world: &mut impl World) {
        let restarted = saved.pin.server != server;
        if restarted || !world.exists(saved.pin) {
            let Some(name) = saved.name else {
                return self.void(
                    Problem::Stale(if restarted {
                        "PipeWire restarted since"
                    } else {
                        "its mic is gone"
                    }),
                    world,
                );
            };
            // owed to that mic wherever it is now: due once it is found
            let found = world.find(&name).is_some();
            let owed = Dormant {
                name,
                description: saved.description,
                was_muted: saved.was_muted,
            };
            // the obligation record goes only once the dormant list holds it
            self.fallback = true;
            self.park(owed.clone(), world);
            if !found {
                world.problem(Problem::Awaiting {
                    owed,
                    server_gone: restarted,
                });
            }
            return;
        }
        // Saved later than any dormant restore of the same mic: it wins.
        if let Some(index) = self
            .dormant
            .iter()
            .position(|waiting| saved.name.as_ref() == Some(&waiting.owed.name))
        {
            self.dormant.remove(index);
            let _ = self.persist(world);
        }
        self.obligation = Some(saved);
        // Startup snapshots do not order writes queued on the old connection.
        // Send our own restore and require its fresh, request-scoped readback.
        self.engage_seen = false;
        self.restore(world);
    }

    /// Writes each dormant restore whose mic is back, even one that already
    /// reads as owed: nothing orders that read after whatever else restores
    /// a returning mic's mute (WirePlumber), so only the write's own
    /// readback finishes it. At most [`TRIES`] writes per appearance.
    fn revive(&mut self, world: &mut impl World) {
        if self.server.is_none() {
            return;
        }
        for index in 0..self.dormant.len() {
            let waiting = &mut self.dormant[index];
            if waiting.sent.is_some_and(|sent| !sent.expired) {
                continue;
            }
            let Some((pin, _)) = world.find(&waiting.owed.name) else {
                continue;
            };
            if waiting.at != Some(pin) {
                waiting.at = Some(pin);
                waiting.tries = 0;
            }
            if waiting.tries >= TRIES {
                continue; // until its next appearance
            }
            waiting.tries += 1;
            self.tokens += 1;
            let token = self.tokens;
            let target = waiting.owed.was_muted;
            match world.set_mute(pin, target, token) {
                Ok(()) => {
                    waiting.sent = Some(Pending {
                        token,
                        pin,
                        target,
                        restore: true,
                        expired: false,
                    });
                    world.arm(token, CONFIRM);
                }
                Err(why) => world.problem(Problem::ReturnFailed {
                    owed: waiting.owed.clone(),
                    why,
                }),
            }
        }
    }

    /// The cycle's mic, or its server, went away: the cycle ends and its
    /// restore waits for the mic to return. One without a name is dropped.
    fn sleep(&mut self, server_gone: bool, world: &mut impl World) {
        let Some(Obligation {
            name: Some(name),
            description,
            was_muted,
            ..
        }) = self.obligation.clone()
        else {
            return self.void(
                if server_gone {
                    Problem::ServerGone
                } else {
                    Problem::NodeGone
                },
                world,
            );
        };
        self.end();
        // the obligation record goes only once the dormant list holds it
        self.fallback = true;
        let owed = Dormant {
            name,
            description,
            was_muted,
        };
        self.park(owed.clone(), world);
        world.problem(Problem::Awaiting { owed, server_gone });
    }

    /// Adds a dormant restore, replacing an older one for the same mic, and
    /// saves the list. Kept in memory whether or not that worked.
    fn park(&mut self, owed: Dormant, world: &mut impl World) {
        self.dormant
            .retain(|waiting| waiting.owed.name != owed.name);
        self.dormant.push(Waiting::new(owed));
        let _ = self.persist(world);
    }

    /// Saves the dormant list, and then drops an obligation record kept as
    /// its fallback. On failure the list stays dirty: every later save, and
    /// the next engage before anything else, tries again.
    fn persist(&mut self, world: &mut impl World) -> Result<(), ()> {
        let owed: Vec<Dormant> = self
            .dormant
            .iter()
            .map(|waiting| waiting.owed.clone())
            .collect();
        if let Err(why) = world.save_dormant(&owed) {
            self.dirty = true;
            world.problem(Problem::Store(why));
            return Err(());
        }
        self.dirty = false;
        if self.fallback && self.obligation.is_none() {
            match world.clear() {
                Ok(()) => self.fallback = false,
                Err(why) => world.problem(Problem::Store(why)),
            }
        }
        Ok(())
    }

    /// The cycle is over and the mic is where the user left it.
    fn finish(&mut self, world: &mut impl World) {
        self.end();
        if let Err(why) = world.clear() {
            world.problem(Problem::Store(why));
        }
        if self.dirty {
            let _ = self.persist(world);
        }
        world.recovered(None);
    }

    /// The cycle can no longer be honoured: drop it, and say so.
    fn void(&mut self, problem: Problem, world: &mut impl World) {
        self.end();
        if let Err(why) = world.clear() {
            world.problem(Problem::Store(why));
        }
        if self.dirty {
            let _ = self.persist(world);
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

    /// Publishes what is owed, if that changed. Once per event, so a cycle
    /// going dormant (still owed) is no change at all.
    fn announce(&mut self, world: &mut impl World) {
        let owed = self.owed();
        if owed != self.published {
            world.publish(&owed);
            self.published = owed;
        }
    }

    fn expect(
        &mut self,
        pin: Pin,
        target: bool,
        restore: bool,
        token: u64,
        world: &mut impl World,
    ) {
        self.pending = Some(Pending {
            token,
            pin,
            target,
            restore,
            expired: false,
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

    /// node.name of a pin not given another in [`Fake::names`].
    pub fn name_of(pin: Pin) -> String {
        if pin == MIC {
            "mic".to_owned()
        } else {
            format!("node {}", pin.serial)
        }
    }

    /// node.description of a pin not given another in [`Fake::descriptions`].
    pub fn description_of(pin: Pin) -> Option<String> {
        (pin == MIC).then(|| "Laptop mic".to_owned())
    }

    /// The obligation a cycle on `pin` owes.
    pub fn owed(pin: Pin, was_muted: bool) -> Obligation {
        Obligation {
            pin,
            name: Some(name_of(pin)),
            description: description_of(pin),
            was_muted,
        }
    }

    /// What [`set_mute`] does to the fake server.
    #[derive(Clone, Copy, Debug, PartialEq)]
    pub enum Apply {
        /// Applies now; the barrier and readback arrive at [`Fake::settle`],
        /// including when the value was already equal.
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
        /// Client observations deliberately differ from the fake server.
        pub observed: Vec<(Pin, bool)>,
        pub lock: Lock,
        pub default: Pin,
        /// node.names other than [`name_of`]'s.
        pub names: Vec<(Pin, String)>,
        pub saved: Option<Obligation>,
        pub dormant: Vec<Dormant>,
        /// Saving the dormant list fails, independently of the obligation.
        pub dormant_fails: bool,
        pub save_fails: bool,
        pub apply: Apply,
        /// Request-scoped readbacks not yet delivered.
        pub unseen: Vec<(u64, Pin, bool)>,
        /// Writes not yet processed by the server.
        pub sent: Vec<(u64, Pin, bool)>,
        pub armed: Vec<u64>,
        pub problems: Vec<Problem>,
        /// Each recovery: the active cycle's (None) or a dormant restore's.
        pub recoveries: Vec<Option<String>>,
        /// The saved record at each set_mute, for ordering checks.
        pub saved_at_set: Vec<Option<Obligation>>,
        pub dormant_at_set: Vec<Vec<Dormant>>,
        /// Each [`World::publish`], in order.
        pub published: Vec<BTreeMap<String, bool>>,
        /// What was published at each set_mute.
        pub published_at_set: Vec<BTreeMap<String, bool>>,
    }

    impl Fake {
        pub fn new(muted: bool) -> Self {
            Fake {
                nodes: vec![(MIC, muted)],
                observed: vec![(MIC, muted)],
                lock: Lock::Unlocked,
                default: MIC,
                names: Vec::new(),
                saved: None,
                dormant: Vec::new(),
                dormant_fails: false,
                save_fails: false,
                apply: Apply::Normally,
                unseen: Vec::new(),
                sent: Vec::new(),
                armed: Vec::new(),
                problems: Vec::new(),
                recoveries: Vec::new(),
                saved_at_set: Vec::new(),
                dormant_at_set: Vec::new(),
                published: Vec::new(),
                published_at_set: Vec::new(),
            }
        }

        /// As the mic layer does: of a name's nodes the newest, settled or not.
        pub fn named(&self, name: &str) -> Option<Pin> {
            self.nodes
                .iter()
                .map(|&(pin, _)| pin)
                .filter(|&pin| self.name(pin) == name)
                .max_by_key(|pin| pin.serial)
        }

        pub fn name(&self, pin: Pin) -> String {
            self.names
                .iter()
                .find_map(|(node, name)| (*node == pin).then(|| name.clone()))
                .unwrap_or_else(|| name_of(pin))
        }

        /// A node appears, named `name`, its mute already seen.
        pub fn add(&mut self, pin: Pin, name: &str, muted: bool) {
            self.nodes.push((pin, muted));
            self.observed.push((pin, muted));
            self.names.push((pin, name.to_owned()));
        }

        /// A node disappears, with whatever was on its way to it.
        pub fn remove(&mut self, pin: Pin) {
            self.nodes.retain(|(node, _)| *node != pin);
            self.observed.retain(|(node, _)| *node != pin);
            self.unseen.retain(|(_, node, _)| *node != pin);
            self.sent.retain(|(_, node, _)| *node != pin);
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

        fn observe(&mut self, pin: Pin, muted: bool) {
            if let Some(entry) = self.observed.iter_mut().find(|(node, _)| *node == pin) {
                entry.1 = muted;
            } else {
                self.observed.push((pin, muted));
            }
        }

        /// Another client changed the server; a fresh snapshot then arrived.
        pub fn external(&mut self, pin: Pin, muted: bool) {
            self.put(pin, muted);
            self.observe(pin, muted);
        }

        pub fn confirm(&mut self, machine: &mut Machine, token: u64, pin: Pin, muted: bool) {
            self.observe(pin, muted);
            machine.handle(Event::Confirmed(token, pin, muted), self);
        }

        /// Delivers the queued barriers/readbacks, never speculative state.
        pub fn settle(&mut self, machine: &mut Machine) {
            for (token, pin, muted) in std::mem::take(&mut self.unseen) {
                self.confirm(machine, token, pin, muted);
            }
            for (token, pin, muted) in std::mem::take(&mut self.sent) {
                if self.get(pin).is_some() {
                    self.put(pin, muted);
                    self.confirm(machine, token, pin, muted);
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
        /// The default is a name: of its nodes, the newest.
        fn default_source(&self) -> Option<Source> {
            let pin = self.named(&self.name(self.default))?;
            self.muted(pin).map(|muted| Source {
                pin,
                name: self.name(pin),
                description: description_of(pin),
                muted,
            })
        }

        fn find(&self, name: &str) -> Option<(Pin, bool)> {
            let pin = self.named(name)?;
            Some((pin, self.muted(pin)?))
        }

        fn lock_state(&mut self) -> Lock {
            self.lock
        }

        fn exists(&self, pin: Pin) -> bool {
            self.get(pin).is_some()
        }

        fn muted(&self, pin: Pin) -> Option<bool> {
            self.get(pin)?;
            self.observed
                .iter()
                .find_map(|&(node, muted)| (node == pin).then_some(muted))
        }

        fn set_mute(&mut self, pin: Pin, muted: bool, token: u64) -> Result<(), String> {
            self.saved_at_set.push(self.saved.clone());
            self.dormant_at_set.push(self.dormant.clone());
            self.published_at_set
                .push(self.published.last().cloned().unwrap_or_default());
            match self.apply {
                Apply::Refuse => return Err("not connected".to_owned()),
                Apply::Drop => {}
                Apply::Normally => {
                    self.put(pin, muted);
                    self.unseen.push((token, pin, muted));
                }
                Apply::Later => self.sent.push((token, pin, muted)),
                Apply::Silently => self.put(pin, muted),
            }
            Ok(())
        }

        fn save(&mut self, obligation: &Obligation) -> Result<(), String> {
            if self.save_fails {
                return Err("disk full".to_owned());
            }
            self.saved = Some(obligation.clone());
            Ok(())
        }

        fn clear(&mut self) -> Result<(), String> {
            self.saved = None;
            Ok(())
        }

        fn save_dormant(&mut self, dormant: &[Dormant]) -> Result<(), String> {
            if self.dormant_fails {
                return Err("disk full".to_owned());
            }
            self.dormant = dormant.to_vec();
            Ok(())
        }

        fn arm(&mut self, token: u64, _after: Duration) {
            self.armed.push(token);
        }

        fn problem(&mut self, problem: Problem) {
            self.problems.push(problem);
        }

        fn recovered(&mut self, dormant: Option<&str>) {
            self.recoveries.push(dormant.map(str::to_owned));
        }

        fn publish(&mut self, owed: &BTreeMap<String, bool>) {
            self.published.push(owed.clone());
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
        let mut machine = Machine::new(None, Vec::new());
        machine.handle(Event::ServerUp(SERVER), &mut fake);
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
            assert_eq!(fake.saved, Some(owed(MIC, start)));
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
            fake.external(MIC, start); // the mic mute key
            assert_eq!(
                fake.saved.as_ref().map(|owed| owed.was_muted),
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
        fake.external(MIC, MUTED);
        let sets = fake.saved_at_set.len();
        run(&mut machine, &mut fake, &[Event::Release(ms(3_000))]);
        assert_eq!(fake.saved_at_set.len(), sets, "no set-mute");
        assert_eq!(fake.saved, None);
    }

    #[test]
    fn a_cycle_stays_on_its_node_and_the_next_one_takes_the_new_default() {
        let (mut machine, mut fake) = ready(MUTED);
        fake.nodes.push((HEADSET, MUTED));
        fake.observe(HEADSET, MUTED);
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
        assert_eq!(fake.saved.as_ref().map(|owed| owed.pin), Some(HEADSET));
    }

    /// The mic, back under a new serial of the same server.
    const BACK: Pin = Pin {
        server: SERVER,
        serial: 91,
    };

    /// The dormant restore of the mic named `name`; "mic" is MIC's.
    fn dormant(name: &str, was_muted: bool) -> Dormant {
        Dormant {
            name: name.to_owned(),
            description: (name == "mic").then(|| "Laptop mic".to_owned()),
            was_muted,
        }
    }

    /// A hold on MIC, from `start`, whose node then goes away.
    fn gone_mid_hold(start: bool) -> (Machine, Fake) {
        let (mut machine, mut fake) = ready(start);
        run(&mut machine, &mut fake, &[Event::Press(ms(1_000))]);
        fake.remove(MIC);
        run(&mut machine, &mut fake, &[Event::NodeGone(MIC)]);
        (machine, fake)
    }

    #[test]
    fn a_mic_gone_mid_hold_gets_its_mute_back_when_it_returns() {
        let (mut machine, mut fake) = gone_mid_hold(MUTED);
        assert_eq!(fake.saved, None, "the cycle is over");
        assert_eq!(fake.dormant, [dormant("mic", MUTED)]);
        assert_eq!(
            fake.problems,
            [Problem::Awaiting {
                owed: dormant("mic", MUTED),
                server_gone: false
            }]
        );
        assert_eq!(
            fake.problems[0].message(),
            "Laptop mic, which talkd flipped, is gone; it goes back to muted when it returns"
        );
        // its release finds nothing owed
        run(&mut machine, &mut fake, &[Event::Release(ms(3_000))]);
        assert_eq!(fake.saved_at_set.len(), 1);
        // back under a new serial, still flipped (WirePlumber kept it)
        fake.add(BACK, "mic", LIVE);
        machine.handle(Event::SourcesChanged, &mut fake);
        assert_eq!(fake.get(BACK), Some(MUTED), "restore sent");
        assert_eq!(fake.dormant.len(), 1, "owed until confirmed");
        fake.settle(&mut machine);
        assert_eq!(fake.dormant, []);
        assert_eq!(fake.recoveries.len(), 1);
        assert_eq!(fake.problems.len(), 1);
    }

    #[test]
    fn a_returning_mic_already_at_its_baseline_is_still_written_and_confirmed() {
        // that read is not ordered after WirePlumber putting back the mute
        // it remembers, which may still bring the flip back
        let (mut machine, mut fake) = gone_mid_hold(LIVE);
        fake.add(BACK, "mic", LIVE);
        fake.apply = Apply::Later;
        machine.handle(Event::SourcesChanged, &mut fake);
        assert_eq!(
            fake.sent.last().map(|&(_, pin, muted)| (pin, muted)),
            Some((BACK, LIVE))
        );
        assert_eq!(
            fake.dormant,
            [dormant("mic", LIVE)],
            "owed until its readback"
        );
        fake.settle(&mut machine);
        assert_eq!(fake.dormant, []);
        assert_eq!(fake.recoveries, [Some("mic".to_owned())]);
    }

    #[test]
    fn a_source_of_another_name_or_without_a_settled_mute_does_not_count() {
        let (mut machine, mut fake) = gone_mid_hold(MUTED);
        fake.add(HEADSET, "headset", LIVE);
        fake.nodes.push((BACK, LIVE)); // there, its params still refreshing
        fake.names.push((BACK, "mic".to_owned()));
        let sets = fake.saved_at_set.len();
        run(&mut machine, &mut fake, &[Event::SourcesChanged]);
        assert_eq!(fake.saved_at_set.len(), sets);
        assert_eq!(fake.dormant, [dormant("mic", MUTED)]);
        fake.observe(BACK, LIVE);
        run(&mut machine, &mut fake, &[Event::SourcesChanged]);
        assert_eq!(fake.get(BACK), Some(MUTED));
        assert_eq!(fake.dormant, []);
    }

    #[test]
    fn a_restore_owed_across_a_pipewire_restart_lands_on_the_new_server() {
        let (mut machine, mut fake) = ready(LIVE);
        run(
            &mut machine,
            &mut fake,
            &[Event::Press(ms(1_000)), Event::Release(ms(1_100))],
        );
        run(&mut machine, &mut fake, &[Event::ServerGone]);
        assert_eq!(fake.saved, None);
        assert_eq!(fake.dormant, [dormant("mic", LIVE)]);
        assert_eq!(
            fake.problems[0].message(),
            "PipeWire went away mid-cycle; Laptop mic goes back to live when it returns"
        );
        run(&mut machine, &mut fake, &[Event::Press(ms(2_000))]);
        assert_eq!(fake.problems.last(), Some(&Problem::NotReady));
        // the restarted server counts serials again, and kept the flip
        let new = Pin {
            server: SERVER + 1,
            serial: 3,
        };
        fake.nodes.clear();
        fake.observed.clear();
        fake.add(new, "mic", MUTED);
        fake.default = new;
        run(&mut machine, &mut fake, &[Event::ServerUp(SERVER + 1)]);
        assert_eq!(fake.get(new), Some(LIVE));
        assert_eq!(fake.dormant, []);
        assert_eq!(fake.recoveries.len(), 1);
        // and presses work again
        run(&mut machine, &mut fake, &[Event::Press(ms(3_000))]);
        assert_eq!(fake.get(new), Some(MUTED));
    }

    #[test]
    fn a_write_lost_with_the_server_is_sent_again_on_the_next_one() {
        let (mut machine, mut fake) = gone_mid_hold(MUTED);
        fake.add(BACK, "mic", LIVE);
        fake.apply = Apply::Drop;
        run(&mut machine, &mut fake, &[Event::SourcesChanged]);
        run(&mut machine, &mut fake, &[Event::ServerGone]);
        fake.remove(BACK);
        let new = Pin {
            server: SERVER + 1,
            serial: 3,
        };
        fake.add(new, "mic", LIVE);
        fake.apply = Apply::Normally;
        run(&mut machine, &mut fake, &[Event::ServerUp(SERVER + 1)]);
        assert_eq!(fake.get(new), Some(MUTED));
        assert_eq!(fake.dormant, []);
    }

    #[test]
    fn a_press_on_another_mic_while_one_is_dormant_keeps_its_restore() {
        let (mut machine, mut fake) = gone_mid_hold(MUTED);
        fake.add(HEADSET, "headset", LIVE);
        fake.default = HEADSET;
        run(&mut machine, &mut fake, &[Event::Press(ms(5_000))]);
        assert_eq!(fake.get(HEADSET), Some(MUTED), "a normal cycle");
        assert_eq!(
            fake.saved.as_ref().map(|owed| (owed.pin, owed.was_muted)),
            Some((HEADSET, LIVE))
        );
        assert_eq!(fake.dormant, [dormant("mic", MUTED)], "kept");
        // the mic returns mid-hold: restored, the hold untouched
        fake.add(BACK, "mic", LIVE);
        run(&mut machine, &mut fake, &[Event::SourcesChanged]);
        assert_eq!(fake.get(BACK), Some(MUTED));
        assert_eq!(fake.get(HEADSET), Some(MUTED));
        assert_eq!(fake.dormant, []);
        assert!(fake.saved.is_some());
        run(&mut machine, &mut fake, &[Event::Release(ms(7_000))]);
        assert_eq!(fake.get(HEADSET), Some(LIVE));
        assert_eq!(fake.saved, None);
    }

    #[test]
    fn a_press_on_a_returned_mic_before_its_restore_starts_from_the_baseline() {
        for in_flight in [false, true] {
            let (mut machine, mut fake) = gone_mid_hold(MUTED);
            // back and still flipped, its restore not yet done or not yet seen
            fake.add(BACK, "mic", LIVE);
            fake.default = BACK;
            if in_flight {
                fake.apply = Apply::Later;
                machine.handle(Event::SourcesChanged, &mut fake);
            }
            machine.handle(Event::Press(ms(5_000)), &mut fake);
            let baseline = Obligation {
                pin: BACK,
                name: Some("mic".to_owned()),
                description: None,
                was_muted: MUTED,
            };
            assert_eq!(fake.saved, Some(baseline.clone()), "from the baseline");
            assert_eq!(
                (fake.saved_at_set.last(), fake.dormant_at_set.last()),
                (Some(&Some(baseline)), Some(&Vec::new())),
                "both records saved before the flip"
            );
            assert_eq!(
                fake.sent
                    .last()
                    .map(|&(_, pin, muted)| (pin, muted))
                    .or(fake.unseen.last().map(|&(_, pin, muted)| (pin, muted))),
                Some((BACK, LIVE)),
                "flipped to the opposite of the baseline"
            );
            fake.settle(&mut machine);
            assert_eq!(fake.get(BACK), Some(LIVE), "talks");
            run(&mut machine, &mut fake, &[Event::Release(ms(8_000))]);
            assert_eq!(fake.get(BACK), Some(MUTED), "{in_flight}");
            assert_eq!(
                (fake.saved.clone(), fake.dormant.clone()),
                (None, Vec::new())
            );
        }
    }

    #[test]
    fn an_unconfirmed_dormant_restore_is_retried_a_few_times_then_waits_for_a_return() {
        let (mut machine, mut fake) = gone_mid_hold(MUTED);
        fake.add(BACK, "mic", LIVE);
        fake.apply = Apply::Drop;
        let sets = fake.saved_at_set.len();
        run(&mut machine, &mut fake, &[Event::SourcesChanged]);
        // each deadline reports it and writes again, up to TRIES writes
        for _ in 0..TRIES {
            fake.expire(&mut machine);
        }
        assert_eq!(fake.saved_at_set.len() - sets, TRIES as usize);
        assert_eq!(
            fake.problems.last(),
            Some(&Problem::ReturnUnconfirmed(dormant("mic", MUTED)))
        );
        assert_eq!(fake.dormant, [dormant("mic", MUTED)], "still owed");
        assert_eq!(fake.recoveries.len(), 0);
        // then no more while it stays
        run(&mut machine, &mut fake, &[Event::SourcesChanged]);
        assert_eq!(fake.saved_at_set.len() - sets, TRIES as usize);
        // its next return starts over
        fake.remove(BACK);
        run(&mut machine, &mut fake, &[Event::NodeGone(BACK)]);
        let again = Pin {
            server: SERVER,
            serial: 92,
        };
        fake.add(again, "mic", LIVE);
        fake.apply = Apply::Normally;
        run(&mut machine, &mut fake, &[Event::SourcesChanged]);
        assert_eq!(fake.get(again), Some(MUTED));
        assert_eq!(fake.dormant, []);
        assert_eq!(fake.recoveries.len(), 1);
    }

    #[test]
    fn a_dormant_restore_retried_after_its_deadline_lands_on_the_next_settle() {
        // its write was lost; the next write gets through
        let (mut machine, mut fake) = gone_mid_hold(MUTED);
        fake.add(BACK, "mic", LIVE);
        fake.apply = Apply::Drop;
        run(&mut machine, &mut fake, &[Event::SourcesChanged]);
        fake.apply = Apply::Normally;
        fake.expire(&mut machine);
        fake.settle(&mut machine);
        assert_eq!(fake.get(BACK), Some(MUTED));
        assert_eq!(fake.dormant, []);
    }

    #[test]
    fn a_late_confirmation_still_completes_a_dormant_restore() {
        let (mut machine, mut fake) = gone_mid_hold(MUTED);
        fake.add(BACK, "mic", LIVE);
        fake.apply = Apply::Silently;
        run(&mut machine, &mut fake, &[Event::SourcesChanged]);
        for _ in 0..TRIES {
            fake.expire(&mut machine);
        }
        let token = *fake.armed.last().unwrap();
        fake.confirm(&mut machine, token, BACK, LIVE);
        assert_eq!(fake.dormant.len(), 1, "a different value proves nothing");
        fake.confirm(&mut machine, token, BACK, MUTED);
        assert_eq!(fake.dormant, []);
    }

    #[test]
    fn each_dormant_restore_is_named_and_recovered_on_its_own() {
        let (mut machine, mut fake) = gone_mid_hold(MUTED);
        fake.add(HEADSET, "headset", LIVE);
        fake.default = HEADSET;
        run(&mut machine, &mut fake, &[Event::Press(ms(5_000))]);
        fake.remove(HEADSET);
        run(&mut machine, &mut fake, &[Event::NodeGone(HEADSET)]);
        let messages: Vec<String> = fake.problems.iter().map(Problem::message).collect();
        assert_eq!(
            messages,
            [
                "Laptop mic, which talkd flipped, is gone; it goes back to muted when it returns",
                "headset, which talkd flipped, is gone; it goes back to live when it returns"
            ],
            "by node.description, else node.name"
        );
        assert_eq!(
            fake.problems
                .iter()
                .map(Problem::subject)
                .collect::<Vec<_>>(),
            [Some("mic"), Some("headset")]
        );
        fake.add(BACK, "mic", LIVE);
        run(&mut machine, &mut fake, &[Event::SourcesChanged]);
        assert_eq!(fake.recoveries, [Some("mic".to_owned())]);
        assert_eq!(fake.dormant, [dormant("headset", LIVE)]);
    }

    #[test]
    fn a_flip_not_sent_on_a_returned_mic_puts_its_dormant_restore_back() {
        let (mut machine, mut fake) = gone_mid_hold(MUTED);
        fake.add(BACK, "mic", LIVE);
        fake.default = BACK;
        fake.apply = Apply::Refuse;
        run(&mut machine, &mut fake, &[Event::Press(ms(5_000))]);
        assert_eq!(fake.saved, None);
        assert_eq!(fake.dormant, [dormant("mic", MUTED)], "unchanged");
        fake.apply = Apply::Normally;
        run(&mut machine, &mut fake, &[Event::SourcesChanged]);
        assert_eq!(fake.get(BACK), Some(MUTED));
    }

    #[test]
    fn dormant_restores_are_kept_one_per_mic_the_newest_winning() {
        let (mut machine, mut fake) = gone_mid_hold(MUTED);
        fake.add(HEADSET, "headset", MUTED);
        fake.default = HEADSET;
        run(&mut machine, &mut fake, &[Event::Press(ms(5_000))]);
        fake.remove(HEADSET);
        run(&mut machine, &mut fake, &[Event::NodeGone(HEADSET)]);
        assert_eq!(
            fake.dormant,
            [dormant("mic", MUTED), dormant("headset", MUTED)]
        );
        // a record from a run before that PipeWire restart, newer than the
        // dormant one for its mic: it replaces it, and waits as well
        let saved = owed(MIC, LIVE);
        let mut fake = Fake::new(MUTED);
        fake.nodes.clear();
        fake.dormant = vec![dormant("mic", MUTED), dormant("headset", MUTED)];
        let mut machine = Machine::new(Some(saved), fake.dormant.clone());
        run(&mut machine, &mut fake, &[Event::ServerUp(SERVER + 1)]);
        assert_eq!(
            fake.dormant,
            [dormant("headset", MUTED), dormant("mic", LIVE)]
        );
        assert_eq!(
            fake.problems,
            [Problem::Awaiting {
                owed: dormant("mic", LIVE),
                server_gone: true
            }]
        );
    }

    #[test]
    fn a_write_whose_mute_moved_is_sent_again_where_it_now_is() {
        // engage: the flip landed on Props, then the mute moved to a Route
        // still holding the old value
        let (mut machine, mut fake) = ready(MUTED);
        machine.handle(Event::Press(ms(1_000)), &mut fake);
        let flip = machine.pending.unwrap().token;
        fake.unseen.clear();
        fake.external(MIC, MUTED);
        machine.handle(Event::Relocated(flip, MIC), &mut fake);
        assert_eq!(fake.get(MIC), Some(LIVE), "sent again");
        fake.settle(&mut machine);
        assert!(machine.engage_seen, "under its new token");
        run(&mut machine, &mut fake, &[Event::Release(ms(3_000))]);
        assert_eq!((fake.get(MIC), fake.saved.clone()), (Some(MUTED), None));
        // a stale token is no reason to write
        let sets = fake.saved_at_set.len();
        machine.handle(Event::Relocated(flip, MIC), &mut fake);
        assert_eq!(fake.saved_at_set.len(), sets);

        // a dormant restore likewise
        let (mut machine, mut fake) = gone_mid_hold(MUTED);
        fake.add(BACK, "mic", LIVE);
        machine.handle(Event::SourcesChanged, &mut fake);
        let token = *fake.armed.last().unwrap();
        fake.unseen.clear();
        fake.external(BACK, LIVE);
        machine.handle(Event::Relocated(token, BACK), &mut fake);
        fake.settle(&mut machine);
        assert_eq!(fake.get(BACK), Some(MUTED));
        assert_eq!(fake.dormant, []);
    }

    /// A restart now: a new machine from what is on disk, a new server
    /// where the mic is back (still flipped to `flipped`).
    fn restarted(fake: &Fake, flipped: bool) -> (Machine, Fake) {
        let mut machine = Machine::new(fake.saved.clone(), fake.dormant.clone());
        let mut fresh = Fake::new(flipped);
        fresh.nodes.clear();
        fresh.observed.clear();
        let new = Pin {
            server: SERVER + 1,
            serial: 3,
        };
        fresh.add(new, "mic", flipped);
        fresh.saved = fake.saved.clone();
        fresh.dormant = fake.dormant.clone();
        run(&mut machine, &mut fresh, &[Event::ServerUp(SERVER + 1)]);
        (machine, fresh)
    }

    #[test]
    fn a_dormant_restore_not_saved_keeps_the_obligation_record_as_its_fallback() {
        let (mut machine, mut fake) = ready(MUTED);
        run(&mut machine, &mut fake, &[Event::Press(ms(1_000))]);
        fake.dormant_fails = true;
        fake.remove(MIC);
        run(&mut machine, &mut fake, &[Event::NodeGone(MIC)]);
        assert_eq!(fake.saved, Some(owed(MIC, MUTED)), "kept");
        assert_eq!(fake.dormant, []);
        assert!(matches!(
            fake.problems[..],
            [Problem::Store(_), Problem::Awaiting { .. }]
        ));
        // a restart now still owes it
        let (_, fresh) = restarted(&fake, LIVE);
        assert_eq!(fresh.nodes[0].1, MUTED, "restored after a restart");
        assert_eq!((fresh.saved, fresh.dormant), (None, Vec::new()));

        // no new cycle may overwrite that record while the list is unsaved
        fake.add(HEADSET, "headset", LIVE);
        fake.default = HEADSET;
        run(&mut machine, &mut fake, &[Event::Press(ms(5_000))]);
        assert_eq!(fake.get(HEADSET), Some(LIVE), "untouched");
        assert_eq!(fake.saved, Some(owed(MIC, MUTED)));
        assert!(matches!(fake.problems.last(), Some(Problem::Store(_))));
        // saved at last: the record is no longer needed, and the cycle starts
        fake.dormant_fails = false;
        run(&mut machine, &mut fake, &[Event::Press(ms(6_000))]);
        assert_eq!(fake.dormant, [dormant("mic", MUTED)]);
        assert_eq!(
            fake.dormant_at_set.last(),
            Some(&vec![dormant("mic", MUTED)])
        );
        assert_eq!(fake.get(HEADSET), Some(MUTED));
        assert_eq!(
            fake.saved.as_ref().map(|owed| (owed.pin, owed.was_muted)),
            Some((HEADSET, LIVE))
        );
    }

    #[test]
    fn a_fallback_record_goes_once_the_dormant_list_is_saved() {
        // also at startup, migrating a record from another server
        let mut fake = Fake::new(LIVE);
        fake.remove(MIC);
        fake.saved = Some(owed(MIC, MUTED));
        fake.dormant_fails = true;
        let mut machine = Machine::new(fake.saved.clone(), Vec::new());
        run(&mut machine, &mut fake, &[Event::ServerUp(SERVER + 1)]);
        assert_eq!(fake.saved, Some(owed(MIC, MUTED)), "kept");
        fake.dormant_fails = false;
        let new = Pin {
            server: SERVER + 1,
            serial: 3,
        };
        fake.add(new, "mic", LIVE);
        run(&mut machine, &mut fake, &[Event::SourcesChanged]);
        assert_eq!(fake.get(new), Some(MUTED));
        assert_eq!((fake.saved, fake.dormant), (None, Vec::new()));
    }

    #[test]
    fn a_taken_dormant_restore_not_saved_as_gone_is_saved_again_later() {
        let (mut machine, mut fake) = gone_mid_hold(MUTED);
        fake.add(BACK, "mic", LIVE);
        fake.default = BACK;
        fake.dormant_fails = true;
        run(&mut machine, &mut fake, &[Event::Press(ms(5_000))]);
        assert_eq!(fake.get(BACK), Some(LIVE), "engaged from the baseline");
        assert_eq!(fake.dormant, [dormant("mic", MUTED)], "stale on disk");
        fake.dormant_fails = false;
        run(&mut machine, &mut fake, &[Event::Release(ms(8_000))]);
        assert_eq!(fake.get(BACK), Some(MUTED));
        assert_eq!(
            (fake.saved.clone(), fake.dormant.clone()),
            (None, Vec::new())
        );
        // so a restart cannot bring it back over a later F14
        fake.external(BACK, LIVE);
        let (_, fresh) = restarted(&fake, LIVE);
        assert_eq!(fresh.nodes[0].1, LIVE, "left alone");
    }

    #[test]
    fn a_refused_flip_whose_dormant_restore_cannot_be_put_back_keeps_its_record() {
        let (mut machine, mut fake) = gone_mid_hold(MUTED);
        fake.add(BACK, "mic", LIVE);
        fake.default = BACK;
        fake.dormant_fails = true;
        fake.apply = Apply::Refuse;
        run(&mut machine, &mut fake, &[Event::Press(ms(5_000))]);
        let baseline = Obligation {
            pin: BACK,
            name: Some("mic".to_owned()),
            description: None,
            was_muted: MUTED,
        };
        assert_eq!(fake.saved, Some(baseline), "the fallback");
        // restart: both records name the same restore
        let (_, fresh) = restarted(&fake, LIVE);
        assert_eq!(fresh.nodes[0].1, MUTED);
        // or go on: it is still owed, and the record goes once it is saved
        fake.dormant_fails = false;
        fake.apply = Apply::Normally;
        run(&mut machine, &mut fake, &[Event::SourcesChanged]);
        assert_eq!(fake.get(BACK), Some(MUTED));
        assert_eq!((fake.saved, fake.dormant), (None, Vec::new()));
    }

    fn owes(owed: &[(&str, bool)]) -> BTreeMap<String, bool> {
        owed.iter()
            .map(|&(name, muted)| (name.to_owned(), muted))
            .collect()
    }

    #[test]
    fn what_is_owed_is_published_before_the_flip_and_until_its_restore_is_done() {
        let (mut machine, mut fake) = ready(MUTED);
        run(&mut machine, &mut fake, &[Event::Press(ms(1_000))]);
        assert_eq!(
            fake.published_at_set,
            [owes(&[("mic", MUTED)])],
            "before the flip"
        );
        assert_eq!(
            fake.saved_at_set,
            [Some(owed(MIC, MUTED))],
            "after the record"
        );
        fake.apply = Apply::Later;
        machine.handle(Event::Release(ms(3_000)), &mut fake);
        assert_eq!(fake.published.len(), 1, "until the restore is seen");
        fake.settle(&mut machine);
        assert_eq!(fake.published, [owes(&[("mic", MUTED)]), owes(&[])]);

        // stopping restores, and so ends it
        let (mut machine, mut fake) = ready(MUTED);
        run(
            &mut machine,
            &mut fake,
            &[
                Event::Press(ms(1_000)),
                Event::Release(ms(1_100)),
                Event::Stop,
            ],
        );
        assert_eq!(fake.published, [owes(&[("mic", MUTED)]), owes(&[])]);

        // a flip not sent owes nothing
        let (mut machine, mut fake) = ready(MUTED);
        fake.apply = Apply::Refuse;
        run(&mut machine, &mut fake, &[Event::Press(ms(1_000))]);
        assert_eq!(fake.published_at_set, [owes(&[("mic", MUTED)])]);
        assert_eq!(fake.published, [owes(&[("mic", MUTED)]), owes(&[])]);

        // a latch put back outside, and a fresh cycle from the same mute:
        // the same restore is owed throughout, so nothing changes
        let (mut machine, mut fake) = ready(MUTED);
        run(
            &mut machine,
            &mut fake,
            &[Event::Press(ms(1_000)), Event::Release(ms(1_100))],
        );
        fake.external(MIC, MUTED);
        run(&mut machine, &mut fake, &[Event::Press(ms(9_000))]);
        assert_eq!(fake.published, [owes(&[("mic", MUTED)])]);
    }

    #[test]
    fn a_restore_stays_published_while_its_mic_is_away_until_its_write_is_seen() {
        for gone in [Event::NodeGone(MIC), Event::ServerGone] {
            let (mut machine, mut fake) = ready(LIVE);
            run(&mut machine, &mut fake, &[Event::Press(ms(1_000))]);
            fake.remove(MIC);
            run(&mut machine, &mut fake, &[gone]);
            assert_eq!(fake.published, [owes(&[("mic", LIVE)])], "{gone:?}");
            // back (on the same server, or a new one), its write on the way
            let back = Pin {
                server: if gone == Event::ServerGone {
                    SERVER + 1
                } else {
                    SERVER
                },
                serial: 91,
            };
            fake.add(back, "mic", MUTED);
            fake.apply = Apply::Later;
            machine.handle(
                if gone == Event::ServerGone {
                    Event::ServerUp(SERVER + 1)
                } else {
                    Event::SourcesChanged
                },
                &mut fake,
            );
            assert_eq!(fake.published.len(), 1, "still owed: {gone:?}");
            fake.settle(&mut machine);
            assert_eq!(fake.get(back), Some(LIVE));
            assert_eq!(fake.published, [owes(&[("mic", LIVE)]), owes(&[])]);
        }
    }

    #[test]
    fn every_restore_owed_is_published_together() {
        let (mut machine, mut fake) = gone_mid_hold(MUTED);
        fake.add(HEADSET, "headset", LIVE);
        fake.default = HEADSET;
        run(&mut machine, &mut fake, &[Event::Press(ms(5_000))]);
        assert_eq!(
            fake.published_at_set.last(),
            Some(&owes(&[("headset", LIVE), ("mic", MUTED)]))
        );
        run(&mut machine, &mut fake, &[Event::Release(ms(7_000))]);
        assert_eq!(fake.published.last(), Some(&owes(&[("mic", MUTED)])));
        // a press on the returned mic owes the same as its dormant restore
        fake.add(BACK, "mic", LIVE);
        fake.default = BACK;
        let before = fake.published.len();
        run(&mut machine, &mut fake, &[Event::Press(ms(9_000))]);
        assert_eq!(fake.published.len(), before, "unchanged");
        run(&mut machine, &mut fake, &[Event::Release(ms(11_000))]);
        assert_eq!(fake.published.last(), Some(&owes(&[])));
    }

    #[test]
    fn what_the_saved_records_owe_is_known_before_the_server() {
        let mut machine = Machine::new(Some(owed(MIC, MUTED)), vec![dormant("headset", LIVE)]);
        assert_eq!(
            machine.owed(),
            owes(&[("headset", LIVE), ("mic", MUTED)]),
            "the caller publishes this at start"
        );
        let mut fake = Fake::new(LIVE);
        run(&mut machine, &mut fake, &[Event::ServerUp(SERVER)]);
        assert_eq!(fake.get(MIC), Some(MUTED));
        assert_eq!(fake.published, [owes(&[("headset", LIVE)])]);
        // a saved obligation outranks a dormant restore of the same mic, here
        // as at recovery
        let machine = Machine::new(Some(owed(MIC, MUTED)), vec![dormant("mic", LIVE)]);
        assert_eq!(machine.owed(), owes(&[("mic", MUTED)]));
        // a record without a name has nothing to say
        let unnamed = Obligation {
            name: None,
            ..owed(MIC, MUTED)
        };
        let mut machine = Machine::new(Some(unnamed), Vec::new());
        assert_eq!(machine.owed(), owes(&[]));
        let mut fake = Fake::new(LIVE);
        run(&mut machine, &mut fake, &[Event::ServerUp(SERVER)]);
        assert_eq!(fake.get(MIC), Some(MUTED));
        assert_eq!(fake.published, Vec::<BTreeMap<String, bool>>::new());
    }

    #[test]
    fn a_done_restore_not_dropped_from_disk_is_dropped_before_stopping() {
        // A and B dormant; A's restore is done, but saving [B] fails
        let (mut machine, mut fake) = gone_mid_hold(MUTED);
        fake.add(HEADSET, "headset", LIVE);
        fake.default = HEADSET;
        run(&mut machine, &mut fake, &[Event::Press(ms(5_000))]);
        fake.remove(HEADSET);
        run(&mut machine, &mut fake, &[Event::NodeGone(HEADSET)]);
        fake.add(BACK, "mic", LIVE);
        fake.dormant_fails = true;
        run(&mut machine, &mut fake, &[Event::SourcesChanged]);
        assert_eq!(fake.get(BACK), Some(MUTED));
        assert_eq!(
            fake.dormant,
            [dormant("mic", MUTED), dormant("headset", LIVE)],
            "still on disk"
        );
        assert_eq!(
            fake.problems.last(),
            Some(&Problem::NotRetired(dormant("mic", MUTED))),
            "said apart from a write not seen"
        );
        // storage is back; F14 sets the mic live; then talkd stops
        fake.dormant_fails = false;
        fake.external(BACK, LIVE);
        machine.handle(Event::Stop, &mut fake);
        assert!(machine.settled());
        assert_eq!(fake.dormant, [dormant("headset", LIVE)]);
        // the next start leaves the mic as F14 left it
        let (_, fresh) = restarted(&fake, LIVE);
        assert_eq!(fresh.nodes[0].1, LIVE);
        assert_eq!(fresh.dormant, [dormant("headset", LIVE)]);
    }

    #[test]
    fn stopping_waits_for_a_done_restore_to_be_dropped_from_disk() {
        let (mut machine, mut fake) = gone_mid_hold(MUTED);
        fake.add(BACK, "mic", LIVE);
        fake.dormant_fails = true;
        run(&mut machine, &mut fake, &[Event::SourcesChanged]);
        machine.handle(Event::Stop, &mut fake);
        assert!(!machine.settled(), "not while it would be done again");
        assert_eq!(fake.dormant, [dormant("mic", MUTED)]);
    }

    #[test]
    fn a_dormant_restore_moved_again_and_again_gets_only_its_tries() {
        let (mut machine, mut fake) = gone_mid_hold(MUTED);
        fake.add(BACK, "mic", LIVE);
        fake.apply = Apply::Drop;
        let sets = fake.saved_at_set.len();
        machine.handle(Event::SourcesChanged, &mut fake);
        for _ in 0..TRIES + 2 {
            let token = *fake.armed.last().unwrap();
            machine.handle(Event::Relocated(token, BACK), &mut fake);
        }
        assert_eq!(fake.saved_at_set.len() - sets, TRIES as usize);
        assert_eq!(
            fake.problems.last(),
            Some(&Problem::ReturnUnconfirmed(dormant("mic", MUTED)))
        );
        // nor does a deadline or settle add more while it stays
        fake.expire(&mut machine);
        run(&mut machine, &mut fake, &[Event::SourcesChanged]);
        assert_eq!(fake.saved_at_set.len() - sets, TRIES as usize);
        assert_eq!(fake.dormant, [dormant("mic", MUTED)], "still owed");
    }

    #[test]
    fn a_press_while_the_revived_node_refreshes_waits_rather_than_take_the_older() {
        let (mut machine, mut fake) = gone_mid_hold(MUTED);
        let newer = Pin {
            server: SERVER,
            serial: 95,
        };
        fake.add(BACK, "mic", LIVE);
        fake.add(newer, "mic", LIVE);
        fake.default = newer;
        fake.apply = Apply::Later;
        machine.handle(Event::SourcesChanged, &mut fake);
        assert_eq!(fake.sent.last().map(|&(_, pin, _)| pin), Some(newer));
        // its readback under way: no mute to read on it for now
        fake.observed.retain(|&(pin, _)| pin != newer);
        let sets = fake.saved_at_set.len();
        machine.handle(Event::Press(ms(5_000)), &mut fake);
        assert_eq!(fake.problems.last(), Some(&Problem::NoDefaultSource));
        assert_eq!(fake.saved_at_set.len(), sets, "BACK untouched");
        assert_eq!(fake.dormant, [dormant("mic", MUTED)], "still the newer's");
        fake.settle(&mut machine);
        assert_eq!(fake.get(newer), Some(MUTED));
        assert_eq!(fake.get(BACK), Some(LIVE));
        assert_eq!(fake.dormant, []);
        // pressed again, the newer one: a cycle of its own
        run(&mut machine, &mut fake, &[Event::Press(ms(6_000))]);
        assert_eq!(
            fake.saved.as_ref().map(|owed| (owed.pin, owed.was_muted)),
            Some((newer, MUTED))
        );
    }

    #[test]
    fn a_replacement_already_there_when_the_old_node_goes_is_revived() {
        let (mut machine, mut fake) = gone_mid_hold(MUTED);
        fake.add(BACK, "mic", LIVE);
        fake.apply = Apply::Later;
        machine.handle(Event::SourcesChanged, &mut fake);
        // the replacement comes and settles while that write is out
        let newer = Pin {
            server: SERVER,
            serial: 95,
        };
        fake.add(newer, "mic", LIVE);
        machine.handle(Event::SourcesChanged, &mut fake);
        assert_eq!(fake.sent.len(), 1, "one write at a time");
        // then the old node goes, its write with it; nothing else comes
        fake.remove(BACK);
        machine.handle(Event::NodeGone(BACK), &mut fake);
        assert_eq!(fake.sent.last().map(|&(_, pin, _)| pin), Some(newer));
        fake.settle(&mut machine);
        assert_eq!(fake.get(newer), Some(MUTED));
        assert_eq!(fake.dormant, []);
    }

    #[test]
    fn a_revival_and_a_press_take_the_newest_source_of_a_name() {
        // the old node lingers while the new one comes up
        let (mut machine, mut fake) = gone_mid_hold(MUTED);
        let newer = Pin {
            server: SERVER,
            serial: 95,
        };
        fake.add(BACK, "mic", LIVE);
        fake.add(newer, "mic", LIVE);
        fake.apply = Apply::Later;
        machine.handle(Event::SourcesChanged, &mut fake);
        assert_eq!(fake.sent.last().map(|&(_, pin, _)| pin), Some(newer));
        // the press on that mic (the mic layer picks the same one)
        fake.default = newer;
        machine.handle(Event::Press(ms(5_000)), &mut fake);
        fake.settle(&mut machine);
        assert_eq!(fake.get(newer), Some(LIVE), "flipped from the baseline");
        run(&mut machine, &mut fake, &[Event::Release(ms(8_000))]);
        assert_eq!(fake.get(newer), Some(MUTED));
        assert_eq!(fake.dormant, []);
    }

    #[test]
    fn stopping_leaves_dormant_restores_for_the_next_start() {
        let (mut machine, mut fake) = gone_mid_hold(MUTED);
        machine.handle(Event::Stop, &mut fake);
        assert!(machine.settled());
        assert_eq!(fake.dormant, [dormant("mic", MUTED)]);
        // the next start
        let mut machine = Machine::new(None, fake.dormant.clone());
        fake.add(BACK, "mic", LIVE);
        run(&mut machine, &mut fake, &[Event::ServerUp(SERVER)]);
        assert_eq!(fake.get(BACK), Some(MUTED));
        assert_eq!(fake.dormant, []);
    }

    #[test]
    fn a_locked_session_ends_cycles_but_never_starts_one() {
        let (mut machine, mut fake) = ready(MUTED);
        run(
            &mut machine,
            &mut fake,
            &[Event::Press(ms(1_000)), Event::Release(ms(1_100))],
        );
        fake.lock = Lock::Locked;
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
        fake.lock = Lock::Unlocked;
        run(
            &mut machine,
            &mut fake,
            &[Event::Press(ms(7_000)), Event::Release(ms(7_100))],
        );
        fake.lock = Lock::Unknown;
        fake.external(MIC, MUTED);
        run(&mut machine, &mut fake, &[Event::Press(ms(8_000))]);
        assert_eq!(fake.get(MIC), Some(MUTED));
        assert_eq!(fake.saved, None);
        assert_eq!(fake.problems.last(), Some(&Problem::Locked(Lock::Unknown)));
    }

    #[test]
    fn the_record_is_saved_before_the_mic_changes_and_kept_until_confirmed() {
        let (mut machine, mut fake) = ready(MUTED);
        let owed = Some(owed(MIC, MUTED));
        machine.handle(Event::Press(ms(1_000)), &mut fake);
        assert_eq!(
            fake.saved_at_set,
            std::slice::from_ref(&owed),
            "saved first"
        );
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
        assert_eq!(fake.recoveries.len(), 0);
        // the next press retries it
        fake.apply = Apply::Normally;
        run(&mut machine, &mut fake, &[Event::Press(ms(9_000))]);
        assert_eq!(fake.get(MIC), Some(MUTED));
        assert_eq!(fake.saved, None);
        assert_eq!(fake.recoveries.len(), 1);
    }

    #[test]
    fn a_late_confirmation_still_completes_the_restore() {
        let (mut machine, mut fake) = ready(MUTED);
        run(&mut machine, &mut fake, &[Event::Press(ms(1_000))]);
        fake.apply = Apply::Silently;
        run(&mut machine, &mut fake, &[Event::Release(ms(3_000))]);
        fake.expire(&mut machine);
        assert!(fake.saved.is_some());
        let token = machine.pending.unwrap().token;
        fake.confirm(&mut machine, token, MIC, MUTED);
        assert_eq!(fake.saved, None);
        assert_eq!(fake.recoveries.len(), 1);
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
        let owed = owed(MIC, MUTED);
        // same server, node there, still flipped: restored
        let mut fake = Fake::new(LIVE);
        fake.saved = Some(owed.clone());
        let mut machine = Machine::new(Some(owed.clone()), Vec::new());
        machine.handle(Event::Press(ms(1)), &mut fake);
        assert_eq!(fake.problems, [Problem::NotReady], "not before the server");
        run(&mut machine, &mut fake, &[Event::ServerUp(SERVER)]);
        assert_eq!(fake.get(MIC), Some(MUTED));
        assert_eq!(fake.saved, None);

        // Already put back: fresh readback still completes recovery.
        let mut fake = Fake::new(MUTED);
        fake.saved = Some(owed.clone());
        let mut machine = Machine::new(Some(owed.clone()), Vec::new());
        run(&mut machine, &mut fake, &[Event::ServerUp(SERVER)]);
        assert_eq!(fake.saved, None);

        // a record without a name (talkd-obligation-v1) from another server,
        // or whose node is gone: dropped, with a report
        let unnamed = Obligation { name: None, ..owed };
        for (server, nodes) in [(SERVER + 1, vec![(MIC, LIVE)]), (SERVER, Vec::new())] {
            let mut fake = Fake::new(LIVE);
            fake.nodes = nodes.clone();
            fake.saved = Some(unnamed.clone());
            let mut machine = Machine::new(Some(unnamed.clone()), Vec::new());
            run(&mut machine, &mut fake, &[Event::ServerUp(server)]);
            assert_eq!(fake.saved, None);
            assert_eq!(fake.saved_at_set.len(), 0, "never touched");
            assert_eq!(fake.dormant, []);
            assert!(matches!(fake.problems[..], [Problem::Stale(_)]));
        }
        // one there is restored, and voided as before if its node goes
        let mut fake = Fake::new(LIVE);
        fake.apply = Apply::Drop;
        let mut machine = Machine::new(Some(unnamed.clone()), Vec::new());
        run(&mut machine, &mut fake, &[Event::ServerUp(SERVER)]);
        fake.remove(MIC);
        run(&mut machine, &mut fake, &[Event::NodeGone(MIC)]);
        assert_eq!((fake.saved, fake.dormant), (None, Vec::new()));
        assert_eq!(fake.problems, [Problem::NodeGone]);
    }

    #[test]
    fn startup_turns_a_named_record_whose_node_is_gone_into_a_dormant_restore() {
        let owed = owed(MIC, MUTED);
        // PipeWire restarted, the mic there under a new serial: restored
        let new = Pin {
            server: SERVER + 1,
            serial: 3,
        };
        let mut fake = Fake::new(LIVE);
        fake.nodes.clear();
        fake.observed.clear();
        fake.add(new, "mic", LIVE);
        fake.saved = Some(owed.clone());
        let mut machine = Machine::new(Some(owed.clone()), Vec::new());
        run(&mut machine, &mut fake, &[Event::ServerUp(SERVER + 1)]);
        assert_eq!(fake.get(new), Some(MUTED));
        assert_eq!(
            (fake.saved.clone(), fake.dormant.clone()),
            (None, Vec::new())
        );
        assert!(fake.problems.is_empty(), "{:?}", fake.problems);

        // same server, node gone, no mic of that name yet: dormant, reported
        let mut fake = Fake::new(LIVE);
        fake.remove(MIC);
        fake.saved = Some(owed.clone());
        let mut machine = Machine::new(Some(owed.clone()), Vec::new());
        run(&mut machine, &mut fake, &[Event::ServerUp(SERVER)]);
        assert_eq!(fake.saved, None);
        assert_eq!(fake.dormant, [dormant("mic", MUTED)]);
        assert_eq!(
            fake.problems,
            [Problem::Awaiting {
                owed: dormant("mic", MUTED),
                server_gone: false
            }]
        );
        fake.add(BACK, "mic", LIVE);
        run(&mut machine, &mut fake, &[Event::SourcesChanged]);
        assert_eq!(fake.get(BACK), Some(MUTED));
        assert_eq!(fake.dormant, []);

        // a saved obligation outranks a dormant restore of the same mic
        let mut fake = Fake::new(LIVE);
        fake.saved = Some(owed.clone());
        fake.dormant = vec![dormant("mic", LIVE)];
        let mut machine = Machine::new(Some(owed.clone()), fake.dormant.clone());
        run(&mut machine, &mut fake, &[Event::ServerUp(SERVER)]);
        assert_eq!(fake.get(MIC), Some(MUTED));
        assert_eq!((fake.saved, fake.dormant), (None, Vec::new()));
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
            [(1, MIC, LIVE), (2, MIC, MUTED)],
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
            [(1, MIC, LIVE), (2, MIC, MUTED)],
            "a restore, not a fresh flip"
        );
        fake.settle(&mut machine);
        assert_eq!((fake.get(MIC), fake.saved), (Some(MUTED), None));
    }

    #[test]
    fn a_dropped_flip_can_be_restored_and_confirmed_without_a_value_change() {
        let (mut machine, mut fake) = ready(MUTED);
        fake.apply = Apply::Drop;
        run(&mut machine, &mut fake, &[Event::Press(ms(1_000))]);
        fake.expire(&mut machine);
        fake.apply = Apply::Normally;
        run(&mut machine, &mut fake, &[Event::Release(ms(3_000))]);
        assert_eq!(fake.get(MIC), Some(MUTED));
        assert_eq!(
            fake.saved, None,
            "fresh readback proves an unchanged restore too"
        );
        assert_eq!(fake.problems, [Problem::EngageUnconfirmed]);
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
    fn a_restore_deadline_never_proves_a_queued_flip_finished() {
        for stop in [true, false] {
            let (mut machine, mut fake) = ready(MUTED);
            fake.apply = Apply::Later;
            machine.handle(Event::Press(ms(1_000)), &mut fake);
            machine.handle(Event::Release(ms(2_000)), &mut fake);
            fake.expire(&mut machine);
            fake.lock = Lock::Locked;
            machine.handle(
                if stop {
                    Event::Stop
                } else {
                    Event::Press(ms(3_000))
                },
                &mut fake,
            );
            assert_eq!(fake.saved, Some(owed(MIC, MUTED)));
            assert_eq!(
                fake.get(MIC),
                Some(MUTED),
                "the old observation proves nothing"
            );
            fake.settle(&mut machine);
            assert_eq!(
                fake.saved, None,
                "only request-scoped readback completes it"
            );
            assert_eq!(fake.get(MIC), Some(MUTED));
        }
    }

    #[test]
    fn stale_or_mismatched_confirmations_cannot_clear_a_restore() {
        let (mut machine, mut fake) = ready(MUTED);
        fake.apply = Apply::Later;
        machine.handle(Event::Press(ms(1_000)), &mut fake);
        let flip = machine.pending.unwrap().token;
        machine.handle(Event::Release(ms(2_000)), &mut fake);
        let restore = machine.pending.unwrap().token;
        for event in [
            Event::Confirmed(flip, MIC, MUTED),
            Event::Confirmed(restore, HEADSET, MUTED),
            Event::Confirmed(restore, MIC, LIVE),
        ] {
            machine.handle(event, &mut fake);
            assert!(fake.saved.is_some());
        }
        fake.expire(&mut machine);
        machine.handle(Event::Confirmed(restore, MIC, MUTED), &mut fake);
        assert_eq!(
            fake.saved, None,
            "a late but fresh matching readback is sufficient"
        );
    }

    #[test]
    fn silent_server_changes_are_not_client_observations() {
        let (mut machine, mut fake) = ready(MUTED);
        fake.apply = Apply::Silently;
        machine.handle(Event::Press(ms(1_000)), &mut fake);
        assert_eq!(fake.get(MIC), Some(LIVE), "server applied it");
        assert_eq!(
            fake.muted(MIC),
            Some(MUTED),
            "client has no fresh observation"
        );
        machine.handle(Event::Release(ms(2_000)), &mut fake);
        fake.expire(&mut machine);
        assert!(
            fake.saved.is_some(),
            "neither a cached value nor timeout confirms"
        );
    }

    #[test]
    fn reader_overflow_never_turns_an_uncertain_hold_into_a_latch() {
        use crate::input::{self, Key};
        fn raw(kind: u16, code: u16, value: i32, at: u64) -> evdev::InputEvent {
            nix::libc::input_event {
                time: nix::libc::timeval {
                    tv_sec: (at / 1000) as _,
                    tv_usec: ((at % 1000) * 1000) as _,
                },
                type_: kind,
                code,
                value,
            }
            .into()
        }
        for seen_press in [false, true] {
            let (mut machine, mut fake) = ready(MUTED);
            let mut reader = input::F13::default();
            let mut feed = |reader: &mut input::F13, events: Vec<evdev::InputEvent>| {
                input::read_batch(reader, events.into_iter(), |key| {
                    let event = match key {
                        Key::Press(at) => Event::Press(at),
                        Key::Release(at) => Event::Release(at),
                        Key::Lost => Event::DeviceLost,
                    };
                    machine.handle(event, &mut fake);
                    fake.settle(&mut machine);
                })
            };
            if seen_press {
                feed(&mut reader, vec![raw(1, 183, 1, 1_000)]);
            }
            assert!(feed(
                &mut reader,
                vec![raw(0, 3, 0, 2_000), raw(0, 0, 0, 2_000)]
            ));
            reader.resync(true); // physically held for a second, not a new press
            feed(&mut reader, vec![raw(1, 183, 0, 2_020)]);
            assert_eq!(fake.get(MIC), Some(MUTED));
            assert_eq!(fake.saved, None);
        }
    }

    #[test]
    fn an_unavailable_saved_target_retains_and_reports_its_restore() {
        let owed = owed(MIC, MUTED);
        let mut fake = Fake::new(LIVE);
        fake.saved = Some(owed.clone());
        fake.observed.clear(); // node exists, but its mixer snapshot failed
        fake.apply = Apply::Refuse;
        let mut machine = Machine::new(Some(owed.clone()), Vec::new());
        machine.handle(Event::ServerUp(SERVER), &mut fake);
        assert_eq!(fake.saved, Some(owed));
        assert!(matches!(
            fake.problems.as_slice(),
            [Problem::RestoreFailed(_)]
        ));
        fake.observe(MIC, LIVE);
        fake.apply = Apply::Normally;
        fake.lock = Lock::Locked;
        run(&mut machine, &mut fake, &[Event::Press(ms(4_000))]);
        assert_eq!(fake.get(MIC), Some(MUTED));
        assert_eq!(fake.saved, None, "a later usable snapshot permits retry");
    }

    #[test]
    fn only_restore_troubles_are_visible() {
        assert!(Problem::RestoreUnconfirmed.visible());
        assert!(Problem::NodeGone.visible());
        assert!(Problem::ServerGone.visible());
        assert!(
            Problem::Awaiting {
                owed: dormant("mic", MUTED),
                server_gone: true
            }
            .visible()
        );
        assert!(Problem::ReturnUnconfirmed(dormant("mic", MUTED)).visible());
        assert!(Problem::NotRetired(dormant("mic", MUTED)).visible());
        assert_eq!(
            Problem::NotRetired(dormant("mic", MUTED)).subject(),
            Some("mic")
        );
        assert_eq!(Problem::RestoreUnconfirmed.subject(), None);
        assert!(!Problem::Locked(Lock::Locked).visible());
        assert!(!Problem::EngageUnconfirmed.visible());
    }
}
