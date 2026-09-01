{
  description = "Nixos config flake";

  # For CI only, in practice. The workflow passes `accept-flake-config = true` to
  # install-nix-action, so a runner picks this up with no extra step.
  #
  # It does NOT take effect locally: nix asks per-flake before trusting a
  # `nixConfig`, and being in `trusted-users` does not waive that — measured,
  # `nix develop --command nix config show substituters` shows no nicnixos and
  # prints "Pass '--accept-flake-config' to trust it". Rather than granting
  # `accept-flake-config` machine-wide (which would trust ANY flake's substituters
  # — a supply-chain footgun), the devShell below exports `NIX_CONFIG` itself, so
  # the cache is live exactly inside this repo's shell and nowhere else.
  #
  # PULL only. Pushing needs a write token and stays explicit: cachix-action in
  # CI, `cachix watch-exec` locally.
  nixConfig = {
    extra-substituters = ["https://nicnixos.cachix.org"];
    extra-trusted-public-keys = [
      "nicnixos.cachix.org-1:360nRdjlB+ydcwCGF3V17ojXcvyWqz/SJ3hTarX6Pqs="
    ];
  };

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";

    home-manager = {
      url = "github:nix-community/home-manager";
      inputs.nixpkgs.follows = "nixpkgs";
    };

    nix-index-database = {
      url = "github:nix-community/nix-index-database";
      inputs.nixpkgs.follows = "nixpkgs";
    };

    nix-colors.url = "github:misterio77/nix-colors";
    # stylix.url = "github:danth/stylix";

    hyprland.url = "github:hyprwm/Hyprland";

    nixvim = {
      url = "github:nix-community/nixvim";
      # If using a stable channel you can use `url = "github:nix-community/nixvim/nixos-<version>"`
      # no nixpkgs follows: nixvim recommends building against its own tested nixpkgs pin
    };

    # sops-nix.url = "github:Mic92/sops-nix";
    # agenix.url = "github:ryantm/agenix";

    # Pinned: 54f75733 (2026-08-10) moved the bind mounts from fileSystems to
    # systemd.mounts with After=sentinelone-init.service. That service is ordered
    # after sysinit.target, which is after local-fs.target, which pulls in the mount
    # itself -- so activation dies with "Transaction order is cyclic" and systemd
    # drops local-fs.target. Unpin once upstream breaks that loop.
    sentinelone.url = "github:devusb/sentinelone-nix/1e58fdcd464ef0b3929c5fd181c837d2d8eaf0d3";

    certus-infra = {
      url = "git+ssh://git@github.com/nicolasauler/certus_infra.git";
      # One source of truth: the private module and the public VM check use the
      # same buildbot-nix revision and API.
      inputs.buildbot-nix.follows = "buildbot-nix";
    };

    # Public direct input: checks.buildbot-fanout can evaluate without fetching
    # certus-infra. Keep the revision explicit; follows above gives the private
    # module this exact source too.
    buildbot-nix = {
      url = "github:nix-community/buildbot-nix/19a89fd4c890433dab7062672ff95efe0128db3c";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    # certus-infra.url = "git+file:///home/nic/certus/certus_infra.git/main";
  };

  outputs = {
    self,
    nixpkgs,
    ...
  } @ inputs: let
    system = "x86_64-linux";
    pkgs = import nixpkgs {
      inherit system;
      overlays = [];
      config = {
        allowUnfree = true;
        # cudaSupport = true;
      };
    };
  in {
    # Replaces the old shell.nix, which did `import <nixpkgs> {}` — the channel,
    # so it was unpinned and impure and had nothing to do with the nixpkgs these
    # hosts are actually built from. This shell uses the locked input.
    #
    # It is also the right scope for the cache: entering this shell (or running
    # any `nix` command against this flake) picks up `nixConfig` above, so
    # nicnixos is consulted exactly when it can help and never adds a lookup to
    # unrelated `nix build`s on the machine.
    devShells.${system}.default = pkgs.mkShell {
      packages = with pkgs; [
        alejandra # formatter this repo is written with
        python3 # the VM tests' testScript is Python
        cachix # push this repo's builds: cachix watch-exec nicnixos -- nix build ...
        actionlint # .github/workflows/ci.yaml is a required-check surface
      ];

      # This — not a machine-wide substituter — is what makes the cache live
      # locally. It applies to `nix` commands run inside this shell, which direnv
      # enters on cd, and to nothing else: nicnixos only ever holds what this repo
      # builds, so putting it in `nix.settings` would add a pointless lookup to
      # every unrelated `nix build` on the machine. Measured: with this set,
      # `nix config show substituters` lists nicnixos; outside the shell it does
      # not. Honoured because `nic` is in `trusted-users`; substituters from an
      # untrusted user are ignored.
      NIX_CONFIG = ''
        extra-substituters = https://nicnixos.cachix.org
        extra-trusted-public-keys = nicnixos.cachix.org-1:360nRdjlB+ydcwCGF3V17ojXcvyWqz/SJ3hTarX6Pqs=
      '';
    };

    checks.${system} = {
      buildbot-workstation = import ./tests/buildbot-workstation.nix {
        inherit pkgs inputs;
      };
      buildbot-fanout = import ./tests/buildbot-fanout.nix {
        inherit pkgs inputs;
      };
    };

    nixosConfigurations = {
      notebook = nixpkgs.lib.nixosSystem {
        inherit pkgs;
        specialArgs = {inherit inputs;};
        modules = [
          ./hosts/notebook/configuration.nix
        ];
      };
      desktop = nixpkgs.lib.nixosSystem {
        inherit pkgs;
        specialArgs = {inherit inputs;};
        modules = [
          # inputs.stylix.nixosModules.stylix
          inputs.certus-infra.nixosModules.observability
          inputs.certus-infra.nixosModules.buildbot
          # bencher: disabled until certus-infra bumps it — its importCargoLock
          # fetches async-stripe-1.0.0-rc.3 from the legacy crates.io download
          # URL, which now returns 403 and blocks every desktop rebuild.
          # inputs.certus-infra.nixosModules.bencher
          ./hosts/desktop/configuration.nix
        ];
      };
      xpsbipa = nixpkgs.lib.nixosSystem {
        inherit pkgs;
        specialArgs = {inherit inputs;};
        modules = [
          ./hosts/xpsbipa/configuration.nix
        ];
      };
      precision = nixpkgs.lib.nixosSystem {
        inherit pkgs;
        specialArgs = {inherit inputs;};
        modules = [
          ./hosts/precision/configuration.nix
        ];
      };
    };
  };
}
