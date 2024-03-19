local builtin = require('telescope.builtin')
local yanky_ext = require("telescope").load_extension("yank_history")

vim.keymap.set('n', '<leader>pf', builtin.find_files, {})
vim.keymap.set('n', '<C-p>', builtin.git_files, {})
--vim.keymap.set('n', '<leader>ps', function()
--	builtin.grep_string({ search = vim.fn.input("Grep > ") });
--end)
vim.keymap.set('n', '<leader>pg', builtin.live_grep, {})
vim.keymap.set('n', '<leader>ph', builtin.help_tags, {})
vim.keymap.set('n', '<leader>py', yanky_ext.yank_history, {})

vim.keymap.set('n', '<leader>pb', builtin.git_branches, {})
vim.keymap.set('n', '<leader>pc', builtin.git_commits, {})
vim.keymap.set('n', '<leader>ps', builtin.git_status, {})

local actions = require("telescope.actions")
local trouble = require("trouble.providers.telescope")
local telescope = require("telescope")

telescope.setup {
  defaults = {
    mappings = {
      i = { ["<leader>tt"] = trouble.open_with_trouble },
      n = { ["<leader>tt"] = trouble.open_with_trouble },
    },
  },
}

local noice_ext = require("telescope").load_extension("noice")
vim.keymap.set('n', '<leader>pn', noice_ext.noice, {})


local dap_ext = telescope.load_extension('dap')
vim.keymap.set('n', '<leader>pd', dap_ext.commands, {})

local git_worktree_ext = telescope.load_extension('git_worktree')
vim.keymap.set('n', '<leader>pw', git_worktree_ext.git_worktrees, {})
vim.keymap.set('n', '<leader>pr', git_worktree_ext.create_git_worktree, {})
