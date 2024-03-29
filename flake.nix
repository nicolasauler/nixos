{
  description = "Nixos config flake";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";

    home-manager = {
      url = "github:nix-community/home-manager";
      inputs.nixpkgs.follows = "nixpkgs";
    };

    nix-colors.url = "github:misterio77/nix-colors";

    hyprland.url = "github:hyprwm/Hyprland";

    neovim-nightly-overlay.url = "github:nix-community/neovim-nightly-overlay";

  };

  outputs = { self, nixpkgs, neovim-nightly-overlay, ... }@inputs:
    let
      system = "x86_64-linux";
      pkgs = import nixpkgs {
        inherit system;
        overlays = [ neovim-nightly-overlay.overlay ];
        config.allowUnfree = true;
      };
    in
    {
      nixosConfigurations = {
        notebook = nixpkgs.lib.nixosSystem {
          inherit pkgs;
          specialArgs = { inherit inputs; };
          modules = [
            ./hosts/notebook/configuration.nix
          ];
        };
        desktop = nixpkgs.lib.nixosSystem {
          specialArgs = { inherit inputs; };
          modules = [
            ./hosts/desktop/configuration.nix
          ];
        };
      };
    };
}
