//! The Kaomoji and Symbols tabs' contents: short, hand-picked tables.
//!
//! Not in `hyprforge-emoji`, deliberately. That crate is a faithful
//! reading of Unicode's own `emoji-test.txt`, generated and checked
//! against it; a kaomoji is not an emoji and has no standard list to be
//! faithful to, and "symbols a person reaches for" — an arrow, an em
//! dash, a degree sign — is a picker's editorial choice rather than a
//! Unicode category. Both belong to this picker.
//!
//! Every entry has a name, because search reads names: typing "shrug" or
//! "arrow" in these tabs finds the entry the same way "fire" finds 🔥 in
//! the emoji tab.

/// One kaomoji or symbol: what is pasted, and what it is called.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Extra {
    pub text: &'static str,
    pub name: &'static str,
}

const fn e(text: &'static str, name: &'static str) -> Extra {
    Extra { text, name }
}

/// Kaomoji, grouped the way the design groups them.
pub const KAOMOJI: &[(&str, &[Extra])] = &[
    (
        "Happy",
        &[
            e("(◕‿◕)", "happy"),
            e("ヽ(•‿•)ノ", "cheering"),
            e("(＾▽＾)", "grinning"),
            e("(ᵔᴥᵔ)", "happy bear"),
            e("٩(◕‿◕)۶", "joy"),
            e("(✿◠‿◠)", "flower smile"),
        ],
    ),
    ("Shrug", &[e("¯\\_(ツ)_/¯", "shrug"), e("┐(´ー｀)┌", "whatever")]),
    (
        "Table flip",
        &[e("(╯°□°)╯︵ ┻━┻", "table flip"), e("┬─┬ノ( º _ ºノ)", "put the table back"), e("(ノಠ益ಠ)ノ彡┻━┻", "rage table flip")],
    ),
    (
        "Love",
        &[e("(♥ω♥)", "love"), e("(´∀｀)♡", "heart"), e("(づ｡◕‿‿◕｡)づ", "hug"), e("(っ˘з(˘⌣˘ )", "kiss")],
    ),
    ("Sad", &[e("(╥﹏╥)", "crying"), e("(ಥ﹏ಥ)", "tears"), e("(︶︹︺)", "sulking")]),
    ("Looks", &[e("ಠ_ಠ", "disapproval"), e("(¬_¬)", "side eye"), e("( ͡° ͜ʖ ͡°)", "lenny face"), e("(•_•)", "blank stare")]),
    ("Animals", &[e("ʕ•ᴥ•ʔ", "bear"), e("(=^･ω･^=)", "cat"), e("∪･ω･∪", "dog"), e("(・⊝・)", "bird")]),
];

/// Symbols, grouped by what they are for.
pub const SYMBOLS: &[(&str, &[Extra])] = &[
    (
        "Arrows",
        &[
            e("→", "right arrow"),
            e("←", "left arrow"),
            e("↑", "up arrow"),
            e("↓", "down arrow"),
            e("↔", "left right arrow"),
            e("↕", "up down arrow"),
            e("⇒", "double right arrow"),
            e("⇐", "double left arrow"),
            e("⇔", "double left right arrow"),
            e("↗", "north east arrow"),
            e("↘", "south east arrow"),
            e("↙", "south west arrow"),
            e("↖", "north west arrow"),
            e("↩", "return arrow"),
            e("↪", "forward return arrow"),
            e("⟶", "long right arrow"),
            e("⟵", "long left arrow"),
            e("↵", "enter arrow"),
        ],
    ),
    (
        "Maths",
        &[
            e("±", "plus minus"),
            e("×", "multiplication times"),
            e("÷", "division"),
            e("≠", "not equal"),
            e("≈", "almost equal approximately"),
            e("≤", "less than or equal"),
            e("≥", "greater than or equal"),
            e("∞", "infinity"),
            e("√", "square root"),
            e("∑", "sum sigma"),
            e("∏", "product"),
            e("∫", "integral"),
            e("∂", "partial derivative"),
            e("∆", "delta increment"),
            e("π", "pi"),
            e("µ", "micro mu"),
            e("°", "degree"),
            e("‰", "per mille"),
            e("²", "superscript two squared"),
            e("³", "superscript three cubed"),
            e("½", "one half"),
            e("¼", "one quarter"),
            e("¾", "three quarters"),
            e("∅", "empty set"),
            e("∈", "element of"),
            e("∀", "for all"),
            e("∃", "there exists"),
        ],
    ),
    (
        "Currency",
        &[
            e("€", "euro"),
            e("£", "pound sterling"),
            e("¥", "yen yuan"),
            e("¢", "cent"),
            e("₹", "rupee"),
            e("₩", "won"),
            e("₽", "rouble"),
            e("₺", "lira"),
            e("₪", "shekel"),
            e("₿", "bitcoin"),
        ],
    ),
    (
        "Punctuation",
        &[
            e("—", "em dash"),
            e("–", "en dash"),
            e("…", "ellipsis"),
            e("•", "bullet"),
            e("·", "middle dot interpunct"),
            e("«", "left guillemet"),
            e("»", "right guillemet"),
            e("“", "left double quote"),
            e("”", "right double quote"),
            e("‘", "left single quote"),
            e("’", "right single quote apostrophe"),
            e("„", "low double quote"),
            e("¡", "inverted exclamation"),
            e("¿", "inverted question"),
            e("§", "section"),
            e("¶", "pilcrow paragraph"),
            e("†", "dagger"),
            e("‡", "double dagger"),
        ],
    ),
    (
        "Marks",
        &[
            e("©", "copyright"),
            e("®", "registered"),
            e("™", "trade mark"),
            e("✓", "check mark tick"),
            e("✗", "ballot x cross"),
            e("★", "black star"),
            e("☆", "white star"),
            e("♥", "heart suit"),
            e("♦", "diamond suit"),
            e("♣", "club suit"),
            e("♠", "spade suit"),
            e("♪", "eighth note music"),
            e("♫", "beamed notes music"),
            e("☐", "ballot box"),
            e("☑", "ballot box with check"),
            e("⌘", "command key"),
            e("⌥", "option key"),
            e("⇧", "shift key"),
            e("⌫", "erase backspace key"),
            e("⏎", "return key"),
        ],
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    fn every_entry() -> impl Iterator<Item = &'static Extra> {
        KAOMOJI.iter().chain(SYMBOLS).flat_map(|(_, entries)| entries.iter())
    }

    #[test]
    fn every_entry_has_something_to_paste_and_a_name_to_find_it_by() {
        for extra in every_entry() {
            assert!(!extra.text.trim().is_empty(), "{:?} pastes nothing", extra.name);
            assert!(!extra.name.trim().is_empty(), "{:?} cannot be searched for", extra.text);
            assert_eq!(extra.name, extra.name.to_lowercase(), "names are lowercase, like CLDR's");
        }
    }

    /// A symbol is one character, so it fits a grid cell the size of an
    /// emoji's. Anything longer belongs with the kaomoji, which get
    /// wider cells.
    #[test]
    fn every_symbol_is_a_single_character() {
        for (_, entries) in SYMBOLS {
            for extra in entries.iter() {
                assert_eq!(extra.text.chars().count(), 1, "{:?} is not one character", extra.name);
            }
        }
    }

    #[test]
    fn nothing_is_listed_twice() {
        let mut seen = std::collections::HashSet::new();
        for extra in every_entry() {
            assert!(seen.insert(extra.text), "{:?} is listed twice", extra.text);
        }
    }
}
