{pkgs, ...}: {
  home.packages = [pkgs.jj-starship];

  programs.starship = {
    enable = true;
    enableBashIntegration = true;
    enableNushellIntegration = true;

    settings = {
      directory = {
        truncation_length = 4;
      };
      gcloud = {
        disabled = true;
      };

      # jj-starship replaces the built-in git modules: jj change/bookmarks in jj repos, branch/commit in plain git repos.
      custom.jj = {
        when = "jj-starship detect";
        shell = ["jj-starship"];
        format = "$output ";
      };
      git_branch = {
        disabled = true;
      };
      git_commit = {
        disabled = true;
      };
      git_status = {
        disabled = true;
      };
    };
  };
}
