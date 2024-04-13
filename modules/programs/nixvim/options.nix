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
  };
}
