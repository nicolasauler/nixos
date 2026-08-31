{
  description = "Nixos config flake";

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

    certus-infra.url = "git+ssh://git@github.com/nicolasauler/certus_infra.git";

    # Same rev certus-infra pins, taken directly from the PUBLIC upstream so
    # that checks.buildbot-fanout does not reach buildbot-nix *through* the
    # private certus-infra input. That is what makes it runnable in this public
    # repo's CI without a deploy key.
    buildbot-nix.url = "github:nix-community/buildbot-nix/19a89fd4c890433dab7062672ff95efe0128db3c";
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
