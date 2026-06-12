{
  lib,
  pkgs,
  ...
}: {
  # laptop variant: lock + screen off + suspend
  # dpms commands use the lua dispatcher form; legacy `hyprctl dispatch dpms`
  # is rejected under lua configs
  services.hypridle = {
    enable = true;
    settings = {
      general = {
        lock_cmd = "pidof hyprlock || ${lib.getExe pkgs.hyprlock}";
        before_sleep_cmd = "loginctl lock-session";
        after_sleep_cmd = "hyprctl dispatch 'hl.dsp.dpms({ action = \"enable\" })'";
      };

      listener = [
        # screen dim + keyboard backlight off; needs pkgs.brightnessctl
        # (device name: check `brightnessctl -l`, often dell::kbd_backlight)
        # {
        #   timeout = 150;
        #   on-timeout = "brightnessctl -s set 10";
        #   on-resume = "brightnessctl -r";
        # }
        # {
        #   timeout = 150;
        #   on-timeout = "brightnessctl -sd rgb:kbd_backlight set 0";
        #   on-resume = "brightnessctl -rd rgb:kbd_backlight";
        # }
        {
          timeout = 300;
          on-timeout = "loginctl lock-session";
        }
        {
          timeout = 330;
          on-timeout = "hyprctl dispatch 'hl.dsp.dpms({ action = \"disable\" })'";
          on-resume = "hyprctl dispatch 'hl.dsp.dpms({ action = \"enable\" })'";
        }
        {
          timeout = 1800;
          on-timeout = "systemctl suspend";
        }
      ];
    };
  };
}
