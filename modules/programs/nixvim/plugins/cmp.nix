{
  programs.nixvim = {
    plugins = {
      luasnip.enable = true;
      cmp-nvim-lsp.enable = true;
      cmp-nvim-lua.enable = true;
      cmp-buffer.enable = true;
      cmp-path.enable = true;
      cmp-cmdline.enable = true;
    };

    plugins.cmp = {
      enable = true;
    };
  };
}
