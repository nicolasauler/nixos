{pkgs, ...}: {
  programs.nixvim = {
    plugins.conform-nvim = {
      enable = true;
      formatOnSave = {
        lspFallback = true;
        timeoutMs = 500;
      };
      notifyOnError = true;
      formattersByFt = {
        c = ["clang-format"];
        cpp = ["clang-format"];
        json = [["prettierd" "prettier"]];
        lua = ["stylua "];
        markdown = [["prettierd" "prettier"]];
        nix = [["alejandra" "nixfmt" "nixpkgs_fmt"]];
        python = ["isort" "black"];
        rust = ["rustfmt"];
        sh = ["shfmt "];
        sql = [["pg_format" "sql_formatter" "sqlfluff"]];
        yaml = ["prettierd"];
      };
    };

    extraConfigLuaPost = ''
      vim.o.formatexpr = "v:lua.require'conform'.formatexpr()"
    '';

    extraPackages = with pkgs; [
      alejandra
      black
      clang-tools
      isort
      nixfmt-rfc-style
      nixpkgs-fmt
      pgformatter
      prettierd
      shfmt
      sqlfluff
      stylua
    ];

    keymaps = [
      {
        key = "<leader>cf";
        action.__raw = "function() require('conform').format() end";
        mode = ["n" "v"];
        options = {
          silent = true;
          noremap = true;
          desc = "[C]onform: [F]ormat current buffer";
        };
      }
    ];
  };
}
