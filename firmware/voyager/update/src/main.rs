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

use nix::fcntl::{AT_FDCWD, RenameFlags, renameat2};
use serde_json::{Value, json};

/// The vendored revision: FILES and PIN, nothing else.
const DIR: &str = "firmware/voyager/oryx";
const PIN: &str = "oryx.json";
const FILES: [&str; 4] = ["keymap.c", "config.h", "rules.mk", "keymap.json"];
/// The ZSA QMK branch (firmwareNN) that firmware/voyager/default.nix builds.
const FIRMWARE_MAJOR: &str = "25";
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

/// The raw name of every record in the zip's central directory, read here
/// rather than through ZipArchive: that keeps one entry per name, so of two
/// records with the same name only the last would ever be seen. Zip64 (more
/// than 65,534 entries or 4 GiB) is refused rather than half read.
fn entry_names(zip: &[u8]) -> Result<Vec<&[u8]>, String> {
    const END: &[u8] = b"PK\x05\x06";
    const RECORD: &[u8] = b"PK\x01\x02";
    let corrupt = |why: &str| format!("the export is not a zip: {why}");
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
    let (on_disk, entries, size, offset) =
        (field(8, 2)?, field(10, 2)?, field(12, 4)?, field(16, 4)?);
    if entries == 0xFFFF || size == 0xFFFF_FFFF || offset == 0xFFFF_FFFF {
        return Err("the export is a zip64 archive, which this updater does not read".to_owned());
    }
    if on_disk != entries {
        return Err(corrupt("a multi-disk archive"));
    }
    let mut names = Vec::with_capacity(entries);
    let mut at = offset;
    for _ in 0..entries {
        let field = |offset, width| {
            le(zip, at + offset, width).ok_or_else(|| corrupt("truncated central directory"))
        };
        if !zip[at.min(zip.len())..].starts_with(RECORD) {
            return Err(corrupt("a central directory record is missing"));
        }
        let (name, extra, comment) = (field(28, 2)?, field(30, 2)?, field(32, 2)?);
        names.push(
            zip.get(at + 46..at + 46 + name)
                .ok_or_else(|| corrupt("truncated central directory"))?,
        );
        at += 46 + name + extra + comment;
    }
    if at != offset + size {
        return Err(corrupt("the central directory is not the size it claims"));
    }
    Ok(names)
}

/// The four layout files of an Oryx export, in FILES order. They must be
/// immediate children of the export's one top-level `*_source/` directory.
/// Anything else that could be a source tree (a second source directory, one
/// below the top level, a nested path inside it, a source file named twice,
/// whether as exactly the same name or one spelt differently) is an error
/// rather than a guess at which copy is meant, as is any entry name that
/// appears twice in the archive.
fn source_files(zip: &[u8]) -> Result<Vec<(&'static str, Vec<u8>)>, String> {
    let names = entry_names(zip)?;
    let mut seen = HashSet::new();
    if let Some(twice) = names.iter().find(|name| !seen.insert(**name)) {
        return Err(format!(
            "the export has two entries named {}",
            String::from_utf8_lossy(twice)
        ));
    }
    let mut archive = zip::ZipArchive::new(Cursor::new(zip))
        .map_err(|error| format!("the export is not a zip: {error}"))?;
    if archive.len() != names.len() {
        return Err(format!(
            "the export's central directory has {} entries, but its zip reader sees {}",
            names.len(),
            archive.len()
        ));
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
    let mut response = ureq::post("https://oryx.zsa.io/graphql")
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
    let mut response = ureq::get(&url)
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
    println!("next: nix build .#voyager-firmware && nix run nixpkgs#zapp -- flash result/*.bin");
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

    #[test]
    fn two_entries_with_exactly_the_same_name_are_an_error() {
        // ZipWriter refuses a name twice, so write another name of the same
        // length and rename it in the bytes, local header and central
        // directory alike
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
        // the zip crate reads it, and sees one keymap.c of the two
        let archive = zip::ZipArchive::new(Cursor::new(&zip[..])).unwrap();
        assert_eq!(archive.len(), 4);
        assert_eq!(entry_names(&zip).unwrap().len(), 5);
        assert!(
            source_files(&zip)
                .unwrap_err()
                .contains("two entries named x_source/keymap.c")
        );
    }

    #[test]
    fn a_zip64_export_is_refused() {
        let mut zip = export_with(&[]);
        let end = zip.len() - 22;
        assert_eq!(&zip[end..end + 4], b"PK\x05\x06");
        zip[end + 8..end + 12].copy_from_slice(&[0xFF; 4]);
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
        let (go, wait) = mpsc::channel();
        thread::scope(|threads| {
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
