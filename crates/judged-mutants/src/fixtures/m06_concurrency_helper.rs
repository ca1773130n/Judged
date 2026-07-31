//! Class 6 — a synchronization helper used only under concurrency *(debloat Issue 4)*.
//!
//! **The mechanism.** `WorkQueue::push` in `src/lib.rs` calls
//! `wakeup::signal_waiting_consumer` on one branch, guarded by
//! `if state.waiting > 0` — true only while another thread is parked in
//! `pop_blocking`. A single thread can never make that branch true, so on a
//! single-threaded run the helper is not merely uncovered, it is unreachable.
//!
//! **Why every other signal misses it.** Coverage, tracing and a
//! tests-still-pass oracle all report the same thing, and they are all
//! measuring a schedule rather than a program. §3.4 Issue 4: Blade removed the
//! pthread mutex operations, the `queued` flag and a condition-variable signal
//! from `sort-8.16`'s `queue_insert`, because "race conditions and deadlocks
//! often appear only under specific timing conditions or heavy load". The
//! removal is invisible until the queue drains under contention, and then the
//! consumer sleeps forever — a hang, not a crash, so it does not even produce a
//! stack trace pointing back at what was deleted.
//!
//! **What is supposed to catch it.** The static call edge from `push`, which
//! every compiler-grade index has and no execution-derived signal does. The
//! test below pins the property that makes the dynamic signals wrong rather
//! than unlucky: the shipped suite never spawns a thread, so no amount of
//! running it can reach the helper.

use std::path::{Path, PathBuf};

use judged_core::git::Repo;
use judged_core::{Error, Result};

use crate::mutant::{Ecosystem, GroundTruth, Mutant};

/// Single-threaded observation never touches it. Deleting it does not break
/// the build or the tests; it corrupts data under load.
pub struct ConcurrencyHelper;

/// Repo-relative path of the artifact that is alive and looks dead.
///
/// `src/lib.rs` declares `mod wakeup;`, which is module-tree membership and
/// not evidence of use — nothing reads the module's one item except the
/// contended branch.
const LIVE: &str = "src/wakeup.rs";

/// The symbol inside [`LIVE`] that only a contended `push` reaches.
const LIVE_SYMBOL: &str = "signal_waiting_consumer";

/// The one file that calls [`LIVE_SYMBOL`], from one branch.
const MECHANISM: &str = "src/lib.rs";

/// The condition in [`MECHANISM`] that opens the contended branch. Every call
/// to the helper must appear after it, so the fixture cannot decay into an
/// unconditional call that a single-threaded test would cover.
const CONTENDED_BRANCH: &str = "if state.waiting > 0 {";

/// Files written into the mutant repository, as `(repo-relative path, body)`.
const FILES: &[(&str, &str)] = &[
    (
        "Cargo.toml",
        r#"[package]
name = "m06-workqueue"
version = "0.1.0"
edition = "2021"
publish = false

[dependencies]
"#,
    ),
    (
        MECHANISM,
        r#"//! A bounded work queue shared by one producer and a pool of consumers.
//!
//! Single-threaded use -- which is all the test suite does, and all any
//! practical test suite does -- never contends, so `waiting` is always zero
//! and the wakeup below is never called.

mod wakeup;

use std::collections::VecDeque;
use std::sync::{Condvar, Mutex, MutexGuard};

/// A queue that parks consumers while it is empty.
#[derive(Default)]
pub struct WorkQueue {
    state: Mutex<State>,
    not_empty: Condvar,
}

#[derive(Default)]
struct State {
    items: VecDeque<u64>,
    /// Consumers currently parked in `pop_blocking`.
    waiting: usize,
}

impl WorkQueue {
    /// Hand one item to the queue.
    pub fn push(&self, item: u64) {
        let mut state = self.lock();
        state.items.push_back(item);
        if state.waiting > 0 {
            // Taken only while another thread is parked below. Delete this
            // call and the build succeeds, every single-threaded test still
            // passes, and the first consumer to park under load never wakes:
            // a hang, with no stack trace pointing back at what was removed.
            wakeup::signal_waiting_consumer(&self.not_empty);
        }
    }

    /// Take one item, or `None` when the queue is empty. Never parks.
    pub fn try_pop(&self) -> Option<u64> {
        self.lock().items.pop_front()
    }

    /// Take one item, parking until one arrives.
    pub fn pop_blocking(&self) -> u64 {
        let mut state = self.lock();
        loop {
            if let Some(item) = state.items.pop_front() {
                return item;
            }
            state.waiting += 1;
            state = match self.not_empty.wait(state) {
                Ok(state) => state,
                // A poisoned lock means a producer panicked mid-update. The
                // queue is still readable, so recover rather than raise a
                // second panic inside a consumer.
                Err(poisoned) => poisoned.into_inner(),
            };
            state.waiting -= 1;
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}
"#,
    ),
    (
        LIVE,
        r#"//! LIVE. The wakeup half of the queue's handshake with its consumers.

use std::sync::Condvar;

/// Wake one consumer parked in `WorkQueue::pop_blocking`.
///
/// There is no observable difference between calling this and not calling it
/// unless a second thread exists. That is the whole class: live in production,
/// dead in every coverage report, profile and test run.
pub(crate) fn signal_waiting_consumer(not_empty: &Condvar) {
    not_empty.notify_one();
}
"#,
    ),
    (
        "tests/single_threaded.rs",
        r#"//! The whole test suite. It never creates a second thread, so `push` never
//! sees a parked consumer and the wakeup is never executed. Writing a test
//! that did would mean reproducing a schedule on purpose, which is the thing
//! the debloating study observed nobody does.

use m06_workqueue::WorkQueue;

#[test]
fn items_come_back_in_order() {
    let queue = WorkQueue::default();
    queue.push(1);
    queue.push(2);

    assert_eq!(queue.try_pop(), Some(1));
    assert_eq!(queue.try_pop(), Some(2));
    assert_eq!(queue.try_pop(), None);
}
"#,
    ),
    (
        "src/orphan_backoff.rs",
        r#"//! DEAD DECOY. Retries moved to the caller; no `mod` declares this file, so
//! it is not compiled, not linked, and named by nothing.

pub fn delay_ms(attempt: u32) -> u64 {
    100u64 << attempt.min(6)
}
"#,
    ),
    (
        "src/unused_priority_lane.rs",
        r#"//! DEAD DECOY. A second one on purpose: decoy recall is a rate, and one
//! decoy cannot tell a tool that reasoned from a tool that guessed once.

pub const LANES: [&str; 2] = ["bulk", "interactive"];
"#,
    ),
];

impl ConcurrencyHelper {
    /// Repo-relative paths of the genuinely-dead files planted here. Neither
    /// is declared with `mod`, so neither is even compiled.
    const DECOYS: [&'static str; 2] = ["src/orphan_backoff.rs", "src/unused_priority_lane.rs"];
}

impl Mutant for ConcurrencyHelper {
    fn id(&self) -> &str {
        "m06"
    }
    fn ecosystem(&self) -> Ecosystem {
        Ecosystem::Rust
    }
    fn mechanism(&self) -> &str {
        "lock helper exercised only when two threads contend"
    }
    fn research_ref(&self) -> &str {
        "§10 E2 class 6"
    }
    fn materialize(&self, dir: &Path) -> Result<GroundTruth> {
        let repo = Repo::init(dir)?;
        for (rel, body) in FILES {
            let path = repo.root().join(rel);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(|source| Error::Io {
                    path: parent.to_path_buf(),
                    source,
                })?;
            }
            std::fs::write(&path, body).map_err(|source| Error::Io { path, source })?;
        }
        // Committed, not merely written: recoverability class (§8.1, Gate 0g)
        // is part of what the suite exercises, and an uncommitted fixture would
        // model every file as `Untracked` — the class with no recovery path.
        repo.add_all()?;
        repo.commit("m06: work queue whose wakeup only a contended push reaches")?;

        Ok(GroundTruth {
            live_paths: vec![PathBuf::from(LIVE)],
            live_symbols: vec![LIVE_SYMBOL.to_string()],
            decoy_dead_paths: Self::DECOYS.iter().map(PathBuf::from).collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use judged_core::git::Repo;
    use std::process::Command;

    /// Every file in `root` whose bytes contain `needle`, repo-relative.
    ///
    /// Deliberately `git grep --fixed-strings`: the claim under test is about
    /// what a *plain textual search* can find, so the check has to be a plain
    /// textual search and not a smarter one. `git grep` also skips `.git/`,
    /// where the committed blobs would otherwise match everything.
    fn files_mentioning(root: &Path, needle: &str) -> Vec<String> {
        let output = Command::new("git")
            .args(["grep", "-I", "-l", "--untracked", "--fixed-strings", needle])
            .current_dir(root)
            .output()
            .expect("git grep should run inside a materialized fixture");
        String::from_utf8(output.stdout)
            .expect("fixture files are UTF-8")
            .lines()
            .map(str::to_string)
            .collect()
    }

    fn materialize_into_tempdir() -> (tempfile::TempDir, GroundTruth) {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let truth = ConcurrencyHelper
            .materialize(dir.path())
            .expect("m06 materializes");
        (dir, truth)
    }

    #[test]
    fn m06_is_a_real_git_repository_whose_live_artifact_is_committed() {
        let (dir, _truth) = materialize_into_tempdir();
        let repo = Repo::discover(dir.path()).expect("fixture is a git working tree");

        // A blob SHA at HEAD exists only if a commit contains it, so this
        // asserts "real repository" and "one commit" together.
        assert!(
            repo.blob_sha(Path::new(LIVE))
                .expect("blob_sha query succeeds")
                .is_some(),
            "{LIVE} must be present in HEAD"
        );
    }

    #[test]
    fn m06_ground_truth_names_files_that_are_really_there() {
        let (dir, truth) = materialize_into_tempdir();

        assert_eq!(truth.live_paths, vec![Path::new(LIVE).to_path_buf()]);
        assert_eq!(truth.live_symbols, vec![LIVE_SYMBOL.to_string()]);
        assert_eq!(truth.decoy_dead_paths.len(), ConcurrencyHelper::DECOYS.len());

        for path in truth.live_paths.iter().chain(&truth.decoy_dead_paths) {
            assert!(
                dir.path().join(path).is_file(),
                "ground truth names {} but it is not on disk",
                path.display()
            );
        }
    }

    #[test]
    fn m06_the_helper_is_called_only_from_the_contended_branch() {
        let (dir, _truth) = materialize_into_tempdir();

        assert_eq!(
            files_mentioning(dir.path(), LIVE_SYMBOL),
            vec![MECHANISM.to_string(), LIVE.to_string()],
            "only the definition and the one contended branch may name the helper"
        );

        let source = std::fs::read_to_string(dir.path().join(MECHANISM))
            .expect("mechanism file is readable");
        let branch = source
            .find(CONTENDED_BRANCH)
            .expect("the mechanism file must open a contended branch");
        let call = source
            .find(&format!("{LIVE_SYMBOL}("))
            .expect("the mechanism file must call the helper");
        assert!(
            call > branch,
            "the helper must be called only after `{CONTENDED_BRANCH}`"
        );
    }

    #[test]
    fn m06_no_test_in_the_repository_can_contend() {
        let (dir, _truth) = materialize_into_tempdir();

        // The branch is reachable only with a second thread in existence. If
        // nothing in the repository can create one, then running the suite --
        // under coverage, under a tracer, under anything -- reports the helper
        // as dead every time, which is §3.4 Issue 4 exactly.
        for spawner in ["thread::spawn", "std::thread", "scope("] {
            assert!(
                files_mentioning(dir.path(), spawner).is_empty(),
                "{spawner} appears in the fixture; the suite must not be able to contend"
            );
        }
    }

    #[test]
    fn m06_decoys_are_named_nowhere_at_all() {
        let (dir, truth) = materialize_into_tempdir();

        for decoy in &truth.decoy_dead_paths {
            let stem = decoy
                .file_stem()
                .and_then(|s| s.to_str())
                .expect("decoy has a UTF-8 stem");
            let mentions = files_mentioning(dir.path(), stem);
            assert!(
                mentions.iter().all(|f| Path::new(f) == decoy),
                "a decoy that anything mentions is not a decoy; {stem} appears in {mentions:?}"
            );
        }
    }
}
