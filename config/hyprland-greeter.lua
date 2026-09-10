-- The compositor the greeter runs under. Install as
-- /etc/greetd/hyprland-greeter.lua.
--
-- The important thing about this file is what is NOT in it.
--
-- Whoever is standing at the keyboard in front of a login screen is
-- unauthenticated, and every keybind this compositor has is theirs. A
-- greeter compositor with the usual `SUPER + Q -> terminal` gets that
-- person a shell as the greeter user without logging in — which is the
-- classic way greeters are broken into, and nothing the greeter program
-- itself can prevent. Verified against the test compositor used while
-- developing this: with the login screen on screen, its binds were
-- still live.
--
-- So: no binds. Not one. Not a "harmless" volume key, because the point
-- is the habit, and the next person to add one will add it next to the
-- others.
--
-- No `exec_cmd` either, beyond the greeter itself.

-- Let Hyprland pick the mode; a greeter has no opinion about
-- resolutions, and a wrong one here is a login screen nobody can read.
hl.monitor({ output = [[]], mode = [[preferred]], position = [[auto]], scale = 1 })

hl.config({
    misc = {
        disable_hyprland_logo = true,
        disable_splash_rendering = true,
        force_default_wallpaper = 0,
        -- Nothing to save and nobody to ask: the greeter is the only
        -- client and it is expected to exit.
        close_special_on_empty = true,
    },
    general = {
        -- No gaps or borders. The greeter is fullscreen and alone, and
        -- a border around it would only advertise that it is a window.
        gaps_in = 0,
        gaps_out = 0,
        border_size = 0,
    },
    decoration = {
        rounding = 0,
    },
    animations = {
        -- A login screen that animates in is a login screen that is
        -- slower to appear.
        enabled = false,
    },
})

-- The greeter, and then out. The compositor exits when the greeter does,
-- because greetd waits for the whole session command to finish before
-- starting the real session — a compositor that outlived its greeter
-- would leave the machine at a blank screen.
--
-- `--user` must name the account to log in. There is no user picker
-- yet, so this is where the choice is made.
hl.exec_cmd(
    [[sh -c 'hyprforge-greet --user CHANGE_ME --command "uwsm start hyprland"; hyprctl dispatch exit']]
)
