//! talkd: push-to-talk on the Voyager's F13.
//!
//! Hold F13 to invert the default source's mute (a muted mic talks, a live
//! one goes quiet), let go to put it back; a tap under 250 ms latches the
//! flip until the next press. One process, one ordered state machine
//! ([`machine`]): key events come straight from the keyboard's evdev node(s)
//! ([`input`]), the session's lock state from Hyprland ([`lock`]), and the
//! mic from one persistent PipeWire connection ([`mic`]), all handled on
//! PipeWire's main loop, one at a time. What a cycle owes is saved before
//! the mic is touched ([`store`]) and restored at the next start if a crash
//! left it owed.

mod input;
mod lock;
mod machine;
mod mic;
mod notices;
mod params;
mod status;
mod store;

#[cfg(test)]
#[path = "../tests/support/mod.rs"]
mod test_support;

use std::cell::RefCell;
use std::env;
use std::path::PathBuf;
use std::process::{Command, ExitCode, Stdio};
use std::rc::{Rc, Weak};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use nix::sys::signal::{SigSet, Signal};
use pipewire as pw;

use machine::{Event, Lock, Machine, Obligation, Pin, Problem, World};
use mic::{Mic, MicEvent, Update};
use notices::Notices;
use store::Store;

/// How long a stop waits for an owed restore to settle.
const STOP: Duration = Duration::from_secs(3);

/// Between attempts to reach PipeWire.
const RETRY: Duration = Duration::from_secs(2);

/// How long notify-send may take.
const NOTIFY: Duration = Duration::from_secs(5);

/// What the other threads hand the main loop.
enum Msg {
    Key(input::Key),
    Wake(Wake),
    /// notify-send's outcome for a message: delivered or not.
    Sent(String, bool),
}

#[derive(Clone, Copy, Debug)]
enum Wake {
    Deadline(u64),
    Reconnect,
    GiveUp,
}

struct Daemon {
    machine: Machine,
    mic: Mic,
    store: Store,
    notices: Notices,
    timers: mpsc::Sender<(Wake, Instant)>,
    notifier: mpsc::Sender<String>,
    main_loop: pw::main_loop::MainLoopWeak,
    stopping: bool,
    /// Reaching PipeWire failed, and that was logged.
    unreachable: bool,
    lock_error: Option<String>,
}

/// The machine's view of the daemon.
struct Effects<'a> {
    mic: &'a mut Mic,
    store: &'a Store,
    notices: &'a mut Notices,
    timers: &'a mpsc::Sender<(Wake, Instant)>,
    notifier: &'a mpsc::Sender<String>,
    lock_error: &'a mut Option<String>,
}

impl Effects<'_> {
    fn report(&mut self, problem: &Problem) {
        let message = problem.message();
        eprintln!("talkd: {message}");
        if problem.visible()
            && let Some(message) = self.notices.raise(&message)
        {
            let _ = self.notifier.send(message);
        }
    }
}

impl World for Effects<'_> {
    fn default_source(&self) -> Option<(Pin, bool)> {
        self.mic.default_source()
    }

    fn lock_state(&mut self) -> Lock {
        match lock::now() {
            Ok(state) => {
                *self.lock_error = None;
                state
            }
            Err(error) => {
                if self.lock_error.as_ref() != Some(&error) {
                    eprintln!("lock: {error}; new cycles are disabled");
                    *self.lock_error = Some(error);
                }
                Lock::Unknown
            }
        }
    }

    fn exists(&self, pin: Pin) -> bool {
        self.mic.exists(pin)
    }

    fn muted(&self, pin: Pin) -> Option<bool> {
        self.mic.muted(pin)
    }

    fn set_mute(&mut self, pin: Pin, muted: bool, token: u64) -> Result<(), String> {
        self.mic.set_mute(pin, muted, token)
    }

    fn save(&mut self, obligation: &Obligation) -> Result<(), String> {
        self.store.save(obligation)
    }

    fn clear(&mut self) -> Result<(), String> {
        self.store.clear()
    }

    fn arm(&mut self, token: u64, after: Duration) {
        let _ = self
            .timers
            .send((Wake::Deadline(token), Instant::now() + after));
    }

    fn problem(&mut self, problem: Problem) {
        self.report(&problem);
    }

    fn recovered(&mut self) {
        self.notices.recover();
    }
}

impl Daemon {
    /// The machine, and the rest as its world.
    fn split(&mut self) -> (&mut Machine, Effects<'_>) {
        let effects = Effects {
            mic: &mut self.mic,
            store: &self.store,
            notices: &mut self.notices,
            timers: &self.timers,
            notifier: &self.notifier,
            lock_error: &mut self.lock_error,
        };
        (&mut self.machine, effects)
    }

    fn feed(&mut self, event: Event) {
        let (machine, mut effects) = self.split();
        machine.handle(event, &mut effects);
        if self.stopping && self.machine.settled() {
            self.quit();
        }
    }

    fn after(&self, wake: Wake, delay: Duration) {
        let _ = self.timers.send((wake, Instant::now() + delay));
    }

    fn quit(&self) {
        if let Some(main_loop) = self.main_loop.upgrade() {
            main_loop.quit();
        }
    }

    fn update(&mut self, update: Update) {
        for event in self.mic.apply(update) {
            match event {
                MicEvent::Ready(server) => {
                    eprintln!("mic: connected to PipeWire (server {server:08x})");
                    self.unreachable = false;
                    self.feed(Event::ServerUp(server));
                }
                MicEvent::Confirmed(token, pin, muted) => {
                    self.feed(Event::Confirmed(token, pin, muted))
                }
                MicEvent::Gone(pin) => self.feed(Event::NodeGone(pin)),
                MicEvent::Lost(why) => {
                    eprintln!("mic: {why}; reconnecting");
                    self.feed(Event::ServerGone);
                    // not from inside the connection's own callback
                    self.after(Wake::Reconnect, Duration::ZERO);
                }
            }
        }
    }

    fn reconnect(&mut self) {
        self.mic.disconnect();
        if let Err(error) = self.mic.connect() {
            if !self.unreachable {
                eprintln!("mic: {error}; retrying every {}s", RETRY.as_secs());
            }
            self.unreachable = true;
            self.after(Wake::Reconnect, RETRY);
        }
    }

    fn message(&mut self, message: Msg) {
        match message {
            Msg::Key(input::Key::Press(at)) => self.feed(Event::Press(at)),
            Msg::Key(input::Key::Release(at)) => self.feed(Event::Release(at)),
            Msg::Key(input::Key::Lost) => self.feed(Event::DeviceLost),
            Msg::Wake(Wake::Deadline(token)) => self.feed(Event::Deadline(token)),
            Msg::Wake(Wake::Reconnect) => self.reconnect(),
            Msg::Wake(Wake::GiveUp) => {
                eprintln!(
                    "talkd: stopping with a restore still unsettled; the next start retries it"
                );
                self.quit();
            }
            Msg::Sent(message, delivered) => self.notices.sent(&message, delivered),
        }
    }

    fn stop(&mut self) {
        if self.stopping {
            return;
        }
        eprintln!("talkd: stopping");
        self.stopping = true;
        self.after(Wake::GiveUp, STOP);
        self.feed(Event::Stop);
    }
}

/// Delivers [`Wake`]s when they are due.
fn timers(requests: mpsc::Receiver<(Wake, Instant)>, send: pw::channel::Sender<Msg>) {
    let mut queue: Vec<(Instant, Wake)> = Vec::new();
    loop {
        queue.sort_by_key(|&(at, _)| at);
        let now = Instant::now();
        let due = queue.iter().take_while(|&&(at, _)| at <= now).count();
        for (_, wake) in queue.drain(..due) {
            if send.send(Msg::Wake(wake)).is_err() {
                return;
            }
        }
        let request = match queue.first() {
            Some(&(at, _)) => requests.recv_timeout(at.saturating_duration_since(now)),
            None => requests.recv().map_err(|_| RecvTimeoutError::Disconnected),
        };
        match request {
            Ok((wake, at)) => queue.push((at, wake)),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

/// Shows `message` through the notification daemon (mako); true only if
/// notify-send says it was delivered.
fn notify(message: &str) -> bool {
    let child = Command::new("notify-send")
        .args(["--app-name=talkd", "--urgency=critical", "talkd", message])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .spawn();
    let mut child = match child {
        Ok(child) => child,
        Err(error) => {
            eprintln!("notify-send: {error}");
            return false;
        }
    };
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) if started.elapsed() < NOTIFY => thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                eprintln!("notify-send: no answer in {}s", NOTIFY.as_secs());
                return false;
            }
        }
    }
}

fn notifier(messages: mpsc::Receiver<String>, send: pw::channel::Sender<Msg>) {
    for message in messages {
        let delivered = notify(&message);
        if send.send(Msg::Sent(message, delivered)).is_err() {
            return;
        }
    }
}

fn main() -> ExitCode {
    let mut args = env::args().skip(1);
    match (args.next().as_deref(), args.next()) {
        (None, _) => {}
        (Some("status"), None) => return status::run(),
        _ => {
            eprintln!("usage: talkd [status]");
            return ExitCode::from(2);
        }
    }
    // the main loop takes SIGTERM and SIGINT through a signalfd; block them
    // before any thread exists so none of the others gets them
    let mut signals = SigSet::empty();
    signals.add(Signal::SIGTERM);
    signals.add(Signal::SIGINT);
    if let Err(error) = signals.thread_block() {
        eprintln!("talkd: blocking signals: {error}");
        return ExitCode::FAILURE;
    }

    let Some(runtime) = env::var_os("XDG_RUNTIME_DIR") else {
        eprintln!("talkd: XDG_RUNTIME_DIR is not set");
        return ExitCode::FAILURE;
    };
    let store = match Store::new(PathBuf::from(runtime).join("talkd")) {
        Ok(store) => store,
        Err(error) => {
            eprintln!("talkd: {error}");
            return ExitCode::FAILURE;
        }
    };
    let (saved, unreadable) = match store.load() {
        Ok(saved) => (saved, None),
        Err(error) => (None, Some(error)),
    };
    if let Some(owed) = saved {
        eprintln!(
            "talkd: a previous run owes a restore: node {} of PipeWire server {:08x}, back to {}",
            owed.pin.serial,
            owed.pin.server,
            if owed.was_muted { "muted" } else { "live" }
        );
    }

    pw::init();
    let main_loop = match pw::main_loop::MainLoopRc::new(None) {
        Ok(main_loop) => main_loop,
        Err(error) => {
            eprintln!("talkd: PipeWire main loop: {error}");
            return ExitCode::FAILURE;
        }
    };
    let context = match pw::context::ContextRc::new(&main_loop, None) {
        Ok(context) => context,
        Err(error) => {
            eprintln!("talkd: PipeWire context: {error}");
            return ExitCode::FAILURE;
        }
    };
    let (sender, receiver) = pw::channel::channel::<Msg>();
    let (timer_requests, timer_queue) = mpsc::channel();
    let (notes, note_queue) = mpsc::channel();
    {
        let send = sender.clone();
        thread::spawn(move || timers(timer_queue, send));
        let send = sender.clone();
        thread::spawn(move || notifier(note_queue, send));
        let send = sender.clone();
        thread::spawn(move || {
            input::run(move |key| {
                let _ = send.send(Msg::Key(key));
            })
        });
    }

    let daemon = Rc::new_cyclic(|weak: &Weak<RefCell<Daemon>>| {
        let weak = weak.clone();
        let hook: Rc<dyn Fn(Update)> = Rc::new(move |update| {
            if let Some(daemon) = weak.upgrade() {
                daemon.borrow_mut().update(update);
            }
        });
        RefCell::new(Daemon {
            machine: Machine::new(saved),
            mic: Mic::new(context.clone(), None, hook),
            store,
            notices: Notices::default(),
            timers: timer_requests,
            notifier: notes,
            main_loop: main_loop.downgrade(),
            stopping: false,
            unreachable: false,
            lock_error: None,
        })
    });
    {
        let mut daemon = daemon.borrow_mut();
        if let Some(error) = unreadable {
            eprintln!("talkd: {error}");
            let (_, mut effects) = daemon.split();
            if let Err(error) = effects.clear() {
                effects.problem(Problem::Store(error));
            }
            effects.problem(Problem::Stale("its record is unreadable"));
        }
        daemon.reconnect();
    }

    let _receiver = receiver.attach(main_loop.loop_(), {
        let daemon = daemon.clone();
        move |message| daemon.borrow_mut().message(message)
    });
    let stop = |daemon: &Rc<RefCell<Daemon>>| {
        let daemon = Rc::downgrade(daemon);
        move || {
            if let Some(daemon) = daemon.upgrade() {
                daemon.borrow_mut().stop();
            }
        }
    };
    let _term = main_loop
        .loop_()
        .add_signal_local(pw::loop_::Signal::TERM, stop(&daemon));
    let _int = main_loop
        .loop_()
        .add_signal_local(pw::loop_::Signal::INT, stop(&daemon));
    eprintln!("talkd: running");
    main_loop.run();
    ExitCode::SUCCESS
}
