//! Other programs' thumbnailers: the `*.thumbnailer` files under
//! `$XDG_DATA_DIRS/thumbnailers`.
//!
//! A package that can read a format nobody else can — glycin for AVIF,
//! HEIF and JPEG XL, evince for comic books, totem for whatever
//! GStreamer plays — installs a small key file naming the MIME types it
//! handles and a command line that writes a PNG:
//!
//! ```text
//! [Thumbnailer Entry]
//! TryExec=/usr/bin/glycin-thumbnailer
//! Exec=/usr/bin/glycin-thumbnailer --input %u --output %o --size %s
//! MimeType=image/avif;image/heif;
//! ```
//!
//! `%i` is the input's path, `%u` its URI, `%o` where the PNG goes and
//! `%s` the edge in pixels; `%%` is a percent sign. Nautilus, Nemo and
//! Thunar all run these, which is why a format gets thumbnails on a
//! desktop without any one file manager learning to read it.
//!
//! # Run as someone else's program, so bounded like one
//!
//! - [`hyprforge_process::output`] with the caller's timeout, so a
//!   thumbnailer that hangs on a damaged file is killed rather than
//!   waited on — CLAUDE.md's rule about waiting on another process.
//!   The kill reaches the program named in `Exec`; anything *it* started
//!   (glycin runs its decoders in a sandbox of their own) is not
//!   followed, and is that program's to clean up.
//! - Into a directory made for the one run, owner-only, with a name no
//!   other process chose: the output path is on the command line, and a
//!   predictable name in `/tmp` is a file somebody else could have put a
//!   symlink at first.
//! - What it wrote is read back through the same caps as a thumbnail
//!   from the cache — the file's size before it is opened, the picture's
//!   dimensions before its buffer is allocated — and scaled down to the
//!   edge asked for when the program overshot.
//!
//! # Which file wins
//!
//! The data directories in order, `$XDG_DATA_HOME` first, and the first
//! file of a given *name* hides any later one — the XDG rule that lets a
//! person override a system thumbnailer by writing one of the same name
//! in `~/.local/share/thumbnailers`. Among different files the first to
//! claim a MIME type keeps it.

use crate::{decode_png, fit, file_uri, Rgba};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

/// A thumbnailer's output larger than this is refused unread: a PNG of
/// [`crate::MAX_EDGE`] square is 4MB of pixels before compression, so
/// twice that is already generous.
const MAX_OUTPUT: u64 = 8 * 1024 * 1024;

/// One `*.thumbnailer` file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Thumbnailer {
    /// The file's name without `.thumbnailer` — what Preferences lists.
    pub name: String,
    /// `TryExec`, when there is one: the program whose absence means the
    /// file is left over from an uninstalled package.
    pub try_exec: Option<String>,
    /// `Exec`, split into words, field codes still in them.
    pub exec: Vec<String>,
    /// `MimeType`, each one.
    pub mime_types: Vec<String>,
}

impl Thumbnailer {
    /// Parses a `.thumbnailer` file's text. `None` when it has no
    /// `[Thumbnailer Entry]` group, no `Exec`, or no types — a file that
    /// cannot run or would never be asked.
    pub fn parse(name: &str, text: &str) -> Option<Thumbnailer> {
        let mut in_entry = false;
        let (mut try_exec, mut exec, mut mime_types) = (None, None, Vec::new());
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if line.starts_with('[') {
                in_entry = line == "[Thumbnailer Entry]";
                continue;
            }
            if !in_entry {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else { continue };
            match key.trim() {
                "TryExec" => try_exec = Some(value.trim().to_string()),
                "Exec" => exec = Some(split_exec(value.trim())?),
                "MimeType" => {
                    mime_types = value.split(';').map(str::trim).filter(|m| !m.is_empty()).map(String::from).collect()
                }
                _ => {}
            }
        }
        let exec = exec.filter(|e| !e.is_empty())?;
        (!mime_types.is_empty()).then(|| Thumbnailer { name: name.to_string(), try_exec, exec, mime_types })
    }

    /// Whether its program is installed: `TryExec` when it says, and the
    /// program `Exec` starts otherwise.
    pub fn is_installed(&self) -> bool {
        let program = self.try_exec.as_deref().unwrap_or(&self.exec[0]);
        executable(program)
    }

    /// The command line for one run: `input` thumbnailed into `output`
    /// at `size` pixels. Every field code inside a word is expanded in
    /// place, so `--size=%s` works as well as `--size %s`; a word is
    /// never split, so a path with spaces in it stays one argument.
    pub fn command_line(&self, input: &Path, output: &Path, size: u32) -> Vec<String> {
        self.exec.iter().map(|word| expand(word, input, output, size)).collect()
    }

    /// Runs it on `input`, waiting at most `timeout`.
    ///
    /// `Ok(Some)` is a picture no larger than `size`; `Ok(None)` is the
    /// program running and saying no — the file's failure, to record.
    /// `Err` is no verdict on the file: the program could not be
    /// started, or did not finish in time and was killed.
    pub fn run(&self, input: &Path, size: u32, timeout: Duration) -> std::io::Result<Option<Rgba>> {
        let dir = PrivateDir::new()?;
        let output = dir.0.join("thumbnail.png");
        let words = self.command_line(input, &output, size);
        let mut command = Command::new(&words[0]);
        command.args(&words[1..]);
        let finished = hyprforge_process::output(&mut command, timeout)?;
        if !finished.status.success() {
            tracing::debug!(thumbnailer = %self.name, status = %finished.status, "thumbnailer said no");
            return Ok(None);
        }
        Ok(read_output(&output).map(|rgba| fit(rgba, size)))
    }
}

/// What a thumbnailer wrote, if it is a picture within the caps.
fn read_output(path: &Path) -> Option<Rgba> {
    let len = std::fs::metadata(path).ok()?.len();
    if len > MAX_OUTPUT {
        tracing::debug!(bytes = len, "thumbnailer output refused as too large");
        return None;
    }
    decode_png(&std::fs::read(path).ok()?)
}

/// `word` with `%i`, `%u`, `%o`, `%s` and `%%` replaced. An unknown
/// code is dropped, as the desktop entry specification says of `Exec`.
fn expand(word: &str, input: &Path, output: &Path, size: u32) -> String {
    let mut out = String::new();
    let mut chars = word.chars();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('i') => out.push_str(&input.to_string_lossy()),
            Some('u') => out.push_str(&file_uri(input)),
            Some('o') => out.push_str(&output.to_string_lossy()),
            Some('s') => out.push_str(&size.to_string()),
            Some('%') => out.push('%'),
            _ => {}
        }
    }
    out
}

/// `Exec`'s value as words: split at spaces, with the desktop entry
/// specification's double quotes — inside which `\"`, `` \` ``, `\$` and
/// `\\` are the character itself. `None` for an unterminated quote,
/// which is a broken file rather than something to guess at.
fn split_exec(value: &str) -> Option<Vec<String>> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut in_word = false;
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                in_word = true;
                loop {
                    match chars.next()? {
                        '"' => break,
                        '\\' => word.push(chars.next()?),
                        other => word.push(other),
                    }
                }
            }
            ' ' | '\t' => {
                if in_word {
                    words.push(std::mem::take(&mut word));
                    in_word = false;
                }
            }
            other => {
                in_word = true;
                word.push(other);
            }
        }
    }
    if in_word {
        words.push(word);
    }
    Some(words)
}

/// Whether `program` — a path, or a name looked up on `$PATH` — is an
/// executable file.
fn executable(program: &str) -> bool {
    use std::os::unix::fs::PermissionsExt;
    let is_exec = |p: &Path| std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0);
    if program.contains('/') {
        return is_exec(Path::new(program));
    }
    std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| is_exec(&dir.join(program))))
}

/// Every thumbnailer installed, in precedence order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Thumbnailers {
    list: Vec<Thumbnailer>,
}

impl Thumbnailers {
    /// The ones under the standard data directories whose program is
    /// installed. Reads a directory or two of small files; once per
    /// process is plenty.
    pub fn discover() -> Thumbnailers {
        Thumbnailers::load_from(&data_dirs())
    }

    /// The ones under each of `dirs`' `thumbnailers/`, first directory
    /// first — the seam the tests use. Leaves out any whose program is
    /// not installed.
    pub fn load_from(dirs: &[PathBuf]) -> Thumbnailers {
        let mut seen = HashSet::new();
        let mut list = Vec::new();
        for dir in dirs {
            let Ok(entries) = std::fs::read_dir(dir.join("thumbnailers")) else { continue };
            let mut files: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
            // Directory order is the filesystem's whim; name order is the
            // same on every machine, which is what makes "the first to
            // claim a type keeps it" mean something.
            files.sort();
            for path in files {
                let Some(name) = path.file_name().and_then(|n| n.to_str()).and_then(|n| n.strip_suffix(".thumbnailer"))
                else {
                    continue;
                };
                if !seen.insert(name.to_string()) {
                    continue;
                }
                let Ok(text) = std::fs::read_to_string(&path) else { continue };
                if let Some(t) = Thumbnailer::parse(name, &text).filter(Thumbnailer::is_installed) {
                    list.push(t);
                }
            }
        }
        Thumbnailers { list }
    }

    /// From a list already made — for a test, or a host that filtered.
    pub fn from_list(list: Vec<Thumbnailer>) -> Thumbnailers {
        Thumbnailers { list }
    }

    /// The thumbnailer for `mime`, if any claims it.
    pub fn for_mime(&self, mime: &str) -> Option<&Thumbnailer> {
        self.list.iter().find(|t| t.mime_types.iter().any(|m| m == mime))
    }

    /// Every one, in precedence order.
    pub fn all(&self) -> &[Thumbnailer] {
        &self.list
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }
}

/// `$XDG_DATA_HOME` (or `~/.local/share`), then each of
/// `$XDG_DATA_DIRS` (or `/usr/local/share:/usr/share`).
pub fn data_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    match std::env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()) {
        Some(home) => dirs.push(PathBuf::from(home)),
        None => {
            if let Some(home) = std::env::var_os("HOME") {
                dirs.push(PathBuf::from(home).join(".local/share"));
            }
        }
    }
    let system = std::env::var("XDG_DATA_DIRS")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".to_string());
    dirs.extend(system.split(':').filter(|d| !d.is_empty()).map(PathBuf::from));
    dirs
}

/// A directory made for one run and removed with everything in it when
/// dropped — a thumbnailer that was killed may have left half a file.
struct PrivateDir(PathBuf);

impl PrivateDir {
    fn new() -> std::io::Result<PrivateDir> {
        use std::os::unix::fs::DirBuilderExt;
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.subsec_nanos());
        let name = format!("hyprforge-thumbnailer-{}-{}-{nanos}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed));
        let path = std::env::temp_dir().join(name);
        // Not `recursive`: creating it must fail if anything is already
        // there, so the directory is certainly the one made here.
        std::fs::DirBuilder::new().mode(0o700).create(&path)?;
        Ok(PrivateDir(path))
    }
}

impl Drop for PrivateDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    const GLYCIN: &str = "[Thumbnailer Entry]\n\
        TryExec=/usr/bin/glycin-thumbnailer\n\
        Exec=/usr/bin/glycin-thumbnailer --input %u --output %o --size %s\n\
        MimeType=image/avif;image/heif;\n";

    /// The shape glycin installs on this machine, word for word.
    #[test]
    fn a_thumbnailer_file_is_read_as_its_program_and_its_types() {
        let t = Thumbnailer::parse("glycin-heif", GLYCIN).unwrap();
        assert_eq!(t.try_exec.as_deref(), Some("/usr/bin/glycin-thumbnailer"));
        assert_eq!(t.exec, ["/usr/bin/glycin-thumbnailer", "--input", "%u", "--output", "%o", "--size", "%s"]);
        assert_eq!(t.mime_types, ["image/avif", "image/heif"]);
    }

    /// Each code becomes the right thing, a path with a space stays one
    /// argument, a code inside a word is expanded in place, and `%%` is a
    /// percent sign.
    #[test]
    fn exec_is_expanded_with_the_right_paths() {
        let text = "[Thumbnailer Entry]\nExec=thumb -i %i -u %u -o %o --size=%s \"quoted %i\" 100%%\nMimeType=a/b;\n";
        let t = Thumbnailer::parse("x", text).unwrap();
        let line = t.command_line(Path::new("/d/a b;c.avif"), Path::new("/tmp/out dir/t.png"), 256);
        assert_eq!(
            line,
            [
                "thumb",
                "-i",
                "/d/a b;c.avif",
                "-u",
                "file:///d/a%20b%3Bc.avif",
                "-o",
                "/tmp/out dir/t.png",
                "--size=256",
                "quoted /d/a b;c.avif",
                "100%",
            ]
        );
    }

    #[test]
    fn a_file_that_could_never_run_is_not_a_thumbnailer() {
        assert_eq!(Thumbnailer::parse("x", "[Thumbnailer Entry]\nMimeType=a/b;\n"), None, "no Exec");
        assert_eq!(Thumbnailer::parse("x", "[Thumbnailer Entry]\nExec=thumb %i %o\n"), None, "no types");
        assert_eq!(Thumbnailer::parse("x", "[Desktop Entry]\nExec=t\nMimeType=a/b;\n"), None, "wrong group");
        assert_eq!(Thumbnailer::parse("x", "[Thumbnailer Entry]\nExec=\"open\nMimeType=a/b;\n"), None, "bad quote");
    }

    /// A file named in `$XDG_DATA_HOME` hides the system's of the same
    /// name, and one whose program is gone is left out.
    #[test]
    fn the_first_file_of_a_name_wins_and_an_uninstalled_one_is_left_out() {
        let dir = tempfile::tempdir().unwrap();
        let (home, system) = (dir.path().join("home"), dir.path().join("system"));
        for d in [&home, &system] {
            std::fs::create_dir_all(d.join("thumbnailers")).unwrap();
        }
        let file = |at: &Path, name: &str, exec: &str, types: &str| {
            let text = format!("[Thumbnailer Entry]\nExec={exec} %i %o\nMimeType={types}\n");
            std::fs::write(at.join("thumbnailers").join(format!("{name}.thumbnailer")), text).unwrap();
        };
        file(&home, "mine", "/bin/sh", "image/x-mine;");
        file(&system, "mine", "/bin/sh", "image/x-theirs;");
        file(&system, "gone", "/nonexistent/thumbnailer", "image/x-gone;");
        let found = Thumbnailers::load_from(&[home, system]);
        assert_eq!(found.all().len(), 1, "{found:?}");
        assert!(found.for_mime("image/x-mine").is_some());
        assert!(found.for_mime("image/x-theirs").is_none(), "hidden by the user's file of the same name");
        assert!(found.for_mime("image/x-gone").is_none());
    }

    fn sh(script: &str) -> Thumbnailer {
        Thumbnailer {
            name: "test".into(),
            try_exec: None,
            exec: vec!["/bin/sh".into(), "-c".into(), script.into(), "sh".into(), "%i".into(), "%o".into(), "%s".into()],
            mime_types: vec!["a/b".into()],
        }
    }

    /// A thumbnailer that hangs is killed at the bound: the call returns
    /// near the timeout, and what the script would have done after its
    /// sleep never happens — the process is gone, not merely abandoned.
    #[test]
    fn a_hanging_thumbnailer_is_killed_at_the_bound() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("still-running");
        let t = sh(&format!("sleep 1; touch '{}'", marker.display()));
        let started = Instant::now();
        let result = t.run(Path::new("/dev/null"), 128, Duration::from_millis(150));
        assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::TimedOut);
        assert!(started.elapsed() < Duration::from_millis(900), "waited {:?}", started.elapsed());
        std::thread::sleep(Duration::from_millis(1300));
        assert!(!marker.exists(), "the shell outlived its timeout");
    }

    /// A PNG written uncompressed by hand, so the tests need no encoder.
    fn png(width: u32, height: u32) -> Vec<u8> {
        let rgba = Rgba { width, height, pixels: [10, 20, 30, 255].repeat((width * height) as usize) };
        crate::encode(&rgba, "file:///x", crate::Stamp { mtime: 0, size: 0 }).unwrap()
    }

    /// What a thumbnailer writes passes the size cap: an overshoot is
    /// scaled down to the size asked for, and a picture past the cap is
    /// refused as the file's failure rather than allocated.
    #[test]
    fn a_thumbnailers_output_passes_the_size_cap() {
        let dir = tempfile::tempdir().unwrap();
        let overshoot = dir.path().join("overshoot.png");
        std::fs::write(&overshoot, png(600, 300)).unwrap();
        let t = sh("cp \"$1\" \"$2\"");
        let made = t.run(&overshoot, 256, hyprforge_process::TIMEOUT).unwrap().unwrap();
        assert_eq!((made.width, made.height), (256, 128));

        let vast = dir.path().join("vast.png");
        std::fs::write(&vast, png(crate::MAX_EDGE + 1, 2)).unwrap();
        assert_eq!(t.run(&vast, 256, hyprforge_process::TIMEOUT).unwrap(), None);
    }

    /// Ran and said no is the file's failure; nothing written likewise.
    #[test]
    fn a_thumbnailer_that_fails_is_a_verdict_on_the_file() {
        assert_eq!(sh("exit 3").run(Path::new("/dev/null"), 128, hyprforge_process::TIMEOUT).unwrap(), None);
        assert_eq!(sh("true").run(Path::new("/dev/null"), 128, hyprforge_process::TIMEOUT).unwrap(), None);
    }

    /// The run's directory goes with it, whatever the program left.
    #[test]
    fn the_temporary_directory_is_removed_after_a_run() {
        let dir = tempfile::tempdir().unwrap();
        let record = dir.path().join("where");
        let t = sh(&format!("dirname \"$2\" > '{}'; echo partial > \"$2\"; exit 1", record.display()));
        assert_eq!(t.run(Path::new("/dev/null"), 128, hyprforge_process::TIMEOUT).unwrap(), None);
        let used = std::fs::read_to_string(&record).unwrap();
        assert!(!Path::new(used.trim()).exists(), "{used} left behind");
    }
}
