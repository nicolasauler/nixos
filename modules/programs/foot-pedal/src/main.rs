//! Foot pedal -> Voyager layers, and Shift.
//!
//! The PCsensor 3-pedal footswitch (3553:b001) sends its factory keys a, b,
//! c. This grabs it exclusively, so those never reach applications, and makes
//! each pedal a hold action:
//!
//! * layer pedals change the Voyager's own firmware layer through its raw-HID
//!   interface, in ZSA's Oryx protocol (zsa/qmk_modules, oryx/oryx.c):
//!   SET_LAYER on (layer_move) on press, SET_LAYER off (layer_off) on
//!   release, each batch preceded by a pairing, and applied again after a
//!   resume. Nothing on the host remaps Voyager keystrokes and keymapp is not
//!   involved. A worker thread owns the keyboard, so a missing or replugged
//!   Voyager never delays reading the pedal.
//! * the Shift pedal holds Shift on a uinput keyboard, immediately. Hyprland
//!   ORs held modifiers across keyboards, so it applies to Voyager keys.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use evdev::raw_stream::RawDevice;
use evdev::uinput::VirtualDevice;
use evdev::{AttributeSet, BusType, EventType, InputEvent, InputId, KeyCode};
use nix::errno::Errno;
use nix::poll::{PollFd, PollFlags, PollTimeout, poll};
use nix::sys::signal::{SigSet, Signal};
use nix::sys::time::TimeSpec;
use nix::sys::timerfd::{ClockId, Expiration, TimerFd, TimerFlags, TimerSetTimeFlags};
use parking_lot::Mutex;

/// Layer indices of Oryx layout JqpmQ ("Colemak dh nicc"). Reordering layers
/// in Oryx means updating these.
const MOUSE: u8 = 5;
const MIRROR: u8 = 6;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Action {
    Key(KeyCode),
    Layer(u8),
}

const PEDALS: [(KeyCode, Action); 3] = [
    (KeyCode::KEY_A, Action::Key(KeyCode::KEY_LEFTSHIFT)), // left
    (KeyCode::KEY_B, Action::Layer(MIRROR)),               // middle
    (KeyCode::KEY_C, Action::Layer(MOUSE)),                // right
];

const PEDAL: &str = "/dev/input/by-id/usb-PCsensor_FootSwitch-event-kbd";

/// Which pedals are down, and what that asks for. No I/O.
struct Pedals {
    map: &'static [(KeyCode, Action)],
    held: Vec<KeyCode>, // pedals down, in press order
}

impl Pedals {
    fn new(map: &'static [(KeyCode, Action)]) -> Self {
        Pedals {
            map,
            held: Vec::new(),
        }
    }

    fn action(&self, code: KeyCode) -> Option<Action> {
        self.map
            .iter()
            .find_map(|&(pedal, action)| (pedal == code).then_some(action))
    }

    /// Synthetic key changes caused by a pedal going down.
    fn press(&mut self, code: KeyCode) -> Vec<(KeyCode, i32)> {
        match self.action(code) {
            Some(action) if !self.held.contains(&code) => {
                self.held.push(code);
                key_change(action, 1)
            }
            _ => Vec::new(),
        }
    }

    /// Synthetic key changes caused by a pedal coming up.
    fn release(&mut self, code: KeyCode) -> Vec<(KeyCode, i32)> {
        match (
            self.held.iter().position(|&held| held == code),
            self.action(code),
        ) {
            (Some(index), Some(action)) => {
                self.held.remove(index);
                key_change(action, 0)
            }
            _ => Vec::new(),
        }
    }

    /// Adopt the device's real key state: at open, after a drop, on loss.
    fn sync(&mut self, down: &[KeyCode]) -> Vec<(KeyCode, i32)> {
        let mut changes = Vec::new();
        for code in self.held.clone() {
            if !down.contains(&code) {
                changes.extend(self.release(code));
            }
        }
        let mut pressed: Vec<KeyCode> = down
            .iter()
            .copied()
            .filter(|code| !self.held.contains(code))
            .collect();
        pressed.sort(); // their press order is unknown
        for code in pressed {
            changes.extend(self.press(code));
        }
        changes
    }

    /// Layer of the latest layer pedal still down.
    fn layer(&self) -> Option<u8> {
        self.held
            .iter()
            .rev()
            .find_map(|&code| match self.action(code) {
                Some(Action::Layer(layer)) => Some(layer),
                _ => None,
            })
    }
}

fn key_change(action: Action, value: i32) -> Vec<(KeyCode, i32)> {
    match action {
        Action::Key(key) => vec![(key, value)],
        Action::Layer(_) => Vec::new(),
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

/// Synthetic key changes for one fetched batch of raw events. A resync reads
/// the device's key state (EVIOCGKEY), which also flushes the key events the
/// kernel still queues for this reader; the rest of the batch is older than
/// that state, so it goes too.
fn pedal_changes(
    events: impl IntoIterator<Item = InputEvent>,
    dropping: &mut Dropping,
    pedals: &mut Pedals,
    held: impl FnOnce() -> io::Result<Vec<KeyCode>>,
) -> io::Result<Vec<(KeyCode, i32)>> {
    let mut changes = Vec::new();
    for event in events {
        match dropping.step(event.event_type().0, event.code(), event.value()) {
            Step::Key(code, 1) => changes.extend(pedals.press(code)),
            Step::Key(code, _) => changes.extend(pedals.release(code)),
            Step::Resync => {
                changes.extend(pedals.sync(&held()?));
                break;
            }
            Step::Nothing => {}
        }
    }
    Ok(changes)
}

/// What the firmware's pedal layers are known to be.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Applied {
    /// Just paired (a previous run may have died mid-hold), or resumed (the
    /// firmware empties its queue of received reports on USB suspend, so the
    /// last batch may never have run).
    Unknown,
    /// No pedal layer on.
    Off,
    On(u8),
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

/// Reports for one batch of commands. The firmware ignores SET_LAYER unless
/// paired, and can drop the pairing on its own (a failed send, e.g. across
/// a USB suspend) without telling the host. Pairing is unconditional and the
/// firmware handles reports in order, so pairing first gets the batch past
/// that check. It does not make the batch land: writes are not acknowledged,
/// and pairing's own replies unpair again if their send fails.
fn batch(commands: &[Command]) -> Vec<[u8; oryx::REPORT + 1]> {
    if commands.is_empty() {
        return Vec::new();
    }
    let mut reports = vec![oryx::report(&[oryx::PAIRING_INIT])];
    reports.extend(commands.iter().map(|command| match *command {
        Command::Set(layer) => oryx::report(&[oryx::SET_LAYER, 1, layer]),
        Command::Unset(layer) => oryx::report(&[oryx::SET_LAYER, 0, layer]),
    }));
    reports
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

    /// The next report with this event code, within a second. Others (key
    /// positions while paired, layer reports) are discarded.
    fn reply(&mut self, event: u8) -> io::Result<[u8; 64]> {
        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let timeout = PollTimeout::try_from(left).unwrap_or(PollTimeout::ZERO);
            if left.is_zero() || !readable(&self.hid, timeout)? {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!("no reply {event:#04x} from the Voyager"),
                ));
            }
            let mut buf = [0; 64];
            if self.hid.read(&mut buf)? > 0 && buf[0] == event {
                return Ok(buf);
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

/// Readable once the system resumes from suspend. Linux treats a resume as
/// the wall clock jumping against the monotonic one (timekeeping_resume ->
/// timerfd_resume) and cancels absolute wall-clock timers armed with
/// TFD_TIMER_CANCEL_ON_SET. This one is set for 2106, so the only other thing
/// that wakes it is setting the clock, which costs one needless batch.
fn resume_alarm() -> nix::Result<TimerFd> {
    let alarm = TimerFd::new(
        ClockId::CLOCK_REALTIME,
        TimerFlags::TFD_NONBLOCK | TimerFlags::TFD_CLOEXEC,
    )?;
    alarm.set(
        Expiration::OneShot(TimeSpec::new(u32::MAX.into(), 0)),
        TimerSetTimeFlags::TFD_TIMER_ABSTIME | TimerSetTimeFlags::TFD_TIMER_CANCEL_ON_SET,
    )?;
    Ok(alarm)
}

/// Owns the Voyager: applies the latest wanted layer, pairing again after a
/// replug and applying again after a resume, so the pedal loop never waits
/// on the keyboard. When stopping, tells `done` whether the pedal layers are
/// known to be off.
fn keyboard_worker(
    wanted: Arc<Mutex<Wanted>>,
    wake: UnixStream,
    resumed: TimerFd,
    done: mpsc::Sender<bool>,
) {
    let owned: Vec<u8> = PEDALS
        .iter()
        .filter_map(|(_, action)| match action {
            Action::Layer(layer) => Some(*layer),
            Action::Key(_) => None,
        })
        .collect();
    let mut keyboard: Option<Keyboard> = None;
    let mut applied = Applied::Unknown;
    let mut trouble = Trouble::default();
    let mut first = true; // not reached the keyboard yet
    loop {
        let (layer, stopping) = {
            let wanted = wanted.lock();
            (wanted.layer, wanted.stopping)
        };
        let Some(connected) = keyboard.as_mut() else {
            if stopping {
                // Unreachable: off only if known off before it went. A run
                // that never reached it turned nothing on.
                let _ = done.send(first || applied == Applied::Off);
                return;
            }
            match Keyboard::connect() {
                Ok(paired) => {
                    if first {
                        eprintln!(
                            "voyager: paired on {}, Oryx protocol {}",
                            paired.path.display(),
                            paired.version
                        );
                        first = false;
                    }
                    keyboard = Some(paired);
                    applied = Applied::Unknown;
                }
                Err(error) => {
                    trouble.report(format!("voyager: {error}; retrying"));
                    rest(&wake, PollTimeout::from(2000u16));
                }
            }
            continue;
        };
        let reports = batch(&plan(applied, layer, &owned));
        if let Err(error) = reports.iter().try_for_each(|report| connected.send(report)) {
            trouble.report(format!("voyager: {error}; reconnecting"));
            keyboard = None;
            rest(&wake, PollTimeout::from(1000u16));
            continue;
        }
        applied = layer.map_or(Applied::Off, Applied::On);
        if !reports.is_empty() && trouble.resolved() {
            eprintln!("voyager: back, layer applied");
        }
        if stopping {
            let _ = done.send(true);
            return;
        }
        // Sleep until the pedals change, the keyboard sends something, or
        // the system resumes.
        let mut fds = [
            PollFd::new(wake.as_fd(), PollFlags::POLLIN),
            PollFd::new(connected.hid.as_fd(), PollFlags::POLLIN),
            PollFd::new(resumed.as_fd(), PollFlags::POLLIN),
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
        let resume = fds[2].revents().unwrap_or(PollFlags::empty());
        drain_wake(&wake);
        if !resume.is_empty() {
            // Take the cancellation (the next resume wakes it again), and
            // send the wanted layer, or clear ours, again.
            let _ = resumed.wait();
            eprintln!("voyager: resumed (or the clock was set); applying the pedal layers again");
            applied = Applied::Unknown;
        }
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

/// The pedal side: what the read loop needs to act.
struct Outputs {
    shift: VirtualDevice,
    wanted: Arc<Mutex<Wanted>>,
    wake: UnixStream,
}

impl Outputs {
    fn apply(&mut self, pedals: &Pedals, changes: Vec<(KeyCode, i32)>) -> io::Result<()> {
        for (key, value) in changes {
            self.shift
                .emit(&[InputEvent::new(EventType::KEY.0, key.code(), value)])?;
        }
        let mut wanted = self.wanted.lock();
        if !wanted.stopping && wanted.layer != pedals.layer() {
            wanted.layer = pedals.layer();
            let _ = (&self.wake).write(&[1]);
        }
        Ok(())
    }
}

fn pedal_loop(mut outputs: Outputs) {
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
        let result = read_pedal(&mut device, &mut pedals, &mut outputs, &mut trouble);
        // Whatever was held is not any more.
        let released = pedals.sync(&[]);
        if let Err(error) = outputs.apply(&pedals, released) {
            eprintln!("shift keyboard: {error}");
        }
        match result {
            Err(error) if error.raw_os_error() == Some(Errno::ENODEV as i32) => {
                eprintln!("pedal disconnected");
            }
            Err(error) => {
                trouble.report(format!("pedal: {error}; retrying every 2s"));
                thread::sleep(Duration::from_secs(2));
            }
            Ok(()) => unreachable!("the read loop only ends in an error"),
        }
    }
}

fn read_pedal(
    device: &mut RawDevice,
    pedals: &mut Pedals,
    outputs: &mut Outputs,
    trouble: &mut Trouble,
) -> io::Result<()> {
    device.grab()?;
    let held = |device: &RawDevice| -> io::Result<Vec<KeyCode>> {
        Ok(device.get_key_state()?.iter().collect())
    };
    // A pedal may already be down.
    let changes = pedals.sync(&held(device)?);
    outputs.apply(pedals, changes)?;
    eprintln!("pedal connected");
    trouble.resolved();
    let mut dropping = Dropping::default();
    loop {
        let events: Vec<InputEvent> = device.fetch_events()?.collect();
        let changes = pedal_changes(events, &mut dropping, pedals, || held(device))?;
        outputs.apply(pedals, changes)?;
    }
}

fn shift_keyboard() -> io::Result<VirtualDevice> {
    let keys: AttributeSet<KeyCode> = PEDALS
        .iter()
        .filter_map(|(_, action)| match action {
            Action::Key(key) => Some(*key),
            Action::Layer(_) => None,
        })
        .collect();
    VirtualDevice::builder()?
        .name("foot-pedal")
        .input_id(InputId::new(BusType::BUS_VIRTUAL, 0, 0, 1))
        .with_keys(&keys)?
        .build()
}

fn main() {
    // main takes SIGTERM/SIGINT synchronously; block them before any thread
    // exists so none of the others is interrupted by them.
    let mut signals = SigSet::empty();
    signals.add(Signal::SIGTERM);
    signals.add(Signal::SIGINT);
    signals.thread_block().expect("blocking SIGTERM and SIGINT");

    let shift = shift_keyboard().unwrap_or_else(|error| {
        eprintln!("cannot create the Shift keyboard on /dev/uinput: {error}");
        std::process::exit(1);
    });
    let wanted = Arc::new(Mutex::new(Wanted {
        layer: None,
        stopping: false,
    }));
    let (wake, wake_worker) = UnixStream::pair().expect("socketpair");
    wake.set_nonblocking(true)
        .expect("non-blocking wake socket");
    let resumed = resume_alarm().expect("resume timer");
    let (done, finished) = mpsc::channel();

    let worker_wanted = Arc::clone(&wanted);
    thread::spawn(move || keyboard_worker(worker_wanted, wake_worker, resumed, done));
    let outputs = Outputs {
        shift,
        wanted: Arc::clone(&wanted),
        wake: wake.try_clone().expect("wake socket"),
    };
    thread::spawn(move || pedal_loop(outputs));

    let _ = signals.wait();
    // Turn the pedal layers off before going. A held Shift needs nothing:
    // the kernel releases it when the uinput device goes with this process.
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

    const SHIFT: KeyCode = KeyCode::KEY_A;
    const MIDDLE: KeyCode = KeyCode::KEY_B;
    const RIGHT: KeyCode = KeyCode::KEY_C;
    const SHIFT_DOWN: (KeyCode, i32) = (KeyCode::KEY_LEFTSHIFT, 1);
    const SHIFT_UP: (KeyCode, i32) = (KeyCode::KEY_LEFTSHIFT, 0);

    #[test]
    fn the_later_layer_pedal_wins_and_hands_back() {
        let mut pedals = Pedals::new(&PEDALS);
        assert!(pedals.press(MIDDLE).is_empty());
        assert_eq!(pedals.layer(), Some(MIRROR));
        pedals.press(RIGHT);
        assert_eq!(pedals.layer(), Some(MOUSE));
        pedals.release(RIGHT);
        assert_eq!(pedals.layer(), Some(MIRROR));
        pedals.release(MIDDLE);
        assert_eq!(pedals.layer(), None);
    }

    #[test]
    fn releasing_the_earlier_pedal_leaves_the_later_in_charge() {
        let mut pedals = Pedals::new(&PEDALS);
        pedals.press(MIDDLE);
        pedals.press(RIGHT);
        pedals.release(MIDDLE);
        assert_eq!(pedals.layer(), Some(MOUSE));
        pedals.release(RIGHT);
        assert_eq!(pedals.layer(), None);
    }

    #[test]
    fn shift_is_a_key_independent_of_layers() {
        let mut pedals = Pedals::new(&PEDALS);
        assert_eq!(pedals.press(SHIFT), [SHIFT_DOWN]);
        assert_eq!(pedals.layer(), None);
        pedals.press(MIDDLE);
        assert_eq!(pedals.release(SHIFT), [SHIFT_UP]);
        assert_eq!(pedals.layer(), Some(MIRROR));
    }

    #[test]
    fn repeats_strays_and_other_keys_change_nothing() {
        let mut pedals = Pedals::new(&PEDALS);
        pedals.press(SHIFT);
        assert!(pedals.press(SHIFT).is_empty());
        assert!(pedals.release(RIGHT).is_empty());
        assert!(pedals.press(KeyCode::KEY_LEFTSHIFT).is_empty());
        assert_eq!(pedals.held, [SHIFT]);
    }

    #[test]
    fn sync_adopts_the_real_state() {
        let mut pedals = Pedals::new(&PEDALS);
        pedals.press(SHIFT);
        pedals.press(MIDDLE);
        assert_eq!(pedals.sync(&[RIGHT]), [SHIFT_UP]);
        assert_eq!(pedals.layer(), Some(MOUSE));
        assert!(pedals.sync(&[]).is_empty());
        assert_eq!(pedals.layer(), None);
    }

    #[test]
    fn pedals_already_down_at_open_take_effect() {
        let mut pedals = Pedals::new(&PEDALS);
        assert_eq!(pedals.sync(&[RIGHT, SHIFT]), [SHIFT_DOWN]);
        assert_eq!(pedals.layer(), Some(MOUSE));
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
    fn a_pedal_held_at_open_and_released_in_a_drop_is_let_go() {
        // held at open: adopted from the device state, not seen pressed...
        let mut pedals = Pedals::new(&PEDALS);
        assert_eq!(pedals.sync(&[SHIFT, MIDDLE]), [SHIFT_DOWN]);
        // ...its release lost to a drop; the resync reads both up
        assert_eq!(pedals.sync(&[]), [SHIFT_UP]);
        assert_eq!(pedals.layer(), None);
    }

    #[test]
    fn a_resync_discards_the_rest_of_its_batch() {
        let mut pedals = Pedals::new(&PEDALS);
        let mut dropping = Dropping::default();
        let key = |code: KeyCode, value| InputEvent::new(EV_KEY, code.code(), value);
        let report = InputEvent::new(EV_SYN, SYN_REPORT, 0);
        // An overflow, then a Shift press already in our batch when the
        // resync read the device and flushed its release from the kernel's
        // queue: replaying it would hold Shift with nothing to let it go.
        let batch = [
            InputEvent::new(EV_SYN, SYN_DROPPED, 0),
            report,
            key(SHIFT, 1),
            report,
        ];
        let changes = pedal_changes(batch, &mut dropping, &mut pedals, || Ok(vec![MIDDLE]));
        assert!(
            changes.unwrap().is_empty(),
            "its press predates the state read"
        );
        assert_eq!(pedals.held, [MIDDLE], "the state read is adopted");
        // only that batch: the next is taken as it comes
        let changes = pedal_changes([key(SHIFT, 1), report], &mut dropping, &mut pedals, || {
            unreachable!("no drop, no resync")
        });
        assert_eq!(changes.unwrap(), [SHIFT_DOWN]);
    }

    #[test]
    fn release_unsets_and_an_unknown_state_clears_every_pedal_layer() {
        let owned = [MIRROR, MOUSE];
        let unset_all = [Command::Unset(MIRROR), Command::Unset(MOUSE)];
        assert_eq!(plan(Applied::Unknown, None, &owned), unset_all);
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
            plan(Applied::On(MIRROR), Some(MOUSE), &owned),
            [Command::Set(MOUSE)]
        );
        assert!(plan(Applied::On(MOUSE), Some(MOUSE), &owned).is_empty());
    }

    #[test]
    fn every_batch_pairs_first_and_reports_match_oryx_c() {
        // raw_hid_receive: data[0] command, param[0] on/off, param[1] layer
        let reports = batch(&[Command::Set(MIRROR), Command::Unset(MOUSE)]);
        assert_eq!(reports.len(), 3);
        assert_eq!(reports[0][..2], [0, oryx::PAIRING_INIT]);
        assert_eq!(reports[1][..4], [0, oryx::SET_LAYER, 1, MIRROR]);
        assert_eq!(reports[2][..4], [0, oryx::SET_LAYER, 0, MOUSE]);
        assert!(reports.iter().all(|report| report.len() == 33));
        assert!(reports[1][4..].iter().all(|&byte| byte == 0));
        assert!(batch(&[]).is_empty(), "nothing to do sends nothing");
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
