//! One persistent PipeWire connection. It follows the "default" metadata's
//! `default.audio.source` (as WirePlumber's default-nodes-api does: the
//! node of that `node.name` whose media.class is not a stream), binds every
//! audio source and audio device, and reads each source's mute where
//! WirePlumber's mixer does ([`params::mixer`]): from param events, so a
//! change made by anyone shows up here.
//!
//! The owner feeds [`Update`]s, which the listeners hand to its hook, back
//! through [`Mic::apply`], and gets [`MicEvent`]s out.

use std::collections::HashMap;
use std::rc::Rc;

use pipewire as pw;
use pw::context::ContextRc;
use pw::core::{CoreRc, PW_ID_CORE};
use pw::device::{Device, DeviceChangeMask, DeviceListener};
use pw::metadata::{Metadata, MetadataListener};
use pw::node::{Node, NodeChangeMask, NodeListener};
use pw::permissions::PermissionFlags;
use pw::proxy::{Proxy, ProxyT};
use pw::registry::{GlobalObject, RegistryRc};
use pw::spa::param::{ParamInfo, ParamInfoFlags, ParamType};
use pw::spa::pod::Pod;
use pw::spa::spa_interface_call_method;
use pw::spa::utils::dict::DictRef;
use pw::spa::utils::result::{AsyncSeq, SpaResult, SpaSuccess};
use pw::types::ObjectType;

use crate::machine::Pin;
use crate::params::{self, Route, Via};

const DEFAULT_SOURCE: &str = "default.audio.source";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Source,
    Device,
    DefaultMetadata,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParamObject {
    Node(u32),
    Device(u32),
}

/// What a listener saw. Param replies retain their actual native sequence;
/// the Rust wrappers discard the return value that supplies this sequence.
pub enum Update {
    Info {
        cookie: u32,
    },
    Done(AsyncSeq),
    Error {
        id: u32,
        seq: i32,
        res: i32,
        message: String,
    },
    Global {
        id: u32,
        kind: Kind,
        permissions: PermissionFlags,
        props: HashMap<String, String>,
    },
    GlobalRemove(u32),
    NodeInfo {
        id: u32,
        props: HashMap<String, String>,
    },
    Params {
        object: ParamObject,
        flags: Option<ParamInfoFlags>,
    },
    NodeParam {
        id: u32,
        seq: i32,
        muted: Option<bool>,
    },
    DeviceParam {
        id: u32,
        seq: i32,
        route: Option<Route>,
    },
    Default(Option<String>),
}

/// What `talkd status` shows of the server and its default source.
#[derive(Clone, Debug, PartialEq)]
pub struct Status {
    pub server: u32,
    /// Or why there is none.
    pub default: Result<DefaultSource, String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DefaultSource {
    pub id: u32,
    pub serial: u64,
    pub name: String,
    pub description: Option<String>,
    /// Where the mixer finds its mute, and the mute; None if nowhere.
    pub mute: Option<(Via, bool)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MicEvent {
    /// Discovery is fenced and initial enumerations finished; individual
    /// snapshots may have failed and remain unavailable for mixing.
    Ready(u32),
    /// Fresh readback after this mutation's write barrier, never a cache hit.
    Confirmed(u64, Pin, bool),
    Gone(Pin),
    /// The connection is dead; [`Mic::connect`] again.
    Lost(String),
}

/// `default.audio.source`'s value: `{"name": "<node.name>"}`, a JSON string.
pub fn default_name(kind: Option<&str>, value: Option<&str>) -> Option<String> {
    if kind != Some("Spa:String:JSON") {
        return None;
    }
    let value: serde_json::Value = serde_json::from_str(value?).ok()?;
    Some(value.get("name")?.as_str()?.to_owned())
}

/// Metadata's null key removes one subject, not all subjects.
fn default_change(
    subject: u32,
    key: Option<&str>,
    kind: Option<&str>,
    value: Option<&str>,
) -> Option<Option<String>> {
    if subject != PW_ID_CORE {
        return None;
    }
    match key {
        Some(DEFAULT_SOURCE) => Some(default_name(kind, value)),
        None => Some(None),
        _ => None,
    }
}

/// The audio sources and devices worth binding.
fn kind(global: &GlobalObject<&DictRef>) -> Option<Kind> {
    let props = global.props?;
    match global.type_ {
        ObjectType::Node => {
            let class = props.get(*pw::keys::MEDIA_CLASS)?;
            (class.starts_with("Audio/Source") || class == "Audio/Duplex").then_some(Kind::Source)
        }
        ObjectType::Device => {
            (props.get(*pw::keys::MEDIA_CLASS)? == "Audio/Device").then_some(Kind::Device)
        }
        ObjectType::Metadata => {
            (props.get("metadata.name")? == "default").then_some(Kind::DefaultMetadata)
        }
        _ => None,
    }
}

fn owned(props: Option<&DictRef>) -> HashMap<String, String> {
    props
        .map(|props| {
            props
                .iter()
                .map(|(key, value)| (key.to_owned(), value.to_owned()))
                .collect()
        })
        .unwrap_or_default()
}

struct Enumeration<T> {
    seq: i32,
    done: AsyncSeq,
    value: T,
    failed: bool,
}

/// Only completed enumerations are visible. A later generation supersedes
/// the old one; empty results replace the cache rather than preserving it.
struct Snapshot<T> {
    flags: Option<ParamInfoFlags>,
    known: bool,
    ready: bool,
    generation: u64,
    value: T,
    pending: Option<Enumeration<T>>,
}

impl<T: Default> Default for Snapshot<T> {
    fn default() -> Self {
        Self {
            flags: None,
            known: false,
            ready: false,
            generation: 0,
            value: T::default(),
            pending: None,
        }
    }
}

impl<T: Default> Snapshot<T> {
    fn info(&mut self, flags: Option<ParamInfoFlags>) -> bool {
        let changed = !self.known || self.flags != flags;
        self.known = true;
        self.flags = flags;
        if changed {
            self.ready = false;
            self.pending = None;
        }
        changed
    }

    fn begin(&mut self, generation: u64, request: Option<(i32, AsyncSeq)>) {
        self.generation = generation;
        self.ready = request.is_none();
        self.pending = request.map(|(seq, done)| Enumeration {
            seq,
            done,
            value: T::default(),
            failed: false,
        });
        if self.ready {
            self.value = T::default();
        }
    }

    fn reply(&mut self, seq: i32) -> Option<&mut T> {
        self.pending
            .as_mut()
            .filter(|pending| pending.seq == seq)
            .map(|pending| &mut pending.value)
    }

    fn error(&mut self, seq: i32) {
        if let Some(pending) = self
            .pending
            .as_mut()
            .filter(|pending| same_seq(pending.seq, seq))
        {
            pending.failed = true;
        }
    }

    fn done(&mut self, seq: AsyncSeq) {
        if self
            .pending
            .as_ref()
            .is_some_and(|pending| pending.done == seq)
        {
            let pending = self.pending.take().expect("matched above");
            self.ready = !pending.failed;
            if self.ready {
                self.value = pending.value;
            }
        }
    }

    fn finished(&self) -> bool {
        self.known && self.pending.is_none()
    }
}

fn same_seq(a: i32, b: i32) -> bool {
    AsyncSeq::from_seq(a) == AsyncSeq::from_seq(b)
}

fn sequence(result: i32) -> Result<i32, String> {
    match SpaResult::from_c(result)
        .into_result()
        .map_err(|error| error.to_string())?
    {
        SpaSuccess::Async(seq) => Ok(seq.raw()),
        SpaSuccess::Sync(seq) => Ok(seq),
    }
}

fn raw_proxy(core: &CoreRc, proxy: &Proxy) -> Result<std::ptr::NonNull<pw::sys::pw_proxy>, String> {
    // SAFETY: core owns this bound proxy. Lookup and the immediately following
    // native call run on the same main-loop thread, with no intervening
    // dispatch; proxy removal/id reuse cannot race them.
    std::ptr::NonNull::new(unsafe { pw::sys::pw_core_find_proxy(core.as_raw_ptr(), proxy.id()) })
        .ok_or_else(|| "the bound PipeWire proxy is gone".to_owned())
}

fn flags(params: &[ParamInfo], id: ParamType) -> Option<ParamInfoFlags> {
    params
        .iter()
        .find(|info| info.id() == id)
        .map(ParamInfo::flags)
}

#[derive(Clone, Copy)]
struct Change {
    token: u64,
    pin: Pin,
    target: bool,
    via: Via,
    object: ParamObject,
    write_seq: Option<i32>,
    barrier: Option<AsyncSeq>,
    after: Option<u64>,
    failed: bool,
}

// Fields drop in order: each listener before its proxy, every proxy before
// the registry and core.
struct Source {
    _listener: NodeListener,
    node: Node,
    serial: u64,
    name: String,
    description: Option<String>,
    device: Option<u32>,
    profile_device: Option<i32>,
    props: Snapshot<Option<bool>>,
}

struct Audio {
    _listener: DeviceListener,
    device: Device,
    routes: Snapshot<Vec<Route>>,
}

struct DefaultMeta {
    _listener: MetadataListener,
    _metadata: Metadata,
    id: u32,
}

struct Link {
    sources: HashMap<u32, Source>,
    devices: HashMap<u32, Audio>,
    metadata: Option<DefaultMeta>,
    _registry_listener: pw::registry::Listener,
    registry: RegistryRc,
    _core_listener: pw::core::Listener,
    core: CoreRc,
    cookie: Option<u32>,
    sync: Option<AsyncSeq>,
    /// Bound something since the last sync was sent.
    bound: bool,
    ready: bool,
    lost: bool,
    default: Option<String>,
    generation: u64,
    change: Option<Change>,
}

impl Link {
    fn mixer(&self, source: &Source) -> Option<(Via, bool)> {
        mixer(source, &self.devices)
    }

    fn pin(&self, source: &Source) -> Option<Pin> {
        Some(Pin {
            server: self.cookie?,
            serial: source.serial,
        })
    }

    /// Startup waits for completion, not universal success. An unrelated
    /// unusable target must not prevent recovery of a healthy pinned mic.
    fn settled(&self) -> bool {
        self.sources.values().all(|source| source.props.finished())
            && self.devices.values().all(|audio| audio.routes.finished())
    }

    fn refresh(&mut self, object: ParamObject) -> Result<u64, String> {
        self.generation = self
            .generation
            .checked_add(1)
            .ok_or("parameter generation overflow")?;
        let generation = self.generation;
        let request = match object {
            ParamObject::Node(id) => {
                let source = self.sources.get_mut(&id).ok_or("source is gone")?;
                source.props.ready = false;
                source.props.pending = None;
                if source
                    .props
                    .flags
                    .is_some_and(|flags| flags.contains(ParamInfoFlags::READ))
                {
                    // SAFETY: the live bound proxy has pw_node_methods and all
                    // arguments follow Node::enum_params. Retain its result:
                    // native marshalling replaces the supplied sequence.
                    Some(sequence(unsafe {
                        spa_interface_call_method!(
                            raw_proxy(&self.core, source.node.upcast_ref())?.as_ptr(),
                            pw::sys::pw_node_methods,
                            enum_params,
                            0,
                            ParamType::Props.as_raw(),
                            0,
                            u32::MAX,
                            std::ptr::null()
                        )
                    })?)
                } else {
                    None
                }
            }
            ParamObject::Device(id) => {
                let audio = self.devices.get_mut(&id).ok_or("device is gone")?;
                audio.routes.ready = false;
                audio.routes.pending = None;
                if audio
                    .routes
                    .flags
                    .is_some_and(|flags| flags.contains(ParamInfoFlags::READ))
                {
                    // SAFETY: as above, with a bound Device and device methods.
                    Some(sequence(unsafe {
                        spa_interface_call_method!(
                            raw_proxy(&self.core, audio.device.upcast_ref())?.as_ptr(),
                            pw::sys::pw_device_methods,
                            enum_params,
                            0,
                            ParamType::Route.as_raw(),
                            0,
                            u32::MAX,
                            std::ptr::null()
                        )
                    })?)
                } else {
                    None
                }
            }
        };
        let request = request
            .map(|seq| {
                self.core
                    .sync(0)
                    .map(|done| (seq, done))
                    .map_err(|error| error.to_string())
            })
            .transpose()?;
        match object {
            ParamObject::Node(id) => self
                .sources
                .get_mut(&id)
                .expect("looked up")
                .props
                .begin(generation, request),
            ParamObject::Device(id) => self
                .devices
                .get_mut(&id)
                .expect("looked up")
                .routes
                .begin(generation, request),
        }
        Ok(generation)
    }

    fn confirm(&mut self, events: &mut Vec<MicEvent>) {
        let Some(change) = self
            .change
            .filter(|change| !change.failed && change.after.is_some())
        else {
            return;
        };
        let generation = match change.object {
            ParamObject::Node(id) => self
                .sources
                .get(&id)
                .filter(|source| source.props.ready)
                .map(|source| source.props.generation),
            ParamObject::Device(id) => self
                .devices
                .get(&id)
                .filter(|audio| audio.routes.ready)
                .map(|audio| audio.routes.generation),
        };
        if !generation
            .zip(change.after)
            .is_some_and(|(actual, needed)| actual >= needed)
        {
            return;
        }
        if self.find(change.pin).and_then(|source| self.mixer(source))
            == Some((change.via, change.target))
        {
            self.change = None;
            events.push(MicEvent::Confirmed(change.token, change.pin, change.target));
        }
    }

    fn find(&self, pin: Pin) -> Option<&Source> {
        if self.lost || self.cookie != Some(pin.server) {
            return None;
        }
        self.sources
            .values()
            .find(|source| source.serial == pin.serial)
    }
}

fn mixer(source: &Source, devices: &HashMap<u32, Audio>) -> Option<(Via, bool)> {
    if !source.props.ready
        || source
            .device
            .and_then(|id| devices.get(&id))
            .is_some_and(|audio| !audio.routes.ready)
    {
        return None;
    }
    params::mixer(
        source.device,
        source.profile_device,
        |on| devices.get(&on).map(|audio| audio.routes.value.as_slice()),
        source.props.value,
    )
}

pub struct Mic {
    context: ContextRc,
    /// An absolute socket path, for tests; None for the session's server.
    remote: Option<String>,
    hook: Rc<dyn Fn(Update)>,
    link: Option<Link>,
}

impl Mic {
    pub fn new(context: ContextRc, remote: Option<String>, hook: Rc<dyn Fn(Update)>) -> Self {
        Mic {
            context,
            remote,
            hook,
            link: None,
        }
    }

    /// Drops the connection. Never from inside one of its own callbacks.
    pub fn disconnect(&mut self) {
        self.link = None;
    }

    pub fn connect(&mut self) -> Result<(), String> {
        self.link = None;
        let props = self.remote.as_deref().map(|remote| {
            pw::properties::properties! {
                *pw::keys::REMOTE_NAME => remote
            }
        });
        let core = self
            .context
            .connect_rc(props)
            .map_err(|error| format!("connecting to PipeWire: {error}"))?;
        let hook = &self.hook;
        let core_listener = core
            .add_listener_local()
            .info({
                let hook = hook.clone();
                move |info| {
                    hook(Update::Info {
                        cookie: info.cookie(),
                    })
                }
            })
            .done({
                let hook = hook.clone();
                move |id, seq| {
                    if id == PW_ID_CORE {
                        hook(Update::Done(seq));
                    }
                }
            })
            .error({
                let hook = hook.clone();
                move |id, seq, res, message| {
                    hook(Update::Error {
                        id,
                        seq,
                        res,
                        message: message.to_owned(),
                    })
                }
            })
            .register();
        let registry = core
            .get_registry_rc()
            .map_err(|error| format!("PipeWire registry: {error}"))?;
        let registry_listener = registry
            .add_listener_local()
            .global({
                let hook = hook.clone();
                move |global| {
                    if let Some(kind) = kind(global) {
                        hook(Update::Global {
                            id: global.id,
                            kind,
                            permissions: global.permissions,
                            props: owned(global.props),
                        });
                    }
                }
            })
            .global_remove({
                let hook = hook.clone();
                move |id| hook(Update::GlobalRemove(id))
            })
            .register();
        let sync = core
            .sync(0)
            .map_err(|error| format!("PipeWire sync: {error}"))?;
        self.link = Some(Link {
            sources: HashMap::new(),
            devices: HashMap::new(),
            metadata: None,
            _registry_listener: registry_listener,
            registry,
            _core_listener: core_listener,
            core,
            cookie: None,
            sync: Some(sync),
            bound: false,
            ready: false,
            lost: false,
            default: None,
            generation: 0,
            change: None,
        });
        Ok(())
    }

    pub fn apply(&mut self, update: Update) -> Vec<MicEvent> {
        let mut events = Vec::new();
        let hook = self.hook.clone();
        let Some(link) = self.link.as_mut().filter(|link| !link.lost) else {
            return events;
        };
        match update {
            Update::Info { cookie } => link.cookie = Some(cookie),
            Update::Done(seq) => {
                for source in link.sources.values_mut() {
                    source.props.done(seq);
                }
                for audio in link.devices.values_mut() {
                    audio.routes.done(seq);
                }
                if let Some(change) = link.change.filter(|change| change.barrier == Some(seq)) {
                    // Only now request readback: cached or already queued
                    // notifications from before the write cannot complete it.
                    let generation = link.refresh(change.object);
                    if let Some(current) = link.change.as_mut() {
                        current.barrier = None;
                        match generation {
                            Ok(generation) => current.after = Some(generation),
                            Err(error) => {
                                current.failed = true;
                                eprintln!("mic: readback: {error}");
                            }
                        }
                    }
                }
                if link.sync == Some(seq) {
                    link.sync = None;
                    if std::mem::take(&mut link.bound) {
                        match link.core.sync(0) {
                            Ok(seq) => link.sync = Some(seq),
                            Err(error) => {
                                link.lost = true;
                                events.push(MicEvent::Lost(format!("PipeWire sync: {error}")));
                            }
                        }
                    }
                }
            }
            Update::Error {
                id, res, message, ..
            } if id == PW_ID_CORE => {
                link.lost = true;
                events.push(MicEvent::Lost(format!("PipeWire: {message} ({res})")));
            }
            Update::Error {
                id,
                seq,
                res,
                message,
            } => {
                for source in link
                    .sources
                    .values_mut()
                    .filter(|source| source.node.upcast_ref().id() == id)
                {
                    source.props.error(seq);
                }
                for audio in link
                    .devices
                    .values_mut()
                    .filter(|audio| audio.device.upcast_ref().id() == id)
                {
                    audio.routes.error(seq);
                }
                if let Some(change) = link
                    .change
                    .as_mut()
                    .filter(|change| change.write_seq.is_some_and(|sent| same_seq(sent, seq)))
                {
                    change.failed = true;
                }
                eprintln!("mic: PipeWire object {id}: {message} ({res})");
            }
            Update::Global {
                id,
                kind,
                permissions,
                props,
            } => {
                let global = GlobalObject::<&DictRef> {
                    id,
                    permissions,
                    type_: match kind {
                        Kind::Source => ObjectType::Node,
                        Kind::Device => ObjectType::Device,
                        Kind::DefaultMetadata => ObjectType::Metadata,
                    },
                    version: 0,
                    props: None,
                };
                if let Err(error) = bind(link, &hook, &global, kind, &props) {
                    eprintln!("mic: binding PipeWire object {id}: {error}");
                }
            }
            Update::GlobalRemove(id) => {
                if let Some(source) = link.sources.remove(&id)
                    && let (true, Some(pin)) = (link.ready, link.pin(&source))
                {
                    events.push(MicEvent::Gone(pin));
                }
                link.devices.remove(&id);
                if link
                    .metadata
                    .as_ref()
                    .is_some_and(|metadata| metadata.id == id)
                {
                    link.metadata = None;
                    link.default = None;
                }
            }
            Update::NodeInfo { id, props } => {
                if let Some(source) = link.sources.get_mut(&id) {
                    let text = |key: &str| props.get(key).map(String::as_str);
                    source.device = text(*pw::keys::DEVICE_ID).and_then(|id| id.parse().ok());
                    source.profile_device =
                        text("card.profile.device").and_then(|device| device.parse().ok());
                    source.description = text("node.description").map(str::to_owned);
                }
            }
            Update::Params { object, flags } => {
                let changed = match object {
                    ParamObject::Node(id) => link
                        .sources
                        .get_mut(&id)
                        .is_some_and(|source| source.props.info(flags)),
                    ParamObject::Device(id) => link
                        .devices
                        .get_mut(&id)
                        .is_some_and(|audio| audio.routes.info(flags)),
                };
                if changed && let Err(error) = link.refresh(object) {
                    eprintln!("mic: parameter refresh: {error}");
                }
            }
            Update::NodeParam { id, seq, muted } => {
                if let Some(value) = link
                    .sources
                    .get_mut(&id)
                    .and_then(|source| source.props.reply(seq))
                {
                    *value = muted;
                }
            }
            Update::DeviceParam { id, seq, route } => {
                if let (Some(routes), Some(route)) = (
                    link.devices
                        .get_mut(&id)
                        .and_then(|audio| audio.routes.reply(seq)),
                    route,
                ) {
                    routes.push(route);
                }
            }
            Update::Default(name) => link.default = name,
        }
        // A late bind may arrive after the last discovery fence, while
        // parameter snapshots are still pending. Fence that bind too; no
        // future Done can clear bound if there is no outstanding sync.
        if !link.lost && !link.ready && link.bound && link.sync.is_none() {
            match link.core.sync(0) {
                Ok(seq) => {
                    link.sync = Some(seq);
                    link.bound = false;
                }
                Err(error) => {
                    link.lost = true;
                    events.push(MicEvent::Lost(format!("PipeWire discovery sync: {error}")));
                }
            }
        }
        if !link.lost
            && !link.ready
            && link.sync.is_none()
            && !link.bound
            && link.settled()
            && let Some(cookie) = link.cookie
        {
            link.ready = true;
            events.push(MicEvent::Ready(cookie));
        }
        if !link.lost {
            link.confirm(&mut events);
        }
        events
    }

    /// The default source and its mute.
    pub fn default_source(&self) -> Option<(Pin, bool)> {
        let link = self.link.as_ref().filter(|link| link.ready && !link.lost)?;
        let name = link.default.as_deref()?;
        let source = link.sources.values().find(|source| source.name == name)?;
        Some((link.pin(source)?, link.mixer(source)?.1))
    }

    /// What `talkd status` shows; None until synced.
    pub fn status(&self) -> Option<Status> {
        let link = self
            .link
            .as_ref()
            .filter(|link| link.ready && !link.lost && link.settled())?;
        let default = match link.default.as_deref() {
            None => Err("none: no default.audio.source in the \"default\" metadata".to_owned()),
            Some(name) => link
                .sources
                .iter()
                .find(|(_, source)| source.name == name)
                .map(|(&id, source)| DefaultSource {
                    id,
                    serial: source.serial,
                    name: source.name.clone(),
                    description: source.description.clone(),
                    mute: link.mixer(source),
                })
                .ok_or_else(|| format!("{name:?}, which is no audio source node")),
        };
        Some(Status {
            server: link.cookie?,
            default,
        })
    }

    pub fn muted(&self, pin: Pin) -> Option<bool> {
        let link = self.link.as_ref()?;
        Some(link.mixer(link.find(pin)?)?.1)
    }

    pub fn exists(&self, pin: Pin) -> bool {
        self.link.as_ref().and_then(|link| link.find(pin)).is_some()
    }

    /// Sends a write, a barrier, then (on its completion) a fresh enum+sync.
    /// Errors returned here precede the write. Failures after it was sent
    /// retain the machine's obligation and expire as unconfirmed.
    pub fn set_mute(&mut self, pin: Pin, muted: bool, token: u64) -> Result<(), String> {
        let link = self
            .link
            .as_mut()
            .filter(|link| !link.lost)
            .ok_or("not connected to PipeWire")?;
        let (&id, source) = link
            .sources
            .iter()
            .find(|(_, source)| source.serial == pin.serial && link.cookie == Some(pin.server))
            .ok_or("the node is gone")?;
        let via = link
            .mixer(source)
            .ok_or("the mute is unavailable or its parameters are refreshing")?
            .0;
        let (object, result) = match via {
            Via::Route { on, index, device } => {
                let audio = link.devices.get(&on).ok_or("its device is gone")?;
                let bytes = params::route_pod(index, device, muted);
                let pod = Pod::from_bytes(&bytes).ok_or("building the Route param")?;
                // SAFETY: live bound Device, same call as Device::set_param;
                // retain the sequence for asynchronous error attribution.
                let result = unsafe {
                    spa_interface_call_method!(
                        raw_proxy(&link.core, audio.device.upcast_ref())?.as_ptr(),
                        pw::sys::pw_device_methods,
                        set_param,
                        ParamType::Route.as_raw(),
                        0,
                        pod.as_raw_ptr()
                    )
                };
                (ParamObject::Device(on), result)
            }
            Via::Props => {
                let bytes = params::props_pod(muted);
                let pod = Pod::from_bytes(&bytes).ok_or("building the Props param")?;
                // SAFETY: live bound Node, matching Node::set_param's arguments.
                let result = unsafe {
                    spa_interface_call_method!(
                        raw_proxy(&link.core, source.node.upcast_ref())?.as_ptr(),
                        pw::sys::pw_node_methods,
                        set_param,
                        ParamType::Props.as_raw(),
                        0,
                        pod.as_raw_ptr()
                    )
                };
                (ParamObject::Node(id), result)
            }
        };
        let write_seq = sequence(result).ok();
        let barrier = link.core.sync(0).ok();
        link.change = Some(Change {
            token,
            pin,
            target: muted,
            via,
            object,
            write_seq,
            barrier,
            after: None,
            failed: write_seq.is_none() || barrier.is_none(),
        });
        Ok(())
    }
}

fn bind(
    link: &mut Link,
    hook: &Rc<dyn Fn(Update)>,
    global: &GlobalObject<&DictRef>,
    kind: Kind,
    props: &HashMap<String, String>,
) -> Result<(), String> {
    let id = global.id;
    match kind {
        Kind::Source => {
            let serial = props
                .get("object.serial")
                .and_then(|serial| serial.parse().ok())
                .ok_or("no object.serial")?;
            let name = props.get(*pw::keys::NODE_NAME).cloned().unwrap_or_default();
            let node: Node = link
                .registry
                .bind(global)
                .map_err(|error| error.to_string())?;
            let listener = node
                .add_listener_local()
                .info({
                    let hook = hook.clone();
                    move |info| {
                        // later infos carry props only when they changed; an
                        // empty set then is no news, not "no device.id"
                        if info.change_mask().contains(NodeChangeMask::PROPS) {
                            hook(Update::NodeInfo {
                                id,
                                props: owned(info.props()),
                            });
                        }
                        if info.change_mask().contains(NodeChangeMask::PARAMS) {
                            hook(Update::Params {
                                object: ParamObject::Node(id),
                                flags: flags(info.params(), ParamType::Props),
                            });
                        }
                    }
                })
                .param({
                    let hook = hook.clone();
                    move |seq, kind, _index, _next, pod| {
                        if kind == ParamType::Props {
                            hook(Update::NodeParam {
                                id,
                                seq,
                                muted: pod.and_then(|pod| params::props_mute(pod.as_bytes())),
                            });
                        }
                    }
                })
                .register();
            link.sources.insert(
                id,
                Source {
                    _listener: listener,
                    node,
                    serial,
                    name,
                    description: None,
                    device: None,
                    profile_device: None,
                    props: Snapshot::default(),
                },
            );
        }
        Kind::Device => {
            let device: Device = link
                .registry
                .bind(global)
                .map_err(|error| error.to_string())?;
            let listener = device
                .add_listener_local()
                .info({
                    let hook = hook.clone();
                    move |info| {
                        if info.change_mask().contains(DeviceChangeMask::PARAMS) {
                            hook(Update::Params {
                                object: ParamObject::Device(id),
                                flags: flags(info.params(), ParamType::Route),
                            });
                        }
                    }
                })
                .param({
                    let hook = hook.clone();
                    move |seq, kind, _index, _next, pod| {
                        if kind == ParamType::Route {
                            hook(Update::DeviceParam {
                                id,
                                seq,
                                route: pod.and_then(|pod| params::route(pod.as_bytes())),
                            });
                        }
                    }
                })
                .register();
            link.devices.insert(
                id,
                Audio {
                    _listener: listener,
                    device,
                    routes: Snapshot::default(),
                },
            );
        }
        Kind::DefaultMetadata => {
            let metadata: Metadata = link
                .registry
                .bind(global)
                .map_err(|error| error.to_string())?;
            let listener = metadata
                .add_listener_local()
                .property({
                    let hook = hook.clone();
                    move |subject, key, kind, value| {
                        if let Some(name) = default_change(subject, key, kind, value) {
                            hook(Update::Default(name));
                        }
                        0
                    }
                })
                .register();
            link.metadata = Some(DefaultMeta {
                _listener: listener,
                _metadata: metadata,
                id,
            });
        }
    }
    if !link.ready {
        link.bound = true;
    }
    Ok(())
}

#[cfg(test)]
impl Mic {
    /// What the connection knows, for test failures.
    fn state(&self) -> String {
        let Some(link) = &self.link else {
            return "not connected".to_owned();
        };
        let sources: Vec<String> = link
            .sources
            .iter()
            .map(|(id, source)| {
                format!(
                    "{id} {} serial {} props {:?} device {:?}/{:?}",
                    source.name,
                    source.serial,
                    source.props.value,
                    source.device,
                    source.profile_device
                )
            })
            .collect();
        format!(
            "default {:?}, metadata {}, sources {sources:?}",
            link.default,
            link.metadata.is_some()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command, Output, Stdio};
    use std::time::{Duration, Instant};

    use pw::loop_::Timeout;
    use pw::main_loop::MainLoopRc;

    #[test]
    fn the_default_source_is_read_from_its_json_like_default_nodes_api() {
        let json = Some("Spa:String:JSON");
        assert_eq!(
            default_name(json, Some(r#"{"name":"alsa_input.pci-0000_00_1f.3"}"#)).as_deref(),
            Some("alsa_input.pci-0000_00_1f.3")
        );
        assert_eq!(
            default_name(json, Some(r#"{ "name": "a b" }"#)).as_deref(),
            Some("a b")
        );
        assert_eq!(default_name(Some("string"), Some(r#"{"name":"x"}"#)), None);
        assert_eq!(default_name(json, Some(r#"{"nom":"x"}"#)), None);
        assert_eq!(default_name(json, None), None);
    }

    #[test]
    fn removing_an_unrelated_metadata_subject_preserves_the_default() {
        let mut source = default_change(
            0,
            Some(DEFAULT_SOURCE),
            Some("Spa:String:JSON"),
            Some(r#"{"name":"mic"}"#),
        )
        .unwrap();
        if let Some(update) = default_change(57, None, None, None) {
            source = update;
        }
        assert_eq!(source.as_deref(), Some("mic"));
        assert_eq!(default_change(0, None, None, None), Some(None));
    }

    #[test]
    fn an_empty_route_snapshot_replaces_old_routes_and_falls_back_to_props() {
        let mut cache = Snapshot::<Vec<Route>>::default();
        cache.info(Some(ParamInfoFlags::READ));
        cache.begin(1, Some((1, AsyncSeq::from_seq(2))));
        cache.reply(1).unwrap().push(Route {
            index: 7,
            device: 3,
            mute: Some(true),
        });
        assert!(!cache.ready, "a partial enumeration is not settled");
        cache.done(AsyncSeq::from_seq(2));
        assert_eq!(
            params::mixer(Some(40), Some(3), |_| Some(&cache.value), Some(false)),
            Some((
                Via::Route {
                    on: 40,
                    index: 7,
                    device: 3
                },
                true
            ))
        );

        cache.info(Some(ParamInfoFlags::READ | ParamInfoFlags::SERIAL));
        cache.begin(2, Some((3, AsyncSeq::from_seq(4))));
        assert!(
            !cache.ready,
            "the old route cannot be acted on during refresh"
        );
        cache.done(AsyncSeq::from_seq(4)); // no Route replies: removed
        assert!(cache.ready);
        assert_eq!(
            params::mixer(Some(40), Some(3), |_| Some(&cache.value), Some(false)),
            Some((Via::Props, false))
        );
    }

    #[test]
    fn missing_props_mute_replaces_the_previous_value() {
        let mut cache = Snapshot::<Option<bool>>::default();
        cache.info(Some(ParamInfoFlags::READ));
        cache.begin(1, Some((1, AsyncSeq::from_seq(2))));
        *cache.reply(1).unwrap() = Some(true);
        cache.done(AsyncSeq::from_seq(2));
        assert_eq!(cache.value, Some(true));

        cache.begin(2, Some((3, AsyncSeq::from_seq(4))));
        *cache.reply(3).unwrap() = None; // new Props lacks the mixer fields
        cache.done(AsyncSeq::from_seq(4));
        assert!(cache.ready);
        assert_eq!(cache.value, None);
    }

    #[test]
    fn superseded_or_failed_enumerations_never_publish_a_snapshot() {
        let mut cache = Snapshot::<Option<bool>>::default();
        cache.begin(1, Some((1, AsyncSeq::from_seq(2))));
        *cache.reply(1).unwrap() = Some(true);
        cache.begin(2, Some((3, AsyncSeq::from_seq(4))));
        assert!(
            cache.reply(1).is_none(),
            "late data belongs to an older generation"
        );
        cache.done(AsyncSeq::from_seq(2));
        assert!(!cache.ready);
        *cache.reply(3).unwrap() = Some(false);
        cache.error(3);
        cache.done(AsyncSeq::from_seq(4));
        assert!(
            !cache.ready,
            "a failed enum is not an empty successful snapshot"
        );
        cache.begin(3, Some((5, AsyncSeq::from_seq(6))));
        *cache.reply(5).unwrap() = Some(false);
        cache.done(AsyncSeq::from_seq(6));
        assert!(cache.ready);
        assert_eq!(cache.value, Some(false));
    }

    #[test]
    fn failed_enumeration_finishes_startup_but_never_supplies_a_mute() {
        let mut cache = Snapshot::<Option<bool>>::default();
        assert!(!cache.finished(), "no initial info yet");
        cache.info(Some(ParamInfoFlags::READ));
        cache.begin(1, Some((1, AsyncSeq::from_seq(2))));
        cache.error(1);
        assert!(
            !cache.finished(),
            "the error does not replace the completion fence"
        );
        cache.done(AsyncSeq::from_seq(2));
        assert!(
            cache.finished(),
            "terminal failure must not block unrelated startup recovery"
        );
        assert!(!cache.ready, "finished is not usable");
    }

    /// A PipeWire and WirePlumber of the test's own: their own runtime,
    /// config and state directories, no D-Bus, and no hardware (WirePlumber's
    /// "policy" profile loads no ALSA, Bluetooth or camera monitor). Two null
    /// sources stand in for a laptop mic and a headset.
    struct Isolated {
        dir: PathBuf,
        pipewire: Option<Child>,
        wireplumber: Option<Child>,
        bin: PathBuf,
        wpctl: PathBuf,
    }

    const PIPEWIRE_CONF: &str = r#"
context.properties = { core.daemon = true core.name = pipewire-0 log.level = 1 }
context.spa-libs = { audio.convert.* = audioconvert/libspa-audioconvert support.* = support/libspa-support }
context.modules = [
  { name = libpipewire-module-protocol-native }
  { name = libpipewire-module-client-node }
  { name = libpipewire-module-metadata }
  { name = libpipewire-module-adapter }
  { name = libpipewire-module-access }
]
context.objects = [
  { factory = adapter args = { factory.name = support.null-audio-sink node.name = "test-mic" node.description = "Test mic"
      media.class = "Audio/Source" audio.position = [ FL FR ] priority.session = 2000 } }
  { factory = adapter args = { factory.name = support.null-audio-sink node.name = "test-headset" node.description = "Test headset"
      media.class = "Audio/Source" audio.position = [ FL FR ] priority.session = 1000 } }
]
"#;

    const WIREPLUMBER_CONF: &str =
        "wireplumber.profiles = { talkd-test = { inherits = [ policy, mixin.stateless ] } }\n";

    fn program(var: &str) -> PathBuf {
        PathBuf::from(
            std::env::var_os(var)
                .unwrap_or_else(|| panic!("{var} must name the binary to run (see default.nix)")),
        )
    }

    impl Isolated {
        fn start() -> Self {
            let pipewire = program("PW_TEST_PIPEWIRE");
            let wireplumber = program("PW_TEST_WIREPLUMBER");
            let dir = std::env::temp_dir().join(format!("talkd-pw-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            let conf = dir.join("xdg/wireplumber/wireplumber.conf.d");
            fs::create_dir_all(&conf).unwrap();
            fs::create_dir_all(dir.join("run")).unwrap();
            fs::write(dir.join("pipewire.conf"), PIPEWIRE_CONF).unwrap();
            fs::write(conf.join("talkd-test.conf"), WIREPLUMBER_CONF).unwrap();
            let mut server = Isolated {
                bin: pipewire.parent().unwrap().to_owned(),
                wpctl: wireplumber.parent().unwrap().join("wpctl"),
                dir,
                pipewire: None,
                wireplumber: None,
            };
            server.pipewire = Some(
                server
                    .command(&pipewire)
                    .arg("-c")
                    .arg(server.dir.join("pipewire.conf"))
                    .spawn()
                    .unwrap(),
            );
            let started = Instant::now();
            while !Path::new(&server.socket()).exists() {
                assert!(
                    started.elapsed() < Duration::from_secs(10),
                    "no PipeWire socket"
                );
                std::thread::sleep(Duration::from_millis(20));
            }
            server.wireplumber = Some(
                server
                    .command(&wireplumber)
                    .args(["-p", "talkd-test"])
                    .spawn()
                    .unwrap(),
            );
            server
        }

        /// A command that can only reach this server.
        fn command(&self, program: &Path) -> Command {
            let mut command = Command::new(program);
            command
                .env_clear()
                .env("XDG_RUNTIME_DIR", self.dir.join("run"))
                .env("PIPEWIRE_RUNTIME_DIR", self.dir.join("run"))
                .env("XDG_CONFIG_HOME", self.dir.join("xdg"))
                .env("XDG_STATE_HOME", self.dir.join("state"))
                .env("HOME", &self.dir)
                .env("DBUS_SESSION_BUS_ADDRESS", "unix:path=/nonexistent")
                .env("DBUS_SYSTEM_BUS_ADDRESS", "unix:path=/nonexistent")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            command
        }

        fn socket(&self) -> String {
            self.dir.join("run/pipewire-0").to_str().unwrap().to_owned()
        }

        fn run(&self, program: &Path, args: &[&str]) -> Output {
            let output = self
                .command(program)
                .args(args)
                .stdout(Stdio::piped())
                .output()
                .unwrap();
            assert!(output.status.success(), "{program:?} {args:?}: {output:?}");
            output
        }

        fn wpctl(&self, args: &[&str]) -> String {
            String::from_utf8(self.run(&self.wpctl, args).stdout).unwrap()
        }

        fn kill_pipewire(&mut self) {
            if let Some(mut pipewire) = self.pipewire.take() {
                let _ = pipewire.kill();
                let _ = pipewire.wait();
            }
        }
    }

    impl Drop for Isolated {
        fn drop(&mut self) {
            for child in [self.wireplumber.as_mut(), self.pipewire.as_mut()]
                .into_iter()
                .flatten()
            {
                let _ = child.kill();
                let _ = child.wait();
            }
            let _ = fs::remove_dir_all(&self.dir);
        }
    }

    struct Owner {
        mic: Option<Mic>,
        events: Vec<MicEvent>,
    }

    #[test]
    #[ignore = "spawns its own PipeWire and WirePlumber; checkPhase runs it"]
    fn against_an_isolated_pipewire_and_wireplumber() {
        // Run the client, not just server subprocesses, with a sanitized
        // environment. PIPEWIRE_REMOTE overrides connect_rc's socket property.
        const CHILD: &str = "TALKD_TEST_ROOT";
        if let Some(root) = std::env::var_os(CHILD) {
            let root = PathBuf::from(root);
            assert_eq!(
                std::env::var_os("XDG_RUNTIME_DIR"),
                Some(root.join("run").into_os_string())
            );
            assert_eq!(
                std::env::var_os("PIPEWIRE_RUNTIME_DIR"),
                Some(root.join("run").into_os_string())
            );
            assert!(std::env::var_os("PIPEWIRE_REMOTE").is_none());
        } else {
            let root = std::env::temp_dir().join(format!("talkd-client-{}", std::process::id()));
            fs::create_dir(&root).unwrap();
            fs::create_dir(root.join("run")).unwrap();
            let output = Command::new(std::env::current_exe().unwrap())
                .env_clear()
                .env(CHILD, &root)
                .env("TMPDIR", &root)
                .env("HOME", &root)
                .env("XDG_RUNTIME_DIR", root.join("run"))
                .env("PIPEWIRE_RUNTIME_DIR", root.join("run"))
                .env("XDG_CONFIG_HOME", root.join("config"))
                .env("XDG_STATE_HOME", root.join("state"))
                .env("DBUS_SESSION_BUS_ADDRESS", "unix:path=/nonexistent")
                .env("DBUS_SYSTEM_BUS_ADDRESS", "unix:path=/nonexistent")
                .env("PW_TEST_PIPEWIRE", program("PW_TEST_PIPEWIRE"))
                .env("PW_TEST_WIREPLUMBER", program("PW_TEST_WIREPLUMBER"))
                .args([
                    "--exact",
                    "mic::tests::against_an_isolated_pipewire_and_wireplumber",
                    "--ignored",
                    "--nocapture",
                ])
                .output()
                .unwrap();
            fs::remove_dir_all(root).unwrap();
            assert!(
                output.status.success(),
                "isolated client failed:\n{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }
        let mut server = Isolated::start();
        pw::init();
        let main_loop = MainLoopRc::new(None).unwrap();
        let context = ContextRc::new(&main_loop, None).unwrap();
        let owner = Rc::new(RefCell::new(Owner {
            mic: None,
            events: Vec::new(),
        }));
        let hook: Rc<dyn Fn(Update)> = {
            let owner = Rc::downgrade(&owner);
            Rc::new(move |update| {
                if let Some(owner) = owner.upgrade() {
                    let mut owner = owner.borrow_mut();
                    let events = owner.mic.as_mut().unwrap().apply(update);
                    owner.events.extend(events);
                }
            })
        };
        owner.borrow_mut().mic = Some(Mic::new(context.clone(), Some(server.socket()), hook));
        owner.borrow_mut().mic.as_mut().unwrap().connect().unwrap();
        // runs the loop until `done`, within 10 s
        let wait = |what: &str, done: &dyn Fn(&Owner) -> bool| {
            let started = Instant::now();
            while !done(&owner.borrow()) {
                assert!(
                    started.elapsed() < Duration::from_secs(10),
                    "timed out waiting for {what}: {:?}; {}",
                    owner.borrow().events,
                    owner.borrow().mic.as_ref().unwrap().state()
                );
                main_loop
                    .loop_()
                    .iterate(Timeout::Finite(Duration::from_millis(20)));
            }
            started.elapsed()
        };
        fn mic(owner: &Owner) -> &Mic {
            owner.mic.as_ref().unwrap()
        }
        let saw = |cursor: usize, event: MicEvent| {
            move |owner: &Owner| owner.events[cursor..].contains(&event)
        };
        let id_of = |name: &str| -> u32 {
            let owner = owner.borrow();
            let link = mic(&owner).link.as_ref().unwrap();
            let found = link.sources.iter().find(|(_, source)| source.name == name);
            *found.unwrap().0
        };

        wait("the first sync", &|owner| {
            matches!(owner.events.first(), Some(MicEvent::Ready(_)))
        });
        let Some(MicEvent::Ready(cookie)) = owner.borrow().events.first().cloned() else {
            unreachable!()
        };
        wait("WirePlumber's default source", &|owner| {
            mic(owner).default_source().is_some()
        });
        let (pin, muted) = mic(&owner.borrow()).default_source().unwrap();
        assert_eq!(pin.server, cookie);
        assert!(!muted, "a null source starts live");
        let mic_id = id_of("test-mic");
        assert_eq!(
            mic(&owner.borrow()).link.as_ref().unwrap().sources[&mic_id].serial,
            pin.serial,
            "the default is the higher-priority source"
        );

        // `talkd status`, on its own connection, reads the same without
        // changing anything
        let status = crate::status::read(Some(server.socket())).unwrap();
        let expected = DefaultSource {
            id: mic_id,
            serial: pin.serial,
            name: "test-mic".to_owned(),
            description: Some("Test mic".to_owned()),
            mute: Some((Via::Props, false)),
        };
        assert_eq!(status.default, Ok(expected));
        assert_eq!(
            mic(&owner.borrow()).status().unwrap().default,
            status.default
        );
        let report = crate::status::format(&status, &Err("no compositor".to_owned()));
        eprint!("{report}");
        assert_eq!(
            server.wpctl(&["get-volume", "@DEFAULT_AUDIO_SOURCE@"]),
            "Volume: 1.00\n"
        );

        // A write barrier followed by a completed, fresh enum proves the set.
        let cursor = owner.borrow().events.len();
        owner
            .borrow_mut()
            .mic
            .as_mut()
            .unwrap()
            .set_mute(pin, true, 1)
            .unwrap();
        let took = wait(
            "the mute readback",
            &saw(cursor, MicEvent::Confirmed(1, pin, true)),
        );
        eprintln!("mute confirmed in {took:?}");
        assert_eq!(mic(&owner.borrow()).muted(pin), Some(true));
        assert_eq!(
            server.wpctl(&["get-volume", "@DEFAULT_AUDIO_SOURCE@"]),
            "Volume: 1.00 [MUTED]\n"
        );

        // a change by anyone else (wpctl, as the MicMute key does) shows up
        server.wpctl(&["set-mute", "@DEFAULT_AUDIO_SOURCE@", "0"]);
        wait("wpctl's unmute", &|owner| {
            mic(owner).muted(pin) == Some(false)
        });

        // the default moves to the headset: the next engage would take it
        let headset_id = id_of("test-headset");
        server.wpctl(&["set-default", &headset_id.to_string()]);
        let headset = wait("the new default", &|owner| {
            mic(owner)
                .default_source()
                .is_some_and(|(default, _)| default != pin)
        });
        eprintln!("default moved in {headset:?}");
        let (headset, _) = mic(&owner.borrow()).default_source().unwrap();
        // the old pin still reads and sets, by serial
        let cursor = owner.borrow().events.len();
        owner
            .borrow_mut()
            .mic
            .as_mut()
            .unwrap()
            .set_mute(pin, true, 2)
            .unwrap();
        wait(
            "the old mic's readback",
            &saw(cursor, MicEvent::Confirmed(2, pin, true)),
        );
        assert_eq!(mic(&owner.borrow()).muted(pin), Some(true));

        // A restore set to its already-current value must also get fresh
        // confirmation, not an earlier identical event.
        let cursor = owner.borrow().events.len();
        owner
            .borrow_mut()
            .mic
            .as_mut()
            .unwrap()
            .set_mute(pin, true, 3)
            .unwrap();
        wait(
            "an unchanged mute's readback",
            &saw(cursor, MicEvent::Confirmed(3, pin, true)),
        );

        // Stage the precise startup interleavings on this real connection,
        // rather than relying on when the server happens to announce globals.
        // First: the last discovery fence finished, a late bind set bound,
        // and a parameter enumeration is still pending.
        let late_fenced = {
            let mut owner = owner.borrow_mut();
            let mic = owner.mic.as_mut().unwrap();
            let link = mic.link.as_mut().unwrap();
            link.ready = false;
            link.sync = None;
            link.bound = true; // the state set by a successful late bind
            link.refresh(ParamObject::Node(mic_id)).unwrap();
            let events = mic.apply(Update::Default(Some("test-headset".to_owned())));
            let fenced = mic.link.as_ref().unwrap().sync.is_some();
            owner.events.extend(events);
            fenced
        };
        wait("the late bind's parameter snapshot", &|owner| {
            mic(owner).link.as_ref().unwrap().sources[&mic_id]
                .props
                .ready
        });

        // Second: the pinned mic is usable, but an unrelated source's enum
        // terminates with a permission error. That failure must not strand
        // ServerUp/startup recovery or become a usable empty snapshot.
        let ready_after_failure = {
            let mut owner = owner.borrow_mut();
            let mic = owner.mic.as_mut().unwrap();
            let link = mic.link.as_mut().unwrap();
            link.ready = false;
            link.sync = None;
            link.bound = false;
            link.refresh(ParamObject::Node(headset_id)).unwrap();
            let source = &link.sources[&headset_id];
            let pending = source.props.pending.as_ref().unwrap();
            let (seq, done, id) = (pending.seq, pending.done, source.node.upcast_ref().id());
            let mut events = mic.apply(Update::Error {
                id,
                seq,
                res: -nix::libc::EACCES,
                message: "injected initialization permission denial".to_owned(),
            });
            events.extend(mic.apply(Update::Done(done)));
            let ready = events.contains(&MicEvent::Ready(cookie));
            owner.events.extend(events);
            ready
        };
        assert!(
            late_fenced && ready_after_failure,
            "initialization stranded: late bind fenced={late_fenced}, unrelated failure completed={ready_after_failure}"
        );
        assert!(mic(&owner.borrow()).exists(headset));
        assert_eq!(
            mic(&owner.borrow()).muted(headset),
            None,
            "failed target remains unavailable"
        );
        let cursor = owner.borrow().events.len();
        owner
            .borrow_mut()
            .mic
            .as_mut()
            .unwrap()
            .set_mute(pin, false, 90)
            .unwrap();
        wait(
            "healthy pinned restore after initialization failure",
            &saw(cursor, MicEvent::Confirmed(90, pin, false)),
        );
        assert_eq!(
            server.wpctl(&["get-volume", &mic_id.to_string()]),
            "Volume: 1.00\n"
        );

        // a node removed: gone, and no longer settable
        let cursor = owner.borrow().events.len();
        server.run(
            &server.bin.join("pw-cli"),
            &["destroy", &headset_id.to_string()],
        );
        wait("the headset gone", &saw(cursor, MicEvent::Gone(headset)));
        assert_eq!(mic(&owner.borrow()).muted(headset), None);
        assert!(
            owner
                .borrow_mut()
                .mic
                .as_mut()
                .unwrap()
                .set_mute(headset, true, 4)
                .is_err()
        );

        // the server gone
        server.kill_pipewire();
        wait("the connection lost", &|owner| {
            owner
                .events
                .iter()
                .any(|event| matches!(event, MicEvent::Lost(_)))
        });
        assert_eq!(mic(&owner.borrow()).muted(pin), None);
        assert!(
            owner
                .borrow_mut()
                .mic
                .as_mut()
                .unwrap()
                .set_mute(pin, false, 5)
                .is_err()
        );
        owner.borrow_mut().mic.as_mut().unwrap().disconnect();
    }
}
