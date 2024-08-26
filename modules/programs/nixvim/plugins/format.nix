{pkgs, ...}: {
  programs.nixvim = {
    plugins.conform-nvim = {
      enable = true;
      settings = {
        format_on_save = {
          lsp_format = "fallback";
          timeout_ms = 500;
        };

        notify_on_error = true;

        formatters_by_ft = {
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
