{
  config,
  lib,
  osConfig,
  pkgs,
  ...
}: let
  # Every host's hypridle lock_cmd.
  #
  # start-hyprland restarts a compositor that crashed while locked as
  # `Hyprland --safe-mode --locked`. Safe mode loads Hyprland's stock recovery
  # config, not hyprland.lua, and with misc:allow_session_lock_restore off it
  # answers every new lock client with `finished` (hyprlock logs "Seems we got
  # yeeten" and exits): the crash screen then stays up until a reboot. Setting
  # the option on the live compositor first lets the next lock request replace
  # that screen with a working hyprlock. It guards nothing against this user's
  # own processes, which can already set it over the IPC socket.
  lockCommand = pkgs.writeShellApplication {
    name = "hyprlock-session";
    runtimeInputs = [pkgs.procps osConfig.programs.hyprland.package config.programs.hyprlock.package];
    text = ''
      if pidof hyprlock >/dev/null; then
        exit 0
      fi
      # Instance 0 is the live compositor even when this service's environment
      # still names the one that crashed.
      hyprctl --instance 0 eval 'hl.config({ misc = { allow_session_lock_restore = true } })' || true
      exec hyprlock
    '';
  };
in {
  _module.args.lockCommand = lib.getExe lockCommand;

  # needs security.pam.services.hyprlock = {}; on the NixOS side to unlock
  programs.hyprlock = {
    enable = true;
    settings = {
      general = {
        hide_cursor = true;
        ignore_empty_input = true;
      };

      background = [
        {
          monitor = "";
          color = "rgba(0, 0, 0, 1.0)";
        }
      ];

      input-field = [
        {
          monitor = "";
          fade_on_empty = true;
          placeholder_text = "<i>Input Password...</i>";
        }
      ];
    };
  };
}
