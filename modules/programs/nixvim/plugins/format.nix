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
          json = {
            __unkeyed-1 = "prettierd";
            __unkeyed-2 = "prettier";
            stop_after_first = true;
          };
          lua = ["stylua "];
          markdown = {
            __unkeyed-1 = "prettierd";
            __unkeyed-2 = "prettier";
            stop_after_first = true;
          };
          nix = {
            __unkeyed-1 = "alejandra";
            __unkeyed-2 = "nixfmt";
            __unkeyed-3 = "nixpkgs_fmt";
            stop_after_first = true;
          };
          python = ["isort" "black"];
          rust = ["rustfmt"];
          sh = ["shfmt "];
          sql = {
            __unkeyed-1 = "pg_format";
            __unkeyed-2 = "sql_formatter";
            __unkeyed-3 = "sqlfluff";
            stop_after_first = true;
          };
          # yaml = ["prettierd"];
          yaml = ["yamlfmt"];
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
      yamlfmt
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
