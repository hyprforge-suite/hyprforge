//! The real [`ShareBackend`]: the mount table, gvfs's FUSE directory,
//! and `gio`.
//!
//! See [`crate::gvfs`] for why the mounting goes through gvfs and its
//! `gio` command; this file runs it. Every wait here is bounded —
//! `gio mount` by [`CONNECT_TIMEOUT`] and by being killed when the
//! person cancels, everything else by `hyprforge_process` — because a
//! server that stops answering is the normal case for a network, not an
//! exception.

use crate::backend::ShareBackend;
use crate::gvfs::{self, Conversation, Reply};
use crate::mountinfo;
use crate::types::{Answers, ConnectError, Gvfs, Share, ShareKind};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;

/// The most one connection attempt may take, prompts included. A
/// server that has not answered in a minute is not going to.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(60);

/// Reading the mount table and the FUSE directory, `gio info`, and
/// disconnecting.
pub const TIMEOUT: Duration = Duration::from_secs(10);

/// Network shares on this machine.
pub struct SystemShares {
    /// How to run `gio` — the program and any arguments before the
    /// subcommand. `["gio"]` for real; a test runs a stand-in script
    /// through `sh` instead, which is never itself exec'd and so cannot
    /// race a sibling test's fork into "text file busy".
    gio: Vec<String>,
    /// `$XDG_RUNTIME_DIR/gvfs`, where gvfs's FUSE bridge shows its mounts.
    fuse_root: PathBuf,
    /// Every `gvfs/mounts` directory in `$XDG_DATA_DIRS`.
    mounts_dirs: Vec<PathBuf>,
}

impl Default for SystemShares {
    fn default() -> Self {
        Self::new()
    }
}

impl SystemShares {
    pub fn new() -> Self {
        let runtime = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(|| {
            use std::os::unix::fs::MetadataExt;
            let uid = std::fs::metadata("/proc/self").map(|m| m.uid()).unwrap_or(0);
            PathBuf::from(format!("/run/user/{uid}"))
        });
        let data_dirs = std::env::var("XDG_DATA_DIRS")
            .ok()
            .filter(|d| !d.is_empty())
            .unwrap_or_else(|| "/usr/local/share:/usr/share".to_string());
        SystemShares {
            gio: vec!["gio".to_string()],
            fuse_root: runtime.join("gvfs"),
            mounts_dirs: data_dirs.split(':').map(|d| Path::new(d).join("gvfs/mounts")).collect(),
        }
    }

    /// The same, running `gio` as `command` — for tests.
    pub fn with_gio(command: Vec<String>, fuse_root: PathBuf, mounts_dirs: Vec<PathBuf>) -> Self {
        SystemShares { gio: command, fuse_root, mounts_dirs }
    }

    fn gio_command(&self) -> tokio::process::Command {
        let mut command = tokio::process::Command::new(&self.gio[0]);
        command.args(&self.gio[1..]);
        // gio's prompts and errors are translated; the conversation reads
        // the English words, so it asks for them.
        command.env("LC_ALL", "C.UTF-8").env_remove("LANGUAGE");
        command
    }

    fn gio_found(&self) -> bool {
        let program = Path::new(&self.gio[0]);
        if program.components().count() > 1 {
            return program.exists();
        }
        std::env::var_os("PATH")
            .map(|path| std::env::split_paths(&path).any(|dir| dir.join(program).is_file()))
            .unwrap_or(false)
    }

    /// What every `gvfs/mounts` directory holds, merged — `None` when
    /// there is no such directory at all, which is gvfs not installed.
    fn mount_files(&self) -> Option<Vec<(String, String)>> {
        let mut found = None::<Vec<(String, String)>>;
        for dir in &self.mounts_dirs {
            let Ok(entries) = std::fs::read_dir(dir) else { continue };
            let files = found.get_or_insert_with(Vec::new);
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if let Ok(text) = std::fs::read_to_string(entry.path()) {
                    files.push((name, text));
                }
            }
        }
        found
    }
}

/// Runs blocking work off the async runtime, bounded. `None` when it
/// did not finish in time — the thread is left to finish on its own,
/// holding nothing anyone is waiting for, the same choice the search
/// walk makes about a mount that stopped answering.
async fn blocking<T: Send + 'static>(limit: Duration, work: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    match tokio::time::timeout(limit, tokio::task::spawn_blocking(work)).await {
        Ok(Ok(value)) => Some(value),
        _ => None,
    }
}

#[async_trait::async_trait]
impl ShareBackend for SystemShares {
    async fn gvfs(&self) -> Gvfs {
        let found = self.gio_found();
        let files = self.mount_files();
        gvfs::availability(found, files.as_deref())
    }

    async fn shares(&self) -> Vec<Share> {
        let mut shares = blocking(TIMEOUT, || {
            std::fs::read_to_string("/proc/self/mountinfo")
                .map(|text| mountinfo::network_shares(&mountinfo::parse(&text)))
                .unwrap_or_default()
        })
        .await
        .unwrap_or_default();
        // A gvfs backend that has stopped answering can wedge a read of
        // its own FUSE directory; the kernel's shares are still worth
        // showing when it does.
        let root = self.fuse_root.clone();
        let gvfs = blocking(TIMEOUT, move || {
            let Ok(entries) = std::fs::read_dir(&root) else { return Vec::new() };
            let mut shares: Vec<Share> = entries
                .flatten()
                .filter_map(|e| gvfs::share(&root, &e.file_name().to_string_lossy()))
                .collect();
            shares.sort_by(|a, b| a.label.cmp(&b.label));
            shares
        })
        .await;
        match gvfs {
            Some(found) => shares.extend(found),
            None => tracing::warn!("gvfs's FUSE directory did not answer within {TIMEOUT:?}; showing the kernel's shares only"),
        }
        shares
    }

    async fn connect(&self, uri: &str, answers: Answers) -> Result<Option<PathBuf>, ConnectError> {
        self.mount(uri, answers, CONNECT_TIMEOUT).await?;
        // Where it can be browsed. A share with no FUSE path (a server's
        // list of shares, say) is still mounted; the window just has
        // nowhere to go.
        let mut info = self.gio_command();
        info.arg("info").arg(uri).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).kill_on_drop(true);
        let output = tokio::time::timeout(TIMEOUT, info.output()).await;
        Ok(match output {
            Ok(Ok(out)) => gvfs::local_path(&String::from_utf8_lossy(&out.stdout)),
            _ => None,
        })
    }

    async fn disconnect(&self, share: &Share) -> Result<(), String> {
        let mut command: Vec<String> = match &share.kind {
            ShareKind::Gvfs { .. } => {
                let mut c = self.gio.clone();
                c.extend(["mount".to_string(), "-u".to_string()]);
                c
            }
            // A FUSE mount is undone by its own helper, which lets a
            // user unmount what is theirs.
            ShareKind::Kernel { fstype } if fstype.starts_with("fuse") => vec!["fusermount3".into(), "-u".into()],
            // `umount` works for a user only on an fstab line marked
            // `user`; otherwise it says why, and that is passed on.
            ShareKind::Kernel { .. } => vec!["umount".into()],
        };
        command.push(share.path.to_string_lossy().into_owned());
        let output = blocking(TIMEOUT + Duration::from_secs(1), move || {
            let mut c = std::process::Command::new(&command[0]);
            c.args(&command[1..]).env("LC_ALL", "C.UTF-8").stdin(Stdio::null());
            hyprforge_process::output(&mut c, TIMEOUT)
        })
        .await;
        match output {
            None => Err(format!("Disconnecting didn't finish within {}s.", TIMEOUT.as_secs())),
            Some(Err(e)) if e.kind() == std::io::ErrorKind::TimedOut => {
                Err(format!("Disconnecting didn't finish within {}s.", TIMEOUT.as_secs()))
            }
            Some(Err(e)) => Err(format!("Couldn't start the program that disconnects it: {e}")),
            Some(Ok(out)) if out.status.success() => Ok(()),
            Some(Ok(out)) => Err(gvfs::failure(&String::from_utf8_lossy(&out.stderr))),
        }
    }

    async fn watch(&self) -> mpsc::Receiver<()> {
        let (tx, rx) = mpsc::channel(1);
        watch_mount_table(tx.clone());
        tokio::spawn(watch_gvfs(tx));
        rx
    }
}

impl SystemShares {
    /// One `gio mount` conversation — see [`gvfs::Conversation`].
    async fn mount(&self, uri: &str, answers: Answers, limit: Duration) -> Result<(), ConnectError> {
        let mut command = self.gio_command();
        command
            .arg("mount")
            .arg(uri)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            // Dropping the future is how a person's Cancel arrives; the
            // `gio` it was running goes with it.
            .kill_on_drop(true);
        let mut child = command.spawn().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                ConnectError::Absent("Connecting to servers needs the gio command from GLib, and it isn't installed.".to_string())
            } else {
                ConnectError::Failed(format!("Couldn't start gio: {e}"))
            }
        })?;
        let (Some(mut stdin), Some(mut stdout), Some(mut stderr)) = (child.stdin.take(), child.stdout.take(), child.stderr.take())
        else {
            return Err(ConnectError::Failed("gio started without its pipes".to_string()));
        };
        let errors = tokio::spawn(async move {
            let mut text = Vec::new();
            let _ = stderr.read_to_end(&mut text).await;
            String::from_utf8_lossy(&text).into_owned()
        });
        let deadline = tokio::time::Instant::now() + limit;
        let mut talk = Conversation::new(uri, answers);
        // What gio printed since the last answer — the prompt being
        // waited at is always at its end.
        let mut since: Vec<u8> = Vec::new();
        let mut buf = [0u8; 1024];
        loop {
            let read = match tokio::time::timeout_at(deadline, stdout.read(&mut buf)).await {
                Err(_) => {
                    let _ = child.kill().await;
                    return Err(ConnectError::TimedOut(limit));
                }
                Ok(Err(e)) => return Err(ConnectError::Failed(format!("Couldn't read what gio said: {e}"))),
                Ok(Ok(n)) => n,
            };
            if read == 0 {
                break;
            }
            since.extend_from_slice(&buf[..read]);
            let Some(asked) = gvfs::asked(&String::from_utf8_lossy(&since)) else { continue };
            match talk.reply(asked) {
                Reply::Write(line) => {
                    // The only place a typed password leaves this crate.
                    if stdin.write_all(line.expose().as_bytes()).await.is_err() || stdin.flush().await.is_err() {
                        break;
                    }
                    since.clear();
                }
                Reply::Stop(error) => {
                    let _ = child.kill().await;
                    return Err(error);
                }
            }
        }
        let status = match tokio::time::timeout_at(deadline, child.wait()).await {
            Err(_) => {
                let _ = child.kill().await;
                return Err(ConnectError::TimedOut(limit));
            }
            Ok(Err(e)) => return Err(ConnectError::Failed(format!("Couldn't wait for gio: {e}"))),
            Ok(Ok(status)) => status,
        };
        if status.success() {
            return Ok(());
        }
        let stderr = tokio::time::timeout(Duration::from_secs(1), errors).await.ok().and_then(Result::ok).unwrap_or_default();
        let said = if stderr.trim().is_empty() { String::from_utf8_lossy(&since).into_owned() } else { stderr };
        Err(ConnectError::Failed(gvfs::failure(&said)))
    }
}

/// Fires `tx` whenever the mount table changes.
///
/// `/proc/self/mountinfo` reports a change to whoever polls it, as
/// `POLLPRI` (proc(5)) — so this waits on exactly that, through tokio's
/// `AsyncFd`, rather than re-reading the table on a timer. A mount by
/// `mount -t cifs`, an automounter or another program arrives this way,
/// with nothing to ask anyone.
fn watch_mount_table(tx: mpsc::Sender<()>) {
    let Ok(file) = std::fs::File::open("/proc/self/mountinfo") else { return };
    let Ok(fd) = tokio::io::unix::AsyncFd::with_interest(file, tokio::io::Interest::PRIORITY) else {
        tracing::warn!("couldn't watch the mount table; network shares mounted elsewhere will appear on the next change");
        return;
    };
    tokio::spawn(async move {
        loop {
            let Ok(mut ready) = fd.ready(tokio::io::Interest::PRIORITY).await else { break };
            ready.clear_ready();
            if tx.is_closed() {
                break;
            }
            let _ = tx.try_send(());
        }
    });
}

/// Fires `tx` whenever gvfs mounts or unmounts something, by its mount
/// tracker's own signals on the session bus.
async fn watch_gvfs(tx: mpsc::Sender<()>) {
    let Ok(Ok(connection)) = tokio::time::timeout(TIMEOUT, zbus::Connection::session()).await else {
        tracing::warn!("no session bus to hear gvfs on; shares will refresh after this window's own changes");
        return;
    };
    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .interface("org.gtk.vfs.MountTracker")
        .map(|b| b.build());
    let Ok(rule) = rule else { return };
    let Ok(Ok(mut stream)) = tokio::time::timeout(TIMEOUT, zbus::MessageStream::for_match_rule(rule, &connection, Some(8))).await
    else {
        return;
    };
    use futures_util::StreamExt;
    while stream.next().await.is_some() {
        if tx.is_closed() {
            break;
        }
        let _ = tx.try_send(());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Login;
    use hyprforge_secret::Secret;

    /// A stand-in `gio` that asks for a user and password the way the
    /// real one does, and mounts only for `bob` / `right`. Run through
    /// `sh`, so the file is never exec'd — see [`SystemShares::gio`].
    const FAKE_GIO: &str = r#"
if [ "$1" = info ]; then echo "local path: /run/user/1000/gvfs/fake:"; exit 0; fi
printf 'Password required for share music on nas\nUser [apost]: '
read user
printf 'Domain [WORKGROUP]: '
read domain
printf 'Password: '
read pass
if [ "$user" = bob ] && [ "$pass" = right ]; then exit 0; fi
printf 'Password required for share music on nas\nUser [apost]: '
read again
echo "gio: $2: Login refused" >&2
exit 2
"#;

    fn fake(script: &str) -> (tempfile::TempDir, SystemShares) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gio.sh");
        std::fs::write(&path, script).unwrap();
        let shares = SystemShares::with_gio(
            vec!["sh".into(), path.to_string_lossy().into_owned()],
            dir.path().join("gvfs"),
            vec![],
        );
        (dir, shares)
    }

    #[tokio::test]
    async fn a_server_that_wants_a_password_is_asked_about_not_answered_blank() {
        let (_dir, shares) = fake(FAKE_GIO);
        match shares.connect("smb://nas/music", Answers::default()).await {
            Err(ConnectError::NeedsLogin(Login { message, user, retry: false, .. })) => {
                assert_eq!(message, "Password required for share music on nas");
                assert_eq!(user.as_deref(), Some("apost"));
            }
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn the_right_answers_mount_it_and_say_where() {
        let (_dir, shares) = fake(FAKE_GIO);
        let answers = Answers {
            user: Some("bob".into()),
            password: Some(Secret::new("right".to_string())),
            ..Answers::default()
        };
        let mounted = shares.connect("smb://nas/music", answers).await;
        assert_eq!(mounted, Ok(Some(PathBuf::from("/run/user/1000/gvfs/fake:"))));
    }

    /// The fake asks a second time after a wrong password, as gvfs does;
    /// the conversation stops there rather than answering again.
    #[tokio::test]
    async fn a_wrong_password_comes_back_as_a_retry_not_a_loop() {
        let (_dir, shares) = fake(FAKE_GIO);
        let answers = Answers {
            user: Some("bob".into()),
            password: Some(Secret::new("wrong".to_string())),
            ..Answers::default()
        };
        assert!(matches!(
            shares.connect("smb://nas/music", answers).await,
            Err(ConnectError::NeedsLogin(Login { retry: true, .. }))
        ));
    }

    #[tokio::test]
    async fn gios_own_error_is_passed_on_in_its_words() {
        let (_dir, shares) = fake("echo 'gio: smb://h/x/: Failed to mount Windows share: Connection refused' >&2; exit 2");
        assert_eq!(
            shares.connect("smb://h/x", Answers::default()).await,
            Err(ConnectError::Failed("Failed to mount Windows share: Connection refused".into()))
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_server_that_never_answers_times_out() {
        let (_dir, shares) = fake("sleep 600");
        let attempt = shares.mount("smb://h/x", Answers::default(), Duration::from_secs(3));
        assert_eq!(attempt.await, Err(ConnectError::TimedOut(Duration::from_secs(3))));
    }

    #[tokio::test]
    async fn no_gio_at_all_is_said_so() {
        let shares = SystemShares::with_gio(vec!["/nonexistent/gio".into()], PathBuf::from("/nonexistent"), vec![]);
        assert!(matches!(shares.connect("smb://h/x", Answers::default()).await, Err(ConnectError::Absent(_))));
        assert!(matches!(shares.gvfs().await, Gvfs::Absent(_)));
    }

    #[tokio::test]
    async fn gvfs_shares_are_read_from_the_fuse_directory() {
        let (dir, shares) = fake("exit 0");
        std::fs::create_dir_all(dir.path().join("gvfs/sftp:host=box,user=u")).unwrap();
        let found = shares.shares().await;
        assert!(found.iter().any(|s| s.label == "u@box" && s.path.ends_with("sftp:host=box,user=u")), "{found:?}");
    }
}
