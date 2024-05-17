{ ... }:
{
  programs.zellij = {
    enable = true;

    settings = {
      default_layout = "compact";
      default_mode = "locked";
      keybinds = {
        unbind = ["Ctrl q"];
        # "shared_except \"locked\"" = {
        #   "bind \"Ctrl q\"" = { Quit = { }; };
        # };
      };
    };
  };
}
