{osConfig, ...}: {
  wayland.windowManager.hyprland = {
    enable = true;
    # Keep HM's CLI, Lua stubs and portal on the host-selected compositor.
    # Its default pkgs.hyprland otherwise shadows the NixOS upstream package.
    package = osConfig.programs.hyprland.package;
    portalPackage = osConfig.programs.hyprland.portalPackage;

    # hand-written Lua config: HM's settings->lua renderer is still young
    # (open rendering bugs as of June 2026), so bypass it entirely.
    # The generated hyprland.lua is just a require() loader + systemd hook.
    configType = "lua";
    extraLuaFiles."main" = ./hyprland.lua;
  };

  # polkit authentication agent (wiki "must-have"; GUI apps can't escalate without one)
  services.hyprpolkitagent.enable = true;
}
