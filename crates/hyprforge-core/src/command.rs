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
//! is about where files live, and deliberately not its own crate for
//! one function. Every caller already depends on this crate; if a
//! non-Hyprland app ever needs it, that is the moment to move it down a
//! layer.

use std::io;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

/// Long enough that a busy machine is not cut off, short enough that a
/// person does not conclude the app is broken.
pub const TIMEOUT: Duration = Duration::from_secs(5);

/// How often the child is checked for having exited.
const POLL: Duration = Duration::from_millis(5);

/// Like [`Command::output`], but gives up.
///
/// A timeout is reported as [`io::ErrorKind::TimedOut`], so callers that
/// already handle a failed spawn need no new branch: not answering and
/// not starting are the same problem from their side.
///
/// The child is killed on timeout rather than left running, because the
/// usual reason to time out is that it is stuck, and a pile of stuck
/// `hyprctl`s helps nobody.
pub fn output(command: &mut Command, timeout: Duration) -> io::Result<Output> {
    let mut child = command.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;

    // Drained on their own threads rather than after the child exits.
    // A pipe holds about 64KB before it blocks, and `fc-list` on a
    // machine with a lot of fonts prints more than that — so reading
    // only after exit would deadlock: the child waiting for the pipe to
    // drain, this waiting for the child.
    let mut stdout = child.stdout.take().map(reader);
    let mut stderr = child.stderr.take().map(reader);

    let started = Instant::now();
    let status = loop {
        match child.try_wait()? {
            Some(status) => break status,
            None if started.elapsed() >= timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!("{:?} did not answer within {:?}", command.get_program(), timeout),
                ));
            }
            None => std::thread::sleep(POLL),
        }
    };

    Ok(Output {
        status,
        stdout: stdout.take().map(collect).unwrap_or_default(),
        stderr: stderr.take().map(collect).unwrap_or_default(),
    })
}

fn reader<R: io::Read + Send + 'static>(mut source: R) -> std::thread::JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut buffer = Vec::new();
        let _ = source.read_to_end(&mut buffer);
        buffer
    })
}

/// A reader thread that panicked would only mean no output, and a
/// panicking settings app is worse than a missing string.
fn collect(handle: std::thread::JoinHandle<Vec<u8>>) -> Vec<u8> {
    handle.join().unwrap_or_default()
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
}
