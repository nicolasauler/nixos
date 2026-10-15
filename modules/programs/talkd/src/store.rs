//! The restore obligation on disk: one line, replaced atomically (temporary
//! file, fsync, rename, fsync of the directory), so a crash leaves either
//! the old record or the new one, never a torn one.

use std::fs::{self, File};
use std::io::{ErrorKind, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use crate::machine::{Obligation, Pin};

const MAGIC: &str = "talkd-obligation-v1";

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

    fn path(&self) -> PathBuf {
        self.dir.join("obligation")
    }

    /// The saved obligation, if any. A record that does not parse is an
    /// error; the caller drops it.
    pub fn load(&self) -> Result<Option<Obligation>, String> {
        let path = self.path();
        match fs::read(&path) {
            Ok(bytes) => parse(&String::from_utf8_lossy(&bytes))
                .map(Some)
                .ok_or_else(|| format!("{}: unreadable record", path.display())),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
            Err(error) => Err(format!("{}: {error}", path.display())),
        }
    }

    pub fn save(&self, obligation: &Obligation) -> Result<(), String> {
        let path = self.path();
        let temporary = self.dir.join("obligation.tmp");
        let failed = |error: std::io::Error| format!("{}: {error}", temporary.display());
        let mut file = File::options()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&temporary)
            .map_err(failed)?;
        file.write_all(line(obligation).as_bytes())
            .map_err(failed)?;
        file.sync_all().map_err(failed)?;
        fs::rename(&temporary, &path).map_err(|error| format!("{}: {error}", path.display()))?;
        sync_dir(&self.dir)
    }

    pub fn clear(&self) -> Result<(), String> {
        let path = self.path();
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

fn line(obligation: &Obligation) -> String {
    format!(
        "{MAGIC} {} {} {}\n",
        obligation.pin.server,
        obligation.pin.serial,
        u8::from(obligation.was_muted)
    )
}

fn parse(line: &str) -> Option<Obligation> {
    let words: Vec<&str> = line.split_whitespace().collect();
    let [MAGIC, server, serial, was_muted] = words[..] else {
        return None;
    };
    Some(Obligation {
        pin: Pin {
            server: server.parse().ok()?,
            serial: serial.parse().ok()?,
        },
        was_muted: match was_muted {
            "0" => false,
            "1" => true,
            _ => return None,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("talkd-test-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    const OWED: Obligation = Obligation {
        pin: Pin {
            server: 4_000_000_000,
            serial: 1234,
        },
        was_muted: true,
    };

    #[test]
    fn a_record_round_trips_and_clears() {
        let dir = scratch("roundtrip");
        let store = Store::new(dir.clone()).unwrap();
        assert_eq!(store.load(), Ok(None));
        store.save(&OWED).unwrap();
        assert_eq!(store.load(), Ok(Some(OWED)));
        assert!(!dir.join("obligation.tmp").exists(), "renamed into place");
        store.clear().unwrap();
        assert_eq!(store.load(), Ok(None));
        store.clear().unwrap();
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
        fs::write(dir.join("obligation.tmp"), line(&OWED)).unwrap();
        assert_eq!(store.load(), Ok(None));
        store.save(&OWED).unwrap();
        assert_eq!(store.load(), Ok(Some(OWED)));
        fs::remove_dir_all(&dir).unwrap();
    }
}
