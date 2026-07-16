{inputs, ...}: {
  imports = [inputs.nix-index-database.homeModules.nix-index];

  programs.nix-index = {
    enable = true;
    enableBashIntegration = true;
  };

  programs.nix-index-database.comma.enable = true;
}
