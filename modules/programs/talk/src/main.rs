//! Push-to-talk on one key.
//!
//! Hyprland runs `talk press` when the key goes down and `talk release` when
//! it comes up. A press inverts the default source's mute, so holding the key
//! talks through a muted mic and mutes a live one; letting go restores the
//! previous state. A tap shorter than [`LATCH_MS`] latches instead: the
//! inverted state stays until the next press, whose own release is ignored.
//!
//! Every invocation is its own process, so the phase lives in
//! `$XDG_RUNTIME_DIR/talk/state`, behind a flock on `lock` in the same
//! directory: a press and its release can race for it, in either order. The
//! phase names the source node it inverted, so a default that changes
//! mid-cycle (a headset plugged or dropped) is never restored in its place.
//! Hyprland drops exec output, so errors also go to the notification daemon
//! (mako) through notify-send, once per distinct message, after the lock is
//! released.

use std::env;
use std::fmt;
use std::fs::{self, File, TryLockError};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, ExitStatus, Stdio};
use std::str::FromStr;
use std::thread;
use std::time::{Duration, Instant};

use nix::time::{ClockId, clock_gettime};

/// A release sooner than this after its press latches.
const LATCH_MS: u64 = 250;

/// Longest wait for the lock, and for each wpctl.
const DEADLINE: Duration = Duration::from_secs(2);

const POLL: Duration = Duration::from_millis(2);

type Result<T, E = String> = std::result::Result<T, E>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Event {
    Press,
    Release,
}

/// A PipeWire node: its id, which wpctl addresses, and its serial, which is
/// never reused, unlike the id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Node {
    id: u32,
    serial: u64,
}

/// The default source and its mute, as wpctl gives them.
trait Mic {
    fn default_source(&mut self) -> Result<(Node, bool)>;
    /// Whether `node` is muted; None once it is gone.
    fn muted(&mut self, node: Node) -> Result<Option<bool>>;
    fn set_muted(&mut self, node: Node, muted: bool) -> Result<()>;
}

/// Where the key's cycle stands; `node` was inverted, from `was_muted`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Idle,
    /// Inverted by a press at `since` (ms of CLOCK_BOOTTIME).
    Held {
        since: u64,
        node: Node,
        was_muted: bool,
    },
    /// Inverted by a tap, until the next press.
    Latched {
        node: Node,
        was_muted: bool,
    },
    /// Restored by a press, whose release is not a tap.
    Ending,
}

/// What a step does to the mic.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Effect {
    Nothing,
    Mute(Node, bool),
    /// The inverted node is gone: nothing to restore.
    Gone(Node),
}

impl Phase {
    /// The effect of `event` at `now`, and the phase after it. `mic` is read
    /// only when a press engages or something is to be restored.
    ///
    /// A press always settles a cycle still going, so lost or reordered
    /// events cannot wedge talk. If the inversion was undone outside talk
    /// meanwhile (the mic mute key, another client), that press starts a new
    /// hold of the same node instead of restoring what already is. A release
    /// handled before its own press (the two processes raced for the lock)
    /// leaves a Held that nothing releases: the next press ends it, as it
    /// would a latch. A release lost after an ending press leaves Ending,
    /// which the next press treats as idle. A node gone mid-cycle ends it
    /// with nothing restored.
    fn step(self, event: Event, now: u64, mic: &mut impl Mic) -> Result<(Effect, Phase)> {
        let engage = |node: Node, was_muted: bool| {
            (
                Effect::Mute(node, !was_muted),
                Phase::Held {
                    since: now,
                    node,
                    was_muted,
                },
            )
        };
        Ok(match (self, event) {
            (Phase::Idle | Phase::Ending, Event::Press) => {
                let (node, muted) = mic.default_source()?;
                engage(node, muted)
            }
            (
                Phase::Held {
                    node, was_muted, ..
                }
                | Phase::Latched { node, was_muted },
                Event::Press,
            ) => match mic.muted(node)? {
                None => (Effect::Gone(node), Phase::Idle),
                Some(muted) if muted == was_muted => engage(node, muted),
                Some(_) => (Effect::Mute(node, was_muted), Phase::Ending),
            },
            (
                Phase::Held {
                    since,
                    node,
                    was_muted,
                },
                Event::Release,
            ) if now.saturating_sub(since) < LATCH_MS => {
                (Effect::Nothing, Phase::Latched { node, was_muted })
            }
            (
                Phase::Held {
                    node, was_muted, ..
                },
                Event::Release,
            ) => match mic.muted(node)? {
                None => (Effect::Gone(node), Phase::Idle),
                // undone outside talk already
                Some(muted) if muted == was_muted => (Effect::Nothing, Phase::Idle),
                Some(_) => (Effect::Mute(node, was_muted), Phase::Idle),
            },
            (Phase::Ending, Event::Release) => (Effect::Nothing, Phase::Idle),
            (phase @ (Phase::Idle | Phase::Latched { .. }), Event::Release) => {
                (Effect::Nothing, phase)
            }
        })
    }
}

/// The state file's one line: `idle`, `held <since> <id> <serial> <0|1>`,
/// `latched <id> <serial> <0|1>` or `ending`.
impl fmt::Display for Phase {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            Phase::Idle => write!(f, "idle"),
            Phase::Held {
                since,
                node,
                was_muted,
            } => write!(
                f,
                "held {since} {} {} {}",
                node.id,
                node.serial,
                u8::from(was_muted)
            ),
            Phase::Latched { node, was_muted } => write!(
                f,
                "latched {} {} {}",
                node.id,
                node.serial,
                u8::from(was_muted)
            ),
            Phase::Ending => write!(f, "ending"),
        }
    }
}

impl FromStr for Phase {
    type Err = String;

    fn from_str(line: &str) -> Result<Phase> {
        let flag = |word: &str| match word {
            "0" => Some(false),
            "1" => Some(true),
            _ => None,
        };
        let node = |id: &str, serial: &str| {
            Some(Node {
                id: id.parse().ok()?,
                serial: serial.parse().ok()?,
            })
        };
        let phase = |words: &[&str]| match *words {
            ["idle"] => Some(Phase::Idle),
            ["held", since, id, serial, was_muted] => Some(Phase::Held {
                since: since.parse().ok()?,
                node: node(id, serial)?,
                was_muted: flag(was_muted)?,
            }),
            ["latched", id, serial, was_muted] => Some(Phase::Latched {
                node: node(id, serial)?,
                was_muted: flag(was_muted)?,
            }),
            ["ending"] => Some(Phase::Ending),
            _ => None,
        };
        phase(&line.split_whitespace().collect::<Vec<_>>())
            .ok_or_else(|| format!("unreadable state {line:?}"))
    }
}

/// Whether `wpctl get-volume` reports the node muted: `Volume: 1.00`, or
/// `Volume: 1.00 [MUTED]`.
fn parse_muted(output: &str) -> Option<bool> {
    let mut words = output.trim().strip_prefix("Volume:")?.split_whitespace();
    words.next()?.parse::<f64>().ok()?;
    match (words.next(), words.next()) {
        (None, _) => Some(false),
        (Some("[MUTED]"), None) => Some(true),
        _ => None,
    }
}

/// The object `wpctl inspect` describes: its first line is
/// `id 57, type PipeWire:Interface:Node`, and its properties, one per line,
/// include `  * object.serial = "1234"`.
fn parse_inspect(output: &str) -> Option<Node> {
    let mut lines = output.lines();
    let (id, _type) = lines.next()?.strip_prefix("id ")?.split_once(", type ")?;
    let serial = lines.find_map(|line| {
        let property = line.trim_start();
        let property = property.strip_prefix('*').unwrap_or(property).trim_start();
        property
            .strip_prefix("object.serial = \"")?
            .strip_suffix('"')
    })?;
    Some(Node {
        id: id.parse().ok()?,
        serial: serial.parse().ok()?,
    })
}

/// A finished command's status and output.
struct Reply {
    status: ExitStatus,
    stdout: String,
    stderr: String,
}

/// Runs `command`, killing it past `deadline`. Its output must fit the pipe
/// buffers (64 KiB), as wpctl's does, since it is read only once it exits.
fn run_until(mut command: Command, deadline: Duration) -> Result<Reply> {
    let name = [command.get_program()]
        .into_iter()
        .chain(command.get_args())
        .map(|word| word.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ");
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("{name}: {error}"))?;
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() < deadline => thread::sleep(POLL),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("{name}: no answer in {deadline:?}"));
            }
            Err(error) => return Err(format!("{name}: {error}")),
        }
    }
    // reaped already: this only drains the pipes
    let output = child
        .wait_with_output()
        .map_err(|error| format!("{name}: {error}"))?;
    Ok(Reply {
        status: output.status,
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

/// wpctl, with each call bounded by [`DEADLINE`].
struct Wpctl;

impl Wpctl {
    fn call(args: &[&str]) -> Result<Reply> {
        let mut command = Command::new("wpctl");
        command.args(args);
        run_until(command, DEADLINE)
    }

    /// wpctl's own words, for errors: it reports on stderr, sometimes while
    /// exiting 0.
    fn failure(args: &[&str], reply: &Reply, what: &str) -> String {
        format!(
            "wpctl {}: {what} ({}): {:?}",
            args.join(" "),
            reply.status,
            reply.stderr.trim()
        )
    }

    /// The object `target` names; None if a numeric `target` names none.
    fn inspect(target: &str) -> Result<Option<Node>> {
        let args = ["inspect", target];
        let reply = Wpctl::call(&args)?;
        if !reply.status.success() {
            if reply.stderr.trim() == format!("Object '{target}' not found") {
                return Ok(None);
            }
            return Err(Wpctl::failure(&args, &reply, "failed"));
        }
        parse_inspect(&reply.stdout)
            .map(Some)
            .ok_or_else(|| Wpctl::failure(&args, &reply, "unexpected output"))
    }

    /// Whether node `id` is muted; None if there is no such node.
    fn get_volume(id: u32) -> Result<Option<bool>> {
        let id = id.to_string();
        let args = ["get-volume", &id];
        let reply = Wpctl::call(&args)?;
        // exits 0 when the node is missing, saying so on stderr only
        if reply.stdout.is_empty() && reply.stderr.trim() == format!("Node '{id}' not found") {
            return Ok(None);
        }
        match parse_muted(&reply.stdout) {
            Some(muted) if reply.status.success() => Ok(Some(muted)),
            _ => Err(Wpctl::failure(
                &args,
                &reply,
                &format!("unexpected output {:?}", reply.stdout),
            )),
        }
    }
}

impl Mic for Wpctl {
    fn default_source(&mut self) -> Result<(Node, bool)> {
        let node = Wpctl::inspect("@DEFAULT_AUDIO_SOURCE@")?
            .ok_or_else(|| "no default source".to_owned())?;
        let muted = Wpctl::get_volume(node.id)?
            .ok_or_else(|| format!("the default source, node {}, went away", node.id))?;
        Ok((node, muted))
    }

    fn muted(&mut self, node: Node) -> Result<Option<bool>> {
        match Wpctl::inspect(&node.id.to_string())? {
            Some(found) if found == node => Wpctl::get_volume(node.id),
            _ => Ok(None), // gone, its id possibly reused
        }
    }

    fn set_muted(&mut self, node: Node, muted: bool) -> Result<()> {
        let id = node.id.to_string();
        let args = ["set-mute", &id, if muted { "1" } else { "0" }];
        let reply = Wpctl::call(&args)?;
        if !reply.status.success() {
            return Err(Wpctl::failure(&args, &reply, "failed"));
        }
        Ok(())
    }
}

/// Milliseconds of CLOCK_BOOTTIME: monotonic, and counting a suspend that
/// falls between a press and its release.
fn now_ms() -> Result<u64> {
    let now = clock_gettime(ClockId::CLOCK_BOOTTIME)
        .map_err(|error| format!("clock_gettime(CLOCK_BOOTTIME): {error}"))?;
    Ok(now.tv_sec() as u64 * 1000 + now.tv_nsec() as u64 / 1_000_000)
}

fn parse_event(mut args: impl Iterator<Item = String>) -> Result<Event> {
    match (args.next().as_deref(), args.next()) {
        (Some("press"), None) => Ok(Event::Press),
        (Some("release"), None) => Ok(Event::Release),
        _ => Err("usage: talk press|release".to_owned()),
    }
}

/// Holds the directory's flock until dropped; the kernel drops it with the
/// process, so a crashed invocation cannot leave it held. Gives up after
/// `deadline`, rather than queueing behind a stuck run.
fn lock(dir: &Path, deadline: Duration) -> Result<File> {
    fs::create_dir_all(dir).map_err(|error| format!("{}: {error}", dir.display()))?;
    let path = dir.join("lock");
    let file = File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .map_err(|error| format!("{}: {error}", path.display()))?;
    let started = Instant::now();
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(file),
            Err(TryLockError::WouldBlock) if started.elapsed() < deadline => thread::sleep(POLL),
            Err(TryLockError::WouldBlock) => {
                return Err(format!(
                    "{}: still locked after {deadline:?}",
                    path.display()
                ));
            }
            Err(TryLockError::Error(error)) => {
                return Err(format!("flock {}: {error}", path.display()));
            }
        }
    }
}

/// Handles `event` under the lock; true if it set the mute. Problems that
/// do not stop it go to `notes`.
fn run(
    dir: &Path,
    event: Event,
    now: u64,
    mic: &mut impl Mic,
    notes: &mut Vec<String>,
) -> Result<bool> {
    let path = dir.join("state");
    let write = |phase: Phase| {
        fs::write(&path, format!("{phase}\n"))
            .map_err(|error| format!("{}: {error}", path.display()))
    };
    let stored = match fs::read(&path) {
        Ok(bytes) => Some(String::from_utf8_lossy(&bytes).into_owned()),
        Err(error) if error.kind() == ErrorKind::NotFound => None,
        Err(error) => return Err(format!("{}: {error}", path.display())),
    };
    let phase = stored.as_deref().map_or(Phase::Idle, |line| {
        line.parse().unwrap_or_else(|error| {
            notes.push(format!("{}: {error}; starting over", path.display()));
            Phase::Idle
        })
    });
    let (effect, next) = phase.step(event, now, mic)?;
    // the next phase is saved before the mic changes, so a run dying in
    // between leaves a phase whose restore is harmless, never an inverted
    // mic with nothing to restore it
    if stored.as_deref() != Some(format!("{next}\n").as_str()) {
        write(next)?;
    }
    match effect {
        Effect::Nothing => Ok(false),
        Effect::Mute(node, muted) => match mic.set_muted(node, muted) {
            Ok(()) => Ok(true),
            Err(error) => Err(match write(phase) {
                Ok(()) => error,
                Err(rollback) => format!("{error}; {rollback}"),
            }),
        },
        Effect::Gone(node) => Err(format!(
            "the source talk inverted (node {}) is gone; nothing restored",
            node.id
        )),
    }
}

/// Of `messages`, those to notify: not the one notified last. A run that set
/// the mute forgets it first; a key used on a broken setup would otherwise
/// notify at every press. Runs under the lock.
fn unreported(dir: &Path, set: bool, messages: Vec<String>) -> Vec<String> {
    let path = dir.join("reported");
    if set {
        match fs::remove_file(&path) {
            Err(error) if error.kind() != ErrorKind::NotFound => {
                eprintln!("talk: {}: {error}", path.display());
            }
            _ => {}
        }
    }
    let mut last = fs::read_to_string(&path).ok();
    let mut fresh = Vec::new();
    for message in messages {
        eprintln!("talk: {message}");
        if last.as_deref() == Some(message.as_str()) {
            continue;
        }
        if let Err(error) = fs::write(&path, &message) {
            eprintln!("talk: {}: {error}", path.display());
        }
        last = Some(message.clone());
        fresh.push(message);
    }
    fresh
}

fn notify(message: &str) {
    let sent = Command::new("notify-send")
        .args(["--app-name=talk", "--urgency=critical", "talk", message])
        .status();
    match sent {
        Ok(status) if status.success() => {}
        Ok(status) => eprintln!("talk: notify-send: {status}"),
        Err(error) => eprintln!("talk: notify-send: {error}"),
    }
}

fn main() -> ExitCode {
    let event = match parse_event(env::args().skip(1)) {
        Ok(event) => event,
        Err(usage) => {
            eprintln!("{usage}");
            return ExitCode::from(2);
        }
    };
    // the key's time is when Hyprland ran us, not when the lock came free
    let now = now_ms();
    let fail = |message: &str| {
        eprintln!("talk: {message}");
        notify(message);
        ExitCode::FAILURE
    };
    let Some(runtime) = env::var_os("XDG_RUNTIME_DIR") else {
        return fail("XDG_RUNTIME_DIR is not set");
    };
    let dir = PathBuf::from(runtime).join("talk");
    let lock = match lock(&dir, DEADLINE) {
        Ok(lock) => lock,
        Err(error) => return fail(&error),
    };
    let mut notes = Vec::new();
    let result = now.and_then(|now| run(&dir, event, now, &mut Wpctl, &mut notes));
    let set = result.as_ref().is_ok_and(|&set| set);
    let code = match result {
        Ok(_) => ExitCode::SUCCESS,
        Err(error) => {
            notes.push(error);
            ExitCode::FAILURE
        }
    };
    let fresh = unreported(&dir, set, notes);
    drop(lock);
    for message in &fresh {
        notify(message);
    }
    code
}

#[cfg(test)]
mod tests {
    use super::*;
    use Event::{Press, Release};

    const LIVE: bool = false;
    const MUTED: bool = true;
    const MIC: Node = Node {
        id: 57,
        serial: 1_001,
    };
    const HEADSET: Node = Node {
        id: 84,
        serial: 2_002,
    };

    /// PipeWire's nodes, and which one is the default source.
    struct FakeMic {
        default: Node,
        nodes: Vec<(Node, bool)>,
        reads: usize,
        /// the state file, as it stood when the mute was set
        state: Option<PathBuf>,
        seen: Vec<String>,
        broken: bool,
    }

    impl FakeMic {
        fn new(muted: bool) -> Self {
            FakeMic {
                default: MIC,
                nodes: vec![(MIC, muted)],
                reads: 0,
                state: None,
                seen: Vec::new(),
                broken: false,
            }
        }

        fn get(&self, node: Node) -> Option<bool> {
            self.nodes
                .iter()
                .find_map(|&(found, muted)| (found == node).then_some(muted))
        }

        /// The mute key, or any client other than talk.
        fn toggle(&mut self, node: Node) {
            let muted = self.get(node).unwrap();
            self.set_muted(node, !muted).unwrap();
        }
    }

    impl Mic for FakeMic {
        fn default_source(&mut self) -> Result<(Node, bool)> {
            self.reads += 1;
            if self.broken {
                return Err("wpctl: gone".to_owned());
            }
            Ok((self.default, self.get(self.default).unwrap()))
        }

        fn muted(&mut self, node: Node) -> Result<Option<bool>> {
            self.reads += 1;
            Ok(self.get(node))
        }

        fn set_muted(&mut self, node: Node, muted: bool) -> Result<()> {
            if let Some(path) = &self.state {
                self.seen.push(fs::read_to_string(path).unwrap());
            }
            if self.broken {
                return Err("wpctl set-mute: failed".to_owned());
            }
            let entry = self.nodes.iter_mut().find(|(found, _)| *found == node);
            entry.ok_or("no such node")?.1 = muted;
            Ok(())
        }
    }

    /// Runs `events` (ms since boot) from `phase`, applying each effect to
    /// `mic`; returns the effects and where the cycle ends.
    fn drive(mut phase: Phase, mic: &mut FakeMic, events: &[(Event, u64)]) -> (Vec<Effect>, Phase) {
        let mut effects = Vec::new();
        for &(event, now) in events {
            let (effect, next) = phase.step(event, now, mic).unwrap();
            if let Effect::Mute(node, muted) = effect {
                mic.set_muted(node, muted).unwrap();
            }
            effects.push(effect);
            phase = next;
        }
        (effects, phase)
    }

    fn mute(muted: bool) -> Effect {
        Effect::Mute(MIC, muted)
    }

    fn latched(was_muted: bool) -> Phase {
        Phase::Latched {
            node: MIC,
            was_muted,
        }
    }

    #[test]
    fn holding_talks_through_a_muted_mic_and_mutes_a_live_one() {
        let hold = [(Press, 1_000), (Release, 3_000)];
        let mut mic = FakeMic::new(MUTED);
        assert_eq!(
            drive(Phase::Idle, &mut mic, &hold),
            (vec![mute(LIVE), mute(MUTED)], Phase::Idle)
        );
        assert_eq!(mic.get(MIC), Some(MUTED));
        let mut mic = FakeMic::new(LIVE);
        assert_eq!(
            drive(Phase::Idle, &mut mic, &hold),
            (vec![mute(MUTED), mute(LIVE)], Phase::Idle)
        );
        assert_eq!(mic.get(MIC), Some(LIVE));
    }

    #[test]
    fn a_tap_latches_until_the_next_press_whose_release_is_ignored() {
        let mut mic = FakeMic::new(MUTED);
        let (effects, phase) = drive(Phase::Idle, &mut mic, &[(Press, 1_000), (Release, 1_100)]);
        assert_eq!(
            (effects, phase),
            (vec![mute(LIVE), Effect::Nothing], latched(MUTED))
        );
        assert_eq!(mic.get(MIC), Some(LIVE));
        let (effects, phase) = drive(phase, &mut mic, &[(Press, 9_000)]);
        assert_eq!((effects, phase), (vec![mute(MUTED)], Phase::Ending));
        // a long hold of the ending press does not invert again
        let (effects, phase) = drive(phase, &mut mic, &[(Release, 20_000)]);
        assert_eq!((effects, phase), (vec![Effect::Nothing], Phase::Idle));
        assert_eq!(mic.get(MIC), Some(MUTED));
    }

    #[test]
    fn the_latch_threshold_is_250_ms() {
        let after = |ms: u64| {
            let mut mic = FakeMic::new(MUTED);
            drive(
                Phase::Idle,
                &mut mic,
                &[(Press, 5_000), (Release, 5_000 + ms)],
            )
            .1
        };
        assert_eq!(after(249), latched(MUTED));
        assert_eq!(after(250), Phase::Idle);
        // a release stamped before its press (it ran first) is a tap
        let mut mic = FakeMic::new(MUTED);
        let (_, phase) = drive(Phase::Idle, &mut mic, &[(Press, 5_000), (Release, 4_990)]);
        assert_eq!(phase, latched(MUTED));
    }

    #[test]
    fn a_release_that_won_the_lock_before_its_press_leaves_a_latch() {
        let mut mic = FakeMic::new(LIVE);
        let (effects, phase) = drive(Phase::Idle, &mut mic, &[(Release, 1_050), (Press, 1_000)]);
        assert_eq!(effects, [Effect::Nothing, mute(MUTED)]);
        // the next press ends it, and its release is ignored
        assert_eq!(
            drive(phase, &mut mic, &[(Press, 4_000), (Release, 4_500)]),
            (vec![mute(LIVE), Effect::Nothing], Phase::Idle)
        );
        assert_eq!(mic.get(MIC), Some(LIVE));
    }

    #[test]
    fn a_lost_release_does_not_wedge_the_next_cycle() {
        // the ending press's release never came: the next press engages
        let mut mic = FakeMic::new(LIVE);
        assert_eq!(
            drive(Phase::Ending, &mut mic, &[(Press, 1_000), (Release, 2_000)]),
            (vec![mute(MUTED), mute(LIVE)], Phase::Idle)
        );
        // a held press's release never came: the next press restores
        let held = Phase::Held {
            since: 1_000,
            node: MIC,
            was_muted: MUTED,
        };
        let mut mic = FakeMic::new(LIVE);
        assert_eq!(
            drive(held, &mut mic, &[(Press, 60_000), (Release, 61_000)]),
            (vec![mute(MUTED), Effect::Nothing], Phase::Idle)
        );
        assert_eq!(mic.get(MIC), Some(MUTED));
    }

    #[test]
    fn a_latch_undone_outside_talk_lets_the_next_press_start_a_hold() {
        for start in [MUTED, LIVE] {
            let mut mic = FakeMic::new(start);
            let (_, phase) = drive(Phase::Idle, &mut mic, &[(Press, 1_000), (Release, 1_100)]);
            assert_eq!(phase, latched(start));
            mic.toggle(MIC); // the mic mute key, back to where it started
            // a 3 s hold inverts it again, and its release restores
            let (effects, phase) = drive(phase, &mut mic, &[(Press, 9_000)]);
            let held = Phase::Held {
                since: 9_000,
                node: MIC,
                was_muted: start,
            };
            assert_eq!((effects, phase), (vec![mute(!start)], held), "from {start}");
            let (effects, phase) = drive(phase, &mut mic, &[(Release, 12_000)]);
            assert_eq!(
                (effects, phase),
                (vec![mute(start)], Phase::Idle),
                "from {start}"
            );
            assert_eq!(mic.get(MIC), Some(start));
        }
    }

    #[test]
    fn a_hold_undone_outside_talk_is_not_restored_over() {
        for start in [MUTED, LIVE] {
            let mut mic = FakeMic::new(start);
            let (_, phase) = drive(Phase::Idle, &mut mic, &[(Press, 1_000)]);
            mic.toggle(MIC);
            let (effects, phase) = drive(phase, &mut mic, &[(Release, 3_000)]);
            assert_eq!(
                (effects, phase),
                (vec![Effect::Nothing], Phase::Idle),
                "from {start}"
            );
            assert_eq!(mic.get(MIC), Some(start));
        }
    }

    #[test]
    fn the_node_inverted_is_restored_even_after_the_default_changes() {
        let mut mic = FakeMic::new(MUTED);
        mic.nodes.push((HEADSET, MUTED));
        let (_, phase) = drive(Phase::Idle, &mut mic, &[(Press, 1_000)]);
        mic.default = HEADSET; // plugged in mid-hold
        let (effects, _) = drive(phase, &mut mic, &[(Release, 3_000)]);
        assert_eq!(effects, [mute(MUTED)]);
        assert_eq!(mic.nodes, [(MIC, MUTED), (HEADSET, MUTED)]);
    }

    #[test]
    fn a_node_gone_mid_cycle_is_skipped_and_the_cycle_ends() {
        let mut mic = FakeMic::new(MUTED);
        mic.nodes.push((HEADSET, LIVE));
        mic.default = HEADSET;
        // gone mid-hold: the release restores nothing
        let (_, phase) = drive(Phase::Idle, &mut mic, &[(Press, 1_000)]);
        mic.nodes.retain(|&(node, _)| node != HEADSET);
        mic.default = MIC;
        assert_eq!(
            drive(phase, &mut mic, &[(Release, 3_000)]),
            (vec![Effect::Gone(HEADSET)], Phase::Idle)
        );
        // gone mid-latch, its id reused: the press ending it restores nothing
        let (_, phase) = drive(Phase::Idle, &mut mic, &[(Press, 5_000), (Release, 5_100)]);
        let reused = Node {
            id: MIC.id,
            serial: 3_003,
        };
        mic.nodes = vec![(reused, LIVE)];
        mic.default = reused;
        assert_eq!(
            drive(phase, &mut mic, &[(Press, 8_000)]),
            (vec![Effect::Gone(MIC)], Phase::Idle)
        );
        assert_eq!(mic.nodes, [(reused, LIVE)]);
        // and the press after that engages the new default
        assert_eq!(
            drive(Phase::Idle, &mut mic, &[(Press, 9_000)]).0,
            [Effect::Mute(reused, MUTED)]
        );
    }

    #[test]
    fn the_mic_is_read_only_when_a_press_engages_or_a_release_restores() {
        let mut mic = FakeMic::new(LIVE);
        let reads = |phase: Phase, event, now| {
            let mut mic = FakeMic::new(LIVE);
            phase.step(event, now, &mut mic).unwrap();
            mic.reads
        };
        let held = Phase::Held {
            since: 1_000,
            node: MIC,
            was_muted: MUTED,
        };
        assert_eq!(reads(Phase::Idle, Release, 0), 0);
        assert_eq!(reads(latched(LIVE), Release, 0), 0);
        assert_eq!(reads(Phase::Ending, Release, 0), 0);
        assert_eq!(reads(held, Release, 1_100), 0, "a tap");
        assert_eq!(reads(held, Release, 1_300), 1);
        assert_eq!(reads(Phase::Idle, Press, 0), 1);
        assert_eq!(reads(latched(LIVE), Press, 0), 1);
        drive(Phase::Idle, &mut mic, &[(Press, 0)]);
        assert_eq!(mic.reads, 1);
    }

    #[test]
    fn an_unreadable_mic_engages_nothing() {
        let mut mic = FakeMic::new(LIVE);
        mic.broken = true;
        assert_eq!(
            Phase::Idle.step(Press, 0, &mut mic),
            Err("wpctl: gone".to_owned())
        );
    }

    #[test]
    fn the_state_line_round_trips_and_garbage_is_rejected() {
        for phase in [
            Phase::Idle,
            Phase::Held {
                since: 123_456_789,
                node: MIC,
                was_muted: MUTED,
            },
            Phase::Held {
                since: 0,
                node: HEADSET,
                was_muted: LIVE,
            },
            latched(LIVE),
            Phase::Ending,
        ] {
            assert_eq!(format!("{phase}\n").parse(), Ok(phase));
        }
        for garbage in [
            "",
            "held",
            "held 12 57 1001",
            "held 12 57 1001 2",
            "held -1 57 1001 0",
            "held 12 1 0", // before nodes were pinned
            "latched 0",
            "latched x 1001 0",
            "idle 1",
            "busy",
        ] {
            assert!(garbage.parse::<Phase>().is_err(), "{garbage:?}");
        }
    }

    #[test]
    fn wpctl_get_volume_parses_mute() {
        assert_eq!(parse_muted("Volume: 1.00\n"), Some(false));
        assert_eq!(parse_muted("Volume: 1.00 [MUTED]\n"), Some(true));
        assert_eq!(parse_muted("Volume: 0.40 [MUTED]"), Some(true));
        for unexpected in [
            "",
            "Volume:",
            "Volume: loud",
            "Volume: 1.00 [MUTED] extra",
            "Volume: 1.00 [muted]",
        ] {
            assert_eq!(parse_muted(unexpected), None, "{unexpected:?}");
        }
    }

    #[test]
    fn wpctl_inspect_parses_id_and_serial() {
        // wpctl.c inspect_print_object, as for a laptop's mic
        let inspect = "id 57, type PipeWire:Interface:Node\n\
            \x20   alsa.card = \"0\"\n\
            \x20 * media.class = \"Audio/Source\"\n\
            \x20 * node.name = \"alsa_input.pci-0000_00_1f.3-platform-sof_sdw.HiFi__Mic__source\"\n\
            \x20 * object.path = \"alsa:acp:sofsoundwire:6:capture\"\n\
            \x20 * object.serial = \"1001\"\n\
            \x20   priority.session = \"2000\"\n";
        assert_eq!(parse_inspect(inspect), Some(MIC));
        for unexpected in [
            "",
            "id 57, type PipeWire:Interface:Node\n",
            "id x, type PipeWire:Interface:Node\n  * object.serial = \"1001\"\n",
            "57, type PipeWire:Interface:Node\n  * object.serial = \"1001\"\n",
            "id 57, type PipeWire:Interface:Node\n  * object.serial = 1001\n",
        ] {
            assert_eq!(parse_inspect(unexpected), None, "{unexpected:?}");
        }
    }

    #[test]
    fn only_press_or_release_is_an_event() {
        let args = |args: &[&str]| parse_event(args.iter().map(|arg| arg.to_string()));
        assert_eq!(args(&["press"]), Ok(Press));
        assert_eq!(args(&["release"]), Ok(Release));
        for wrong in [&[][..], &["toggle"], &["press", "release"]] {
            assert!(args(wrong).is_err(), "{wrong:?}");
        }
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = env::temp_dir().join(format!("talk-test-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_phase_is_saved_before_the_mute_and_rolled_back_if_it_fails() {
        let dir = scratch("order");
        let mut mic = FakeMic::new(MUTED);
        mic.state = Some(dir.join("state"));
        let mut notes = Vec::new();
        assert_eq!(run(&dir, Press, 1_000, &mut mic, &mut notes), Ok(true));
        let held = "held 1000 57 1001 1\n";
        assert_eq!(mic.seen, [held]);
        mic.broken = true;
        let failed = run(&dir, Release, 5_000, &mut mic, &mut notes);
        assert_eq!(failed, Err("wpctl set-mute: failed".to_owned()));
        assert_eq!(mic.seen[1], "idle\n", "saved first");
        assert_eq!(
            fs::read_to_string(dir.join("state")).unwrap(),
            held,
            "rolled back"
        );
        assert!(notes.is_empty());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn unreadable_state_is_reported_and_starts_over() {
        for garbage in [&b"\xff\xfe\n"[..], b"held 12 1 0\n"] {
            let dir = scratch("garbage");
            fs::write(dir.join("state"), garbage).unwrap();
            let mut notes = Vec::new();
            let mut mic = FakeMic::new(MUTED);
            assert_eq!(run(&dir, Release, 0, &mut mic, &mut notes), Ok(false));
            assert_eq!(notes.len(), 1, "{notes:?}");
            assert!(notes[0].ends_with("; starting over"), "{notes:?}");
            assert_eq!(fs::read_to_string(dir.join("state")).unwrap(), "idle\n");
            fs::remove_dir_all(&dir).unwrap();
        }
    }

    #[test]
    fn a_gone_node_ends_the_cycle_with_an_error() {
        let dir = scratch("gone");
        let mut mic = FakeMic::new(MUTED);
        let mut notes = Vec::new();
        run(&dir, Press, 0, &mut mic, &mut notes).unwrap();
        mic.nodes.clear();
        let gone = run(&dir, Release, 1_000, &mut mic, &mut notes).unwrap_err();
        assert!(gone.contains("node 57) is gone"), "{gone}");
        assert_eq!(fs::read_to_string(dir.join("state")).unwrap(), "idle\n");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn errors_notify_once_until_the_mute_is_set_again() {
        let dir = scratch("reported");
        let error = || vec!["wpctl: gone".to_owned()];
        assert_eq!(unreported(&dir, false, error()), error());
        assert!(unreported(&dir, false, error()).is_empty(), "repeated");
        // a release that set nothing forgets nothing
        assert!(unreported(&dir, false, Vec::new()).is_empty());
        assert!(unreported(&dir, false, error()).is_empty());
        // one that set the mute does
        assert!(unreported(&dir, true, Vec::new()).is_empty());
        assert_eq!(unreported(&dir, false, error()), error());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_lock_gives_up_at_its_deadline() {
        let dir = scratch("lock");
        let held = lock(&dir, DEADLINE).unwrap();
        let started = Instant::now();
        let error = lock(&dir, Duration::from_millis(50)).unwrap_err();
        assert!(error.ends_with("still locked after 50ms"), "{error}");
        assert!(started.elapsed() < Duration::from_secs(1));
        drop(held);
        lock(&dir, Duration::from_millis(50)).unwrap();
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_command_past_its_deadline_is_killed() {
        let mut sleep = Command::new("sleep");
        sleep.arg("10");
        let started = Instant::now();
        let error = run_until(sleep, Duration::from_millis(50)).err().unwrap();
        assert_eq!(error, "sleep 10: no answer in 50ms");
        assert!(started.elapsed() < Duration::from_secs(1));
        let mut echo = Command::new("echo");
        echo.arg("Volume: 1.00");
        let reply = run_until(echo, DEADLINE).unwrap();
        assert!(reply.status.success());
        assert_eq!(reply.stdout, "Volume: 1.00\n");
    }
}
