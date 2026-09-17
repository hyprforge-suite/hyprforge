//! Copying and moving files without freezing the window.
//!
//! A paste becomes a *job*: a worker thread drives one
//! `hyprforge_fileops::ops::Operation` per pasted item, a bounded step at
//! a time, and reports back through a stream the window turns into
//! messages with `Task::run`. The window keeps painting, and can cancel
//! between any two steps — mid-file included.
//!
//! A conflict — something already at a destination — pauses the job.
//! The worker sends [`JobEvent::Collision`] and blocks until the window
//! answers through [`JobControl::answer`]. That wait has no timeout, and
//! it is the one wait in this suite that should not: it is waiting on a
//! *person*, not on another process. If the window goes away instead,
//! the answer channel closes and the job cancels, so the thread cannot
//! outlive the process's interest in it.
//!
//! What is decided without asking, in order:
//!
//! 1. A copy into its own folder collides with itself, and is always
//!    kept as a numbered duplicate — see
//!    `hyprforge_files_core::clipboard::PasteStep::duplicate`.
//! 2. An "apply to the rest" answer, or `[behaviour] on-conflict` when it
//!    is anything but `ask`, answers every later conflict in the job —
//!    across items, not only within one, which is what "the rest" means
//!    to the person who ticked it.

use hyprforge_files_core::clipboard::PasteStep;
use hyprforge_files_core::config::OnConflict;
use hyprforge_fileops::{Collision, CollisionDecision, CollisionPolicy, Operation, Progress, StepOutcome};
use iced::futures::channel::mpsc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub type JobId = u64;

/// What a job reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobEvent {
    Progress { job: JobId, progress: Progress },
    /// Paused: something is in the way. Answer with [`JobControl::answer`].
    Collision { job: JobId, collision: Collision },
    Finished { job: JobId, summary: JobSummary },
}

/// How a job went, item by item.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct JobSummary {
    /// Items placed.
    pub done: usize,
    /// Items left alone because of a Skip answer.
    pub skipped: usize,
    /// One sentence per thing that went wrong — already the actionable
    /// wording `OpsError`'s `Display` produces.
    pub failed: Vec<String>,
    pub cancelled: bool,
}

impl JobSummary {
    /// Whether every item arrived, so a cut's clipboard can be emptied.
    pub fn complete(&self) -> bool {
        self.failed.is_empty() && !self.cancelled && self.skipped == 0
    }
}

/// The window's handle on a running job.
#[derive(Debug, Clone)]
pub struct JobControl {
    decisions: std::sync::mpsc::Sender<CollisionDecision>,
    cancel: Arc<AtomicBool>,
}

impl JobControl {
    /// Answers the collision the job is paused on.
    pub fn answer(&self, decision: CollisionDecision) {
        // A job that already finished has dropped its receiver; an answer
        // to nothing is nothing to report.
        let _ = self.decisions.send(decision);
    }

    /// Stops the job at its next step. A paused job is answered with
    /// Cancel so it does not sit waiting for a decision nobody will make.
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
        self.answer(CollisionDecision { policy: CollisionPolicy::Cancel, apply_to_rest: true });
    }
}

/// How often progress is reported, at most. A copy of small files makes
/// thousands of steps a second, and a message per step would rebuild the
/// window thousands of times a second to move a number nobody can read
/// that fast.
const PROGRESS_EVERY: Duration = Duration::from_millis(100);

/// Starts `steps` on a worker thread.
pub fn start(
    job: JobId,
    steps: Vec<PasteStep>,
    on_conflict: OnConflict,
) -> (JobControl, mpsc::UnboundedReceiver<JobEvent>) {
    let (events, receiver) = mpsc::unbounded();
    let (decisions, answers) = std::sync::mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let control = JobControl { decisions, cancel: cancel.clone() };
    std::thread::Builder::new()
        .name(format!("files-job-{job}"))
        .spawn(move || {
            let summary = run(job, steps, on_conflict, &answers, &cancel, &events);
            let _ = events.unbounded_send(JobEvent::Finished { job, summary });
        })
        .expect("spawning a thread only fails when the process is out of resources");
    (control, receiver)
}

fn run(
    job: JobId,
    steps: Vec<PasteStep>,
    on_conflict: OnConflict,
    answers: &std::sync::mpsc::Receiver<CollisionDecision>,
    cancel: &AtomicBool,
    events: &mpsc::UnboundedSender<JobEvent>,
) -> JobSummary {
    let mut summary = JobSummary::default();
    let mut for_the_rest: Option<CollisionPolicy> = match on_conflict {
        OnConflict::Ask => None,
        OnConflict::KeepBoth => Some(CollisionPolicy::KeepBoth),
        OnConflict::Skip => Some(CollisionPolicy::Skip),
        OnConflict::Replace => Some(CollisionPolicy::Replace),
    };
    let mut last_progress: Option<Instant> = None;

    'items: for step in steps {
        if cancel.load(Ordering::Relaxed) {
            summary.cancelled = true;
            break;
        }
        let mut op = Operation::real(step.kind, &step.source, &step.dest);
        loop {
            if cancel.load(Ordering::Relaxed) {
                op.request_cancel();
            }
            match op.step() {
                StepOutcome::Progress(progress) => {
                    if last_progress.is_none_or(|at| at.elapsed() >= PROGRESS_EVERY) {
                        last_progress = Some(Instant::now());
                        let _ = events.unbounded_send(JobEvent::Progress { job, progress });
                    }
                }
                StepOutcome::Collision(collision) => {
                    let policy = if step.duplicate && collision.dest == step.dest {
                        CollisionPolicy::KeepBoth
                    } else if let Some(policy) = for_the_rest {
                        policy
                    } else {
                        let _ = events.unbounded_send(JobEvent::Collision { job, collision });
                        match answers.recv() {
                            Ok(decision) => {
                                if decision.apply_to_rest {
                                    for_the_rest = Some(decision.policy);
                                }
                                decision.policy
                            }
                            // The window is gone; nobody will answer.
                            Err(_) => CollisionPolicy::Cancel,
                        }
                    };
                    op.resolve(CollisionDecision { policy, apply_to_rest: false });
                }
                StepOutcome::Done(report) => {
                    summary.failed.extend(report.failed.into_iter().map(|(_, message)| message));
                    if let Some(message) = report.source_removal_failed {
                        summary.failed.push(message);
                    }
                    if report.cancelled {
                        summary.cancelled = true;
                        break 'items;
                    }
                    if report.succeeded.is_empty() && !report.skipped.is_empty() {
                        summary.skipped += 1;
                    } else if !report.succeeded.is_empty() {
                        summary.done += 1;
                    }
                    continue 'items;
                }
            }
        }
    }
    summary
}

/// Renames `from` to `to` in one go — a rename is a single `rename(2)`
/// in the same folder, too quick to be worth a job.
///
/// Through `Operation` rather than `std::fs::rename`, because
/// `std::fs::rename` replaces whatever is at `to` without a word. The
/// name was checked against the listing when Enter was pressed, but
/// something can appear in between; here that is a collision, answered
/// Cancel, and reported — never an overwrite.
pub fn rename(from: &std::path::Path, to: &std::path::Path) -> Result<(), String> {
    let mut op = Operation::real(hyprforge_fileops::OpKind::Move, from, to);
    loop {
        match op.step() {
            StepOutcome::Progress(_) => {}
            StepOutcome::Collision(_) => {
                op.resolve(CollisionDecision { policy: CollisionPolicy::Cancel, apply_to_rest: true });
            }
            StepOutcome::Done(report) => {
                if let Some((_, message)) = report.failed.into_iter().next() {
                    return Err(message);
                }
                if report.cancelled {
                    let name = to.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    return Err(format!("Something called \"{name}\" appeared there first."));
                }
                return Ok(());
            }
        }
    }
}

/// Makes a new, empty folder. `create_dir` and not `create_dir_all`, so
/// a folder that appeared under the same name in the meantime is an
/// error rather than silently "created" again.
pub fn create_folder(path: &std::path::Path) -> Result<(), String> {
    std::fs::create_dir(path).map_err(|e| {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        match e.kind() {
            std::io::ErrorKind::AlreadyExists => format!("\"{name}\" already exists."),
            std::io::ErrorKind::PermissionDenied => {
                "You don't have permission to make a folder here.".to_string()
            }
            _ => format!("Couldn't make \"{name}\": {e}"),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use hyprforge_fileops::OpKind;
    use iced::futures::channel::mpsc::TryRecvError;
    use std::fs;
    use std::path::Path;

    /// Collects a job's events, answering each collision with `answer`.
    /// Bounded: a job that never finishes fails the test instead of
    /// hanging it.
    fn drive(
        control: &JobControl,
        mut events: mpsc::UnboundedReceiver<JobEvent>,
        mut answer: impl FnMut(&Collision) -> CollisionDecision,
    ) -> (JobSummary, usize) {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut collisions = 0;
        while Instant::now() < deadline {
            match events.try_recv() {
                Ok(JobEvent::Finished { summary, .. }) => return (summary, collisions),
                Ok(JobEvent::Collision { collision, .. }) => {
                    collisions += 1;
                    control.answer(answer(&collision));
                }
                Ok(JobEvent::Progress { .. }) => {}
                Err(TryRecvError::Closed) => panic!("the job ended without finishing"),
                Err(TryRecvError::Empty) => std::thread::sleep(Duration::from_millis(5)),
            }
        }
        panic!("the job did not finish in time");
    }

    fn never(_: &Collision) -> CollisionDecision {
        panic!("no collision was expected");
    }

    fn step(source: &Path, dest: &Path, kind: OpKind) -> PasteStep {
        PasteStep { source: source.into(), dest: dest.into(), kind, duplicate: false }
    }

    #[test]
    fn a_copy_lands_and_the_original_stays() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.txt");
        fs::write(&a, "hello").unwrap();
        let b = dir.path().join("b.txt");
        let (control, events) = start(1, vec![step(&a, &b, OpKind::Copy)], OnConflict::Ask);
        let (summary, _) = drive(&control, events, never);
        assert_eq!(summary, JobSummary { done: 1, ..JobSummary::default() });
        assert!(summary.complete());
        assert_eq!(fs::read_to_string(&b).unwrap(), "hello");
        assert!(a.exists());
    }

    #[test]
    fn a_move_takes_a_whole_folder() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        fs::create_dir_all(src.join("deep")).unwrap();
        fs::write(src.join("deep").join("f.txt"), "x").unwrap();
        let dest = dir.path().join("elsewhere");
        let (control, events) = start(1, vec![step(&src, &dest, OpKind::Move)], OnConflict::Ask);
        let (summary, _) = drive(&control, events, never);
        assert!(summary.complete(), "{summary:?}");
        assert!(dest.join("deep").join("f.txt").exists());
        assert!(!src.exists());
    }

    /// A copy into its own folder makes a second copy without asking —
    /// "replace this file with itself?" would be absurd.
    #[test]
    fn a_duplicate_is_kept_beside_the_original_without_asking() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.txt");
        fs::write(&a, "hello").unwrap();
        let dup = PasteStep { source: a.clone(), dest: a.clone(), kind: OpKind::Copy, duplicate: true };
        let (control, events) = start(1, vec![dup], OnConflict::Ask);
        let (summary, collisions) = drive(&control, events, never);
        assert_eq!(collisions, 0);
        assert!(summary.complete(), "{summary:?}");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2, "the original and one copy");
    }

    /// A conflict pauses the job and the answer is carried out.
    #[test]
    fn a_conflict_is_asked_about_and_the_answer_is_carried_out() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.txt");
        let b = dir.path().join("b.txt");
        fs::write(&a, "new").unwrap();
        fs::write(&b, "old").unwrap();
        let (control, events) = start(1, vec![step(&a, &b, OpKind::Copy)], OnConflict::Ask);
        let (summary, collisions) = drive(&control, events, |c| {
            assert_eq!(c.dest, b);
            CollisionDecision { policy: CollisionPolicy::Replace, apply_to_rest: false }
        });
        assert_eq!(collisions, 1);
        assert!(summary.complete(), "{summary:?}");
        assert_eq!(fs::read_to_string(&b).unwrap(), "new");
    }

    /// "Apply to the rest" reaches the *next item*, not only the rest of
    /// the item it was ticked on.
    #[test]
    fn apply_to_the_rest_answers_every_later_item_too() {
        let dir = tempfile::tempdir().unwrap();
        let from = dir.path().join("from");
        let to = dir.path().join("to");
        fs::create_dir_all(&from).unwrap();
        fs::create_dir_all(&to).unwrap();
        let mut steps = Vec::new();
        for name in ["a", "b", "c"] {
            fs::write(from.join(name), "new").unwrap();
            fs::write(to.join(name), "old").unwrap();
            steps.push(step(&from.join(name), &to.join(name), OpKind::Copy));
        }
        let (control, events) = start(1, steps, OnConflict::Ask);
        let (summary, collisions) = drive(&control, events, |_| CollisionDecision {
            policy: CollisionPolicy::Skip,
            apply_to_rest: true,
        });
        assert_eq!(collisions, 1, "asked once, for the first");
        assert_eq!(summary.skipped, 3);
        assert!(!summary.complete());
        assert_eq!(fs::read_to_string(to.join("c")).unwrap(), "old");
    }

    /// With a configured policy nobody is asked at all.
    #[test]
    fn a_configured_policy_never_asks() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.txt");
        let b = dir.path().join("b.txt");
        fs::write(&a, "new").unwrap();
        fs::write(&b, "old").unwrap();
        let (control, events) = start(1, vec![step(&a, &b, OpKind::Copy)], OnConflict::KeepBoth);
        let (summary, collisions) = drive(&control, events, never);
        assert_eq!(collisions, 0);
        assert!(summary.complete());
        assert_eq!(fs::read_to_string(&b).unwrap(), "old", "the existing file is untouched");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 3);
    }

    /// Cancelling a paused job answers its question, so the worker does
    /// not sit waiting forever, and nothing after it runs.
    #[test]
    fn cancelling_a_paused_job_ends_it() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.txt");
        let b = dir.path().join("b.txt");
        let c = dir.path().join("c.txt");
        fs::write(&a, "new").unwrap();
        fs::write(&b, "old").unwrap();
        let steps = vec![step(&a, &b, OpKind::Copy), step(&a, &c, OpKind::Copy)];
        let (control, events) = start(1, steps, OnConflict::Ask);
        let canceller = control.clone();
        let (summary, _) = drive(&control, events, move |_| {
            canceller.cancel();
            // `cancel` already answered; this second answer goes nowhere.
            CollisionDecision { policy: CollisionPolicy::Cancel, apply_to_rest: false }
        });
        assert!(summary.cancelled);
        assert!(!c.exists(), "the item after the cancel never ran");
        assert_eq!(fs::read_to_string(&b).unwrap(), "old");
    }

    /// If the window goes away mid-question, the job cancels rather
    /// than leaving a thread blocked for the life of the process.
    #[test]
    fn a_job_whose_window_went_away_cancels() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.txt");
        let b = dir.path().join("b.txt");
        fs::write(&a, "new").unwrap();
        fs::write(&b, "old").unwrap();
        let (control, mut events) = start(1, vec![step(&a, &b, OpKind::Copy)], OnConflict::Ask);
        drop(control);
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            assert!(Instant::now() < deadline, "the job did not end");
            match events.try_recv() {
                Ok(JobEvent::Finished { summary, .. }) => {
                    assert!(summary.cancelled);
                    break;
                }
                Ok(_) => {}
                Err(TryRecvError::Closed) => panic!("ended without finishing"),
                Err(TryRecvError::Empty) => std::thread::sleep(Duration::from_millis(5)),
            }
        }
        assert_eq!(fs::read_to_string(&b).unwrap(), "old");
    }

    #[test]
    fn a_rename_renames() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.txt");
        fs::write(&a, "x").unwrap();
        rename(&a, &dir.path().join("b.txt")).unwrap();
        assert!(dir.path().join("b.txt").exists());
        assert!(!a.exists());
    }

    /// The case `std::fs::rename` gets wrong: it would replace `b.txt`
    /// without a word.
    #[test]
    fn a_rename_onto_an_existing_name_is_refused_and_overwrites_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.txt");
        let b = dir.path().join("b.txt");
        fs::write(&a, "mine").unwrap();
        fs::write(&b, "theirs").unwrap();
        let err = rename(&a, &b).unwrap_err();
        assert!(err.contains("b.txt"), "{err}");
        assert_eq!(fs::read_to_string(&b).unwrap(), "theirs");
        assert!(a.exists());
    }

    #[test]
    fn a_new_folder_is_made_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("New folder");
        create_folder(&path).unwrap();
        assert!(path.is_dir());
        assert!(create_folder(&path).unwrap_err().contains("already exists"));
    }

    #[test]
    fn a_failure_is_reported_in_words_and_the_rest_still_runs() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("gone.txt");
        let a = dir.path().join("a.txt");
        fs::write(&a, "x").unwrap();
        let steps = vec![
            step(&missing, &dir.path().join("x.txt"), OpKind::Copy),
            step(&a, &dir.path().join("b.txt"), OpKind::Copy),
        ];
        let (control, events) = start(1, steps, OnConflict::Ask);
        let (summary, _) = drive(&control, events, never);
        assert_eq!(summary.done, 1);
        assert_eq!(summary.failed.len(), 1, "{summary:?}");
        assert!(!summary.complete());
    }
}
