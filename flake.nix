{
  description = "Nixos config flake";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";

    home-manager = {
      url = "github:nix-community/home-manager";
      inputs.nixpkgs.follows = "nixpkgs";
    };

    nix-colors.url = "github:misterio77/nix-colors";

    hyprland.url = "git+https://github.com/hyprwm/Hyprland?submodules=1";

    # neovim-nightly-overlay.url = "github:nix-community/neovim-nightly-overlay";
    nixvim.url = "github:nix-community/nixvim";
  };

  # outputs = { self, nixpkgs, neovim-nightly-overlay, ... }@inputs:
  outputs = {
    self,
    nixpkgs,
    ...
  } @ inputs: let
    system = "x86_64-linux";
    pkgs = import nixpkgs {
      inherit system;
      # overlays = [ neovim-nightly-overlay.overlay ];
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
          ./hosts/desktop/configuration.nix
        ];
      };
      xps_bipa = nixpkgs.lib.nixosSystem {
        inherit pkgs;
        specialArgs = {inherit inputs;};
        modules = [
          ./hosts/xps_bipa/configuration.nix
        ];
      };
    };
  };
}
