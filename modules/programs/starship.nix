{ ... }:
{
  programs.starship = {
    enable = true;
    enableBashIntegration = true;
    enableNushellIntegration = true;

    settings = {
      directory = {
        truncation_length = 4;
      };
    };
  };
}
