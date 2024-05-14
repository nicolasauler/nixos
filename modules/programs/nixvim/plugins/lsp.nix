{
  programs.nixvim = {
    plugins.lsp = {
      enable = true;
      servers = {
        bashls.enable = true;
        clangd.enable = true;
        lua-ls.enable = true;
        nil_ls = {
          enable = true;
          settings.formatting.command = [ "nixpkgs-fmt" ];
        };
        pyright.enable = true;
        # rust-analyzer = {
        #   enable = true;
        #   installCargo = false;
        #   installRustc = false;
        # };
        svelte.enable = true;
        tsserver.enable = true;
        yamlls.enable = true;
      };

      keymaps = {
        diagnostic = {
          "[d" = {
            action = "goto_prev";
            desc = "Go to prev diagnostic";
          };
          "]d" = {
            action = "goto_next";
            desc = "Go to next diagnostic";
          };
          "<leader>ld" = {
            action = "open_float";
            desc = "Show Line Diagnostics";
          };
        };

        lspBuf = {
          "<leader>ca" = {
            action = "code_action";
            desc = "Code Actions";
          };
          "<leader>rn" = {
            action = "rename";
            desc = "Rename Symbol";
          };
          "<leader>f" = {
            action = "format";
            desc = "Format";
          };
          "gd" = {
            action = "definition";
            desc = "Goto definition (assignment)";
          };
          "gD" = {
            action = "declaration";
            desc = "Goto declaration (first occurrence)";
          };
          "gy" = {
            action = "type_definition";
            desc = "Goto Type Defition";
          };
          "gi" = {
            action = "implementation";
            desc = "Goto Implementation";
          };
          "<leader>K" = {
            action = "hover";
            desc = "Hover";
          };
          "<leader>sh" = {
            action = "signature_help";
            desc = "Signature Help";
          };
          "<leader>gr" = {
            action = "references";
            desc = "References to thing";
          };
          "<leader>vws" = {
            action = "workspace_symbol";
            desc = "Workspace symbol";
          };
        };
      };
    };

    plugins.lspsaga.enable = true;
  };
}
