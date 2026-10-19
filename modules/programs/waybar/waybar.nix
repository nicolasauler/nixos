{
  config,
  lib,
  pkgs,
  ...
}: let
  palette = config.colorScheme.palette;
  processes = "${lib.getExe pkgs.alacritty} -e ${lib.getExe pkgs.htop}";
  talkd = pkgs.callPackage ../talkd/package.nix {};
  cores = pkgs.callPackage ./cores {};
  # Labels in blue, values in the normal foreground, so the two read apart.
  label = text: "<span color='#${palette.base0D}'>${text}</span>";
in {
  # Host files label their own modules (precision's sensors) the same way.
  _module.args.waybarLabel = label;

  programs.waybar = {
    enable = true;
    package = pkgs.waybar;
    style =
      ''
        @define-color bar-bg #${palette.base00};
        @define-color bar-hover #${palette.base01};
        @define-color bar-border #${palette.base02};
        @define-color bar-muted #${palette.base04};
        /* White, as in the previous bar: Gruvbox's beige foreground read as grey. */
        @define-color bar-fg #ffffff;
        @define-color bar-accent #${palette.base0C};
        @define-color bar-warning #${palette.base0A};
        @define-color bar-error #${palette.base08};
        /* The clock is the most-read value: a hue no status uses. */
        @define-color bar-clock #${palette.base0E};
      ''
      + builtins.readFile ./style.css;

    settings.mainBar = {
      layer = "top";
      position = "top";
      # One line of 15px text in the same 24px as the previous bar.
      height = 24;
      fixed-center = false;
      modules-left = ["hyprland/workspaces" "hyprland/window"];
      # Right-aligned: the hardware block, host sensors (order 600), then
      # status. Laptop homes append battery.
      modules-right = lib.mkMerge [
        (lib.mkOrder 500 ["cpu" "custom/cores" "memory" "disk"])
        ["custom/microphone" "wireplumber#output" "network" "clock" "tray"]
      ];

      "hyprland/workspaces" = {
        format = "{id}";
        all-outputs = false;
      };
      "hyprland/window" = {
        # Ellipsizes before the right-aligned modules would clip.
        max-length = 80;
        separate-outputs = true;
        rewrite = {
          "(.*) — Mozilla Firefox" = "󰈹  $1";
          "(.*)Steam" = "Steam 󰓓";
        };
      };
      clock = {
        format = "{:%d/%m %H:%M}";
        format-alt = "{:%a %d %b %Y %H:%M}";
        # The whole year with today highlighted; scroll moves, right click
        # switches to a single month.
        tooltip-format = "<tt><small>{calendar}</small></tt>";
        calendar = {
          mode = "year";
          mode-mon-col = 3;
          weeks-pos = "right";
          on-scroll = 1;
          format = {
            months = "<span color='#ffead3'><b>{}</b></span>";
            days = "<span color='#ecc6d9'><b>{}</b></span>";
            weeks = "<span color='#99ffdd'><b>W{}</b></span>";
            weekdays = "<span color='#ffcc66'><b>{}</b></span>";
            today = "<span color='#ff6699'><b><u>{}</u></b></span>";
          };
        };
        actions = {
          on-click-right = "mode";
          on-scroll-up = "shift_up";
          on-scroll-down = "shift_down";
        };
      };
      cpu = {
        interval = 2;
        format = "${label "CPU"} {usage:3}% ${label "load"} {load:.1f} {avg_frequency:.1f}GHz";
        on-click = processes;
      };
      "custom/cores" = {
        exec = "exec ${lib.getExe cores}";
        return-type = "json";
        format = "{text}";
        escape = true;
        restart-interval = 2;
        on-click = processes;
        exec-on-event = false;
      };
      memory = {
        interval = 5;
        format = "${label "RAM"} {used:4.1f}/{total:.1f}GiB";
        tooltip-format = "{used:.1f} / {total:.1f} GiB used\n{avail:.1f} GiB available\nSwap: {swapUsed:.1f} / {swapTotal:.1f} GiB\nClick for processes";
        states = {
          warning = 85;
          critical = 95;
        };
        on-click = processes;
      };
      disk = {
        path = "/";
        interval = 15;
        format = "${label "/"} {percentage_used}% ${label "free"} {free}";
        states = {
          warning = 85;
          critical = 95;
        };
        tooltip-format = "{free} available on {path}\n{used} / {total} used";
      };
      "custom/microphone" = {
        exec = "exec ${lib.getExe talkd} waybar";
        return-type = "json";
        format = "${label "MIC"} {text}";
        escape = true;
        restart-interval = 2;
        # No click/scroll actions: this is an indicator, not another mute control.
      };
      "wireplumber#output" = {
        node-type = "Audio/Sink";
        format = "${label "VOL"} {volume}%";
        format-muted = "${label "VOL"} muted";
        tooltip-format = "{node_name}\nScroll: output volume; click: audio settings";
        scroll-step = 2.0;
        max-volume = 100.0;
        on-click = lib.getExe pkgs.pavucontrol;
      };
      network = {
        interval = 10;
        # One line, at most 26 characters: an SSID can be long, and {ipaddr}
        # with ipv4_6 always holds a newline, so addresses stay in the tooltip.
        max-length = 26;
        family = "ipv4_6";
        format-wifi = "${label "{ifname}"} {essid}";
        format-ethernet = "${label "{ifname}"} wired";
        format-linked = "${label "{ifname}"} NO IP";
        format-disconnected = "OFFLINE";
        format-disabled = "RADIO OFF";
        tooltip-format = "{ifname}\n{ipaddr}";
        tooltip-format-wifi = "{essid} · {signalStrength}%\n{ifname}\n{ipaddr}";
        tooltip-format-disconnected = "No network connection";
        tooltip-format-disabled = "Wireless radio disabled";
      };
      battery = {
        interval = 30;
        format = "${label "BAT"} {capacity}%";
        format-charging = "${label "AC"} +{capacity}%";
        format-plugged = "${label "AC"} {capacity}%";
        format-not-charging = "${label "AC"} {capacity}%";
        format-full = "${label "AC"} full";
        states = {
          warning = 20;
          critical = 10;
        };
        tooltip-format = "{capacity}% · {timeTo}\n{power:.1f} W";
      };
      tray = {
        icon-size = 16;
        spacing = 8;
      };
    };
  };
}
