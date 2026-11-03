//! What talkd owes, on disk: the obligation, one line, and the dormant
//! restores, a header and one line each. Each file is replaced atomically
//! (temporary file, fsync, rename, fsync of the directory), so a crash
//! leaves either the old record or the new one, never a torn one.
//!
//! An obligation is `talkd-obligation-v2 <server> <serial> <0|1> <name>
//! <description>`, a dormant restore `<0|1> <name> <description>`: node.name
//! and node.description last, as a JSON string (the description null if
//! there is none), so neither can run into the fields or the next line.
//! Records written before names were kept (`talkd-obligation-v1`, no name)
//! still load.

use std::fs::{self, File};
use std::io::{ErrorKind, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use crate::machine::{Dormant, Obligation, Pin};

const V1: &str = "talkd-obligation-v1";
const V2: &str = "talkd-obligation-v2";
const DORMANT: &str = "talkd-dormant-v1";

pub struct Store {
    dir: PathBuf,
}

impl Store {
    pub fn new(dir: PathBuf) -> Result<Self, String> {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&dir)
            .map_err(|error| format!("{}: {error}", dir.display()))?;
        Ok(Store { dir })
    }

    /// The saved obligation, if any. A record that does not parse is an
    /// error; the caller drops it.
    pub fn load(&self) -> Result<Option<Obligation>, String> {
        self.read("obligation", parse)
    }

    pub fn save(&self, obligation: &Obligation) -> Result<(), String> {
        self.replace("obligation", &line(obligation))
    }

    pub fn clear(&self) -> Result<(), String> {
        self.remove("obligation")
    }

    /// The saved dormant restores, oldest first. A file that does not parse
    /// is an error, as for [`Store::load`].
    pub fn load_dormant(&self) -> Result<Vec<Dormant>, String> {
        self.read("dormant", parse_dormant)
            .map(Option::unwrap_or_default)
    }

    /// Replaces the dormant restores; none removes the file.
    pub fn save_dormant(&self, dormant: &[Dormant]) -> Result<(), String> {
        if dormant.is_empty() {
            return self.remove("dormant");
        }
        let mut text = format!("{DORMANT}\n");
        for owed in dormant {
            text += &format!(
                "{} {}\n",
                u8::from(owed.was_muted),
                names(&owed.name, owed.description.as_deref())
            );
        }
        self.replace("dormant", &text)
    }

    fn read<T>(&self, name: &str, parse: fn(&str) -> Option<T>) -> Result<Option<T>, String> {
        let path = self.dir.join(name);
        match fs::read(&path) {
            Ok(bytes) => parse(&String::from_utf8_lossy(&bytes))
                .map(Some)
                .ok_or_else(|| format!("{}: unreadable record", path.display())),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
            Err(error) => Err(format!("{}: {error}", path.display())),
        }
    }

    fn replace(&self, name: &str, text: &str) -> Result<(), String> {
        let path = self.dir.join(name);
        let temporary = self.dir.join(format!("{name}.tmp"));
        let failed = |error: std::io::Error| format!("{}: {error}", temporary.display());
        let mut file = File::options()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&temporary)
            .map_err(failed)?;
        file.write_all(text.as_bytes()).map_err(failed)?;
        file.sync_all().map_err(failed)?;
        fs::rename(&temporary, &path).map_err(|error| format!("{}: {error}", path.display()))?;
        sync_dir(&self.dir)
    }

    fn remove(&self, name: &str) -> Result<(), String> {
        let path = self.dir.join(name);
        match fs::remove_file(&path) {
            Ok(()) => sync_dir(&self.dir),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!("{}: {error}", path.display())),
        }
    }
}

fn sync_dir(dir: &Path) -> Result<(), String> {
    File::open(dir)
        .and_then(|dir| dir.sync_all())
        .map_err(|error| format!("{}: {error}", dir.display()))
}

/// `<name> <description>`, each a JSON string, the description null if none.
fn names(name: &str, description: Option<&str>) -> String {
    serde_json::to_string(name).expect("a string serializes")
        + " "
        + &serde_json::to_string(&description).expect("a string serializes")
}

fn flag(word: &str) -> Option<bool> {
    match word {
        "0" => Some(false),
        "1" => Some(true),
        _ => None,
    }
}

/// What [`names`] wrote, and nothing after it.
fn parse_names(text: &str) -> Option<(String, Option<String>)> {
    use serde_json::Value;
    let mut values = serde_json::Deserializer::from_str(text).into_iter::<Value>();
    let Value::String(name) = values.next()?.ok()? else {
        return None;
    };
    let description = match values.next()?.ok()? {
        Value::String(description) => Some(description),
        Value::Null => None,
        _ => return None,
    };
    values.next().is_none().then_some((name, description))
}

/// `<0|1> <name> <description>`, the rest of the line.
fn muted_and_names(text: &str) -> Option<(bool, String, Option<String>)> {
    let (muted, names) = text.split_once(' ')?;
    let (name, description) = parse_names(names)?;
    Some((flag(muted)?, name, description))
}

/// Without a name, the old format: talkd before names were kept reads it.
fn line(obligation: &Obligation) -> String {
    let Obligation {
        pin,
        name,
        description,
        was_muted,
    } = obligation;
    let muted = u8::from(*was_muted);
    match name {
        Some(name) => format!(
            "{V2} {} {} {muted} {}\n",
            pin.server,
            pin.serial,
            names(name, description.as_deref())
        ),
        None => format!("{V1} {} {} {muted}\n", pin.server, pin.serial),
    }
}

fn parse(text: &str) -> Option<Obligation> {
    if let Some(rest) = text
        .strip_prefix(V2)
        .and_then(|rest| rest.strip_prefix(' '))
    {
        let mut fields = rest.strip_suffix('\n')?.splitn(3, ' ');
        let (server, serial) = (fields.next()?, fields.next()?);
        let (was_muted, name, description) = muted_and_names(fields.next()?)?;
        return Some(Obligation {
            pin: Pin {
                server: server.parse().ok()?,
                serial: serial.parse().ok()?,
            },
            name: Some(name),
            description,
            was_muted,
        });
    }
    let words: Vec<&str> = text.split_whitespace().collect();
    let [V1, server, serial, was_muted] = words[..] else {
        return None;
    };
    Some(Obligation {
        pin: Pin {
            server: server.parse().ok()?,
            serial: serial.parse().ok()?,
        },
        name: None,
        description: None,
        was_muted: flag(was_muted)?,
    })
}

fn parse_dormant(text: &str) -> Option<Vec<Dormant>> {
    let mut lines = text.strip_suffix('\n')?.split('\n');
    if lines.next()? != DORMANT {
        return None;
    }
    lines
        .map(|line| {
            let (was_muted, name, description) = muted_and_names(line)?;
            Some(Dormant {
                name,
                description,
                was_muted,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("talkd-test-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn owed(name: &str) -> Obligation {
        Obligation {
            pin: Pin {
                server: 4_000_000_000,
                serial: 1234,
            },
            name: Some(name.to_owned()),
            description: Some(format!("{name} (described)")),
            was_muted: true,
        }
    }

    /// Spaces, colons, quotes, a backslash, a newline, and more than ASCII.
    const AWKWARD: &str = "bluez_input.AC:3E:B1:00:11:22 \"Nic's\" AirPods\\Pro\n2 – ü";

    #[test]
    fn a_record_round_trips_and_clears() {
        let dir = scratch("roundtrip");
        let store = Store::new(dir.clone()).unwrap();
        assert_eq!(store.load(), Ok(None));
        for name in ["alsa_input.pci-0000_00_1f.3", AWKWARD, ""] {
            store.save(&owed(name)).unwrap();
            assert_eq!(store.load(), Ok(Some(owed(name))));
        }
        assert!(!dir.join("obligation.tmp").exists(), "renamed into place");
        assert_eq!(
            fs::read_to_string(dir.join("obligation"))
                .unwrap()
                .lines()
                .count(),
            1,
            "one line, whatever the name"
        );
        store.clear().unwrap();
        assert_eq!(store.load(), Ok(None));
        store.clear().unwrap();
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_record_from_before_names_were_kept_still_loads_without_one() {
        let dir = scratch("v1");
        let store = Store::new(dir.clone()).unwrap();
        fs::write(
            dir.join("obligation"),
            "talkd-obligation-v1 4000000000 1234 1\n",
        )
        .unwrap();
        let old = Obligation {
            name: None,
            description: None,
            ..owed("")
        };
        assert_eq!(store.load(), Ok(Some(old.clone())));
        // and one without a name is written the old way
        store.save(&old).unwrap();
        assert_eq!(
            fs::read_to_string(dir.join("obligation")).unwrap(),
            "talkd-obligation-v1 4000000000 1234 1\n"
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_torn_or_foreign_record_is_an_error_not_an_obligation() {
        let dir = scratch("garbage");
        let store = Store::new(dir.clone()).unwrap();
        for garbage in [
            &b"talkd-obligation-v1 4000000000 1234"[..],
            b"talkd-obligation-v1 4000000000 1234 2\n",
            b"talkd-obligation-v1 -1 1234 1\n",
            b"talkd-obligation-v2 4000000000 1234 1\n",
            b"talkd-obligation-v2 4000000000 1234 1 \"mic\"",
            b"talkd-obligation-v2 4000000000 1234 1 \"mi",
            b"talkd-obligation-v2 4000000000 1234 1 mic\n",
            b"talkd-obligation-v2 4000000000  1234 1 \"mic\"\n",
            b"talkd-obligation-v2 4000000000 1234 2 \"mic\"\n",
            b"talkd-obligation-v2 4000000000 1234 1 \"mic\" x\n",
            b"talkd-obligation-v2 4000000000 1234 1 \"mic\" null x\n",
            b"talkd-obligation-v2 4000000000 1234 1 \"mic\" 7\n",
            b"talkd-obligation-v2 4000000000 1234 1 null null\n",
            b"held 1000 57 1001 1\n",
            b"\xff\xfe",
            b"",
        ] {
            fs::write(dir.join("obligation"), garbage).unwrap();
            assert!(store.load().is_err(), "{garbage:?}");
        }
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_leftover_temporary_does_not_count() {
        // a crash between writing the temporary and the rename
        let dir = scratch("leftover");
        let store = Store::new(dir.clone()).unwrap();
        fs::write(dir.join("obligation.tmp"), line(&owed("mic"))).unwrap();
        assert_eq!(store.load(), Ok(None));
        store.save(&owed("mic")).unwrap();
        assert_eq!(store.load(), Ok(Some(owed("mic"))));
        fs::write(
            dir.join("dormant.tmp"),
            "talkd-dormant-v1\n1 \"mic\" null\n",
        )
        .unwrap();
        assert_eq!(store.load_dormant(), Ok(Vec::new()));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn dormant_restores_round_trip_and_clear() {
        let dir = scratch("dormant");
        let store = Store::new(dir.clone()).unwrap();
        assert_eq!(store.load_dormant(), Ok(Vec::new()));
        let dormant = vec![
            Dormant {
                name: AWKWARD.to_owned(),
                description: Some(AWKWARD.to_owned()),
                was_muted: true,
            },
            Dormant {
                name: "alsa_input.usb-headset: mono".to_owned(),
                description: None,
                was_muted: false,
            },
        ];
        store.save_dormant(&dormant).unwrap();
        assert_eq!(store.load_dormant(), Ok(dormant.clone()));
        assert!(!dir.join("dormant.tmp").exists(), "renamed into place");
        assert_eq!(
            fs::read_to_string(dir.join("dormant"))
                .unwrap()
                .lines()
                .count(),
            3,
            "a header and a line each"
        );
        store.save_dormant(&dormant[1..]).unwrap();
        assert_eq!(store.load_dormant(), Ok(dormant[1..].to_vec()));
        // independent of the obligation
        store.save(&owed("mic")).unwrap();
        store.clear().unwrap();
        assert_eq!(store.load_dormant(), Ok(dormant[1..].to_vec()));
        store.save_dormant(&[]).unwrap();
        assert!(!dir.join("dormant").exists());
        assert_eq!(store.load_dormant(), Ok(Vec::new()));
        store.save_dormant(&[]).unwrap();
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_records_carry_names_last_as_json() {
        let dir = scratch("format");
        let store = Store::new(dir.clone()).unwrap();
        store.save(&owed("mic \"1\"")).unwrap();
        assert_eq!(
            fs::read_to_string(dir.join("obligation")).unwrap(),
            "talkd-obligation-v2 4000000000 1234 1 \"mic \\\"1\\\"\" \"mic \\\"1\\\" (described)\"\n"
        );
        store
            .save_dormant(&[Dormant {
                name: "mic".to_owned(),
                description: None,
                was_muted: false,
            }])
            .unwrap();
        assert_eq!(
            fs::read_to_string(dir.join("dormant")).unwrap(),
            "talkd-dormant-v1\n0 \"mic\" null\n"
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_torn_or_foreign_dormant_file_is_an_error() {
        let dir = scratch("dormant-garbage");
        let store = Store::new(dir.clone()).unwrap();
        for garbage in [
            &b"talkd-dormant-v1\n1 \"mic\""[..],
            b"talkd-dormant-v1\n1 mic\n",
            b"talkd-dormant-v1\n1 \"mic\" \"Mic\" null\n",
            b"talkd-dormant-v1\n1 \"mic\" true\n",
            b"talkd-dormant-v1\n2 \"mic\"\n",
            b"talkd-dormant-v1\n\n",
            b"talkd-dormant-v2\n1 \"mic\"\n",
            b"1 \"mic\"\n",
            b"talkd-obligation-v2 4000000000 1234 1 \"mic\"\n",
            b"",
        ] {
            fs::write(dir.join("dormant"), garbage).unwrap();
            assert!(store.load_dormant().is_err(), "{garbage:?}");
        }
        fs::remove_dir_all(&dir).unwrap();
    }
}
