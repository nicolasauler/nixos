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

    keymaps = [
      {
        mode = "n";
        key = "]t";
        options.silent = true;
        action = "require('todo-comments').jump_next()";
      }
      {
        mode = "n";
        key = "[t";
        options.silent = true;
        action = "require('todo-comments').jump_prev()";
      }
    ];

    plugins.which-key.enable = true;
  };
}
