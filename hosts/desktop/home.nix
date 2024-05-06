{ config, inputs, lib, pkgs, ... }:

{
  imports = [
    inputs.nix-colors.homeManagerModules.default
    inputs.hyprlock.homeManagerModules.default
    ../../modules/programs/alacritty.nix
    ../../modules/programs/git.nix
    ../../modules/programs/gtk.nix
    ../../modules/programs/hyprland.nix
    ../../modules/programs/mako.nix
    ../../modules/programs/nixvim
    ../../modules/programs/qutebrowser.nix
    ../../modules/programs/rofi.nix
    ../../modules/programs/starship.nix
    ../../modules/programs/waybar.nix
    ../../modules/programs/zathura.nix
    ../../modules/programs/zellij.nix
  ];

  nixpkgs.overlays = [
    inputs.neovim-nightly-overlay.overlay
  ];

  colorScheme = inputs.nix-colors.colorSchemes.gruvbox-dark-medium;

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
    pkgs.adwaita-qt

    # # It is sometimes useful to fine-tune packages, for example, by applying
    # # overrides. You can do that directly here, just don't forget the
    # # parentheses. Maybe you want to install Nerd Fonts with a limited number of
    # # fonts?
    # (pkgs.nerdfonts.override { fonts = [ "FantasqueSansMono" ]; })
    (pkgs.nerdfonts.override
      {
        fonts = [ "Inconsolata" "InconsolataGo" "FiraCode" ];
      }
    )

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
  };

  home.shellAliases = {
    cd = "z";
    ls = "eza --icons";
    g = "git";
    n = "nvim";
    vi = "nvim .";
    cat = "bat";
    ps = "procs";
  };

  programs.home-manager.enable = true;

  programs.bash = {
    enable = true;
    enableCompletion = true;
    bashrcExtra = ''
      export PATH="$HOME/.local/bin:$PATH"
      eval "$(zoxide init bash)"
      eval "$(zellij setup --generate-auto-start bash)"
      eval "$(starship init bash)"
    '';
  };

  programs.hyprlock = {
    enable = true;
    general = {
      ignore_empty_input = true;
      hide_cursor = true;
    };
  };

  services.hypridle = {
    enable = true;
    settings = {
      general = {
        lockCmd = "pidof hyprlock || ${lib.getExe pkgs.hyprlock}";
        beforeSleepCmd = "loginctl lock-session"; #lock before suspend
        afterSleepCmd = "hyprctl dispatch dpms on";
      };
      listener = [
        {
          timeout = 150;
          onTimeout = "brightnessctl -s set 10"; #monitor backlight minimum
          onResume = "brightnessctl -r"; # monitor backlight restore
        }
        {
          timeout = 150;
          onTimeout = "brightnessctl -sd rgb:kbd_backlight set 0"; #keyboard backlight
          onResume = "brightnessctl -rd rgb:kbd_backlight";
        }
        {
          timeout = 300;
          onTimeout = "loginctl lock-session";
        }
        {
          timeout = 330;
          onTimeout = "hyprctl dispatch dpms off"; #screen off
          onResume = "hyprctl dispatch dpms on"; #screen on
        }
      ];
    };
  };

  qt = {
    enable = true;
    platformTheme.name = "gtk";
    style.name = "adwaita-dark";
  };

}
