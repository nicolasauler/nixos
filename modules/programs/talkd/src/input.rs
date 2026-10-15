//! F13 from the Voyager, read from its evdev node(s) without grabbing them:
//! Hyprland still gets the key (and swallows it with a no-op bind). Only
//! F13 is looked at; every other key is dropped unread. Event times are the
//! kernel's, on CLOCK_BOOTTIME.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::os::fd::{AsFd, AsRawFd};
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

use evdev::raw_stream::RawDevice;
use evdev::{AttributeSetRef, InputId, KeyCode};
use nix::errno::Errno;
use nix::poll::{PollFd, PollFlags, PollTimeout, poll};
use nix::sys::inotify::{AddWatchFlags, InitFlags, Inotify};

/// ZSA Voyager.
const VENDOR: u16 = 0x3297;
const PRODUCT: u16 = 0x1977;

const EV_SYN: u16 = 0;
const EV_KEY: u16 = 1;
const SYN_REPORT: u16 = 0;
const SYN_DROPPED: u16 = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Press(Duration),
    Release(Duration),
    /// The node went away with F13 down.
    Lost,
}

/// A Voyager node that can send F13.
pub fn wanted(id: InputId, keys: Option<&AttributeSetRef<KeyCode>>) -> bool {
    id.vendor() == VENDOR
        && id.product() == PRODUCT
        && keys.is_some_and(|keys| keys.contains(KeyCode::KEY_F13))
}

/// One node's F13. After SYN_DROPPED the kernel's rule
/// (Documentation/input/event-codes.rst) is to drop everything up to the
/// next SYN_REPORT and read the device's state, which [`F13::resync`] takes.
#[derive(Debug, Default)]
pub struct F13 {
    down: bool,
    dropping: bool,
}

/// What one raw event asks of the reader.
#[derive(Debug, PartialEq, Eq)]
pub enum Step {
    Key(Key),
    /// Read the key state and pass it to [`F13::resync`].
    Resync,
    Nothing,
}

impl F13 {
    /// At open: F13 may already be down; its press was not seen, so its
    /// release will not count either.
    pub fn opened(down: bool) -> Self {
        F13 {
            down,
            dropping: false,
        }
    }

    pub fn event(&mut self, kind: u16, code: u16, value: i32, at: Duration) -> Step {
        match (kind, code) {
            (EV_SYN, SYN_DROPPED) => {
                self.dropping = true;
                Step::Nothing
            }
            (EV_SYN, SYN_REPORT) if self.dropping => {
                self.dropping = false;
                Step::Resync
            }
            _ if self.dropping => Step::Nothing,
            (EV_KEY, code) if code == KeyCode::KEY_F13.code() => match value {
                1 if !self.down => {
                    self.down = true;
                    Step::Key(Key::Press(at))
                }
                0 if self.down => {
                    self.down = false;
                    Step::Key(Key::Release(at))
                }
                _ => Step::Nothing, // autorepeat, or a repeat of the state
            },
            _ => Step::Nothing,
        }
    }

    /// The real state after a drop, read at `at`.
    pub fn resync(&mut self, down: bool, at: Duration) -> Option<Key> {
        match (self.down, down) {
            (false, true) => {
                self.down = true;
                Some(Key::Press(at))
            }
            (true, false) => {
                self.down = false;
                Some(Key::Release(at))
            }
            _ => None,
        }
    }

    pub fn lost(&mut self) -> Option<Key> {
        std::mem::take(&mut self.down).then_some(Key::Lost)
    }
}

/// An open Voyager node.
struct Node {
    device: RawDevice,
    f13: F13,
}

nix::ioctl_write_ptr!(eviocsclockid, b'E', 0xa0, nix::libc::c_int);

/// Puts the node's event times on CLOCK_BOOTTIME: it never steps, and counts
/// a suspend between a press and its release.
fn use_boottime(device: &RawDevice) -> io::Result<()> {
    let clock = nix::libc::CLOCK_BOOTTIME;
    // SAFETY: EVIOCSCLOCKID reads one int through the pointer, which is valid
    // for the call, from the device's own open fd.
    unsafe { eviocsclockid(device.as_raw_fd(), &clock) }
        .map(drop)
        .map_err(io::Error::from)
}

fn boottime() -> Duration {
    nix::time::clock_gettime(nix::time::ClockId::CLOCK_BOOTTIME)
        .map(Duration::from)
        .unwrap_or_default()
}

fn f13_down(device: &RawDevice) -> io::Result<bool> {
    Ok(device.get_key_state()?.contains(KeyCode::KEY_F13))
}

/// Opens the Voyager nodes not open yet.
fn scan(nodes: &mut BTreeMap<PathBuf, Node>) {
    let Ok(entries) = fs::read_dir("/dev/input") else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let is_event = path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("event"));
        if !is_event || nodes.contains_key(&path) {
            continue;
        }
        // not ours, not readable (yet: udev sets the group just after), or gone
        let Ok(device) = RawDevice::open(&path) else {
            continue;
        };
        if !wanted(device.input_id(), device.supported_keys()) {
            continue;
        }
        if let Err(error) = use_boottime(&device) {
            eprintln!("input: {}: EVIOCSCLOCKID: {error}", path.display());
        }
        let down = f13_down(&device).unwrap_or(false);
        eprintln!("input: reading F13 from {}", path.display());
        nodes.insert(
            path,
            Node {
                device,
                f13: F13::opened(down),
            },
        );
    }
}

/// Reads one node's pending events; Err once it is unusable.
fn read(node: &mut Node, send: &impl Fn(Key)) -> io::Result<()> {
    let events: Vec<_> = node.device.fetch_events()?.collect();
    for event in events {
        let at = event
            .timestamp()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        match node
            .f13
            .event(event.event_type().0, event.code(), event.value(), at)
        {
            Step::Key(key) => send(key),
            Step::Resync => {
                let down = f13_down(&node.device)?;
                if let Some(key) = node.f13.resync(down, boottime()) {
                    send(key);
                }
                break; // the rest of the batch is older than that state
            }
            Step::Nothing => {}
        }
    }
    Ok(())
}

/// Follows every Voyager node, across hotplug, forever.
pub fn run(send: impl Fn(Key)) -> ! {
    let watch = Inotify::init(InitFlags::IN_NONBLOCK | InitFlags::IN_CLOEXEC).and_then(|inotify| {
        inotify.add_watch(
            Path::new("/dev/input"),
            AddWatchFlags::IN_CREATE | AddWatchFlags::IN_ATTRIB,
        )?;
        Ok(inotify)
    });
    let watch = match watch {
        Ok(watch) => Some(watch),
        Err(error) => {
            eprintln!("input: inotify on /dev/input: {error}; rescanning every 5 s");
            None
        }
    };
    let mut nodes = BTreeMap::new();
    scan(&mut nodes);
    loop {
        let paths: Vec<PathBuf> = nodes.keys().cloned().collect();
        let (changed, ready) = {
            let mut fds: Vec<PollFd> = nodes
                .values()
                .map(|node| PollFd::new(node.device.as_fd(), PollFlags::POLLIN))
                .collect();
            if let Some(watch) = &watch {
                fds.push(PollFd::new(watch.as_fd(), PollFlags::POLLIN));
            }
            let timeout = if watch.is_some() {
                PollTimeout::NONE
            } else {
                PollTimeout::from(5000u16)
            };
            match poll(&mut fds, timeout) {
                Ok(_) | Err(Errno::EINTR) => {}
                Err(error) => {
                    eprintln!("input: poll: {error}");
                    std::thread::sleep(Duration::from_secs(1));
                }
            }
            let revents: Vec<bool> = fds
                .iter()
                .map(|fd| fd.revents().is_some_and(|events| !events.is_empty()))
                .collect();
            let changed = watch.is_none() || revents.last().copied().unwrap_or(false);
            (changed, revents)
        };
        for (path, &ready) in paths.iter().zip(&ready) {
            if !ready {
                continue;
            }
            let Some(node) = nodes.get_mut(path) else {
                continue;
            };
            if let Err(error) = read(node, &send) {
                if let Some(key) = node.f13.lost() {
                    send(key);
                }
                if error.raw_os_error() != Some(Errno::ENODEV as i32) {
                    eprintln!("input: {}: {error}", path.display());
                }
                eprintln!("input: {} gone", path.display());
                nodes.remove(path);
            }
        }
        if changed {
            if let Some(watch) = &watch {
                while matches!(watch.read_events(), Ok(events) if !events.is_empty()) {}
            }
            scan(&mut nodes);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use evdev::{AttributeSet, BusType};

    const F13: u16 = 183;
    const A: u16 = 30;

    fn ms(ms: u64) -> Duration {
        Duration::from_millis(ms)
    }

    /// Feeds raw events, as the kernel would hand them over, and collects the
    /// keys; `state` answers the key-state read of a resync.
    fn feed(f13: &mut F13, events: &[(u16, u16, i32, u64)], state: bool) -> Vec<Key> {
        let mut keys = Vec::new();
        for &(kind, code, value, at) in events {
            match f13.event(kind, code, value, ms(at)) {
                Step::Key(key) => keys.push(key),
                Step::Resync => keys.extend(f13.resync(state, ms(at))),
                Step::Nothing => {}
            }
        }
        keys
    }

    #[test]
    fn f13_press_and_release_come_through_in_order_and_nothing_else_does() {
        let mut f13 = F13::default();
        let keys = feed(
            &mut f13,
            &[
                (EV_KEY, A, 1, 5),
                (EV_SYN, SYN_REPORT, 0, 5),
                (EV_KEY, F13, 1, 10),
                (EV_SYN, SYN_REPORT, 0, 10),
                (EV_KEY, F13, 2, 500), // autorepeat
                (EV_KEY, A, 0, 600),
                (EV_KEY, F13, 0, 900),
            ],
            false,
        );
        assert_eq!(KeyCode::KEY_F13.code(), F13);
        assert_eq!(keys, [Key::Press(ms(10)), Key::Release(ms(900))]);
    }

    #[test]
    fn a_drop_hiding_the_release_is_resynchronised_at_its_report() {
        let mut f13 = F13::default();
        let keys = feed(
            &mut f13,
            &[
                (EV_KEY, F13, 1, 10),
                (EV_SYN, SYN_DROPPED, 0, 20),
                (EV_KEY, F13, 0, 30), // not trusted
                (EV_SYN, SYN_REPORT, 0, 40),
            ],
            false,
        );
        assert_eq!(keys, [Key::Press(ms(10)), Key::Release(ms(40))]);
        // and one hiding a press shows it, late
        let keys = feed(
            &mut f13,
            &[(EV_SYN, SYN_DROPPED, 0, 50), (EV_SYN, SYN_REPORT, 0, 60)],
            true,
        );
        assert_eq!(keys, [Key::Press(ms(60))]);
    }

    #[test]
    fn a_node_lost_with_f13_down_is_a_release_and_otherwise_nothing() {
        let mut f13 = F13::default();
        feed(&mut f13, &[(EV_KEY, F13, 1, 10)], false);
        assert_eq!(f13.lost(), Some(Key::Lost));
        assert_eq!(f13.lost(), None);
    }

    #[test]
    fn a_key_already_down_at_open_has_its_release_ignored_too() {
        let mut f13 = F13::opened(true);
        // its release goes through; the machine finds no press to match
        assert_eq!(
            feed(&mut f13, &[(EV_KEY, F13, 0, 10)], false),
            [Key::Release(ms(10))]
        );
        assert_eq!(F13::opened(true).lost(), Some(Key::Lost));
    }

    #[test]
    fn only_voyager_nodes_with_f13_are_read() {
        let f13: AttributeSet<KeyCode> = [KeyCode::KEY_F13, KeyCode::KEY_A].into_iter().collect();
        let plain: AttributeSet<KeyCode> = [KeyCode::KEY_A].into_iter().collect();
        let voyager = || InputId::new(BusType::BUS_USB, 0x3297, 0x1977, 1);
        let moonlander = InputId::new(BusType::BUS_USB, 0x3297, 0x1969, 1);
        assert!(wanted(voyager(), Some(&f13)));
        assert!(
            !wanted(voyager(), Some(&plain)),
            "its mouse or consumer node"
        );
        assert!(!wanted(voyager(), None));
        assert!(!wanted(moonlander, Some(&f13)));
    }
}
