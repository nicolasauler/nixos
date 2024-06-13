{
  programs.nixvim = {
    plugins.yanky = {
      enable = true;
      picker.telescope = {
        enable = true;
        useDefaultMappings = true;
      };
    };

    plugins.telescope = {
      enable = true;

      keymaps = {
        "<leader>pf" = "find_files";
        #vim.keymap.set('n', '<leader>pt', function()
        #	builtin.grep_string({ search = vim.fn.input("Grep > ") });
        #end)
        "<C-p>" = "git_files";
        "<leader>pg" = "live_grep";
        "<leader>ph" = "help_tags";
        "<leader>pb" = "git_branches";
        "<leader>pc" = "git_commits";
        "<leader>ps" = "git_status";
      };

      settings = {
        pickers = {
          find_files = {
            hidden = true;
            no_ignore = true;
            file_ignore_patterns = ["%.git/.*"];
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
