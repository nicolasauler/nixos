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
  # SentinelOne's agent is proprietary EDR. The dispositive licensing fact is the
  # customer agreement: it licenses us to USE the agent, not to redistribute it.
  # So the .deb is not vendored in this public repo, and nixpkgs does not package
  # it either.
  #
  # `requireFile` is nixpkgs' mechanism for exactly that situation: a
  # fixed-output derivation keyed on name + hash, which resolves if the file is
  # already in the store and otherwise fails the BUILD with instructions. Three
  # properties matter here, and each replaces something worse:
  #
  #   pure       — no absolute path and no `builtins.getEnv`, so this host
  #                evaluates purely. That is what lets CI check it, and it means
  #                `--impure` is no longer needed to switch this machine at all.
  #   fail-CLOSED — a missing .deb is a loud build error. The previous design
  #                took the path from the environment, so forgetting the variable
  #                silently produced a system with NO EDR agent: no warning, no
  #                assertion, nothing in the diff. `sudo`'s env_reset made that a
  #                live trap, since this host's env_keep holds only
  #                NIXOS_NO_CHECK and TERMINFO*.
  #   hash-pinned — `src = /some/path` verified nothing. A swapped, truncated or
  #                tampered agent binary is now detected.
  #
  # One-time setup per machine, and again whenever the agent version bumps.
  # BOTH commands matter — the second one is not optional:
  #
  #   nix-store --add-fixed sha256 /home/nic/bipa/SentinelAgent_linux_x86_64_v25_2_1_20.deb
  #   nix build --out-link ~/.cache/gcroots/sentinelone-deb '/home/nic/nixos#nixosConfigurations.precision.config.services.sentinelone.package.src'
  #
  # Each command is ONE line, with no shell variable, no command substitution and
  # no line continuation. That is not fussiness — this file has now broken twice
  # on exactly that:
  #   - `P=$(...)` is a syntax error in nushell, the shell this host's owner uses;
  #   - a trailing `\` is NOT a line continuation in nushell either. It is passed
  #     as a literal argument and the next line is parsed as a separate command,
  #     so the two-line spelling failed with `--add-fixed` already done and the
  #     GC root skipped — the precise state these two commands exist to prevent.
  # Both were measured on nu 0.115.0; `~` expansion in an argument position is
  # fine, so the forms above work in nushell, bash, zsh and fish alike.
  #
  # The second command deliberately asks the FLAKE for the path rather than
  # quoting a store path literally. A literal would be correct — a flat
  # fixed-output path is `base32(compress(sha256("output:out:sha256:" +
  # hex(sha256("fixed:out:sha256:<hex>:")) + ":" + storeDir + ":" + name)))`, so it
  # is identical on every machine using /nix/store — but it would be UNCOUPLED
  # from the name and hash below. Bump the version, update those two and forget
  # the literal, and command 1 adds the new .deb unrooted while command 2 happily
  # re-roots the old one and exits 0. `--out-link` cannot go stale, and it
  # registers a real GC root (it shows up in `nix-store --gc --print-roots`).
  #
  # then switch normally — no --impure, no environment variables:
  #
  #   nh os switch -a
  #
  # Why the GC root: `--add-fixed` registers a store path with NO root, and a
  # build input is not part of a system's runtime closure, so garbage collection
  # deletes it. This host GCs on a schedule (`programs.nh.clean`, weekly,
  # `--keep-since 7d --keep 5`), so without a root the pin dies within a couple of
  # weeks and the next `nh os switch` fails until it is re-added. The running
  # system is never affected — it fails closed at build time — but it is a
  # recurring papercut, and the root removes it permanently.
  #
  # Two measured traps, hence the exact spelling above:
  #   - `nix-store --add-fixed --add-root LINK sha256 FILE` SILENTLY IGNORES
  #     --add-root: exit 0, prints the path, creates no symlink. It has to be two
  #     commands.
  #   - `requireFile` defaults to `hashMode = "flat"`. The modern `nix store add`
  #     defaults to NAR/recursive and produces a DIFFERENT hash under the same
  #     store name, so it will not satisfy this. Use `nix-store --add-fixed
  #     sha256` as written, or `nix store add --mode flat`.
  #
  # Rejected alternative: `system.extraDependencies = [sentinelOneDeb]` also
  # survives GC (measured — it becomes a direct reference of the system), but it
  # pulls a 58 MB proprietary binary into the closure of every generation. Since
  # not redistributing that binary is the whole reason for this design, and a
  # binary cache is on the roadmap, keeping it out of the closure is worth one
  # extra setup command.
  #
  # CI evaluates this block like any other. It does not BUILD it, so the .deb
  # never needs to exist on a runner.
  sentinelOneDeb = pkgs.requireFile {
    name = "SentinelAgent_linux_x86_64_v25_2_1_20.deb";
    hash = "sha256-fbyo1z5gknlpVZ5qCJ2CNMPuCs4F/HVChnIFZY+RpUQ=";
    message = ''
      SentinelOne's agent cannot be fetched automatically: it is proprietary
      software under a customer agreement that does not permit redistribution.

      Obtain SentinelAgent_linux_x86_64_v25_2_1_20.deb from the internal IT
      distribution point, then add it to the store AND give it a GC root -
      without the root, the next garbage collection deletes it and you will land
      back here:

        nix-store --add-fixed sha256 /path/to/SentinelAgent_linux_x86_64_v25_2_1_20.deb
        nix build --out-link ~/.cache/gcroots/sentinelone-deb '/home/nic/nixos#nixosConfigurations.precision.config.services.sentinelone.package.src'

      Two commands, each on ONE line. Do not reflow them onto two with a trailing
      backslash: that is not a line continuation in nushell, and the failure is
      quiet in the worst way - the add succeeds and the GC root is skipped, which
      is how you would end up reading this message again in a fortnight.

      Two more traps. `nix store add` defaults to NAR/recursive hashing and
      produces a different path that will NOT satisfy this derivation - use
      `--add-fixed sha256` as above, or `nix store add --mode flat`. And passing
      --add-root to --add-fixed is silently ignored, which is why the root comes
      from the second command instead.

      If you are already sure the file is in the store, the hash did not match:
      the file is not the version this host pins. Do not "fix" that by editing
      the hash without finding out why it differs.
    '';
  };

  # Deliberately a STRING, not a Nix path. `types.path` accepts an absolute
  # string and interpolates it literally; a path VALUE is copied into the Nix
  # store, where every uid on this machine can read it — which is what was
  # happening to this token (it sat at
  # /nix/store/3vqb5jmmba0mz1wrxwpkr1knvhgn27k8-sentinel_one_token, confirmed
  # readable by an unprivileged process). Upstream's README prescribes the string
  # form for exactly this reason.
  #
  # The string costs one property the path had, and it has to be bought back
  # below: a path VALUE gets copied into the store, so a MISSING file was a hard
  # eval error. A string is checked by nothing — not eval, not build, not
  # activation. And upstream's init script reads the token only on first
  # initialisation, has no `set -e`, and pipes it through `jq`, so with the file
  # absent it writes an empty S1_AGENT_MANAGEMENT_TOKEN and a malformed
  # basic.conf and still exits 0; the unit is Type=oneshot, so activation would
  # SUCCEED with an unenrolled agent. That is the same silent-no-EDR outcome the
  # requireFile change removes for the .deb, so it gets an explicit check in
  # `system.activationScripts` below.
  #
  # It cannot be an eval-time assertion: `builtins.pathExists` returns false
  # under pure evaluation rather than erroring, so an assertion would fail the
  # required CI check on every PR. Activation is the only correct place.
  #
  # This is an exposure fix, not secret management. The token is still a
  # plaintext file, and the upstream module separately writes it to
  # /var/lib/sentinelone/configuration/install_config and then `chmod -R 0755`s
  # it (module.nix:24-25,47-48) — which nothing here, and not agenix either, can
  # prevent. Rotate it: it is world-readable in this machine's live closure right
  # now, at /nix/store/3vqb5jmmba0mz1wrxwpkr1knvhgn27k8-sentinel_one_token.
  sentinelOneToken = "/home/nic/bipa/sentinel_one_token";
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

  # Buys back the existence check the string token gave up (see the note on
  # sentinelOneToken above). Fails the switch rather than letting the agent come
  # up unenrolled: upstream's init script tolerates a missing token, writes an
  # empty one, and exits 0, so without this a fresh install — or the token
  # rotation this file tells you to do — would silently produce a host with no
  # working EDR. `-s` covers absent AND empty, since an empty file is exactly
  # what a half-finished rotation leaves behind.
  system.activationScripts.sentinelOneToken.text = ''
    if [ ! -s ${sentinelOneToken} ]; then
      echo "sentinelone: ${sentinelOneToken} is missing or empty." >&2
      echo "  The agent would enrol with an empty management token and report" >&2
      echo "  healthy while protecting nothing. Restore the file, then switch." >&2
      exit 1
    fi
  '';

  services.sentinelone = {
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
  # Not gated on anything: `requireFile` keeps this pure, so the agent is
  # configured unconditionally and its absence can no longer be the silent
  # default. (An earlier version gated both this and `services.sentinelone` on an
  # environment variable. Worth recording why that was subtly wrong even before
  # the fail-open argument: `lib.mkIf` has to sit on `systemd.services`, not on
  # `systemd.services.sentinelone.serviceConfig` — `systemd.services` is an
  # `attrsOf` submodule, so naming the element at all instantiates it, and
  # `serviceConfig = mkIf false {…}` still rendered a stray ExecStart-less
  # `sentinelone.service` into the closure.)
  systemd.services.sentinelone.serviceConfig = {
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
      # Our own cache, for what this repo builds: the VM checks and host
      # closures. PUBLIC on purpose — everything in this repo is public. Nothing
      # from bipa or certus-infra belongs here, which is why pushing is an
      # explicit per-command action and never a post-build-hook (see below).
      "https://nicnixos.cachix.org"
    ];
    trusted-public-keys = [
      "cuda-maintainers.cachix.org-1:0dq3bujKpuEPMCX6U4WylrUDZ9JyUG0VpVZa7CNfq5E="
      "hyprland.cachix.org-1:a7pgxzMz7+chwVL3/pzj6jIBMioiJM7ypFP8PwtkuGc="
      "devenv.cachix.org-1:w1cLUi8dv3hnoSPGAuibQv+f9TZLr6cv/Hm9XgU50cw="
      "nicnixos.cachix.org-1:360nRdjlB+ydcwCGF3V17ojXcvyWqz/SJ3hTarX6Pqs="
    ];
  };

  # Pushing to nicnixos is deliberately NOT a post-build-hook. This machine also
  # builds bipa, a private employer monorepo, and a blanket hook would publish its
  # artifacts to a public cache. Push explicitly, scoped to the command:
  #
  #   cachix authtoken <write-token>        # once; writes ~/.config/cachix/cachix.dhall (0600)
  #   cachix watch-exec nicnixos -- nix build .#checks.x86_64-linux.buildbot-fanout
  #
  # `watch-exec` pushes only what that one command realises, so there is no way to
  # sweep something private in by accident. Both lines are single-line and
  # substitution-free, so they work in nushell as written.
}
