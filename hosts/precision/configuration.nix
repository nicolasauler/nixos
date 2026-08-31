# Edit this configuration file to define what should be installed on
# your system.  Help is available in the configuration.nix(5) man page
# and in the NixOS manual (accessible by running ‘nixos-help’).
{
  config,
  pkgs,
  inputs,
  lib,
  ...
}: let
  # SentinelOne's agent cannot be referenced by absolute path in a flake: pure
  # evaluation forbids it, which made this host the only one CI could not check
  # at all. The .deb also cannot be vendored here — it is proprietary EDR under
  # a customer agreement, nixpkgs does not carry it, and the community flake
  # only works by fetching it from a third-party mirror. So both local paths come
  # from the environment instead.
  #
  # Unset (CI, and any pure evaluation): the agent is simply not configured, and
  # the other ~300 lines of this host are checked. Note that CI therefore does
  # NOT validate the sentinelone block.
  #
  # On the machine, pass them — this host is already switched with --impure:
  #   SENTINELONE_DEB=/home/nic/bipa/SentinelAgent_linux_x86_64_v25_2_1_20.deb \
  #   SENTINELONE_TOKEN=/home/nic/bipa/sentinel_one_token \
  #     nh os switch -a -- --impure
  #
  # `builtins.getEnv` returns "" under pure evaluation rather than failing, which
  # is what makes this work in both directions. The token belongs in the secret
  # manager once agenix lands; this is a purity fix, not a secrets fix.
  sentinelOneDeb = let p = builtins.getEnv "SENTINELONE_DEB"; in
    if p == "" then null else /. + p;
  sentinelOneToken = let p = builtins.getEnv "SENTINELONE_TOKEN"; in
    if p == "" then null else /. + p;
  sentinelOneEnabled = sentinelOneDeb != null && sentinelOneToken != null;
in {
  imports = [
    # Include the results of the hardware scan.
    ./hardware-configuration.nix
    inputs.home-manager.nixosModules.default
    inputs.sentinelone.nixosModules.sentinelone
  ];

  # Bootloader.
  boot.loader.systemd-boot.enable = true;
  boot.loader.efi.canTouchEfiVariables = true;

  networking.hostName = "precision"; # Define your hostname.
  # networking.wireless.enable = true;  # Enables wireless support via wpa_supplicant.

  # Configure network proxy if necessary
  # networking.proxy.default = "http://user:password@proxy:port/";
  # networking.proxy.noProxy = "127.0.0.1,localhost,internal.domain";

  # Enable networking
  networking.networkmanager.enable = true;

  # Set your time zone.
  time.timeZone = "America/Sao_Paulo";

  # Select internationalisation properties.
  i18n.defaultLocale = "en_US.UTF-8";

  i18n.extraLocaleSettings = {
    LC_ADDRESS = "pt_BR.UTF-8";
    LC_IDENTIFICATION = "pt_BR.UTF-8";
    LC_MEASUREMENT = "pt_BR.UTF-8";
    LC_MONETARY = "pt_BR.UTF-8";
    LC_NAME = "pt_BR.UTF-8";
    LC_NUMERIC = "pt_BR.UTF-8";
    LC_PAPER = "pt_BR.UTF-8";
    LC_TELEPHONE = "pt_BR.UTF-8";
    LC_TIME = "pt_BR.UTF-8";
  };

  # Configure keymap in X11
  # services.xserver = {
  #   xkb.layout = "br";
  #   xkb.variant = "";
  # };

  security.rtkit.enable = true;
  services.pipewire = {
    enable = true;
    alsa.enable = true;
    alsa.support32Bit = true;
    pulse.enable = true;
  };

  # Configure console keymap
  console.keyMap = "br-abnt2";

  # Define a user account. Don't forget to set a password with ‘passwd’.
  users.users.nic = {
    isNormalUser = true;
    description = "nicolas";
    extraGroups = ["networkmanager" "wheel" "docker" "uinput" "dialout"];
    packages = with pkgs; [];
  };

  # home man
  home-manager = {
    extraSpecialArgs = {inherit inputs;};
    users = {
      "nic" = import ./home.nix;
    };
    backupFileExtension = "backup";
  };

  ## don't need docker for now
  # virtualisation.docker.enable = true;
  virtualisation.podman.enable = true;

  # List packages installed in system profile. To search, run:
  # $ nix search wget
  environment.systemPackages = with pkgs; [
    bat
    dust
    firefox
    fzf
    grim
    htop
    keepassxc
    keymapp
    mako
    networkmanagerapplet
    pavucontrol
    playerctl
    procs
    ripgrep
    # rofi-wayland
    slack
    slurp
    sxiv
    tailscale
    tree-sitter
    vscode-extensions.vadimcn.vscode-lldb
    wl-clipboard
    xdg-utils
    xournalpp
    zathura
  ];

  # Flakes
  nix.settings.experimental-features = ["nix-command" "flakes"];

  # Some programs need SUID wrappers, can be configured further or are
  # started in user sessions.
  # programs.mtr.enable = true;
  # programs.gnupg.agent = {
  #   enable = true;
  #   enableSSHSupport = true;
  #   settings = {
  #     default-cache-ttl = 600;
  #   };
  # };

  # List services that you want to enable:

  services.sentinelone = lib.mkIf sentinelOneEnabled {
    enable = true;
    sentinelOneManagementTokenPath = sentinelOneToken;
    # the module built "<email>-<serialNumber>" from the deprecated options; same value, new option
    customerId = "nicolas@bipa.app-nicolas_precision";
    package = pkgs.sentinelone.overrideAttrs (old: {
      version = "v25_2_1_20";
      src = sentinelOneDeb;
    });
  };

  # Constrain the agent's resource footprint. This limits impact on the machine, not
  # what the agent monitors — the deep visibility is inherent to EDR and reducing it
  # would only make the agent report unhealthy to the console, which defeats the point
  # of having it. The module leaves all of this unbounded (TasksMax = infinity, no CPU
  # or memory ceiling). Sized from measured usage: ~300 MB peak, ~73 threads, ~12% of
  # one core average with scan spikes.
  # Also gated: with the agent unconfigured there is no unit to constrain, and
  # defining serviceConfig for a non-existent unit would synthesise one.
  systemd.services.sentinelone.serviceConfig = lib.mkIf sentinelOneEnabled {
    # Pin it to the E-cores (12-21 on this Meteor Lake), so it stays off the P-cores
    # you actually work on. Widen this if scans feel slow.
    AllowedCPUs = "12-21";
    # Yield to foreground work under contention (default weight is 100).
    CPUWeight = 20;
    # Hard cap on runaway. Generous (2 cores) so it never starves past the 30s
    # watchdog and flaps.
    CPUQuota = "200%";
    # Soft throttle above the observed peak; hard ceiling at ~3x so it never OOM-flaps.
    MemoryHigh = "512M";
    MemoryMax = "1G";
    # Deprioritise its disk IO against yours.
    IOWeight = 20;
    # Module sets infinity; it uses ~73, so this is 7x headroom, not a squeeze.
    TasksMax = lib.mkForce 512;
  };

  # Enable the OpenSSH daemon.
  # services.openssh.enable = true;

  services.tailscale.enable = true;

  # Open ports in the firewall.
  # networking.firewall.allowedTCPPorts = [ ... ];
  # networking.firewall.allowedUDPPorts = [ ... ];
  # Or disable the firewall altogether.
  # networking.firewall.enable = false;

  # This value determines the NixOS release from which the default
  # settings for stateful data, like file locations and database versions
  # on your system were taken. It‘s perfectly fine and recommended to leave
  # this value at the release version of the first install of this system.
  # Before changing this value read the documentation for this option
  # (e.g. man configuration.nix or on https://nixos.org/nixos/options.html).
  system.stateVersion = "25.05"; # Did you read the comment?

  xdg.portal = {
    enable = true;
    # extraPortals = [ pkgs.xdg-desktop-portal-gtk ];
  };

  programs.hyprland = {
    enable = true;
    xwayland.enable = true;
    package = inputs.hyprland.packages."${pkgs.stdenv.hostPlatform.system}".hyprland;
  };

  # hyprlock cannot authenticate without its pam service (HM only installs the binary)
  security.pam.services.hyprlock = {};

  hardware = {
    graphics = {
      enable = true;
      extraPackages = with pkgs; [
        vpl-gpu-rt
      ];
    };
    cpu.intel.updateMicrocode = true;
    bluetooth = {
      enable = true;
      powerOnBoot = true;
    };
    opentabletdriver.enable = true;
  };
  services.blueman.enable = true;

  services.thermald.enable = true;
  services.auto-cpufreq.enable = true;
  powerManagement.powertop.enable = true;

  powerManagement = {
    enable = true;
    cpuFreqGovernor = "performance";
  };

  programs.nix-ld.enable = true;
  programs.nix-ld.libraries = with pkgs; [
    # Add any missing dynamic libraries for unpackaged programs
    # here, NOT in environment.systemPackages
    postgresql.lib
  ];

  programs.nh = {
    enable = true;
    clean.enable = true;
    clean.extraArgs = "--keep-since 7d --keep 5";
    flake = "/home/nic/nixos";
  };

  hardware.uinput.enable = true;
  users.groups.uinput.members = ["nic"];
  users.groups.input.members = ["nic"];

  services.postgresql = {
    enable = true;
    package = pkgs.postgresql;

    enableTCPIP = true;
    settings.port = 6543;

    # ensureDatabases = ["finapp"];
    authentication = pkgs.lib.mkOverride 10 ''
      #type database  DBuser  auth-method
      local all       all     trust

      #type database DBuser origin-address auth-method
      # ipv4
      host  all      all     127.0.0.1/32   trust
      # ipv6
      host all       all     ::1/128        trust
    '';

    initialScript = pkgs.writeText "backend-initScript" ''
      CREATE ROLE finapp WITH LOGIN PASSWORD 'finapp' CREATEDB;
      CREATE DATABASE finapp;
      GRANT ALL PRIVILEGES ON DATABASE finapp TO finapp;
    '';
  };

  hardware.keyboard.zsa.enable = true;

  fonts = {
    packages = with pkgs; [
      nerd-fonts.inconsolata
      nerd-fonts.inconsolata-go
      nerd-fonts.fira-code
    ];

    fontconfig = {
      defaultFonts = {
        serif = ["InconslataGo Nerd Font"];
        sansSerif = ["InconslataGo Nerd Font"];
        monospace = ["InconslataGo Nerd Font Mono"];
        emoji = ["InconslataGo Nerd Font"];
      };
    };

    fontDir.enable = true;
  };

  nix.settings = {
    trusted-users = ["nic"];
    trusted-substituters = [
      "https://cache.nixos.org/"
    ];
    substituters = [
      "https://cuda-maintainers.cachix.org"
      "https://hyprland.cachix.org"
      "https://devenv.cachix.org"
    ];
    trusted-public-keys = [
      "cuda-maintainers.cachix.org-1:0dq3bujKpuEPMCX6U4WylrUDZ9JyUG0VpVZa7CNfq5E="
      "hyprland.cachix.org-1:a7pgxzMz7+chwVL3/pzj6jIBMioiJM7ypFP8PwtkuGc="
      "devenv.cachix.org-1:w1cLUi8dv3hnoSPGAuibQv+f9TZLr6cv/Hm9XgU50cw="
    ];
  };
}
