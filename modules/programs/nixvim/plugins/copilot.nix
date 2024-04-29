{
  programs.nixvim = {
    # plugins.copilot-vim.enable = true;
    plugins.copilot-lua = {
      enable = true;
      suggestion.keymap.accept = "<C-j>";
    };
  };
}
