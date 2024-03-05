{ pkgs, ... }:
{
  programs.rofi-wayland = {
    enable = true;

    extraConfig = {
      show-icons = true;
      auto-select = true;
    };

    theme = "${pkgs.rofi-wayland}/share/rofi/themes/gruvbox-dark-hard.rasi";
  };
}
