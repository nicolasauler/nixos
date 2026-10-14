{
  lib,
  pkgs,
  ...
}: let
  foot-pedal = pkgs.writers.writePython3Bin "foot-pedal" {} (builtins.readFile ./foot-pedal.py);
in {
  # PCsensor 3-pedal footswitch as hold keys for the Voyager: left = Shift,
  # middle = mirror layer, right = mouse layer (mapping in foot-pedal.py).
  # Layer pedals go pedal -> kontroll -> keymapp's API -> firmware, so keymapp
  # must be running with its API enabled (hyprland_note.lua starts it); the
  # Voyager's own input is never intercepted. Needs the input and uinput
  # groups, which this user has.
  systemd.user.services.foot-pedal = {
    Unit = {
      Description = "Foot pedal: Voyager layers via keymapp, and Shift";
      PartOf = ["graphical-session.target"];
      After = ["graphical-session.target"];
    };
    Service = {
      ExecStart = lib.getExe foot-pedal;
      Environment = "PATH=${lib.makeBinPath [pkgs.kontroll]}";
      Restart = "on-failure";
      RestartSec = 2;
    };
    Install.WantedBy = ["graphical-session.target"];
  };
}
