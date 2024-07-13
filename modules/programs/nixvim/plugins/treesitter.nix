{
  programs.nixvim = {
    plugins.treesitter = {
      enable = true;
      folding = false;
      nixGrammars = true;
      settings = {
        indent.enable = true;
        ensure_installed = [
          "c"
          "lua"
          "vim"
          "vimdoc"
          "query"
          "rust"
          "bash"
          "json"
          "yaml"
          "toml"
          "python"
          "go"
          "http"
        ];
        ignore_install = [
          "javascript"
        ];
        highlight.disable = [];
      };
    };

    plugins.treesitter-context.enable = true;
    plugins.treesitter-refactor.enable = true;
    plugins.treesitter-textobjects = {
      enable = true;
      lspInterop = {
        enable = true;
        peekDefinitionCode = {
          "<leader>df" = "@function.outer";
          "<leader>dF" = "@class.outer";
        };
      };
      select = {
        enable = true;
        lookahead = true;
        includeSurroundingWhitespace = true;

        selectionModes = {
          "@parameter.outer" = "v"; # charwise
          "@function.outer" = "V"; # linewise
          "@class.outer" = "<c-v>"; # blockwise
        };

        keymaps = {
          # You can use the capture groups defined in textobjects.scm
          "af" = "@function.outer";
          "if" = "@function.inner";
          "ac" = "@class.outer";
          # You can optionally set descriptions to the mappings (used in the desc parameter of
          # nvim_buf_set_keymap) which plugins like which-key display
          "ic" = {
            query = "@class.inner";
            desc = "Select inner part of a class region";
          };
          # You can also use captures from other query groups like `locals.scm`
          "as" = {
            query = "@scope";
            queryGroup = "locals";
            desc = "Select language scope";
          };
        };
      };
    };
  };
}
