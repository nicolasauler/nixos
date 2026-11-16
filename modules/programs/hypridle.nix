{lockCommand, ...}: {
  # Desktop: lock only, with no automatic DPMS or suspend. Monitor power-off went
  # after the 11-10 lockout, suspected to be NVIDIA DPMS taking Hyprland down
  # (https://github.com/hyprwm/Hyprland/discussions/16840); README "Desktop
  # lockouts" has what is known.
  services.hypridle = {
    enable = true;
    settings = {
      general = {
        lock_cmd = lockCommand;
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
