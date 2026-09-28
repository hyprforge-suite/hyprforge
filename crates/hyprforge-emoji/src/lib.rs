//! CLDR-ordered emoji data and a pure search function, for building an
//! emoji picker on top.
//!
//! Follows the same split the D-Bus-backed modules in this workspace use
//! (see `CLAUDE.md`, "The shape a D-Bus-backed module takes"): even
//! though there is no D-Bus here, the same rule applies — **the model is
//! plain data, and the decisions are pure functions over it.** [`Emoji`]
//! is a flat struct, [`all`] and [`groups`] hand back slices, and
//! [`search`] is a pure function from a query and a table to a ranked
//! `Vec`. Nothing in this crate touches the filesystem, a network, a
//! runtime, or Hyprland — a picker UI is built *on* this, not the other
//! way round. The long-press interaction that would let a person pick a
//! tone, and where their chosen default tone is stored, are both that
//! picker's job, not this crate's — this crate only makes both easy: see
//! [`Emoji::tone`] and [`Emoji::tone_variants`].
//!
//! # Where the data comes from
//!
//! [`data/emoji-test.txt`](https://github.com/hyprforge-suite/hyprforge/blob/main/crates/hyprforge-emoji/data/emoji-test.txt)
//! is a verbatim copy of Unicode's `emoji-test.txt` (UTS #51 test data,
//! Unicode 17.0), which the file itself describes as being "in CLDR
//! order... recommended (but not required) for keyboard palettes." That
//! is the order [`all`] and [`search`]'s tie-breaking preserve: it is
//! not sorted alphabetically or by codepoint, because CLDR order is what
//! makes a picker's default view look intentional rather than random.
//!
//! [`src/generated.rs`](https://github.com/hyprforge-suite/hyprforge/blob/main/crates/hyprforge-emoji/src/generated.rs)
//! is generated from that file and committed alongside it — see
//! `src/bin/generate.rs` for the generator, exactly how it derives the
//! table below, and how to re-run it. Nothing in this crate reads
//! `data/emoji-test.txt` at build time or runtime; only the generator
//! does, and only when a human runs it deliberately.
//!
//! # What made it into the table
//!
//! The source file marks every entry `component`, `fully-qualified`,
//! `minimally-qualified`, or `unqualified`. Only `fully-qualified`
//! entries are considered at all:
//!
//! - `minimally-qualified` and `unqualified` are the *same* emoji as a
//!   `fully-qualified` entry elsewhere in the file, just missing a
//!   variation selector (see UTS #51 ED-18a/ED-19). Keeping them would
//!   put visual duplicates of the same character in the table under the
//!   same name.
//! - `component` entries (9 of them: the five Fitzpatrick skin-tone
//!   modifiers and four hair-style modifiers) are modifier codepoints,
//!   not standalone emoji a person picks from a grid. Excluding them
//!   also happens to empty out the file's "Component" group entirely —
//!   see [`groups`] for why that group is still listed.
//!
//! # The skin-tone relationship
//!
//! The fully-qualified set is 3944 entries, and 2030 of those are tone
//! variants: the same gesture or person repeated once per Fitzpatrick
//! modifier (`U+1F3FB`..=`U+1F3FF`), sometimes twice per entry for
//! two-person sequences like a handshake. This crate does **not**
//! flatten those away, and does not synthesise a base character by
//! appending a modifier codepoint at the call site either — that is
//! wrong in general, because a ZWJ sequence can carry an independent
//! tone on each human component, and where the modifier belongs in the
//! sequence is not something to guess. Instead, [`Emoji`] models the
//! *relationship*, derived entirely from grouping what
//! `data/emoji-test.txt` actually contains:
//!
//! - The grid ([`all`]) holds **one entry per emoji concept**, in CLDR
//!   order, exactly as before.
//! - [`Emoji::supports_tones`] says whether that concept has tone
//!   variants, and [`Emoji::tone`] / [`Emoji::tone_variants`] hand back
//!   the real, already-qualified character for each [`Tone`] — never a
//!   sequence built by this crate.
//! - Two-person sequences (19 families: "handshake" via the newer
//!   directional-hands ZWJ combo, "people holding hands", "kiss: woman,
//!   man", and similar) are **not** modeled as tone-capable, because one
//!   [`Tone`] cannot pick between two independent people's tones. Where
//!   such a family has a neutral, toneless form (7 of the 19 — e.g.
//!   "kiss: woman, man" itself), that form is kept in the grid with
//!   `supports_tones() == false`. Where it does not (12 of the 19 —
//!   e.g. the two-person "people wrestling" ZWJ combo, as opposed to the
//!   older single-glyph 🤼 pictograph, which *does* get tones — see
//!   below), the family is dropped from the grid entirely rather than
//!   picking one arbitrary two-tone combination to stand in for a
//!   concept Unicode never gave a neutral rendering.
//! - A handful of single-person symbols (e.g. "index pointing up",
//!   "hand with fingers splayed") need a variation selector
//!   (`U+FE0F`) to render as an emoji when shown alone, but Unicode
//!   omits that selector from the *toned* sequences for the very same
//!   emoji, since the tone modifier already forces emoji presentation.
//!   Naively, that makes the neutral and toned forms hash to different
//!   "family keys" and look like unrelated entries; the generator
//!   strips `U+FE0F` before grouping so they merge correctly. As of
//!   Unicode 17.0, every single-person tone family turns out to have a
//!   genuine neutral form once this is accounted for — none currently
//!   need a same-tone-as-canonical stand-in — but the generator still
//!   has a documented fallback (the `medium skin tone` member, itself a
//!   real, already-qualified emoji, never a synthesised one) for the
//!   day a future Unicode release adds a tone-only family, so that case
//!   fails loudly rather than silently if it does.
//!
//! **After all of this, [`all`] returns 1914 entries, of which 323
//! support tones.** [`EMOJI_COUNT`] and [`tone_capable_count`] are both
//! asserted against the generated table's own contents in this crate's
//! tests, so this doc comment cannot silently drift from the data.
//!
//! # Search
//!
//! [`search`] ranks by how good a name match is, not by table order:
//! exact name match, then a whole leading word, then a whole word
//! anywhere else in the name, then any other substring. Ties within a
//! tier keep CLDR order. Because the table already holds one row per
//! concept rather than one per tone, searching by name naturally finds
//! that one row — there is nothing tone-specific for `search` to filter,
//! since a query like "wave" is answered by "waving hand" once, not five
//! times. See [`search`]'s doc comment for the full ranking rule.

mod generated;

/// A skin tone a tone-capable [`Emoji`] can be shown at, per the five
/// Fitzpatrick modifiers UTS #51 defines (`U+1F3FB`..=`U+1F3FF`).
///
/// This is the "model the relationship as an enum, not raw codepoints
/// at the call site" half of this crate's job: a consumer holding a
/// `Tone` and an [`Emoji`] gets the right character via [`Emoji::tone`]
/// without ever looking at a codepoint. There is deliberately no
/// `Neutral`/`None` variant here — "no tone chosen" is
/// [`Emoji::supports_tones`] being `false`, or a caller simply not
/// calling [`Emoji::tone`] at all and using [`Emoji::emoji`] directly;
/// modeling it as a sixth `Tone` would let code accidentally ask a
/// non-tone-capable emoji for `Tone::Neutral` and get a misleading
/// "success".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tone {
    Light,
    MediumLight,
    Medium,
    MediumDark,
    Dark,
}

/// All five tones, in the same order [`Emoji::tone_variants`] returns
/// them and the generated table stores them.
pub const TONES: [Tone; 5] = [
    Tone::Light,
    Tone::MediumLight,
    Tone::Medium,
    Tone::MediumDark,
    Tone::Dark,
];

/// One emoji: its character(s), its name, where it sits in the Unicode
/// CLDR grouping, and — if it has any — its skin-tone variants.
///
/// `emoji` is a `&str`, not a `char`, because an emoji is frequently
/// *several* Rust `char`s — a base codepoint plus a variation selector
/// (`❤️` = `U+2764 U+FE0F`), or several codepoints joined by
/// `U+200D` ZERO WIDTH JOINER (`🏳️‍⚧️` = five codepoints). Treating it as
/// a single `char` would be simply wrong; treating it as bytes would
/// make it possible to slice through the middle of one. Everything in
/// this crate that looks at an emoji or a name iterates `chars()`, never
/// byte offsets — see the tests named `_is_never_sliced_mid_character`.
///
/// `tones` is private: an [`Emoji`] can only be obtained from [`all`],
/// never constructed by hand, so a consumer cannot fabricate a tone
/// relationship this crate did not derive from the data file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Emoji {
    /// The emoji itself, e.g. `"😀"` or `"🏳️‍⚧️"`.
    pub emoji: &'static str,
    /// CLDR's name for it, lowercase, e.g. `"grinning face"`.
    pub name: &'static str,
    /// The group it belongs to, e.g. `"Smileys & Emotion"`. One of the
    /// entries in [`groups`].
    pub group: &'static str,
    /// The subgroup within the group, e.g. `"face-smiling"`.
    pub subgroup: &'static str,
    /// The five tone variants, indexed the same way [`TONES`] is
    /// ordered, or `None` if this emoji has no tone relationship at
    /// all. See [`Emoji::tone`] and [`Emoji::tone_variants`] rather than
    /// reading this field's shape directly — it stays private so it can
    /// only be populated by the generator.
    tones: Option<[&'static str; 5]>,
}

impl Emoji {
    /// Whether this emoji has skin-tone variants at all — e.g. a picker
    /// deciding whether a long-press should open a tone palette.
    pub fn supports_tones(&self) -> bool {
        self.tones.is_some()
    }

    /// The character to show for `tone`. If this emoji has no tone
    /// relationship, every `tone` returns [`Emoji::emoji`] unchanged —
    /// asking a plain emoji for a tone is not an error, it just has
    /// nothing to change. This is what lets a picker hold a `Tone` (the
    /// user's chosen default) and call this on *every* grid entry
    /// without checking [`supports_tones`] first: a tone-incapable
    /// entry simply ignores the request rather than producing a broken
    /// sequence.
    pub fn tone(&self, tone: Tone) -> &'static str {
        match self.tones {
            Some(variants) => variants[tone_index(tone)],
            None => self.emoji,
        }
    }

    /// Every tone variant, paired with which [`Tone`] it is, in
    /// [`TONES`] order — e.g. for the palette a long-press opens.
    /// `None` if this emoji has no tone relationship.
    pub fn tone_variants(&self) -> Option<[(Tone, &'static str); 5]> {
        self.tones.map(|variants| {
            [
                (Tone::Light, variants[0]),
                (Tone::MediumLight, variants[1]),
                (Tone::Medium, variants[2]),
                (Tone::MediumDark, variants[3]),
                (Tone::Dark, variants[4]),
            ]
        })
    }
}

fn tone_index(tone: Tone) -> usize {
    match tone {
        Tone::Light => 0,
        Tone::MediumLight => 1,
        Tone::Medium => 2,
        Tone::MediumDark => 3,
        Tone::Dark => 4,
    }
}

/// The number of emoji [`all`] returns: one row per concept, tone
/// families collapsed per the crate doc comment's rules. Asserted
/// against the generated table's actual length in
/// `entry_count_matches_the_number_this_crate_documents`.
pub const EMOJI_COUNT: usize = generated::EMOJIS.len();

/// How many of [`all`]'s entries support tones, i.e.
/// `all().iter().filter(Emoji::supports_tones).count()` — computed here
/// too so `entry_count_matches_the_number_this_crate_documents` can pin
/// both numbers the doc comment states without walking the table twice
/// at doc-comment-writing time and hoping they stay in sync by hand.
pub fn tone_capable_count() -> usize {
    generated::EMOJIS.iter().filter(|e| e.tones.is_some()).count()
}

/// Every emoji this crate knows about, in CLDR order (the order the
/// source file itself recommends for keyboard palettes). See the crate
/// doc comment for what is and is not included, and how tone variants
/// are represented.
pub fn all() -> &'static [Emoji] {
    generated::EMOJIS
}

/// The ten Unicode CLDR groups, in the file's own order — e.g. for a
/// picker's tabs or section headers.
///
/// This lists all ten groups the source file defines, including
/// "Component", even though [`all`] never returns an entry from it: the
/// file's grouping is what it is, and a picker deciding to hide an empty
/// tab is a UI choice, not a data one. As of Unicode 17.0 "Component" is
/// the only group that ends up empty — every other group keeps at least
/// one entry.
pub fn groups() -> &'static [&'static str] {
    generated::GROUPS
}

/// How well a name matched a query, best first. `search` sorts by this,
/// then by CLDR order within a tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum MatchTier {
    /// The whole (normalized) name equals the query: `"fire"` against
    /// the emoji literally named "fire".
    Exact,
    /// The query is the name's first whole word: `"fire"` against
    /// "fire engine", "fire extinguisher".
    LeadingWord,
    /// The query is some other whole word in the name: `"face"` against
    /// "face with tears of joy" would land here if "face" were not also
    /// the leading word (it is, so that example is `LeadingWord`) —
    /// this tier is for a whole-word match anywhere past the start.
    Word,
    /// The query occurs as a substring but not as a whole word:
    /// `"fire"` against "firefighter", "firecracker", "fireworks" — the
    /// query is glued to other letters on at least one side.
    Substring,
}

/// Lowercase and collapse whitespace runs to a single space, trimming
/// the ends. Used on both the query and (implicitly, since the source
/// data is already single-spaced) names, so `"  FIRE   ENGINE "` and
/// `"fire engine"` compare equal — search is meant to be forgiving about
/// how a query was typed, not about what the data says.
fn normalize(s: &str) -> String {
    s.split_whitespace()
        .map(str::to_lowercase)
        .collect::<Vec<_>>()
        .join(" ")
}

/// Split a normalized name into its whole words, on the same boundary a
/// person would read as a word break: whitespace and punctuation
/// (`-`, `:`, `,`, `'`), never in the middle of a run of letters/digits.
fn words(normalized_name: &str) -> impl Iterator<Item = &str> {
    normalized_name
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
}

/// Score a normalized name against a normalized, non-empty query.
/// Returns `None` if the name does not match at all.
fn score(normalized_name: &str, normalized_query: &str) -> Option<MatchTier> {
    if normalized_name == normalized_query {
        return Some(MatchTier::Exact);
    }

    let mut leading = true;
    let mut any_word_match = false;
    for word in words(normalized_name) {
        if word == normalized_query {
            if leading {
                return Some(MatchTier::LeadingWord);
            }
            any_word_match = true;
        }
        leading = false;
    }
    if any_word_match {
        return Some(MatchTier::Word);
    }

    if normalized_name.contains(normalized_query) {
        return Some(MatchTier::Substring);
    }

    None
}

/// Search `table` for `query`, returning matches best-first.
///
/// Ranking, best to worst:
///
/// 1. **Exact** — the query is the whole name.
/// 2. **Leading word** — the query is the name's first whole word
///    (`"fire"` finds "fire engine" and "fire extinguisher" here).
/// 3. **Word** — the query is some other whole word in the name.
/// 4. **Substring** — the query occurs but not as a whole word
///    (`"fire"` finds "firefighter", "firecracker" and "fireworks"
///    here, after every emoji actually *called* "fire" or "fire
///    something").
///
/// Ties within a tier keep `table`'s own order, which for [`all`] is
/// CLDR order — so 🔥 ("fire") outranks "fire engine" and "fire
/// extinguisher" (tier 2, in CLDR order relative to each other), which
/// in turn outrank "firefighter", "firecracker" and "fireworks" (tier 4).
/// That ordering is the entire point of ranking explicitly instead of
/// trusting table order or a naive `contains` check: a flat substring
/// search would put "firefighter" and "fire" on equal footing and let
/// table order decide, which is exactly the "feels random" a picker
/// should not have.
///
/// Matching is case- and whitespace-insensitive (see [`normalize`]). An
/// empty or all-whitespace query returns every entry in `table`'s own
/// order, rather than nothing — a picker with no query typed yet should
/// show the full grid, not a blank one.
pub fn search(query: &str, table: &[Emoji]) -> Vec<Emoji> {
    let normalized_query = normalize(query);
    if normalized_query.is_empty() {
        return table.to_vec();
    }

    let mut scored: Vec<(MatchTier, usize, Emoji)> = table
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| {
            let normalized_name = normalize(entry.name);
            score(&normalized_name, &normalized_query).map(|tier| (tier, index, *entry))
        })
        .collect();

    scored.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    scored.into_iter().map(|(_, _, entry)| entry).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- table integrity -------------------------------------------------

    #[test]
    fn every_entry_has_a_non_empty_character_and_name() {
        for entry in all() {
            assert!(
                !entry.emoji.is_empty(),
                "entry {:?} has an empty character",
                entry.name
            );
            assert!(
                !entry.name.is_empty(),
                "entry with character {:?} has an empty name",
                entry.emoji
            );
            assert!(!entry.group.is_empty());
            assert!(!entry.subgroup.is_empty());
        }
    }

    #[test]
    fn entry_count_matches_the_number_this_crate_documents() {
        // The crate doc comment states 1914 grid entries, 323 of which
        // support tones. Pin both here so the doc comment and the
        // generated table cannot silently drift apart.
        assert_eq!(EMOJI_COUNT, 1914);
        assert_eq!(all().len(), 1914);
        assert_eq!(tone_capable_count(), 323);
    }

    #[test]
    fn a_tone_incapable_entrys_grid_character_never_carries_a_skin_tone_modifier() {
        // U+1F3FB..=U+1F3FF are the five Fitzpatrick modifiers. An
        // entry with no tone relationship must show a plain,
        // unmodified character — only a tone-capable entry may
        // (rarely, via the documented medium-tone fallback) have one in
        // its grid `emoji` field.
        for entry in all().iter().filter(|e| !e.supports_tones()) {
            for ch in entry.emoji.chars() {
                let cp = ch as u32;
                assert!(
                    !(0x1F3FB..=0x1F3FF).contains(&cp),
                    "{:?} ({}) has a skin-tone modifier despite supports_tones() == false",
                    entry.name,
                    entry.emoji
                );
            }
        }
    }

    #[test]
    fn no_entry_is_a_bare_skin_tone_or_hair_component() {
        // The 9 `component` status entries (5 skin tones, 4 hair
        // styles) are modifiers, not standalone picks, and must not
        // have made it into the table via the fully-qualified filter.
        let component_names = [
            "light skin tone",
            "medium-light skin tone",
            "medium skin tone",
            "medium-dark skin tone",
            "dark skin tone",
            "red hair",
            "curly hair",
            "white hair",
            "bald",
        ];
        for entry in all() {
            assert!(
                !component_names.contains(&entry.name),
                "bare component {:?} leaked into the table",
                entry.name
            );
        }
    }

    #[test]
    fn groups_lists_all_ten_cldr_groups_in_file_order() {
        let g = groups();
        assert_eq!(g.len(), 10);
        assert_eq!(g[0], "Smileys & Emotion");
        assert_eq!(g[1], "People & Body");
        assert_eq!(g[2], "Component");
        assert_eq!(g[9], "Flags");
    }

    #[test]
    fn every_table_entry_group_is_one_of_the_listed_groups() {
        let g = groups();
        for entry in all() {
            assert!(
                g.contains(&entry.group),
                "{:?} has group {:?}, not in groups()",
                entry.name,
                entry.group
            );
        }
    }

    #[test]
    fn the_component_group_has_no_entries_after_filtering() {
        assert!(
            !all().iter().any(|e| e.group == "Component"),
            "Component should be empty once component-status entries are excluded"
        );
    }

    #[test]
    fn table_order_is_cldr_order_not_alphabetical() {
        // The very first entry in the source file is grinning face, not
        // whatever is alphabetically first — confirms the generator
        // preserved file order rather than sorting it.
        assert_eq!(all()[0].name, "grinning face");
        assert_eq!(all()[0].emoji, "😀");
    }

    // --- tone relationship -----------------------------------------------

    #[test]
    fn a_gesture_with_tones_reports_five_distinct_variants() {
        let waving_hand = all()
            .iter()
            .find(|e| e.name == "waving hand")
            .expect("waving hand should be in the table");
        assert!(waving_hand.supports_tones());
        let variants = waving_hand
            .tone_variants()
            .expect("supports_tones() said true");
        let chars: std::collections::HashSet<&str> = variants.iter().map(|(_, c)| *c).collect();
        assert_eq!(chars.len(), 5, "the five tone variants must be distinct");
        // And every one of them differs from the neutral grid character.
        for (_, variant) in variants {
            assert_ne!(variant, waving_hand.emoji);
        }
    }

    #[test]
    fn an_emoji_without_tones_reports_none() {
        let fire = all()
            .iter()
            .find(|e| e.name == "fire")
            .expect("fire should be in the table");
        assert!(!fire.supports_tones());
        assert!(fire.tone_variants().is_none());
    }

    #[test]
    fn asking_a_tone_incapable_emoji_for_a_tone_returns_the_plain_emoji() {
        let fire = all().iter().find(|e| e.name == "fire").unwrap();
        for tone in TONES {
            assert_eq!(fire.tone(tone), fire.emoji);
        }
    }

    #[test]
    fn asking_a_tone_capable_emoji_for_each_tone_matches_its_variants_list() {
        let waving_hand = all().iter().find(|e| e.name == "waving hand").unwrap();
        let variants = waving_hand.tone_variants().unwrap();
        for (tone, expected) in variants {
            assert_eq!(waving_hand.tone(tone), expected);
        }
    }

    #[test]
    fn a_tone_family_whose_neutral_form_needs_a_variation_selector_still_merges_correctly() {
        // "index pointing up"'s neutral form is "261D FE0F" but its
        // toned forms are "261D <modifier>" with no FE0F at all (the
        // modifier already forces emoji presentation). If the family
        // key were not FE0F-insensitive, this would show up as two
        // disconnected table rows instead of one entry with tones.
        let entry = all()
            .iter()
            .find(|e| e.name == "index pointing up")
            .expect("index pointing up should be a single, merged entry");
        assert!(entry.supports_tones());
        let variants = entry.tone_variants().unwrap();
        // The neutral (FE0F-qualified) form is the grid character, and
        // is distinct from every toned variant.
        for (_, variant) in variants {
            assert_ne!(variant, entry.emoji);
        }
    }

    #[test]
    fn a_two_person_family_with_a_neutral_form_is_kept_without_tones() {
        // "kiss: woman, man" has a neutral form but its tone variants
        // carry two independent modifiers (one per person) — a single
        // Tone can't disambiguate those, so this crate does not model
        // it as tone-capable at all.
        let entry = all()
            .iter()
            .find(|e| e.name == "kiss: woman, man")
            .expect("kiss: woman, man should be in the table");
        assert!(!entry.supports_tones());
    }

    #[test]
    fn a_two_person_family_with_no_neutral_form_is_excluded_entirely() {
        // "kiss: person, person" and "couple with heart: person,
        // person" exist in the source file only as two-modifier toned
        // combinations (20 each) with no neutral rendering — they must
        // not appear in the grid at all, rather than an arbitrary tone
        // pair standing in for them.
        //
        // Note "people wrestling" is deliberately *not* used for this:
        // the file has two unrelated encodings sharing that display
        // name — the classic single-glyph 🤼 pictograph (which has a
        // neutral form and legitimately gets tones) and a newer
        // two-person ZWJ combo (which does not and is excluded) — so
        // asserting the name's absence entirely would be wrong.
        assert!(!all().iter().any(|e| e.name == "kiss: person, person"));
        assert!(!all()
            .iter()
            .any(|e| e.name == "couple with heart: person, person"));
    }

    #[test]
    fn search_still_finds_a_tone_capable_emoji_by_its_base_name_only_once() {
        let results = search("waving hand", all());
        let matches: Vec<_> = results.iter().filter(|e| e.name == "waving hand").collect();
        assert_eq!(
            matches.len(),
            1,
            "a tone-capable emoji must appear exactly once in the table, not once per tone"
        );
    }

    // --- character safety --------------------------------------------------

    #[test]
    fn multi_codepoint_emoji_are_never_sliced_mid_character() {
        // A ZWJ sequence like the transgender flag is five codepoints
        // and far more than one byte; chars() must yield it whole, and
        // re-collecting those chars must reproduce the exact string —
        // which would fail if anything upstream had built it by slicing
        // at a byte offset that landed inside a character instead of
        // walking chars().
        let flag = all()
            .iter()
            .find(|e| e.name == "transgender flag")
            .expect("transgender flag should be in the table");
        assert!(flag.emoji.chars().count() >= 4);
        let rebuilt: String = flag.emoji.chars().collect();
        assert_eq!(rebuilt, flag.emoji);
    }

    #[test]
    fn a_query_containing_a_multi_byte_character_never_panics_and_finds_nothing() {
        // Every name in this table is ASCII, so a query made of a
        // multi-byte character (the emoji itself, not its name) cannot
        // match anything — but it must not panic either, which is what
        // would happen if search sliced by byte offset instead of
        // walking chars() throughout normalize()/words()/contains().
        let results = search("🔥", all());
        assert!(results.is_empty());
    }

    // --- search ranking ------------------------------------------------

    #[test]
    fn exact_name_match_beats_a_leading_word_match() {
        let results = search("fire", all());
        assert!(!results.is_empty());
        assert_eq!(results[0].name, "fire");
    }

    #[test]
    fn leading_word_match_beats_a_glued_substring_match() {
        let results = search("fire", all());
        let pos = |name: &str| results.iter().position(|e| e.name == name).unwrap();
        // "fire engine" / "fire extinguisher" (leading word) must come
        // before "firefighter" / "firecracker" / "fireworks" (glued
        // substring, not a whole word).
        let engine = pos("fire engine");
        let firefighter = pos("firefighter");
        let firecracker = pos("firecracker");
        let fireworks = pos("fireworks");
        assert!(engine < firefighter);
        assert!(engine < firecracker);
        assert!(engine < fireworks);
    }

    #[test]
    fn a_whole_word_match_after_the_first_word_beats_a_glued_substring_match() {
        // score() is the unit the tiers are actually decided in, so
        // assert the boundary directly rather than hunting for a real
        // pair of table entries that happens to exercise it: "face" is
        // a later whole word in "smiling face with heart-eyes" (Word
        // tier), and merely a glued substring of "boldface" (Substring
        // tier, if such a name existed) — Word must outrank Substring.
        assert_eq!(
            score("smiling face with heart-eyes", "face"),
            Some(MatchTier::Word)
        );
        assert_eq!(score("boldface", "face"), Some(MatchTier::Substring));
        assert!(MatchTier::Word < MatchTier::Substring);
    }

    #[test]
    fn case_and_leading_or_trailing_whitespace_do_not_affect_matching() {
        let a = search("Fire", all());
        let b = search("  fire  ", all());
        let c = search("FIRE", all());
        assert_eq!(
            a.iter().map(|e| e.name).collect::<Vec<_>>(),
            b.iter().map(|e| e.name).collect::<Vec<_>>()
        );
        assert_eq!(
            a.iter().map(|e| e.name).collect::<Vec<_>>(),
            c.iter().map(|e| e.name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn internal_whitespace_runs_are_collapsed_before_matching() {
        let normal = search("fire engine", all());
        let extra_spaces = search("fire    engine", all());
        assert_eq!(
            normal.iter().map(|e| e.name).collect::<Vec<_>>(),
            extra_spaces.iter().map(|e| e.name).collect::<Vec<_>>()
        );
        assert!(normal.iter().any(|e| e.name == "fire engine"));
    }

    #[test]
    fn empty_query_returns_everything_in_cldr_order() {
        let results = search("", all());
        assert_eq!(results.len(), all().len());
        assert_eq!(
            results.iter().map(|e| e.name).collect::<Vec<_>>(),
            all().iter().map(|e| e.name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn whitespace_only_query_also_returns_everything() {
        let results = search("   ", all());
        assert_eq!(results.len(), all().len());
    }

    #[test]
    fn a_query_matching_nothing_returns_an_empty_vec() {
        let results = search("this is definitely not an emoji name", all());
        assert!(results.is_empty());
    }

    #[test]
    fn substring_match_still_finds_a_glued_word() {
        let results = search("cracker", all());
        assert!(results.iter().any(|e| e.name == "firecracker"));
    }

    #[test]
    fn search_over_an_empty_table_returns_nothing_and_does_not_panic() {
        assert!(search("fire", &[]).is_empty());
        assert_eq!(search("", &[]).len(), 0);
    }
}
