# Edit this configuration file to define what should be installed on
# your system.  Help is available in the configuration.nix(5) man page
# and in the NixOS manual (accessible by running ‘nixos-help’).
{
  config,
  pkgs,
  inputs,
  ...
}: {
  imports = [
    # Include the results of the hardware scan.
    ./hardware-configuration.nix
    inputs.home-manager.nixosModules.default
  ];

  # Bootloader.
  boot.loader.systemd-boot.enable = true;
  boot.loader.efi.canTouchEfiVariables = true;

  networking.hostName = "desktop"; # Define your hostname.
  # networking.wireless.enable = true;  # Enables wireless support via wpa_supplicant.

  # Configure network proxy if necessary
  # networking.proxy.default = "http://user:password@proxy:port/";
  # networking.proxy.noProxy = "127.0.0.1,localhost,internal.domain";

  # Enable networking
  networking.networkmanager.enable = true;

  # Set your time zone.
  time.timeZone = "America/Sao_Paulo";

  # Select internationalisation properties.
  i18n.defaultLocale = "en_GB.UTF-8";

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
  #   layout = "us";
  #   xkbVariant = "altgr-intl";
  # };

  services.xserver.xkb.extraLayouts.dh = {
    description = "attempt at colemak-dh";
    languages = ["eng"];
    symbolsFile = ../../symbols/colemakdh;
  };

  security.rtkit.enable = true;
  services.pipewire = {
    enable = true;
    alsa.enable = true;
    alsa.support32Bit = true;
    pulse.enable = true;
  };

  console.keyMap = "us";

  # Define a user account. Don't forget to set a password with ‘passwd’.
  users.users.nic = {
    isNormalUser = true;
    description = "nic";
    extraGroups = ["networkmanager" "wheel" "docker" "uinput"];
    packages = with pkgs; [];
  };

  # Allow unfree packages
  # nixpkgs.config.allowUnfree = true;

  home-manager = {
    extraSpecialArgs = {inherit inputs;};
    users = {
      "nic" = import ./home.nix;
    };
    backupFileExtension = "backup";
  };

  virtualisation.docker.enable = true;
  virtualisation.podman.enable = true;

  # List packages installed in system profile. To search, run:
  # $ nix search wget
  environment.systemPackages = with pkgs; [
    bat
    du-dust
    firefox-wayland
    fzf
    grim
    htop
    keepassxc
    mako
    # networkmanagerapplet
    nix-output-monitor
    pavucontrol
    playerctl
    procs
    ripgrep
    rofi-wayland
    slurp
    sxiv
    tree-sitter
    vale
    vscode-extensions.vadimcn.vscode-lldb
    wl-clipboard
    xdg_utils
    xournalpp
  ];

  nix.settings.experimental-features = ["nix-command" "flakes"];

  # Some programs need SUID wrappers, can be configured further or are
  # started in user sessions.
  # programs.mtr.enable = true;
  # programs.gnupg.agent = {
  #   enable = true;
  #   enableSSHSupport = true;
  # };

  # List services that you want to enable:

  # Enable the OpenSSH daemon.
  # services.openssh.enable = true;

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
  system.stateVersion = "23.11"; # Did you read the comment?

  xdg.portal = {
    enable = true;
    # extraPortals = [ pkgs.xdg-desktop-portal-gtk ];
  };

  programs.hyprland = {
    enable = true;
    xwayland.enable = true;
    package = inputs.hyprland.packages."${pkgs.system}".hyprland;
  };

  services.xserver.videoDrivers = ["nvidia"];
  hardware = {
    enableAllFirmware = true;

    graphics.enable = true;

    nvidia = {
      modesetting.enable = true;
      open = false;
      nvidiaSettings = true;
    };

    opentabletdriver.enable = true;
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

  # add extend layer with arrows for my default colemak-dh-wide
  services.kanata = {
    enable = true;
    keyboards.ek68 = {
      devices = ["/dev/input/by-id/usb-hfd.cn_EK68-event-kbd"];
      config = ''
        (defsrc
          esc  1    2    3    4    5    6    7    8    9    0    -    =    bspc
          tab  q    w    e    r    t    y    u    i    o    p    [    ]    \
          caps a    s    d    f    g    h    j    k    l    ;    '    ret
          lsft z    x    c    v    b    n    m    ,    .    /    rsft
          lctl lmet lalt           spc            ralt       rmet
        )

        (defalias
          ext (tap-hold 200 200 spc (layer-toggle extend))
          sym (layer-toggle symbols)
          vim (tap-hold 200 200 z (layer-toggle vim-compat))

          ;; shifted keys
          _ S--
          ! S-1
          @ S-2
          # S-3
          { S-[
          $ S-4
          % S-5
          ^ S-6
          } S-]
          & S-7
          * S-8
          op S-9
          cp S-0
          til S-grv
          ? S-/
          pipe S-\
        )

        (deflayer colemak-dh-wide
          caps 1    2    3    4    5    6    =    7    8    9    0    -    bspc
          tab  q    w    f    p    b    [    j    l    u    y    ;    '    \
          esc  a    r    s    t    g    ]    m    n    e    i    o    ret
          @vim x    c    d    v    ralt /    k    h    ,    .    rsft
          lctl lmet lsft           @ext            @sym       rmet
        )

        (deflayer extend
          _    _    _    _    _    _    _    _    _    _    _    _    _    _
          _    _    prnt _    _    _    _    del  _    _    _    _    _    _
          _    lctl lmet lalt _    _    _    bspc lft  down up   rght _
          _    _    _    _    _    _    _    home pgdn pgup end  _
          _    _    _              _              _           _
        )

        (deflayer symbols
          _    _    _    _    _    _    _    _     _    _    _    _    _    _
          _    @!   @@   @%   @$   =    _    @pipe 7    8    9    +    @_    _
          _    @#   @til @{   @op  [    _    @*    4    5    6    -    _
          _    @^   @&   @}   @cp  ]    0    1     2    3    @?   rsft
          _    _    _              _              _           _
        )

        (deflayer vim-compat
          _    _    _    _    _    _    _    _    _    _    _    _    _    _
          _    _    _    _    _    _    _    _    _    _    _    _    _    _
          _    _    _    _    _    _    _    _    h    j    k    l    _
          _    _    _    _    _    _    _    _    _    _    _    _
          _    _    _              _              _           _
        )
      '';
    };
  };

  nix.settings = {
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
