{
  programs.nixvim = {
    opts = {
      number = true;
      relativenumber = true;

      tabstop = 4;
      softtabstop = 4;
      shiftwidth = 4;
      expandtab = true;

      smartindent = true;

      wrap = false;

      swapfile = false;
      backup = false;
      # undodir = "os.getenv(\"HOME\") .. \"/.vim/undodir\"";
      undofile = true;

      hlsearch = true;
      incsearch = true;
      ignorecase = true;
      smartcase = true;

      termguicolors = true;

      scrolloff = 8;
      signcolumn = "yes";
      #isfname:append("@-@");

      updatetime = 50;

      colorcolumn = "80";
    };

    extraConfigLua = ''
      if os.getenv("SSH_CONNECTION") then
        vim.g.clipboard = 'osc52'
      else
        vim.g.clipboard = {
          name = "wayland",
          copy = {
            ["+"] = "wl-copy",
            ["*"] = "wl-copy",
          },
          paste = {
            ["+"] = "wl-paste",
            ["*"] = "wl-paste",
          },
          cache_enabled = false,
        }
      end
    '';
  };
}
