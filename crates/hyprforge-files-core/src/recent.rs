//! Recent: what was opened lately, by Files and by everything else.
//!
//! The list is not Files' own. It is `$XDG_DATA_HOME/recently-used.xbel`,
//! the freedesktop bookmark file every GTK application, the desktop
//! portal and most file managers write to — so a file saved from a
//! browser or opened in an image editor turns up here, and a file opened
//! from Files turns up in their "Recent" too. A list of only what Files
//! opened would be a smaller, worse answer to "where was that thing I
//! had open this morning".
//!
//! # Reading
//!
//! [`list`] is pure: bytes in, entries out. Three states stay apart, the
//! rule CLAUDE.md draws from `hlconfig::storage`:
//!
//! - **No file** is first run — nobody has opened anything yet — and
//!   Recent is empty with nothing to report.
//! - **A file that is not XML this can read** is an error the view says
//!   aloud. It is never shown as an empty Recent, and nothing here ever
//!   writes over it.
//! - **One broken bookmark** in a good file — no `href`, a `href` that is
//!   not a local file, no date anyone can read — costs that bookmark and
//!   nothing else.
//!
//! # Writing
//!
//! [`record`] and [`forget_ours`] edit the file as it was written. Every
//! other application's bookmark is kept as the bytes that application
//! wrote: a bookmark Files touches has its start tag rewritten (its
//! `modified` and `visited` dates) and Files' own `<bookmark:application>`
//! line replaced or added, and nothing outside those spans moves. A
//! rewrite through a parse-and-serialise round trip would have reflowed
//! every other application's entries, dropped anything this parser does
//! not model (groups, icons, private flags), and been one bug away from
//! losing another program's history.
//!
//! Written with mode `0600`, as GTK writes it: the list of files someone
//! opened is nobody else's business, and [`hyprforge_paths::write_atomic`]
//! creates with the default mode.
//!
//! # What this does not do
//!
//! Two writers at once is possible — a GTK application can save the file
//! between our read and our rename, and its change is then lost. GTK's own
//! writers have exactly the same window with each other; the file has no
//! lock to take. The window is one read-edit-write, kept short by doing
//! nothing else in between.

use crate::backend::FsBackend;
use crate::types::Entry;
use chrono::{DateTime, Utc};
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;
use std::path::{Path, PathBuf};

/// The name Files writes in a bookmark's `<bookmark:application>` — and
/// the only one [`forget_ours`] removes.
pub const APPLICATION: &str = "hyprforge-files";

/// What opens a bookmark again, in GTK's own spelling: the command line,
/// single-quoted, with `%u` for the URI.
const EXEC: &str = "'hyprforge-files %u'";

/// The largest file Recent reads. This machine's is 75KB after months;
/// past 16MB something other than a list of recent files is there, and
/// reading it whole on every click of the sidebar row would be the cost.
pub const MAX_BYTES: u64 = 16 * 1024 * 1024;

/// How many Recent shows — the newest, after anything that is gone has
/// been left out.
pub const SHOWN: usize = 200;

/// How many bookmarks are looked up on disk at most, to find
/// [`SHOWN`] that are still there. Bounds the stats a folder of
/// thousands of deleted downloads could cost.
const LOOKED_AT: usize = 2_000;

/// `$XDG_DATA_HOME/recently-used.xbel`.
pub fn path() -> PathBuf {
    hyprforge_paths::data_home().join("recently-used.xbel")
}

/// One thing somebody opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Used {
    pub path: PathBuf,
    /// The latest date the bookmark carries — added, modified, visited,
    /// or any application's own.
    pub when: DateTime<Utc>,
    pub mime: Option<String>,
    /// Whether Files is one of the applications that opened it.
    pub by_files: bool,
}

/// Why the file could not be used. The sentence names the file, because
/// "Recent couldn't be read" with no path leaves nowhere to look.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RecentError {
    #[error("{path} couldn't be read: {why}")]
    Read { path: String, why: String },
    #[error("{path} is {size} bytes, more than the {MAX_BYTES} Recent reads, so it was left alone")]
    TooLarge { path: String, size: u64 },
    #[error("{path} isn't a recent-files list this can read ({why}), so it was left as it is")]
    Malformed { path: String, why: String },
    #[error("{path} couldn't be written: {why}")]
    Write { path: String, why: String },
}

/// Every bookmark in `text` this can use, in the file's order. `Err` for
/// a file that is not well-formed XML — see the module doc.
pub fn list(text: &str) -> Result<Vec<Used>, String> {
    let scanned = scan(text)?;
    Ok(scanned
        .bookmarks
        .into_iter()
        .filter_map(|mark| {
            let path = crate::clipboard::path_of_uri(mark.href.as_deref()?)?;
            let when = mark.times.iter().max().copied()?;
            Some(Used { path, when, mime: mark.mime, by_files: mark.apps.iter().any(|a| a.name == APPLICATION) })
        })
        .collect())
}

/// `used`, newest first, each path once — at the date it was last used.
pub fn newest(mut used: Vec<Used>) -> Vec<Used> {
    // Stable, so two bookmarks with the same date keep the file's order.
    used.sort_by_key(|u| std::cmp::Reverse(u.when));
    let mut seen = std::collections::HashSet::new();
    used.retain(|u| seen.insert(u.path.clone()));
    used
}

/// `text` with `path` recorded as opened by Files at `now` — see the
/// module doc for what is and is not touched. `None` (or an empty file)
/// makes a new document.
pub fn record(text: Option<&str>, path: &Path, mime: Option<&str>, now: DateTime<Utc>) -> Result<String, String> {
    let stamp = stamp(now);
    let mime = mime.unwrap_or("application/octet-stream");
    let Some(text) = text.filter(|t| !t.trim().is_empty()) else {
        return Ok(new_document(&bookmark(path, mime, &stamp)));
    };
    let scanned = scan(text)?;
    let close = scanned.close.ok_or_else(|| "it has no closing </xbel>".to_string())?;
    // By the path the href names, not the href's spelling: GTK escapes
    // fewer characters than this does, and `a%2Cb` and `a,b` are one
    // file.
    let existing = scanned
        .bookmarks
        .iter()
        .find(|m| m.href.as_deref().and_then(crate::clipboard::path_of_uri).as_deref() == Some(path));
    let mut edits: Vec<(usize, usize, String)> = Vec::new();
    match existing {
        Some(mark) if mark.apps_close.is_some() && !mark.empty => {
            edits.push((mark.start, mark.tag_end, start_tag(&mark.attrs, &stamp)));
            match mark.apps.iter().find(|a| a.name == APPLICATION) {
                Some(ours) => edits.push((ours.start, ours.end, application(&stamp, ours.count.saturating_add(1)))),
                None => {
                    let at = mark.apps_close.expect("matched above");
                    edits.push(insert_line(text, at, &application(&stamp, 1), "          "));
                }
            }
        }
        // A bookmark with nowhere to put an application — written by
        // something that left its metadata out. Only this one bookmark is
        // rebuilt, and it is the file Files is recording: there is no
        // other application's line in it to lose.
        Some(mark) => edits.push((mark.start, mark.end, bookmark(path, mime, &stamp).trim_start().trim_end().to_string())),
        None => edits.push(insert_line(text, close, bookmark(path, mime, &stamp).trim_end(), "  ")),
    }
    Ok(splice(text, edits))
}

/// `text` without anything Files recorded: Files' own application line
/// goes from every bookmark, and a bookmark nobody else opened goes
/// altogether. Every other bookmark is untouched. Also answers how many
/// files were forgotten.
pub fn forget_ours(text: &str) -> Result<(String, usize), String> {
    let scanned = scan(text)?;
    let mut edits = Vec::new();
    let mut forgotten = 0;
    for mark in &scanned.bookmarks {
        let Some(ours) = mark.apps.iter().find(|a| a.name == APPLICATION) else { continue };
        forgotten += 1;
        let (start, end) = if mark.apps.len() == 1 { (mark.start, mark.end) } else { (ours.start, ours.end) };
        let (start, end) = whole_lines(text, start, end);
        edits.push((start, end, String::new()));
    }
    Ok((splice(text, edits), forgotten))
}

// --- the file --------------------------------------------------------------

/// The file's text, `None` when there is none (first run). Bounded by
/// [`MAX_BYTES`]. Blocking.
pub fn read_file(file: &Path) -> Result<Option<String>, RecentError> {
    use std::io::Read as _;
    let shown = file.display().to_string();
    let opened = match std::fs::File::open(file) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(RecentError::Read { path: shown, why: e.to_string() }),
    };
    let mut bytes = Vec::new();
    // `take` rather than trusting the length `stat` gave: the file can
    // grow between the two, and the bound is about what is read.
    opened
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| RecentError::Read { path: shown.clone(), why: e.to_string() })?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(RecentError::TooLarge { path: shown, size: bytes.len() as u64 });
    }
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|_| RecentError::Malformed { path: shown, why: "it isn't UTF-8".to_string() })
}

/// What Recent shows: newest first, at most [`SHOWN`], every one still
/// on disk — looked up through `backend`, which also fills in each
/// entry's size and date. A bookmark for a file that is gone is left
/// out: Recent never lists something a click on would fail to open.
/// Each entry's `origin` is its folder, for the Folder column and Show
/// in Folder. Blocking.
pub fn read_entries<B: FsBackend + ?Sized>(backend: &B, file: &Path) -> Result<Vec<Entry>, RecentError> {
    let Some(text) = read_file(file)? else { return Ok(Vec::new()) };
    let used = list(&text).map_err(|why| RecentError::Malformed { path: file.display().to_string(), why })?;
    let mut shown = Vec::new();
    for item in newest(used).into_iter().take(LOOKED_AT) {
        if let Ok(mut entry) = backend.stat(&item.path) {
            entry.origin = item.path.parent().map(Path::to_path_buf);
            shown.push(entry);
            if shown.len() == SHOWN {
                break;
            }
        }
    }
    Ok(shown)
}

/// Records `path` in `file` as opened by Files now. A file that will not
/// parse is reported and left exactly as it was. Blocking.
pub fn record_in(file: &Path, path: &Path, mime: Option<&str>) -> Result<(), RecentError> {
    let text = read_file(file)?;
    let edited = record(text.as_deref(), path, mime, Utc::now())
        .map_err(|why| RecentError::Malformed { path: file.display().to_string(), why })?;
    write_private(file, &edited)
}

/// Forgets everything Files recorded in `file` — Preferences' "Clear
/// Recent". Answers how many files were forgotten; nothing is written
/// when that is none. Blocking.
pub fn forget_ours_in(file: &Path) -> Result<usize, RecentError> {
    let Some(text) = read_file(file)? else { return Ok(0) };
    let (edited, forgotten) =
        forget_ours(&text).map_err(|why| RecentError::Malformed { path: file.display().to_string(), why })?;
    if forgotten > 0 {
        write_private(file, &edited)?;
    }
    Ok(forgotten)
}

/// Writes `text` over `file` atomically, readable by its owner only.
///
/// [`hyprforge_paths::write_atomic`]'s arrangement — a unique temporary
/// beside it, synced, renamed over — with the temporary created `0600`,
/// because the rename makes its mode the file's.
fn write_private(file: &Path, text: &str) -> Result<(), RecentError> {
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt as _;
    let shown = file.display().to_string();
    let fail = |e: std::io::Error| RecentError::Write { path: shown.clone(), why: e.to_string() };
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).map_err(fail)?;
    }
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let unique = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp = file.with_file_name(format!(".recently-used.xbel.{}.{unique}.tmp", std::process::id()));
    let written = (|| {
        let mut out = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&tmp)?;
        out.write_all(text.as_bytes())?;
        out.sync_all()?;
        std::fs::rename(&tmp, file)
    })();
    if let Err(e) = written {
        let _ = std::fs::remove_file(&tmp);
        return Err(fail(e));
    }
    Ok(())
}

// --- scanning ----------------------------------------------------------------

/// What a pass over the file found, with where each piece is.
struct Scanned {
    bookmarks: Vec<Mark>,
    /// Where `</xbel>` starts.
    close: Option<usize>,
}

/// One `<bookmark>`, with byte offsets into the text it came from.
struct Mark {
    start: usize,
    end: usize,
    /// Where its start tag ends — `start..tag_end` is `<bookmark …>`.
    tag_end: usize,
    /// `<bookmark …/>`, with nothing inside.
    empty: bool,
    /// Its attributes, names and values as written (still escaped).
    attrs: Vec<(String, String)>,
    /// `href`, unescaped. `None` when it has none or it would not decode.
    href: Option<String>,
    times: Vec<DateTime<Utc>>,
    mime: Option<String>,
    apps: Vec<App>,
    /// Where `</bookmark:applications>` starts.
    apps_close: Option<usize>,
}

struct App {
    start: usize,
    end: usize,
    name: String,
    count: u64,
}

/// Names compared as written, prefix included: GTK, GLib and every
/// writer seen in the wild bind `bookmark:` and `mime:` to the spec's
/// namespaces, and a file that bound them to other prefixes would have
/// its bookmarks listed without their applications — which only matters
/// to [`forget_ours`], whose worst case is then forgetting nothing.
fn scan(text: &str) -> Result<Scanned, String> {
    let mut reader = Reader::from_str(text);
    let mut bookmarks = Vec::new();
    let mut current: Option<Mark> = None;
    let mut close = None;
    let mut saw_root = false;
    loop {
        let before = reader.buffer_position() as usize;
        let event = reader
            .read_event()
            .map_err(|e| format!("at byte {}: {e}", reader.error_position()))?;
        let after = reader.buffer_position() as usize;
        match event {
            Event::Start(tag) | Event::Empty(tag) if tag.name().as_ref() == b"xbel" => saw_root = true,
            Event::Start(tag) if tag.name().as_ref() == b"bookmark" => {
                current = Some(open_mark(&tag, before, after, false));
            }
            Event::Empty(tag) if tag.name().as_ref() == b"bookmark" => {
                let mut mark = open_mark(&tag, before, after, true);
                mark.end = after;
                bookmarks.push(mark);
            }
            Event::End(tag) if tag.name().as_ref() == b"bookmark" => {
                if let Some(mut mark) = current.take() {
                    mark.end = after;
                    bookmarks.push(mark);
                }
            }
            Event::Start(tag) | Event::Empty(tag) if tag.name().as_ref() == b"bookmark:application" => {
                if let Some(mark) = current.as_mut() {
                    let name = attr(&tag, b"name").unwrap_or_default();
                    let count = attr(&tag, b"count").and_then(|c| c.trim().parse().ok()).unwrap_or(0);
                    if let Some(when) = attr(&tag, b"modified").as_deref().and_then(date) {
                        mark.times.push(when);
                    }
                    mark.apps.push(App { start: before, end: after, name, count });
                }
            }
            // The long form, `<bookmark:application …></bookmark:application>`:
            // the element ends here rather than with its tag.
            Event::End(tag) if tag.name().as_ref() == b"bookmark:application" => {
                if let Some(app) = current.as_mut().and_then(|m| m.apps.last_mut()) {
                    app.end = after;
                }
            }
            Event::End(tag) if tag.name().as_ref() == b"bookmark:applications" => {
                if let Some(mark) = current.as_mut() {
                    mark.apps_close = Some(before);
                }
            }
            Event::Start(tag) | Event::Empty(tag) if tag.name().as_ref() == b"mime:mime-type" => {
                if let Some(mark) = current.as_mut() {
                    mark.mime = attr(&tag, b"type");
                }
            }
            Event::End(tag) if tag.name().as_ref() == b"xbel" => close = Some(before),
            Event::Eof => break,
            _ => {}
        }
    }
    if !saw_root {
        return Err("it has no <xbel> element".to_string());
    }
    Ok(Scanned { bookmarks, close })
}

fn open_mark(tag: &BytesStart<'_>, start: usize, tag_end: usize, empty: bool) -> Mark {
    let mut attrs = Vec::new();
    let mut times = Vec::new();
    let mut href = None;
    for a in tag.attributes().flatten() {
        let name = String::from_utf8_lossy(a.key.as_ref()).into_owned();
        let raw = String::from_utf8_lossy(&a.value).into_owned();
        let value = a.normalized_value(quick_xml::XmlVersion::Implicit1_0).ok().map(|v| v.into_owned());
        match name.as_str() {
            "href" => href = value.clone(),
            "added" | "modified" | "visited" => times.extend(value.as_deref().and_then(date)),
            _ => {}
        }
        attrs.push((name, raw));
    }
    Mark { start, end: tag_end, tag_end, empty, attrs, href, times, mime: None, apps: Vec::new(), apps_close: None }
}

fn attr(tag: &BytesStart<'_>, name: &[u8]) -> Option<String> {
    tag.attributes()
        .flatten()
        .find(|a| a.key.as_ref() == name)
        .and_then(|a| a.normalized_value(quick_xml::XmlVersion::Implicit1_0).ok().map(|v| v.into_owned()))
}

/// An xbel date — RFC 3339, `2026-07-15T00:30:47.396150Z`. `None` for
/// anything else, which costs only that date.
fn date(text: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(text.trim()).ok().map(|d| d.with_timezone(&Utc))
}

/// A date as GTK writes one: UTC, microseconds, `Z`.
fn stamp(when: DateTime<Utc>) -> String {
    when.format("%Y-%m-%dT%H:%M:%S%.6fZ").to_string()
}

// --- writing -------------------------------------------------------------------

fn escape(text: &str) -> String {
    quick_xml::escape::escape(text).into_owned()
}

/// A bookmark's start tag with `modified` and `visited` moved to `stamp`
/// and every other attribute as written.
fn start_tag(attrs: &[(String, String)], stamp: &str) -> String {
    let mut out = String::from("<bookmark");
    let mut dated = (false, false);
    for (name, raw) in attrs {
        let value = match name.as_str() {
            "modified" => {
                dated.0 = true;
                stamp.to_string()
            }
            "visited" => {
                dated.1 = true;
                stamp.to_string()
            }
            // Written back as read, which is already escaped — except a
            // `"` that a single-quoted original could hold.
            _ => raw.replace('"', "&quot;"),
        };
        out.push_str(&format!(" {name}=\"{value}\""));
    }
    if !dated.0 {
        out.push_str(&format!(" modified=\"{stamp}\""));
    }
    if !dated.1 {
        out.push_str(&format!(" visited=\"{stamp}\""));
    }
    out.push('>');
    out
}

fn application(stamp: &str, count: u64) -> String {
    format!(
        "<bookmark:application name=\"{APPLICATION}\" exec=\"{}\" modified=\"{stamp}\" count=\"{count}\"/>",
        escape(EXEC)
    )
}

/// A whole bookmark, indented the way GTK indents one, ending in a
/// newline.
fn bookmark(path: &Path, mime: &str, stamp: &str) -> String {
    let href = escape(&crate::clipboard::file_uri(path));
    format!(
        "  <bookmark href=\"{href}\" added=\"{stamp}\" modified=\"{stamp}\" visited=\"{stamp}\">\n\
         \x20   <info>\n\
         \x20     <metadata owner=\"http://freedesktop.org\">\n\
         \x20       <mime:mime-type type=\"{}\"/>\n\
         \x20       <bookmark:applications>\n\
         \x20         {}\n\
         \x20       </bookmark:applications>\n\
         \x20     </metadata>\n\
         \x20   </info>\n\
         \x20 </bookmark>\n",
        escape(mime),
        application(stamp, 1)
    )
}

fn new_document(bookmark: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <xbel version=\"1.0\"\n\
         \x20     xmlns:bookmark=\"http://www.freedesktop.org/standards/desktop-bookmarks\"\n\
         \x20     xmlns:mime=\"http://www.freedesktop.org/standards/shared-mime-info\"\n\
         >\n\
         {bookmark}</xbel>\n"
    )
}

/// An insertion of `line` just before the tag at `at`: on a line of its
/// own, indented by `indent`, when the tag sits alone on its line (as it
/// does in every file GTK writes); in front of it otherwise.
fn insert_line(text: &str, at: usize, line: &str, indent: &str) -> (usize, usize, String) {
    let line_start = text[..at].rfind('\n').map_or(0, |i| i + 1);
    if text[line_start..at].trim().is_empty() {
        (line_start, line_start, format!("{indent}{}\n", line.trim_start()))
    } else {
        (at, at, line.trim_start().to_string())
    }
}

/// `start..end` widened to whole lines when the span is alone on its
/// lines, so removing it leaves no blank line behind.
fn whole_lines(text: &str, start: usize, end: usize) -> (usize, usize) {
    let line_start = text[..start].rfind('\n').map_or(0, |i| i + 1);
    let rest = &text[end..];
    let line_end = rest.find('\n').map(|i| end + i + 1);
    match line_end {
        Some(line_end) if text[line_start..start].trim().is_empty() && text[end..line_end].trim().is_empty() => {
            (line_start, line_end)
        }
        _ => (start, end),
    }
}

/// `text` with each `(start, end, replacement)` applied. Spans never
/// overlap; applied from the end so the earlier offsets stay true.
fn splice(text: &str, mut edits: Vec<(usize, usize, String)>) -> String {
    edits.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
    let mut out = text.to_string();
    for (start, end, with) in edits {
        out.replace_range(start..end, &with);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Shaped on a real `recently-used.xbel` as GTK writes it — the
    /// declaration, the two namespaces, the indentation — with made-up
    /// paths and applications. One bookmark per way of being broken.
    const OTHERS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<xbel version="1.0"
      xmlns:bookmark="http://www.freedesktop.org/standards/desktop-bookmarks"
      xmlns:mime="http://www.freedesktop.org/standards/shared-mime-info"
>
  <bookmark href="file:///home/someone/Pictures/a%20cat.png" added="2026-06-29T00:00:24.228238Z" modified="2026-09-03T21:28:43.980867Z" visited="2026-06-29T00:00:24.228239Z">
    <info>
      <metadata owner="http://freedesktop.org">
        <mime:mime-type type="image/png"/>
        <bookmark:applications>
          <bookmark:application name="an-image-editor" exec="&apos;an-image-editor %u&apos;" modified="2026-09-03T21:28:43.980866Z" count="4"/>
        </bookmark:applications>
      </metadata>
    </info>
  </bookmark>
  <bookmark href="file:///home/someone/Models/part.3mf" added="2026-07-15T00:30:47.396150Z" modified="2026-09-09T22:47:44.770450Z" visited="2026-07-15T00:30:47.396151Z">
    <info>
      <metadata owner="http://freedesktop.org">
        <mime:mime-type type="model/3mf"/>
        <bookmark:applications>
          <bookmark:application name="a-slicer" exec="&apos;a-slicer %u&apos;" modified="2026-09-09T22:47:44.770449Z" count="36"/>
        </bookmark:applications>
      </metadata>
    </info>
  </bookmark>
  <bookmark href="https://example.org/not-a-file" added="2026-09-10T00:00:00Z" modified="2026-09-10T00:00:00Z" visited="2026-09-10T00:00:00Z">
  </bookmark>
  <bookmark added="2026-09-11T00:00:00Z" modified="2026-09-11T00:00:00Z" visited="2026-09-11T00:00:00Z">
  </bookmark>
  <bookmark href="file:///home/someone/undated.txt" added="last tuesday" modified="soon">
  </bookmark>
</xbel>
"#;

    fn at(text: &str) -> DateTime<Utc> {
        date(text).unwrap()
    }

    #[test]
    fn an_xbel_with_other_apps_entries_parses_and_broken_entries_are_skipped() {
        let used = list(OTHERS).unwrap();
        let paths: Vec<&Path> = used.iter().map(|u| u.path.as_path()).collect();
        assert_eq!(paths, [Path::new("/home/someone/Pictures/a cat.png"), Path::new("/home/someone/Models/part.3mf")]);
        assert_eq!(used[1].when, at("2026-09-09T22:47:44.770450Z"), "the latest date the bookmark carries");
        assert_eq!(used[0].mime.as_deref(), Some("image/png"));
        assert!(!used[0].by_files);
    }

    #[test]
    fn newest_comes_first_and_each_path_once() {
        let one = |p: &str, t: &str| Used { path: p.into(), when: at(t), mime: None, by_files: false };
        let sorted = newest(vec![
            one("/a", "2026-01-01T00:00:00Z"),
            one("/b", "2026-03-01T00:00:00Z"),
            one("/a", "2026-02-01T00:00:00Z"),
        ]);
        let paths: Vec<&str> = sorted.iter().map(|u| u.path.to_str().unwrap()).collect();
        assert_eq!(paths, ["/b", "/a"]);
        assert_eq!(sorted[1].when, at("2026-02-01T00:00:00Z"));
    }

    /// The other rule from the 37 binds: a file that is there and is not
    /// XML is an error, never an empty list, and is never written over.
    #[test]
    fn a_file_that_is_not_xml_is_an_error_and_is_never_recorded_over() {
        assert!(list("<xbel><bookmark href=\"file:///a\"></xbel>").is_err());
        assert!(list("not xml at all").is_err(), "no <xbel> at all is not an empty list either");
        assert!(record(Some("<xbel><bookmark></xbel>"), Path::new("/a"), None, Utc::now()).is_err());
    }

    #[test]
    fn recording_keeps_every_other_application_s_bookmark_byte_for_byte() {
        let now = at("2026-10-04T12:00:00Z");
        let out = record(Some(OTHERS), Path::new("/home/me/new file.txt"), Some("text/plain"), now).unwrap();
        // Everything before `</xbel>` is still there, as it was.
        let before_close = &OTHERS[..OTHERS.find("</xbel>").unwrap()];
        assert!(out.starts_with(before_close), "{out}");
        let used = list(&out).unwrap();
        let ours = used.iter().find(|u| u.path == Path::new("/home/me/new file.txt")).unwrap();
        assert!(ours.by_files);
        assert_eq!(ours.when, now);
        assert_eq!(ours.mime.as_deref(), Some("text/plain"));
        assert_eq!(used.len(), 3, "the two that were readable, and ours");
    }

    /// A file another application already has: its start tag's dates and
    /// a line for Files change, and the other application's line stays.
    #[test]
    fn recording_a_file_another_app_opened_adds_our_line_beside_theirs() {
        let now = at("2026-10-04T12:00:00Z");
        let out = record(Some(OTHERS), Path::new("/home/someone/Models/part.3mf"), None, now).unwrap();
        assert!(out.contains(r#"<bookmark:application name="a-slicer" exec="&apos;a-slicer %u&apos;" modified="2026-09-09T22:47:44.770449Z" count="36"/>"#));
        assert!(out.contains(r#"added="2026-07-15T00:30:47.396150Z" modified="2026-10-04T12:00:00.000000Z""#), "{out}");
        let used = list(&out).unwrap();
        assert!(used.iter().find(|u| u.path.ends_with("part.3mf")).unwrap().by_files);
        assert_eq!(out.matches("<bookmark ").count(), OTHERS.matches("<bookmark ").count(), "no second bookmark");
        // And the first bookmark, which was not touched, is byte for byte.
        let first_end = OTHERS.find("</bookmark>").unwrap();
        assert_eq!(&out[..first_end], &OTHERS[..first_end]);
    }

    #[test]
    fn recording_again_counts_up_rather_than_adding_a_line() {
        let path = Path::new("/home/me/a.txt");
        let once = record(None, path, Some("text/plain"), at("2026-10-01T00:00:00Z")).unwrap();
        let twice = record(Some(&once), path, Some("text/plain"), at("2026-10-02T00:00:00Z")).unwrap();
        assert_eq!(twice.matches("name=\"hyprforge-files\"").count(), 1);
        assert!(twice.contains("count=\"2\""), "{twice}");
        assert_eq!(list(&twice).unwrap()[0].when, at("2026-10-02T00:00:00Z"));
    }

    #[test]
    fn a_path_needing_escapes_survives_the_round_trip() {
        let path = Path::new("/home/me/R&D <draft> \u{e9}t\u{e9}.txt");
        let out = record(None, path, None, Utc::now()).unwrap();
        assert_eq!(list(&out).unwrap()[0].path, path);
    }

    /// Clear Recent is Files' own history, not the desktop's: a file only
    /// Files opened goes, and one another application also opened stays,
    /// with that application's line exactly as it was.
    #[test]
    fn forgetting_removes_only_what_files_recorded() {
        let now = at("2026-10-04T12:00:00Z");
        let shared = record(Some(OTHERS), Path::new("/home/someone/Models/part.3mf"), None, now).unwrap();
        let mine = record(Some(&shared), Path::new("/home/me/only-mine.txt"), None, now).unwrap();
        let (cleared, forgotten) = forget_ours(&mine).unwrap();
        assert_eq!(forgotten, 2);
        assert!(!cleared.contains(APPLICATION), "{cleared}");
        let used = list(&cleared).unwrap();
        assert!(used.iter().all(|u| !u.path.ends_with("only-mine.txt")));
        assert!(used.iter().any(|u| u.path.ends_with("part.3mf")), "the slicer still has it");
        assert!(cleared.contains(r#"name="a-slicer""#));
        // The untouched bookmark is still byte for byte.
        let first_end = OTHERS.find("</bookmark>").unwrap();
        assert_eq!(&cleared[..first_end], &OTHERS[..first_end]);
    }

    #[test]
    fn forgetting_with_nothing_of_ours_changes_nothing() {
        let (same, forgotten) = forget_ours(OTHERS).unwrap();
        assert_eq!((same.as_str(), forgotten), (OTHERS, 0));
    }

    #[test]
    fn a_missing_file_is_first_run_and_recording_creates_it_private() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("share").join("recently-used.xbel");
        assert_eq!(read_file(&file).unwrap(), None);
        record_in(&file, Path::new("/home/me/a.txt"), Some("text/plain")).unwrap();
        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(std::fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(list(&read_file(&file).unwrap().unwrap()).unwrap().len(), 1);
    }

    /// The instrument for "never write over a file that will not parse":
    /// the bytes on disk afterwards are the bytes that were there.
    #[test]
    fn an_unreadable_xbel_is_reported_and_left_exactly_as_it_was() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("recently-used.xbel");
        std::fs::write(&file, "<xbel><bookmark href=").unwrap();
        let backend = crate::backend::mock::MockBackend::new();
        assert!(matches!(read_entries(&backend, &file), Err(RecentError::Malformed { .. })));
        assert!(record_in(&file, Path::new("/a"), None).is_err());
        assert!(forget_ours_in(&file).is_err());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "<xbel><bookmark href=");
    }

    #[test]
    fn a_file_past_the_bound_is_refused_unread() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("recently-used.xbel");
        let big = std::fs::File::create(&file).unwrap();
        big.set_len(MAX_BYTES + 1).unwrap();
        assert!(matches!(read_file(&file), Err(RecentError::TooLarge { .. })));
    }

    /// Recent never lists a missing file: a bookmark whose file is gone
    /// is left out, and the rest keep their order and their folders.
    #[test]
    fn recent_never_lists_a_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("recently-used.xbel");
        let backend = crate::backend::mock::MockBackend::new();
        backend.seed("/home/me", vec![crate::backend::mock::MockBackend::file(Path::new("/home/me"), "here.txt", 3)]);
        let text = record(None, Path::new("/home/me/gone.txt"), None, at("2026-10-02T00:00:00Z")).unwrap();
        let text = record(Some(&text), Path::new("/home/me/here.txt"), None, at("2026-10-01T00:00:00Z")).unwrap();
        std::fs::write(&file, text).unwrap();
        let shown = read_entries(&backend, &file).unwrap();
        assert_eq!(shown.len(), 1);
        assert_eq!(shown[0].path, Path::new("/home/me/here.txt"));
        assert_eq!(shown[0].origin.as_deref(), Some(Path::new("/home/me")));
    }
}
