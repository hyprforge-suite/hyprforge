# hyprforge-archive

Reads and writes zip, tar and 7z archives: what is inside one, as a directory tree; extracting from it; and rewriting it.

What is inside a zip, a tar or a 7z — as a directory tree, as bytes,
and as a file that can be written back.

# Why this is its own crate

The file manager browses archives, but it is not the only thing that
wants to: the portal's open/save dialog shows the same listing, a
preview pane would want one member's bytes, and none of them should
have to pull in a GUI toolkit to ask what is in a tarball. So this is
a leaf — `thiserror` and `tracing` and the format libraries, and
nothing of this suite's at all, not even `hyprforge-paths`. Paths
arrive from the caller; this crate never goes looking for one.

# The shape

| Question | Module |
|---|---|
| What kind of archive is this, really? | `format` |
| What is inside it? | `model` |
| How do I get at it? | `backend` |
| What will extracting write, and where? | `extract` |
| How is one created or changed? | `write` |

`backend::ArchiveBackend` is the seam: `StdArchives` is the real
one, and `backend::mock::MockArchives` (behind the `mock` feature)
is what a caller's tests browse instead of arranging real archives on
whatever disk they run on.

# Two things this deliberately does not do

**It does not decide what a file is for.** Whether a `.docx` — a zip,
technically — should be browsed as an archive or opened in a word
processor is the caller's question; `format::Format::plausible_by_name`
answers only "could this be opened as an archive", and the list it
consults is short on purpose.

**It does not stream a member it has not sized.** `read_member`
answers with a `Vec<u8>`, which is the wrong shape for a
four-gigabyte disk image inside a tar, and the right one for
everything a file manager does with a single member today (preview
it, hand it to an application, copy it out). Extraction, where the
sizes genuinely are unbounded, goes through `extract` and never
holds more than one member at a time.

## Where this lives

Part of [Hyprforge](https://github.com/hyprforge-suite/hyprforge), a suite of native Hyprland desktop
applications. This crate is `crates/hyprforge-archive` there; it is published to
crates.io so the suite's applications can be built from their own
repositories, and its API follows the suite. Issues and pull requests go
to the suite repository.

```
cargo add hyprforge-archive
```

This README is rendered from the crate's `//!` documentation by
`tools/crate-readme.py`; edit `src/lib.rs`, not this file.

## Licence

MIT. See `LICENSE`.
