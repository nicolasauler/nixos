{ ... }:
{
  programs.starship = {
    enable = true;

    settings = {
      directory = {
        truncation_length = 4;
      };
    };
  };
}
