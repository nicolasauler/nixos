{...}: {
  programs.zellij = {
    enable = true;

    settings = {
      # copy_command = "wl-copy";
      default_layout = "compact";
      default_mode = "locked";
      default_shell = "nu";
      keybinds = {
        unbind = ["Ctrl q"];
        # "shared_except \"locked\"" = {
        #   "bind \"Ctrl q\"" = { Quit = { }; };
        # };
      };
    };
  };
}
