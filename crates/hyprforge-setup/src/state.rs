//! Where an item stands, and the one line the installer reads about it.

/// Where an item stands on this machine.
///
/// Four states, not two, for the same reason the installer's own checks
/// have three: a check that could not run is not a check that passed.
/// Reading `systemctl` timing out as "the service is enabled", or an
/// unparseable `shortcuts.toml` as "no binds yet", is how a setup step
/// reports success for something it never looked at — and, for the
/// second, how an apply would overwrite a file it could not read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    /// Already the way setup would leave it. Nothing to apply.
    Done,
    /// Setup can do this; `what` says what it would change, in words a
    /// person ticking a box can judge.
    Todo { what: String },
    /// Setup cannot do this here, and `why` says what would have to
    /// change first — the component is not installed, the chord is
    /// already someone else's, the config is `hyprland.conf`. Never
    /// applied, by `--yes` or anyone else.
    Unavailable { why: String },
    /// The check itself could not run: a file that exists and will not
    /// parse, a `hyprctl` that did not answer. Never applied either —
    /// applying on top of something unread is exactly the data-loss shape
    /// CLAUDE.md warns about.
    Unknown { why: String },
}

impl State {
    /// The word the installer parses: `done`, `todo`, `unavailable` or
    /// `unknown`. Part of the `--porcelain` contract, so it never changes
    /// spelling.
    pub fn name(&self) -> &'static str {
        match self {
            State::Done => "done",
            State::Todo { .. } => "todo",
            State::Unavailable { .. } => "unavailable",
            State::Unknown { .. } => "unknown",
        }
    }

    /// The sentence that goes with the state; empty for [`State::Done`].
    pub fn reason(&self) -> &str {
        match self {
            State::Done => "",
            State::Todo { what } => what,
            State::Unavailable { why } | State::Unknown { why } => why,
        }
    }

    pub fn is_todo(&self) -> bool {
        matches!(self, State::Todo { .. })
    }
}

/// One `--porcelain` line: `<id>\t<state>\t<reason>`.
///
/// The format the installer parses, pinned by a test in both this crate
/// and the Settings app's `--setup`. A reason carries no tab or newline —
/// a message that quotes an error from another program could have either,
/// and one stray tab would shift every field after it.
pub fn porcelain_line(id: &str, state: &State) -> String {
    let reason: String = state
        .reason()
        .chars()
        .map(|c| if c == '\t' || c == '\n' || c == '\r' { ' ' } else { c })
        .collect();
    format!("{id}\t{}\t{}", state.name(), reason.trim())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The installer splits on tabs and matches these four words. A change
    /// to either is a change to `./hyprforge --status`.
    #[test]
    fn the_porcelain_format_is_id_tab_state_tab_reason() {
        assert_eq!(porcelain_line("bind-files", &State::Done), "bind-files\tdone\t");
        assert_eq!(
            porcelain_line("bind-files", &State::Todo { what: "Bind Super+E".into() }),
            "bind-files\ttodo\tBind Super+E"
        );
        assert_eq!(
            porcelain_line("gtk-portal", &State::Unavailable { why: "not installed".into() }),
            "gtk-portal\tunavailable\tnot installed"
        );
        assert_eq!(
            porcelain_line("wiring", &State::Unknown { why: "unreadable".into() }),
            "wiring\tunknown\tunreadable"
        );
    }

    /// A quoted error message with a tab in it must not add a fourth field.
    #[test]
    fn a_reason_never_carries_a_tab_or_a_newline() {
        let line = porcelain_line("x", &State::Unknown { why: "a\tb\nc".into() });
        assert_eq!(line.split('\t').count(), 3, "{line:?}");
        assert!(!line.contains('\n'));
    }
}
