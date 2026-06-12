-- Laptop Hyprland config (Lua API; stubs for lua-language-server are wired
-- via the generated ~/.config/hypr/.luarc.json)

hl.monitor({ output = "DP-2", mode = "preferred", position = "1920x0", scale = 1 })
hl.monitor({ output = "eDP-1", mode = "preferred", position = "0x0", scale = 1 })
hl.monitor({ output = "", mode = "preferred", position = "auto", scale = 1 })

hl.on("hyprland.start", function()
	hl.exec_cmd("waybar")
	hl.exec_cmd("mako")
	hl.exec_cmd("nm-applet")
	hl.exec_cmd("dbus-update-activation-environment --systemd WAYLAND_DISPLAY XDG_CURRENT_DESKTOP")
	hl.exec_cmd("alacritty -e bash -c 'curl wttr.in | less && zellij'")
end)

hl.env("XCURSOR_SIZE", "24")

hl.config({
	input = {
		kb_layout = "us",
		kb_variant = "altgr-intl",
		follow_mouse = 1,

		touchpad = {
			clickfinger_behavior = true,
			disable_while_typing = true,
			middle_button_emulation = false,
			natural_scroll = false,
			tap_to_click = true,
		},
	},

	general = {
		gaps_in = 2,
		gaps_out = 2,
		border_size = 2,
		col = {
			active_border = { colors = { "rgba(33ccffee)", "rgba(00ff99ee)" }, angle = 45 },
			inactive_border = "rgba(595959aa)",
		},
		layout = "dwindle",
		allow_tearing = false,
	},

	decoration = {
		rounding = 2,
	},

	animations = {
		enabled = false,
	},

	dwindle = {
		preserve_split = true,
	},

	master = {
		new_status = "slave",
	},

	misc = {
		disable_hyprland_logo = true,
		disable_splash_rendering = true,
		force_default_wallpaper = 0,
	},
})

local mainMod = "SUPER"

hl.bind(mainMod .. " + Return", hl.dsp.exec_cmd("alacritty"))
hl.bind(mainMod .. " + SHIFT + Q", hl.dsp.window.close())
hl.bind(mainMod .. " + SHIFT + E", hl.dsp.exit())
hl.bind(mainMod .. " + F", hl.dsp.window.fullscreen())
hl.bind(mainMod .. " + W", hl.dsp.exec_cmd("rofi -show window"))
hl.bind(mainMod .. " + B", hl.dsp.exec_cmd("pkill --signal USR1 waybar"))
hl.bind(mainMod .. " + SHIFT + F", hl.dsp.window.float())
hl.bind("ALT + F11", hl.dsp.exec_cmd("loginctl lock-session"))
hl.bind(mainMod .. " + D", hl.dsp.exec_cmd("rofi -modi drun,run -show drun"))
hl.bind(mainMod .. " + P", hl.dsp.window.pseudo()) -- dwindle
hl.bind(mainMod .. " + T", hl.dsp.layout("togglesplit")) -- dwindle

hl.bind(mainMod .. " + SHIFT + Print", hl.dsp.exec_cmd([[grim -g "$(slurp -d)" - | wl-copy -t image/png]]))
hl.bind(mainMod .. " + CTRL + Print", hl.dsp.exec_cmd([[grim -g "$(slurp)" $HOME/Pictures/$(date +'%s_grim.png')]]))
hl.bind(mainMod .. " + Print", hl.dsp.exec_cmd([[grim $HOME/Pictures/$(date +'%s_grim.png')]]))

local dirKeys = {
	left = "left",
	right = "right",
	up = "up",
	down = "down",
	h = "left",
	l = "right",
	k = "up",
	j = "down",
}
for key, dir in pairs(dirKeys) do
	hl.bind(mainMod .. " + " .. key, hl.dsp.focus({ direction = dir }))
	hl.bind(mainMod .. " + SHIFT + " .. key, hl.dsp.window.move({ direction = dir }))
end

-- special workspace (scratchpad)
hl.bind(mainMod .. " + S", hl.dsp.workspace.toggle_special("magic"))
hl.bind(mainMod .. " + SHIFT + S", hl.dsp.window.move({ workspace = "special:magic" }))

-- scroll through existing workspaces with mainMod + scroll
hl.bind(mainMod .. " + mouse_down", hl.dsp.focus({ workspace = "e+1" }))
hl.bind(mainMod .. " + mouse_up", hl.dsp.focus({ workspace = "e-1" }))

for i = 1, 10 do
	local key = i % 10 -- key 0 maps to workspace 10
	hl.bind(mainMod .. " + " .. key, hl.dsp.focus({ workspace = i }))
	hl.bind(mainMod .. " + SHIFT + " .. key, hl.dsp.window.move({ workspace = i }))
end

-- moveactive (floating windows; tiled use movewindow above)
local moveStep = {
	left = { -20, 0 },
	right = { 20, 0 },
	up = { 0, -20 },
	down = { 0, 20 },
	h = { -20, 0 },
	l = { 20, 0 },
	k = { 0, -20 },
	j = { 0, 20 },
}
for key, step in pairs(moveStep) do
	hl.bind(
		mainMod .. " + SHIFT + " .. key,
		hl.dsp.window.move({ x = step[1], y = step[2], relative = true }),
		{ repeating = true }
	)
end

local resizeStep = {
	left = { 30, 0 },
	right = { -30, 0 },
	up = { 0, -10 },
	down = { 0, 10 },
	l = { 30, 0 },
	h = { -30, 0 },
	k = { 0, -10 },
	j = { 0, 10 },
}
for key, step in pairs(resizeStep) do
	hl.bind(
		mainMod .. " + CTRL + " .. key,
		hl.dsp.window.resize({ x = step[1], y = step[2], relative = true }),
		{ repeating = true }
	)
end

hl.bind("XF86AudioRaiseVolume", hl.dsp.exec_cmd("wpctl set-volume -l 1.5 @DEFAULT_AUDIO_SINK@ 5%+"), { repeating = true })

hl.bind("XF86AudioLowerVolume", hl.dsp.exec_cmd("wpctl set-volume @DEFAULT_AUDIO_SINK@ 5%-"), { locked = true })
hl.bind("XF86AudioMute", hl.dsp.exec_cmd("wpctl set-mute @DEFAULT_AUDIO_SINK@ toggle"), { locked = true })
hl.bind("XF86AudioMicMute", hl.dsp.exec_cmd("wpctl set-mute @DEFAULT_AUDIO_SOURCE@ toggle"), { locked = true })
hl.bind("XF86AudioPlay", hl.dsp.exec_cmd("playerctl play-pause"), { locked = true })

-- move/resize windows with mainMod + LMB/RMB and dragging
hl.bind(mainMod .. " + mouse:272", hl.dsp.window.drag(), { mouse = true })
hl.bind(mainMod .. " + mouse:273", hl.dsp.window.resize(), { mouse = true })
