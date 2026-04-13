{
  programs.nixvim = {
    plugins.snacks = {
      enable = true;
      settings = {
        gh = {
          enabled = true;
        };
        picker = {
          enabled = true;
          win.input.keys = {
            "<a-g>" = {
              __raw = ''{ "toggle_live", mode = { "i", "n" } }'';
            };
          };
        };
        input.enabled = true;
        words.enabled = true;
        scope.enabled = true;
        indent.enabled = true;
      };
    };

    keymaps = [
      {
        mode = "n";
        key = "<leader>ns";
        options = {
          silent = true;
          desc = "Neovim [S]nacks";
        };
        action.__raw = "function() Snacks.picker() end";
      }
      {
        mode = "n";
        key = "<leader>pf";
        action.__raw = "function() Snacks.picker.files({ hidden = true, ignored = true }) end";
        options = {
          silent = true;
          desc = "Snacks [F]ind files";
        };
      }
      {
        mode = "n";
        key = "<leader>pg";
        action.__raw = "function() Snacks.picker.grep() end";
        options = {
          silent = true;
          desc = "Snacks Live [G]rep";
        };
      }
      {
        mode = "n";
        key = "<leader>pm";
        action.__raw = "function() Snacks.picker.grep_word() end";
        options = {
          silent = true;
          desc = "Snacks Grep Word [M]atch";
        };
      }
      {
        mode = "n";
        key = "<leader>ob";
        action.__raw = "function() Snacks.picker.buffers() end";
        options = {
          silent = true;
          desc = "Snacks [O]pen [B]uffers";
        };
      }
      {
        mode = "n";
        key = "<leader>po";
        action.__raw = "function() Snacks.picker.recent() end";
        options = {
          silent = true;
          desc = "Snacks Recent/[O]ld files";
        };
      }
      {
        mode = "n";
        key = "<leader>pe";
        action.__raw = "function() Snacks.picker.commands() end";
        options = {
          silent = true;
          desc = "Snacks commands ([E]xecutables)";
        };
      }
      {
        mode = "n";
        key = "<leader>pl";
        action.__raw = "function() Snacks.picker.command_history() end";
        options = {
          silent = true;
          desc = "Snacks command history ([L]og)";
        };
      }
      {
        mode = "n";
        key = "<leader>pj";
        action.__raw = "function() Snacks.picker.jumps() end";
        options = {
          silent = true;
          desc = "Snacks [J]umplist";
        };
      }
      {
        mode = "n";
        key = "<leader>pr";
        action.__raw = "function() Snacks.picker.resume() end";
        options = {
          silent = true;
          desc = "Snacks [R]esume previous picker";
        };
      }
      {
        mode = "n";
        key = "<C-p>";
        action.__raw = "function() Snacks.picker.git_files() end";
        options = {
          silent = true;
          desc = "Snacks Git Files";
        };
      }
      {
        mode = "n";
        key = "<leader>ph";
        action.__raw = "function() Snacks.picker.help() end";
        options = {
          silent = true;
          desc = "Snacks [H]elp tags";
        };
      }
      {
        mode = "n";
        key = "<leader>pb";
        action.__raw = "function() Snacks.picker.git_branches() end";
        options = {
          silent = true;
          desc = "Snacks git [B]ranches";
        };
      }
      {
        mode = "n";
        key = "<leader>pc";
        action.__raw = "function() Snacks.picker.git_log() end";
        options = {
          silent = true;
          desc = "Snacks git [C]ommits";
        };
      }
      {
        mode = "n";
        key = "<leader>ps";
        action.__raw = "function() Snacks.picker.git_status() end";
        options = {
          silent = true;
          desc = "Snacks git [S]tatus";
        };
      }
      {
        mode = "n";
        key = "<leader>pk";
        action.__raw = "function() Snacks.picker.keymaps() end";
        options = {
          silent = true;
          desc = "Snacks [K]eymaps";
        };
      }
      # Words: jump between LSP references
      {
        mode = ["n" "t"];
        key = "]]";
        action.__raw = "function() Snacks.words.jump(vim.v.count1) end";
        options = {
          silent = true;
          desc = "Next LSP Reference";
        };
      }
      {
        mode = ["n" "t"];
        key = "[[";
        action.__raw = "function() Snacks.words.jump(-vim.v.count1) end";
        options = {
          silent = true;
          desc = "Prev LSP Reference";
        };
      }
      # GitHub integration (requires gh CLI)
      {
        mode = "n";
        key = "<leader>gi";
        action.__raw = "function() Snacks.picker.gh_issue() end";
        options = {
          silent = true;
          desc = "GitHub [I]ssues (open)";
        };
      }
      {
        mode = "n";
        key = "<leader>gI";
        action.__raw = "function() Snacks.picker.gh_issue({ state = 'all' }) end";
        options = {
          silent = true;
          desc = "GitHub [I]ssues (all)";
        };
      }
      {
        mode = "n";
        key = "<leader>gp";
        action.__raw = "function() Snacks.picker.gh_pr() end";
        options = {
          silent = true;
          desc = "GitHub [P]ull Requests (open)";
        };
      }
      {
        mode = "n";
        key = "<leader>gP";
        action.__raw = "function() Snacks.picker.gh_pr({ state = 'all' }) end";
        options = {
          silent = true;
          desc = "GitHub [P]ull Requests (all)";
        };
      }
    ];
  };
}
