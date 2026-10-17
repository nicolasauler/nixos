{
  config,
  lib,
  pkgs,
  ...
}: let
  palette = config.colorScheme.palette;
  processes = "${lib.getExe pkgs.alacritty} -e ${lib.getExe pkgs.htop}";
  talkd = pkgs.callPackage ../talkd/package.nix {};
in {
  programs.waybar = {
    enable = true;
    package = pkgs.waybar;
    style =
      ''
        @define-color bar-bg #${palette.base00};
        @define-color bar-hover #${palette.base01};
        @define-color bar-border #${palette.base02};
        @define-color bar-muted #${palette.base04};
        @define-color bar-fg #${palette.base05};
        @define-color bar-accent #${palette.base0C};
        @define-color bar-warning #${palette.base0A};
        @define-color bar-error #${palette.base08};
      ''
      + builtins.readFile ./style.css;

    settings.mainBar = {
      layer = "top";
      position = "top";
      height = 30;
      # Allow the clock to yield space on narrower/portrait outputs.
      fixed-center = false;
      modules-left = ["hyprland/workspaces" "hyprland/window"];
      modules-center = ["clock"];
      # Laptop homes append battery; desktop uses the same bar without it.
      modules-right = ["cpu" "memory" "disk" "custom/microphone" "wireplumber#output" "network" "tray"];

      "hyprland/workspaces" = {
        format = "{id}";
        all-outputs = false;
      };
      "hyprland/window" = {
        format = "{}";
        max-length = 40;
        separate-outputs = true;
      };
      clock = {
        format = "{:%a %d %b  %H:%M}";
        tooltip-format = "<big>{:%B %Y}</big>\n<tt><small>{calendar}</small></tt>";
        calendar = {
          mode = "month";
          weeks-pos = "left";
          format.today = "<span color='#${palette.base0C}'><b>{}</b></span>";
        };
      };
      cpu = {
        interval = 5;
        format = "CPU {usage:2}%";
        # The native tooltip retains per-core detail without filling the bar.
        on-click = processes;
      };
      memory = {
        interval = 10;
        format = "RAM {percentage:2}%";
        tooltip-format = "{used:.1f} / {total:.1f} GiB used\n{avail:.1f} GiB available\nSwap: {swapUsed:.1f} / {swapTotal:.1f} GiB\nClick for processes";
        states = {
          warning = 85;
          critical = 95;
        };
        on-click = processes;
      };
      disk = {
        path = "/";
        interval = 60;
        # Free-space pressure matters; a healthy filesystem need not occupy space.
        format = "";
        format-warning = "DISK {percentage_used}%";
        format-critical = "DISK {percentage_used}%";
        states = {
          warning = 85;
          critical = 95;
        };
        tooltip-format = "{free} available on {path}\n{used} / {total} used";
      };
      "custom/microphone" = {
        exec = "exec ${lib.getExe talkd} waybar";
        return-type = "json";
        format = "{text}";
        escape = true;
        restart-interval = 2;
        # No click/scroll actions: this is an indicator, not another mute control.
      };
      "wireplumber#output" = {
        node-type = "Audio/Sink";
        format = "VOL {volume}%";
        format-muted = "VOL muted";
        tooltip-format = "{node_name}\nScroll: output volume; click: audio settings";
        scroll-step = 2.0;
        max-volume = 100.0;
        on-click = lib.getExe pkgs.pavucontrol;
      };
      network = {
        interval = 10;
        family = "ipv4_6";
        format-wifi = "Wi-Fi";
        format-ethernet = "LAN";
        format-linked = "NO IP";
        format-disconnected = "OFFLINE";
        format-disabled = "RADIO OFF";
        tooltip-format = "{ifname}\n{ipaddr}";
        tooltip-format-wifi = "{essid} · {signalStrength}%\n{ifname}\n{ipaddr}";
        tooltip-format-disconnected = "No network connection";
        tooltip-format-disabled = "Wireless radio disabled";
      };
      battery = {
        interval = 30;
        format = "BAT {capacity}%";
        format-charging = "AC +{capacity}%";
        format-plugged = "AC {capacity}%";
        format-not-charging = "AC {capacity}%";
        format-full = "AC full";
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
