//! Foot pedal -> Voyager layers.
//!
//! The PCsensor 3-pedal footswitch (3553:b001) sends its factory keys a, b,
//! c. This grabs it exclusively, so those never reach applications, and makes
//! each pedal hold a layer of the Voyager's own firmware, through its raw-HID
//! interface in ZSA's Oryx protocol (zsa/qmk_modules, oryx/oryx.c): SET_LAYER
//! on (layer_move) on press, SET_LAYER off (layer_off) on release. A tap on
//! the mirror or mouse pedal locks its layer until the next tap. Each batch
//! of commands sits between two pairings and counts as applied once the
//! firmware has answered both, which is best effort ([`Answers`]). Nothing on
//! the host remaps Voyager keystrokes, and keymapp is not involved. A worker
//! thread owns the keyboard, so a missing or replugged Voyager never delays
//! reading the pedal.

use std::collections::VecDeque;
use std::convert::Infallible;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::fd::{AsFd, AsRawFd};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant, UNIX_EPOCH};

use evdev::raw_stream::RawDevice;
use evdev::{InputEvent, KeyCode};
use nix::errno::Errno;
use nix::poll::{PollFd, PollFlags, PollTimeout, poll};
use nix::sys::signal::{SigSet, Signal};
use nix::time::{ClockId, clock_gettime};
use parking_lot::Mutex;

/// Layer indices of Oryx layout JqpmQ ("Colemak dh nicc"). Reordering layers
/// in Oryx means updating these.
const NAV: u8 = 4;
const MOUSE: u8 = 5;
const MIRROR: u8 = 6;

/// A pedal: its factory key, the layer it holds, and whether a tap locks it.
#[derive(Clone, Copy)]
struct Pedal {
    key: KeyCode,
    layer: u8,
    name: &'static str,
    lockable: bool,
}

/// Nav sits under the resting foot, as it is entered between bursts of
/// typing; mirror and mouse are sessions, so a tap locks them.
const PEDALS: [Pedal; 3] = [
    Pedal {
        key: KeyCode::KEY_A, // left
        layer: MIRROR,
        name: "mirror",
        lockable: true,
    },
    Pedal {
        key: KeyCode::KEY_B, // middle
        layer: NAV,
        name: "nav",
        lockable: false,
    },
    Pedal {
        key: KeyCode::KEY_C, // right
        layer: MOUSE,
        name: "mouse",
        lockable: true,
    },
];

/// A press shorter than this, with no key typed during it, is a tap.
const TAP: Duration = Duration::from_millis(250);

/// A Voyager key report arriving this long after a tap's release still
/// counts as typed during it: it may have been on its way.
const GRACE: Duration = Duration::from_millis(40);

const PEDAL: &str = "/dev/input/by-id/usb-PCsensor_FootSwitch-event-kbd";

/// A pedal down, and when it went down (CLOCK_BOOTTIME). None if it was read
/// rather than seen going down (at open, after a drop), or if it took a lock
/// over: such a press is never a tap.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Press {
    key: KeyCode,
    seen: Option<Duration>,
}

/// Which pedals are down, which is locked, and the layer that asks for.
/// No I/O.
struct Pedals {
    map: &'static [Pedal],
    held: Vec<Press>, // in press order
    locked: Option<KeyCode>,
}

impl Pedals {
    fn new(map: &'static [Pedal]) -> Self {
        Pedals {
            map,
            held: Vec::new(),
            locked: None,
        }
    }

    fn pedal(&self, key: KeyCode) -> Option<&Pedal> {
        self.map.iter().find(|pedal| pedal.key == key)
    }

    fn is_down(&self, key: KeyCode) -> bool {
        self.held.iter().any(|press| press.key == key)
    }

    fn press(&mut self, key: KeyCode, at: Duration) {
        if self.pedal(key).is_some() && !self.is_down(key) {
            let seen = (!self.take_over(key)).then_some(at);
            self.held.push(Press { key, seen });
        }
    }

    /// Pressing the locked pedal turns its lock into a hold, which its
    /// release ends. True if it was locked.
    fn take_over(&mut self, key: KeyCode) -> bool {
        let locked = self.locked == Some(key);
        if locked {
            self.locked = None;
        }
        locked
    }

    /// A tap unlocks whatever is locked, or else locks a lockable pedal.
    /// `typed(from, to)` says whether a Voyager key went down in that span;
    /// it is asked only when the answer can lock or unlock.
    fn release(
        &mut self,
        key: KeyCode,
        at: Duration,
        typed: impl FnOnce(Duration, Duration) -> bool,
    ) {
        let Some(index) = self.held.iter().position(|press| press.key == key) else {
            return;
        };
        // a release timed before its press makes no tap either
        let short = |start: &Duration| at.checked_sub(*start).is_some_and(|held| held < TAP);
        let Some(start) = self.held.remove(index).seen.filter(short) else {
            return;
        };
        let lockable = self.pedal(key).is_some_and(|pedal| pedal.lockable);
        if (self.locked.is_none() && !lockable) || typed(start, at) {
            return;
        }
        if self.locked.take().is_none() && lockable {
            self.locked = Some(key);
        }
    }

    /// Adopt the device's real key state: at open and after a drop. A lock
    /// stays unless its pedal is found down, which takes it over. No press is
    /// a tap after this: a drop may have hidden a release and a new press of
    /// a pedal that is still down.
    fn sync(&mut self, down: &[KeyCode]) {
        self.held.retain(|press| down.contains(&press.key));
        for press in &mut self.held {
            press.seen = None;
        }
        let mut found: Vec<KeyCode> = down
            .iter()
            .copied()
            .filter(|&key| self.pedal(key).is_some() && !self.is_down(key))
            .collect();
        found.sort(); // their press order is unknown
        for key in found {
            self.take_over(key);
            self.held.push(Press { key, seen: None });
        }
    }

    /// The pedal is gone: nothing is down, and nothing could unlock a lock.
    fn lost(&mut self) {
        self.held.clear();
        self.locked = None;
    }

    /// Layer of the latest pedal down, or else of the locked one.
    fn layer(&self) -> Option<u8> {
        let key = self.held.last().map(|press| press.key).or(self.locked);
        key.and_then(|key| self.pedal(key)).map(|pedal| pedal.layer)
    }

    fn locked_name(&self) -> Option<&'static str> {
        let pedal = self.locked.and_then(|key| self.pedal(key));
        pedal.map(|pedal| pedal.name)
    }
}

/// What one raw pedal event means. After SYN_DROPPED the kernel's rule
/// (Documentation/input/event-codes.rst) is to discard everything up to the
/// next SYN_REPORT and then read the device's state, which is what Resync
/// asks for, without waiting for any further event.
#[derive(Debug, PartialEq)]
enum Step {
    Key(KeyCode, i32),
    Resync,
    Nothing,
}

const EV_SYN: u16 = 0;
const EV_KEY: u16 = 1;
const SYN_REPORT: u16 = 0;
const SYN_DROPPED: u16 = 3;

#[derive(Default)]
struct Dropping(bool);

impl Dropping {
    fn step(&mut self, kind: u16, code: u16, value: i32) -> Step {
        match (kind, code) {
            (EV_SYN, SYN_DROPPED) => {
                self.0 = true;
                Step::Nothing
            }
            (EV_SYN, SYN_REPORT) if self.0 => {
                self.0 = false;
                Step::Resync
            }
            _ if self.0 => Step::Nothing,
            (EV_KEY, _) if value == 0 || value == 1 => Step::Key(KeyCode::new(code), value),
            _ => Step::Nothing, // autorepeat, scan codes, reports
        }
    }
}

/// Feeds one fetched batch of raw events to `pedals`. A resync reads the
/// device's key state (EVIOCGKEY), which also flushes the key events the
/// kernel still queues for this reader; the rest of the batch is older than
/// that state, so it goes too. `typed` answers for [`Pedals::release`].
fn take_batch(
    events: impl IntoIterator<Item = InputEvent>,
    dropping: &mut Dropping,
    pedals: &mut Pedals,
    typed: &impl Fn(Duration, Duration) -> bool,
    held: impl FnOnce() -> io::Result<Vec<KeyCode>>,
) -> io::Result<()> {
    for event in events {
        // the kernel's time for the event, on CLOCK_BOOTTIME (use_boottime),
        // which evdev hands over as a time since the epoch
        let at = event
            .timestamp()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        match dropping.step(event.event_type().0, event.code(), event.value()) {
            Step::Key(code, 1) => pedals.press(code, at),
            Step::Key(code, _) => pedals.release(code, at, typed),
            Step::Resync => {
                pedals.sync(&held()?);
                break;
            }
            Step::Nothing => {}
        }
    }
    Ok(())
}

/// When the keyboard worker read reports of Voyager keys going down, on
/// CLOCK_BOOTTIME: tells a pedal tap from a press typed under. Best effort:
/// it holds only what the firmware sends while paired, read within the grace
/// after a release.
#[derive(Default)]
struct KeyLog(Mutex<VecDeque<Duration>>);

impl KeyLog {
    /// Far more than a pedal press could have typed under it.
    const KEPT: usize = 256;

    fn note(&self, at: Duration) {
        let mut times = self.0.lock();
        if times.len() == Self::KEPT {
            times.pop_front();
        }
        times.push_back(at);
    }

    /// Whether a key went down from `from` to [`GRACE`] past `to`. Unless one
    /// already has, waits out that grace first, so a report on its way is
    /// counted; the rest of the pedal's batch waits with it.
    fn typed(&self, from: Duration, to: Duration) -> bool {
        self.typed_waiting(from, to, boottime(), thread::sleep)
    }

    fn typed_waiting(
        &self,
        from: Duration,
        to: Duration,
        now: Option<Duration>,
        wait: impl FnOnce(Duration),
    ) -> bool {
        let until = to + GRACE;
        let seen = || self.0.lock().iter().any(|at| (from..=until).contains(at));
        if seen() {
            return true;
        }
        let left = now.and_then(|now| until.checked_sub(now));
        match left.filter(|left| !left.is_zero()) {
            Some(left) => {
                wait(left.min(GRACE));
                seen()
            }
            None => false,
        }
    }
}

fn boottime() -> Option<Duration> {
    clock_gettime(ClockId::CLOCK_BOOTTIME)
        .ok()
        .map(Duration::from)
}

nix::ioctl_write_ptr!(eviocsclockid, b'E', 0xa0, nix::libc::c_int);

/// Puts the pedal's event times on CLOCK_BOOTTIME, the clock key reports are
/// timed on: it never steps, and it counts a suspend between a press and its
/// release.
fn use_boottime(device: &RawDevice) -> io::Result<()> {
    let clock = nix::libc::CLOCK_BOOTTIME;
    // SAFETY: EVIOCSCLOCKID reads one int through the pointer, which is valid
    // for the call, from the device's own open fd.
    unsafe { eviocsclockid(device.as_raw_fd(), &clock) }
        .map(drop)
        .map_err(io::Error::from)
}

/// What the firmware's pedal layers are known to be.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Applied {
    /// Just paired (a previous run may have died mid-hold), or a batch
    /// failed after any of it may have run.
    Unknown,
    /// No pedal layer on.
    Off,
    On(u8),
}

/// What the worker knows of the firmware's pedal layers.
struct Knowledge {
    applied: Applied,
    reached: bool, // the keyboard, at least once
}

impl Knowledge {
    fn new() -> Self {
        Knowledge {
            applied: Applied::Unknown,
            reached: false,
        }
    }

    fn paired(&mut self) {
        self.applied = Applied::Unknown;
        self.reached = true;
    }

    /// The firmware answered the batch for `wanted`.
    fn confirmed(&mut self, wanted: Option<u8>) {
        self.applied = wanted.map_or(Applied::Off, Applied::On);
    }

    fn failed(&mut self) {
        self.applied = Applied::Unknown;
    }

    /// Stopping without the keyboard: are the pedal layers known to be off?
    /// A run that never reached it turned nothing on.
    fn off(&self) -> bool {
        !self.reached || self.applied == Applied::Off
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Command {
    Set(u8),
    Unset(u8),
}

/// Commands that take the firmware from `applied` to `wanted`.
fn plan(applied: Applied, wanted: Option<u8>, owned: &[u8]) -> Vec<Command> {
    match (applied, wanted) {
        (Applied::On(on), Some(layer)) if on == layer => Vec::new(),
        (_, Some(layer)) => vec![Command::Set(layer)],
        (Applied::Off, None) => Vec::new(),
        (Applied::On(on), None) => vec![Command::Unset(on)],
        (Applied::Unknown, None) => owned.iter().map(|&layer| Command::Unset(layer)).collect(),
    }
}

/// ZSA's Oryx raw-HID protocol (zsa/qmk_modules oryx/oryx.h): reports of 32
/// bytes, command or event code first.
mod oryx {
    pub const PAIRING_INIT: u8 = 1;
    pub const SET_LAYER: u8 = 4;
    pub const GET_PROTOCOL_VERSION: u8 = 0xFE;
    pub const EVT_PAIRING_SUCCESS: u8 = 4;
    pub const EVT_LAYER: u8 = 5;
    pub const EVT_KEYDOWN: u8 = 6;
    pub const EVT_PROTOCOL_VERSION: u8 = 0xFE;
    /// Versions with this SET_LAYER: 4 (firmware 25) and 5 (adds auto-mouse).
    pub const KNOWN_VERSIONS: [u8; 2] = [4, 5];
    pub const REPORT: usize = 32;

    /// A host-to-keyboard report as hidraw takes it: report number 0, then
    /// the payload, zero-padded.
    pub fn report(payload: &[u8]) -> [u8; REPORT + 1] {
        let mut out = [0; REPORT + 1];
        out[1..=payload.len()].copy_from_slice(payload);
        out
    }
}

/// Reports for one batch of commands, between two pairings. The firmware
/// ignores SET_LAYER unless paired, and can drop the pairing on its own (a
/// failed send, e.g. across a USB suspend) without telling the host: the
/// opening pairing gets the commands past that check. Writes are not
/// acknowledged: the closing pairing's answer is the acknowledgement, as the
/// firmware handles reports in order ([`Answers`]).
fn batch(commands: &[Command]) -> Vec<[u8; oryx::REPORT + 1]> {
    if commands.is_empty() {
        return Vec::new();
    }
    let pairing = oryx::report(&[oryx::PAIRING_INIT]);
    let mut reports = vec![pairing];
    reports.extend(commands.iter().map(|command| match *command {
        Command::Set(layer) => oryx::report(&[oryx::SET_LAYER, 1, layer]),
        Command::Unset(layer) => oryx::report(&[oryx::SET_LAYER, 0, layer]),
    }));
    reports.push(pairing);
    reports
}

/// How far the firmware's answers to a batch have got. A pairing is answered
/// with PAIRING_SUCCESS and then a layer snapshot, and either send failing
/// drops the pairing again (oryx.c, raw_hid_send_oryx). So the opening
/// pairing's success must be directly followed by its snapshot (paired at
/// that point), and then the closing pairing's success shows that the
/// firmware got through every report before it. Key positions, layer
/// changes and a snapshot left over from before are skipped.
///
/// Best effort, not proof. A batch spanning a USB suspend or reset (either
/// empties the firmware's queue of received reports: usb_main.c), or one
/// during which the pairing drops (any failed send unpairs, key positions
/// included), can be acknowledged with a command lost or ignored. No bound
/// on the firmware's dispatch delay argues that away: its main loop also
/// flushes the RGB LEDs over I2C and can wait on a full IN queue. A pedal
/// layer can then stay on with no pedal down, and a stop report it off; a
/// pedal press the worker applies, then its release, or a restart of the
/// service clears it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum Answers {
    #[default]
    Nothing,
    Paired,
    StillPaired,
    Confirmed,
}

impl Answers {
    fn step(self, event: u8) -> Self {
        match (self, event) {
            (Answers::Confirmed, _) => Answers::Confirmed,
            (Answers::StillPaired, oryx::EVT_PAIRING_SUCCESS) => Answers::Confirmed,
            (Answers::StillPaired, _) => Answers::StillPaired,
            (Answers::Paired, oryx::EVT_LAYER) => Answers::StillPaired,
            (_, oryx::EVT_PAIRING_SUCCESS) => Answers::Paired,
            _ => Answers::Nothing,
        }
    }
}

/// Is this hidraw node a Voyager's Oryx interface? Its USB ids, and QMK's
/// raw-HID usage page 0xFF60 (descriptor starts 06 60 FF). The
/// /dev/input/by-id names would not do: they embed the firmware revision.
fn is_voyager_oryx(uevent: &str, descriptor: &[u8]) -> bool {
    uevent
        .lines()
        .any(|line| line.starts_with("HID_ID=") && line.ends_with(":00003297:00001977"))
        && descriptor.starts_with(&[0x06, 0x60, 0xFF])
}

fn find_keyboard() -> io::Result<PathBuf> {
    let mut found = Vec::new();
    for entry in fs::read_dir("/sys/class/hidraw")? {
        let dir = entry?.path();
        let uevent = fs::read_to_string(dir.join("device/uevent")).unwrap_or_default();
        let descriptor = fs::read(dir.join("device/report_descriptor")).unwrap_or_default();
        if let (true, Some(name)) = (is_voyager_oryx(&uevent, &descriptor), dir.file_name()) {
            found.push(PathBuf::from("/dev").join(name));
        }
    }
    match found.len() {
        1 => Ok(found.remove(0)),
        0 => Err(io::Error::new(
            io::ErrorKind::NotFound,
            "no Voyager attached",
        )),
        n => Err(io::Error::other(format!(
            "{n} Voyagers attached; not guessing which one the pedal is for"
        ))),
    }
}

fn readable(fd: &impl AsFd, timeout: PollTimeout) -> io::Result<bool> {
    let mut fds = [PollFd::new(fd.as_fd(), PollFlags::POLLIN)];
    Ok(poll(&mut fds, timeout)? > 0)
}

/// The Voyager's Oryx interface, open and checked.
struct Keyboard {
    hid: File,
    path: PathBuf,
    version: u8,
    /// When the Voyager reported keys going down (see [`KeyLog`]).
    keys: Arc<KeyLog>,
}

impl Keyboard {
    fn connect(keys: Arc<KeyLog>) -> io::Result<Self> {
        let path = find_keyboard()?;
        let hid = OpenOptions::new().read(true).write(true).open(&path)?;
        let mut keyboard = Keyboard {
            hid,
            path,
            version: 0,
            keys,
        };
        keyboard.send(&oryx::report(&[oryx::GET_PROTOCOL_VERSION]))?;
        keyboard.version = keyboard.reply(oryx::EVT_PROTOCOL_VERSION)?[1];
        if !oryx::KNOWN_VERSIONS.contains(&keyboard.version) {
            return Err(io::Error::other(format!(
                "Oryx protocol version {} is untested; layer pedals stay off",
                keyboard.version
            )));
        }
        keyboard.send(&oryx::report(&[oryx::PAIRING_INIT]))?;
        keyboard.reply(oryx::EVT_PAIRING_SUCCESS)?;
        Ok(keyboard)
    }

    fn send(&mut self, report: &[u8]) -> io::Result<()> {
        self.hid.write_all(report)
    }

    /// The next report the keyboard sends, before `deadline`.
    fn next(&mut self, deadline: Instant) -> io::Result<[u8; 64]> {
        let left = deadline.saturating_duration_since(Instant::now());
        let timeout = PollTimeout::try_from(left).unwrap_or(PollTimeout::ZERO);
        if left.is_zero() || !readable(&self.hid, timeout)? {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "the Voyager did not answer",
            ));
        }
        let mut buf = [0; 64];
        match self.hid.read(&mut buf)? {
            0 => Err(io::Error::from(io::ErrorKind::UnexpectedEof)),
            _ => {
                self.record(&buf);
                Ok(buf)
            }
        }
    }

    /// The next report with this event code, within a second. Others (key
    /// positions while paired, layer reports) are discarded.
    fn reply(&mut self, event: u8) -> io::Result<[u8; 64]> {
        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            let report = self.next(deadline)?;
            if report[0] == event {
                return Ok(report);
            }
        }
    }

    /// Sends a batch, then reads the firmware's answers to it, with a
    /// one-second deadline after the writes ([`Answers`]).
    fn apply(&mut self, reports: &[[u8; oryx::REPORT + 1]]) -> io::Result<()> {
        if reports.is_empty() {
            return Ok(());
        }
        self.drain()?; // answers to anything earlier are not answers to this
        for report in reports {
            self.send(report)?;
        }
        let deadline = Instant::now() + Duration::from_secs(1);
        let mut answers = Answers::default();
        while answers != Answers::Confirmed {
            answers = answers.step(self.next(deadline)?[0]);
        }
        Ok(())
    }

    /// Discard what the keyboard sent, noting key presses. While paired it
    /// reports every key position; reading them keeps the kernel's queue
    /// from filling.
    fn drain(&mut self) -> io::Result<()> {
        let mut buf = [0; 64];
        while readable(&self.hid, PollTimeout::ZERO)? {
            if self.hid.read(&mut buf)? == 0 {
                return Err(io::Error::from(io::ErrorKind::UnexpectedEof));
            }
            self.record(&buf);
        }
        Ok(())
    }

    fn record(&self, report: &[u8]) {
        if report[0] == oryx::EVT_KEYDOWN
            && let Some(at) = boottime()
        {
            self.keys.note(at);
        }
    }
}

/// What the pedals ask of the keyboard, shared with the worker.
struct Wanted {
    layer: Option<u8>,
    stopping: bool,
}

/// Logs a problem once, until it is resolved.
#[derive(Default)]
struct Trouble(bool);

impl Trouble {
    fn report(&mut self, message: impl std::fmt::Display) {
        if !self.0 {
            eprintln!("{message}");
        }
        self.0 = true;
    }

    /// Ends the streak: true if there was one.
    fn resolved(&mut self) -> bool {
        std::mem::take(&mut self.0)
    }
}

/// Wait until the pedals change, or `timeout` passes.
fn rest(wake: &UnixStream, timeout: PollTimeout) {
    let _ = readable(wake, timeout);
    drain_wake(wake);
}

/// Owns the Voyager: applies the latest wanted layer, pairing again after a
/// replug, so the pedal loop never waits on the keyboard. When stopping,
/// tells `done` whether the pedal layers are known to be off.
fn keyboard_worker(
    wanted: Arc<Mutex<Wanted>>,
    wake: UnixStream,
    keys: Arc<KeyLog>,
    done: mpsc::Sender<bool>,
) {
    let owned: Vec<u8> = PEDALS.iter().map(|pedal| pedal.layer).collect();
    let mut keyboard: Option<Keyboard> = None;
    let mut knowledge = Knowledge::new();
    let mut trouble = Trouble::default();
    loop {
        let (layer, stopping) = {
            let wanted = wanted.lock();
            (wanted.layer, wanted.stopping)
        };
        let Some(connected) = keyboard.as_mut() else {
            if stopping {
                let _ = done.send(knowledge.off());
                return;
            }
            match Keyboard::connect(Arc::clone(&keys)) {
                Ok(paired) => {
                    if !knowledge.reached {
                        eprintln!(
                            "voyager: paired on {}, Oryx protocol {}",
                            paired.path.display(),
                            paired.version
                        );
                    }
                    knowledge.paired();
                    keyboard = Some(paired);
                }
                Err(error) => {
                    trouble.report(format!("voyager: {error}; retrying"));
                    rest(&wake, PollTimeout::from(2000u16));
                }
            }
            continue;
        };
        let reports = batch(&plan(knowledge.applied, layer, &owned));
        if let Err(error) = connected.apply(&reports) {
            // Any of it may have run: the next connection plans from scratch.
            knowledge.failed();
            trouble.report(format!("voyager: {error}; reconnecting"));
            keyboard = None;
            rest(&wake, PollTimeout::from(1000u16));
            continue;
        }
        knowledge.confirmed(layer);
        if !reports.is_empty() && trouble.resolved() {
            eprintln!("voyager: back, layer applied");
        }
        if stopping {
            let _ = done.send(true);
            return;
        }
        // Sleep until the pedals change or the keyboard sends something.
        let mut fds = [
            PollFd::new(wake.as_fd(), PollFlags::POLLIN),
            PollFd::new(connected.hid.as_fd(), PollFlags::POLLIN),
        ];
        match poll(&mut fds, PollTimeout::NONE) {
            Ok(_) => {}
            Err(Errno::EINTR) => continue,
            Err(error) => {
                trouble.report(format!("voyager: poll: {error}"));
                rest(&wake, PollTimeout::from(1000u16));
                continue;
            }
        }
        let hid = fds[1].revents().unwrap_or(PollFlags::empty());
        drain_wake(&wake);
        if !hid.is_empty()
            && let Err(error) = connected.drain()
        {
            trouble.report(format!("voyager: gone ({error}); waiting for it"));
            keyboard = None;
        }
    }
}

fn drain_wake(wake: &UnixStream) {
    let mut buf = [0; 64];
    while matches!(readable(wake, PollTimeout::ZERO), Ok(true)) {
        if matches!((&*wake).read(&mut buf), Ok(0) | Err(_)) {
            return;
        }
    }
}

/// The pedal side's line to the keyboard worker.
struct ToWorker {
    wanted: Arc<Mutex<Wanted>>,
    wake: UnixStream,
    /// When the Voyager reported keys going down, noted by the worker.
    keys: Arc<KeyLog>,
}

impl ToWorker {
    fn want(&self, layer: Option<u8>) {
        let mut wanted = self.wanted.lock();
        if !wanted.stopping && wanted.layer != layer {
            wanted.layer = layer;
            let _ = (&self.wake).write(&[1]);
        }
    }
}

fn pedal_loop(worker: ToWorker) {
    let mut pedals = Pedals::new(&PEDALS);
    let mut trouble = Trouble::default();
    loop {
        let mut device = match RawDevice::open(PEDAL) {
            Ok(device) => device,
            Err(error) => {
                if error.kind() != io::ErrorKind::NotFound {
                    trouble.report(format!("pedal: {error}; retrying every 2s"));
                }
                thread::sleep(Duration::from_secs(2));
                continue;
            }
        };
        let Err(error) = read_pedal(&mut device, &mut pedals, &worker, &mut trouble);
        // Whatever was held is not any more, and nothing can unlock a lock.
        if let Some(name) = pedals.locked_name() {
            eprintln!("pedal: {name} unlocked");
        }
        pedals.lost();
        worker.want(pedals.layer());
        if error.raw_os_error() == Some(Errno::ENODEV as i32) {
            eprintln!("pedal disconnected");
        } else {
            trouble.report(format!("pedal: {error}; retrying every 2s"));
            thread::sleep(Duration::from_secs(2));
        }
    }
}

/// Follows the pedal until reading it fails.
fn read_pedal(
    device: &mut RawDevice,
    pedals: &mut Pedals,
    worker: &ToWorker,
    trouble: &mut Trouble,
) -> io::Result<Infallible> {
    use_boottime(device)?;
    device.grab()?;
    let held = |device: &RawDevice| -> io::Result<Vec<KeyCode>> {
        Ok(device.get_key_state()?.iter().collect())
    };
    // A pedal may already be down.
    pedals.sync(&held(device)?);
    worker.want(pedals.layer());
    eprintln!("pedal connected");
    trouble.resolved();
    let mut dropping = Dropping::default();
    loop {
        let events: Vec<InputEvent> = device.fetch_events()?.collect();
        // the lock's net change over the batch
        let was = pedals.locked_name();
        let typed = |from, to| worker.keys.typed(from, to);
        take_batch(events, &mut dropping, pedals, &typed, || held(device))?;
        let is = pedals.locked_name();
        if was != is {
            if let Some(name) = was {
                eprintln!("pedal: {name} unlocked");
            }
            if let Some(name) = is {
                eprintln!("pedal: {name} locked; tap a pedal to unlock");
            }
        }
        worker.want(pedals.layer());
    }
}

fn main() {
    // main takes SIGTERM/SIGINT synchronously; block them before any thread
    // exists so none of the others is interrupted by them.
    let mut signals = SigSet::empty();
    signals.add(Signal::SIGTERM);
    signals.add(Signal::SIGINT);
    signals.thread_block().expect("blocking SIGTERM and SIGINT");

    let wanted = Arc::new(Mutex::new(Wanted {
        layer: None,
        stopping: false,
    }));
    let (wake, wake_worker) = UnixStream::pair().expect("socketpair");
    wake.set_nonblocking(true)
        .expect("non-blocking wake socket");
    let (done, finished) = mpsc::channel();
    let keys = Arc::new(KeyLog::default());

    let worker_wanted = Arc::clone(&wanted);
    let worker_keys = Arc::clone(&keys);
    thread::spawn(move || keyboard_worker(worker_wanted, wake_worker, worker_keys, done));
    let worker = ToWorker {
        wanted: Arc::clone(&wanted),
        wake: wake.try_clone().expect("wake socket"),
        keys,
    };
    thread::spawn(move || pedal_loop(worker));

    let _ = signals.wait();
    // Turn the pedal layers off before going.
    {
        let mut wanted = wanted.lock();
        wanted.layer = None;
        wanted.stopping = true;
    }
    let _ = (&wake).write(&[1]);
    match finished.recv_timeout(Duration::from_secs(2)) {
        Ok(true) => {}
        Ok(false) => eprintln!(
            "stopping: the Voyager is unreachable, so a pedal layer may stay on until the next start"
        ),
        Err(_) => eprintln!(
            "stopping: the Voyager did not take the layer reset in time; a pedal layer may stay on until the next start"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LEFT: KeyCode = KeyCode::KEY_A;
    const MIDDLE: KeyCode = KeyCode::KEY_B;
    const RIGHT: KeyCode = KeyCode::KEY_C;

    fn ms(ms: u64) -> Duration {
        Duration::from_millis(ms)
    }

    /// No Voyager key went down during the press.
    fn quiet(_: Duration, _: Duration) -> bool {
        false
    }

    fn down(pedals: &Pedals) -> Vec<KeyCode> {
        pedals.held.iter().map(|press| press.key).collect()
    }

    #[test]
    fn the_later_pedal_wins_and_hands_back() {
        let mut pedals = Pedals::new(&PEDALS);
        pedals.press(MIDDLE, ms(0));
        assert_eq!(pedals.layer(), Some(NAV));
        pedals.press(RIGHT, ms(1000));
        assert_eq!(pedals.layer(), Some(MOUSE));
        pedals.press(LEFT, ms(2000));
        assert_eq!(pedals.layer(), Some(MIRROR));
        pedals.release(LEFT, ms(3000), quiet);
        assert_eq!(pedals.layer(), Some(MOUSE));
        pedals.release(RIGHT, ms(4000), quiet);
        assert_eq!(pedals.layer(), Some(NAV));
        pedals.release(MIDDLE, ms(5000), quiet);
        assert_eq!(pedals.layer(), None);
    }

    #[test]
    fn releasing_an_earlier_pedal_leaves_the_later_in_charge() {
        let mut pedals = Pedals::new(&PEDALS);
        pedals.press(MIDDLE, ms(0));
        pedals.press(RIGHT, ms(1000));
        pedals.release(MIDDLE, ms(2000), quiet);
        assert_eq!(pedals.layer(), Some(MOUSE));
        pedals.release(RIGHT, ms(3000), quiet);
        assert_eq!(pedals.layer(), None);
    }

    #[test]
    fn repeats_strays_and_other_keys_change_nothing() {
        let mut pedals = Pedals::new(&PEDALS);
        pedals.press(LEFT, ms(0));
        pedals.press(LEFT, ms(10));
        pedals.release(RIGHT, ms(20), quiet);
        pedals.press(KeyCode::KEY_LEFTSHIFT, ms(30));
        assert_eq!(down(&pedals), [LEFT]);
    }

    #[test]
    fn sync_adopts_the_real_state() {
        let mut pedals = Pedals::new(&PEDALS);
        pedals.press(LEFT, ms(0));
        pedals.press(MIDDLE, ms(1000));
        pedals.sync(&[RIGHT]);
        assert_eq!(pedals.layer(), Some(MOUSE));
        pedals.sync(&[]);
        assert_eq!(pedals.layer(), None);
    }

    #[test]
    fn pedals_already_down_at_open_take_effect() {
        let mut pedals = Pedals::new(&PEDALS);
        pedals.sync(&[RIGHT, KeyCode::KEY_LEFTSHIFT]);
        assert_eq!(pedals.layer(), Some(MOUSE));
        assert_eq!(down(&pedals), [RIGHT], "not a pedal");
    }

    #[test]
    fn a_tap_locks_mirror_or_mouse_and_the_next_tap_unlocks() {
        let mut pedals = Pedals::new(&PEDALS);
        pedals.press(LEFT, ms(0));
        pedals.release(LEFT, ms(120), quiet);
        assert_eq!(pedals.layer(), Some(MIRROR), "locked after the release");
        // a tap on another pedal unlocks, and does not lock that one
        pedals.press(RIGHT, ms(1000));
        assert_eq!(pedals.layer(), Some(MOUSE));
        pedals.release(RIGHT, ms(1100), quiet);
        assert_eq!(pedals.layer(), None);
        assert_eq!(pedals.locked, None);
    }

    #[test]
    fn a_long_press_or_one_typed_under_is_a_hold() {
        let mut pedals = Pedals::new(&PEDALS);
        pedals.press(RIGHT, ms(0));
        pedals.release(RIGHT, ms(250), quiet);
        assert_eq!(pedals.layer(), None, "250 ms is a hold");
        // a quick click on the mouse layer
        pedals.press(RIGHT, ms(1000));
        pedals.release(RIGHT, ms(1100), |_, _| true);
        assert_eq!(pedals.layer(), None);
        // a release timed before its press
        pedals.press(RIGHT, ms(3000));
        pedals.release(RIGHT, ms(2900), quiet);
        assert_eq!(pedals.layer(), None);
        // what is asked about is the press's own span
        pedals.press(RIGHT, ms(4000));
        pedals.release(RIGHT, ms(4100), |from, to| {
            assert_eq!((from, to), (ms(4000), ms(4100)));
            false
        });
        assert_eq!(pedals.layer(), Some(MOUSE));
    }

    #[test]
    fn keys_count_from_the_press_to_a_grace_past_the_release() {
        let keys = KeyLog::default();
        // long past (boot time is far later), so nothing is waited for
        keys.note(ms(1000));
        keys.note(ms(2030));
        assert!(!keys.typed(ms(1001), ms(1100)), "before the press");
        assert!(keys.typed(ms(900), ms(1100)), "during the press");
        assert!(
            keys.typed(ms(1950), ms(2000)),
            "arrived within the grace after the release"
        );
        assert!(!keys.typed(ms(1950), ms(1980)), "after the grace");
    }

    #[test]
    fn a_report_on_its_way_is_waited_for_and_a_seen_one_is_not() {
        let keys = KeyLog::default();
        // decided right at the release: the grace is waited out, and a key
        // read during it counts
        let read_late = |left| {
            assert_eq!(left, GRACE);
            keys.note(ms(1010));
        };
        assert!(keys.typed_waiting(ms(900), ms(1000), Some(ms(1000)), read_late));
        // a key already seen answers at once
        let no_wait = |_| unreachable!("answered already");
        assert!(keys.typed_waiting(ms(900), ms(1000), Some(ms(1000)), no_wait));
        // nor is anything waited for once the grace is over
        assert!(!keys.typed_waiting(ms(2000), ms(2100), Some(ms(2140)), no_wait));
    }

    #[test]
    fn nothing_is_asked_when_the_answer_cannot_matter() {
        // nav cannot lock, and nothing is locked to undo
        let mut pedals = Pedals::new(&PEDALS);
        pedals.press(MIDDLE, ms(0));
        pedals.release(MIDDLE, ms(50), |_, _| unreachable!("asked"));
        assert_eq!(pedals.layer(), None);
    }

    #[test]
    fn nav_never_locks_but_its_tap_unlocks() {
        let mut pedals = Pedals::new(&PEDALS);
        pedals.press(MIDDLE, ms(0));
        pedals.release(MIDDLE, ms(50), quiet);
        assert_eq!(pedals.layer(), None);
        pedals.press(RIGHT, ms(1000));
        pedals.release(RIGHT, ms(1050), quiet);
        assert_eq!(pedals.layer(), Some(MOUSE));
        pedals.press(MIDDLE, ms(2000));
        pedals.release(MIDDLE, ms(2050), quiet);
        assert_eq!(pedals.layer(), None);
    }

    #[test]
    fn holding_another_pedal_over_a_lock_hands_back_to_it() {
        let mut pedals = Pedals::new(&PEDALS);
        pedals.press(RIGHT, ms(0));
        pedals.release(RIGHT, ms(80), quiet);
        pedals.press(MIDDLE, ms(1000));
        assert_eq!(pedals.layer(), Some(NAV));
        pedals.release(MIDDLE, ms(2000), quiet);
        assert_eq!(pedals.layer(), Some(MOUSE), "still locked");
    }

    #[test]
    fn pressing_the_locked_pedal_turns_the_lock_into_a_hold() {
        let mut pedals = Pedals::new(&PEDALS);
        pedals.press(LEFT, ms(0));
        pedals.release(LEFT, ms(80), quiet);
        pedals.press(LEFT, ms(1000));
        assert_eq!(pedals.layer(), Some(MIRROR));
        // short, but the end of the lock: not a new one
        pedals.release(LEFT, ms(1080), quiet);
        assert_eq!(pedals.layer(), None);
        // held long over a lock: its release ends the lock too
        pedals.press(LEFT, ms(2000));
        pedals.release(LEFT, ms(2080), quiet);
        pedals.press(LEFT, ms(3000));
        pedals.release(LEFT, ms(4000), quiet);
        assert_eq!(pedals.layer(), None);
    }

    #[test]
    fn a_resync_keeps_the_lock_and_never_finds_a_tap() {
        let mut pedals = Pedals::new(&PEDALS);
        pedals.press(RIGHT, ms(0));
        pedals.release(RIGHT, ms(80), quiet);
        pedals.sync(&[MIDDLE]);
        assert_eq!(pedals.layer(), Some(NAV));
        // released within a drop: no time to judge it by
        pedals.sync(&[]);
        assert_eq!(pedals.layer(), Some(MOUSE), "the lock survives");
        // seen down before a drop and down after it, it may have been let go
        // and pressed again: its short release does not unlock...
        pedals.press(LEFT, ms(1000));
        pedals.sync(&[LEFT]);
        pedals.release(LEFT, ms(1050), quiet);
        assert_eq!(pedals.layer(), Some(MOUSE), "still locked");
        // ...nor lock
        pedals.sync(&[RIGHT]); // the locked pedal found down takes it over
        pedals.sync(&[]);
        assert_eq!(pedals.layer(), None);
        pedals.press(RIGHT, ms(2000));
        pedals.sync(&[RIGHT]);
        pedals.release(RIGHT, ms(2050), quiet);
        assert_eq!(pedals.layer(), None);
    }

    #[test]
    fn losing_the_pedal_drops_the_lock() {
        let mut pedals = Pedals::new(&PEDALS);
        pedals.press(LEFT, ms(0));
        pedals.release(LEFT, ms(80), quiet);
        pedals.lost();
        assert_eq!(pedals.layer(), None);
    }

    #[test]
    fn a_drop_is_resynchronised_at_its_report_without_another_event() {
        let mut dropping = Dropping::default();
        let b = KeyCode::KEY_B.code();
        assert_eq!(dropping.step(EV_KEY, b, 1), Step::Key(KeyCode::KEY_B, 1));
        assert_eq!(dropping.step(EV_SYN, SYN_DROPPED, 0), Step::Nothing);
        // the release in the overflowed batch is not trusted...
        assert_eq!(dropping.step(EV_KEY, b, 0), Step::Nothing);
        // ...the device state is read at the report that ends it
        assert_eq!(dropping.step(EV_SYN, SYN_REPORT, 0), Step::Resync);
        assert_eq!(dropping.step(EV_KEY, b, 1), Step::Key(KeyCode::KEY_B, 1));
        assert_eq!(dropping.step(EV_KEY, b, 2), Step::Nothing, "autorepeat");
        assert_eq!(dropping.step(EV_SYN, SYN_REPORT, 0), Step::Nothing);
    }

    #[test]
    fn a_resync_discards_the_rest_of_its_batch() {
        let mut pedals = Pedals::new(&PEDALS);
        let mut dropping = Dropping::default();
        let key = |code: KeyCode, value| InputEvent::new(EV_KEY, code.code(), value);
        let report = InputEvent::new(EV_SYN, SYN_REPORT, 0);
        // An overflow, then a press already in our batch when the resync
        // read the device and flushed its release from the kernel's queue:
        // replaying it would hold that layer with nothing to let it go.
        let events = [
            InputEvent::new(EV_SYN, SYN_DROPPED, 0),
            report,
            key(LEFT, 1),
            report,
        ];
        take_batch(events, &mut dropping, &mut pedals, &quiet, || {
            Ok(vec![MIDDLE])
        })
        .unwrap();
        assert_eq!(
            down(&pedals),
            [MIDDLE],
            "the state read, not the stale press"
        );
        // only that batch: the next is taken as it comes
        take_batch(
            [key(LEFT, 1), report],
            &mut dropping,
            &mut pedals,
            &quiet,
            || unreachable!("no drop, no resync"),
        )
        .unwrap();
        assert_eq!(pedals.layer(), Some(MIRROR));
    }

    #[test]
    fn taps_are_judged_by_the_kernels_event_times() {
        // A press and its release read together, long after the fact (the
        // daemon was busy): their own times decide.
        let mut pedals = Pedals::new(&PEDALS);
        let mut dropping = Dropping::default();
        let at = |ms: i64, value| {
            InputEvent::from(nix::libc::input_event {
                time: nix::libc::timeval {
                    tv_sec: ms / 1000,
                    tv_usec: ms % 1000 * 1000,
                },
                type_: EV_KEY,
                code: RIGHT.code(),
                value,
            })
        };
        let no_resync = || unreachable!("no drop, no resync");
        take_batch(
            [at(1000, 1), at(1600, 0)],
            &mut dropping,
            &mut pedals,
            &quiet,
            no_resync,
        )
        .unwrap();
        assert_eq!(pedals.layer(), None, "600 ms apart: a hold");
        take_batch(
            [at(2000, 1), at(2100, 0)],
            &mut dropping,
            &mut pedals,
            &quiet,
            no_resync,
        )
        .unwrap();
        assert_eq!(pedals.layer(), Some(MOUSE), "100 ms apart: a tap");
    }

    #[test]
    fn release_unsets_and_an_unknown_state_clears_every_pedal_layer() {
        let owned = [NAV, MIRROR, MOUSE];
        assert_eq!(
            plan(Applied::Unknown, None, &owned),
            [
                Command::Unset(NAV),
                Command::Unset(MIRROR),
                Command::Unset(MOUSE)
            ]
        );
        assert_eq!(
            plan(Applied::On(MIRROR), None, &owned),
            [Command::Unset(MIRROR)]
        );
        assert!(plan(Applied::Off, None, &owned).is_empty());
        assert_eq!(
            plan(Applied::Unknown, Some(MOUSE), &owned),
            [Command::Set(MOUSE)]
        );
        assert_eq!(
            plan(Applied::On(MIRROR), Some(NAV), &owned),
            [Command::Set(NAV)]
        );
        assert!(plan(Applied::On(MOUSE), Some(MOUSE), &owned).is_empty());
    }

    #[test]
    fn every_batch_sits_between_two_pairings_and_matches_oryx_c() {
        // raw_hid_receive: data[0] command, param[0] on/off, param[1] layer
        let reports = batch(&[Command::Set(MIRROR), Command::Unset(MOUSE)]);
        assert_eq!(reports.len(), 4);
        assert_eq!(reports[0][..2], [0, oryx::PAIRING_INIT]);
        assert_eq!(reports[1][..4], [0, oryx::SET_LAYER, 1, MIRROR]);
        assert_eq!(reports[2][..4], [0, oryx::SET_LAYER, 0, MOUSE]);
        assert_eq!(reports[3][..2], [0, oryx::PAIRING_INIT]);
        assert!(reports.iter().all(|report| report.len() == 33));
        assert!(reports[1][4..].iter().all(|&byte| byte == 0));
        assert!(batch(&[]).is_empty(), "nothing to do sends nothing");
    }

    #[test]
    fn a_batch_counts_only_once_both_pairings_are_answered_in_order() {
        use oryx::{EVT_LAYER as LAYER, EVT_PAIRING_SUCCESS as PAIRED};
        const KEYDOWN: u8 = 6;
        let answers = |events: &[u8]| {
            events
                .iter()
                .fold(Answers::default(), |answers, &event| answers.step(event))
        };
        assert_eq!(answers(&[PAIRED, LAYER, PAIRED]), Answers::Confirmed);
        // the previous batch's closing snapshot arriving late, a layer
        // change, and keys typed meanwhile
        assert_eq!(
            answers(&[LAYER, PAIRED, LAYER, KEYDOWN, LAYER, PAIRED, LAYER]),
            Answers::Confirmed
        );
        // the opening snapshot's send failed, and unpaired: the commands
        // were ignored, whatever answers the closing pairing
        assert_ne!(answers(&[PAIRED, PAIRED, LAYER]), Answers::Confirmed);
        assert_ne!(
            answers(&[PAIRED, KEYDOWN, LAYER, PAIRED]),
            Answers::Confirmed
        );
        // one pairing answered is not both
        assert_ne!(answers(&[PAIRED, LAYER]), Answers::Confirmed);
    }

    #[test]
    fn a_failed_batch_leaves_a_stop_without_the_keyboard_uncertain() {
        let mut knowledge = Knowledge::new();
        assert!(knowledge.off(), "never reached: nothing of ours is on");
        knowledge.paired();
        assert!(!knowledge.off(), "a previous run's layer may be on");
        knowledge.confirmed(None);
        assert!(knowledge.off());
        // a nav press ran, and its answers were lost
        knowledge.failed();
        assert!(!knowledge.off());
    }

    #[test]
    fn only_a_voyager_raw_hid_interface_is_the_keyboard() {
        let voyager = "DRIVER=hid-generic\nHID_ID=0003:00003297:00001977\nHID_NAME=ZSA Technology Labs Voyager\n";
        let moonlander = "HID_ID=0003:00003297:00001969\n";
        let pedal = "HID_ID=0003:00003553:0000B001\n";
        let raw_hid = [0x06, 0x60, 0xFF, 0x09, 0x61];
        let keyboard = [0x05, 0x01, 0x09, 0x06];
        assert!(is_voyager_oryx(voyager, &raw_hid));
        assert!(
            !is_voyager_oryx(voyager, &keyboard),
            "the Voyager's keyboard interface"
        );
        assert!(!is_voyager_oryx(moonlander, &raw_hid), "another ZSA board");
        assert!(
            !is_voyager_oryx(pedal, &raw_hid),
            "another vendor's raw HID"
        );
    }
}
