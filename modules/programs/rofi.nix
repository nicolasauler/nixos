{pkgs, ...}: {
  programs.rofi = {
    enable = true;

    package = pkgs.rofi-wayland;

    extraConfig = {
      show-icons = true;
      auto-select = true;
    };

    theme = "${pkgs.rofi-wayland}/share/rofi/themes/gruvbox-dark-hard.rasi";
    # theme = "${pkgs.rofi-wayland}/share/rofi/themes/gruvbox-dark-hard.rasi";
  };
}
