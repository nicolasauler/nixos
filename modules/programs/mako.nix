{config, ...}: {
  services.mako = {
    enable = true;
    settings = {
      default-timeout = 4000;
      background-color = "#${config.colorScheme.palette.base01}";
      border-color = "#${config.colorScheme.palette.base0E}";
      # backgroundColor = "#${config.colorScheme.palette.base01}";
      # borderColor = "#${config.colorScheme.palette.base0E}";
      border-radius = 5;
      border-size = 2;
      text-color = "#${config.colorScheme.palette.base04}";
      # textColor = "#${config.colorScheme.palette.base04}";
      layer = "overlay";
    };
  };
}
