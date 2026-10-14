{
  lib,
  pkgs,
  ...
}: let
  python = pkgs.python3.withPackages (ps: [ps.grpcio]);
  # The test runs at build time, as in modules/services/buildbot-pr-policy.nix:
  # a failing test means no package.
  foot-pedal =
    pkgs.runCommand "foot-pedal" {
      nativeBuildInputs = [python pkgs.python3Packages.flake8];
      meta.mainProgram = "foot-pedal";
    } ''
      cp ${./foot-pedal.py} foot_pedal.py
      flake8 foot_pedal.py
      PYTHONPATH="$PWD" python ${./foot-pedal-test.py}
      { echo "#!${python}/bin/python3"; cat foot_pedal.py; } > foot-pedal
      install -Dm0555 foot-pedal "$out/bin/foot-pedal"
    '';
in {
  # PCsensor 3-pedal footswitch as hold keys for the Voyager: left = Shift,
  # middle = mirror layer, right = mouse layer (mapping in foot-pedal.py).
  # Layer pedals talk to keymapp's API, so keymapp must be running with its API
  # enabled (hyprland_note.lua starts it); the Voyager's own input is never
  # intercepted. Needs the input and uinput groups, which this user has; the
  # pedal is hidden from libinput by a udev rule in the host configuration.
  systemd.user.services.foot-pedal = {
    Unit = {
      Description = "Foot pedal: Voyager layers via keymapp, and Shift";
      PartOf = ["graphical-session.target"];
      After = ["graphical-session.target"];
    };
    Service = {
      ExecStart = lib.getExe foot-pedal;
      Restart = "on-failure";
      RestartSec = 2;
    };
    Install.WantedBy = ["graphical-session.target"];
  };
}
