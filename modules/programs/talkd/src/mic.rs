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
use pw::device::{Device, DeviceListener};
use pw::metadata::{Metadata, MetadataListener};
use pw::node::{Node, NodeChangeMask, NodeListener};
use pw::permissions::PermissionFlags;
use pw::registry::{GlobalObject, RegistryRc};
use pw::spa::param::ParamType;
use pw::spa::pod::Pod;
use pw::spa::utils::dict::DictRef;
use pw::spa::utils::result::AsyncSeq;
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

/// What a listener saw.
pub enum Update {
    Info {
        cookie: u32,
    },
    Done(AsyncSeq),
    Error {
        id: u32,
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
    NodeParam {
        id: u32,
        pod: Vec<u8>,
    },
    DeviceParam {
        id: u32,
        pod: Vec<u8>,
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
    /// Synced: every source and device present is known, with its mute.
    Ready(u32),
    Observed(Pin, bool),
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
    props_mute: Option<bool>,
    /// The mute last reported.
    seen: Option<bool>,
}

struct Audio {
    _listener: DeviceListener,
    device: Device,
    /// The latest Route per profile device.
    routes: HashMap<i32, Route>,
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
}

impl Link {
    fn mixer(&self, source: &Source) -> Option<(Via, bool)> {
        params::mixer(
            source.device,
            source.profile_device,
            |on| {
                self.devices
                    .get(&on)
                    .map(|audio| audio.routes.values().copied().collect())
            },
            source.props_mute,
        )
    }

    fn pin(&self, source: &Source) -> Option<Pin> {
        Some(Pin {
            server: self.cookie?,
            serial: source.serial,
        })
    }

    /// Reports `id`'s mute if it changed.
    fn recompute(&mut self, id: u32, events: &mut Vec<MicEvent>) {
        let Some(source) = self.sources.get(&id) else {
            return;
        };
        let muted = self.mixer(source).map(|(_, muted)| muted);
        let pin = self.pin(source);
        let source = self.sources.get_mut(&id).expect("looked up above");
        if muted == source.seen {
            return;
        }
        source.seen = muted;
        if let (true, Some(pin), Some(muted)) = (self.ready, pin, muted) {
            events.push(MicEvent::Observed(pin, muted));
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
                move |id, _seq, res, message| {
                    hook(Update::Error {
                        id,
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
            Update::Done(seq) if link.sync == Some(seq) => {
                link.sync = None;
                if std::mem::take(&mut link.bound) {
                    // what those binds bring in arrives before this one's done
                    match link.core.sync(0) {
                        Ok(seq) => link.sync = Some(seq),
                        Err(error) => {
                            events.push(MicEvent::Lost(format!("PipeWire sync: {error}")))
                        }
                    }
                } else if !link.ready
                    && let Some(cookie) = link.cookie
                {
                    link.ready = true;
                    events.push(MicEvent::Ready(cookie));
                }
            }
            Update::Done(_) => {}
            Update::Error { id, res, message } if id == PW_ID_CORE => {
                link.lost = true;
                events.push(MicEvent::Lost(format!("PipeWire: {message} ({res})")));
            }
            Update::Error { id, res, message } => {
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
                if let Some(source) = link.sources.remove(&id) {
                    if let (true, Some(pin)) = (link.ready, link.pin(&source)) {
                        events.push(MicEvent::Gone(pin));
                    }
                } else if link.devices.remove(&id).is_some() {
                    let users: Vec<u32> = link
                        .sources
                        .iter()
                        .filter_map(|(&source, node)| (node.device == Some(id)).then_some(source))
                        .collect();
                    for source in users {
                        link.recompute(source, &mut events);
                    }
                } else if link
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
                    link.recompute(id, &mut events);
                }
            }
            Update::NodeParam { id, pod } => {
                if let Some(source) = link.sources.get_mut(&id) {
                    if let Some(muted) = params::props_mute(&pod) {
                        source.props_mute = Some(muted);
                    }
                    link.recompute(id, &mut events);
                }
            }
            Update::DeviceParam { id, pod } => {
                if let (Some(audio), Some(route)) = (link.devices.get_mut(&id), params::route(&pod))
                {
                    audio.routes.insert(route.device, route);
                    let users: Vec<u32> = link
                        .sources
                        .iter()
                        .filter_map(|(&source, node)| (node.device == Some(id)).then_some(source))
                        .collect();
                    for source in users {
                        link.recompute(source, &mut events);
                    }
                }
            }
            Update::Default(name) => link.default = name,
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
        let link = self.link.as_ref().filter(|link| link.ready && !link.lost)?;
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

    /// Sends the change the way the mixer would. Ok means sent, no more.
    pub fn set_mute(&self, pin: Pin, muted: bool) -> Result<(), String> {
        let link = self
            .link
            .as_ref()
            .filter(|link| !link.lost)
            .ok_or("not connected to PipeWire")?;
        let source = link.find(pin).ok_or("the node is gone")?;
        match link.mixer(source).ok_or("the node has no mute")?.0 {
            Via::Route { on, index, device } => {
                let audio = link.devices.get(&on).ok_or("its device is gone")?;
                let pod = params::route_pod(index, device, muted);
                let pod = Pod::from_bytes(&pod).ok_or("building the Route param")?;
                audio.device.set_param(ParamType::Route, 0, pod);
            }
            Via::Props => {
                let pod = params::props_pod(muted);
                let pod = Pod::from_bytes(&pod).ok_or("building the Props param")?;
                source.node.set_param(ParamType::Props, 0, pod);
            }
        }
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
                            })
                        }
                    }
                })
                .param({
                    let hook = hook.clone();
                    move |_seq, _kind, _index, _next, pod| {
                        if let Some(pod) = pod {
                            hook(Update::NodeParam {
                                id,
                                pod: pod.as_bytes().to_vec(),
                            });
                        }
                    }
                })
                .register();
            node.subscribe_params(&[ParamType::Props]);
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
                    props_mute: None,
                    seen: None,
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
                .param({
                    let hook = hook.clone();
                    move |_seq, _kind, _index, _next, pod| {
                        if let Some(pod) = pod {
                            hook(Update::DeviceParam {
                                id,
                                pod: pod.as_bytes().to_vec(),
                            });
                        }
                    }
                })
                .register();
            device.subscribe_params(&[ParamType::Route]);
            link.devices.insert(
                id,
                Audio {
                    _listener: listener,
                    device,
                    routes: HashMap::new(),
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
                        match key {
                            Some(DEFAULT_SOURCE) if subject == 0 => {
                                hook(Update::Default(default_name(kind, value)))
                            }
                            // every key of every subject removed
                            None => hook(Update::Default(None)),
                            _ => {}
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
    link.bound = true;
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
                    source.props_mute,
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
        let saw = |event: MicEvent| move |owner: &Owner| owner.events.contains(&event);
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
        assert_ne!(status.server, 0);
        let report = crate::status::format(&status, &Err("no compositor".to_owned()));
        eprint!("{report}");
        assert!(report.contains("mute lives in: the node's Props\nmuted: no\n"));
        assert_eq!(
            server.wpctl(&["get-volume", "@DEFAULT_AUDIO_SOURCE@"]),
            "Volume: 1.00\n"
        );

        // set, and confirmed by the param coming back; wpctl agrees
        mic(&owner.borrow()).set_mute(pin, true).unwrap();
        let took = wait("the mute to show", &saw(MicEvent::Observed(pin, true)));
        eprintln!("mute confirmed in {took:?}");
        assert_eq!(mic(&owner.borrow()).muted(pin), Some(true));
        assert_eq!(
            server.wpctl(&["get-volume", "@DEFAULT_AUDIO_SOURCE@"]),
            "Volume: 1.00 [MUTED]\n"
        );

        // a change by anyone else (wpctl, as the MicMute key does) shows up
        server.wpctl(&["set-mute", "@DEFAULT_AUDIO_SOURCE@", "0"]);
        wait("wpctl's unmute", &saw(MicEvent::Observed(pin, false)));

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
        mic(&owner.borrow()).set_mute(pin, true).unwrap();
        wait("the old mic's mute", &saw(MicEvent::Observed(pin, true)));

        // a node removed: gone, and no longer settable
        server.run(
            &server.bin.join("pw-cli"),
            &["destroy", &headset_id.to_string()],
        );
        wait("the headset gone", &saw(MicEvent::Gone(headset)));
        assert_eq!(mic(&owner.borrow()).muted(headset), None);
        assert!(mic(&owner.borrow()).set_mute(headset, true).is_err());

        // the server gone
        server.kill_pipewire();
        wait("the connection lost", &|owner| {
            owner
                .events
                .iter()
                .any(|event| matches!(event, MicEvent::Lost(_)))
        });
        assert_eq!(mic(&owner.borrow()).muted(pin), None);
        assert!(mic(&owner.borrow()).set_mute(pin, false).is_err());
        owner.borrow_mut().mic.as_mut().unwrap().disconnect();
    }
}
