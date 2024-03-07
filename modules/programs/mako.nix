{ pkgs, config, ... }:

{
  services.mako {
  enable = true;
  backgroundColor = "#${config.colorScheme.colors.base01}";
  borderColor = "#${config.colorScheme.colors.base0E}";
  borderRadius = 5;
  borderSize = 2;
  textColor = "#${config.colorScheme.colors.base04}";
  layer = "overlay";
};
}
