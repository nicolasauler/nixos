//! Whether the session is locked, from Hyprland's hyprland-lock-notify-v1
//! (hyprland-protocols): "locked" once the ext-session-lock client
//! (hyprlock) has presented a lock frame on every output (Hyprland
//! src/managers/SessionLockManager.cpp, at most 5 s after the lock request),
//! "unlocked" when it unlocks; a lock already in place is reported at once.
//! logind's LockedHint is no use here: nothing in this session sets it.
//! Without the compositor, the state is unknown, which counts as locked.

use std::convert::Infallible;
use std::time::Duration;

use wayland_client::protocol::wl_registry::{self, WlRegistry};
use wayland_client::{Connection, Dispatch, QueueHandle};

use crate::machine::Lock;

#[allow(non_upper_case_globals, non_camel_case_types, dead_code, clippy::all)]
mod protocol {
    use wayland_client;

    pub mod __interfaces {
        wayland_scanner::generate_interfaces!("protocols/hyprland-lock-notify-v1.xml");
    }
    use self::__interfaces::*;

    wayland_scanner::generate_client_code!("protocols/hyprland-lock-notify-v1.xml");
}

use protocol::hyprland_lock_notification_v1::{self, HyprlandLockNotificationV1};
use protocol::hyprland_lock_notifier_v1::HyprlandLockNotifierV1;

#[derive(Default)]
struct State {
    notifier: Option<HyprlandLockNotifierV1>,
    locked: bool,
}

impl Dispatch<WlRegistry, ()> for State {
    fn event(
        state: &mut Self,
        registry: &WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        queue: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name, interface, ..
        } = event
            && interface == "hyprland_lock_notifier_v1"
        {
            state.notifier = Some(registry.bind(name, 1, queue, ()));
        }
    }
}

impl Dispatch<HyprlandLockNotifierV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &HyprlandLockNotifierV1,
        _: <HyprlandLockNotifierV1 as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<HyprlandLockNotificationV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &HyprlandLockNotificationV1,
        event: hyprland_lock_notification_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            hyprland_lock_notification_v1::Event::Locked => state.locked = true,
            hyprland_lock_notification_v1::Event::Unlocked => state.locked = false,
        }
    }
}

fn lock(locked: bool) -> Lock {
    if locked { Lock::Locked } else { Lock::Unlocked }
}

/// Follows the lock state until the connection fails; `up` is set once the
/// first state is known.
fn watch(send: &impl Fn(Lock), up: &mut bool) -> Result<Infallible, String> {
    let connection = Connection::connect_to_env().map_err(|error| error.to_string())?;
    let mut queue = connection.new_event_queue();
    let handle = queue.handle();
    connection.display().get_registry(&handle, ());
    let mut state = State::default();
    queue
        .roundtrip(&mut state)
        .map_err(|error| error.to_string())?;
    let notifier = state
        .notifier
        .take()
        .ok_or("the compositor offers no hyprland_lock_notifier_v1")?;
    let _notification = notifier.get_lock_notification(&handle, ());
    // a lock already in place is announced right away
    queue
        .roundtrip(&mut state)
        .map_err(|error| error.to_string())?;
    let mut sent = state.locked;
    send(lock(sent));
    *up = true;
    loop {
        queue
            .blocking_dispatch(&mut state)
            .map_err(|error| error.to_string())?;
        if state.locked != sent {
            sent = state.locked;
            send(lock(sent));
        }
    }
}

/// Reports the lock state forever, reconnecting every 2 s.
pub fn run(send: impl Fn(Lock)) -> ! {
    let mut failing = false;
    loop {
        let mut up = false;
        let Err(error) = watch(&send, &mut up);
        send(Lock::Unknown);
        // logged once per outage, not every 2 s
        if up || !failing {
            eprintln!("lock: {error}; presses cannot start a cycle until it is back");
        }
        failing = true;
        std::thread::sleep(Duration::from_secs(2));
    }
}
