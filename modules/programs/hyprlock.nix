{...}: {
  # needs security.pam.services.hyprlock = {}; on the NixOS side to unlock
  programs.hyprlock = {
    enable = true;
    settings = {
      general = {
        hide_cursor = true;
        ignore_empty_input = true;
      };

      background = [
        {
          monitor = "";
          color = "rgba(0, 0, 0, 1.0)";
        }
      ];

      input-field = [
        {
          monitor = "";
          fade_on_empty = true;
          placeholder_text = "<i>Input Password...</i>";
        }
      ];
    };
  };
}
