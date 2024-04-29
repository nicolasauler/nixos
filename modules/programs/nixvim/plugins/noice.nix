{
  programs.nixvim = {
    plugins.notify = {
      enable = true;
      topDown = true; # false to make notification appear in bottom
      # however I'm going to stick with it at the top and dismiss
      # when I want with <leader>nd (NoiceDismiss)
    };

    plugins.noice = {
      enable = true;

      lsp.override = {
        "vim.lsp.util.convert_input_to_markdown_lines" = true;
        "vim.lsp.util.stylize_markdown" = true;
        "cmp.entry.get_documentation" = true;
      };

      notify.enabled = true;

      presets = {
        bottom_search = false; # use a classic bottom cmdline for search
        command_palette = false; # position the cmdline and popupmenu together
        long_message_to_split = true; # long messages will be sent to a split
        inc_rename = false; # enables an input dialog for inc-rename.nvim
        lsp_doc_border = false; # add a border to hover docs and signature help
      };
    };

    keymaps = [
      {
        mode = "n";
        key = "<leader>nd";
        options.silent = true;
        action = "<cmd>NoiceDismiss<CR>";
      }
    ];
  };
}
