# hyprforge-secret

A Secret<T> wrapper whose Debug impl renders only a length, never the wrapped value, so every place a password or passphrase is exposed is a single greppable expose() call.

One implementation of "never render this".

A keysym name *is* the character it names, and the same is true of a
typed password and a Wi-Fi passphrase: the moment a `Debug` impl, a
`tracing::debug!(?x)`, or a panic message prints the value, it is on
disk in a barely-encoded form, and in an agent session it reaches the
session transcript through tool output. That rule got written by hand
three separate times in this workspace — the lock screen's typed
password, the Wi-Fi passphrase, and the clipboard's remembered content
— and each of those was a fresh chance to get it wrong: to forget a
field, to `#[derive(Debug)]` over it later, to print the value "just
this once" for a bug report.

`Secret<T>` is the one implementation for the two of those that are
a genuine "hide this, entirely" case — a password and a passphrase,
where the *value* itself is the whole risk and nothing about it is
worth describing. Its `Debug` renders a character count and nothing
else, and `Secret::expose` is the single accessor: `grep expose` in
any crate that depends on this one finds every place a secret leaves
the wrapper.

The clipboard is a different shape of the same problem and is
deliberately *not* built on `Secret<T>` — see
`hyprforge-clipboard`'s `types.rs` for why, and `chars`/`bytes`
for the smaller piece of this crate it does share: the formatting of
"a redacted count", used by a `Debug` impl that describes rather than
hides.

## Where this lives

Part of [Hyprforge](https://github.com/hyprforge-suite/hyprforge), a suite of native Hyprland desktop
applications. This crate is `crates/hyprforge-secret` there; it is published to
crates.io so the suite's applications can be built from their own
repositories, and its API follows the suite. Issues and pull requests go
to the suite repository.

```
cargo add hyprforge-secret
```

This README is rendered from the crate's `//!` documentation by
`tools/crate-readme.py`; edit `src/lib.rs`, not this file.

## Licence

MIT. See `LICENSE`.
