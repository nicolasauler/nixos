{...}: {
  wayland.windowManager.hyprland = {
    enable = true;

    # hand-written Lua config: HM's settings->lua renderer is still young
    # (open rendering bugs as of June 2026), so bypass it entirely.
    # The generated hyprland.lua is just a require() loader + systemd hook.
    configType = "lua";
    extraLuaFiles."main" = ./hyprland.lua;
  };
}
