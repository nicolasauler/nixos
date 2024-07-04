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
        #vim.keymap.set('n', '<leader>pt', function()
        #	builtin.grep_string({ search = vim.fn.input("Grep > ") });
        #end)
        "<leader>tb" = {
          action = "buffers";
          options.desc = "Telescope open [B]uffers";
        };
        "<C-p>" = {
          action = "git_files";
          options.desc = "Telescope Git Files";
        };
        "<leader>pg" = {
          action = "live_grep";
          options.desc = "Telescope Live [G]rep";
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

      local git_worktree_ext = require("telescope").load_extension('git_worktree')
      vim.keymap.set('n', '<leader>pw', git_worktree_ext.git_worktrees, {})
      vim.keymap.set('n', '<leader>pr', git_worktree_ext.create_git_worktree, {})
    '';
  };
}
