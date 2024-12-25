{
  pkgs,
  inputs,
  ...
}: {
  programs.nixvim = {
    plugins.lsp = {
      enable = true;
      # inlayHints = true;
      servers = {
        # arduino_lsp doesn't work with nvim 0.10
        # really annoying
        # arduino_language_server.enable = true;
        bashls.enable = true;
        clangd.enable = true;
        lua_ls.enable = true;
        ltex = {
          enable = true;
          settings.additionalRules = {
            enablePickyRules = true;
            languageModel = "/home/nic/models/ngrams/";
            motherTongue = "en-GB";
          };
        };
        kotlin_language_server.enable = true;
        nil_ls = {
          enable = true;
          settings.formatting.command = ["alejandra"];
          # settings.formatting.command = [ "nixpkgs-fmt" ];
        };
        pyright.enable = true;
        # rust-analyzer = {
        #   enable = true;
        #   installCargo = false;
        #   installRustc = false;
        # };
        svelte.enable = true;
        ts_ls.enable = true;
        yamlls.enable = true;
        zls = {
          enable = true;
          package = inputs.zls-overlay.packages.${pkgs.system}.zls;
        };
      };

      keymaps = {
        # diagnostic = {
        #   "[d" = {
        #     action = "goto_prev";
        #     desc = "Go to prev diagnostic";
        #   };
        #   "]d" = {
        #     action = "goto_next";
        #     desc = "Go to next diagnostic";
        #   };
        #   "<leader>ld" = {
        #     action = "open_float";
        #     desc = "Show Line Diagnostics";
        #   };
        # };

        lspBuf = {
          # "<leader>ca" = {
          #   action = "code_action";
          #   desc = "Code Actions";
          # };
          # "<leader>rn" = {
          #   action = "rename";
          #   desc = "Rename Symbol";
          # };
          "<leader>f" = {
            action = "format";
            desc = "Format";
          };
          # "gd" = {
          #   action = "definition";
          #   desc = "Goto definition (assignment)";
          # };
          # "gD" = {
          #   action = "declaration";
          #   desc = "Goto declaration (first occurrence)";
          # };
          # "gt" = {
          #   action = "type_definition";
          #   desc = "Goto Type Defition";
          # };
          # "gi" = {
          #   action = "implementation";
          #   desc = "Goto Implementation";
          # };
          # "K" = {
          #   action = "hover";
          #   desc = "Hover";
          # };
          # "<leader>gr" = {
          #   action = "references";
          #   desc = "References to thing";
          # };
          "<leader>sh" = {
            action = "signature_help";
            desc = "Signature Help";
          };
          "<leader>vws" = {
            action = "workspace_symbol";
            desc = "Workspace symbol";
          };
        };
      };
    };

    plugins.clangd-extensions = {
      enable = true;
      enableOffsetEncodingWorkaround = true;
    };

    plugins.lspsaga = {
      enable = true;

      codeAction.extendGitSigns = false; # this adds gitsigns actions to code actions
      hover.openCmd = "!firefox";
      rename.inSelect = false;
      lightbulb.virtualText = false;
      symbolInWinbar.folderLevel = 2;
    };

    keymaps = [
      # lspsaga
      {
        mode = "n";
        key = "<leader>go";
        options.silent = false;
        action = "<cmd>Lspsaga outline<CR>";
      }
      {
        mode = "n";
        key = "<leader>gr";
        options.silent = false;
        action = "<cmd>Lspsaga finder<CR>";
      }
      {
        mode = "n";
        key = "<leader>wd";
        options.silent = false;
        action = "<cmd>Lspsaga show_workspace_diagnostics ++unfocus<CR>";
      }
      {
        mode = "n";
        key = "<leader>ld";
        options.silent = false;
        action = "<cmd>Lspsaga show_line_diagnostics ++unfocus<CR>";
      }
      {
        mode = "n";
        key = "[d";
        options.silent = false;
        action = "<cmd>Lspsaga diagnostic_jump_prev<CR>";
      }
      {
        mode = "n";
        key = "]d";
        options.silent = false;
        action = "<cmd>Lspsaga diagnostic_jump_next<CR>";
      }
      # {
      #   mode = "n";
      #   key = "]e";
      #   options.silent = false;
      #   action = "<cmd>Lspsaga diagnostic_jump_next<CR>";
      # }
      {
        mode = "n";
        key = "<leader>Pd";
        options.silent = false;
        action = "<cmd>Lspsaga peek_definition<CR>";
      }
      {
        mode = "n";
        key = "<leader>Pt";
        options.silent = false;
        action = "<cmd>Lspsaga peek_type_definition<CR>";
      }
      {
        mode = "n";
        key = "gd";
        options.silent = false;
        action = "<cmd>Lspsaga goto_definition<CR>";
      }
      {
        mode = "n";
        key = "gt";
        options.silent = false;
        action = "<cmd>Lspsaga goto_type_definition<CR>";
      }
      {
        mode = "n";
        key = "<leader>ca";
        options.silent = false;
        action = "<cmd>Lspsaga code_action<CR>";
      }
      {
        mode = "n";
        key = "<leader>rn";
        options.silent = false;
        action = "<cmd>Lspsaga rename ++project<CR>";
      }
      {
        mode = "n";
        key = "K";
        options.silent = false;
        action = "<cmd>Lspsaga hover_doc<CR>";
      }
      {
        mode = "n";
        key = "<leader>K";
        options.silent = false;
        action = "<cmd>Lspsaga hover_doc ++keep<CR>";
      }
      {
        mode = "n";
        key = "<leader>ci";
        options.silent = false;
        action = "<cmd>Lspsaga incoming_calls<CR>";
      }
      {
        mode = "n";
        key = "<leader>co";
        options.silent = false;
        action = "<cmd>Lspsaga outgoing_calls<CR>";
      }
    ];
  };
}
