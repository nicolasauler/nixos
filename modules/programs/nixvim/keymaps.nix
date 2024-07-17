{
  programs.nixvim = {
    keymaps = [
      {
        mode = "n";
        key = "<leader>ls";
        options.silent = true;
        action = "<cmd>Oil<CR>";
      }
      {
        mode = "v";
        key = "J";
        options.silent = true;
        action = ":m '>+1<CR>gv=gv";
      }
      {
        mode = "v";
        key = "K";
        options.silent = true;
        action = ":m '<-2<CR>gv=gv";
      }
      {
        mode = "n";
        key = "J";
        options.silent = true;
        action = "mzJ'z";
      }
      {
        mode = "n";
        key = "<C-d>";
        options.silent = true;
        action = "<C-d>zz";
      }
      {
        mode = "n";
        key = "<C-u>";
        options.silent = true;
        action = "<C-u>zz";
      }
      {
        mode = "n";
        key = "n";
        options.silent = true;
        action = "nzzzv";
      }
      {
        mode = "n";
        key = "N";
        options.silent = true;
        action = "Nzzzv";
      }
      {
        mode = "n";
        key = "<leader>y";
        options.silent = true;
        action = "\"+y";
      }
      {
        mode = "v";
        key = "<leader>y";
        options.silent = true;
        action = "\"+y";
      }
      {
        mode = "n";
        key = "<leader>Y";
        options.silent = true;
        action = "\"+Y";
      }
      {
        mode = "n";
        key = "<leader>d";
        options.silent = true;
        action = "\"_d";
      }
      {
        mode = "v";
        key = "<leader>d";
        options.silent = true;
        action = "\"_d";
      }
      {
        mode = "n";
        key = "Q";
        options.silent = true;
        action = "<nop>";
      }
      {
        mode = "n";
        key = "<C-k>";
        options.silent = true;
        action = "<cmd>cnext<CR>zz";
      }
      {
        mode = "n";
        key = "<C-j>";
        options.silent = true;
        action = "<cmd>cprev<CR>zz";
      }
      {
        mode = "n";
        key = "<leader>k";
        options.silent = true;
        action = "<cmd>lnext<CR>zz";
      }
      {
        mode = "n";
        key = "<leader>j";
        options.silent = true;
        action = "<cmd>lprev<CR>zz";
      }
      {
        mode = "n";
        key = "<leader>ss";
        options = {
          desc = "[S]ubstitute [S]tring in buffer";
          silent = true;
        };
        action = "[[:%s/\<<C-r><C-w>\>/<C-r><C-w>/gI<Left><Left><Left>]]";
      }
      {
        mode = "n";
        key = "<leader>x";
        options.silent = true;
        action = "<cmd>!chmod +x %<CR>";
      }
    ];
  };
}
