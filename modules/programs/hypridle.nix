{
  lib,
  pkgs,
  ...
}: {
  # Desktop: lock only, with no automatic DPMS or suspend.
  # NVIDIA DPMS currently disconnects the compositor and strands the locker:
  # https://github.com/hyprwm/Hyprland/discussions/16840
  services.hypridle = {
    enable = true;
    settings = {
      general = {
        lock_cmd = "pidof hyprlock || ${lib.getExe pkgs.hyprlock}";
        before_sleep_cmd = "loginctl lock-session";
      };

      listener = [
        {
          timeout = 300;
          on-timeout = "loginctl lock-session";
        }
      ];
    };
  };
}
