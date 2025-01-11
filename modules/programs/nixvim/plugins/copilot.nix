{
  programs.nixvim = {
    # plugins.copilot-vim.enable = true;
    plugins.copilot-lua = {
      enable = true;
      settings.suggestion.keymap.accept = "<C-j>";
    };
  };
}
