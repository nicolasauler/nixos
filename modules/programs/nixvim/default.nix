{
  pkgs,
  inputs,
  ...
}: {
  # Import all your configuration modules here
  imports = [
    inputs.nixvim.homeModules.nixvim
    ./plugins
    ./options.nix
    ./keymaps.nix
  ];

  programs.nixvim = {
    enable = true;
    defaultEditor = true;
    # package = pkgs.neovim-nightly;

    globals = {
      mapleader = " ";
    };

    extraConfigLuaPre = ''
      if vim.env.SSH_CONNECTION then
        local osc52 = require('vim.ui.clipboard.osc52')
        local clipboard_cache = {
          ['+'] = { {}, 'v' },
          ['*'] = { {}, 'v' },
        }

        vim.g.clipboard = {
          name = 'OSC52 copy-only',
          copy = {
            ['+'] = function(lines, regtype)
              clipboard_cache['+'] = { lines, regtype }
              osc52.copy('+')(lines)
            end,
            ['*'] = function(lines, regtype)
              clipboard_cache['*'] = { lines, regtype }
              osc52.copy('*')(lines)
            end,
          },
          paste = {
            ['+'] = function()
              return clipboard_cache['+']
            end,
            ['*'] = function()
              return clipboard_cache['*']
            end,
          },
          cache_enabled = 0,
        }
      end
    '';

    colorschemes.gruvbox.enable = true;

    # ... and even highlights and autocommands !
    highlight.ExtraWhitespace.bg = "red";
    highlightOverride.NotifyBackground = {
      bg = "#000000";
    };

    match.ExtraWhitespace = "\\s\\+$";
    autoCmd = [
      {
        event = "FileType";
        pattern = "nix";
        command = "setlocal tabstop=2 shiftwidth=2";
      }
      {
        event = "FileType";
        pattern = "proto";
        command = "setlocal tabstop=2 shiftwidth=2";
      }
    ];

    extraPlugins = with pkgs.vimPlugins; [
      neodev-nvim
      nvim-nio
      vim-dadbod
      vim-dadbod-ui
      vim-dadbod-completion
      telescope-dap-nvim
      nui-nvim
    ];
  };
}
