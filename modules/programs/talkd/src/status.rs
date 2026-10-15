//! `talkd status`: what talkd would act on, read-only. It connects to
//! PipeWire, syncs, prints the server and the default source with where its
//! mute lives, and asks the compositor whether the session is locked. It
//! sets no param, writes no state, notifies no one and opens no input
//! device.

use std::cell::RefCell;
use std::process::ExitCode;
use std::rc::Rc;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use pipewire as pw;
use pw::loop_::Timeout;

use crate::lock;
use crate::machine::Lock;
use crate::mic::{Mic, Status, Update};
use crate::params::Via;

/// How long PipeWire and the compositor may take to answer.
const WAIT: Duration = Duration::from_secs(3);

/// The report, one fact per line.
pub fn format(status: &Status, lock: &Result<Lock, String>) -> String {
    let mut out = format!("PipeWire server: cookie {:08x}\n", status.server);
    match &status.default {
        Err(why) => out += &format!("default source: {why}\n"),
        Ok(source) => {
            out += &format!(
                "default source: node {}, serial {}, {}",
                source.id, source.serial, source.name
            );
            if let Some(description) = &source.description {
                out += &format!(" ({description})");
            }
            out += "\n";
            match source.mute {
                None => {
                    out += "mute lives in: nowhere the mixer reads (no Route or Props with mute and channelVolumes)\n"
                }
                Some((via, muted)) => {
                    out += &match via {
                        Via::Route { on, index, device } => format!(
                            "mute lives in: Route index {index}, card profile device {device}, of PipeWire device {on}\n"
                        ),
                        Via::Props => "mute lives in: the node's Props\n".to_owned(),
                    };
                    out += if muted { "muted: yes\n" } else { "muted: no\n" };
                }
            }
        }
    }
    out += &match lock {
        Ok(Lock::Locked) => "session: locked\n".to_owned(),
        Ok(Lock::Unlocked) => "session: unlocked\n".to_owned(),
        Ok(Lock::Unknown) => "session: unknown\n".to_owned(),
        Err(why) => format!("session: unknown ({why})\n"),
    };
    out
}

/// Connects to `remote` (None: the session's server) and waits for the sync.
pub fn read(remote: Option<String>) -> Result<Status, String> {
    pw::init();
    let main_loop = pw::main_loop::MainLoopRc::new(None).map_err(|error| error.to_string())?;
    let context =
        pw::context::ContextRc::new(&main_loop, None).map_err(|error| error.to_string())?;
    let cell: Rc<RefCell<Option<Mic>>> = Rc::new(RefCell::new(None));
    let hook: Rc<dyn Fn(Update)> = {
        let cell = Rc::downgrade(&cell);
        Rc::new(move |update| {
            if let Some(cell) = cell.upgrade()
                && let Some(mic) = cell.borrow_mut().as_mut()
            {
                mic.apply(update);
            }
        })
    };
    let mut mic = Mic::new(context, remote, hook);
    mic.connect()?;
    *cell.borrow_mut() = Some(mic);
    let started = Instant::now();
    loop {
        if let Some(status) = cell.borrow().as_ref().and_then(Mic::status) {
            return Ok(status);
        }
        if started.elapsed() > WAIT {
            return Err(format!("PipeWire did not sync within {}s", WAIT.as_secs()));
        }
        main_loop
            .loop_()
            .iterate(Timeout::Finite(Duration::from_millis(20)));
    }
}

pub fn run() -> ExitCode {
    // the compositor is asked on the side, so neither waits on the other
    let (answer, lock_state) = mpsc::channel();
    thread::spawn(move || {
        let _ = answer.send(lock::now());
    });
    let status = match read(None) {
        Ok(status) => status,
        Err(error) => {
            eprintln!("talkd status: {error}");
            return ExitCode::FAILURE;
        }
    };
    let lock = lock_state
        .recv_timeout(WAIT)
        .unwrap_or_else(|_| Err("no answer from the compositor".to_owned()));
    print!("{}", format(&status, &lock));
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mic::DefaultSource;

    fn source(mute: Option<(Via, bool)>, description: Option<&str>) -> Status {
        Status {
            server: 0x4d91_2e15,
            default: Ok(DefaultSource {
                id: 57,
                serial: 1001,
                name: "alsa_input.pci-0000_00_1f.3".to_owned(),
                description: description.map(str::to_owned),
                mute,
            }),
        }
    }

    #[test]
    fn a_route_mute_names_its_device_and_route() {
        let route = Via::Route {
            on: 40,
            index: 1,
            device: 5,
        };
        assert_eq!(
            format(
                &source(Some((route, true)), Some("Built-in Audio")),
                &Ok(Lock::Unlocked)
            ),
            "PipeWire server: cookie 4d912e15\n\
             default source: node 57, serial 1001, alsa_input.pci-0000_00_1f.3 (Built-in Audio)\n\
             mute lives in: Route index 1, card profile device 5, of PipeWire device 40\n\
             muted: yes\n\
             session: unlocked\n"
        );
    }

    #[test]
    fn a_props_mute_and_an_unknown_lock() {
        let out = format(
            &source(Some((Via::Props, false)), None),
            &Err("Could not find wayland compositor".to_owned()),
        );
        assert!(
            out.contains("serial 1001, alsa_input.pci-0000_00_1f.3\n"),
            "{out}"
        );
        assert!(
            out.contains("mute lives in: the node's Props\nmuted: no\n"),
            "{out}"
        );
        assert!(
            out.ends_with("session: unknown (Could not find wayland compositor)\n"),
            "{out}"
        );
    }

    #[test]
    fn no_mute_or_no_default_says_why() {
        let out = format(&source(None, None), &Ok(Lock::Locked));
        assert!(
            out.contains("mute lives in: nowhere the mixer reads"),
            "{out}"
        );
        assert!(!out.contains("muted:"), "{out}");
        assert!(out.ends_with("session: locked\n"));
        let none = Status {
            server: 1,
            default: Err("none: no default.audio.source in the \"default\" metadata".to_owned()),
        };
        assert_eq!(
            format(&none, &Ok(Lock::Unlocked)),
            "PipeWire server: cookie 00000001\n\
             default source: none: no default.audio.source in the \"default\" metadata\n\
             session: unlocked\n"
        );
    }
}
