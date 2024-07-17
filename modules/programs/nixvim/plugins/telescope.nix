{
  programs.nixvim = {
    plugins.yanky = {
      enable = true;
      enableTelescope = true;
      settings.picker.telescope.use_default_mappings = true;
    };

    plugins.telescope = {
      enable = true;
      enabledExtensions = [
        "notify"
      ];

      keymaps = {
        "<leader>pf" = {
          action = "find_files";
          options.desc = "Telescope [F]ind files";
        };
        "<leader>ob" = {
          action = "buffers";
          options.desc = "Telescope [O]pen [B]uffers";
        };
        "<leader>po" = {
          action = "oldfiles";
          options.desc = "Telescope [O]ld files";
        };
        "<leader>pe" = {
          action = "commands";
          options.desc = "Telescope commands ([E]xecutables)";
        };
        "<leader>pl" = {
          action = "command_history";
          options.desc = "Telescope command history ([L]og)";
        };
        "<leader>pj" = {
          action = "jumplist";
          options.desc = "Telescope [J]umplist";
        };
        "<leader>pr" = {
          action = "resume";
          options.desc = "Telescope [R]esume previous picker";
        };
        "<C-p>" = {
          action = "git_files";
          options.desc = "Telescope Git Files";
        };
        "<leader>pg" = {
          action = "live_grep";
          options.desc = "Telescope Live [G]rep";
        };
        "<leader>pm" = {
          action = "grep_string";
          options.desc = "Telescope Grep String [M]atch";
        };
        "<leader>ph" = {
          action = "help_tags";
          options.desc = "Telescope [H]elp tags";
        };
        "<leader>pb" = {
          action = "git_branches";
          options.desc = "Telescope git [B]ranches";
        };
        "<leader>pc" = {
          action = "git_commits";
          options.desc = "Telescope git [C]ommits";
        };
        "<leader>ps" = {
          action = "git_status";
          options.desc = "Telescope git [S]tatus";
        };
      };

      settings = {
        pickers = {
          find_files = {
            hidden = true;
            no_ignore = true;
            file_ignore_patterns = ["%.git/.*"];
          };
        };
        defaults = {
          layout_strategy = "flex";
          mappings = let
            open_with_trouble = {
              __raw = "require(\"trouble.sources.telescope\").open";
            };
          in {
            i = {
              "<c-t>" = open_with_trouble;
            };
            n = {
              "<c-t>" = open_with_trouble;
            };
          };
        };
      };
    };

    extraConfigLua = ''
      local yanky_ext = require("telescope").load_extension("yank_history")
      vim.keymap.set('n', '<leader>py', yanky_ext.yank_history, {})

      local noice_ext = require("telescope").load_extension("noice")
      vim.keymap.set('n', '<leader>pn', noice_ext.noice, {})

      local dap_ext = require("telescope").load_extension('dap')
      vim.keymap.set('n', '<leader>pd', dap_ext.commands, {})
    '';
  };
}
