//! Network shares through gvfs: what it can reach, what it has mounted,
//! and the conversation `gio mount` has when a server wants a password.
//!
//! # Why gvfs, and why through `gio`
//!
//! Mounting `smb://` or `sftp://` without root needs something that
//! already holds the session's credentials, speaks the protocols and
//! exposes the result as files. On a desktop that is gvfs: its daemon is
//! running for any GTK application that has touched a remote location,
//! and every share it mounts appears as a real directory under
//! `$XDG_RUNTIME_DIR/gvfs` through its FUSE bridge — so once a share is
//! mounted, this file manager browses it with exactly the code it
//! browses a home directory with. Writing a second SMB and SFTP client
//! would be this suite reimplementing a daemon the desktop already runs,
//! which it does nowhere else.
//!
//! The mounting goes through the `gio mount` command rather than gvfs's
//! private D-Bus protocol, which is gvfs's own business and changes
//! between releases; `gio`'s command line is the published interface.
//! Its one awkwardness is that it asks for a password by printing a
//! prompt and reading a line, so [`Conversation`] reads those prompts
//! and answers each from what the person typed into a dialog. A prompt
//! nothing typed answers stops the attempt and becomes a
//! [`ConnectError::NeedsLogin`] or [`ConnectError::Question`] for the
//! dialog to ask — then the attempt runs again with the answers.
//!
//! Everything in this module is pure — the text `gio` printed in, a
//! decision out. Running it is `crate::network`'s.

use crate::types::{Answers, ConnectError, Gvfs, Login, Question, Share, ShareKind};
use hyprforge_secret::Secret;
use std::path::Path;

/// The schemes gvfs can mount, from its `*.mount` files — each one's
/// `Scheme=` (or `Type=` when it has none) and `SchemeAliases=`.
///
/// `files` is `(file name, contents)`; anything that is not a `.mount`
/// file is ignored.
pub fn schemes(files: &[(String, String)]) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for (name, text) in files {
        if !name.ends_with(".mount") {
            continue;
        }
        let value = |key: &str| {
            text.lines()
                .find_map(|line| line.trim().strip_prefix(key)?.strip_prefix('=').map(|v| v.trim().to_string()))
        };
        let scheme = value("Scheme").or_else(|| value("Type"));
        let aliases = value("SchemeAliases").unwrap_or_default();
        for s in scheme.into_iter().chain(aliases.split(';').map(str::to_string)) {
            let s = s.trim().to_ascii_lowercase();
            if !s.is_empty() && !found.contains(&s) {
                found.push(s);
            }
        }
    }
    found.sort();
    found
}

/// The schemes worth naming in a dialog's hint — the ones a person
/// types — of those gvfs actually has.
pub fn offered(schemes: &[String]) -> Vec<&str> {
    ["smb", "sftp", "ftp", "ftps", "dav", "davs", "afp", "nfs"]
        .into_iter()
        .filter(|s| schemes.iter().any(|have| have == s))
        .collect()
}

/// Whether gvfs is here: `gio_found` says whether the `gio` command is
/// on `PATH`, `mount_files` is what its mounts directory held — `None`
/// when there was no such directory.
pub fn availability(gio_found: bool, mount_files: Option<&[(String, String)]>) -> Gvfs {
    let Some(files) = mount_files else {
        return Gvfs::Absent(
            "Connecting to servers needs gvfs, and it isn't installed. Install gvfs (and gvfs-smb for Windows shares) to use it."
                .to_string(),
        );
    };
    if !gio_found {
        return Gvfs::Absent(
            "Connecting to servers needs the gio command from GLib, and it isn't on this system's PATH.".to_string(),
        );
    }
    let schemes = schemes(files);
    if schemes.is_empty() {
        return Gvfs::Absent("gvfs is installed, but has no backends for reaching servers.".to_string());
    }
    Gvfs::Available { schemes }
}

/// What a typed address means: the URI to hand to `gio mount`, or why
/// it cannot be one.
///
/// `smb://nas/music` and `sftp://u@box` as typed; a scheme in capitals
/// is lowered (`SMB://` is what a person copying from a Windows box
/// types). A bare host is refused with an example rather than guessed
/// at — `nas` could be SMB, SFTP or a typo, and guessing wrong costs a
/// timeout.
pub fn address(text: &str, schemes: &[String]) -> Result<String, ConnectError> {
    let text = text.trim();
    let Some((scheme, rest)) = text.split_once(':') else {
        return Err(ConnectError::NotAnAddress(text.to_string()));
    };
    let scheme_ok = !scheme.is_empty()
        && scheme.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    if !scheme_ok || !rest.starts_with("//") {
        return Err(ConnectError::NotAnAddress(text.to_string()));
    }
    let scheme = scheme.to_ascii_lowercase();
    if !schemes.contains(&scheme) {
        return Err(ConnectError::Unsupported { scheme });
    }
    Ok(format!("{scheme}:{rest}"))
}

/// A share gvfs has mounted, from its directory's name under the FUSE
/// root — `smb-share:server=nas,share=music`, `sftp:host=box,user=u`,
/// `localtest:`.
///
/// The name is gvfs's mount spec: a type, then `key=value` pairs, with
/// anything awkward in a value percent-escaped. It is all a sidebar
/// needs — who and where — without asking gvfs a question per share.
pub fn share(fuse_root: &Path, name: &str) -> Option<Share> {
    let (kind, spec) = name.split_once(':')?;
    if kind.is_empty() {
        return None;
    }
    let field = |key: &str| {
        spec.split(',')
            .find_map(|pair| pair.split_once('=').filter(|(k, _)| *k == key).map(|(_, v)| unescape(v)))
    };
    let host = field("host").or_else(|| field("server"));
    let label = match (field("share"), &host, field("user")) {
        (Some(share), Some(host), _) => format!("{share} on {host}"),
        (None, Some(host), Some(user)) => format!("{user}@{host}"),
        (None, Some(host), None) => host.clone(),
        _ => kind.to_string(),
    };
    let scheme = match kind {
        "smb-share" | "smb-server" => "smb".to_string(),
        "dav" if field("ssl").as_deref() == Some("true") => "davs".to_string(),
        other => other.to_string(),
    };
    Some(Share { label, path: fuse_root.join(name), kind: ShareKind::Gvfs { scheme } })
}

/// `%2C` back to `,`. A malformed escape is kept as written.
fn unescape(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if let Some(byte) = bytes.get(i + 1..i + 3).and_then(|h| std::str::from_utf8(h).ok()).and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// What `gio mount` is asking for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Prompt {
    User { default: Option<String> },
    Domain { default: Option<String> },
    Password,
    Choice,
}

/// A prompt, with what was printed above it since the last answer: the
/// server's message, and for a question its numbered choices.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asked {
    pub prompt: Prompt,
    pub message: String,
    pub choices: Vec<String>,
}

/// The prompt `output` ends on, if it ends on one.
///
/// `gio` prints `User [bob]: `, `Domain [WORKGROUP]: `, `Password: ` and
/// `Choice: ` — always as the last thing on a line with no newline after
/// it, because it is waiting. Text that merely *contains* "Password:"
/// earlier is a message, not a prompt. These words are only English
/// because the runner starts `gio` with `LC_ALL=C.UTF-8`; under a German
/// locale they would be `Benutzer`, and reading them would fail.
pub fn asked(output: &str) -> Option<Asked> {
    let (above, last) = match output.rfind('\n') {
        Some(at) => (&output[..at], &output[at + 1..]),
        None => ("", output),
    };
    let head = last.strip_suffix(": ")?;
    let (word, default) = match head.split_once(" [") {
        Some((word, rest)) => (word, rest.strip_suffix(']').map(str::to_string).filter(|d| !d.is_empty())),
        None => (head, None),
    };
    let prompt = match word {
        "User" => Prompt::User { default },
        "Domain" => Prompt::Domain { default },
        "Password" => Prompt::Password,
        "Choice" => Prompt::Choice,
        _ => return None,
    };
    let mut message = Vec::new();
    let mut choices = Vec::new();
    for line in above.lines() {
        let numbered = line
            .strip_prefix('[')
            .and_then(|rest| rest.split_once("] "))
            .filter(|(n, _)| n.parse::<usize>().is_ok());
        match numbered {
            Some((_, choice)) => choices.push(choice.to_string()),
            None if !line.trim().is_empty() => message.push(line.trim()),
            None => {}
        }
    }
    Some(Asked { prompt, message: message.join(" "), choices })
}

/// What to do about a prompt: type a line, or stop and ask the person.
///
/// The line is a [`Secret`] whatever it holds — a user name is not a
/// password, but a `Debug` of this enum must not be the one place a
/// password could be printed from.
#[derive(Debug, PartialEq, Eq)]
pub enum Reply {
    Write(Secret<String>),
    Stop(ConnectError),
}

/// One `gio mount` attempt's answers, and which prompts it has answered.
///
/// Each prompt is answered **once**. gvfs asks again when an answer was
/// refused, and answering the second time with the same password would
/// be a loop spending a server's lockout attempts — so a second `User`
/// or `Password` stops with [`Login::retry`] set, and the dialog says
/// that did not work.
#[derive(Debug)]
pub struct Conversation {
    answers: Answers,
    scheme: String,
    answered: Vec<std::mem::Discriminant<Prompt>>,
    /// The server's message, kept from the first prompt — later prompts
    /// arrive with nothing above them.
    message: String,
    user_default: Option<String>,
    domain_default: Option<String>,
}

impl Conversation {
    pub fn new(uri: &str, answers: Answers) -> Self {
        let scheme = uri.split_once(':').map(|(s, _)| s.to_string()).unwrap_or_default();
        Conversation {
            answers,
            scheme,
            answered: Vec::new(),
            message: String::new(),
            user_default: None,
            domain_default: None,
        }
    }

    fn login(&self, retry: bool) -> ConnectError {
        ConnectError::NeedsLogin(Login {
            message: if self.message.is_empty() {
                "The server wants a name and password.".to_string()
            } else {
                self.message.clone()
            },
            user: self.answers.user.clone().or_else(|| self.user_default.clone()),
            domain: self.answers.domain.clone().or_else(|| self.domain_default.clone()),
            asks_domain: self.scheme == "smb" || self.domain_default.is_some(),
            retry,
        })
    }

    pub fn reply(&mut self, asked: Asked) -> Reply {
        if !asked.message.is_empty() {
            self.message = asked.message.clone();
        }
        let kind = std::mem::discriminant(&asked.prompt);
        let again = self.answered.contains(&kind);
        self.answered.push(kind);
        let line = |s: &str| Reply::Write(Secret::new(format!("{s}\n")));
        match asked.prompt {
            Prompt::User { default } => {
                self.user_default = default;
                // Nothing typed yet means the dialog has not been shown:
                // stop, and ask. A second User prompt means the first
                // answer was refused.
                match (&self.answers.password, again) {
                    (_, true) => Reply::Stop(self.login(true)),
                    (None, false) => Reply::Stop(self.login(false)),
                    // An empty line takes gio's default name.
                    (Some(_), false) => line(self.answers.user.as_deref().unwrap_or("")),
                }
            }
            Prompt::Domain { default } => {
                self.domain_default = default;
                if again {
                    return Reply::Stop(self.login(true));
                }
                line(self.answers.domain.as_deref().unwrap_or(""))
            }
            Prompt::Password => match (&self.answers.password, again) {
                (_, true) => Reply::Stop(self.login(true)),
                (None, false) => Reply::Stop(self.login(false)),
                (Some(password), false) => Reply::Write(Secret::new(format!("{}\n", password.expose()))),
            },
            Prompt::Choice => match (self.answers.choice, again) {
                (Some(choice), false) if choice < asked.choices.len() => line(&(choice + 1).to_string()),
                _ => Reply::Stop(ConnectError::Question(Question {
                    message: if asked.message.is_empty() { self.message.clone() } else { asked.message },
                    choices: asked.choices,
                })),
            },
        }
    }
}

/// gio's error, without gio's prefix: `gio: smb://h/x/: Failed to mount
/// Windows share: Connection refused` becomes the part a person can
/// read. The last non-empty line, because gio prints the error last.
pub fn failure(stderr: &str) -> String {
    let line = stderr.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
    let line = line.strip_prefix("gio: ").unwrap_or(line);
    // The location comes next, ending in ": " — a URI has no ": " of its
    // own, so the first one is where the message starts.
    match line.split_once(": ") {
        Some((location, message)) if location.contains(':') => message.to_string(),
        _ => line.to_string(),
    }
}

/// `local path: /run/user/1000/gvfs/…` out of `gio info`'s output — where
/// a share just mounted can be browsed.
pub fn local_path(info: &str) -> Option<std::path::PathBuf> {
    info.lines()
        .find_map(|l| l.strip_prefix("local path: "))
        .map(|p| std::path::PathBuf::from(p.trim()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(list: &[(&str, &str)]) -> Vec<(String, String)> {
        list.iter().map(|(n, t)| (n.to_string(), t.to_string())).collect()
    }

    /// Copied from `/usr/share/gvfs/mounts` on the machine this was
    /// written on: `sftp.mount` has an alias, `smb.mount` a scheme that
    /// is not its type, and `http.mount` no `Scheme=` at all.
    fn real_mounts() -> Vec<(String, String)> {
        files(&[
            ("sftp.mount", "[Mount]\nType=sftp\nExec=/usr/lib/gvfsd-sftp\nAutoMount=false\nScheme=sftp\nSchemeAliases=ssh\nDefaultPort=22\nHostnameIsInetAddress=true\n"),
            ("smb.mount", "[Mount]\nType=smb-share\nExec=/usr/lib/gvfsd-smb\nAutoMount=false\nScheme=smb\n"),
            ("http.mount", "[Mount]\nType=http\nExec=/usr/lib/gvfsd-http\nAutoMount=true\nDBusName=org.gtk.vfs.mountpoint_http\n"),
            ("README", "not a mount file"),
        ])
    }

    #[test]
    fn schemes_come_from_scheme_type_and_aliases() {
        assert_eq!(schemes(&real_mounts()), ["http", "sftp", "smb", "ssh"]);
    }

    /// The machine this was written on has no WebDAV backend; offering
    /// `dav://` in the hint would be offering something that fails.
    #[test]
    fn the_dialog_offers_only_schemes_gvfs_has() {
        let have = schemes(&real_mounts());
        assert_eq!(offered(&have), ["smb", "sftp"]);
    }

    #[test]
    fn gvfs_missing_and_gio_missing_say_different_things() {
        let no_gvfs = availability(true, None);
        let no_gio = availability(false, Some(&real_mounts()));
        assert!(matches!(&no_gvfs, Gvfs::Absent(s) if s.contains("gvfs") && s.contains("isn't installed")));
        assert!(matches!(&no_gio, Gvfs::Absent(s) if s.contains("gio")));
        assert!(matches!(availability(true, Some(&real_mounts())), Gvfs::Available { .. }));
    }

    #[test]
    fn an_address_needs_a_scheme_gvfs_can_reach() {
        let have = schemes(&real_mounts());
        assert_eq!(address("  SMB://nas/music ", &have).unwrap(), "smb://nas/music");
        assert_eq!(address("ssh://u@box", &have).unwrap(), "ssh://u@box");
        assert!(matches!(address("nas", &have), Err(ConnectError::NotAnAddress(_))));
        assert!(matches!(address("smb:nas", &have), Err(ConnectError::NotAnAddress(_))));
        assert!(matches!(
            address("dav://box/files", &have),
            Err(ConnectError::Unsupported { scheme }) if scheme == "dav"
        ));
    }

    /// `localtest:` is the one name read off a live FUSE directory here;
    /// the others are in the shape gvfs's mount specs take.
    #[test]
    fn a_fuse_name_becomes_a_labelled_share() {
        let root = Path::new("/run/user/1000/gvfs");
        let smb = share(root, "smb-share:server=nas,share=music").unwrap();
        assert_eq!(smb.label, "music on nas");
        assert_eq!(smb.kind, ShareKind::Gvfs { scheme: "smb".into() });
        assert_eq!(smb.path, root.join("smb-share:server=nas,share=music"));
        assert_eq!(share(root, "sftp:host=box,user=u").unwrap().label, "u@box");
        assert_eq!(share(root, "ftp:host=files.example.com").unwrap().label, "files.example.com");
        assert_eq!(share(root, "localtest:").unwrap().label, "localtest");
        assert_eq!(share(root, "dav:host=h,ssl=true").unwrap().kind, ShareKind::Gvfs { scheme: "davs".into() });
        assert_eq!(share(root, "smb-share:server=nas,share=a%2Cb").unwrap().label, "a,b on nas");
        assert!(share(root, "no-colon").is_none());
    }

    /// Shaped as `gio mount` prints it: the server's message on a line,
    /// then the prompt with its default in brackets and nothing after.
    #[test]
    fn a_prompt_is_recognised_only_at_the_end() {
        let out = "Password required for share music on nas\nUser [apost]: ";
        let a = asked(out).unwrap();
        assert_eq!(a.prompt, Prompt::User { default: Some("apost".into()) });
        assert_eq!(a.message, "Password required for share music on nas");
        assert_eq!(asked("Password: ").unwrap().prompt, Prompt::Password);
        assert!(asked("Password: \nmounted\n").is_none(), "answered already");
        assert!(asked("Mounted smb://nas/music").is_none());
    }

    #[test]
    fn a_question_carries_its_choices() {
        let out = "The identity of the remote computer (box) is unknown.\n[1] Log In Anyway\n[2] Cancel Login\nChoice: ";
        let a = asked(out).unwrap();
        assert_eq!(a.prompt, Prompt::Choice);
        assert_eq!(a.choices, ["Log In Anyway", "Cancel Login"]);
        assert!(a.message.starts_with("The identity"));
    }

    fn user_prompt() -> Asked {
        Asked { prompt: Prompt::User { default: Some("apost".into()) }, message: "Password required".into(), choices: vec![] }
    }

    fn password_prompt() -> Asked {
        Asked { prompt: Prompt::Password, message: String::new(), choices: vec![] }
    }

    #[test]
    fn with_nothing_typed_the_first_prompt_stops_and_asks() {
        let mut talk = Conversation::new("smb://nas/music", Answers::default());
        match talk.reply(user_prompt()) {
            Reply::Stop(ConnectError::NeedsLogin(login)) => {
                assert_eq!(login.user.as_deref(), Some("apost"), "gio's default is offered");
                assert!(login.asks_domain, "a Windows share asks for a domain");
                assert!(!login.retry);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn typed_answers_are_written_one_per_prompt() {
        let answers = Answers {
            user: Some("bob".into()),
            password: Some(Secret::new("pw".to_string())),
            ..Answers::default()
        };
        let mut talk = Conversation::new("sftp://box", answers);
        assert_eq!(talk.reply(user_prompt()), Reply::Write(Secret::new("bob\n".into())));
        let domain = Asked { prompt: Prompt::Domain { default: Some("WORKGROUP".into()) }, message: String::new(), choices: vec![] };
        assert_eq!(talk.reply(domain), Reply::Write(Secret::new("\n".into())), "an empty line takes the default");
        assert_eq!(talk.reply(password_prompt()), Reply::Write(Secret::new("pw\n".into())));
    }

    /// gvfs asks again after a wrong password. Answering again would
    /// spend the server's lockout attempts in a loop.
    #[test]
    fn a_refused_password_is_never_sent_twice() {
        let answers = Answers { password: Some(Secret::new("wrong".to_string())), ..Answers::default() };
        let mut talk = Conversation::new("sftp://box", answers);
        assert!(matches!(talk.reply(user_prompt()), Reply::Write(_)));
        assert!(matches!(talk.reply(password_prompt()), Reply::Write(_)));
        assert!(matches!(talk.reply(user_prompt()), Reply::Stop(ConnectError::NeedsLogin(Login { retry: true, .. }))));
    }

    #[test]
    fn a_question_is_answered_only_with_a_choice_it_offered() {
        let question = Asked { prompt: Prompt::Choice, message: "Unknown host".into(), choices: vec!["Log In Anyway".into(), "Cancel".into()] };
        let mut ask = Conversation::new("sftp://box", Answers::default());
        assert!(matches!(ask.reply(question.clone()), Reply::Stop(ConnectError::Question(q)) if q.choices.len() == 2));
        let mut chosen = Conversation::new("sftp://box", Answers { choice: Some(0), ..Answers::default() });
        assert_eq!(chosen.reply(question.clone()), Reply::Write(Secret::new("1\n".into())));
        let mut out_of_range = Conversation::new("sftp://box", Answers { choice: Some(7), ..Answers::default() });
        assert!(matches!(out_of_range.reply(question), Reply::Stop(ConnectError::Question(_))));
    }

    /// Both copied from `gio mount` run on the machine this was written
    /// on.
    #[test]
    fn gios_prefix_and_location_are_taken_off_its_error() {
        assert_eq!(
            failure("gio: smb://127.0.0.1/nothing/: Failed to mount Windows share: Connection refused\n"),
            "Failed to mount Windows share: Connection refused"
        );
        assert_eq!(failure("gio: localtest:///: The specified location is not mounted"), "The specified location is not mounted");
        assert_eq!(failure("something else entirely"), "something else entirely");
    }

    /// `gio info localtest:///`, copied from this machine.
    #[test]
    fn the_local_path_is_read_from_gio_info() {
        let info = "display name: /\nedit name: /\nname: /\ntype: directory\nuri: localtest:///\nlocal path: /run/user/1000/gvfs/localtest:\nunix mount: gvfsd-fuse /run/user/1000/gvfs fuse.gvfsd-fuse rw\n";
        assert_eq!(local_path(info), Some(std::path::PathBuf::from("/run/user/1000/gvfs/localtest:")));
        assert_eq!(local_path("type: directory\n"), None);
    }
}
