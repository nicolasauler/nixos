{
  programs.nixvim = {
    plugins.lualine = {
      enable = true;
      # https://nix-community.github.io/nixvim/plugins/lualine/sections/lualine_c/index.html
      #sections.lualine_c.extraConfig = ''
      # path = 1 (show relative path)
      #'';
    };
  };
}
