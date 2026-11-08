//! Running a subprocess that might not come back.
//!
//! `Command::output()` waits forever. Every external program this
//! project talks to — `hyprctl`, `gsettings`, `fc-list` — is one that
//! can stop answering: a wedged compositor, a hung dconf, an NFS home
//! directory. Waiting forever on any of them means an app that has to
//! be killed, and for a login screen it would mean nobody can log in.
//!
//! That is the same shape of mistake as waiting forever for a
//! compositor to grant a session lock, which cost five visible seconds
//! before anyone noticed. Unbounded waits on another process are worth
//! treating as a category.
//!
//! Deliberately not in `hyprforge-paths`, which has no dependencies and
//! is about where files live: this is about talking to another process,
//! not about the filesystem. It used to live in `hyprforge-core`, on the
//! theory that every caller already depended on that crate — but
//! `hyprforge-network`, `hyprforge-bluetooth` and `hyprforge-power` are
//! clients of other daemons (NetworkManager, BlueZ, systemd-logind) with
//! no Hyprland config to read or write, and pulling in 4,000+ lines of
//! `hlconfig`/`hyprlang`/`lua` machinery for a bounded-subprocess helper
//! made the layering a lie. This crate has no workspace dependencies at
//! all, same as `hyprforge-paths`, so a non-Hyprland app can use it
//! without dragging in Hyprland at all. `hyprforge-core` re-exports this
//! module as `hyprforge_core::command` so existing call sites keep
//! working; new code should depend on `hyprforge-process` directly.

use std::io;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

/// Long enough that a busy machine is not cut off, short enough that a
/// person does not conclude the app is broken.
pub const TIMEOUT: Duration = Duration::from_secs(5);

/// How often the child is checked for having exited.
const POLL: Duration = Duration::from_millis(5);

/// The most output kept from either stream. Far beyond anything a
/// command here prints (`fc-list` on a font-heavy machine is a few
/// hundred KB), and small enough that a command stuck in a loop writing
/// cannot make the caller allocate without limit before the timeout
/// lands. Past it the rest is read and thrown away, so the child never
/// blocks on a full pipe.
const MAX_OUTPUT: usize = 64 * 1024 * 1024;

/// Like [`Command::output`], but gives up.
///
/// A timeout is reported as [`io::ErrorKind::TimedOut`], so callers that
/// already handle a failed spawn need no new branch: not answering and
/// not starting are the same problem from their side.
///
/// The child is killed on timeout rather than left running, because the
/// usual reason to time out is that it is stuck, and a pile of stuck
/// `hyprctl`s helps nobody. It is killed with everything it started:
/// the command runs in a process group of its own, and the whole group
/// goes. Killing only the direct child left a shell wrapper's `ffmpeg`
/// running after its `sh` was gone.
///
/// The bound covers the *output* too, not only the exit. A grandchild
/// that outlives the child — `sh -c 'daemon & echo started'` — still
/// holds stdout open, so waiting for end-of-file after the child exited
/// waited for the grandchild: past the timeout, possibly forever. The
/// readers are waited on only until the deadline; past it, what they have
/// read so far is the output, and the grandchild is left alone, because
/// the child said it was finished and a backgrounded process is usually
/// meant to keep running.
///
/// stdin is `/dev/null`, always. An inherited stdin let a child wait on a
/// terminal nobody was typing into — and in a process group of its own,
/// a read from that terminal stops it with SIGTTIN. Nothing here can
/// feed a child input anyway: the caller never sees its handle.
pub fn output(command: &mut Command, timeout: Duration) -> io::Result<Output> {
    use std::os::unix::process::CommandExt as _;

    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn()?;
    let started = Instant::now();
    let deadline = started + timeout;

    // Drained on their own threads rather than after the child exits.
    // A pipe holds about 64KB before it blocks, and `fc-list` on a
    // machine with a lot of fonts prints more than that — so reading
    // only after exit would deadlock: the child waiting for the pipe to
    // drain, this waiting for the child. Each thread sends what it reads
    // as it reads it, then `None` when its pipe closes — so a thread
    // whose pipe never closes can be abandoned rather than joined, and
    // what it had already read is not lost with it.
    let (sender, results) = std::sync::mpsc::channel();
    let mut pending = 0;
    if let Some(out) = child.stdout.take() {
        reader(out, Stream::Stdout, sender.clone());
        pending += 1;
    }
    if let Some(err) = child.stderr.take() {
        reader(err, Stream::Stderr, sender.clone());
        pending += 1;
    }
    drop(sender);

    let status = loop {
        match child.try_wait()? {
            Some(status) => break status,
            None if Instant::now() >= deadline => {
                kill_group(&mut child);
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!("{:?} did not answer within {:?}", command.get_program(), timeout),
                ));
            }
            None => std::thread::sleep(POLL),
        }
    };

    let mut output = Output { status, stdout: Vec::new(), stderr: Vec::new() };
    while pending > 0 {
        let left = deadline.saturating_duration_since(Instant::now());
        let (stream, chunk) = match results.recv_timeout(left) {
            Ok(message) => message,
            // Past the deadline, or every reader gone (one that panicked
            // only means less output, and a panicking settings app is
            // worse than a missing string).
            Err(_) => break,
        };
        let Some(chunk) = chunk else {
            pending -= 1;
            continue;
        };
        let kept = match stream {
            Stream::Stdout => &mut output.stdout,
            Stream::Stderr => &mut output.stderr,
        };
        // Past the cap the rest is still read — the child must never
        // block on a full pipe — and thrown away.
        let room = MAX_OUTPUT.saturating_sub(kept.len());
        kept.extend_from_slice(&chunk[..chunk.len().min(room)]);
    }
    Ok(output)
}

#[derive(Clone, Copy)]
enum Stream {
    Stdout,
    Stderr,
}

fn reader<R: io::Read + Send + 'static>(
    mut source: R,
    stream: Stream,
    chunks: std::sync::mpsc::Sender<(Stream, Option<Vec<u8>>)>,
) {
    std::thread::spawn(move || {
        let mut buffer = vec![0; 64 * 1024];
        loop {
            match source.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => {
                    // The receiver gone means `output` already returned;
                    // keep draining so the writer is not stopped by a
                    // full pipe, just stop sending.
                    let _ = chunks.send((stream, Some(buffer[..n].to_vec())));
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(_) => break,
            }
        }
        let _ = chunks.send((stream, None));
    });
}

/// Kills the child's whole process group, then the child itself.
///
/// std has no `killpg` and this crate has no `libc`, so the group goes by
/// way of `kill(1)` with a negative pid — the documented way to name a
/// group from the command line. Best effort, and bounded like everything
/// else here: if `kill` is missing or itself hangs, the direct child is
/// still killed and reaped below, which is the old behaviour.
fn kill_group(child: &mut std::process::Child) {
    let group = format!("-{}", child.id());
    if let Ok(mut kill) = Command::new("kill")
        .args(["-KILL", "--", &group])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        let until = Instant::now() + Duration::from_secs(1);
        while matches!(kill.try_wait(), Ok(None)) && Instant::now() < until {
            std::thread::sleep(POLL);
        }
        let _ = kill.kill();
        let _ = kill.wait();
    }
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_ordinary_command_still_returns_its_output() {
        let out = output(Command::new("echo").arg("hello"), TIMEOUT).unwrap();
        assert!(out.status.success());
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "hello");
    }

    #[test]
    fn a_failing_command_is_reported_by_its_status_not_an_error() {
        let out = output(&mut Command::new("false"), TIMEOUT).unwrap();
        assert!(!out.status.success(), "a non-zero exit is data, not an error");
    }

    /// The whole point. Before this existed, a command that never
    /// returned took the calling app with it.
    #[test]
    fn a_command_that_never_returns_gives_up() {
        let started = Instant::now();
        let result = output(Command::new("sleep").arg("60"), Duration::from_millis(200));
        let error = result.expect_err("sleep 60 must not be waited on");
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(5), "waited {:?}", started.elapsed());
    }

    /// More output than a pipe holds, so reading it only after the child
    /// exited would deadlock — the child blocked on a full pipe, this
    /// blocked on the child.
    #[test]
    fn output_larger_than_a_pipe_buffer_does_not_deadlock() {
        let out = output(
            Command::new("sh").arg("-c").arg("yes hyprforge | head -c 400000"),
            Duration::from_secs(10),
        )
        .expect("a chatty command is not a hung one");
        assert_eq!(out.stdout.len(), 400_000);
    }

    #[test]
    fn a_program_that_does_not_exist_is_an_error_like_before() {
        let error = output(&mut Command::new("hyprforge-no-such-program"), TIMEOUT).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
    }

    /// A grandchild that keeps stdout open used to keep `output` waiting
    /// for end-of-file long after the child itself had exited.
    #[test]
    fn a_backgrounded_grandchild_does_not_hold_output_past_its_timeout() {
        let started = Instant::now();
        let out = output(
            Command::new("sh").arg("-c").arg("sleep 30 & echo hi"),
            Duration::from_millis(500),
        )
        .expect("the child itself exited, so this is an answer");
        assert!(started.elapsed() < Duration::from_secs(3), "waited {:?}", started.elapsed());
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "hi");
    }

    /// On timeout the whole group goes, not only the shell in front of it.
    #[test]
    fn a_timeout_kills_what_the_child_started_too() {
        let dir = std::env::temp_dir().join(format!("hyprforge-process-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let pidfile = dir.join("grandchild.pid");
        let script = format!("sleep 60 & echo $! > {}; wait", pidfile.display());
        let started = Instant::now();
        let error = output(Command::new("sh").arg("-c").arg(script), Duration::from_millis(300)).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(3), "waited {:?}", started.elapsed());

        let pid = std::fs::read_to_string(&pidfile).unwrap().trim().to_string();
        let _ = std::fs::remove_dir_all(&dir);
        // A killed process may linger briefly as a zombie of init's; it
        // is gone, or dead, once its state is no longer running/sleeping.
        let alive = || {
            std::fs::read_to_string(format!("/proc/{pid}/stat"))
                .map(|stat| !stat.rsplit(") ").next().unwrap_or("").starts_with('Z'))
                .unwrap_or(false)
        };
        let until = Instant::now() + Duration::from_secs(2);
        while alive() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(!alive(), "the grandchild sleep survived the timeout");
    }

    #[test]
    fn a_child_reads_end_of_file_not_the_callers_terminal() {
        let out = output(Command::new("sh").arg("-c").arg("cat; echo done"), Duration::from_secs(2))
            .expect("cat on /dev/null finishes at once");
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "done");
    }
}
