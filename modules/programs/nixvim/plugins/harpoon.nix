{
  programs.nixvim = {
    plugins.harpoon = {
      enable = true;
      keymapsSilent = true;

      keymaps = {
        addFile = "<leader>a";
        toggleQuickMenu = "<C-e>";
        navFile = {
          "1" = "<C-h>";
          "2" = "<C-n>";
          "3" = "<C-m>";
          "4" = "<C-i>";
        };
      };
    };
  };
}
