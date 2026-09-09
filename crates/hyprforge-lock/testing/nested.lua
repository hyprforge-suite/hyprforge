-- Minimal nested Hyprland for testing the lock screen.
--
-- Lua rather than .conf: Hyprland 0.56 paints a deprecation banner over
-- the nested window for .conf files, and a test compositor that shows
-- warnings teaches you to ignore warnings.
--
-- No autostart, no wallpaper daemon, nothing that touches the real
-- session.

-- WAYLAND-1 is what the nested backend actually names its output. An
-- earlier version of this file said WL-1, which matches nothing, so the
-- rule was silently ignored entirely and every run came out a different
-- size.
--
-- `mode` stays "preferred" on purpose: a nested output is a window in
-- the host compositor, and its size is the host's to decide, so asking
-- for a fixed one just re-creates the silent no-op. Scale is worth
-- pinning though — the host's 1.5 put the lock surface on fractional
-- sizes.
hl.monitor({ output = [[WAYLAND-1]], mode = [[preferred]], position = [[0x0]], scale = 1 })

hl.config({
    misc = {
        disable_hyprland_logo = true,
        disable_splash_rendering = true,
        force_default_wallpaper = 0,
    },
    general = {
        ["col.active_border"] = [[rgba(bd93f9ff)]],
    },
})

hl.bind("SUPER + Q", hl.dsp.exec_cmd([[kitty]]), { description = "Terminal" })
hl.bind("SUPER + SHIFT + E", hl.dsp.exit(), { description = "Exit" })
