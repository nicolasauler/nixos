{...}: {
  wayland.windowManager.hyprland = {
    enable = true;

    extraConfig = ''
      monitor=DP-4,1920x1080,0x0,1
      monitor=eDP-1,preferred,1920x0,2
      monitor=,preferred,auto,1
    '';

    settings = {
      #monitor = "DP-4,1920x1080,0x0,1";
      #monitor = "eDP-1,preferred,1920x0,2";
      #monitor = ",preferred,auto,1";

      exec-once = [
        "waybar"
        "mako"
        "nm-applet"
        "dbus-update-activation-environment --systemd WAYLAND_DISPLAY XDG_CURRENT_DESKTOP"
        "alacritty -e bash -c 'curl wttr.in | less && zellij'"
      ];

      env = [
        "XCURSOR_SIZE,24"
        #"QT_QPA_PLATFORMTHEME,gtk"
        # "GDK_BACKEND,wayland,x11"
        # "SDL_VIDEODRIVER,wayland"
        # "CLUTTER_BACKEND,wayland"
        # "MOZ_ENABLE_WAYLAND,1"
        # "MOZ_DISABLE_RDD_SANDBOX,1"
        # "_JAVA_AWT_WM_NONREPARENTING=1"
        # "QT_AUTO_SCREEN_SCALE_FACTOR,1"
        # "QT_QPA_PLATFORM,wayland"
        # "LIBVA_DRIVER_NAME,nvidia"
        # "GBM_BACKEND,nvidia-drm"
        # "__GLX_VENDOR_LIBRARY_NAME,nvidia"
        # "WLR_NO_HARDWARE_CURSORS,1"
        # "__NV_PRIME_RENDER_OFFLOAD,1"
        # "__VK_LAYER_NV_optimus,NVIDIA_only"
        # "PROTON_ENABLE_NGX_UPDATER,1"
        # "NVD_BACKEND,direct"
        # "__GL_GSYNC_ALLOWED,1"
        # "__GL_VRR_ALLOWED,1"
        # "WLR_DRM_NO_ATOMIC,1"
        # "WLR_USE_LIBINPUT,1"
        # "XWAYLAND_NO_GLAMOR,1"
        # "__GL_MaxFramesAllowed,1"
        # "WLR_RENDERER_ALLOW_SOFTWARE,1"
      ];

      input = {
        kb_layout = "us";
        kb_variant = "altgr-intl";
        kb_model = "";
        # kb_options = "caps:swapescape,grp:alt_space_toggle";
        # kb_options = "caps:swapescape";
        kb_rules = "";

        follow_mouse = 1;

        touchpad = {
          clickfinger_behavior = true;
          disable_while_typing = true;
          middle_button_emulation = false;
          natural_scroll = false;
          tap-to-click = true;
        };

        # sensitivity = 0.0;
        # accel_profile
        # force_no_accel
      };

      general = {
        gaps_in = 2;
        gaps_out = 2;
        border_size = 2;
        "col.active_border" = "rgba(33ccffee) rgba(00ff99ee) 45deg";
        "col.inactive_border" = "rgba(595959aa)";

        layout = "dwindle";

        allow_tearing = false;
      };

      decoration = {
        # See https://wiki.hyprland.org/Configuring/Variables/ for more

        rounding = 2;

        # drop_shadow = true;
        # shadow_range = 4;
        # shadow_render_power = 3;
        # "col.shadow" = "rgba(1a1a1aee)";
      };

      animations = {
        enabled = false;
      };

      dwindle = {
        # See https://wiki.hyprland.org/Configuring/Dwindle-Layout/ for more
        pseudotile = true; # master switch for pseudotiling. Enabling is bound to mainMod + P in the keybinds section below
        preserve_split = true; # you probably want this
      };

      master = {
        # See https://wiki.hyprland.org/Configuring/Master-Layout/ for more
        new_status = "slave";
        # new_status = master;
      };

      gestures = {
        # See https://wiki.hyprland.org/Configuring/Variables/ for more
        workspace_swipe = false;
      };

      misc = {
        disable_hyprland_logo = true;
        force_default_wallpaper = 0;
      };

      # See https://wiki.hyprland.org/Configuring/Keywords/ for more
      "$mainMod" = "SUPER";

      # notification dismiss
      # makoctl dismiss when I hit mainmod + n + d
      # binds = [
      #   "$mainMod, n&d, exec, makoctl dismiss"
      # ];

      # Example binds, see https://wiki.hyprland.org/Configuring/Binds/ for more
      bind =
        [
          "$mainMod, return, exec, alacritty"
          "$mainMod SHIFT, Q, killactive,"
          "$mainMod SHIFT, E, exit,"
          "$mainMod, F, fullscreen,"
          "$mainMod, w, exec, rofi -show window"
          "$mainMod, b, exec, pkill --signal USR1 waybar"
          "$mainMod SHIFT, F, togglefloating,"
          "ALT, F11, exec, loginctl lock-session"
          "$mainMod, D, exec, rofi -modi drun,run -show drun"
          "$mainMod, P, pseudo," # dwindle
          "$mainMod, t, togglesplit," # dwindle

          "$mainMod SHIFT, Print, exec, grim -g \"$(slurp -d)\" - | wl-copy -t image/png"
          "$mainMod CTRL, Print, exec, grim -g \"$(slurp)\" $HOME/Pictures/$(date +'%s_grim.png')"
          "$mainMod, Print, exec, grim $HOME/Pictures/$(date +'%s_grim.png')"

          "$mainMod, left, movefocus, l"
          "$mainMod, right, movefocus, r"
          "$mainMod, up, movefocus, u"
          "$mainMod, down, movefocus, d"

          "$mainMod, h, movefocus, l"
          "$mainMod, l, movefocus, r"
          "$mainMod, k, movefocus, u"
          "$mainMod, j, movefocus, d"

          "$mainMod SHIFT, left, movewindow, l"
          "$mainMod SHIFT, right, movewindow, r"
          "$mainMod SHIFT, up, movewindow, u"
          "$mainMod SHIFT, down, movewindow, d"

          "$mainMod SHIFT, h, movewindow, l"
          "$mainMod SHIFT, l, movewindow, r"
          "$mainMod SHIFT, k, movewindow, u"
          "$mainMod SHIFT, j, movewindow, d"

          # Example special workspce (scratchpad)
          "$mainMod, S, togglespecialworkspace, magic"
          "$mainMod SHIFT, S, movetoworkspace, special:magic"

          # Scroll through existing workspaces with mainMod + scroll
          "bind = $mainMod, mouse_down, workspace, e+1"
          "bind = $mainMod, mouse_up, workspace, e-1"
        ]
        ++ map
        (n: "$mainMod SHIFT, ${toString n}, movetoworkspace, ${toString (
          if n == 0
          then 10
          else n
        )}") [1 2 3 4 5 6 7 8 9 0]
        ++ map
        (n: "$mainMod, ${toString n}, workspace, ${toString (
          if n == 0
          then 10
          else n
        )}") [1 2 3 4 5 6 7 8 9 0];

      binde = [
        "$mainMod SHIFT, left, moveactive, -20 0"
        "$mainMod SHIFT, right, moveactive, 20 0"
        "$mainMod SHIFT, up, moveactive, 0 -20"
        "$mainMod SHIFT, down, moveactive, 0 20"

        "$mainMod SHIFT, h, moveactive, -20 0"
        "$mainMod SHIFT, l, moveactive, 20 0"
        "$mainMod SHIFT, k, moveactive, 0 -20"
        "$mainMod SHIFT, j, moveactive, 0 20"

        "$mainMod CTRL, left, resizeactive, 30 0"
        "$mainMod CTRL, right, resizeactive, -30 0"
        "$mainMod CTRL, up, resizeactive, 0 -10"
        "$mainMod CTRL, down, resizeactive, 0 10"

        "$mainMod CTRL, l, resizeactive, 30 0"
        "$mainMod CTRL, h, resizeactive, -30 0"
        "$mainMod CTRL, k, resizeactive, 0 -10"
        "$mainMod CTRL, j, resizeactive, 0 10"

        ", XF86AudioRaiseVolume, exec, wpctl set-volume -l 1.5 @DEFAULT_AUDIO_SINK@ 5%+"
      ];

      bindl = [
        ", XF86AudioLowerVolume, exec, wpctl set-volume @DEFAULT_AUDIO_SINK@ 5%-"
        ", XF86AudioMute, exec, wpctl set-mute @DEFAULT_AUDIO_SINK@ toggle"
        ", XF86AudioMicMute, exec, wpctl set-mute @DEFAULT_AUDIO_SOURCE@ toggle"
        ", XF86AudioPlay, exec, playerctl play-pause"
        #", XF86AudioPrev, exec, playerctl previous"
        #", XF86AudioNext, exec, playerctl next"
      ];

      bindm = [
        # Move/resize windows with mainMod + LMB/RMB and dragging
        "$mainMod, mouse:272, movewindow"
        "$mainMod, mouse:273, resizewindow"
      ];
    };
  };
}
