{ ... }:
{
  home.file.".ssh/allowed_signers".text =
    "* ${builtins.readFile /home/nic/.ssh/id_ed25519.pub}";

  programs.git = {
    enable = true;
    userName = "Nicolas Auler";
    userEmail = "nicolasauler@usp.br";

    extraConfig = {
      commit = {
        template = "/home/nic/nixos/config/git/commit_template";
        gpgsign = true;
      };
      gpg = {
        format = "ssh";
        ssh.allowedSignersfile = "/home/nic/.ssh/allowed_signers";
      };
      user.signingkey = "/home/nic/.ssh/id_ed25519.pub";

      merge = { tool = "nvimdiff4"; prompt = false; };
      mergeTool = {
        nvimdiff4 = {
          cmd = "nvim -d $LOCAL $BASE $REMOTE $MERGED -c '$wincmd w' -c 'wincmd J'";
        };
      };
      core.editor = "nvim";
      init.defaultBranch = "main";
    };

    # signing.signByDefault = false;
  };
}
