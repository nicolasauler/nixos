{ pkgs
, config
, ...
}:
let
  workspaces = {
    format = "{id}"; # add urgent in CSS
    on-click = "activate";
  };

  mainWaybarConfig = {
    mod = "dock";
    # layer = "top";
    # gtk-layer-shell = true;
    height = 14;
    position = "top";

    modules-left = [ "hyprland/workspaces" "hyprland/window" ];
    modules-right = [
      "clock"
      "hyprland/language"
      "disk"
      "cpu"
      "cpu#cores"
      "custom/gpu_usage"
      "custom/gpu_temp"
      "memory"
      "temperature"
      "pulseaudio"
      #"battery"
      #"bluetooth"
      "network"
      "tray"
    ];

    "wlr/workspaces" = workspaces;
    "hyprland/workspaces" = workspaces;

    bluetooth = {
      format = "";
      format-connected = " {num_connections}";
      format-disabled = "󰂲";
      tooltip-format = " {device_alias}";
      tooltip-format-connected = "{device_enumerate}";
      tooltip-format-enumerate-connected = " {device_alias}";
    };

    clock = {
      format = "󰥔  {:%m-%d %H:%M}";
      format-alt = "󰥔  {:%A, %B %d, %Y (%R)}";
      tooltip-format = "<tt><small>{calendar}</small></tt>";
      calendar = {
        mode = "year";
        mode-mon-col = 3;
        weeks-pos = "right";
        on-scroll = 1;
        on-click-right = "mode";
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
        on-click-forward = "tz_up";
        on-click-backward = "tz_down";
        on-scroll-up = "shift_up";
        on-scroll-down = "shift_down";
      };
    };

    cpu = {
      interval = 10;
      format = "󰍛 {usage}%";
    };

    "cpu#cores" = {
      interval = 10;
      format = "󰍛 {usage0}% {usage2}% {usage4}% {usage6}% {usage8}% {usage10}% {usage12}% {usage14}%";
    };

    battery = {
      states = {
        warning = 30;
        critical = 15;
      };
      max-length = 20;
      format = "{capacity}% {icon}";
      format-icons = [ "" "" "" "" "" ];
      #format-charging = "<span font-family='Font Awesome 6 Free'></span>";
      format-plugged = "󰚥";
      format-notcharging = "󰚥";
      format-full = "󰂄";
    };

    disk = {
      interval = 30;
      format = "/ {percentage_used}%";
      max-length = 10;
      path = "/";
    };

    temperature = {
      interval = 10;
      #thermal-zone = 2;
      hwmon-path = "/sys/class/hwmon/hwmon1/temp3_input";

      critical-threshold = 80;
      format-critical = "{temperatureC}°C ";
      format = "{temperatureC}°C ";
    };

    "custom/gpu_usage" = {
      exec = "nvidia-smi --query-gpu=utilization.gpu --format=csv,noheader,nounits";
      format = " 󰨜 {} ";
      interval = 10;
      max-length = 8;
    };

    "custom/gpu_temp" = {
      exec = "nvidia-smi --query-gpu=temperature.gpu --format=csv,noheader,nounits";
      format = "  {} ";
      interval = 10;
      max-length = 8;
    };

    "hyprland/window" = {
      format = "  {}";
      rewrite = {
        "(.*) — Mozilla Firefox" = "󰈹  $1";
        "(.*)Steam" = "Steam 󰓓";
      };
      separate-outputs = true;
    };

    "hyprland/language" = {
      format = " {}";
      format-br = "br";
      format-pt = "pt";
      format-en = "en";
    };

    memory = {
      interval = 30;
      format = " {used:0.1f} GiB";
      max-length = 25;
    };

    network = {
      max-length = 70;
      interval = 10;
      format = "{ifname}";
      format-disconnected = " Disconnected󰤭 ";
      format-ethernet = "{ifname} 󰈀 {ipaddr}/{cidr}";
      format-wifi = "{ifname} {icon} {essid}";
      tooltip-format = "󱘖 {ipaddr}  {bandwidthUpBytes}  {bandwidthDownBytes}";
      format-icons = [ "󰤯" "󰤟" "󰤢" "󰤥" "󰤨" ];
    };

    pulseaudio = {
      format = "{icon}  {volume}%";
      format-bluetooth = "{icon} {volume}%";
      format-muted = "";
      format-icons = {
        headphone = " ";
        hands-free = " ";
        headset = " ";
        phone = " ";
        portable = " ";
        car = " ";
        default = [ "" "" "" ];
      };
      scroll-step = 5;
      #on-click = "mute";
      #on-click-right = "pavucontrol";
      max-volume = 150;
    };

    tray = {
      icon-size = 15;
      spacing = 5;
    };
  };
in
{
  programs.waybar = {
    enable = true;
    package = pkgs.waybar.overrideAttrs (oldAttrs: {
      mesonFlags = oldAttrs.mesonFlags ++ [ "-Dexperimental=true" ];
    });
    settings = { mainBar = mainWaybarConfig; };
    style = ./style.css;
  };
}
