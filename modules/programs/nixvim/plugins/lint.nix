{pkgs, ...}: {
  programs.nixvim = {
    plugins.lint = {
      enable = true;
      lintersByFt = {
        lua = ["selene"];
        markdown = ["vale"];
        proto = ["protolint"];
        python = ["ruff"];
        sql = ["sqlfluff"];
        text = ["vale"];
        yaml = ["yamllint"];
      };
    };

    extraPackages = with pkgs; [
      protolint
      ruff
      selene
      sqlfluff
      vale
      yamllint
    ];
  };
}
