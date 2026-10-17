mod support;

use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Child, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use serde_json::Value;
use support::Isolated;

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
            let matched = self.latest["class"] == expected;
            // Observe the latest row through a quiet window, not any transient
            // historical match in the middle of default-node replacement.
            let timeout = if matched {
                remaining.min(Duration::from_millis(150))
            } else {
                remaining
            };
            match self.next(timeout) {
                Ok(()) => {}
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

fn remove_source(server: &Isolated, name: &str) {
    let objects: Value =
        serde_json::from_slice(&server.run(&server.bin.join("pw-dump"), &[]).stdout).unwrap();
    let object = objects
        .as_array()
        .unwrap()
        .iter()
        .find(|object| object["info"]["props"]["node.name"] == name)
        .expect("fixture input does not exist");
    server.run(
        &server.bin.join("pw-cli"),
        &["destroy", &object["id"].to_string()],
    );
}

#[test]
#[ignore = "spawns isolated PipeWire/WirePlumber; Nix checkPhase runs this"]
fn microphone_feed_invalidates_and_recovers_in_the_same_process() {
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
