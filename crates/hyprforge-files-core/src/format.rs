//! Turning a byte count, or a modification time, into something a person
//! reads at a glance.
//!
//! Pure, and kept separate from [`crate::types::Entry`] itself so the
//! boundary math (where "999 B" becomes "1.0 KiB") is one function with
//! its own tests, rather than an inline `format!` call duplicated once
//! for the list view and once for the grid view.

use crate::types::{Entry, EntryKind, EntrySize, ItemCount};
use std::time::SystemTime;

/// What the status bar says about a listing.
///
/// A pure function over what is on screen, so the thing the bar claims
/// is testable without building a window — and so the claims themselves
/// can be argued about in one place.
///
/// The facts, in the order they earn their space:
///
/// - **What is here**, split into folders and files. "43 items" is a
///   worse answer than "12 folders, 31 files" for the question anyone
///   actually has, which is whether this is a place with things in it or
///   a place with places in it.
/// - **How big**, summing only the files. A directory contributes
///   nothing, because this crate never sums a tree — see [`EntrySize`].
///   Omitted entirely when nothing here has a byte size, rather than
///   claiming "0 B" for a directory of directories.
/// - **What is hidden**, when a filter is actually hiding something.
///   Silence here would make "this folder is empty" a lie in the one
///   case where the user can do something about it.
/// - **What is selected**, and how big that is.
///
/// Separated by `\u{00B7}` rather than commas: these are separate facts,
/// not a list, and the design uses the same separator.
pub fn status_summary(
    rows: &[&Entry],
    selected: &std::collections::HashSet<std::path::PathBuf>,
    hidden: usize,
    archive: Option<&str>,
) -> String {
    let mut parts: Vec<String> = Vec::new();

    // Mockup `1j` heads an archive's summary with what compressed it:
    // "zstd \u{00B7} 3 entries \u{00B7} 20.3 MB \u{2192} 7.0 MB". Nothing in a
    // listing of members says that otherwise.
    if let Some(format) = archive {
        parts.push(format.to_string());
    }

    let folders = rows.iter().filter(|e| e.is_dir).count();
    let files = rows.len() - folders;
    match (folders, files) {
        (0, 0) => parts.push("Empty folder".to_string()),
        (0, f) => parts.push(plural(f, "file", "files")),
        (d, 0) => parts.push(plural(d, "folder", "folders")),
        (d, f) => parts.push(format!("{}, {}", plural(d, "folder", "folders"), plural(f, "file", "files"))),
    }

    if let Some(total) = total_bytes(rows.iter().copied()) {
        // Inside an archive, what it comes to packed as well — but only
        // when every file here states one. A tar says nothing per
        // member (it is one compressed stream), and adding up the few
        // that did answer would print a total that is not the total of
        // anything.
        let packed: Option<u64> = archive.and_then(|_| {
            rows.iter()
                .filter(|e| !e.is_dir)
                .map(|e| e.packed)
                .sum::<Option<u64>>()
        });
        match packed {
            Some(packed) => parts.push(format!(
                "{} \u{2192} {}",
                human_readable_size(total),
                human_readable_size(packed)
            )),
            None => parts.push(human_readable_size(total)),
        }
    }

    if hidden > 0 {
        parts.push(format!("{hidden} hidden"));
    }

    if !selected.is_empty() {
        let chosen: Vec<&Entry> =
            rows.iter().copied().filter(|e| selected.contains(&e.path)).collect();
        let mut text = format!("{} selected", chosen.len());
        if let Some(total) = total_bytes(chosen.iter().copied()) {
            text.push_str(&format!(" ({})", human_readable_size(total)));
        }
        parts.push(text);
    }

    parts.join(" \u{00B7} ")
}

/// The summed byte size of whatever here *has* one, or `None` when
/// nothing does.
///
/// `None` rather than `Some(0)`, so a directory containing only
/// directories shows no size at all instead of claiming to hold zero
/// bytes — the same distinction [`ItemCount`] draws for one folder,
/// applied to a whole listing.
fn total_bytes<'a>(entries: impl Iterator<Item = &'a Entry>) -> Option<u64> {
    let mut any = false;
    let mut total = 0u64;
    for entry in entries {
        if let EntrySize::Bytes(n) = entry.size {
            any = true;
            total = total.saturating_add(n);
        }
    }
    any.then_some(total)
}

fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {many}")
    }
}

/// One entry's Permissions cell, in the `drwxr-xr-x` form `ls -l` uses.
///
/// That form and not an octal `755`, because this column is for reading
/// at a glance down a listing: the shape of `-rw-------` is recognisable
/// as "only I can read this" without being parsed, and octal is not.
///
/// The leading character is the file *type*, which is why this takes
/// `is_dir`/`is_symlink` separately — [`Entry::mode`] deliberately holds
/// only the permission bits, so the type is not stored twice.
///
/// setuid/setgid/sticky replace the matching `x` with `s`/`s`/`t`, and
/// with `S`/`S`/`T` when the underlying `x` is *not* set — the standard
/// spelling, and the distinction matters: a setuid bit on a file nobody
/// can execute is a different and more suspicious thing than one on a
/// file they can.
pub fn format_permissions(mode: u32, is_dir: bool, is_symlink: bool) -> String {
    let type_char = if is_symlink {
        'l'
    } else if is_dir {
        'd'
    } else {
        '-'
    };
    let mut out = String::with_capacity(10);
    out.push(type_char);
    // Owner, group, other — each read/write/execute, with the special
    // bit for that triad folded into its execute position.
    let triads = [
        (mode >> 6, mode & 0o4000 != 0, 's'),
        (mode >> 3, mode & 0o2000 != 0, 's'),
        (mode, mode & 0o1000 != 0, 't'),
    ];
    for (bits, special, special_char) in triads {
        out.push(if bits & 0b100 != 0 { 'r' } else { '-' });
        out.push(if bits & 0b010 != 0 { 'w' } else { '-' });
        let executable = bits & 0b001 != 0;
        out.push(match (special, executable) {
            (true, true) => special_char,
            (true, false) => special_char.to_ascii_uppercase(),
            (false, true) => 'x',
            (false, false) => '-',
        });
    }
    out
}

/// One entry's Owner cell: the login name, or the raw uid when this
/// system cannot resolve one.
///
/// The number rather than a dash, deliberately. An unresolvable uid is
/// not missing information — it is a file whose owner does not exist
/// here, which is worth seeing and is exactly what `ls -l` shows too.
pub fn format_owner(entry: &Entry) -> String {
    match &entry.owner {
        Some(name) => name.clone(),
        None => entry.uid.to_string(),
    }
}

/// The "Packed" cell: how much room a member takes up inside its
/// archive.
///
/// An em dash when the archive does not say — the ordinary case for a
/// tar, where compression is applied to the whole stream and no
/// individual member has a compressed size at all. The same rendering
/// `format_size` gives a folder nobody could count, and for the same
/// reason: a number nobody stated must not be shown as zero.
pub fn format_packed(entry: &Entry) -> String {
    match entry.packed {
        Some(bytes) => format_size(EntrySize::Bytes(bytes)),
        None => "\u{2014}".to_string(),
    }
}

/// A trashed item's Original Location cell: the folder it came from,
/// with the home directory written `~` the way the path bar writes it —
/// the part of a long path that tells two locations apart is the end,
/// and `/home/alex/` in front of every row pushes it out of the column.
pub fn format_origin(entry: &Entry, home: Option<&std::path::Path>) -> String {
    let Some(origin) = &entry.origin else {
        return String::new();
    };
    match home.and_then(|h| origin.strip_prefix(h).ok()) {
        Some(rest) if rest.as_os_str().is_empty() => "~".to_string(),
        Some(rest) => format!("~/{}", rest.display()),
        None => origin.display().to_string(),
    }
}

/// One entry's Kind cell.
///
/// Finer than [`EntryKind`], which is a handful of categories chosen to
/// pick an icon. A Kind *column* needs to earn its width, and "Document"
/// forty times down a directory of source files does not — this reads
/// the extension for a specific name ("Rust source", "PNG image") and
/// falls back to the category only when the extension says nothing.
///
/// The column was removed once before for exactly that reason: it read
/// "Folder" forty times and was noise dressed as information. It comes
/// back spelled out, and switchable — see `prefs::Columns`.
pub fn format_kind(entry: &Entry) -> String {
    if entry.is_dir {
        return "Folder".to_string();
    }
    if entry.link_broken {
        return "Broken link".to_string();
    }
    let ext = entry
        .name
        .rsplit_once('.')
        .filter(|(stem, _)| !stem.is_empty())
        .map(|(_, ext)| ext.to_ascii_lowercase());
    let named = ext.as_deref().and_then(|ext| match ext {
        "rs" => Some("Rust source"),
        "py" => Some("Python source"),
        "js" | "mjs" | "cjs" => Some("JavaScript"),
        "ts" => Some("TypeScript"),
        "jsx" | "tsx" => Some("React source"),
        "c" | "h" => Some("C source"),
        "cpp" | "hpp" | "cc" | "hh" => Some("C++ source"),
        "go" => Some("Go source"),
        "java" => Some("Java source"),
        "rb" => Some("Ruby source"),
        "sh" | "bash" | "zsh" => Some("Shell script"),
        "lua" => Some("Lua source"),
        "md" => Some("Markdown"),
        "txt" => Some("Text"),
        "toml" => Some("TOML"),
        "json" => Some("JSON"),
        "yaml" | "yml" => Some("YAML"),
        "html" | "htm" => Some("HTML"),
        "css" => Some("CSS"),
        "pdf" => Some("PDF"),
        "svg" => Some("SVG image"),
        "png" => Some("PNG image"),
        "jpg" | "jpeg" => Some("JPEG image"),
        "gif" => Some("GIF image"),
        "webp" => Some("WebP image"),
        "zip" => Some("ZIP archive"),
        "tar" => Some("TAR archive"),
        "gz" | "bz2" | "xz" | "zst" => Some("Compressed archive"),
        "7z" => Some("7-Zip archive"),
        "rar" => Some("RAR archive"),
        "mp3" => Some("MP3 audio"),
        "flac" => Some("FLAC audio"),
        "wav" => Some("WAV audio"),
        "ogg" | "opus" => Some("Ogg audio"),
        "mp4" => Some("MP4 video"),
        "mkv" => Some("Matroska video"),
        "webm" => Some("WebM video"),
        _ => None,
    });
    if let Some(named) = named {
        return named.to_string();
    }
    // No specific name: fall back to the coarse category, and to the
    // uppercased extension for something this list has never heard of —
    // "FOO file" still tells a reader more than "Other" does.
    match entry.kind {
        EntryKind::Folder => "Folder".to_string(),
        EntryKind::Image => "Image".to_string(),
        EntryKind::Document => "Document".to_string(),
        EntryKind::Archive => "Archive".to_string(),
        EntryKind::Code => "Source code".to_string(),
        EntryKind::Audio => "Audio".to_string(),
        EntryKind::Video => "Video".to_string(),
        EntryKind::Other => match ext {
            Some(ext) => format!("{} file", ext.to_uppercase()),
            None => "File".to_string(),
        },
    }
}

/// One entry's Size cell, whichever kind of size it has.
///
/// The four cases in one place, because they are one column. This used
/// to be an inline `match` at the row-building call site, decoding an
/// `Option<Option<usize>>` with a comment explaining why `.flatten()`
/// was wrong — the two levels being "not counted yet" (blank) and
/// "could not be read" (an em dash), which flattening collapses into
/// showing every folder as unreadable for the moment before its count
/// lands. [`ItemCount`] makes them separate variants instead, and this
/// is the only function that has to know what each one looks like.
pub fn format_size(size: EntrySize) -> String {
    match size {
        EntrySize::Bytes(n) => human_readable_size(n),
        // Blank, not "0 items" and not a dash: nothing is yet known,
        // and a number appearing a moment later is expected. A dash
        // here would say "we looked and could not tell you", which is
        // the next case and a different thing.
        EntrySize::Items(ItemCount::Pending) => String::new(),
        EntrySize::Items(ItemCount::Known(1)) => "1 item".to_string(),
        EntrySize::Items(ItemCount::Known(n)) => format!("{n} items"),
        // An em dash for a folder that could not be read. A folder you
        // have no permission to open must not read as "0 items".
        EntrySize::Items(ItemCount::Unreadable) => "\u{2014}".to_string(),
    }
}

/// The binary unit ladder, `1024` a step — matching what `stat`, `du`
/// and every mainstream file manager actually compute with, even where
/// they print a decimal-looking label. The brief's own boundary values
/// (999/1000/1023/1024) only make sense against `1024`, not `1000`: a
/// decimal ladder would put both 999 and 1000 on opposite sides of a
/// unit change, and it does not.
const UNITS: &[&str] = &["B", "KiB", "MiB", "GiB", "TiB", "PiB", "EiB"];

/// Formats `bytes` as a human-readable size.
///
/// Anything under 1024 bytes is shown as a bare integer — "999 B", "1000
/// B" — because a fractional byte count (`0.98 KiB`) is a worse answer
/// than the exact number when the exact number is this small. From 1024
/// up, one decimal place and the next unit, walking the ladder until the
/// value is back under 1024 or the units run out.
pub fn human_readable_size(bytes: u64) -> String {
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

/// Formats a modification time the way the design's list view shows it:
/// local time, a fixed short calendar form so a column of these lines up
/// rather than each row producing a differently-shaped string.
///
/// `None` — [`crate::types::Entry::modified`]'s own state for "the
/// filesystem wouldn't say" — renders as an em dash rather than an empty
/// cell, so a missing timestamp reads as "unknown" and not as a blank
/// the eye skips over.
///
/// "Now" is a parameter and not a call to `chrono::Local::now()` inside
/// here, for two reasons that turn out to be the same one. A caller
/// rendering a whole listing reads the clock once and every row is
/// relative to the same instant, rather than each row consulting
/// `/etc/localtime` on every redraw and the top of a long list
/// potentially disagreeing with the bottom about where "Today" ends. And
/// a test can hand it a fixed clock instead of whatever today happens to
/// be.
pub fn format_modified_at(
    modified: Option<SystemTime>,
    now: chrono::DateTime<chrono::Local>,
) -> String {
    let Some(t) = modified else {
        return "\u{2014}".to_string();
    };
    let t = chrono::DateTime::<chrono::Local>::from(t);

    // Relative near the present, absolute further back — what the design
    // shows, and what every file manager does, because the question a
    // date column answers changes with distance. For something touched
    // this afternoon you want the *time*; for last week, the day; for
    // last year, the year. One absolute format answers none of them
    // well: "Dec 28, 2025 12:35" is nine characters of noise when the
    // answer is "an hour ago".
    use chrono::Datelike;
    let days = now.date_naive().signed_duration_since(t.date_naive()).num_days();
    match days {
        0 => format!("Today {}", t.format("%H:%M")),
        1 => "Yesterday".to_string(),
        // Inside the last week the weekday is more use than the date —
        // "Mon 09:15" locates it in a way "Sep 8" does not.
        2..=6 => t.format("%a %H:%M").to_string(),
        // A date in the future: a clock skew or a file copied with a
        // preserved timestamp from a machine set wrong. Fall through to
        // the absolute form rather than saying "in 3 days", which reads
        // as a bug in us rather than in the timestamp.
        _ if days < 0 => t.format("%b %-d, %Y").to_string(),
        _ if t.year() == now.year() => t.format("%-d %b").to_string(),
        _ => t.format("%b %-d, %Y").to_string(),
    }
}

#[cfg(test)]
mod column_tests {
    use super::*;
    use std::path::PathBuf;

    fn entry(name: &str, is_dir: bool) -> Entry {
        Entry {
            name: name.to_string(),
            path: PathBuf::from("/dir").join(name),
            is_dir,
            size: EntrySize::Bytes(1),
            modified: None,
            is_symlink: false,
            link_broken: false,
            hidden: false,
            kind: EntryKind::classify(is_dir, name),
            mode: 0o644,
            uid: 1000,
            owner: Some("alex".to_string()),
            origin: None,
            packed: None,
        }
    }

    #[test]
    fn permissions_read_the_way_ls_prints_them() {
        assert_eq!(format_permissions(0o755, true, false), "drwxr-xr-x");
        assert_eq!(format_permissions(0o644, false, false), "-rw-r--r--");
        assert_eq!(format_permissions(0o600, false, false), "-rw-------");
        assert_eq!(format_permissions(0o777, false, true), "lrwxrwxrwx");
    }

    /// A setuid bit on a file nobody can execute is a different and more
    /// suspicious thing than one on a file they can, and the standard
    /// spelling keeps them apart with case.
    #[test]
    fn a_special_bit_without_its_execute_bit_is_upper_case() {
        assert_eq!(format_permissions(0o4755, false, false), "-rwsr-xr-x");
        assert_eq!(format_permissions(0o4644, false, false), "-rwSr--r--");
        assert_eq!(format_permissions(0o2755, false, false), "-rwxr-sr-x");
        assert_eq!(format_permissions(0o1777, true, false), "drwxrwxrwt");
        // 0o1666 has no execute bit anywhere, so the sticky bit shows
        // as an upper-case T in the other triad and the owner triad
        // really is `rw-`.
        assert_eq!(format_permissions(0o1666, true, false), "drw-rw-rwT");
    }

    /// The number, never a dash: a uid this system cannot name is a fact
    /// worth showing, not missing information.
    #[test]
    fn an_unresolvable_owner_shows_the_uid_rather_than_a_dash() {
        let mut e = entry("thing.txt", false);
        e.owner = None;
        e.uid = 100_000;
        assert_eq!(format_owner(&e), "100000");
    }

    #[test]
    fn a_resolvable_owner_shows_the_name() {
        assert_eq!(format_owner(&entry("thing.txt", false)), "alex");
    }

    #[test]
    fn an_original_location_under_home_is_written_with_a_tilde() {
        let mut e = entry("x.txt", false);
        let home = std::path::Path::new("/home/alex");
        e.origin = Some("/home/alex/projects/site".into());
        assert_eq!(format_origin(&e, Some(home)), "~/projects/site");
        e.origin = Some("/home/alex".into());
        assert_eq!(format_origin(&e, Some(home)), "~");
        e.origin = Some("/srv/data".into());
        assert_eq!(format_origin(&e, Some(home)), "/srv/data");
        e.origin = None;
        assert_eq!(format_origin(&e, Some(home)), "", "not a trashed item");
    }

    /// The Kind column was removed once for reading "Folder" forty times
    /// down a directory. It earns its width by being specific.
    #[test]
    fn kind_names_the_file_type_rather_than_its_broad_category() {
        assert_eq!(format_kind(&entry("main.rs", false)), "Rust source");
        assert_eq!(format_kind(&entry("notes.md", false)), "Markdown");
        assert_eq!(format_kind(&entry("photo.JPG", false)), "JPEG image");
        assert_eq!(format_kind(&entry("projects", true)), "Folder");
    }

    /// An extension this list has never heard of still says more than
    /// "Other" does.
    #[test]
    fn an_unknown_extension_is_named_after_itself() {
        assert_eq!(format_kind(&entry("model.safetensors", false)), "SAFETENSORS file");
        assert_eq!(format_kind(&entry("Makefile", false)), "File");
    }

    #[test]
    fn a_broken_link_says_so_rather_than_guessing_at_its_target() {
        let mut e = entry("dangling", false);
        e.is_symlink = true;
        e.link_broken = true;
        assert_eq!(format_kind(&e), "Broken link");
    }
}

#[cfg(test)]
mod status_tests {
    use super::*;
    use std::collections::HashSet;
    use std::path::PathBuf;

    fn file(name: &str, bytes: u64) -> Entry {
        Entry {
            name: name.to_string(),
            path: PathBuf::from("/dir").join(name),
            is_dir: false,
            size: EntrySize::Bytes(bytes),
            modified: None,
            is_symlink: false,
            link_broken: false,
            hidden: false,
            kind: EntryKind::Other,
            mode: 0o644,
            uid: 1000,
            owner: Some("alex".to_string()),
            origin: None,
            packed: None,
        }
    }

    fn folder(name: &str) -> Entry {
        Entry { is_dir: true, size: EntrySize::UNCOUNTED, kind: EntryKind::Folder, ..file(name, 0) }
    }

    fn summary(entries: &[Entry], selected: &[&str], hidden: usize) -> String {
        let rows: Vec<&Entry> = entries.iter().collect();
        let chosen: HashSet<PathBuf> =
            selected.iter().map(|n| PathBuf::from("/dir").join(n)).collect();
        status_summary(&rows, &chosen, hidden, None)
    }

    /// "43 items" is a worse answer than "12 folders, 31 files" for the
    /// question anyone actually has of a listing.
    #[test]
    fn the_bar_counts_folders_and_files_separately() {
        let entries = vec![folder("a"), folder("b"), file("x.txt", 100), file("y.txt", 200)];
        assert_eq!(summary(&entries, &[], 0), "2 folders, 2 files \u{00B7} 300 B");
    }

    #[test]
    fn one_of_each_is_singular() {
        let entries = vec![folder("a"), file("x.txt", 1)];
        assert_eq!(summary(&entries, &[], 0), "1 folder, 1 file \u{00B7} 1 B");
    }

    /// A directory of directories has no byte size to report, and must
    /// not claim "0 B" — the same distinction `ItemCount` draws for one
    /// folder, applied to a whole listing.
    #[test]
    fn a_listing_with_no_files_reports_no_size_rather_than_zero_bytes() {
        let entries = vec![folder("a"), folder("b")];
        assert_eq!(summary(&entries, &[], 0), "2 folders");
    }

    #[test]
    fn an_empty_listing_says_so_in_words() {
        assert_eq!(summary(&[], &[], 0), "Empty folder");
    }

    /// The case that matters most: a folder that looks empty and is not.
    /// Silence here makes "this folder is empty" a lie in exactly the
    /// situation the user could do something about.
    #[test]
    fn a_listing_emptied_by_a_filter_says_how_much_is_hidden() {
        assert_eq!(summary(&[], &[], 4), "Empty folder \u{00B7} 4 hidden");
    }

    #[test]
    fn nothing_hidden_says_nothing_about_hiding() {
        let entries = vec![file("x.txt", 5)];
        assert_eq!(summary(&entries, &[], 0), "1 file \u{00B7} 5 B");
    }

    #[test]
    fn a_selection_reports_its_own_count_and_size() {
        let entries = vec![file("x.txt", 1024), file("y.txt", 1024), file("z.txt", 99)];
        assert_eq!(
            summary(&entries, &["x.txt", "y.txt"], 0),
            "3 files \u{00B7} 2.1 KiB \u{00B7} 2 selected (2.0 KiB)"
        );
    }

    /// Selecting only folders gives a count but no size, for the same
    /// reason the listing total does.
    #[test]
    fn a_selection_of_folders_reports_no_size() {
        let entries = vec![folder("a"), folder("b")];
        assert_eq!(summary(&entries, &["a"], 0), "2 folders \u{00B7} 1 selected");
    }

    /// A directory of enormous files must not wrap around to a small
    /// number. Saturating rather than panicking or overflowing, because
    /// a status bar is never worth taking the window down for.
    #[test]
    fn an_impossible_total_saturates_rather_than_wrapping() {
        let entries = vec![file("a", u64::MAX), file("b", u64::MAX)];
        assert!(summary(&entries, &[], 0).contains("EiB"));
    }
}

#[cfg(test)]
mod size_cell_tests {
    use super::*;

    #[test]
    fn an_uncounted_folder_renders_blank_and_an_unreadable_one_renders_a_dash() {
        // The distinction the whole `ItemCount` type exists for: these
        // two must never render the same, or a folder being counted is
        // indistinguishable from one that failed.
        assert_eq!(format_size(EntrySize::UNCOUNTED), "");
        assert_eq!(format_size(EntrySize::Items(ItemCount::Unreadable)), "\u{2014}");
    }

    #[test]
    fn an_empty_folder_says_so_rather_than_rendering_like_an_unreadable_one() {
        assert_eq!(format_size(EntrySize::Items(ItemCount::Known(0))), "0 items");
    }

    #[test]
    fn one_item_is_singular() {
        assert_eq!(format_size(EntrySize::Items(ItemCount::Known(1))), "1 item");
        assert_eq!(format_size(EntrySize::Items(ItemCount::Known(2))), "2 items");
    }

    #[test]
    fn a_file_renders_bytes_not_a_count() {
        assert_eq!(format_size(EntrySize::Bytes(2048)), "2.0 KiB");
    }
}

#[cfg(test)]
mod archive_summary_tests {
    use super::*;
    use crate::types::{EntryKind, EntrySize};
    use std::collections::HashSet;

    fn member(name: &str, size: u64, packed: Option<u64>) -> Entry {
        Entry {
            name: name.to_string(),
            path: std::path::PathBuf::from("/a.zip").join(name),
            is_dir: false,
            size: EntrySize::Bytes(size),
            modified: None,
            is_symlink: false,
            link_broken: false,
            hidden: false,
            kind: EntryKind::Other,
            mode: 0o644,
            uid: 1000,
            owner: None,
            origin: None,
            packed,
        }
    }

    /// Mockup `1j`: "zstd · 3 entries · 20.3 MB → 7.0 MB". Nothing in a
    /// listing of members says what compressed them.
    #[test]
    fn an_archive_summary_names_the_format_and_both_totals() {
        let rows = [member("a", 1_000_000, Some(400_000)), member("b", 1_000_000, Some(600_000))];
        let rows: Vec<&Entry> = rows.iter().collect();

        let summary = status_summary(&rows, &HashSet::new(), 0, Some("zstd"));
        assert!(summary.starts_with("zstd \u{00B7} "), "{summary}");
        assert!(summary.contains('\u{2192}'), "the arrow between the two totals: {summary}");
    }

    /// A compressed tar is one stream, so no member in it has a packed
    /// size. Adding up the few that answered would print a total that
    /// is not the total of anything.
    #[test]
    fn a_tar_gets_no_packed_total_rather_than_a_partial_one() {
        let rows = [member("a", 1_000_000, None), member("b", 1_000_000, None)];
        let rows: Vec<&Entry> = rows.iter().collect();

        let summary = status_summary(&rows, &HashSet::new(), 0, Some("gzip"));
        assert!(summary.starts_with("gzip \u{00B7} "), "{summary}");
        assert!(!summary.contains('\u{2192}'), "no arrow without a second number: {summary}");
    }

    #[test]
    fn one_member_without_a_packed_size_suppresses_the_whole_total() {
        let rows = [member("a", 1_000_000, Some(400_000)), member("b", 1_000_000, None)];
        let rows: Vec<&Entry> = rows.iter().collect();

        let summary = status_summary(&rows, &HashSet::new(), 0, Some("zip"));
        assert!(
            !summary.contains('\u{2192}'),
            "a total missing one member is not the total: {summary}"
        );
    }

    #[test]
    fn an_ordinary_folder_says_nothing_about_packing() {
        let rows = [member("a", 10, None)];
        let rows: Vec<&Entry> = rows.iter().collect();

        let summary = status_summary(&rows, &HashSet::new(), 0, None);
        assert!(!summary.contains('\u{2192}'), "{summary}");
        assert!(!summary.starts_with("zip"), "{summary}");
    }

    /// A number nobody stated must not render as zero — the rule
    /// `ItemCount` already holds for a folder nobody could count.
    #[test]
    fn a_member_with_no_packed_size_shows_a_dash_rather_than_zero() {
        assert_eq!(format_packed(&member("a", 10, None)), "\u{2014}");
        assert_ne!(format_packed(&member("a", 10, Some(0))), "\u{2014}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_modified_time_is_an_em_dash_not_a_blank_cell() {
        assert_eq!(format_modified_at(None, chrono::Local::now()), "\u{2014}");
    }

    /// A fixed "now" so the relative forms below are about the code and
    /// not about what day the suite happens to be tested on.
    fn at(now: &str) -> chrono::DateTime<chrono::Local> {
        use chrono::TimeZone as _;
        let naive = chrono::NaiveDateTime::parse_from_str(now, "%Y-%m-%d %H:%M:%S").expect("a valid fixture time");
        chrono::Local.from_local_datetime(&naive).single().expect("an unambiguous local time")
    }

    /// Converts a fixture time to the `SystemTime` the real API takes.
    fn as_system_time(when: &str) -> SystemTime {
        let local = at(when);
        SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(local.timestamp() as u64)
    }

    #[test]
    fn something_touched_today_shows_the_time_because_that_is_the_question() {
        let now = at("2026-09-15 14:00:00");
        assert_eq!(format_modified_at(Some(as_system_time("2026-09-15 09:03:00")), now), "Today 09:03");
    }

    #[test]
    fn yesterday_is_named_rather_than_dated() {
        let now = at("2026-09-15 14:00:00");
        // Note this is 23 hours earlier, not 24: "yesterday" is a
        // calendar day apart, not a duration. Subtracting hours is the
        // obvious implementation and gets this wrong every evening.
        assert_eq!(format_modified_at(Some(as_system_time("2026-09-14 15:00:00")), now), "Yesterday");
    }

    #[test]
    fn inside_the_last_week_the_weekday_locates_it_better_than_a_date() {
        let now = at("2026-09-15 14:00:00");
        let formatted = format_modified_at(Some(as_system_time("2026-09-11 09:15:00")), now);
        assert_eq!(formatted, "Fri 09:15");
    }

    #[test]
    fn earlier_this_year_drops_the_year_that_is_already_implied() {
        let now = at("2026-09-15 14:00:00");
        assert_eq!(format_modified_at(Some(as_system_time("2026-08-12 10:00:00")), now), "12 Aug");
    }

    #[test]
    fn a_previous_year_is_spelled_out_in_full() {
        let now = at("2026-09-15 14:00:00");
        assert_eq!(format_modified_at(Some(as_system_time("2024-01-05 09:03:00")), now), "Jan 5, 2024");
    }

    /// A file whose timestamp is in the future — a clock skew, or a copy
    /// that preserved a timestamp from a machine set wrong. It must not
    /// read as "in 3 days", which looks like a bug in us rather than in
    /// the timestamp.
    #[test]
    fn a_timestamp_from_the_future_falls_back_to_an_absolute_date() {
        let now = at("2026-09-15 14:00:00");
        assert_eq!(format_modified_at(Some(as_system_time("2027-03-01 09:00:00")), now), "Mar 1, 2027");
    }

    #[test]
    fn nine_hundred_ninety_nine_bytes_has_no_unit_change() {
        assert_eq!(human_readable_size(999), "999 B");
    }

    #[test]
    fn one_thousand_bytes_is_still_plain_bytes() {
        // The decimal-looking boundary that a `>= 1000` off-by-one would
        // get wrong if this ladder were base-1000 instead of base-1024.
        assert_eq!(human_readable_size(1000), "1000 B");
    }

    #[test]
    fn ten_twenty_three_bytes_is_the_last_value_still_in_bytes() {
        assert_eq!(human_readable_size(1023), "1023 B");
    }

    #[test]
    fn ten_twenty_four_bytes_crosses_into_kibibytes() {
        assert_eq!(human_readable_size(1024), "1.0 KiB");
    }

    #[test]
    fn the_largest_unit_is_reached_without_running_off_the_ladder() {
        // u64::MAX is a little under 16 EiB; the ladder must stop at the
        // last unit rather than index past it.
        assert_eq!(human_readable_size(u64::MAX), "16.0 EiB");
    }

    #[test]
    fn a_mid_ladder_value_rounds_to_one_decimal_place() {
        assert_eq!(human_readable_size(1024 * 1024 * 3 / 2), "1.5 MiB");
    }

    #[test]
    fn zero_bytes_is_plain() {
        assert_eq!(human_readable_size(0), "0 B");
    }
}
