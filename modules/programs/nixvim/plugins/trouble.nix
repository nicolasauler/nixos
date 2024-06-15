{
  programs.nixvim = {
    plugins.trouble = {
      enable = true;
    };

    keymaps = [
      {
        mode = "n";
        key = "<leader>tx";
        action = "<cmd>Trouble diagnostics toggle<CR>";
        options = {
          silent = true;
          desc = "[T]rouble diagnostics";
        };
      }
      {
        mode = "n";
        key = "<leader>td";
        action = "<cmd>Trouble diagnostics toogle filter.buf=0<CR>";
        options = {
          silent = true;
          desc = "[T]rouble buffer diagnostic";
        };
      }
      {
        mode = "n";
        key = "<leader>tl";
        action = "<cmd>Trouble loclist toggle<CR>";
        options = {
          silent = true;
          desc = "[T]rouble [L]oclist toggle";
        };
      }
      {
        mode = "n";
        key = "<leader>tq";
        action = "<cmd>Trouble qflist toggle<CR>";
        options = {
          silent = true;
          desc = "[T]rouble [Q]uickfix list toggle";
        };
      }
      {
        mode = "n";
        key = "<leader>ts";
        action = "<cmd>Trouble symbols toggle focus=false<CR>";
        options = {
          silent = true;
          desc = "[T]rouble [S]ymbols";
        };
      }
      {
        mode = "n";
        key = "<leader>tr";
        action = "<cmd>Trouble lsp toggle focus=false win.position=right<CR>";
        options = {
          silent = true;
          desc = "[T]rouble lsp definitions / [R]eferences";
        };
      }
    ];
  };
}
