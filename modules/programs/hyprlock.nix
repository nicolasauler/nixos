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
  # `Hyprland --safe-mode --locked`: locked, with no lock client and no
  # `--locked-cmd`. Unless misc:allow_session_lock_restore is on (it is off by
  # default, and safe mode skips hyprland.lua anyway), Hyprland then answers
  # every new lock client with `finished` (hyprlock logs "Seems we got yeeten"
  # and exits), and the crash screen stays up until a reboot. A hyprlock that
  # dies while Hyprland lives leaves the same state.
  #
  # So when that compositor is locked and no hyprlock runs, the option is turned
  # on there before hyprlock starts, which lets it take over. Healthy sessions
  # are left alone. Sandboxed (security-context) clients cannot bind the
  # session-lock protocol, and this user's own processes could already set the
  # option over the IPC socket.
  lockCommand = pkgs.writeShellApplication {
    # hypridle's automatic sleep inhibition waits for the lock only while
    # lock_cmd contains "hyprlock": keep it in the name.
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
      if [ -z "$instance" ]; then
        echo "hyprlock-session: no Hyprland instance serves WAYLAND_DISPLAY=''${WAYLAND_DISPLAY:-unset}" >&2
      elif [ "$(hyprctl --instance "$instance" locked)" = true ]; then
        hyprctl --instance "$instance" eval 'hl.config({ misc = { allow_session_lock_restore = true } })' || true
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
