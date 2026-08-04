use std::path::PathBuf;

/// `$XDG_CONFIG_HOME`, falling back to `~/.config`.
pub fn config_home() -> PathBuf {
    if let Some(dir) = std::env::var_os("XDG_CONFIG_HOME") {
        let dir = PathBuf::from(dir);
        if dir.is_absolute() {
            return dir;
        }
    }
    let home = std::env::var_os("HOME").expect("HOME must be set");
    PathBuf::from(home).join(".config")
}

/// `$XDG_CONFIG_HOME/hyprforge` — canonical storage for all Hyprforge-owned
/// config (display profiles, window-rules TOML).
pub fn hyprforge_config_dir() -> PathBuf {
    config_home().join("hyprforge")
}

/// `$XDG_CONFIG_HOME/hypr` — the user's own Hyprland config directory.
/// Hyprforge only ever writes to `hypr/hyprforge/` inside this, and only
/// touches `hyprland.lua` itself via the explicit, confirmed one-time
/// `require()` insertion.
pub fn hypr_config_dir() -> PathBuf {
    config_home().join("hypr")
}

/// `$XDG_CONFIG_HOME/hypr/hyprforge` — generated Lua artifacts live here,
/// sourced from the user's `hyprland.lua` via `require("hyprforge/...")`.
pub fn hypr_hyprforge_dir() -> PathBuf {
    hypr_config_dir().join("hyprforge")
}

pub fn display_profiles_path() -> PathBuf {
    hyprforge_config_dir().join("display-profiles.toml")
}

pub fn window_rules_toml_path() -> PathBuf {
    hyprforge_config_dir().join("window-rules.toml")
}

pub fn window_rules_lua_path() -> PathBuf {
    hypr_hyprforge_dir().join("window-rules.lua")
}

pub fn hyprland_lua_path() -> PathBuf {
    hypr_config_dir().join("hyprland.lua")
}

/// Write `contents` to `path` atomically: write to a sibling temp file, then
/// rename over the target. Avoids ever leaving a half-written config file.
pub fn write_atomic(path: &std::path::Path, contents: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension(format!(
        "{}.tmp",
        path.extension().and_then(|e| e.to_str()).unwrap_or("")
    ));
    std::fs::write(&tmp, contents)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}
