{...}: {
  programs.alacritty = {
    enable = true;

    settings = {
      font = {
        size = 16.0;
        normal = {
          family = "InconsolataGo Nerd Font";
          style = "Bold";
        };
      };

      # set in zellij
      # terminal.shell.program = "nu";
    };
  };
}
