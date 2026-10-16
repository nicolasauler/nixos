//! Pulls the latest saved revision of the Oryx layout pinned in
//! firmware/voyager/oryx/oryx.json into firmware/voyager/oryx/. Run from the
//! root of the nixos checkout: `nix run .#voyager-layout-update`.
//!
//! Oryx's public GraphQL API names the latest revision; its source export (a
//! zip with `<layout>_source/{keymap.c,config.h,rules.mk,keymap.json}`, a
//! README, a build log and a compiled .bin) is fetched from
//! /source/<revision>. This is what ZSA's own "Oryx with custom QMK" tooling
//! does.
//!
//! The directory holds one whole revision, the four source files and the pin,
//! and is replaced as a whole: the new revision is staged in a sibling
//! directory, synced, and exchanged with the live one in a single rename. A
//! failure before that rename leaves the old revision; one after it leaves the
//! new one, with only its durability or the old tree's removal in doubt. Never
//! a mix. One updater runs at a time: each holds an exclusive lock on a file
//! beside the directory (`.oryx.lock`) for the whole run, and a second one
//! stops with an error rather than touch the staging directory.

use std::collections::HashSet;
use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{self, Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use nix::fcntl::{AT_FDCWD, RenameFlags, renameat2};
use serde_json::{Value, json};

/// The vendored revision: FILES and PIN, nothing else.
const DIR: &str = "firmware/voyager/oryx";
const PIN: &str = "oryx.json";
const FILES: [&str; 4] = ["keymap.c", "config.h", "rules.mk", "keymap.json"];
/// The ZSA QMK branch (firmwareNN) that firmware/voyager/default.nix builds.
const FIRMWARE_MAJOR: &str = "25";
/// ureq's `timeout_global` for each HTTP request to Oryx (each its own, not
/// one shared budget). It stops a server that goes silent, but it is not a
/// hard wall-clock ceiling: the TLS layer reuses the relative timeout on each
/// underlying read, so a server trickling one TLS record can stretch a single
/// read past it.
const TIMEOUT: Duration = Duration::from_secs(60);
const QUERY: &str = "query($hashId: String!, $revisionId: String!, $geometry: String) { \
    layout(hashId: $hashId, geometry: $geometry, revisionId: $revisionId) { \
    revision { hashId qmkVersion title } } }";

#[derive(Debug, PartialEq)]
struct Revision {
    hash_id: String,
    firmware: String,
    title: String,
}

fn parse_revision(answer: &str) -> Result<Revision, String> {
    let value: Value =
        serde_json::from_str(answer).map_err(|error| format!("Oryx answered non-JSON: {error}"))?;
    let revision = &value["data"]["layout"]["revision"];
    let field = |name: &str| {
        revision[name]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| format!("Oryx's answer has no revision {name}: {answer}"))
    };
    Ok(Revision {
        hash_id: field("hashId")?,
        firmware: field("qmkVersion")?,
        title: revision["title"].as_str().unwrap_or_default().to_owned(),
    })
}

fn firmware_major(version: &str) -> &str {
    version.split('.').next().unwrap_or(version)
}

/// The little-endian integer of `width` bytes at `at`.
fn le(bytes: &[u8], at: usize, width: usize) -> Option<usize> {
    let field = bytes.get(at..at.checked_add(width)?)?;
    Some(
        field
            .iter()
            .rev()
            .fold(0, |value, &byte| value << 8 | usize::from(byte)),
    )
}

/// One central-directory record: where it starts, and the raw name it gives.
#[derive(Debug)]
struct Record<'a> {
    offset: usize,
    name: &'a [u8],
}

/// The zip's central directory as read here, rather than through ZipArchive,
/// which keeps one entry per name (of two records with the same name only the
/// last would ever be seen) and finds its directory by a search of its own.
#[derive(Debug)]
struct CentralDirectory<'a> {
    start: usize,
    records: Vec<Record<'a>>,
}

/// Reads the central directory named by the end-of-central-directory record
/// whose comment runs exactly to the end of the file. Every record must lie
/// wholly inside the directory, and the records must fill it. Zip64 is refused
/// rather than half read where this reads it: zip64 values in the selected
/// end record, a zip64 end-of-directory locator before it (more than 65,534
/// entries or 4 GiB), and zip64 extra fields (header 0x0001) in central
/// directory records. Local file headers are not inspected: the zip crate
/// takes sizes and offsets from the central directory, which is checked here.
fn central_directory(zip: &[u8]) -> Result<CentralDirectory<'_>, String> {
    const END: &[u8] = b"PK\x05\x06";
    const ZIP64_LOCATOR: &[u8] = b"PK\x06\x07";
    const RECORD: &[u8] = b"PK\x01\x02";
    const ZIP64_EXTRA: usize = 0x0001;
    let corrupt = |why: &str| format!("the export is not a zip: {why}");
    let zip64 = || "the export uses zip64, which this updater does not read".to_owned();
    // the end-of-central-directory record: 22 bytes, then a comment of up to
    // 64 KiB that runs to the end of the file
    let last = zip
        .len()
        .checked_sub(22)
        .ok_or_else(|| corrupt("too short"))?;
    let end = (last.saturating_sub(0xFFFF)..=last)
        .rev()
        .find(|&at| zip[at..].starts_with(END) && le(zip, at + 20, 2) == Some(last - at))
        .ok_or_else(|| corrupt("no end of central directory"))?;
    let field = |at, width| le(zip, end + at, width).ok_or_else(|| corrupt("truncated"));
    let (on_disk, entries, size, start) =
        (field(8, 2)?, field(10, 2)?, field(12, 4)?, field(16, 4)?);
    if entries == 0xFFFF
        || size == 0xFFFF_FFFF
        || start == 0xFFFF_FFFF
        || end
            .checked_sub(20)
            .is_some_and(|locator| zip[locator..].starts_with(ZIP64_LOCATOR))
    {
        return Err(zip64());
    }
    if on_disk != entries || field(4, 2)? != 0 || field(6, 2)? != 0 {
        return Err(corrupt("a multi-disk archive"));
    }
    let directory_end = start
        .checked_add(size)
        .filter(|&directory_end| directory_end <= end)
        .ok_or_else(|| corrupt("the central directory runs past its end record"))?;
    // only the directory's own bytes: a record cannot reach beyond them
    let directory = &zip[..directory_end];
    let truncated = || corrupt("a central directory record runs past the directory");
    let mut records = Vec::with_capacity(entries);
    let mut at = start;
    for _ in 0..entries {
        let field = |offset, width| le(directory, at + offset, width).ok_or_else(truncated);
        if !directory[at.min(directory_end)..].starts_with(RECORD) {
            return Err(corrupt("a central directory record is missing"));
        }
        let (name, extra, comment) = (field(28, 2)?, field(30, 2)?, field(32, 2)?);
        let name_at = at + 46;
        let extra_at = name_at + name;
        let next = extra_at + extra + comment;
        if next > directory_end {
            return Err(truncated());
        }
        let mut extra = &directory[extra_at..extra_at + extra];
        while !extra.is_empty() {
            let (id, length) = (le(extra, 0, 2), le(extra, 2, 2));
            let (Some(id), Some(length)) = (id, length) else {
                return Err(corrupt("a malformed extra field"));
            };
            if id == ZIP64_EXTRA {
                return Err(zip64());
            }
            extra = extra
                .get(4 + length..)
                .ok_or_else(|| corrupt("a malformed extra field"))?;
        }
        records.push(Record {
            offset: at,
            name: &directory[name_at..extra_at],
        });
        at = next;
    }
    if at != directory_end {
        return Err(corrupt("the central directory is not the size it claims"));
    }
    Ok(CentralDirectory { start, records })
}

/// The four layout files of an Oryx export, in FILES order. They must be
/// immediate children of the export's one top-level `*_source/` directory.
/// Anything else that could be a source tree (a second source directory, one
/// below the top level, a nested path inside it, a source file named twice,
/// whether as exactly the same name or one spelt differently) is an error
/// rather than a guess at which copy is meant, as is any entry name that
/// appears twice in the archive. The zip crate must read exactly the central
/// directory checked here: the same start, the same records at the same
/// offsets with the same names, so a file with two candidate directories
/// cannot be validated by one and extracted from the other.
fn source_files(zip: &[u8]) -> Result<Vec<(&'static str, Vec<u8>)>, String> {
    let directory = central_directory(zip)?;
    let mut seen = HashSet::new();
    if let Some(twice) = directory
        .records
        .iter()
        .find(|record| !seen.insert(record.name))
    {
        return Err(format!(
            "the export has two entries named {}",
            String::from_utf8_lossy(twice.name)
        ));
    }
    let mut archive = zip::ZipArchive::new(Cursor::new(zip))
        .map_err(|error| format!("the export is not a zip: {error}"))?;
    let differs = |what: String| {
        Err(format!(
            "the zip reader reads another central directory than the one checked: {what}"
        ))
    };
    if archive.central_directory_start() != directory.start as u64 {
        return differs(format!(
            "it starts at {}, not {}",
            archive.central_directory_start(),
            directory.start
        ));
    }
    if archive.len() != directory.records.len() {
        return differs(format!(
            "{} entries, not {}",
            archive.len(),
            directory.records.len()
        ));
    }
    for (index, record) in directory.records.iter().enumerate() {
        let entry = archive
            .by_index_raw(index)
            .map_err(|error| error.to_string())?;
        if entry.central_header_start() != record.offset as u64 || entry.name_raw() != record.name {
            return differs(format!(
                "entry {index} is {} at {}, not {} at {}",
                String::from_utf8_lossy(entry.name_raw()),
                entry.central_header_start(),
                String::from_utf8_lossy(record.name),
                record.offset
            ));
        }
    }
    let mut root: Option<String> = None;
    let mut found: [Option<Vec<u8>>; FILES.len()] = Default::default();
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|error| error.to_string())?;
        let path = entry
            .name()
            .map_err(|error| error.to_string())?
            .into_owned();
        // "a//b" and "./a/b" name the same file as "a/b"
        let parts: Vec<&str> = path
            .split('/')
            .filter(|part| !part.is_empty() && *part != ".")
            .collect();
        let dirs = if entry.is_dir() {
            &parts[..]
        } else {
            &parts[..parts.len().saturating_sub(1)]
        };
        let Some(depth) = dirs.iter().position(|dir| dir.ends_with("_source")) else {
            continue;
        };
        if depth != 0 {
            return Err(format!(
                "the export has a source directory below its top level: {path}"
            ));
        }
        match &root {
            None => root = Some(parts[0].to_owned()),
            Some(root) if root != parts[0] => {
                return Err(format!(
                    "the export has more than one source directory: {root}/ and {}/",
                    parts[0]
                ));
            }
            Some(_) => {}
        }
        let name = match parts[1..] {
            [] => continue,
            [name] if !entry.is_dir() => name,
            _ => {
                return Err(format!(
                    "the export's source directory has a nested path: {path}"
                ));
            }
        };
        let Some(slot) = FILES.iter().position(|&file| file == name) else {
            continue;
        };
        if found[slot].is_some() {
            return Err(format!("the export has {name} twice: {path}"));
        }
        let mut content = Vec::new();
        entry
            .read_to_end(&mut content)
            .map_err(|error| format!("{path}: {error}"))?;
        found[slot] = Some(content);
    }
    if root.is_none() {
        return Err("the export has no *_source/ directory".to_owned());
    }
    FILES
        .iter()
        .zip(found)
        .map(|(&name, content)| {
            content
                .ok_or_else(|| format!("{name} is missing from the export"))
                .map(|content| (name, content))
        })
        .collect()
}

/// The filesystem steps of publishing a revision, each of which can fail: a
/// seam, so the tests can make each one fail or pause.
trait Ops {
    /// Creates `path` with `content` and syncs it to disk.
    fn write(&mut self, path: &Path, content: &[u8]) -> io::Result<()>;
    /// Syncs a directory's entries to disk.
    fn sync_dir(&mut self, dir: &Path) -> io::Result<()>;
    /// Swaps two directories in one step.
    fn exchange(&mut self, staged: &Path, live: &Path) -> io::Result<()>;
    /// Removes a directory and everything in it.
    fn remove_tree(&mut self, dir: &Path) -> io::Result<()>;
}

struct Disk;

impl Ops for Disk {
    fn write(&mut self, path: &Path, content: &[u8]) -> io::Result<()> {
        let mut file = File::create_new(path)?;
        file.write_all(content)?;
        file.sync_all()
    }

    fn sync_dir(&mut self, dir: &Path) -> io::Result<()> {
        File::open(dir)?.sync_all()
    }

    fn exchange(&mut self, staged: &Path, live: &Path) -> io::Result<()> {
        renameat2(
            AT_FDCWD,
            staged,
            AT_FDCWD,
            live,
            RenameFlags::RENAME_EXCHANGE,
        )
        .map_err(io::Error::from)
    }

    fn remove_tree(&mut self, dir: &Path) -> io::Result<()> {
        fs::remove_dir_all(dir)
    }
}

/// Sole use of the vendored directory `live` and its staging directory
/// `.<name>.staged` beside it, for as long as this lives: an exclusive flock
/// on `.<name>.lock`, also beside it, so outside the directories that are
/// exchanged. The kernel drops the lock when the process ends, crash or not.
struct Publication<'a> {
    live: &'a Path,
    parent: PathBuf,
    staged: PathBuf,
    _lock: File,
}

impl<'a> Publication<'a> {
    /// Takes the lock, or fails at once if another updater holds it, then
    /// removes whatever an interrupted run left at the staging directory.
    fn begin(live: &'a Path, ops: &mut impl Ops) -> Result<Self, String> {
        let name = live
            .file_name()
            .ok_or_else(|| format!("{} has no directory name", live.display()))?
            .to_string_lossy();
        let parent = live
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or(Path::new("."))
            .to_owned();
        let lock_path = parent.join(format!(".{name}.lock"));
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&lock_path)
            .map_err(|error| format!("{}: {error}", lock_path.display()))?;
        match lock.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => {
                return Err(format!(
                    "another voyager-layout-update is running (it holds {}); let it finish, then run again",
                    lock_path.display()
                ));
            }
            Err(TryLockError::Error(error)) => {
                return Err(format!("locking {}: {error}", lock_path.display()));
            }
        }
        let publication = Self {
            live,
            staged: parent.join(format!(".{name}.staged")),
            parent,
            _lock: lock,
        };
        publication.recover(ops)?;
        Ok(publication)
    }

    /// Removes the staging directory if a run that held the lock was stopped
    /// with it in place: an unpublished new tree (stopped before the
    /// exchange) or the old one (stopped after). Neither is live, and no one
    /// else is using it while the lock is held.
    fn recover(&self, ops: &mut impl Ops) -> Result<(), String> {
        match fs::symlink_metadata(&self.staged) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(format!("{}: {error}", self.staged.display())),
            Ok(_) => {}
        }
        ops.remove_tree(&self.staged).map_err(|error| {
            format!(
                "removing {}, left by an interrupted run: {error}",
                self.staged.display()
            )
        })?;
        ops.sync_dir(&self.parent).map_err(|error| {
            format!(
                "syncing {} after removing the tree an interrupted run left: {error}",
                self.parent.display()
            )
        })
    }

    /// Replaces `live` with a directory holding exactly `files`, at once: the
    /// new tree is staged beside it (same filesystem), every file and the
    /// directory synced, then exchanged with `live` by
    /// renameat2(RENAME_EXCHANGE), the exchange synced, and the old tree
    /// removed and that synced. A failure before the exchange leaves `live` as
    /// it was; from the exchange on, `live` is wholly new, and an error says
    /// so.
    fn publish(&self, files: &[(&str, Vec<u8>)], ops: &mut impl Ops) -> Result<(), String> {
        if let Err(error) = self.stage_and_exchange(files, ops) {
            // best effort: what is left there is the unpublished new tree, and
            // the next run removes it anyway
            let _ = fs::remove_dir_all(&self.staged);
            return Err(format!("{error}; {} is unchanged", self.live.display()));
        }
        let published = |what: String| {
            format!(
                "{} now holds the new revision, but {what}",
                self.live.display()
            )
        };
        ops.sync_dir(&self.parent).map_err(|error| {
            published(format!(
                "syncing {} failed, so the switch may not survive a crash: {error}",
                self.parent.display()
            ))
        })?;
        // `staged` now holds the old tree
        ops.remove_tree(&self.staged).map_err(|error| {
            published(format!(
                "removing the old revision at {} failed (the next run removes it): {error}",
                self.staged.display()
            ))
        })?;
        ops.sync_dir(&self.parent).map_err(|error| {
            published(format!(
                "syncing {} after removing the old revision failed: {error}",
                self.parent.display()
            ))
        })
    }

    /// Writes `files` into the new staging directory, syncs it and exchanges
    /// it with `live`.
    fn stage_and_exchange(
        &self,
        files: &[(&str, Vec<u8>)],
        ops: &mut impl Ops,
    ) -> Result<(), String> {
        let staged = &self.staged;
        fs::create_dir(staged).map_err(|error| format!("{}: {error}", staged.display()))?;
        for (name, content) in files {
            let path = staged.join(name);
            ops.write(&path, content)
                .map_err(|error| format!("{}: {error}", path.display()))?;
        }
        ops.sync_dir(staged)
            .map_err(|error| format!("syncing {}: {error}", staged.display()))?;
        ops.exchange(staged, self.live).map_err(|error| {
            format!(
                "exchanging {} with {}: {error}",
                staged.display(),
                self.live.display()
            )
        })
    }
}

fn run() -> Result<(), String> {
    let live = Path::new(DIR);
    if !live.is_dir() {
        return Err("run me from the root of the nixos checkout".to_owned());
    }
    // held until run returns, also while the network is asked; it cleans up
    // after an interrupted run even when there is nothing new to publish
    let publication = Publication::begin(live, &mut Disk)?;
    let pin_path = live.join(PIN);
    let pin_text = fs::read_to_string(&pin_path)
        .map_err(|error| format!("{}: {error}", pin_path.display()))?;
    let pin: Value = serde_json::from_str(&pin_text)
        .map_err(|error| format!("{}: {error}", pin_path.display()))?;
    let layout = pin["layout"].as_str().ok_or("oryx.json has no layout")?;
    let current = pin["revision"].as_str().unwrap_or_default();

    let request = json!({
        "query": QUERY,
        "variables": {"hashId": layout, "geometry": "voyager", "revisionId": "latest"},
    });
    // one agent for both requests, each with its own `timeout_global` (see
    // TIMEOUT for what that does and does not bound); the lock is held
    // meanwhile, so a silent server must not hold it for ever
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .build()
        .into();
    let mut response = agent
        .post("https://oryx.zsa.io/graphql")
        .header("Content-Type", "application/json")
        .send(request.to_string())
        .map_err(|error| format!("Oryx GraphQL: {error}"))?;
    let answer = response
        .body_mut()
        .read_to_string()
        .map_err(|error| format!("Oryx GraphQL: {error}"))?;
    let latest = parse_revision(&answer)?;

    if latest.hash_id == current {
        println!("{layout}/{current} is already the latest revision");
        return Ok(());
    }
    if firmware_major(&latest.firmware) != FIRMWARE_MAJOR {
        return Err(format!(
            "{layout}/{} is for Oryx firmware {}, but firmware/voyager/default.nix builds \
             ZSA's firmware{FIRMWARE_MAJOR} branch: bump that first",
            latest.hash_id, latest.firmware
        ));
    }

    let url = format!("https://oryx.zsa.io/source/{}", latest.hash_id);
    let mut response = agent
        .get(&url)
        .call()
        .map_err(|error| format!("{url}: {error}"))?;
    let zip = response
        .body_mut()
        .with_config()
        .limit(64 << 20)
        .read_to_vec()
        .map_err(|error| format!("{url}: {error}"))?;
    let mut files = source_files(&zip)?;

    let pin = json!({
        "layout": layout,
        "revision": latest.hash_id,
        "firmware": latest.firmware,
        "title": latest.title,
    });
    let pin_text = serde_json::to_string_pretty(&pin).map_err(|error| error.to_string())? + "\n";
    files.push((PIN, pin_text.into_bytes()));
    publication.publish(&files, &mut Disk)?;
    println!(
        "{layout}: {current} -> {} ({})",
        latest.hash_id, latest.title
    );
    println!("next: nix build .#voyager-firmware; after review, flash its local .bin with: env NIXPKGS_ALLOW_UNFREE=1 nix run --impure nixpkgs#zapp -- flash result/*.bin");
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("voyager-layout-update: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::io::Write;
    use std::sync::mpsc;
    use std::thread;

    use super::*;

    fn export(entries: &[(&str, &str)]) -> Vec<u8> {
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (path, content) in entries {
            zip.start_file(*path, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(content.as_bytes()).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }

    #[test]
    fn reads_the_latest_revision_from_oryx_s_answer() {
        // the shape Oryx returned for this layout
        let answer = r#"{"data":{"layout":{"title":"Colemak dh nicc","privacy":false,"revision":{"hashId":"bvrAW7","qmkVersion":"25.0","title":"revert mouse stuff"}}}}"#;
        assert_eq!(
            parse_revision(answer),
            Ok(Revision {
                hash_id: "bvrAW7".into(),
                firmware: "25.0".into(),
                title: "revert mouse stuff".into(),
            })
        );
    }

    #[test]
    fn an_unknown_layout_is_an_error_not_an_empty_revision() {
        let answer = r#"{"data":{"layout":null},"errors":[{"message":"not found"}]}"#;
        assert!(parse_revision(answer).unwrap_err().contains("hashId"));
    }

    /// The four source files at the top of one source directory, as Oryx's
    /// export has them, plus `extra`.
    fn export_with(extra: &[(&str, &str)]) -> Vec<u8> {
        let mut entries = vec![
            ("x_source/keymap.c", "keys"),
            ("x_source/config.h", "config"),
            ("x_source/rules.mk", "rules"),
            ("x_source/keymap.json", "{}"),
        ];
        entries.extend_from_slice(extra);
        export(&entries)
    }

    #[test]
    fn takes_the_four_source_files_and_nothing_else() {
        // the entries of Oryx's export of this layout's revision bvrAW7
        let zip = export(&[
            ("README.md", "readme"),
            ("build.log", "log"),
            ("zsa_voyager_JqpmQ.bin", "firmware"),
            ("zsa_voyager_JqpmQ.bin.md5", "md5"),
            ("zsa_voyager_colemak-dh-nicc_source/", ""),
            ("zsa_voyager_colemak-dh-nicc_source/config.h", "config"),
            ("zsa_voyager_colemak-dh-nicc_source/keymap.c", "keys"),
            ("zsa_voyager_colemak-dh-nicc_source/keymap.json", "{}"),
            ("zsa_voyager_colemak-dh-nicc_source/rules.mk", "rules"),
        ]);
        let files = source_files(&zip).unwrap();
        let names: Vec<&str> = files.iter().map(|(name, _)| *name).collect();
        assert_eq!(names, FILES);
        assert_eq!(files[0].1, b"keys");
    }

    #[test]
    fn a_nested_path_in_the_source_directory_is_an_error() {
        let zip = export_with(&[("x_source/backup/keymap.c", "old keys")]);
        assert!(source_files(&zip).unwrap_err().contains("nested path"));
    }

    #[test]
    fn a_second_source_directory_is_an_error() {
        let zip = export_with(&[("y_source/config.h", "other config")]);
        assert!(
            source_files(&zip)
                .unwrap_err()
                .contains("more than one source directory")
        );
    }

    #[test]
    fn a_source_directory_below_the_top_level_is_an_error() {
        let zip = export_with(&[("old/y_source/keymap.c", "old keys")]);
        assert!(
            source_files(&zip)
                .unwrap_err()
                .contains("below its top level")
        );
    }

    #[test]
    fn a_source_file_named_twice_is_an_error() {
        let zip = export_with(&[("x_source//keymap.c", "other keys")]);
        assert!(source_files(&zip).unwrap_err().contains("keymap.c twice"));
    }

    /// The source export plus a fifth entry, also named x_source/keymap.c
    /// but with other content. ZipWriter refuses a name twice, so it writes
    /// another name of the same length, renamed here in the bytes, local
    /// header and central directory alike.
    fn export_with_keymap_twice() -> Vec<u8> {
        let mut zip = export_with(&[("x_source/keymap.X", "other keys")]);
        let (from, to) = (b"x_source/keymap.X", b"x_source/keymap.c");
        let mut renamed = 0;
        for at in 0..=zip.len() - from.len() {
            if &zip[at..at + from.len()] == from {
                zip[at..at + to.len()].copy_from_slice(to);
                renamed += 1;
            }
        }
        assert_eq!(renamed, 2);
        zip
    }

    /// An end-of-central-directory record: `entries` records, `size` bytes
    /// from `start`, then a comment of `comment` bytes.
    fn end_record(entries: u16, size: usize, start: usize, comment: u16) -> Vec<u8> {
        let mut end = b"PK\x05\x06\0\0\0\0".to_vec();
        end.extend_from_slice(&entries.to_le_bytes());
        end.extend_from_slice(&entries.to_le_bytes());
        end.extend_from_slice(&u32::try_from(size).unwrap().to_le_bytes());
        end.extend_from_slice(&u32::try_from(start).unwrap().to_le_bytes());
        end.extend_from_slice(&comment.to_le_bytes());
        end
    }

    #[test]
    fn two_entries_with_exactly_the_same_name_are_an_error() {
        let zip = export_with_keymap_twice();
        // the zip crate reads it, and sees one keymap.c of the two
        let archive = zip::ZipArchive::new(Cursor::new(&zip[..])).unwrap();
        assert_eq!(archive.len(), 4);
        assert_eq!(central_directory(&zip).unwrap().records.len(), 5);
        assert!(
            source_files(&zip)
                .unwrap_err()
                .contains("two entries named x_source/keymap.c")
        );
    }

    #[test]
    fn the_zip_reader_must_read_the_central_directory_that_was_checked() {
        // Two central directories: the five records with keymap.c twice, then
        // the first four, distinct. The last end record, whose comment runs
        // to the end of the file, names the four; its comment hides another
        // end record, naming the five, followed by one more byte.
        let zip = export_with_keymap_twice();
        let five = central_directory(&zip).unwrap();
        let (start, fifth) = (five.start, five.records[4].offset);
        let end = zip.len() - 22;
        let mut ambiguous = zip[..end].to_vec();
        let four_start = ambiguous.len();
        ambiguous.extend_from_within(start..fifth);
        let four_size = ambiguous.len() - four_start;
        ambiguous.extend(end_record(4, four_size, four_start, 23));
        ambiguous.extend(end_record(5, end - start, start, 0));
        ambiguous.push(0);

        let checked = central_directory(&ambiguous).unwrap();
        assert_eq!(checked.start, four_start);
        assert_eq!(checked.records.len(), 4);
        // the zip crate takes the hidden one, and sees keymap.c once
        let archive = zip::ZipArchive::new(Cursor::new(&ambiguous[..])).unwrap();
        assert_eq!(archive.central_directory_start(), start as u64);
        assert_eq!(archive.len(), 4);
        let error = source_files(&ambiguous).unwrap_err();
        assert!(
            error.contains("reads another central directory than the one checked"),
            "{error}"
        );
    }

    #[test]
    fn a_zip64_end_record_is_refused() {
        let mut zip = export_with(&[]);
        let end = zip.len() - 22;
        assert_eq!(&zip[end..end + 4], b"PK\x05\x06");
        zip[end + 8..end + 12].copy_from_slice(&[0xFF; 4]);
        assert!(source_files(&zip).unwrap_err().contains("zip64"));
    }

    #[test]
    fn a_zip64_extra_field_on_an_entry_is_refused() {
        // a small archive with ordinary end values, but whose entry carries
        // zip64 sizes in an extra field (header 0x0001)
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let large = zip::write::SimpleFileOptions::default().large_file(true);
        zip.start_file("x_source/keymap.c", large).unwrap();
        zip.write_all(b"keys").unwrap();
        let zip = zip.finish().unwrap().into_inner();
        let end = zip.len() - 22;
        assert_eq!(&zip[end + 8..end + 10], &1u16.to_le_bytes());
        let record = usize::try_from(u32::from_le_bytes(
            zip[end + 16..end + 20].try_into().unwrap(),
        ))
        .unwrap();
        let extra = record + 46 + "x_source/keymap.c".len();
        assert_ne!(&zip[record + 30..record + 32], &[0, 0], "no extra field");
        assert_eq!(&zip[extra..extra + 2], &1u16.to_le_bytes(), "not zip64");
        assert!(zip::ZipArchive::new(Cursor::new(&zip[..])).is_ok());
        assert!(source_files(&zip).unwrap_err().contains("zip64"));
    }

    #[test]
    fn an_export_without_a_source_directory_is_an_error() {
        let zip = export(&[("keymap.c", ""), ("config.h", "")]);
        assert!(source_files(&zip).unwrap_err().contains("no *_source/"));
    }

    #[test]
    fn a_missing_source_file_is_an_error_before_anything_is_written() {
        let zip = export(&[
            ("x_source/keymap.c", ""),
            ("x_source/config.h", ""),
            ("x_source/keymap.json", ""),
        ]);
        assert!(source_files(&zip).unwrap_err().contains("rules.mk"));
    }

    #[test]
    fn the_firmware_branch_is_the_major_version() {
        assert_eq!(firmware_major("25.0"), "25");
        assert_eq!(firmware_major("26"), "26");
    }

    /// A directory of its own under the temporary directory, removed on drop.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(test: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "voyager-layout-update-{}-{test}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn revision(tag: &str) -> Vec<(&'static str, Vec<u8>)> {
        FILES
            .iter()
            .chain([&PIN])
            .map(|&name| (name, format!("{name} of {tag}\n").repeat(64).into_bytes()))
            .collect()
    }

    /// A live tree holding revision "old", in a scratch directory.
    fn live_old(test: &str) -> (Scratch, PathBuf) {
        let scratch = Scratch::new(test);
        let live = scratch.0.join("oryx");
        fs::create_dir(&live).unwrap();
        for (name, content) in revision("old") {
            fs::write(live.join(name), content).unwrap();
        }
        (scratch, live)
    }

    fn tree(dir: &Path) -> BTreeMap<String, Vec<u8>> {
        fs::read_dir(dir)
            .unwrap()
            .map(|entry| {
                let entry = entry.unwrap();
                let name = entry.file_name().into_string().unwrap();
                (name, fs::read(entry.path()).unwrap())
            })
            .collect()
    }

    fn expected(tag: &str) -> BTreeMap<String, Vec<u8>> {
        revision(tag)
            .into_iter()
            .map(|(name, content)| (name.to_owned(), content))
            .collect()
    }

    /// The scratch directory holds the live tree, the lock file and nothing
    /// else: no staged or old tree is left behind.
    fn only_live(scratch: &Scratch) -> bool {
        let mut names: Vec<String> = fs::read_dir(&scratch.0)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names == [".oryx.lock", "oryx"]
    }

    fn staged(scratch: &Scratch) -> PathBuf {
        scratch.0.join(".oryx.staged")
    }

    /// Disk, except that step number `fail` (every operation, counted from 1)
    /// fails; a failing write first writes half its content, as a short write
    /// or a full disk would, and a failing removal removes nothing.
    struct FailAt {
        fail: usize,
        step: usize,
    }

    impl FailAt {
        fn new(fail: usize) -> Self {
            Self { fail, step: 0 }
        }

        fn fails_now(&mut self, what: &str) -> io::Result<()> {
            self.step += 1;
            if self.step == self.fail {
                return Err(io::Error::other(format!("injected {what} failure")));
            }
            Ok(())
        }
    }

    impl Ops for FailAt {
        fn write(&mut self, path: &Path, content: &[u8]) -> io::Result<()> {
            if let Err(error) = self.fails_now("write") {
                fs::write(path, &content[..content.len() / 2])?;
                return Err(error);
            }
            Disk.write(path, content)
        }

        fn sync_dir(&mut self, dir: &Path) -> io::Result<()> {
            self.fails_now("sync")?;
            Disk.sync_dir(dir)
        }

        fn exchange(&mut self, staged: &Path, live: &Path) -> io::Result<()> {
            self.fails_now("exchange")?;
            Disk.exchange(staged, live)
        }

        fn remove_tree(&mut self, dir: &Path) -> io::Result<()> {
            self.fails_now("removal")?;
            Disk.remove_tree(dir)
        }
    }

    /// The operations of publishing, in order: five writes, syncing the
    /// staged directory, the exchange, syncing the parent, removing the old
    /// tree, syncing the parent again.
    const STEPS: usize = FILES.len() + 6;
    const EXCHANGE: usize = FILES.len() + 3;

    fn publish(live: &Path, tag: &str, ops: &mut impl Ops) -> Result<(), String> {
        Publication::begin(live, &mut Disk)?.publish(&revision(tag), ops)
    }

    #[test]
    fn publishing_replaces_the_whole_tree() {
        let (scratch, live) = live_old("success");
        let mut counted = FailAt::new(0);
        publish(&live, "new", &mut counted).unwrap();
        assert_eq!(counted.step, STEPS);
        assert_eq!(tree(&live), expected("new"));
        assert!(only_live(&scratch));
    }

    #[test]
    fn a_failure_at_any_step_before_the_exchange_leaves_the_old_tree() {
        for fail in 1..=EXCHANGE {
            let (scratch, live) = live_old(&format!("fail-{fail}"));
            let error = publish(&live, "new", &mut FailAt::new(fail)).unwrap_err();
            assert!(error.contains("injected"), "step {fail}: {error}");
            assert!(error.ends_with("is unchanged"), "step {fail}: {error}");
            assert_eq!(tree(&live), expected("old"), "step {fail}");
            assert!(only_live(&scratch), "step {fail}");
        }
    }

    #[test]
    fn a_failure_after_the_exchange_leaves_the_new_tree_and_says_so() {
        for fail in EXCHANGE + 1..=STEPS {
            let (scratch, live) = live_old(&format!("after-{fail}"));
            let error = publish(&live, "new", &mut FailAt::new(fail)).unwrap_err();
            assert!(error.contains("injected"), "step {fail}: {error}");
            assert!(
                error.contains("now holds the new revision"),
                "step {fail}: {error}"
            );
            assert!(!error.contains("unchanged"), "step {fail}: {error}");
            assert_eq!(tree(&live), expected("new"), "step {fail}");
            // the next run, even one with nothing to publish, removes the old
            // tree a failed removal left
            drop(Publication::begin(&live, &mut Disk).unwrap());
            assert_eq!(tree(&live), expected("new"), "step {fail}");
            assert!(only_live(&scratch), "step {fail}");
        }
    }

    #[test]
    fn a_tree_left_by_an_interrupted_run_is_removed_on_the_next_run() {
        let (scratch, live) = live_old("stale");
        fs::create_dir(staged(&scratch)).unwrap();
        fs::write(staged(&scratch).join("keymap.c"), "half a keymap").unwrap();
        let mut counted = FailAt::new(0);
        drop(Publication::begin(&live, &mut counted).unwrap());
        // the removal, then syncing the parent
        assert_eq!(counted.step, 2);
        assert_eq!(tree(&live), expected("old"));
        assert!(only_live(&scratch));
    }

    #[test]
    fn a_failure_removing_a_left_over_tree_stops_before_anything_is_staged() {
        for fail in 1..=2 {
            let (scratch, live) = live_old(&format!("recover-{fail}"));
            fs::create_dir(staged(&scratch)).unwrap();
            let error = Publication::begin(&live, &mut FailAt::new(fail))
                .err()
                .unwrap();
            assert!(error.contains("injected"), "step {fail}: {error}");
            assert!(error.contains("interrupted run"), "step {fail}: {error}");
            assert_eq!(tree(&live), expected("old"), "step {fail}");
        }
    }

    /// Disk, except that the exchange first says it is about to happen, then
    /// waits to be let go.
    struct PauseBeforeExchange {
        about_to_exchange: mpsc::Sender<()>,
        go: mpsc::Receiver<()>,
    }

    impl Ops for PauseBeforeExchange {
        fn write(&mut self, path: &Path, content: &[u8]) -> io::Result<()> {
            Disk.write(path, content)
        }

        fn sync_dir(&mut self, dir: &Path) -> io::Result<()> {
            Disk.sync_dir(dir)
        }

        fn exchange(&mut self, staged: &Path, live: &Path) -> io::Result<()> {
            self.about_to_exchange.send(()).unwrap();
            self.go.recv().unwrap();
            Disk.exchange(staged, live)
        }

        fn remove_tree(&mut self, dir: &Path) -> io::Result<()> {
            Disk.remove_tree(dir)
        }
    }

    #[test]
    fn a_second_updater_cannot_touch_the_first_one_s_staging() {
        let (scratch, live) = live_old("two");
        let (about_to_exchange, paused) = mpsc::channel();
        thread::scope(|threads| {
            // made in here, so that a failed assertion below drops `go` as it
            // unwinds, before the scope joins the first publisher: that one
            // then fails at its pause instead of waiting there for ever
            let (go, wait) = mpsc::channel();
            let first = threads.spawn(|| {
                let mut ops = PauseBeforeExchange {
                    about_to_exchange,
                    go: wait,
                };
                publish(&live, "first", &mut ops)
            });
            paused.recv().unwrap();
            // the first one has staged its tree in full and is about to
            // exchange it; the second, whatever it would publish, stops before
            // its stale-tree cleanup or staging
            let error = publish(&live, "second", &mut Disk).unwrap_err();
            assert!(
                error.contains("another voyager-layout-update is running"),
                "{error}"
            );
            assert_eq!(tree(&live), expected("old"));
            assert_eq!(tree(&staged(&scratch)), expected("first"));
            go.send(()).unwrap();
            first.join().unwrap().unwrap();
        });
        assert_eq!(tree(&live), expected("first"));
        assert!(only_live(&scratch));
        // the lock goes with the first run
        publish(&live, "second", &mut Disk).unwrap();
        assert_eq!(tree(&live), expected("second"));
        assert!(only_live(&scratch));
    }
}
