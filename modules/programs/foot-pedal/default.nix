{
  lib,
  pkgs,
  ...
}: let
  # cargo test runs in buildRustPackage's checkPhase: a failing test means no
  # package, as for modules/services/buildbot-pr-policy.nix.
  foot-pedal = pkgs.rustPlatform.buildRustPackage {
    pname = "foot-pedal";
    version = "0.1.0";
    src = lib.fileset.toSource {
      root = ./.;
      fileset = lib.fileset.unions [./Cargo.toml ./Cargo.lock ./src];
    };
    cargoLock.lockFile = ./Cargo.lock;
    meta.mainProgram = "foot-pedal";
  };
in {
  # PCsensor 3-pedal footswitch as hold keys for the Voyager: left = Shift,
  # middle = mirror layer, right = mouse layer (mapping in src/main.rs). Layer
  # pedals talk to the keyboard's raw-HID interface directly (ZSA's Oryx
  # protocol), so keymapp is not needed; hardware.keyboard.zsa makes that node
  # writable. The Voyager's own input is never intercepted. Needs the input and
  # uinput groups, which this user has; the pedal is hidden from libinput by a
  # udev rule in the host configuration.
  systemd.user.services.foot-pedal = {
    Unit = {
      Description = "Foot pedal: Voyager layers and Shift";
      PartOf = ["graphical-session.target"];
      After = ["graphical-session.target"];
      # failures retry in-process; a start that dies (no /dev/uinput) keeps
      # retrying instead of hitting the start limit and staying dead
      StartLimitIntervalSec = 0;
    };
    Service = {
      ExecStart = lib.getExe foot-pedal;
      Restart = "on-failure";
      RestartSec = 2;
    };
    Install.WantedBy = ["graphical-session.target"];
  };
}
