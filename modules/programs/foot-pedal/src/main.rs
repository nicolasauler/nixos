//! Foot pedal -> Voyager layers.
//!
//! The PCsensor 3-pedal footswitch (3553:b001) sends its factory keys a, b,
//! c. This grabs it exclusively, so those never reach applications, and makes
//! each pedal hold a layer of the Voyager's own firmware, through its raw-HID
//! interface in ZSA's Oryx protocol (zsa/qmk_modules, oryx/oryx.c): SET_LAYER
//! on (layer_move) on press, SET_LAYER off (layer_off) on release. Each batch
//! of commands sits between two pairings and counts as applied once the
//! firmware has answered both. Nothing on the host remaps Voyager keystrokes,
//! and keymapp is not involved. A worker thread owns the keyboard, so a
//! missing or replugged Voyager never delays reading the pedal.

use std::convert::Infallible;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use evdev::raw_stream::RawDevice;
use evdev::{InputEvent, KeyCode};
use nix::errno::Errno;
use nix::poll::{PollFd, PollFlags, PollTimeout, poll};
use nix::sys::signal::{SigSet, Signal};
use parking_lot::Mutex;

/// Layer indices of Oryx layout JqpmQ ("Colemak dh nicc"). Reordering layers
/// in Oryx means updating these.
const NAV: u8 = 4;
const MOUSE: u8 = 5;
const MIRROR: u8 = 6;

/// Each pedal's factory key, and the layer it holds.
const PEDALS: [(KeyCode, u8); 3] = [
    (KeyCode::KEY_A, NAV),    // left
    (KeyCode::KEY_B, MIRROR), // middle
    (KeyCode::KEY_C, MOUSE),  // right
];

const PEDAL: &str = "/dev/input/by-id/usb-PCsensor_FootSwitch-event-kbd";

/// Which pedals are down, and the layer that asks for. No I/O.
struct Pedals {
    map: &'static [(KeyCode, u8)],
    held: Vec<KeyCode>, // pedals down, in press order
}

impl Pedals {
    fn new(map: &'static [(KeyCode, u8)]) -> Self {
        Pedals {
            map,
            held: Vec::new(),
        }
    }

    fn layer_of(&self, code: KeyCode) -> Option<u8> {
        self.map
            .iter()
            .find_map(|&(pedal, layer)| (pedal == code).then_some(layer))
    }

    fn press(&mut self, code: KeyCode) {
        if self.layer_of(code).is_some() && !self.held.contains(&code) {
            self.held.push(code);
        }
    }

    fn release(&mut self, code: KeyCode) {
        self.held.retain(|&held| held != code);
    }

    /// Adopt the device's real key state: at open, after a drop, on loss.
    fn sync(&mut self, down: &[KeyCode]) {
        self.held.retain(|held| down.contains(held));
        let mut pressed: Vec<KeyCode> = down
            .iter()
            .copied()
            .filter(|&code| self.layer_of(code).is_some() && !self.held.contains(&code))
            .collect();
        pressed.sort(); // their press order is unknown
        self.held.extend(pressed);
    }

    /// Layer of the latest pedal still down.
    fn layer(&self) -> Option<u8> {
        self.held.last().and_then(|&code| self.layer_of(code))
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
/// that state, so it goes too.
fn take_batch(
    events: impl IntoIterator<Item = InputEvent>,
    dropping: &mut Dropping,
    pedals: &mut Pedals,
    held: impl FnOnce() -> io::Result<Vec<KeyCode>>,
) -> io::Result<()> {
    for event in events {
        match dropping.step(event.event_type().0, event.code(), event.value()) {
            Step::Key(code, 1) => pedals.press(code),
            Step::Key(code, _) => pedals.release(code),
            Step::Resync => {
                pedals.sync(&held()?);
                break;
            }
            Step::Nothing => {}
        }
    }
    Ok(())
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

    /// A pairing's snapshot showed the batch for `wanted` done.
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
/// acknowledged, and on USB suspend the firmware empties its queue of
/// received reports: the closing pairing's answer shows what the commands
/// did ([`Answers`]).
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

/// Whether a layer snapshot (the top layer on) shows a batch's commands
/// done: the wanted layer on top, or every layer it unset off, which a top
/// layer below that one proves.
fn shows(snapshot: u8, wanted: Option<u8>, commands: &[Command]) -> bool {
    match wanted {
        Some(layer) => snapshot == layer,
        None => commands
            .iter()
            .all(|&command| matches!(command, Command::Unset(layer) if snapshot < layer)),
    }
}

/// Reading a batch's answers. Every pairing is answered with
/// PAIRING_SUCCESS and, directly after it, a snapshot of the top layer on
/// (oryx.c, pairing_success_event); the closing pairing's comes after every
/// command before it. A snapshot showing the commands done confirms the
/// batch, whichever pairing it answers, as the commands only move toward
/// that state. The closing pairing's snapshot not showing it means a command
/// was lost. Key positions and layer-change events are skipped.
#[derive(Default)]
struct Answers {
    pairings: u8,
    after_pairing: bool,
}

#[derive(Debug, PartialEq)]
enum Verdict {
    Waiting,
    Confirmed,
    /// The closing pairing's snapshot, the commands not done.
    Refused(u8),
}

impl Answers {
    fn step(&mut self, report: &[u8], done: impl Fn(u8) -> bool) -> Verdict {
        let after_pairing = std::mem::replace(
            &mut self.after_pairing,
            report[0] == oryx::EVT_PAIRING_SUCCESS,
        );
        match report[0] {
            oryx::EVT_PAIRING_SUCCESS => {
                self.pairings = self.pairings.saturating_add(1);
                Verdict::Waiting
            }
            oryx::EVT_LAYER if after_pairing => match report[1] {
                layer if done(layer) => Verdict::Confirmed,
                layer if self.pairings >= 2 => Verdict::Refused(layer),
                _ => Verdict::Waiting,
            },
            _ => Verdict::Waiting,
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
}

impl Keyboard {
    fn connect() -> io::Result<Self> {
        let path = find_keyboard()?;
        let hid = OpenOptions::new().read(true).write(true).open(&path)?;
        let mut keyboard = Keyboard {
            hid,
            path,
            version: 0,
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
            _ => Ok(buf),
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

    /// Sends a batch, then reads the firmware's answers, a second at most,
    /// until a snapshot shows it `done` ([`Answers`]).
    fn apply(
        &mut self,
        reports: &[[u8; oryx::REPORT + 1]],
        done: impl Fn(u8) -> bool,
    ) -> io::Result<()> {
        if reports.is_empty() {
            return Ok(());
        }
        self.drain()?; // answers to anything earlier are not answers to this
        for report in reports {
            self.send(report)?;
        }
        let deadline = Instant::now() + Duration::from_secs(1);
        let mut answers = Answers::default();
        loop {
            match answers.step(&self.next(deadline)?, &done) {
                Verdict::Waiting => {}
                Verdict::Confirmed => return Ok(()),
                Verdict::Refused(layer) => {
                    return Err(io::Error::other(format!(
                        "a command was lost: on layer {layer} after the batch"
                    )));
                }
            }
        }
    }

    /// Discard what the keyboard sent. While paired it reports every key
    /// position; reading them keeps the kernel's queue from filling.
    fn drain(&mut self) -> io::Result<()> {
        let mut buf = [0; 64];
        while readable(&self.hid, PollTimeout::ZERO)? {
            if self.hid.read(&mut buf)? == 0 {
                return Err(io::Error::from(io::ErrorKind::UnexpectedEof));
            }
        }
        Ok(())
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
fn keyboard_worker(wanted: Arc<Mutex<Wanted>>, wake: UnixStream, done: mpsc::Sender<bool>) {
    let owned: Vec<u8> = PEDALS.iter().map(|&(_, layer)| layer).collect();
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
            match Keyboard::connect() {
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
        let commands = plan(knowledge.applied, layer, &owned);
        let reports = batch(&commands);
        if let Err(error) = connected.apply(&reports, |snapshot| shows(snapshot, layer, &commands))
        {
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
        // Whatever was held is not any more.
        pedals.sync(&[]);
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
        take_batch(events, &mut dropping, pedals, || held(device))?;
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

    let worker_wanted = Arc::clone(&wanted);
    thread::spawn(move || keyboard_worker(worker_wanted, wake_worker, done));
    let worker = ToWorker {
        wanted: Arc::clone(&wanted),
        wake: wake.try_clone().expect("wake socket"),
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

    #[test]
    fn the_later_pedal_wins_and_hands_back() {
        let mut pedals = Pedals::new(&PEDALS);
        pedals.press(MIDDLE);
        assert_eq!(pedals.layer(), Some(MIRROR));
        pedals.press(RIGHT);
        assert_eq!(pedals.layer(), Some(MOUSE));
        pedals.press(LEFT);
        assert_eq!(pedals.layer(), Some(NAV));
        pedals.release(LEFT);
        assert_eq!(pedals.layer(), Some(MOUSE));
        pedals.release(RIGHT);
        assert_eq!(pedals.layer(), Some(MIRROR));
        pedals.release(MIDDLE);
        assert_eq!(pedals.layer(), None);
    }

    #[test]
    fn releasing_an_earlier_pedal_leaves_the_later_in_charge() {
        let mut pedals = Pedals::new(&PEDALS);
        pedals.press(MIDDLE);
        pedals.press(RIGHT);
        pedals.release(MIDDLE);
        assert_eq!(pedals.layer(), Some(MOUSE));
        pedals.release(RIGHT);
        assert_eq!(pedals.layer(), None);
    }

    #[test]
    fn repeats_strays_and_other_keys_change_nothing() {
        let mut pedals = Pedals::new(&PEDALS);
        pedals.press(LEFT);
        pedals.press(LEFT);
        pedals.release(RIGHT);
        pedals.press(KeyCode::KEY_LEFTSHIFT);
        assert_eq!(pedals.held, [LEFT]);
    }

    #[test]
    fn sync_adopts_the_real_state() {
        let mut pedals = Pedals::new(&PEDALS);
        pedals.press(LEFT);
        pedals.press(MIDDLE);
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
        assert_eq!(pedals.held, [RIGHT], "not a pedal");
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
        take_batch(events, &mut dropping, &mut pedals, || Ok(vec![MIDDLE])).unwrap();
        assert_eq!(pedals.held, [MIDDLE], "the state read, not the stale press");
        // only that batch: the next is taken as it comes
        take_batch([key(LEFT, 1), report], &mut dropping, &mut pedals, || {
            unreachable!("no drop, no resync")
        })
        .unwrap();
        assert_eq!(pedals.layer(), Some(NAV));
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
    fn a_batch_counts_once_a_pairing_snapshot_shows_it_done() {
        use oryx::{EVT_LAYER as LAYER, EVT_PAIRING_SUCCESS as PAIRED};
        const KEYDOWN: u8 = 6;
        let verdict = |reports: &[[u8; 2]], wanted: Option<u8>, commands: &[Command]| {
            let mut answers = Answers::default();
            reports
                .iter()
                .map(|report| answers.step(report, |top| shows(top, wanted, commands)))
                .find(|verdict| *verdict != Verdict::Waiting)
                .unwrap_or(Verdict::Waiting)
        };
        let nav = [Command::Set(NAV)];
        let nav_off = [Command::Unset(NAV)];
        // the opening snapshot shows the old state; a change event (not a
        // snapshot) and a key come between; the closing snapshot shows nav
        let answered = [
            [PAIRED, 0],
            [LAYER, 0],
            [LAYER, NAV],
            [KEYDOWN, 3],
            [PAIRED, 0],
            [LAYER, NAV],
        ];
        assert_eq!(verdict(&answered, Some(NAV), &nav), Verdict::Confirmed);
        // the unset lost between the pairings (the firmware emptied its
        // queue on suspend): both answered, the closing snapshot on nav
        let lost = [[PAIRED, 0], [LAYER, NAV], [PAIRED, 0], [LAYER, NAV]];
        assert_eq!(verdict(&lost, None, &nav_off), Verdict::Refused(NAV));
        // already so before the commands ran: done either way
        assert_eq!(
            verdict(&[[PAIRED, 0], [LAYER, 0]], None, &nav_off),
            Verdict::Confirmed
        );
        // one pairing answered, not showing it: more to come, or a timeout
        assert_eq!(
            verdict(&[[PAIRED, 0], [LAYER, 0]], Some(NAV), &nav),
            Verdict::Waiting
        );
    }

    #[test]
    fn an_unset_layer_is_known_off_only_below_the_top_layer() {
        let mirror_off = [Command::Unset(MIRROR)];
        assert!(
            shows(NAV, None, &mirror_off),
            "nav held from the keyboard is on top: mirror, above it, is off"
        );
        assert!(!shows(MIRROR, None, &mirror_off));
        let all_off = [
            Command::Unset(NAV),
            Command::Unset(MIRROR),
            Command::Unset(MOUSE),
        ];
        assert!(shows(3, None, &all_off));
        assert!(!shows(NAV, None, &all_off), "nav could be ours");
        assert!(shows(MOUSE, Some(MOUSE), &[Command::Set(MOUSE)]));
        assert!(!shows(MIRROR, Some(MOUSE), &[Command::Set(MOUSE)]));
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
