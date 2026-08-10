# home-manager instantiates its own nixpkgs (useGlobalPkgs is not set), so the
# allowUnfree in flake.nix covers system packages but not anything home-manager
# installs.
{lib, ...}: {
  # Allow-list rather than a blanket allowUnfree, so a new unfree dependency has to be
  # added here deliberately instead of slipping in. cpptools is needed for the cppdbg
  # adapter that drives rr — see modules/programs/nixvim/plugins/debug.nix.
  nixpkgs.config.allowUnfreePredicate = pkg:
    builtins.elem (lib.getName pkg) [
      "vscode-extension-ms-vscode-cpptools"
    ];

  # nixpkgs.overlays = [
  #   inputs.neovim-nightly-overlay.overlay
  # ];
}
