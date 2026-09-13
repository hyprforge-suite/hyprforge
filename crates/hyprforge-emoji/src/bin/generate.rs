//! Regenerates `src/generated.rs` from `data/emoji-test.txt`.
//!
//! This is a bin target inside `hyprforge-emoji` itself rather than a
//! `build.rs`, deliberately: a `build.rs` would re-run this parse on
//! every build of every crate that depends on `hyprforge-emoji`, for a
//! file that changes only when Unicode publishes a new version. Running
//! it by hand and committing the result means normal builds — and
//! `check.sh` — never touch `data/emoji-test.txt` at all; only a human
//! re-running this binary does.
//!
//! To regenerate (e.g. after updating `data/emoji-test.txt` to a newer
//! Unicode release):
//!
//! ```sh
//! cargo run -p hyprforge-emoji --bin generate
//! ```
//!
//! It reads `data/emoji-test.txt` next to this crate's `Cargo.toml` (via
//! `CARGO_MANIFEST_DIR`, never a cargo registry path — that copy is
//! exactly why `data/emoji-test.txt` is committed) and overwrites
//! `src/generated.rs` in place. Uses only `std`.
//!
//! # How the base/tone relationship is derived
//!
//! See `lib.rs`'s crate doc comment for the full reasoning; this is the
//! mechanics. Everything here comes from grouping what
//! `data/emoji-test.txt` actually contains — nothing is synthesised by
//! appending a modifier codepoint to a base character, because that is
//! wrong in general (a two-person ZWJ sequence can carry a tone on each
//! human component, and the modifier's position in the sequence is not
//! something to guess).
//!
//! 1. Parse every `fully-qualified` line, keeping its full codepoint
//!    sequence, in file (CLDR) order.
//! 2. For each entry, strip any Fitzpatrick modifier codepoints
//!    (`U+1F3FB..=U+1F3FF`) out of its codepoint sequence — what is left
//!    is its "family key". Entries that share a family key are the same
//!    gesture/person at different tones (or the neutral form).
//! 3. Group entries by family key. Walk the file once more, and for each
//!    family key seen for the first time, decide what — if anything —
//!    represents it in the table:
//!    - If the family has no toned member at all, the neutral entry
//!      represents itself, with no tone variants.
//!    - If every toned member carries exactly one modifier (a
//!      single-person gesture), the family supports tones: the neutral
//!      member is the canonical entry if one exists; otherwise (17
//!      families as of Unicode 17.0 — e.g. "index pointing up", "victory
//!      hand" — RGI only defines the toned forms) the `medium skin
//!      tone` member stands in as canonical, since it is itself a real,
//!      valid, already-qualified emoji and not a synthesised one.
//!    - If any toned member carries *two* modifiers (a two-person
//!      sequence, e.g. "handshake", "people holding hands"), a single
//!      [`Tone`] cannot pick between the independent tones of each
//!      person, so tone modeling is deliberately not attempted: the
//!      neutral member represents the family with no tone variants if
//!      one exists, and the family is dropped from the table entirely if
//!      it does not (rather than picking one arbitrary two-tone
//!      combination to stand in for a concept that has no neutral
//!      rendering, which would misrepresent it).

use std::collections::HashMap;
use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;

/// The five Fitzpatrick skin-tone modifier codepoints, in increasing
/// tone order (this is also the order `Tone`'s variants are declared in,
/// in `lib.rs`, and the order variants are emitted per entry).
const SKIN_TONES: [u32; 5] = [0x1F3FB, 0x1F3FC, 0x1F3FD, 0x1F3FE, 0x1F3FF];

fn is_skin_tone(cp: u32) -> bool {
    SKIN_TONES.contains(&cp)
}

/// A single parsed `fully-qualified` line.
#[derive(Clone)]
struct RawEntry {
    codepoints: Vec<u32>,
    emoji: String,
    name: String,
    group: String,
    subgroup: String,
}

/// U+FE0F VARIATION SELECTOR-16 (request emoji presentation). Some
/// symbols need it to render as an emoji when they stand alone (e.g.
/// "index pointing up" is `261D FE0F`), but the *toned* sequence for the
/// very same emoji never carries it — the tone modifier already forces
/// emoji presentation, so Unicode omits the otherwise-required FE0F
/// (`261D 1F3FB`, no FE0F). Left in the family key, that discrepancy
/// makes the neutral and toned forms of "index pointing up" hash to two
/// different keys and produces two separate, wrongly-unlinked table
/// rows. Stripping FE0F is safe generally: it is purely a presentation
/// selector and never distinguishes one emoji from another semantically
/// (that is exactly why "fully-qualified" vs "minimally-qualified" is
/// its own status field rather than part of identity).
const VARIATION_SELECTOR_16: u32 = 0xFE0F;

impl RawEntry {
    fn family_key(&self) -> Vec<u32> {
        self.codepoints
            .iter()
            .copied()
            .filter(|cp| !is_skin_tone(*cp) && *cp != VARIATION_SELECTOR_16)
            .collect()
    }

    /// How many skin-tone modifier codepoints this entry's sequence
    /// carries: 0 (neutral), 1 (single-person gesture at one tone), or
    /// 2 (a two-person sequence with a tone on each side).
    fn modifier_count(&self) -> usize {
        self.codepoints.iter().filter(|cp| is_skin_tone(**cp)).count()
    }

    /// The single tone this entry carries, if `modifier_count() == 1`.
    fn single_tone_index(&self) -> Option<usize> {
        let mods: Vec<u32> = self
            .codepoints
            .iter()
            .copied()
            .filter(|cp| is_skin_tone(*cp))
            .collect();
        match mods.as_slice() {
            [m] => SKIN_TONES.iter().position(|t| t == m),
            _ => None,
        }
    }
}

/// One row of the generated table: a canonical (grid) entry, plus its
/// five tone variants if it has them (indexed the same way `Tone`'s
/// discriminants are: light, medium-light, medium, medium-dark, dark).
struct Row {
    emoji: String,
    name: String,
    group: String,
    subgroup: String,
    tones: Option<[String; 5]>,
}

fn main() {
    let manifest_dir =
        env::var("CARGO_MANIFEST_DIR").expect("run this via `cargo run`, not directly");
    let data_path = PathBuf::from(&manifest_dir).join("data/emoji-test.txt");
    let source = fs::read_to_string(&data_path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", data_path.display()));

    let (raw, all_groups) = parse(&source);
    let rows = build_rows(&raw);

    let out_path = PathBuf::from(&manifest_dir).join("src/generated.rs");
    fs::write(&out_path, render(&rows, &all_groups))
        .unwrap_or_else(|e| panic!("failed to write {}: {e}", out_path.display()));

    let with_tones = rows.iter().filter(|r| r.tones.is_some()).count();
    eprintln!(
        "wrote {} ({} grid entries, {} support tones, {} groups)",
        out_path.display(),
        rows.len(),
        with_tones,
        all_groups.len()
    );
}

/// Parse every `fully-qualified` line into a [`RawEntry`], in file
/// order, plus the full list of group headers in file order (unfiltered
/// — see `groups()` in `lib.rs` for why the group list itself is not
/// filtered the way the entry table is).
fn parse(source: &str) -> (Vec<RawEntry>, Vec<String>) {
    let mut entries = Vec::new();
    let mut all_groups: Vec<String> = Vec::new();
    let mut current_group = String::new();
    let mut current_subgroup = String::new();

    for (line_no, line) in source.lines().enumerate() {
        let line_no = line_no + 1;
        let trimmed = line.trim();

        if let Some(rest) = trimmed.strip_prefix("# group:") {
            current_group = rest.trim().to_string();
            all_groups.push(current_group.clone());
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("# subgroup:") {
            current_subgroup = rest.trim().to_string();
            continue;
        }
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        // A data line: "<codepoints> ; <status> # <emoji> <version> <name>"
        let mut parts = trimmed.splitn(2, ';');
        let codepoints_field = parts
            .next()
            .unwrap_or_else(|| panic!("line {line_no}: missing codepoint field"));
        let rest = parts
            .next()
            .unwrap_or_else(|| panic!("line {line_no}: missing ';' separator"));

        let mut rest_parts = rest.splitn(2, '#');
        let status = rest_parts
            .next()
            .unwrap_or_else(|| panic!("line {line_no}: missing status field"))
            .trim();
        let comment = rest_parts
            .next()
            .unwrap_or_else(|| panic!("line {line_no}: missing '#' comment field"))
            .trim();

        if status != "fully-qualified" {
            continue;
        }

        let codepoints: Vec<u32> = codepoints_field
            .split_whitespace()
            .map(|hex| {
                u32::from_str_radix(hex, 16)
                    .unwrap_or_else(|e| panic!("line {line_no}: bad codepoint {hex:?}: {e}"))
            })
            .collect();
        if codepoints.is_empty() {
            panic!("line {line_no}: no codepoints parsed");
        }

        // The emoji character is built from the codepoints, not parsed
        // out of the comment: the comment's rendering is illustrative
        // and the codepoint list is the actual source of truth.
        let emoji: String = codepoints
            .iter()
            .map(|&cp| {
                char::from_u32(cp)
                    .unwrap_or_else(|| panic!("line {line_no}: {cp:#X} is not a valid char"))
            })
            .collect();

        // comment == "<rendered emoji> E<version> <name...>". Skip the
        // rendered emoji (first whitespace-separated token) and the
        // version token (the next token, which always starts with 'E'
        // followed by digits), then the remainder — rejoined on single
        // spaces — is the name.
        let mut tokens = comment.split_whitespace();
        tokens.next().unwrap_or_else(|| {
            panic!("line {line_no}: comment has no rendered-emoji token: {comment:?}")
        });
        let version = tokens.next().unwrap_or_else(|| {
            panic!("line {line_no}: comment has no version token: {comment:?}")
        });
        if !is_version_token(version) {
            panic!("line {line_no}: expected a version token like \"E1.0\", got {version:?}");
        }
        let name = tokens.collect::<Vec<_>>().join(" ");
        if name.is_empty() {
            panic!("line {line_no}: parsed an empty name from comment {comment:?}");
        }

        if current_group.is_empty() || current_subgroup.is_empty() {
            panic!("line {line_no}: data line before any # group:/# subgroup: header");
        }

        entries.push(RawEntry {
            codepoints,
            emoji,
            name,
            group: current_group.clone(),
            subgroup: current_subgroup.clone(),
        });
    }

    if entries.is_empty() {
        panic!("parsed zero entries — did the file format change?");
    }
    if all_groups.len() != 10 {
        panic!(
            "expected 10 '# group:' headers, found {} — did the file format change?",
            all_groups.len()
        );
    }

    (entries, all_groups)
}

/// `"E1.0"`, `"E0.6"`, `"E14.0"` etc: `E` followed by digits, optionally
/// a `.` and more digits.
fn is_version_token(token: &str) -> bool {
    let Some(rest) = token.strip_prefix('E') else {
        return false;
    };
    let mut saw_digit = false;
    let mut saw_dot = false;
    for c in rest.chars() {
        if c.is_ascii_digit() {
            saw_digit = true;
        } else if c == '.' && !saw_dot {
            saw_dot = true;
        } else {
            return false;
        }
    }
    saw_digit
}

/// The five tone-suffix phrases CLDR appends to a single-person
/// gesture's base name, in the same order as [`SKIN_TONES`]. Used only
/// to recover a clean base name for the 17 families that have no
/// neutral member at all (see the module doc comment) — every other
/// entry's name already comes straight from its own, un-suffixed line.
const TONE_SUFFIXES: [&str; 5] = [
    ": light skin tone",
    ": medium-light skin tone",
    ": medium skin tone",
    ": medium-dark skin tone",
    ": dark skin tone",
];

fn strip_tone_suffix(name: &str) -> &str {
    for suffix in TONE_SUFFIXES {
        if let Some(stripped) = name.strip_suffix(suffix) {
            return stripped;
        }
    }
    name
}

/// Build the grid rows: one per family, in the order each family first
/// appears in `raw`, with tone variants attached per the rule in this
/// file's module doc comment.
fn build_rows(raw: &[RawEntry]) -> Vec<Row> {
    let mut families: HashMap<Vec<u32>, Vec<&RawEntry>> = HashMap::new();
    for entry in raw {
        families.entry(entry.family_key()).or_default().push(entry);
    }

    let mut rows = Vec::new();
    let mut emitted_keys: std::collections::HashSet<Vec<u32>> = std::collections::HashSet::new();

    for entry in raw {
        let key = entry.family_key();
        if !emitted_keys.insert(key.clone()) {
            continue; // this family already produced its row
        }

        let members = &families[&key];
        let neutral = members.iter().find(|m| m.modifier_count() == 0);
        let max_modifiers = members.iter().map(|m| m.modifier_count()).max().unwrap_or(0);

        if max_modifiers >= 2 {
            // Two-person sequence: a single Tone can't disambiguate two
            // independent people's tones, so tone modeling is skipped.
            // Represent the family only if it has a neutral form;
            // otherwise drop it rather than guess a stand-in.
            if let Some(neutral) = neutral {
                rows.push(Row {
                    emoji: neutral.emoji.clone(),
                    name: neutral.name.clone(),
                    group: neutral.group.clone(),
                    subgroup: neutral.subgroup.clone(),
                    tones: None,
                });
            }
            continue;
        }

        let single_tone_members: Vec<&&RawEntry> = members
            .iter()
            .filter(|m| m.modifier_count() == 1)
            .collect();

        if single_tone_members.is_empty() {
            // No toned variants at all: plain entry, must have a
            // neutral form (modifier_count() == 0 is the only option
            // left besides >= 2, already handled above).
            let neutral = neutral.unwrap_or_else(|| {
                panic!(
                    "family with key {key:?} has neither a neutral form nor toned variants"
                )
            });
            rows.push(Row {
                emoji: neutral.emoji.clone(),
                name: neutral.name.clone(),
                group: neutral.group.clone(),
                subgroup: neutral.subgroup.clone(),
                tones: None,
            });
            continue;
        }

        // Single-person gesture with tone variants. Build the 5-slot
        // array indexed by tone order; every family observed in
        // Unicode 17.0 has all five, but require it explicitly rather
        // than silently leaving a gap that would panic obscurely later.
        let mut variants: [Option<String>; 5] = Default::default();
        for member in &single_tone_members {
            let idx = member.single_tone_index().unwrap_or_else(|| {
                panic!(
                    "entry {:?} has an unrecognised single modifier",
                    member.name
                )
            });
            if variants[idx].is_some() {
                panic!("family with key {key:?} has two entries for the same tone");
            }
            variants[idx] = Some(member.emoji.clone());
        }
        let tones: [String; 5] = variants.map(|v| {
            v.unwrap_or_else(|| {
                panic!("family with key {key:?} is missing a tone variant — expected all 5")
            })
        });

        let (canonical_emoji, canonical_name, canonical_group, canonical_subgroup) =
            if let Some(neutral) = neutral {
                (
                    neutral.emoji.clone(),
                    neutral.name.clone(),
                    neutral.group.clone(),
                    neutral.subgroup.clone(),
                )
            } else {
                // No neutral form is defined by Unicode for this
                // family (17 cases, e.g. "index pointing up"): fall
                // back to the medium-tone member as the canonical,
                // displayed form. It is a real, already-qualified
                // emoji — not a synthesised sequence — chosen only
                // because "medium" is the least tone-specific of the
                // five real options to show by default.
                let medium = single_tone_members
                    .iter()
                    .find(|m| m.single_tone_index() == Some(2))
                    .expect("medium tone variant must exist alongside the other 4");
                (
                    medium.emoji.clone(),
                    strip_tone_suffix(&medium.name).to_string(),
                    medium.group.clone(),
                    medium.subgroup.clone(),
                )
            };

        rows.push(Row {
            emoji: canonical_emoji,
            name: canonical_name,
            group: canonical_group,
            subgroup: canonical_subgroup,
            tones: Some(tones),
        });
    }

    rows
}

/// Emit `src/generated.rs`. Every string is emitted via `{:?}`
/// (`Debug`), which is what makes this safe for arbitrary emoji and
/// names: it produces a valid, escaped Rust string literal for any
/// `&str`, handling embedded quotes, backslashes, and non-printable
/// codepoints (`Debug` renders those as `\u{...}` escapes, which parse
/// back to the exact same `char` — so a ZWJ or variation selector round
/// -trips correctly even though it looks escaped in the source file).
/// This is exactly the trap the task called out: a name with a quote or
/// apostrophe, or a multi-codepoint emoji, must not produce source that
/// fails to compile or that compiles with a mangled character.
fn render(rows: &[Row], all_groups: &[String]) -> String {
    let mut out = String::new();
    out.push_str(
        "// GENERATED FILE. Do not edit by hand.\n\
         //\n\
         // Produced by `cargo run -p hyprforge-emoji --bin generate` from\n\
         // `data/emoji-test.txt`. See that binary's doc comment for what it\n\
         // does and why it is a checked-in table rather than a build.rs.\n\n\
         use crate::Emoji;\n\n",
    );

    out.push_str("pub(crate) const GROUPS: &[&str] = &[\n");
    for group in all_groups {
        writeln!(out, "    {group:?},").unwrap();
    }
    out.push_str("];\n\n");

    writeln!(out, "pub(crate) const EMOJIS: &[Emoji] = &[").unwrap();
    for row in rows {
        match &row.tones {
            None => writeln!(
                out,
                "    Emoji {{ emoji: {:?}, name: {:?}, group: {:?}, subgroup: {:?}, tones: None }},",
                row.emoji, row.name, row.group, row.subgroup
            )
            .unwrap(),
            Some(tones) => writeln!(
                out,
                "    Emoji {{ emoji: {:?}, name: {:?}, group: {:?}, subgroup: {:?}, tones: Some([{:?}, {:?}, {:?}, {:?}, {:?}]) }},",
                row.emoji, row.name, row.group, row.subgroup,
                tones[0], tones[1], tones[2], tones[3], tones[4],
            )
            .unwrap(),
        }
    }
    out.push_str("];\n");

    out
}
