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
//! directory, synced, and exchanged with the live one in a single rename, so a
//! failure leaves the old revision, never a mix.

use std::fs::{self, File};
use std::io::{self, Cursor, Read, Write};
use std::path::Path;
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

/// The four layout files of an Oryx export, in FILES order. They must be
/// immediate children of the export's one top-level `*_source/` directory.
/// Anything else that could be a source tree (a second source directory, one
/// below the top level, a nested path inside it, a file named twice) is an
/// error rather than a guess at which copy is meant.
fn source_files(zip: &[u8]) -> Result<Vec<(&'static str, Vec<u8>)>, String> {
    let mut archive = zip::ZipArchive::new(Cursor::new(zip))
        .map_err(|error| format!("the export is not a zip: {error}"))?;
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

/// The steps of publishing a revision that can fail midway: a seam, so the
/// tests can make each one fail.
trait Publish {
    /// Creates `path` with `content` and syncs it to disk.
    fn write(&mut self, path: &Path, content: &[u8]) -> io::Result<()>;
    /// Swaps two directories in one step.
    fn exchange(&mut self, staged: &Path, live: &Path) -> io::Result<()>;
}

struct Disk;

impl Publish for Disk {
    fn write(&mut self, path: &Path, content: &[u8]) -> io::Result<()> {
        let mut file = File::create_new(path)?;
        file.write_all(content)?;
        file.sync_all()
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
}

fn sync_dir(dir: &Path) -> Result<(), String> {
    File::open(dir)
        .and_then(|dir| dir.sync_all())
        .map_err(|error| format!("{}: {error}", dir.display()))
}

/// Writes `files` into the new directory `staged`, syncs it and exchanges it
/// with `live`.
fn stage_and_exchange(
    staged: &Path,
    live: &Path,
    files: &[(&str, Vec<u8>)],
    disk: &mut impl Publish,
) -> Result<(), String> {
    fs::create_dir(staged).map_err(|error| format!("{}: {error}", staged.display()))?;
    for (name, content) in files {
        let path = staged.join(name);
        disk.write(&path, content)
            .map_err(|error| format!("{}: {error}", path.display()))?;
    }
    sync_dir(staged)?;
    disk.exchange(staged, live).map_err(|error| {
        format!(
            "exchanging {} with {}: {error}",
            staged.display(),
            live.display()
        )
    })
}

/// Replaces the directory `live` with one holding exactly `files`, at once.
/// The new tree is staged next to it (same filesystem), every file and the
/// directory synced, then exchanged with `live` by renameat2(RENAME_EXCHANGE)
/// and the old tree removed. A failure before the exchange leaves `live` as it
/// was; from the exchange on, `live` is wholly new.
fn publish(live: &Path, files: &[(&str, Vec<u8>)], disk: &mut impl Publish) -> Result<(), String> {
    let name = live
        .file_name()
        .ok_or_else(|| format!("{} has no directory name", live.display()))?;
    let staged = live.with_file_name(format!(".{}.staged", name.to_string_lossy()));
    let parent = staged
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    // only ever left by an interrupted run, and never live
    match fs::remove_dir_all(&staged) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => {
            return Err(format!("{}: {error}", staged.display()));
        }
        _ => {}
    }
    if let Err(error) = stage_and_exchange(&staged, live, files, disk) {
        // best effort: what is left there is the unpublished new tree
        let _ = fs::remove_dir_all(&staged);
        return Err(format!("{error}; {} is unchanged", live.display()));
    }
    // `staged` now holds the old tree
    sync_dir(parent)?;
    fs::remove_dir_all(&staged).map_err(|error| {
        format!(
            "{} is updated, but its old tree is left at {}: {error}",
            live.display(),
            staged.display()
        )
    })
}

fn run() -> Result<(), String> {
    let live = Path::new(DIR);
    let pin_path = live.join(PIN);
    let pin_text = fs::read_to_string(&pin_path)
        .map_err(|_| "run me from the root of the nixos checkout".to_owned())?;
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
    publish(live, &files, &mut Disk)?;
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
    use std::path::PathBuf;

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

    /// The scratch directory holds the live tree and nothing else: no staged
    /// or old tree is left behind.
    fn only_live(scratch: &Scratch) -> bool {
        tree_names(&scratch.0) == ["oryx"]
    }

    fn tree_names(dir: &Path) -> Vec<String> {
        fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect()
    }

    /// Disk, except that step number `fail` (writes and the exchange, counted
    /// from 1) fails; a failing write first writes half its content, as a
    /// short write or a full disk would.
    struct FailAt {
        fail: usize,
        step: usize,
    }

    impl FailAt {
        fn fails_now(&mut self) -> bool {
            self.step += 1;
            self.step == self.fail
        }
    }

    impl Publish for FailAt {
        fn write(&mut self, path: &Path, content: &[u8]) -> io::Result<()> {
            if self.fails_now() {
                fs::write(path, &content[..content.len() / 2])?;
                return Err(io::Error::other("injected write failure"));
            }
            Disk.write(path, content)
        }

        fn exchange(&mut self, staged: &Path, live: &Path) -> io::Result<()> {
            if self.fails_now() {
                return Err(io::Error::other("injected exchange failure"));
            }
            Disk.exchange(staged, live)
        }
    }

    #[test]
    fn publishing_replaces_the_whole_tree() {
        let (scratch, live) = live_old("success");
        publish(&live, &revision("new"), &mut Disk).unwrap();
        assert_eq!(tree(&live), expected("new"));
        assert!(only_live(&scratch));
    }

    #[test]
    fn a_failure_at_any_step_before_the_exchange_leaves_the_old_tree() {
        // five writes, then the exchange
        for fail in 1..=FILES.len() + 2 {
            let (scratch, live) = live_old(&format!("fail-{fail}"));
            let error =
                publish(&live, &revision("new"), &mut FailAt { fail, step: 0 }).unwrap_err();
            assert!(error.contains("injected"), "step {fail}: {error}");
            assert!(error.ends_with("is unchanged"), "step {fail}: {error}");
            assert_eq!(tree(&live), expected("old"), "step {fail}");
            assert!(only_live(&scratch), "step {fail}");
        }
    }

    #[test]
    fn a_staged_tree_left_by_an_interrupted_run_is_replaced() {
        let (scratch, live) = live_old("stale");
        let stale = scratch.0.join(".oryx.staged");
        fs::create_dir(&stale).unwrap();
        fs::write(stale.join("keymap.c"), "half a keymap").unwrap();
        publish(&live, &revision("new"), &mut Disk).unwrap();
        assert_eq!(tree(&live), expected("new"));
        assert!(only_live(&scratch));
    }
}
