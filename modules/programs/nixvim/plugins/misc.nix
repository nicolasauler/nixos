{
  programs.nixvim = {
    plugins.crates-nvim.enable = true;

    plugins.todo-comments = {
      enable = true;

      keymaps = {
        # todoLocList.key = "<leader>lt";
        # todoQuickFix.key = "<leader>lt";
        # todoTelescope.key = "<leader>lt";
        todoTrouble.key = "<leader>tc";
      };
    };

    plugins.which-key.enable = true;

    keymaps = [
      {
        mode = "n";
        key = "]t";
        options.silent = true;
        action = ''
          function() require('todo-comments').jump_next() end
        '';
      }
      {
        mode = "n";
        key = "[t";
        options.silent = true;
        action = ''
          function() require('todo-comments').jump_prev() end
        '';
      }
      {
        mode = "n";
        key = "<leader>wk";
        action = "<cmd>WhichKey<CR>";
        options.desc = "[W]hich [K]ey";
      }
    ];
  };
}
