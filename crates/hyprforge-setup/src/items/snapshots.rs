//! Previous Versions: letting this user read their home's snapshots.
//!
//! snapper keeps hourly snapshots of `/home` on a btrfs machine, under
//! `/home/.snapshots` — readable by root alone, so Files cannot offer
//! "Previous Versions" from them. snapper's own answer is two settings
//! in the `home` config: `ALLOW_USERS` names who may use it, and
//! `SYNC_ACL=yes` has snapper grant those users read access to
//! `.snapshots` with an ACL, and keep it granted.
//!
//! The first item here that needs root. It changes the settings with
//! snapper's own `set-config`, through `pkexec` — not by editing
//! `/etc/snapper/configs/home`, which snapper would not notice until its
//! daemon reread it and which is snapper's file to write. The check
//! reads that file, which is world-readable: `snapper get-config` itself
//! refuses a user who is not yet allowed.
//!
//! Only the `home` config, never `root`'s: a user being able to read
//! old copies of the system is not something Files needs, or asks for.
//!
//! Off by default — a password prompt nobody expected, in the middle of
//! setting up everything else, is not a default.

use super::{Applied, Cx};
use crate::record::Change;
use crate::state::State;

/// The snapper config this item changes.
pub const CONFIG: &str = "home";

/// What the two settings say now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Settings {
    pub allow_users: String,
    pub sync_acl: String,
}

/// `KEY="value"` lines, as snapper writes its configs.
pub(crate) fn parse(text: &str) -> Settings {
    let value = |key: &str| {
        text.lines()
            .rev()
            .find_map(|l| l.trim().strip_prefix(key)?.strip_prefix('='))
            .map(|v| v.trim().trim_matches('"').to_string())
            .unwrap_or_default()
    };
    Settings { allow_users: value("ALLOW_USERS"), sync_acl: value("SYNC_ACL") }
}

/// Whether `settings` already let `user` read the snapshots.
fn allows(settings: &Settings, user: &str) -> bool {
    settings.allow_users.split_whitespace().any(|u| u == user) && settings.sync_acl == "yes"
}

fn read(cx: &Cx<'_>) -> Result<Option<Settings>, String> {
    let path = cx.env.snapper_configs.join(CONFIG);
    match std::fs::read_to_string(&path) {
        Ok(text) => Ok(Some(parse(&text))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("couldn't read {}: {e}", path.display())),
    }
}

pub(super) fn check(cx: &Cx<'_>) -> State {
    match read(cx) {
        Err(e) => State::Unknown { why: e },
        Ok(None) => State::Unavailable { why: "snapper keeps no snapshots of /home here".into() },
        Ok(Some(settings)) if allows(&settings, &cx.env.user) => State::Done,
        Ok(Some(_)) => State::Todo {
            what: format!("Let {} read the snapshots of /home (asks for your password)", cx.env.user),
        },
    }
}

pub(super) fn apply(cx: &Cx<'_>) -> Result<Applied, String> {
    let previous = read(cx)?.ok_or_else(|| "snapper keeps no snapshots of /home here".to_string())?;
    let mut users: Vec<&str> = previous.allow_users.split_whitespace().collect();
    if !users.contains(&cx.env.user.as_str()) {
        users.push(&cx.env.user);
    }
    cx.sys.snapper_set_config(
        CONFIG,
        &[("ALLOW_USERS".to_string(), users.join(" ")), ("SYNC_ACL".to_string(), "yes".to_string())],
    )?;
    Ok(Applied {
        change: Change::Snapper { allow_users: previous.allow_users, sync_acl: previous.sync_acl },
        note: None,
    })
}

/// Puts both settings back. `ALLOW_USERS` first, while `SYNC_ACL` is
/// still on, so snapper takes this user's ACL back off `.snapshots` —
/// turning syncing off first would leave the grant in place with nothing
/// left to remove it.
pub(super) fn undo(cx: &Cx<'_>, allow_users: &str, sync_acl: &str) -> Result<Option<String>, String> {
    cx.sys.snapper_set_config(CONFIG, &[("ALLOW_USERS".to_string(), allow_users.to_string())])?;
    if sync_acl != "yes" {
        cx.sys.snapper_set_config(CONFIG, &[("SYNC_ACL".to_string(), sync_acl.to_string())])?;
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shape of Arch's `/etc/snapper/configs/home`, read here.
    const ARCH: &str = "SUBVOLUME=\"/home\"\nFSTYPE=\"btrfs\"\nALLOW_USERS=\"\"\nALLOW_GROUPS=\"\"\n# sync users and groups from ALLOW_USERS and ALLOW_GROUPS to .snapshots\nSYNC_ACL=\"no\"\nTIMELINE_CREATE=\"yes\"\n";

    #[test]
    fn the_settings_are_read_the_way_snapper_writes_them() {
        assert_eq!(parse(ARCH), Settings { allow_users: String::new(), sync_acl: "no".into() });
    }

    #[test]
    fn a_user_is_allowed_only_when_named_and_synced() {
        let named = Settings { allow_users: "sam alex".into(), sync_acl: "yes".into() };
        assert!(allows(&named, "alex"));
        assert!(!allows(&named, "al"), "a whole name, not a prefix");
        assert!(!allows(&Settings { sync_acl: "no".into(), ..named }, "alex"), "named but no ACL is not readable");
    }
}
