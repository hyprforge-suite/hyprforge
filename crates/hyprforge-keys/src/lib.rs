//! The suite's keyboard grammar: what a binding is, and what a press means.
//!
//! One parser, one merge policy, one set of rules about which keys may be
//! bound — so `Ctrl+Shift+N` means the same thing, and is spelled the same
//! way, in every app here. That is vision pillar 9 ("one keyboard shortcut
//! grammar, enforced identically across every app") as a crate rather than
//! as a sentence somebody has to remember.
//!
//! Extracted from `hyprforge-files-core` when the image viewer became the
//! second app to need it. The alternative was a copy, and a copy is how two
//! grammars come to disagree about `Ctrl+Plus` while both look right.
//!
//! # The shape
//!
//! - [`Combo`] is a binding — a [`Key`] plus the [`Modifiers`] held. It
//!   parses and renders the same syntax the config files use.
//! - [`KeyPress`] is what the host saw. The host converts its toolkit's
//!   event into one of these; `hyprforge_ui::keys::key_press` is the iced
//!   adapter.
//! - [`Keymap`] turns the second into the first's meaning.
//!
//! # Generic over the action, because every app binds different things
//!
//! An app supplies its own `Action` enum and an [`Bindable`] impl. The
//! keymap knows nothing about what an action *does* — only that it has a
//! stable id the config file can name and a list of default keys.
//!
//! # Two kinds of app, and the axis between them
//!
//! A file manager has a search box, so a bare `n` types the letter n and
//! must not be bindable. An image viewer has nowhere to type, so bare `n`
//! is simply the key for "next" — as it is in every image viewer anyone
//! has used. That is not a preference; it is a property of the app, and
//! neither answer is right for both. [`BareKeys`] is how a host states
//! which it is, and both [`Keymap::resolve`] and [`merge::keymap_with`]
//! consult it.
//!
//! Getting this wrong is quiet: with the file manager's answer forced on a
//! viewer, the viewer's own default bindings are refused at load, and the
//! user is told about a search box their app does not have.
//!
//! # Nothing here may log what was typed
//!
//! A [`KeyPress`] carries the character a key produced. CLAUDE.md's rule —
//! never write anything derived from a keystroke to a log, a panic message
//! or a debug dump — is kept by a hand-written `Debug` in [`press`], which
//! renders that character as `<char>`. A binding *string from a config
//! file* is fine to print; a key the user pressed is not.

pub mod combo;
pub mod keymap;
pub mod merge;
pub mod press;

pub use combo::{Combo, ComboError, Key, Modifiers};
pub use keymap::{BareKeys, Bindable, Keymap, Resolved};
pub use merge::Problem;
pub use press::KeyPress;
