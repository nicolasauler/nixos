{
  lib,
  pkgs,
  ...
}: let
  # cargo test runs in buildRustPackage's checkPhase: a failing test means no
  # package, as for ../foot-pedal.
  talk = pkgs.rustPlatform.buildRustPackage {
    pname = "talk";
    version = "0.1.0";
    src = lib.fileset.toSource {
      root = ./.;
      fileset = lib.fileset.unions [./Cargo.toml ./Cargo.lock ./src];
    };
    cargoLock.lockFile = ./Cargo.lock;
    nativeBuildInputs = [pkgs.makeWrapper];
    # wpctl drives the default source; notify-send reaches mako, since
    # Hyprland drops exec output
    postInstall = ''
      wrapProgram $out/bin/talk \
        --prefix PATH : ${lib.makeBinPath [pkgs.wireplumber pkgs.libnotify]}
    '';
    meta.mainProgram = "talk";
  };
in {
  # Push-to-talk on F13, a Voyager key set in Oryx: hold to invert the mic's
  # mute, tap to latch it until the next press (src/main.rs). The binds live
  # here rather than in hyprland_note.lua, which xpsbipa shares without talk.
  # F13 is bound by keycode (KEY_F13 183, + 8 for xkb): xkb's evdev keymap
  # names that key XF86Tools, so the keysym F13 would never match.
  # - release: transparent, since Hyprland 0.56 shadows a release bind whose
  #   key is still held after any other key, click or bind (the lock keys
  #   included), and would skip it; only transparent binds are exempt.
  # - press: auto_consuming, so keys typed during the hold reach the window
  #   instead of being claimed as a partial match of the F13 bind.
  # - both locked, so they still run under hyprlock. A keyboard unplugged or
  #   a VT switched mid-hold still loses the release; the next press then
  #   restores.
  wayland.windowManager.hyprland.extraLuaFiles.talk = ''
    hl.bind("code:191", hl.dsp.exec_cmd("${lib.getExe talk} press"), { locked = true, auto_consuming = true })
    hl.bind("code:191", hl.dsp.exec_cmd("${lib.getExe talk} release"), { locked = true, release = true, transparent = true })
  '';
}
