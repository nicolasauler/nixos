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
    layer = "top";
    gtk-layer-shell = true;
    height = 14;
    position = "top";

    modules-left = [ "hyprland/workspaces", "hyprland/window" ];
    modules-right = [
      "clock"
      "hyprland/language"
      "disk"
      "cpu"
      "custom/gpu-usage"
      "memory"
      "temperature"
      "pulseaudio"
      "battery"
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

    battery = {
      states = {
        warning = 30;
        critical = 15;
      };
      max-length = 20;
      format = "{capacity}% {icon}";
      format-icons = [ "", "", "", "", "" ];
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
      #hwmon-path = "/sys/class/hwmon/hwmon2/temp1_input";
      critical-threshold = 80;
      format-critical = "{temperatureC}°C ";
      format = "{temperatureC}°C ";
    };

    "custom/gpu-usage" = {
      exec = "nvidia-smi --query-gpu=temperature.gpu --format=csv,noheader,nounits";
      format = "{}";
      interval = 10;
    };

    "hyprland/window" = {
      format = "  {}";
      rewrite = {
        "(.*) — Mozilla Firefox" = "󰈹  $1";
        "(.*)Steam" = "Steam 󰓓";
      };
    };

    "hyprland/language" = {
      format = " {}";
      format-en = "english";
    };

    memory = {
      interval = 30;
      format = "󰾆 {used:0.1f} GiB%";
      max-length = 10;
    };

    network = {
      max-length = 30;
      interval = 10;
      format = "{ifname}";
      format-disconnected = " Disconnected󰤭 ";
      format-ethernet = "󰈀 {ipaddr}/{cidr}";
      format-wifi = "󰤨 {essid} {icon}";
      tooltip-format = "󱘖 {ipaddr}  {bandwidthUpBytes}  {bandwidthDownBytes}";
      format-icons = [ "󰤯" "󰤟" "󰤢" "󰤥" "󰤨" ];
    };

    pulseaudio = {
      format = "{icon} {volume}%";
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
      on-click = "pavucontrol";
    };

    tray = {
      icon-size = 15;
      spacing = 5;
    };
  };

  css = ''
    * {
        border: none;
        border-radius: 0px;
        font-family: "JetBrainsMono Nerd Font";
        font-weight: bold;
        font-size: 14px;
        min-height: 0px;
    }

    window#waybar {
    }

    tooltip {
        background: @theme_unfocused_base_color;
        color: @theme_text_color;
        border-radius: 10px;
        border-width: 1px;
        border-style: solid;
        border-color: shade(alpha(@theme_text_colors, 0.9), 1.25);
    }

    #workspaces button {
        box-shadow: none;
    	text-shadow: none;
        padding: 0px;
        border-radius: 7px;
        padding-right: 0px;
        padding-left: 4px;
        margin-right: 7px;
        margin-left: 7px;
        color: @theme_text_color;
        animation: gradient_f 2s ease-in infinite;
        transition: all 0.2s cubic-bezier(.55,-0.68,.48,1.682);
    }

    #workspaces button.active {
        color: @accent_color;
        animation: gradient_f 20s ease-in infinite;
        transition: all 0.3s cubic-bezier(.55,-0.68,.48,1.682);
    }

    #workspaces button:hover {
        color: @accent_color;
        animation: gradient_f 20s ease-in infinite;
        transition: all 0.3s cubic-bezier(.55,-0.68,.48,1.682);
    }

    #cpu,
    #memory,
    #custom-power,
    #clock,
    #workspaces,
    #window,
    #custom-updates,
    #network,
    #bluetooth,
    #pulseaudio,
    #custom-wallchange,
    #custom-mode,
    #tray {
        color: @theme_text_color;
        background: shade(alpha(@theme_text_colors, 0.9), 1.25);
        opacity: 1;
        padding: 0px;
        margin: 3px 3px 3px 3px;
    }

    #custom-battery {
        color: @green_1
    }

    /* resource monitor block */

    #cpu {
        border-radius: 10px 0px 0px 10px;
        margin-left: 25px;
        padding-left: 12px;
        padding-right: 4px;
    }

    #memory {
        border-radius: 0px 10px 10px 0px;
        border-left-width: 0px;
        padding-left: 4px;
        padding-right: 12px;
        margin-right: 6px;
    }


    /* date time block */
    #clock {
        color: @yellow_1;
        padding-left: 12px;
        padding-right: 12px;
    }


    /* workspace window block */
    #workspaces {
        border-radius: 9px 9px 9px 9px;
        background: mix(@theme_unfocused_base_color,white,0.1);
    }

    #window {
        /* border-radius: 0px 10px 10px 0px; */
        /* padding-right: 12px; */
    }


    /* control center block */
    #custom-updates {
        border-radius: 10px 0px 0px 10px;
        margin-left: 6px;
        padding-left: 12px;
        padding-right: 4px;
    }

    #network {
        color: @purple_1;
        padding-left: 4px;
        padding-right: 4px;
    }

    #language {
        color: @orange_1;
        padding-left: 9px;
        padding-right: 9px;
    }

    #bluetooth {
        color: @blue_1;
        padding-left: 4px;
        padding-right: 0px;
    }

    #pulseaudio {
        color: @purple_1;
        padding-left: 4px;
        padding-right: 4px;
    }

    #pulseaudio.microphone {
        color: @red_1;
        padding-left: 0px;
        padding-right: 4px;
    }


    /* system tray block */
    #custom-mode {
        border-radius: 10px 0px 0px 10px;
        margin-left: 6px;
        padding-left: 12px;
        padding-right: 4px;
    }

    #custom-logo {
        margin-left: 6px;
        padding-right: 4px;
        color: @blue_1;
        font-size: 16px;

    }

    #tray {
        padding-left: 4px;
        padding-right: 4px;
    }
  '';
in
{
  programs.waybar = {
    enable = true;
    package = pkgs.waybar.overrideAttrs (oldAttrs: {
      mesonFlags = oldAttrs.mesonFlags ++ [ "-Dexperimental=true" ];
    });
    style = css;
    settings = { mainBar = mainWaybarConfig; };
  };
}
