{ ... }:
{
  programs.git = {
    enable = true;
    userName = "Nicolas Auler";
    userEmail = "nicolasauler@usp.br";

    extraConfig = {
      commit = { template = "home/nic/nixos/config/git/commit_template"; };
      merge = { tool = "nvimdiff4"; prompt = false; };
      mergeTool = {
        nvimdiff4 = {
          cmd = "nvim -d $LOCAL $BASE $REMOTE $MERGED -c '$wincmd w' -c 'wincmd J'";
        };
      };
      core = { editor = "nvim"; };
      init = { defaultBranch = "main"; };
    };
  };
}
