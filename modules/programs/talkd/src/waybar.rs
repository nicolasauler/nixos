//! An event-driven, read-only Waybar feed. Reuses the daemon's Route/Props
//! selection, but never constructs its state machine, store or input reader.

use std::cell::RefCell;
use std::io::{self, Write};
use std::process::ExitCode;
use std::rc::Rc;
use std::thread;

use pipewire as pw;
use pw::loop_::Timeout;

use crate::mic::{Mic, MicEvent, Status, Update};

fn publish(out: &mut impl Write, status: Option<&Status>) -> io::Result<()> {
    let source = status.and_then(|status| status.default.as_ref().ok());
    let (text, class) = match source.and_then(|source| source.mute) {
        Some((_, true)) => ("MIC MUTED", "muted"),
        Some((_, false)) => ("MIC LIVE", "live"),
        None => ("MIC ?", "unknown"),
    };
    let mut tooltip = match source {
        Some(source) => format!(
            "Default input: {}\n{}",
            source.description.as_deref().unwrap_or(&source.name),
            if source.mute.is_some() {
                ""
            } else {
                "Mute state unavailable.\n"
            }
        ),
        None => match status.map(|status| &status.default) {
            Some(Err(why)) => format!("Default input unavailable: {why}\n"),
            _ => "Audio unavailable or synchronizing.\n".to_owned(),
        },
    };
    tooltip.push_str("Unmuted does not mean an application is recording.\nWhen talkd's keyboard controls are configured:\nF14 sets the baseline; F13 temporarily inverts it.\nAn active F13 cycle stays on its original input if the default changes.");
    // Static state names need no escaping; serialize the untrusted device text.
    // Waybar's escape=true additionally treats it as text rather than Pango.
    write!(
        out,
        "{{\"text\":\"{text}\",\"class\":\"{class}\",\"tooltip\":"
    )?;
    serde_json::to_writer(&mut *out, &tooltip)?;
    out.write_all(b"}\n")?;
    out.flush()
}

struct Reader {
    mic: Option<Mic>,
    lost: bool,
}

fn watch(out: &mut impl Write) -> Result<(), Box<dyn std::error::Error>> {
    // No optimistic live/muted default while the connection is being opened.
    publish(out, None)?;
    pw::init();
    let main_loop = pw::main_loop::MainLoopRc::new(None)?;
    let context = pw::context::ContextRc::new(&main_loop, None)?;
    let reader = Rc::new(RefCell::new(Reader {
        mic: None,
        lost: true,
    }));
    let hook: Rc<dyn Fn(Update)> = {
        let weak = Rc::downgrade(&reader);
        Rc::new(move |update| {
            if let Some(reader) = weak.upgrade() {
                let mut reader = reader.borrow_mut();
                if let Some(mic) = reader.mic.as_mut()
                    && mic
                        .apply(update)
                        .iter()
                        .any(|event| matches!(event, MicEvent::Lost(_)))
                {
                    reader.lost = true;
                }
            }
        })
    };
    reader.borrow_mut().mic = Some(Mic::new(context, None, hook));
    let mut previous = None;
    loop {
        if reader.borrow().lost {
            let connected = reader.borrow_mut().mic.as_mut().unwrap().connect().is_ok();
            if !connected {
                // Retry only a missing connection; healthy observations use events.
                thread::sleep(crate::RETRY);
                continue;
            }
            reader.borrow_mut().lost = false;
        }
        main_loop.loop_().iterate(Timeout::Infinite);
        let current = reader.borrow().mic.as_ref().and_then(Mic::status);
        if current != previous {
            publish(out, current.as_ref())?;
            previous = current;
        }
        if reader.borrow().lost {
            // Listeners must be dropped outside their callbacks. A lost link is
            // already unknown above; it can never keep the last muted label.
            reader.borrow_mut().mic.as_mut().unwrap().disconnect();
            thread::sleep(crate::RETRY);
        }
    }
}

pub fn run() -> ExitCode {
    let stdout = io::stdout();
    let mut out = io::BufWriter::new(stdout.lock());
    match watch(&mut out) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error)
            if error
                .downcast_ref::<io::Error>()
                .is_some_and(|error| error.kind() == io::ErrorKind::BrokenPipe) =>
        {
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("talkd waybar: {error}");
            ExitCode::FAILURE
        }
    }
}
