# hyprforge-emoji

CLDR-ordered emoji data and pure search, generated from Unicode's emoji-test.txt, for building an emoji picker.

CLDR-ordered emoji data and a pure search function, for building an
emoji picker on top.

Follows the same split the D-Bus-backed modules in this workspace use
(see `CLAUDE.md`, "The shape a D-Bus-backed module takes"): even
though there is no D-Bus here, the same rule applies — **the model is
plain data, and the decisions are pure functions over it.** `Emoji`
is a flat struct, `all` and `groups` hand back slices, and
`search` is a pure function from a query and a table to a ranked
`Vec`. Nothing in this crate touches the filesystem, a network, a
runtime, or Hyprland — a picker UI is built *on* this, not the other
way round. The long-press interaction that would let a person pick a
tone, and where their chosen default tone is stored, are both that
picker's job, not this crate's — this crate only makes both easy: see
`Emoji::tone` and `Emoji::tone_variants`.

# Where the data comes from

`data/emoji-test.txt`
is a verbatim copy of Unicode's `emoji-test.txt` (UTS #51 test data,
Unicode 17.0), which the file itself describes as being "in CLDR
order... recommended (but not required) for keyboard palettes." That
is the order `all` and `search`'s tie-breaking preserve: it is
not sorted alphabetically or by codepoint, because CLDR order is what
makes a picker's default view look intentional rather than random.

`src/generated.rs`
is generated from that file and committed alongside it — see
`src/bin/generate.rs` for the generator, exactly how it derives the
table below, and how to re-run it. Nothing in this crate reads
`data/emoji-test.txt` at build time or runtime; only the generator
does, and only when a human runs it deliberately.

# What made it into the table

The source file marks every entry `component`, `fully-qualified`,
`minimally-qualified`, or `unqualified`. Only `fully-qualified`
entries are considered at all:

- `minimally-qualified` and `unqualified` are the *same* emoji as a
  `fully-qualified` entry elsewhere in the file, just missing a
  variation selector (see UTS #51 ED-18a/ED-19). Keeping them would
  put visual duplicates of the same character in the table under the
  same name.
- `component` entries (9 of them: the five Fitzpatrick skin-tone
  modifiers and four hair-style modifiers) are modifier codepoints,
  not standalone emoji a person picks from a grid. Excluding them
  also happens to empty out the file's "Component" group entirely —
  see `groups` for why that group is still listed.

# The skin-tone relationship

The fully-qualified set is 3944 entries, and 2030 of those are tone
variants: the same gesture or person repeated once per Fitzpatrick
modifier (`U+1F3FB`..=`U+1F3FF`), sometimes twice per entry for
two-person sequences like a handshake. This crate does **not**
flatten those away, and does not synthesise a base character by
appending a modifier codepoint at the call site either — that is
wrong in general, because a ZWJ sequence can carry an independent
tone on each human component, and where the modifier belongs in the
sequence is not something to guess. Instead, `Emoji` models the
*relationship*, derived entirely from grouping what
`data/emoji-test.txt` actually contains:

- The grid (`all`) holds **one entry per emoji concept**, in CLDR
  order, exactly as before.
- `Emoji::supports_tones` says whether that concept has tone
  variants, and `Emoji::tone` / `Emoji::tone_variants` hand back
  the real, already-qualified character for each `Tone` — never a
  sequence built by this crate.
- Two-person sequences (19 families: "handshake" via the newer
  directional-hands ZWJ combo, "people holding hands", "kiss: woman,
  man", and similar) are **not** modeled as tone-capable, because one
  `Tone` cannot pick between two independent people's tones. Where
  such a family has a neutral, toneless form (7 of the 19 — e.g.
  "kiss: woman, man" itself), that form is kept in the grid with
  `supports_tones() == false`. Where it does not (12 of the 19 —
  e.g. the two-person "people wrestling" ZWJ combo, as opposed to the
  older single-glyph 🤼 pictograph, which *does* get tones — see
  below), the family is dropped from the grid entirely rather than
  picking one arbitrary two-tone combination to stand in for a
  concept Unicode never gave a neutral rendering.
- A handful of single-person symbols (e.g. "index pointing up",
  "hand with fingers splayed") need a variation selector
  (`U+FE0F`) to render as an emoji when shown alone, but Unicode
  omits that selector from the *toned* sequences for the very same
  emoji, since the tone modifier already forces emoji presentation.
  Naively, that makes the neutral and toned forms hash to different
  "family keys" and look like unrelated entries; the generator
  strips `U+FE0F` before grouping so they merge correctly. As of
  Unicode 17.0, every single-person tone family turns out to have a
  genuine neutral form once this is accounted for — none currently
  need a same-tone-as-canonical stand-in — but the generator still
  has a documented fallback (the `medium skin tone` member, itself a
  real, already-qualified emoji, never a synthesised one) for the
  day a future Unicode release adds a tone-only family, so that case
  fails loudly rather than silently if it does.

**After all of this, `all` returns 1914 entries, of which 323
support tones.** `EMOJI_COUNT` and `tone_capable_count` are both
asserted against the generated table's own contents in this crate's
tests, so this doc comment cannot silently drift from the data.

# Search

`search` ranks by how good a name match is, not by table order:
exact name match, then a whole leading word, then a whole word
anywhere else in the name, then any other substring. Ties within a
tier keep CLDR order. Because the table already holds one row per
concept rather than one per tone, searching by name naturally finds
that one row — there is nothing tone-specific for `search` to filter,
since a query like "wave" is answered by "waving hand" once, not five
times. See `search`'s doc comment for the full ranking rule.

## Where this lives

Part of [Hyprforge](https://github.com/hyprforge-suite/hyprforge), a suite of native Hyprland desktop
applications. This crate is `crates/hyprforge-emoji` there; it is published to
crates.io so the suite's applications can be built from their own
repositories, and its API follows the suite. Issues and pull requests go
to the suite repository.

```
cargo add hyprforge-emoji
```

This README is rendered from the crate's `//!` documentation by
`tools/crate-readme.py`; edit `src/lib.rs`, not this file.

## Licence

MIT. See `LICENSE`.
