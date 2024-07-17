{
  programs.nixvim = {
    plugins.flash = {
      enable = true;
      settings = {
        jump.autojump = true;
        label.rainbow.enabled = true;
        modes.char.jump_labels = true;
      };
    };

    keymaps = [
      {
        key = "s";
        action.__raw = "function() require('flash').jump() end";
        mode = ["n" "x" "o"];
        options.desc = "flash jump";
      }
      {
        key = "S";
        action.__raw = "function() require('flash').treesitter() end";
        mode = ["n" "x" "o"];
        options.desc = "flash treesitter";
      }
      {
        key = "r";
        action.__raw = "function() require('flash').remote() end";
        mode = "o";
        options.desc = "[r]emote flash";
      }
      {
        key = "R";
        action.__raw = "function() require('flash').treesitter_search() end";
        mode = ["o" "x"];
        options.desc = "flash treesitte[R] search";
      }
      {
        key = "<c-s>";
        action.__raw = "function() require('flash').toggle() end";
        mode = "c";
        options.desc = "toggle flash [s]earch";
      }
    ];
  };
}
