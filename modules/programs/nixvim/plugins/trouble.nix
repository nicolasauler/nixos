{
  programs.nixvim = {
    plugins.trouble = {
      enable = true;
    };

    keymaps = [
      {
        mode = "n";
        key = "<leader>tx";
        options.silent = true;
        action = "<cmd>TroubleToggle<CR>";
      }
      {
        mode = "n";
        key = "<leader>tw";
        options.silent = true;
        action = "<cmd>TroubleToggle workspace_diagnostics<CR>";
      }
      {
        mode = "n";
        key = "<leader>td";
        options.silent = true;
        action = "<cmd>TroubleToggle document_diagnostics<CR>";
      }
      {
        mode = "n";
        key = "<leader>tl";
        options.silent = true;
        action = "<cmd>TroubleToggle loclist<CR>";
      }
      {
        mode = "n";
        key = "<leader>tq";
        options.silent = true;
        action = "<cmd>TroubleToggle quickfix<CR>";
      }
    ];
  };
}
