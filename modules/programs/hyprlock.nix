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
  # the option on that compositor first lets the next lock request replace the
  # screen with a working hyprlock. It guards nothing against this user's own
  # processes, which can already set it over the IPC socket; sandboxed
  # (security-context) clients cannot bind the session-lock protocol at all.
  lockCommand = pkgs.writeShellApplication {
    name = "hyprlock-session";
    runtimeInputs = [pkgs.jq pkgs.procps osConfig.programs.hyprland.package config.programs.hyprlock.package];
    text = ''
      if pidof hyprlock >/dev/null; then
        exit 0
      fi
      # Target the compositor hyprlock is about to connect to: the newest live
      # instance serving $WAYLAND_DISPLAY. A crashed instance leaves its directory
      # behind, and hyprctl's defaults (this environment's signature, or the
      # oldest instance) can name another compositor.
      instance=$(hyprctl instances -j |
        jq -r '[.[] | select(.wl_socket == env.WAYLAND_DISPLAY)] | max_by(.time) | .instance // empty') || instance=
      if [ -n "$instance" ]; then
        hyprctl --instance "$instance" eval 'hl.config({ misc = { allow_session_lock_restore = true } })' || true
      else
        echo "hyprlock-session: no Hyprland instance serves WAYLAND_DISPLAY=''${WAYLAND_DISPLAY:-unset}" >&2
      fi
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
