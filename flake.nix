{
  description = "Nixos config flake";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";

    home-manager = {
      url = "github:nix-community/home-manager";
      inputs.nixpkgs.follows = "nixpkgs";
    };

    nix-colors.url = "github:misterio77/nix-colors";
    # stylix.url = "github:danth/stylix";

    hyprland.url = "github:hyprwm/Hyprland";

    nixvim = {
      url = "github:nix-community/nixvim";
      # If using a stable channel you can use `url = "github:nix-community/nixvim/nixos-<version>"`
      inputs.nixpkgs.follows = "nixpkgs";
    };

    zig = {
      url = "github:mitchellh/zig-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    zls-overlay = {
      url = "github:zigtools/zls";
      inputs.nixpkgs.follows = "nixpkgs";
    };

    # sops-nix.url = "github:Mic92/sops-nix";
    # agenix.url = "github:ryantm/agenix";

    sentinelone.url = "github:devusb/sentinelone-nix";
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
