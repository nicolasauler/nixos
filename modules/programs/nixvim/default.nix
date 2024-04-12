{ pkgs, inputs, ... }: {
  # Import all your configuration modules here
  imports = [
    inputs.nixvim.homeManagerModules.nixvim
    ./plugins
    ./options.nix
  ];

  programs.nixvim = {
    enable = true;
    defaultEditor = true;
    globals = {
      mapleader = " ";
    };

    colorschemes.gruvbox.enable = true;

    # ... and even highlights and autocommands !
    highlight.ExtraWhitespace.bg = "red";
    match.ExtraWhitespace = "\\s\\+$";
    autoCmd = [
      {
        event = "FileType";
        pattern = "nix";
        command = "setlocal tabstop=2 shiftwidth=2";
      }
    ];

    extraPlugins = with pkgs.vimPlugins; [
      harpoon2
      neodev-nvim
      # nvim-nio
      vim-dadbod
      vim-dadbod-ui
      vim-dadbod-completion
      telescope-dap-nvim
    ];
  };
}
