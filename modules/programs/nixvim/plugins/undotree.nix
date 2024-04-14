{
  programs.nixvim = {
    plugins.undotree = {
      enable = true;
    };

    keymaps = [
      #{
      #  key = ";";
      #  action = ":";
      #}
      {
        mode = "n";
        key = "<leader>u";
        options.silent = false;
        action = "<cmd>UndotreeToggle<CR>";
      }
    ];

  };
}
