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

  networking.hostName = "xpsbipa"; # Define your hostname.
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
    du-dust
    firefox-wayland
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
    rofi-wayland
    slurp
    sxiv
    tailscale
    tree-sitter
    vscode-extensions.vadimcn.vscode-lldb
    wl-clipboard
    xdg-utils
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

  # add extend layer with arrows for my default colemak-dh-wide
  ## services.kanata = {
  ##   enable = true;
  ##   keyboards.ek68 = {
  ##     devices = ["/dev/input/by-id/usb-hfd.cn_EK68-event-kbd"];
  ##     config = ''
  ##       (defsrc
  ##         esc  1    2    3    4    5    6    7    8    9    0    -    =    bspc
  ##         tab  q    w    e    r    t    y    u    i    o    p    [    ]    \
  ##         caps a    s    d    f    g    h    j    k    l    ;    '    ret
  ##         lsft z    x    c    v    b    n    m    ,    .    /    rsft
  ##         lctl lmet lalt           spc            ralt       rmet
  ##       )

  ##       (defalias
  ##         ext (tap-hold 200 200 spc (layer-toggle extend))
  ##         sym (layer-toggle symbols)
  ##         vim (tap-hold 200 200 z (layer-toggle vim-compat))

  ##         ;; shifted keys
  ##         _ S--
  ##         ! S-1
  ##         @ S-2
  ##         # S-3
  ##         { S-[
  ##         $ S-4
  ##         % S-5
  ##         ^ S-6
  ##         } S-]
  ##         & S-7
  ##         * S-8
  ##         op S-9
  ##         cp S-0
  ##         til S-grv
  ##         ? S-/
  ##         pipe S-\
  ##       )

  ##       (deflayer colemak-dh-wide
  ##         caps 1    2    3    4    5    6    =    7    8    9    0    -    bspc
  ##         tab  q    w    f    p    b    [    j    l    u    y    ;    '    \
  ##         esc  a    r    s    t    g    ]    m    n    e    i    o    ret
  ##         @vim x    c    d    v    ralt /    k    h    ,    .    rsft
  ##         lctl lmet lsft           @ext            @sym       rmet
  ##       )

  ##       (deflayer extend
  ##         _    _    _    _    _    _    _    _    _    _    _    _    _    _
  ##         _    _    prnt _    _    _    _    del  _    _    _    _    _    _
  ##         _    lctl lmet lalt _    _    _    bspc lft  down up   rght _
  ##         _    _    _    _    _    _    _    home pgdn pgup end  _
  ##         _    _    _              _              _           _
  ##       )

  ##       (deflayer symbols
  ##         _    _    _    _    _    _    _    _     _    _    _    _    _    _
  ##         _    @!   @@   @%   @$   =    _    @pipe 7    8    9    +    @_    _
  ##         _    @#   @til @{   @op  [    _    @*    4    5    6    -    _
  ##         _    @^   @&   @}   @cp  ]    0    1     2    3    @?   rsft
  ##         _    _    _              _              _           _
  ##       )

  ##       (deflayer vim-compat
  ##         _    _    _    _    _    _    _    _    _    _    _    _    _    _
  ##         _    _    _    _    _    _    _    _    _    _    _    _    _    _
  ##         _    _    _    _    _    _    _    _    h    j    k    l    _
  ##         _    _    _    _    _    _    _    _    _    _    _    _
  ##         _    _    _              _              _           _
  ##       )
  ##     '';
  ##   };
  ## };

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

  ## environment.etc."alloy/client.alloy" = {
  ##   text = ''
  ##     logging {
  ##       level  = "debug"
  ##       format = "logfmt"
  ##     }

  ##     otelcol.receiver.otlp "default" {
  ##       grpc {
  ##         endpoint = "0.0.0.0:4317"
  ##       }

  ##       output {
  ##         logs = [otelcol.processor.batch.default.input]
  ##         traces = [otelcol.processor.batch.default.input]
  ##         metrics = [otelcol.processor.batch.default.input]
  ##       }
  ##     }

  ##     otelcol.processor.batch "default" {
  ##       output {
  ##         logs = [otelcol.exporter.otlphttp.loki.input]
  ##         traces = [otelcol.exporter.otlp.tempo.input]
  ##         metrics = [otelcol.exporter.prometheus.default.input]
  ##       }
  ##     }

  ##     otelcol.exporter.otlphttp "loki" {
  ##       client {
  ##         endpoint = "http://127.0.0.1:3100/otlp"
  ##       }
  ##     }

  ##     otelcol.exporter.otlp "tempo" {
  ##       client {
  ##         endpoint = "http://127.0.0.1:14319"
  ##         tls { insecure = true }
  ##       }
  ##     }

  ##     otelcol.exporter.prometheus "default" {
  ##       forward_to = [prometheus.remote_write.mimir.receiver]
  ##     }

  ##     prometheus.remote_write "mimir" {
  ##       endpoint {
  ##         url = "http://127.0.0.1:3300/api/v1/push"
  ##       }
  ##     }

  ##   '';
  ## };

  ## ## in tempo alloy
  ## #           tls {
  ## #             insecure = true
  ## #             insecure_skip_verify = true
  ## #           }

  ## services = {
  ##   grafana = {
  ##     enable = true;
  ##     settings = {
  ##       server = {
  ##         http_addr = "127.0.0.1";
  ##         http_port = 3000;
  ##       };
  ##       security.admin_user = "nic";
  ##       security.admin_password = "senhadificil";
  ##       # security.disable_initial_admin_creation = true;
  ##     };
  ##     provision = {
  ##       enable = true;
  ##       datasources.settings.datasources = [
  ##         {
  ##           name = "Loki";
  ##           type = "loki";
  ##           access = "proxy";
  ##           url = "http://127.0.0.1:${toString config.services.loki.configuration.server.http_listen_port}";
  ##         }
  ##         {
  ##           name = "Tempo";
  ##           type = "tempo";
  ##           access = "proxy";
  ##           url = "http://127.0.0.1:${toString config.services.tempo.settings.server.http_listen_port}";
  ##         }
  ##         {
  ##           name = "Mimir";
  ##           type = "prometheus";
  ##           access = "proxy";
  ##           url = "http://127.0.0.1:${toString config.services.mimir.configuration.server.http_listen_port}/prometheus";
  ##         }
  ##       ];
  ##       ## add alerting with sops secrets
  ##       # alerting.contactPoints.settings.contactPoints = [
  ##       #   {
  ##       #     orgId = 1;
  ##       #     name = "Telegram";
  ##       #     receivers = [
  ##       #       {
  ##       #         uid = "1";
  ##       #         type = "telegram";
  ##       #         settings = {
  ##       #         };
  ##       #       }
  ##       #     ];
  ##       #   }
  ##       # ];
  ##     };
  ##   };

  ##   alloy = {
  ##     enable = true;
  ##   };

  ##   loki = {
  ##     enable = true;
  ##     configuration = {
  ##       server = {
  ##         http_listen_port = 3100;
  ##         grpc_listen_port = 9096;
  ##       };
  ##       auth_enabled = false;

  ##       # common = {
  ##       #  replication_factor = 1;
  ##       # };

  ##       ingester = {
  ##         lifecycler = {
  ##           address = "127.0.0.1";
  ##           ring = {
  ##             kvstore = {
  ##               store = "inmemory";
  ##             };
  ##             replication_factor = 1;
  ##           };
  ##         };
  ##         chunk_idle_period = "1h";
  ##         max_chunk_age = "1h";
  ##         chunk_target_size = 999999;
  ##         chunk_retain_period = "30s";
  ##       };

  ##       schema_config = {
  ##         configs = [
  ##           {
  ##             from = "2024-07-12";
  ##             store = "tsdb";
  ##             object_store = "filesystem";
  ##             schema = "v13";
  ##             index = {
  ##               prefix = "index_";
  ##               period = "24h";
  ##             };
  ##           }
  ##         ];
  ##       };

  ##       storage_config = {
  ##         tsdb_shipper = {
  ##           active_index_directory = "/var/lib/loki/tsdb-index";
  ##           cache_location = "/var/lib/loki/tsdb-cache";
  ##           cache_ttl = "24h";
  ##         };

  ##         filesystem = {
  ##           directory = "/var/lib/loki/chunks";
  ##         };
  ##       };

  ##       limits_config = {
  ##         reject_old_samples = true;
  ##         reject_old_samples_max_age = "168h";
  ##       };

  ##       table_manager = {
  ##         retention_deletes_enabled = false;
  ##         retention_period = "0s";
  ##       };

  ##       compactor = {
  ##         working_directory = "/var/lib/loki";
  ##         compactor_ring = {
  ##           kvstore = {
  ##             store = "inmemory";
  ##           };
  ##         };
  ##       };
  ##     };
  ##   };

  ##   tempo = {
  ##     enable = true;
  ##     settings = {
  ##       server = {
  ##         http_listen_address = "127.0.0.1";
  ##         http_listen_port = 3200;
  ##         grpc_listen_address = "127.0.0.1";
  ##         grpc_listen_port = 9095;
  ##       };
  ##       distributor.receivers = {
  ##         otlp.protocols = {
  ##           http.endpoint = "127.0.0.1:14318";
  ##           grpc.endpoint = "127.0.0.1:14319";
  ##         };
  ##       };
  ##       storage.trace = {
  ##         backend = "local";
  ##         wal.path = "/var/lib/tempo/wal";
  ##         local.path = "/var/lib/tempo/blocks";
  ##       };
  ##       ingester = {
  ##         trace_idle_period = "30s";
  ##         max_block_bytes = 1000000;
  ##         max_block_duration = "5m";
  ##       };
  ##       compactor = {
  ##         compaction = {
  ##           compaction_window = "1h";
  ##           max_block_bytes = 100000000;
  ##           compacted_block_retention = "10m";
  ##         };
  ##       };
  ##     };
  ##   };

  ##   mimir = {
  ##     enable = true;
  ##     configuration = {
  ##       multitenancy_enabled = false;
  ##       server = {
  ##         http_listen_address = "127.0.0.1";
  ##         http_listen_port = 3300;
  ##         grpc_listen_address = "127.0.0.1";
  ##         grpc_listen_port = 9097;
  ##       };

  ##       common = {
  ##         storage = {
  ##           backend = "filesystem";
  ##           filesystem.dir = "/var/lib/mimir/metrics";
  ##         };
  ##       };

  ##       blocks_storage = {
  ##         backend = "filesystem";
  ##         bucket_store.sync_dir = "/var/lib/mimir/tsdb-sync";
  ##         filesystem.dir = "/var/lib/mimir/data/tsdb";
  ##         tsdb.dir = "/var/lib/mimir/tsdb";
  ##       };

  ##       compactor = {
  ##         data_dir = "/var/lib/mimir/data/compactor";
  ##         sharding_ring.kvstore.store = "memberlist";
  ##       };

  ##       limits = {
  ##         compactor_blocks_retention_period = "90d";
  ##       };

  ##       distributor = {
  ##         ring = {
  ##           instance_addr = "127.0.0.1";
  ##           kvstore.store = "memberlist";
  ##         };
  ##       };

  ##       ingester = {
  ##         ring = {
  ##           instance_addr = "127.0.0.1";
  ##           kvstore.store = "memberlist";
  ##           replication_factor = 1;
  ##         };
  ##       };

  ##       ruler_storage = {
  ##         backend = "filesystem";
  ##         filesystem.dir = "/var/lib/mimir/data/rules";
  ##       };

  ##       store_gateway.sharding_ring.replication_factor = 1;
  ##     };
  ##   };
  ## };

  ## systemd.services.alloy = {
  ##   serviceConfig.TimeoutStopSec = 4;
  ##   reloadTriggers = ["/etc/alloy/client.alloy"];
  ## };

  ## services.telegraf = {
  ##   enable = true;
  ##   extraConfig = {
  ##     inputs = {
  ##       mqtt_consumer = {
  ##         servers = ["tcp://georgesilva.duckdns.org:3883"];
  ##         topics = ["BitDevsMesh/#"];
  ##         data_format = "json";
  ##         username = "nDataMarket";
  ##         password = "BTCppLFG!";
  ##         name_override = "mqtt_raw";
  ##       };
  ##     };
  ##     outputs = {
  ##       postgresql = {
  ##         connection = "host=localhost user=postgres dbname=mercedado sslmode=disable";
  ##         fields_as_jsonb = true;
  ##         schema = "public";
  ##         # create_templates = [
  ##         #   "CREATE TABLE IF NOT EXISTS {{ .table }} (time TIMESTAMP WITH TIME ZONE, topic TEXT, fields JSONB)"
  ##         # ];
  ##       };
  ##     };
  ##   };
  ## };
}
