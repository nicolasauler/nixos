mod layout;
mod profile;
mod source;

use std::env;
use std::io::Write as _;
use std::path::PathBuf;
use std::process::{Command, ExitCode, Stdio};

const ROFI: &str = env!("ROFI_BIN");
const WRAP: usize = 88;

// Untrusted layout labels must remain text, not Pango or dmenu row metadata.
fn escape(out: &mut String, text: &str) {
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            c if c.is_control() => out.push(' '),
            c => out.push(c),
        }
    }
}

// Rofi list entries ellipsize rather than soft-wrap. Insert our own line breaks,
// leaving each menu entry on one physical input line. Long words also wrap.
fn wrapped(out: &mut String, text: &str) -> usize {
    let mut width = 0;
    let mut lines = 1;
    for word in text.split_whitespace() {
        let length = word.chars().count();
        if width > 0 {
            if width + 1 + length > WRAP {
                out.push_str("&#10;");
                lines += 1;
                width = 0;
            } else {
                out.push(' ');
                width += 1;
            }
        }
        for c in word.chars() {
            if width == WRAP {
                out.push_str("&#10;");
                lines += 1;
                width = 0;
            }
            let mut encoded = [0; 4];
            escape(out, c.encode_utf8(&mut encoded));
            width += 1;
        }
    }
    lines
}

fn render(entries: &[layout::Entry]) -> (String, usize) {
    let mut text = String::new();
    let mut height = 1;
    for entry in entries {
        text.push_str("<b>");
        let mut lines = wrapped(&mut text, &entry.name);
        text.push_str("</b>&#10;");
        lines += wrapped(&mut text, &entry.gesture);
        text.push_str("&#10;Layer: ");
        lines += wrapped(&mut text, &entry.layer);
        text.push('\t');
        for alias in &entry.aliases {
            escape(&mut text, alias);
            text.push(' ');
        }
        text.push('\n');
        height = height.max(lines);
    }
    (text, height)
}

fn rofi() -> Result<Command, String> {
    let runtime = env::var_os("XDG_RUNTIME_DIR")
        .filter(|value| !value.is_empty())
        .ok_or("XDG_RUNTIME_DIR is not set; open the reference in the graphical session")?;
    let mut command = Command::new(ROFI);
    // drun may still own its default lock while launching this program.
    command
        .arg("-pid")
        .arg(PathBuf::from(runtime).join("voyager-help-rofi.pid"));
    Ok(command)
}

fn run() -> Result<(), String> {
    let layout = source::load()?;
    let entries = layout::generate(&layout)?;
    if entries.is_empty() {
        return Err("The reported revision contains no describable keyboard actions".into());
    }
    let (rows, height) = render(&entries);
    let mut message = String::new();
    escape(
        &mut message,
        &format!(
            "USB-reported Oryx revision {}/{} at launch: {}. ",
            layout.layout, layout.revision, layout.title
        ),
    );
    if let Some(profile) = &layout.profile {
        escape(
            &mut message,
            &format!(
                "Recognized custom Swap Hands build (mirror layer {}). Normal and mirrored routes are separate; pedal-combo routes require the matching daemon. ",
                profile.swap_layer
            ),
        );
    } else {
        message
            .push_str("Stock Oryx interpretation; unmarked custom patches cannot be identified. ");
    }
    message.push_str("Generated from the exact reported revision, not Oryx latest. Start from base with no held keys or latches; symbols assume us / altgr-intl. Escape closes; Enter closes a selected result, without executing it.");
    let visible_rows = (20 / height).clamp(1, 4);
    let mut child = rofi()?
        .args([
            "-dmenu",
            "-i",
            "-no-case-smart",
            "-matching-negate-char",
            "\\0",
            "-no-auto-select",
            "-no-custom",
            "-markup-rows",
            "-display-columns",
            "1",
            "-display-column-separator",
            "\\t",
            "-format",
            "",
            "-p",
            "Voyager Reference",
            "-mesg",
            &message,
            "-eh",
            &height.to_string(),
            "-no-show-icons",
            "-theme-str",
            &format!("window {{ width: 96%; }} listview {{ lines: {visible_rows}; columns: 1; fixed-height: false; }}"),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .map_err(|error| format!("Cannot start Rofi: {error}"))?;
    let write_result = child
        .stdin
        .take()
        .expect("piped Rofi stdin")
        .write_all(rows.as_bytes());
    let status = child
        .wait()
        .map_err(|error| format!("Waiting for Rofi: {error}"))?;
    // Escape is normal cancellation, including before the input was fully read.
    if status.code() == Some(1) {
        return Ok(());
    }
    write_result.map_err(|error| format!("Sending the reference to Rofi: {error}"))?;
    if !status.success() {
        return Err(format!("Rofi exited with {status}"));
    }
    Ok(())
}

fn diagnostic(error: &str) -> String {
    let mut message = String::with_capacity(error.len() + 128);
    message.push_str("Voyager reference unavailable\n");
    for c in error.chars() {
        if c == '\0' {
            message.push_str("\\0");
        } else {
            message.push(c);
        }
    }
    message.push_str("\nNo mapping from another revision was substituted.");
    message
}

fn main() -> ExitCode {
    if env::args_os().len() != 1 {
        eprintln!("usage: voyager-help");
        return ExitCode::from(2);
    }
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("voyager-help: {error}");
            let message = diagnostic(&error);
            if let Err(display_error) = rofi().and_then(|mut command| {
                command
                    .args(["-e", &message])
                    .stdout(Stdio::null())
                    .status()
                    .map_err(|error| format!("Cannot show Rofi error: {error}"))
            }) {
                eprintln!("{display_error}");
            }
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostic_text_is_valid_argv_without_reinterpreting_markup() {
        let message = diagnostic("returned id Other\0; literal <b>&amp;</b>");
        assert!(!message.as_bytes().contains(&0));
        assert!(message.contains("Other\\0"));
        assert!(message.contains("<b>&amp;</b>"));
    }

    #[test]
    fn source_controls_and_markup_cannot_create_menu_structure() {
        let malicious = "<b>fake</b>&amp;\0display\u{1f}payload\nextra\tcolumn";
        let entries = [layout::Entry {
            name: malicious.into(),
            gesture: "x".repeat(WRAP * 2 + 1),
            layer: malicious.into(),
            aliases: vec![malicious.into()],
        }];
        let (rows, height) = render(&entries);
        assert_eq!(rows.lines().count(), 1);
        assert_eq!(rows.bytes().filter(|&byte| byte == b'\t').count(), 1);
        assert!(
            !rows
                .chars()
                .any(|c| c == '\0' || c == '\u{1f}' || c == '\r')
        );
        assert!(!rows.contains("<b>fake</b>"));
        assert!(rows.contains("&lt;b&gt;fake&lt;/b&gt;&amp;amp;"));
        let wrapped_word = format!("{}&#10;{}&#10;x", "x".repeat(WRAP), "x".repeat(WRAP));
        assert!(rows.contains(&wrapped_word));
        assert!(height >= 5);
    }
}
