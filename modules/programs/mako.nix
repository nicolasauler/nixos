{config, ...}: {
  services.mako = {
    enable = true;
    defaultTimeout = 4000;
    backgroundColor = "#${config.colorScheme.palette.base01}";
    borderColor = "#${config.colorScheme.palette.base0E}";
    # backgroundColor = "#${config.colorScheme.palette.base01}";
    # borderColor = "#${config.colorScheme.palette.base0E}";
    borderRadius = 5;
    borderSize = 2;
    textColor = "#${config.colorScheme.palette.base04}";
    # textColor = "#${config.colorScheme.palette.base04}";
    layer = "overlay";
  };
}
