//! F13 from the Voyager, read from its evdev node(s) without grabbing them.
//! Per-fd masks allow only F13 key events and kernel synchronization events.
//! Events queued before mask installation can exist; the reader ignores other
//! types/codes without inspecting their value or timestamp. State recovery
//! briefly receives a key bitmap, retains only F13, and wipes that bitmap.
//! Event times are the kernel's, on CLOCK_BOOTTIME.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::os::fd::{AsFd, AsRawFd};
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

use evdev::raw_stream::RawDevice;
use evdev::{AttributeSetRef, InputEvent, InputId, KeyCode};
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
    /// An accepted press became uncertain, or its node went away.
    Lost,
}

/// A Voyager node that can send F13.
pub fn wanted(id: InputId, keys: Option<&AttributeSetRef<KeyCode>>) -> bool {
    id.vendor() == VENDOR
        && id.product() == PRODUCT
        && keys.is_some_and(|keys| keys.contains(KeyCode::KEY_F13))
}

/// One node's F13. SYN_DROPPED terminates an accepted hold with Lost; after
/// the next SYN_REPORT, a snapshot suppresses any down key until release.
/// No recovered press can become a tap/latch.
#[derive(Debug, Default)]
pub struct F13 {
    /// A press actually delivered to the machine.
    down: bool,
    dropping: bool,
    /// A startup/recovered hold whose press was not reliably observed.
    suppressed: bool,
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
    /// At open: an already-held key is suppressed through its release.
    pub fn opened(down: bool) -> Self {
        F13 {
            suppressed: down,
            ..Self::default()
        }
    }

    pub fn event(&mut self, kind: u16, code: u16, value: i32, at: Duration) -> Step {
        match (kind, code) {
            (EV_SYN, SYN_DROPPED) => {
                self.dropping = true;
                self.lost().map_or(Step::Nothing, Step::Key)
            }
            (EV_SYN, SYN_REPORT) if self.dropping => {
                self.dropping = false;
                Step::Resync
            }
            _ if self.dropping => Step::Nothing,
            (EV_KEY, code) if code == KeyCode::KEY_F13.code() => match value {
                1 if !self.down && !self.suppressed => {
                    self.down = true;
                    Step::Key(Key::Press(at))
                }
                0 if self.suppressed => {
                    self.suppressed = false;
                    Step::Nothing
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

    /// Adopt the snapshot without inventing a press time. A down key remains
    /// suppressed even if it was down before the overflow: a release/repress
    /// could have been lost in between.
    pub fn resync(&mut self, down: bool) {
        self.down = false;
        self.suppressed = down;
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
nix::ioctl_write_ptr!(eviocsmask, b'E', 0x93, InputMask);
nix::ioctl_read_buf!(eviocgkey, b'E', 0x18, nix::libc::c_ulong);

/// Linux input.h's input_mask; codes_ptr is a u64 even on 32-bit targets.
#[repr(C)]
pub struct InputMask {
    kind: u32,
    codes_size: u32,
    codes_ptr: u64,
}

const WORD_BITS: usize = nix::libc::c_ulong::BITS as usize;
const F13_CODE: usize = KeyCode::KEY_F13.code() as usize;
const KEY_WORDS: usize = F13_CODE / WORD_BITS + 1;
const TYPE_MASK: [nix::libc::c_ulong; 1] = [1 << EV_KEY];
const KEY_MASK: [nix::libc::c_ulong; KEY_WORDS] = {
    let mut mask = [0; KEY_WORDS];
    mask[F13_CODE / WORD_BITS] = 1 << (F13_CODE % WORD_BITS);
    mask
};

fn set_mask(device: &RawDevice, kind: u16, codes: &[nix::libc::c_ulong]) -> io::Result<()> {
    let mask = InputMask {
        kind: u32::from(kind),
        codes_size: std::mem::size_of_val(codes) as u32,
        codes_ptr: codes.as_ptr() as u64,
    };
    // SAFETY: input_mask and its native-word bitmap both live through this
    // call; their ABI and lengths match Linux EVIOCSMASK.
    unsafe { eviocsmask(device.as_raw_fd(), &mask) }
        .map(drop)
        .map_err(io::Error::from)
}

fn filter_f13(device: &RawDevice) -> io::Result<()> {
    // Linux drivers/input/evdev.c: mask type 0 (EV_SYN) is the event-TYPE
    // bitmap, not a SYN-code bitmap. SYN events always bypass the masks.
    // Short code masks clear all remaining codes. Neither ioctl flushes
    // events queued before installation, and neither affects other clients.
    set_mask(device, EV_KEY, &KEY_MASK)?;
    set_mask(device, EV_SYN, &TYPE_MASK)
}

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

fn f13_down(device: &RawDevice) -> io::Result<bool> {
    // EVIOCGKEY returns a bitmap, not a single key. Request only native words
    // through F13; preceding unrelated key bits are unavoidable. Linux takes
    // the snapshot and flushes queued EV_KEY under its buffer lock. Events
    // already fetched into userspace must be discarded BEFORE this ioctl.
    let mut bits = [0; KEY_WORDS];
    // SAFETY: the ioctl writes at most the supplied slice's byte length.
    let result = unsafe { eviocgkey(device.as_raw_fd(), &mut bits) }
        .map_err(io::Error::from)
        .and_then(|bytes| {
            if bytes as usize == std::mem::size_of_val(&bits) {
                Ok(bits[F13_CODE / WORD_BITS] & KEY_MASK[F13_CODE / WORD_BITS] != 0)
            } else {
                Err(io::Error::other("short EVIOCGKEY bitmap"))
            }
        });
    for word in &mut bits {
        // SAFETY: each pointer refers to a live, exclusively borrowed word.
        // Volatile writes prevent elimination of this best-effort wipe; this
        // is not a guarantee against copies made by the kernel/compiler.
        unsafe { std::ptr::write_volatile(word, 0) };
    }
    result
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
        if let Err(error) = filter_f13(&device) {
            eprintln!("input: {}: EVIOCSMASK: {error}", path.display());
            continue; // never fetch from an unfiltered fd
        }
        if let Err(error) = use_boottime(&device) {
            eprintln!("input: {}: EVIOCSCLOCKID: {error}", path.display());
            continue; // never interpret timestamps from an unknown clock
        }
        // Changing the clock flushes pending old-clock events (and may queue
        // SYN_DROPPED). The snapshot also flushes any queued EV_KEY.
        let down = match f13_down(&device) {
            Ok(down) => down,
            Err(error) => {
                eprintln!("input: {}: EVIOCGKEY: {error}", path.display());
                continue;
            }
        };
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

/// Inspect payload/time lazily, after accepting the type/code. SYN payloads
/// and all events during overflow recovery are irrelevant.
fn reader_event(
    f13: &mut F13,
    kind: u16,
    code: u16,
    payload: impl FnOnce() -> (i32, Duration),
) -> Step {
    match (kind, code) {
        (EV_SYN, SYN_DROPPED | SYN_REPORT) => f13.event(kind, code, 0, Duration::ZERO),
        (EV_KEY, code) if code == KeyCode::KEY_F13.code() && !f13.dropping => {
            let (value, at) = payload();
            f13.event(kind, code, value, at)
        }
        _ => Step::Nothing,
    }
}

/// Consume one fetched batch; true requests a snapshot. Own the iterator so
/// returning drops evdev's Drain and discards every unconsumed, older event
/// before the caller issues EVIOCGKEY. No second raw-event buffer is made.
pub(crate) fn read_batch(
    f13: &mut F13,
    events: impl Iterator<Item = InputEvent>,
    mut send: impl FnMut(Key),
) -> bool {
    for event in events {
        match reader_event(f13, event.event_type().0, event.code(), || {
            let at = event
                .timestamp()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default();
            (event.value(), at)
        }) {
            Step::Key(key) => send(key),
            Step::Resync => return true,
            Step::Nothing => {}
        }
    }
    false
}

/// Reads one node's pending events; Err once it is unusable.
fn read(node: &mut Node, send: &impl Fn(Key)) -> io::Result<()> {
    if read_batch(&mut node.f13, node.device.fetch_events()?, send) {
        let down = f13_down(&node.device)?;
        node.f13.resync(down);
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

    fn raw(kind: u16, code: u16, value: i32, at: u64) -> InputEvent {
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

    /// Exercise the actual reader boundary with evdev's Drain semantics,
    /// without opening any device. `state` supplies the post-drop snapshot.
    fn feed(f13: &mut F13, events: &[(u16, u16, i32, u64)], state: bool) -> Vec<Key> {
        let mut keys = Vec::new();
        let mut buffer: Vec<_> = events
            .iter()
            .map(|&(kind, code, value, at)| raw(kind, code, value, at))
            .collect();
        let snapshot = read_batch(f13, buffer.drain(..), |key| keys.push(key));
        assert!(
            buffer.is_empty(),
            "drop the entire batch before the snapshot"
        );
        if snapshot {
            f13.resync(state);
        }
        keys
    }

    #[test]
    fn masks_allow_only_key_type_and_f13_code() {
        assert_eq!(TYPE_MASK[0], 1 << EV_KEY);
        // EV_SYN bypasses filtering in the kernel; bit 0 is NOT a SYN mask.
        assert_eq!(TYPE_MASK[0] & (1 << EV_SYN), 0);
        for bit in 0..KEY_WORDS * WORD_BITS {
            assert_eq!(
                KEY_MASK[bit / WORD_BITS] & (1 << (bit % WORD_BITS)) != 0,
                bit == F13_CODE
            );
        }
        // EVIOCSMASK zero-fills all codes beyond this short bitmap.
        assert_eq!(std::mem::size_of::<InputMask>(), 16);
    }

    #[test]
    fn reader_boundary_ignores_other_key_and_scancode_payloads() {
        let mut f13 = F13::default();
        for (kind, code) in [
            (EV_KEY, A),
            (4, 4),      // EV_MSC / MSC_SCAN
            (4, F13),    // a non-key event with the same numeric code
            (EV_SYN, 1), // SYN_CONFIG is not needed
            (EV_SYN, SYN_REPORT),
        ] {
            assert_eq!(
                reader_event(&mut f13, kind, code, || panic!("unrelated payload read")),
                Step::Nothing
            );
        }
        assert_eq!(
            reader_event(&mut f13, EV_SYN, SYN_DROPPED, || panic!("SYN payload read")),
            Step::Nothing
        );
        assert_eq!(
            reader_event(&mut f13, EV_KEY, F13, || panic!("dropped payload read")),
            Step::Nothing
        );
        assert_eq!(
            reader_event(&mut f13, EV_SYN, SYN_REPORT, || panic!("SYN payload read")),
            Step::Resync
        );
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
    fn a_drop_hiding_the_release_terminates_without_a_short_release() {
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
        assert_eq!(keys, [Key::Press(ms(10)), Key::Lost]);
        assert_eq!(
            feed(
                &mut f13,
                &[(EV_KEY, F13, 1, 50), (EV_KEY, F13, 0, 400)],
                false
            ),
            [Key::Press(ms(50)), Key::Release(ms(400))]
        );
    }

    #[test]
    fn losing_an_unseen_long_press_never_synthesizes_a_tap() {
        let mut f13 = F13::default();
        // The unseen physical press was at 0; snapshot discovers it at 1 s.
        assert!(
            feed(
                &mut f13,
                &[(EV_SYN, SYN_DROPPED, 0, 990), (EV_SYN, SYN_REPORT, 0, 1000)],
                true
            )
            .is_empty()
        );
        assert!(
            feed(
                &mut f13,
                &[(EV_KEY, F13, 2, 1010), (EV_KEY, F13, 0, 1020)],
                false
            )
            .is_empty()
        );
        assert_eq!(
            feed(
                &mut f13,
                &[(EV_KEY, F13, 1, 2000), (EV_KEY, F13, 0, 2300)],
                false
            ),
            [Key::Press(ms(2000)), Key::Release(ms(2300))]
        );
    }

    #[test]
    fn losing_a_release_and_repress_while_down_ends_the_original_hold() {
        let mut f13 = F13::default();
        assert_eq!(
            feed(
                &mut f13,
                &[
                    (EV_KEY, F13, 1, 10),
                    (EV_SYN, SYN_DROPPED, 0, 1000),
                    (EV_KEY, F13, 0, 1010), // uncertain release/repress
                    (EV_KEY, F13, 1, 1020),
                    (EV_SYN, SYN_REPORT, 0, 1100),
                ],
                true
            ),
            [Key::Press(ms(10)), Key::Lost]
        );
        assert!(
            feed(
                &mut f13,
                &[(EV_KEY, F13, 1, 1110), (EV_KEY, F13, 0, 1120)],
                false
            )
            .is_empty()
        );
        assert_eq!(
            feed(
                &mut f13,
                &[(EV_KEY, F13, 1, 2000), (EV_KEY, F13, 0, 2300)],
                false
            ),
            [Key::Press(ms(2000)), Key::Release(ms(2300))]
        );
    }

    #[test]
    fn dropped_hold_is_terminated_before_waiting_for_a_report() {
        let mut f13 = F13::default();
        assert_eq!(
            feed(
                &mut f13,
                &[(EV_KEY, F13, 1, 10), (EV_SYN, SYN_DROPPED, 0, 20)],
                false
            ),
            [Key::Press(ms(10)), Key::Lost]
        );
        assert!(feed(&mut f13, &[(EV_SYN, SYN_DROPPED, 0, 30)], false).is_empty());
        assert_eq!(f13.lost(), None, "never deliver the same loss twice");
    }

    #[test]
    fn stale_events_after_the_report_are_discarded_before_snapshot() {
        for down in [false, true] {
            let mut f13 = F13::default();
            assert!(
                feed(
                    &mut f13,
                    &[
                        (EV_SYN, SYN_DROPPED, 0, 10),
                        (EV_SYN, SYN_REPORT, 0, 20),
                        (EV_KEY, F13, 0, 30), // would clear suppression
                        (EV_KEY, F13, 1, 40), // would invent a press
                        (EV_KEY, F13, 0, 50), // would turn it into a tap
                    ],
                    down
                )
                .is_empty()
            );
            // If the snapshot was down, the next real release only lifts
            // suppression. If up, this unmatched release is also ignored.
            assert!(feed(&mut f13, &[(EV_KEY, F13, 0, 60)], false).is_empty());
            assert_eq!(
                feed(&mut f13, &[(EV_KEY, F13, 1, 70)], false),
                [Key::Press(ms(70))]
            );
        }
    }

    #[test]
    fn a_node_lost_with_an_accepted_press_emits_lost_only_once() {
        let mut f13 = F13::default();
        feed(&mut f13, &[(EV_KEY, F13, 1, 10)], false);
        assert_eq!(f13.lost(), Some(Key::Lost));
        assert_eq!(f13.lost(), None);
    }

    #[test]
    fn a_key_already_down_at_open_has_its_release_ignored_too() {
        let mut f13 = F13::opened(true);
        assert!(
            feed(
                &mut f13,
                &[
                    (EV_KEY, F13, 1, 5),
                    (EV_KEY, F13, 2, 7),
                    (EV_KEY, F13, 0, 10)
                ],
                false
            )
            .is_empty()
        );
        assert_eq!(
            feed(
                &mut f13,
                &[(EV_KEY, F13, 1, 20), (EV_KEY, F13, 0, 400)],
                false
            ),
            [Key::Press(ms(20)), Key::Release(ms(400))]
        );
        assert_eq!(F13::opened(true).lost(), None);
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
