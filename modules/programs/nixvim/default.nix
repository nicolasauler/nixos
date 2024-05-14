{ pkgs, inputs, ... }: {
  # Import all your configuration modules here
  imports = [
    inputs.nixvim.homeManagerModules.nixvim
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
      # copilot_no_tab_map = true; -> for copilot.vim
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
      {
        event = "FileType";
        pattern = "proto";
        command = "setlocal tabstop=2 shiftwidth=2";
      }
    ];

    extraPlugins = with pkgs.vimPlugins; [
      # harpoon2
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
