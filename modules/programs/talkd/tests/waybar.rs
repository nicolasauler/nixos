mod support;

use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Child, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Mutex, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use serde_json::Value;
use support::Isolated;

/// `Isolated` uses one directory per process, and the recovery test restarts
/// its server on the same socket path: run these tests one at a time.
static ISOLATED: Mutex<()> = Mutex::new(());

struct Observer {
    child: Child,
    lines: Receiver<Result<Value, String>>,
    reader: Option<JoinHandle<()>>,
    latest: Value,
}

impl Observer {
    fn start(server: &Isolated) -> Self {
        let mut child = server
            .command(Path::new(env!("CARGO_BIN_EXE_talkd")))
            .arg("waybar")
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let (send, lines) = mpsc::channel();
        let reader = thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let value = line.map_err(|error| error.to_string()).and_then(|line| {
                    serde_json::from_str(&line).map_err(|error| error.to_string())
                });
                if send.send(value).is_err() {
                    break;
                }
            }
        });
        Self {
            child,
            lines,
            reader: Some(reader),
            latest: Value::Null,
        }
    }

    fn next(&mut self, timeout: Duration) -> Result<(), RecvTimeoutError> {
        self.latest = self
            .lines
            .recv_timeout(timeout)?
            .expect("invalid observer JSON");
        // Records carry the state alone; the bar's format adds the MIC label.
        let text = match self.latest["class"].as_str() {
            Some("muted") => "MUTED",
            Some("live") => "LIVE",
            Some("unknown") => "?",
            other => panic!("unexpected class {other:?}: {}", self.latest),
        };
        assert_eq!(self.latest["text"], text, "{}", self.latest);
        Ok(())
    }

    fn known(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !matches!(self.latest["class"].as_str(), Some("muted" | "live")) {
            self.next(
                deadline
                    .checked_duration_since(Instant::now())
                    .expect("no default input became readable"),
            )
            .expect("observer stopped before a known input");
        }
    }

    fn state(&mut self, expected: &str) {
        self.settle(expected, |latest| latest["class"] == expected, |_| false);
    }

    /// The state of one particular default input, identified by the device
    /// description in the tooltip.
    fn input(&mut self, class: &str, device: &str) {
        self.settle(
            &format!("{class} from {device}"),
            |latest| latest["class"] == class && names(latest, device),
            |_| false,
        );
    }

    /// `input`, failing on any record that names `other` on the way there.
    fn input_without(&mut self, class: &str, device: &str, other: &str) {
        self.settle(
            &format!("{class} from {device}, never {other}"),
            |latest| latest["class"] == class && names(latest, device),
            |latest| names(latest, other),
        );
    }

    fn settle(
        &mut self,
        expected: &str,
        matches: impl Fn(&Value) -> bool,
        forbidden: impl Fn(&Value) -> bool,
    ) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .unwrap_or_else(|| {
                    panic!(
                        "expected {expected}, latest observer state: {}",
                        self.latest
                    )
                });
            let matched = matches(&self.latest);
            // Observe the latest row through a quiet window, not any transient
            // historical match in the middle of default-node replacement.
            let timeout = if matched {
                remaining.min(Duration::from_millis(150))
            } else {
                remaining
            };
            match self.next(timeout) {
                Ok(()) => assert!(
                    !forbidden(&self.latest),
                    "expected {expected}, got {}",
                    self.latest
                ),
                Err(RecvTimeoutError::Timeout) if matched => {
                    assert!(self.child.try_wait().unwrap().is_none(), "observer exited");
                    return;
                }
                Err(error) => panic!("expected {expected}, latest {}: {error}", self.latest),
            }
        }
    }
}

impl Drop for Observer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(reader) = self.reader.take() {
            reader.join().unwrap();
        }
    }
}

fn names(record: &Value, device: &str) -> bool {
    record["tooltip"]
        .as_str()
        .is_some_and(|tooltip| tooltip.contains(device))
}

fn node_id(server: &Isolated, name: &str) -> String {
    let objects: Value =
        serde_json::from_slice(&server.run(&server.bin.join("pw-dump"), &[]).stdout).unwrap();
    objects
        .as_array()
        .unwrap()
        .iter()
        .find(|object| object["info"]["props"]["node.name"] == name)
        .expect("fixture input does not exist")["id"]
        .to_string()
}

fn remove_source(server: &Isolated, name: &str) {
    server.run(
        &server.bin.join("pw-cli"),
        &["destroy", &node_id(server, name)],
    );
}

#[test]
#[ignore = "spawns isolated PipeWire/WirePlumber; Nix checkPhase runs this"]
fn microphone_feed_invalidates_and_recovers_in_the_same_process() {
    let _isolated = ISOLATED.lock().unwrap_or_else(PoisonError::into_inner);
    let mut server = Isolated::start();
    // One input makes the default deterministic across policy startup/restart.
    remove_source(&server, "test-headset");
    let mut observer = Observer::start(&server);
    observer.next(Duration::from_secs(5)).unwrap();
    assert_eq!(
        observer.latest["class"], "unknown",
        "startup must not assume a mute state"
    );
    observer.known();
    server.wpctl(&["set-mute", "@DEFAULT_AUDIO_SOURCE@", "0"]);
    observer.state("live");
    server.wpctl(&["set-mute", "@DEFAULT_AUDIO_SOURCE@", "1"]);
    observer.state("muted");
    assert!(
        !server.dir.join("run/talkd").exists(),
        "observer created restoration state"
    );

    server.kill_pipewire();
    observer.state("unknown");
    drop(server);

    // The same private socket path, but an entirely new server. Do not replace
    // the observer: its actual CLI process must reconnect by itself.
    let server = Isolated::start();
    remove_source(&server, "test-headset");
    observer.known();
    server.wpctl(&["set-mute", "@DEFAULT_AUDIO_SOURCE@", "1"]);
    observer.state("muted");
    assert!(
        !server.dir.join("run/talkd").exists(),
        "recovery created restoration state"
    );

    remove_source(&server, "test-mic");
    observer.state("unknown");
}

#[test]
#[ignore = "spawns isolated PipeWire/WirePlumber; Nix checkPhase runs this"]
fn microphone_feed_follows_the_default_input() {
    let _isolated = ISOLATED.lock().unwrap_or_else(PoisonError::into_inner);
    let server = Isolated::start();
    let mic = node_id(&server, "test-mic");
    let headset = node_id(&server, "test-headset");
    // Opposite states: still reporting the previous default cannot pass.
    server.wpctl(&["set-mute", &mic, "1"]);
    server.wpctl(&["set-mute", &headset, "0"]);
    server.wpctl(&["set-default", &mic]);
    let mut observer = Observer::start(&server);
    observer.input("muted", "Test mic");

    server.wpctl(&["set-default", &headset]);
    observer.input("live", "Test headset");
    // Later changes come from the new default, not the input seen at startup.
    server.wpctl(&["set-mute", &headset, "1"]);
    observer.input("muted", "Test headset");
    // The old default's change must never surface. The headset change after it
    // marks that the observer has processed both events, in order.
    server.wpctl(&["set-mute", &mic, "0"]);
    server.wpctl(&["set-mute", &headset, "0"]);
    observer.input_without("live", "Test headset", "Test mic");

    server.wpctl(&["set-default", &mic]);
    observer.input("live", "Test mic");
}
