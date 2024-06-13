{pkgs, ...}: {
  programs.nixvim = {
    plugins.conform-nvim = {
      enable = true;
      formatOnSave = {
        lspFallback = true;
        timeoutMs = 500;
      };
      notifyOnError = true;
      formattersByFt = {
        c = ["clang-format"];
        cpp = ["clang-format"];
        json = [["prettierd" "prettier"]];
        lua = ["stylua "];
        markdown = [["prettierd" "prettier"]];
        nix = [["alejandra" "nixfmt" "nixpkgs-fmt"]];
        python = ["isort" "black"];
        rust = ["rustfmt"];
        sh = ["shfmt "];
        sql = [["pg_format" "sql_formatter" "sqlfluff"]];
        yaml = ["prettierd"];
      };
    };
    extraPackages = with pkgs; [
      alejandra
      black
      clang-tools
      isort
      nixfmt-rfc-style
      nixpkgs-fmt
      pgformatter
      prettierd
      rustfmt
      shfmt
      sqlfluff
      stylua
    ];
  };
}
