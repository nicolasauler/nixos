{
  lib,
  pkgs,
  ...
}: let
  # cargo test runs in buildRustPackage's checkPhase: a failing test means no
  # package. --include-ignored adds the PipeWire test (src/mic.rs), which
  # spawns its own pipewire and wireplumber inside the build sandbox: no
  # sound card, no session bus, no user's PipeWire to reach.
  talkd = pkgs.rustPlatform.buildRustPackage {
    pname = "talkd";
    version = "0.1.0";
    src = lib.fileset.toSource {
      root = ./.;
      fileset = lib.fileset.unions [./Cargo.toml ./Cargo.lock ./src ./protocols];
    };
    cargoLock.lockFile = ./Cargo.lock;
    nativeBuildInputs = [pkgs.pkg-config pkgs.rustPlatform.bindgenHook pkgs.makeWrapper];
    buildInputs = [pkgs.pipewire];
    checkFlags = ["--include-ignored"];
    PW_TEST_PIPEWIRE = lib.getExe' pkgs.pipewire "pipewire";
    PW_TEST_WIREPLUMBER = lib.getExe' pkgs.wireplumber "wireplumber";
    # notify-send reaches mako for the errors that leave the mic flipped
    postInstall = ''
      wrapProgram $out/bin/talkd --prefix PATH : ${lib.makeBinPath [pkgs.libnotify]}
    '';
    meta.mainProgram = "talkd";
  };
in {
  # Push-to-talk on F13, a Voyager key set in Oryx: hold to invert the mic's
  # mute, tap to latch it until the next press; while the session is locked a
  # press only ends a cycle (src/main.rs). talkd reads F13 from the Voyager's
  # evdev node itself (the input group, which this user has), never grabbing
  # it, so Hyprland's bind handling (shadowing, shortcut inhibitors) has no
  # say over press or release.
  systemd.user.services.talkd = {
    Unit = {
      Description = "talkd: push-to-talk on F13";
      PartOf = ["graphical-session.target"];
      After = ["graphical-session.target" "pipewire.service" "wireplumber.service"];
      # failures retry in-process; should a start still die, keep retrying
      # instead of hitting the start limit and staying dead
      StartLimitIntervalSec = 0;
    };
    Service = {
      ExecStart = lib.getExe talkd;
      Restart = "on-failure";
      RestartSec = 2;
    };
    Install.WantedBy = ["graphical-session.target"];
  };

  # F13 still reaches Hyprland, as XF86Tools (xkb's evdev keymap). Swallow it
  # so no focused app acts on a key that means "mic" here; auto_consuming
  # keeps the bind from claiming keys typed while F13 is held, and locked
  # keeps it swallowed under hyprlock too.
  wayland.windowManager.hyprland.extraLuaFiles.talkd = ''
    hl.bind("code:191", hl.dsp.no_op(), { locked = true, auto_consuming = true })
  '';
}
