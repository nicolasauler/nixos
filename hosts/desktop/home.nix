{
  config,
  inputs,
  lib,
  osConfig,
  pkgs,
  waybarLabel,
  ...
}: let
  # Utilization for the bar and temperature for the tooltip, from a long-running
  # nvidia-smi per bar that this script owns: however the script ends (Waybar's
  # SIGTERM on a reload, Waybar dying, a closed stdout, even SIGKILL),
  # nvidia-smi ends too instead of polling into a broken pipe. A line that is
  # not a reading, or no line for 10 s, shows `?` until readings resume (a new
  # nvidia-smi would only hang beside a stuck one). nvidia-smi exiting shows `?`
  # and ends this successfully: Waybar hides a widget whose script fails, and
  # restarts this after its restart-interval either way. The 3060 Ti drives the
  # displays, so the polling never wakes it.
  waybarGpu = pkgs.writeShellApplication {
    name = "waybar-gpu";
    runtimeInputs = [pkgs.coreutils osConfig.hardware.nvidia.package.bin];
    text = ''
      # Four columns, like a reading (`%3s%%`), so `?` never shifts the bar.
      unavailable() {
        printf '{"text":"  ?%%","tooltip":"%s","class":"unavailable"}\n' "$1"
      }
      smi_pid=""
      # Armed before nvidia-smi starts, so no ending of this script outlives it.
      trap '[ -z "$smi_pid" ] || kill "$smi_pid" 2>/dev/null || true' EXIT
      trap 'exit 0' TERM INT HUP
      # Waybar runs this with SIGPIPE ignored, which nvidia-smi would inherit and
      # then poll into a pipe nobody reads. Back at its default, nvidia-smi dies
      # with its reader whenever this script does, even by SIGKILL or before $!
      # is taken. The exec-held descriptor stays readable until EOF, however
      # early nvidia-smi exits.
      exec {readings}< <(exec env --default-signal=PIPE nvidia-smi --id=0 \
        --query-gpu=utilization.gpu,temperature.gpu \
        --format=csv,noheader,nounits --loop-ms=2000)
      smi_pid=$!
      logged=""
      while true; do
        status=0
        IFS= read -r -t 10 -u "$readings" line || status=$?
        if [ "$status" -gt 128 ]; then
          unavailable "No reading from nvidia-smi for 10 s"
          continue
        elif [ "$status" -ne 0 ]; then
          unavailable "nvidia-smi exited"
          exit 0
        fi
        utilization=''${line%%,*}
        utilization=''${utilization// /}
        celsius=''${line#*,}
        celsius=''${celsius// /}
        case $utilization in
          "" | *[!0-9]*)
            # Once per run of identical bad lines, not every 2 s.
            if [ "$logged" != "$line" ]; then
              printf 'waybar-gpu: unexpected nvidia-smi output: %s\n' "$line" >&2
              logged=$line
            fi
            unavailable "nvidia-smi gave no utilization reading"
            continue
            ;;
        esac
        logged=""
        case $celsius in
          "" | *[!0-9]*) temperature="temperature n/a" ;;
          *) temperature="$celsius°C" ;;
        esac
        printf '{"text":"%3s%%","tooltip":"GPU %s","class":"normal"}\n' "$utilization" "$temperature"
      done
    '';
  };
in {
  imports = [
    inputs.nix-colors.homeManagerModules.default
    ../../modules/programs/alacritty.nix
    ../../modules/programs/cliphist.nix
    ../../modules/programs/direnv.nix
    ../../modules/programs/git
    ../../modules/programs/jujutsu.nix
    #../../modules/programs/gtk.nix
    ../../modules/programs/hyprland/hyprland.nix
    ../../modules/programs/hyprlock.nix
    ../../modules/programs/hypridle.nix
    ../../modules/programs/mako.nix
    ../../modules/programs/nix-index.nix
    ../../modules/programs/nixvim
    ../../modules/programs/nushell.nix
    ../../modules/programs/qutebrowser.nix
    ../../modules/programs/rofi.nix
    ../../modules/home/nixpkgs.nix
    ../../modules/programs/shell-aliases.nix
    ../../modules/programs/starship.nix
    ../../modules/programs/waybar/waybar.nix
    ../../modules/programs/zathura.nix
    ../../modules/programs/zellij.nix
  ];

  colorScheme = inputs.nix-colors.colorSchemes.gruvbox-dark-medium;

  # Temperatures go in tooltips: CPU and SSD lead the per-CPU strip's, the GPU's
  # is the GPU widget's.
  programs.waybarCores.args = [
    # k10temp Tctl: the 5800X3D's control temperature.
    "--hwmon"
    "CPU"
    "/sys/devices/pci0000:00/0000:00:18.3/hwmon"
    "temp1_input"
    # nvme0 composite.
    "--hwmon"
    "SSD"
    "/sys/class/nvme/nvme0"
    "temp1_input"
  ];

  programs.waybar.settings.mainBar = {
    # GPU utilization closes the hardware block, before the status modules.
    modules-right = lib.mkOrder 600 ["custom/gpu"];
    # Without the GHz figure: with it and the GPU widget, the 1080px portrait
    # monitor's bar pushes the clock off its edge. The average clock is in the
    # per-CPU strip's tooltip instead.
    cpu.format = lib.mkForce "${waybarLabel "CPU"} {usage:3}% ${waybarLabel "load"} {load:.1f}";
    "custom/gpu" = {
      exec = "exec ${lib.getExe waybarGpu}";
      return-type = "json";
      format = "${waybarLabel "GPU"} {text}";
      escape = true;
      restart-interval = 10;
      exec-on-event = false;
    };
  };

  # Home Manager needs a bit of information about you and the paths it should
  # manage.
  home.username = "nic";
  home.homeDirectory = "/home/nic";

  # This value determines the Home Manager release that your configuration is
  # compatible with. This helps avoid breakage when a new Home Manager release
  # introduces backwards incompatible changes.
  #
  # You should not change this value, even if you update Home Manager. If you do
  # want to update the value, then make sure to first check the Home Manager
  # release notes.
  home.stateVersion = "23.11"; # Please read the comment before changing.

  # The home.packages option allows you to install Nix packages into your
  # environment.
  home.packages = [
    # # Adds the 'hello' command to your environment. It prints a friendly
    # # "Hello, world!" when run.
    # pkgs.hello
    ## pkgs.adwaita-qt

    # # It is sometimes useful to fine-tune packages, for example, by applying
    # # overrides. You can do that directly here, just don't forget the
    # # parentheses. Maybe you want to install Nerd Fonts with a limited number of
    # # fonts?
    # (pkgs.nerdfonts.override { fonts = [ "FantasqueSansMono" ]; })
    # (
    #   pkgs.nerdfonts.override
    #   {
    #     fonts = ["Inconsolata" "InconsolataGo" "FiraCode"];
    #   }
    # )

    # # You can also create simple shell scripts directly inside your
    # # configuration. For example, this adds a command 'my-hello' to your
    # # environment:
    # (pkgs.writeShellScriptBin "my-hello" ''
    #   echo "Hello, ${config.home.username}!"
    # '')
  ];

  # Home Manager is pretty good at managing dotfiles. The primary way to manage
  # plain files is through 'home.file'.
  home.file = {
    # # Building this configuration will create a copy of 'dotfiles/screenrc' in
    # # the Nix store. Activating the configuration will then make '~/.screenrc' a
    # # symlink to the Nix store copy.
    # ".screenrc".source = dotfiles/screenrc;

    # # You can also set the file content immediately.
    # ".gradle/gradle.properties".text = ''
    #   org.gradle.console=verbose
    #   org.gradle.daemon.idletimeout=3600000
    # '';
  };

  # Home Manager can also manage your environment variables through
  # 'home.sessionVariables'. If you don't want to manage your shell through Home
  # Manager then you have to manually source 'hm-session-vars.sh' located at
  # either
  #
  #  ~/.nix-profile/etc/profile.d/hm-session-vars.sh
  #
  # or
  #
  #  ~/.local/state/nix/profiles/profile/etc/profile.d/hm-session-vars.sh
  #
  # or
  #
  #  /etc/profiles/per-user/nic/etc/profile.d/hm-session-vars.sh
  #
  home.sessionVariables = {
    # EDITOR = "emacs";
    BROWSER = "firefox";
    # BROWSER = "${lib.getExe pkgs.firefox-wayland}";
  };

  programs.home-manager.enable = true;

  programs.bash = {
    enable = true;
    enableCompletion = true;
    bashrcExtra = ''
      export PATH="$HOME/.local/bin:$PATH"
    '';
    initExtra = ''
      if [[ -z "$SSH_CLIENT" && -z "$SSH_TTY" ]]; then
        eval "$(zellij setup --generate-auto-start bash)"
      fi
    '';
    # eval "$(zoxide init bash)"
    # eval "$(starship init bash)"
  };

  services.ssh-agent.enable = true;

  # qt = {
  #   enable = true;
  #   platformTheme.name = "gtk";
  #   style.name = "adwaita-dark";
  # };

  xdg.userDirs = {
    enable = true;
    # nothing reads the XDG_*_DIR env vars; apps use user-dirs.dirs, which is still generated
    setSessionVariables = false;
  };
  xdg.mimeApps = {
    enable = true;
    defaultApplications = {
      "default-web-browser" = ["firefox.desktop"];
      "text/html" = ["firefox.desktop"];
      "text/xml" = ["firefox.desktop"];
      "x-scheme-handler/http" = ["firefox.desktop"];
      "x-scheme-handler/https" = ["firefox.desktop"];
      "x-scheme-handler/about" = ["firefox.desktop"];
      "x-scheme-handler/unknown" = ["firefox.desktop"];
    };
  };
}
