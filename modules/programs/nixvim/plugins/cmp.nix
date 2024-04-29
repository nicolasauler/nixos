{ pkgs, ... }:
{
  programs.nixvim = {
    plugins = {
      cmp-nvim-lsp.enable = true;
      cmp-nvim-lua.enable = true;
      cmp-buffer.enable = true;
      cmp-path.enable = true;
      cmp-cmdline.enable = true;

      luasnip = {
        enable = true;
        fromVscode = [
          {
            paths = "${pkgs.vimPlugins.friendly-snippets}";
          }
        ];
      };

      lspkind = {
        enable = true;

        cmp = {
          enable = true;
          menu = {
            nvim_lsp = "[LSP]";
            nvim_lua = "[lua]";
            path = "[path]";
            luasnip = "[snip]";
            buffer = "[buffer]";
            vim-dadbod-completion = "[dadbod]";
          };
        };
      };

      cmp = {
        enable = true;
        settings = {
          # snippet.expand = "luasnip";
          snippet.expand = ''
            function(args)
              require('luasnip').lsp_expand(args.body)
            end
          '';
          sources = [
            { name = "nvim_lsp"; }
            { name = "luasnip"; }
            { name = "nvim_lua"; }
            { name = "path"; }
            {
              name = "buffer";
              # Words from other open buffers can also be suggested.
              option.get_bufnrs.__raw = "vim.api.nvim_list_bufs";
            }
            { name = "vim-dadbod-completion"; }
          ];
          mapping = {
            "<C-y>" = "cmp.mapping.confirm({ select = true })";
            "<C-Space>" = "cmp.mapping.complete()";
            #"<C-p>" = "cmp.mapping.select_prev_item(cmp_select)";
            #"<C-n>" = "cmp.mapping.select_next_item(cmp_select)";
            "<C-d>" = "cmp.mapping.scroll_docs(-4)";
            "<C-e>" = "cmp.mapping.close()";
            "<C-f>" = "cmp.mapping.scroll_docs(4)";
            "<C-p>" = "cmp.mapping(cmp.mapping.select_prev_item(), {'i', 's'})";
            "<C-n>" = "cmp.mapping(cmp.mapping.select_next_item(), {'i', 's'})";
          };
        };
      };
    };
  };
}
