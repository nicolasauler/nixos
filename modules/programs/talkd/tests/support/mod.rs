use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

/// A PipeWire and WirePlumber of the test's own: their own runtime,
/// config and state directories, no D-Bus, and no hardware (WirePlumber's
/// "policy" profile loads no ALSA, Bluetooth or camera monitor). Two null
/// sources stand in for a laptop mic and a headset.
pub struct Isolated {
    pub dir: PathBuf,
    pipewire: Option<Child>,
    wireplumber: Option<Child>,
    pub bin: PathBuf,
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

pub fn program(var: &str) -> PathBuf {
    PathBuf::from(
        std::env::var_os(var)
            .unwrap_or_else(|| panic!("{var} must name the binary to run (see package.nix)")),
    )
}

impl Isolated {
    pub fn start() -> Self {
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
    pub fn command(&self, program: &Path) -> Command {
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

    pub fn socket(&self) -> String {
        self.dir.join("run/pipewire-0").to_str().unwrap().to_owned()
    }

    pub fn run(&self, program: &Path, args: &[&str]) -> Output {
        let output = self
            .command(program)
            .args(args)
            .stdout(Stdio::piped())
            .output()
            .unwrap();
        assert!(output.status.success(), "{program:?} {args:?}: {output:?}");
        output
    }

    pub fn wpctl(&self, args: &[&str]) -> String {
        String::from_utf8(self.run(&self.wpctl, args).stdout).unwrap()
    }

    pub fn kill_pipewire(&mut self) {
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
