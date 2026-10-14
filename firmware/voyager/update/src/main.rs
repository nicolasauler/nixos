//! Pulls the latest saved revision of the Oryx layout pinned in
//! firmware/voyager/oryx.json into firmware/voyager/oryx/. Run from the root
//! of the nixos checkout: `nix run .#voyager-layout-update`.
//!
//! Oryx's public GraphQL API names the latest revision; its source export (a
//! zip with `<layout>_source/{keymap.c,config.h,rules.mk,keymap.json}`, a
//! build log and a compiled .bin) is fetched from /source/<revision>. This is
//! what ZSA's own "Oryx with custom QMK" tooling does.

use std::fs;
use std::io::{Cursor, Read};
use std::path::Path;
use std::process::ExitCode;

use serde_json::{Value, json};

const DIR: &str = "firmware/voyager";
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

/// The four layout files of an Oryx export, in FILES order.
fn source_files(zip: &[u8]) -> Result<Vec<(&'static str, Vec<u8>)>, String> {
    let mut archive = zip::ZipArchive::new(Cursor::new(zip))
        .map_err(|error| format!("the export is not a zip: {error}"))?;
    let mut found: Vec<(&'static str, Vec<u8>)> = Vec::new();
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|error| error.to_string())?;
        let path = entry
            .name()
            .map_err(|error| error.to_string())?
            .into_owned();
        let Some(name) = FILES
            .iter()
            .find(|&&name| path.contains("_source/") && path.ends_with(&format!("/{name}")))
        else {
            continue;
        };
        let mut content = Vec::new();
        entry
            .read_to_end(&mut content)
            .map_err(|error| format!("{path}: {error}"))?;
        found.push((name, content));
    }
    FILES
        .iter()
        .map(|&name| {
            found
                .iter()
                .position(|(found, _)| *found == name)
                .map(|index| found.swap_remove(index))
                .ok_or_else(|| format!("{name} is missing from the export"))
        })
        .collect()
}

fn run() -> Result<(), String> {
    let pin_path = Path::new(DIR).join("oryx.json");
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
    let files = source_files(&zip)?;

    let oryx = Path::new(DIR).join("oryx");
    for (name, content) in &files {
        fs::write(oryx.join(name), content)
            .map_err(|error| format!("{}: {error}", oryx.join(name).display()))?;
    }
    let pin = json!({
        "layout": layout,
        "revision": latest.hash_id,
        "firmware": latest.firmware,
        "title": latest.title,
    });
    let pin_text = serde_json::to_string_pretty(&pin).map_err(|error| error.to_string())? + "\n";
    fs::write(&pin_path, pin_text).map_err(|error| format!("{}: {error}", pin_path.display()))?;
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
    use std::io::Write;

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

    #[test]
    fn takes_the_four_source_files_and_nothing_else() {
        let zip = export(&[
            ("build.log", "log"),
            ("zsa_voyager_x_source/keymap.json", "{}"),
            ("zsa_voyager_x_source/keymap.c", "keys"),
            ("zsa_voyager_x.bin", "firmware"),
            ("zsa_voyager_x_source/rules.mk", "rules"),
            ("zsa_voyager_x_source/config.h", "config"),
        ]);
        let files = source_files(&zip).unwrap();
        let names: Vec<&str> = files.iter().map(|(name, _)| *name).collect();
        assert_eq!(names, FILES);
        assert_eq!(files[0].1, b"keys");
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
}
