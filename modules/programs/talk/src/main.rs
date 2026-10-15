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
//! directory: a press and its release can race for it, in either order.
//! Hyprland drops exec output, so errors also go to the notification daemon
//! (mako) through notify-send, once per distinct message.

use std::env;
use std::fmt;
use std::fs::{self, File};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::str::FromStr;

use nix::time::{ClockId, clock_gettime};

/// A release sooner than this after its press latches.
const LATCH_MS: u64 = 250;

const SOURCE: &str = "@DEFAULT_AUDIO_SOURCE@";

type Result<T, E = String> = std::result::Result<T, E>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Event {
    Press,
    Release,
}

/// Where the key's cycle stands; `was_muted` is the state to restore.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Idle,
    /// Inverted by a press at `since` (ms of CLOCK_BOOTTIME).
    Held {
        since: u64,
        was_muted: bool,
    },
    /// Inverted by a tap, until the next press.
    Latched {
        was_muted: bool,
    },
    /// Restored by a press, whose release is not a tap.
    Ending,
}

impl Phase {
    /// The mute to apply for `event` at `now`, if any, and the phase after
    /// it. `muted` reads the source's mute, only when a press engages.
    ///
    /// A press always settles a cycle still going, so lost or reordered
    /// events cannot wedge talk. A release handled before its own press (the
    /// two processes raced for the lock) leaves a Held that nothing releases:
    /// the next press ends it, as it would a latch, which is right since that
    /// release came within a tap. A release lost after an ending press leaves
    /// Ending, which the next press treats as idle.
    fn step(
        self,
        event: Event,
        now: u64,
        muted: impl FnOnce() -> Result<bool>,
    ) -> Result<(Option<bool>, Phase)> {
        Ok(match (self, event) {
            (Phase::Idle | Phase::Ending, Event::Press) => {
                let was_muted = muted()?;
                (
                    Some(!was_muted),
                    Phase::Held {
                        since: now,
                        was_muted,
                    },
                )
            }
            (Phase::Held { was_muted, .. } | Phase::Latched { was_muted }, Event::Press) => {
                (Some(was_muted), Phase::Ending)
            }
            (Phase::Held { since, was_muted }, Event::Release) => {
                if now.saturating_sub(since) < LATCH_MS {
                    (None, Phase::Latched { was_muted })
                } else {
                    (Some(was_muted), Phase::Idle)
                }
            }
            (Phase::Ending, Event::Release) => (None, Phase::Idle),
            (phase @ (Phase::Idle | Phase::Latched { .. }), Event::Release) => (None, phase),
        })
    }
}

/// The state file's one line: `idle`, `held <since> <0|1>`, `latched <0|1>`
/// or `ending`.
impl fmt::Display for Phase {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            Phase::Idle => write!(f, "idle"),
            Phase::Held { since, was_muted } => write!(f, "held {since} {}", u8::from(was_muted)),
            Phase::Latched { was_muted } => write!(f, "latched {}", u8::from(was_muted)),
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
        let words: Vec<&str> = line.split_whitespace().collect();
        match words[..] {
            ["idle"] => Some(Phase::Idle),
            ["held", since, was_muted] => since
                .parse()
                .ok()
                .zip(flag(was_muted))
                .map(|(since, was_muted)| Phase::Held { since, was_muted }),
            ["latched", was_muted] => flag(was_muted).map(|was_muted| Phase::Latched { was_muted }),
            ["ending"] => Some(Phase::Ending),
            _ => None,
        }
        .ok_or_else(|| format!("unreadable state {line:?}"))
    }
}

/// Whether `wpctl get-volume` reports the node muted: `Volume: 1.00`, or
/// `Volume: 1.00 [MUTED]`.
fn parse_muted(output: &str) -> Result<bool> {
    let unexpected = || format!("unexpected wpctl get-volume output {output:?}");
    let mut words = output
        .trim()
        .strip_prefix("Volume:")
        .ok_or_else(unexpected)?
        .split_whitespace();
    words
        .next()
        .and_then(|volume| volume.parse::<f64>().ok())
        .ok_or_else(unexpected)?;
    match (words.next(), words.next()) {
        (None, _) => Ok(false),
        (Some("[MUTED]"), None) => Ok(true),
        _ => Err(unexpected()),
    }
}

fn wpctl(args: &[&str]) -> Result<String> {
    let command = || format!("wpctl {}", args.join(" "));
    let output = Command::new("wpctl")
        .args(args)
        .output()
        .map_err(|error| format!("{}: {error}", command()))?;
    if !output.status.success() {
        return Err(format!(
            "{}: {}: {}",
            command(),
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    String::from_utf8(output.stdout).map_err(|error| format!("{}: {error}", command()))
}

fn source_muted() -> Result<bool> {
    parse_muted(&wpctl(&["get-volume", SOURCE])?)
}

fn set_source_muted(muted: bool) -> Result<()> {
    wpctl(&["set-mute", SOURCE, if muted { "1" } else { "0" }]).map(drop)
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
/// process, so a crashed invocation cannot leave it held.
fn lock(dir: &Path) -> Result<File> {
    fs::create_dir_all(dir).map_err(|error| format!("{}: {error}", dir.display()))?;
    let path = dir.join("lock");
    let file = File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .map_err(|error| format!("{}: {error}", path.display()))?;
    file.lock()
        .map_err(|error| format!("flock {}: {error}", path.display()))?;
    Ok(file)
}

fn run(dir: &Path, event: Event, now: u64) -> Result<()> {
    let path = dir.join("state");
    let stored = match fs::read_to_string(&path) {
        Ok(line) => Some(line),
        Err(error) if error.kind() == ErrorKind::NotFound => None,
        Err(error) => return Err(format!("{}: {error}", path.display())),
    };
    let phase = stored.as_deref().map_or(Phase::Idle, |line| {
        line.parse().unwrap_or_else(|error| {
            report(dir, &format!("{}: {error}; starting over", path.display()));
            Phase::Idle
        })
    });
    // the phase only moves once its mute is applied, so a failure leaves the
    // cycle where it was, for the next press to retry
    let (mute, next) = phase.step(event, now, source_muted)?;
    if let Some(muted) = mute {
        set_source_muted(muted)?;
    }
    let line = format!("{next}\n");
    if stored.as_deref() != Some(line.as_str()) {
        fs::write(&path, line).map_err(|error| format!("{}: {error}", path.display()))?;
    }
    Ok(())
}

fn notify(message: &str) {
    eprintln!("talk: {message}");
    let sent = Command::new("notify-send")
        .args(["--app-name=talk", "--urgency=critical", "talk", message])
        .status();
    match sent {
        Ok(status) if status.success() => {}
        Ok(status) => eprintln!("talk: notify-send: {status}"),
        Err(error) => eprintln!("talk: notify-send: {error}"),
    }
}

/// Notifies `message` unless it is the one notified last, which a success
/// forgets: a key used on a broken setup would otherwise notify at every
/// press and release.
fn report(dir: &Path, message: &str) {
    let path = dir.join("reported");
    if fs::read_to_string(&path).is_ok_and(|last| last == message) {
        eprintln!("talk: {message}");
        return;
    }
    if let Err(error) = fs::write(&path, message) {
        eprintln!("talk: {}: {error}", path.display());
    }
    notify(message);
}

fn main() -> ExitCode {
    // the key's time is when Hyprland ran us, not when the lock came free
    let now = now_ms();
    let Some(runtime) = env::var_os("XDG_RUNTIME_DIR") else {
        notify("XDG_RUNTIME_DIR is not set");
        return ExitCode::FAILURE;
    };
    let dir = PathBuf::from(runtime).join("talk");
    let _lock = match lock(&dir) {
        Ok(lock) => lock,
        Err(error) => {
            notify(&error);
            return ExitCode::FAILURE;
        }
    };
    let result = parse_event(env::args().skip(1)).and_then(|event| run(&dir, event, now?));
    match result {
        Ok(()) => {
            match fs::remove_file(dir.join("reported")) {
                Err(error) if error.kind() != ErrorKind::NotFound => {
                    eprintln!("talk: {}: {error}", dir.join("reported").display());
                }
                _ => {}
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            report(&dir, &error);
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIVE: bool = false;
    const MUTED: bool = true;

    /// Runs `events` (ms since boot) from `phase` on a source muted
    /// `muted`, applying each mute as wpctl would; returns the mutes applied
    /// and where the cycle ends.
    fn drive(
        mut phase: Phase,
        mut muted: bool,
        events: &[(Event, u64)],
    ) -> (Vec<bool>, Phase, bool) {
        let mut applied = Vec::new();
        for &(event, now) in events {
            let (mute, next) = phase.step(event, now, || Ok(muted)).unwrap();
            if let Some(mute) = mute {
                applied.push(mute);
                muted = mute;
            }
            phase = next;
        }
        (applied, phase, muted)
    }

    use Event::{Press, Release};

    #[test]
    fn holding_talks_through_a_muted_mic_and_mutes_a_live_one() {
        let hold = [(Press, 1_000), (Release, 3_000)];
        assert_eq!(
            drive(Phase::Idle, MUTED, &hold),
            (vec![LIVE, MUTED], Phase::Idle, MUTED)
        );
        assert_eq!(
            drive(Phase::Idle, LIVE, &hold),
            (vec![MUTED, LIVE], Phase::Idle, LIVE)
        );
    }

    #[test]
    fn a_tap_latches_until_the_next_press_whose_release_is_ignored() {
        let (applied, phase, muted) =
            drive(Phase::Idle, MUTED, &[(Press, 1_000), (Release, 1_100)]);
        assert_eq!(
            (applied, phase, muted),
            (vec![LIVE], Phase::Latched { was_muted: MUTED }, LIVE)
        );
        let (applied, phase, muted) = drive(phase, muted, &[(Press, 9_000)]);
        assert_eq!((applied, phase, muted), (vec![MUTED], Phase::Ending, MUTED));
        // a long hold of the ending press does not invert again
        let (applied, phase, muted) = drive(phase, muted, &[(Release, 20_000)]);
        assert_eq!((applied, phase, muted), (vec![], Phase::Idle, MUTED));
    }

    #[test]
    fn the_latch_threshold_is_250_ms() {
        let held = |ms: u64| drive(Phase::Idle, MUTED, &[(Press, 5_000), (Release, 5_000 + ms)]).1;
        assert_eq!(held(249), Phase::Latched { was_muted: MUTED });
        assert_eq!(held(250), Phase::Idle);
        // a release stamped before its press (it ran first) is a tap
        assert_eq!(
            drive(Phase::Idle, MUTED, &[(Press, 5_000), (Release, 4_990)]).1,
            Phase::Latched { was_muted: MUTED }
        );
    }

    #[test]
    fn a_release_that_won_the_lock_before_its_press_leaves_a_latch() {
        let (applied, phase, muted) = drive(Phase::Idle, LIVE, &[(Release, 1_050), (Press, 1_000)]);
        assert_eq!(applied, [MUTED]);
        // the next press ends it, and its release is ignored
        assert_eq!(
            drive(phase, muted, &[(Press, 4_000), (Release, 4_500)]),
            (vec![LIVE], Phase::Idle, LIVE)
        );
    }

    #[test]
    fn a_lost_release_does_not_wedge_the_next_cycle() {
        // the ending press's release never came: the next press engages
        assert_eq!(
            drive(Phase::Ending, LIVE, &[(Press, 1_000), (Release, 2_000)]),
            (vec![MUTED, LIVE], Phase::Idle, LIVE)
        );
        // a held press's release never came: the next press restores
        let held = Phase::Held {
            since: 1_000,
            was_muted: MUTED,
        };
        assert_eq!(
            drive(held, LIVE, &[(Press, 60_000), (Release, 61_000)]),
            (vec![MUTED], Phase::Idle, MUTED)
        );
    }

    #[test]
    fn mute_is_read_only_when_a_press_engages() {
        let unread = || -> Result<bool> { unreachable!("mute read") };
        let latched = Phase::Latched { was_muted: LIVE };
        assert_eq!(
            Phase::Idle.step(Release, 0, unread),
            Ok((None, Phase::Idle))
        );
        assert_eq!(latched.step(Release, 0, unread), Ok((None, latched)));
        assert_eq!(
            latched.step(Press, 0, unread),
            Ok((Some(LIVE), Phase::Ending))
        );
        assert_eq!(
            Phase::Ending.step(Release, 0, unread),
            Ok((None, Phase::Idle))
        );
    }

    #[test]
    fn an_unreadable_mute_engages_nothing() {
        let failed = Phase::Idle.step(Press, 0, || Err("wpctl: gone".to_owned()));
        assert_eq!(failed, Err("wpctl: gone".to_owned()));
    }

    #[test]
    fn the_state_line_round_trips_and_garbage_is_rejected() {
        for phase in [
            Phase::Idle,
            Phase::Held {
                since: 123_456_789,
                was_muted: MUTED,
            },
            Phase::Held {
                since: 0,
                was_muted: LIVE,
            },
            Phase::Latched { was_muted: LIVE },
            Phase::Ending,
        ] {
            assert_eq!(format!("{phase}\n").parse(), Ok(phase));
        }
        for garbage in [
            "",
            "held",
            "held 12 2",
            "held -1 0",
            "latched",
            "idle 1",
            "busy",
        ] {
            assert!(garbage.parse::<Phase>().is_err(), "{garbage:?}");
        }
    }

    #[test]
    fn wpctl_get_volume_parses_mute() {
        assert_eq!(parse_muted("Volume: 1.00\n"), Ok(false));
        assert_eq!(parse_muted("Volume: 1.00 [MUTED]\n"), Ok(true));
        assert_eq!(parse_muted("Volume: 0.40 [MUTED]"), Ok(true));
        for unexpected in [
            "",
            "Volume:",
            "Volume: loud",
            "Volume: 1.00 [MUTED] extra",
            "Volume: 1.00 [muted]",
            "Translate ID error: '@DEFAULT_AUDIO_SOURCE@' is not a valid ID",
        ] {
            assert!(parse_muted(unexpected).is_err(), "{unexpected:?}");
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
}
