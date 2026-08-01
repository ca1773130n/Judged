//! Gate 2e — the recent-modification veto (§9.3 2e).
//!
//! A file whose last modification is within `N` days (default 7) is **vetoed**.
//! That is the whole gate, and it is the most counter-intuitive one in the set,
//! so the measurement that produced it is written out here rather than cited.
//!
//! # Age is anti-predictive, and this module uses it inverted
//!
//! §6.18 labelled 9,588 tracked files across six mature Python repositories
//! (django, scikit-learn, pytest, fastapi, flask, requests) at a 2021-07-31
//! snapshot by whether a human deleted them by 2026-07, rename-corrected with
//! `-M90%`:
//!
//! | Last touched at snapshot | n | P(deleted within 4y) |
//! |---|---|---|
//! | <90d | 1726 | 9.4% |
//! | 90–365d | 2079 | 6.3% |
//! | 1–2y | 2024 | 12.5% |
//! | 2–4y | 1936 | 1.9% |
//! | **>4y** | **1823** | **1.4%** |
//! | *base rate* | 9588 | **6.4%** |
//!
//! The `>4y` bucket is the **lowest** bucket, below the base rate, and it is
//! lowest in every one of the six repositories individually. Flagging
//! "untouched for more than four years" has ~1.4% precision: **70 wrong
//! deletions per right one.**
//!
//! **So: nothing in this module may ever accuse.** Someone will eventually read
//! `Abstain` on a file untouched for five years and reach for the obvious
//! change — make old files a deletion signal, or let them raise a score. Three
//! shipped tools already encode exactly that (NickCrew's *"No imports, >6 months
//! old | Remove"*; rohitg00's *"Code untouched for 6+ months with no references
//! is likely dead"*; repowise assigning its **maximum** confidence 1.00 to "no
//! commits in 90 days"). The table above is the measurement that says all three
//! are backwards. Age measures **stability**, not deadness.
//!
//! The research is honest about that table's own limits, and so is this module:
//! the label is "a human deleted this path within four years", which is not
//! "was dead at T"; surviving files are unlabelled; django alone contributes
//! 6,482 of the 9,588 files; the corpus holds no abandoned enterprise features.
//! Its own verdict is *"direction: probably right. Confidence: not earned."*
//! That is precisely why the one use it is put to is the veto: a weakly-founded
//! signal is safe to spend where a wrong answer costs disk space, and unsafe to
//! spend where a wrong answer deletes live code (§1.3).
//!
//! Inverted, the signal earns its place: **recent modification is the only
//! evidence that catches the work-in-progress false positive**, the case where
//! static analysis, coverage and production telemetry all agree a file is
//! unused and all are wrong, because the code that will call it was written
//! this week and is not wired up yet. No other gate sees that file.
//!
//! # A veto can only rescue
//!
//! [`RecencyVerdict`] has two states: [`Vetoed`](RecencyVerdict::Vetoed) and
//! [`Abstain`](RecencyVerdict::Abstain). There is deliberately no third. A veto
//! is absorbing — no later evidence overrides it — and `Abstain` means only
//! "2e has nothing to say about this candidate", never "2e agrees it is dead".
//!
//! # Two environment faults that silently void this gate (§6.19)
//!
//! 1. **Shallow clones are the CI default.** `actions/checkout` fetches a
//!    single commit unless `fetch-depth: 0`. Measured on git 2.50.1, cloning
//!    one bare repository twice — a file genuinely committed at unix
//!    1600000000 reports `git log -1 --format=%ct` = 1600000000 in the full
//!    clone and **1750000000 in the `--depth 1` clone**, the tip commit's date,
//!    wrong by 4.75 years and *identical to every other file in the tree*.
//!    Every query still exits 0. The gate's answer therefore collapses into a
//!    single repository-wide constant with no per-file discrimination left in
//!    it, and recording that as "2e examined this file and cleared it" is a
//!    claim of evidence nobody gathered. So when [`Repo::is_shallow`] is true
//!    this module vetoes **everything**, with [`VetoReason::ShallowHistory`],
//!    and names the fix. A gate that cannot run is not a gate that found
//!    nothing — §6.20's rule applied to the substrate rather than to an
//!    analyzer.
//! 2. **mtime is void after any checkout.** Clone, `rsync`, `docker COPY`, CI
//!    checkout and Time Machine restore all reset it, so in the unattended
//!    context where this tool is most dangerous *every* file's mtime is today.
//!    In the other direction `.env` files, keystores and datasets are written
//!    once and never touched, giving live configuration the oldest mtimes in
//!    the repository. This module therefore reads **git commit timestamps only,
//!    and never `std::fs::Metadata::modified`** — a rule the test suite pins in
//!    both directions rather than trusting to review.
//!
//! # Cost
//!
//! Two or three `git` invocations per candidate, one of which refreshes the
//! index. That is deliberate — a wrong answer here is unrecoverable and a slow
//! one is not — but a whole-tree scan should grow a batched entry point (hoist
//! the shallow check, and `git log`/`git status` both accept many pathspecs)
//! rather than calling [`RecencyVeto::judge`] in a loop.

use std::ffi::{OsStr, OsString};
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::git::Repo;

/// Environment variables that redirect git at a different repository.
///
/// Mirrors the list in [`crate::git`], and for the same reason: every git hook
/// runs with `GIT_DIR` set, so an inherited one would have this gate answer
/// "when was path X last modified" out of repository Y — an answer that is
/// wrong with total confidence. The child never sees them.
///
/// The duplication is a file-ownership artefact, not a design: once these
/// modules land together, this and [`crate::git`]'s private `GitRun` should
/// become one runner.
const INHERITED_GIT_ENV: [&str; 6] = [
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_COMMON_DIR",
    "GIT_NAMESPACE",
];

/// Why Gate 2e rescued a candidate.
///
/// Every variant is a *rescue*. None of them is, or may become, an accusation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VetoReason {
    /// The last commit touching this path is inside the window.
    ///
    /// The work-in-progress case this gate exists for.
    RecentCommit {
        /// Unix seconds of the newest of the commit's author and committer
        /// dates.
        committed_at: i64,
        /// How long ago that was, clamped at zero for clock skew.
        age: Duration,
    },

    /// The working tree differs from `HEAD` at this path, or git has never seen
    /// this content at all.
    ///
    /// An uncommitted edit is the most recent modification there is, and it is
    /// invisible to a history query: a file last committed two years ago and
    /// being rewritten right now reads as two years old (§9.3 Gate 0d refuses
    /// to auto-act on a dirty tree for the same reason).
    UncommittedChange,

    /// No commit in this repository's history touches this path, so git holds
    /// no timestamp for it.
    ///
    /// "No timestamp" must never be read as "old": the newest thing in any
    /// repository is precisely the thing not yet committed.
    NoCommitTimestamp,

    /// The clone is shallow or partial, so no history-derived signal can run.
    ///
    /// The CI default (§6.19). Vetoing everything is the only sound response:
    /// in a depth-1 clone every file reports the same commit.
    ShallowHistory,

    /// A query failed, timed out, or returned something unparseable.
    ///
    /// §6.20's rule, and the reason this whole judgement is infallible: a
    /// search that did not finish has found nothing *because it did not look*.
    /// Meta hit exactly this in production, where a truncated BigGrep read as
    /// "no references" turned the safety net into the deletion trigger.
    EvidenceUnavailable {
        /// The failure, quoted, so it can be fixed rather than guessed at.
        detail: String,
    },
}

impl fmt::Display for VetoReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VetoReason::RecentCommit { committed_at, age } => write!(
                f,
                "last commit touching this path was {} day(s) ago (unix {committed_at}); \
                 recent modification is the only signal that catches work in progress (§6.18)",
                age.as_secs() / 86_400
            ),
            VetoReason::UncommittedChange => f.write_str(
                "the working tree differs from HEAD at this path (or the content was never \
                 committed); an uncommitted edit is the most recent modification there is",
            ),
            VetoReason::NoCommitTimestamp => f.write_str(
                "no commit in this repository touches this path, so git holds no timestamp \
                 for it; absence of a timestamp is not evidence of age",
            ),
            VetoReason::ShallowHistory => f.write_str(
                "this clone is shallow or partial, so every file reports the same commit and \
                 no history-derived signal can run; re-run with full history \
                 (actions/checkout: fetch-depth: 0) — a gate that cannot run is not a gate \
                 that found nothing (§6.19)",
            ),
            VetoReason::EvidenceUnavailable { detail } => write!(
                f,
                "could not establish when this path was last modified: {detail}; \
                 a failed search is a hit, never an absence (§6.20)",
            ),
        }
    }
}

/// What Gate 2e has to say about one candidate.
///
/// Two states, on purpose. A veto rescues; abstention says nothing at all.
/// There is no variant meaning "dead", and adding one would make this module a
/// second accuser rather than a veto (§9.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecencyVerdict {
    /// Rescued, for the stated reason. Absorbing: no later evidence overrides
    /// it.
    Vetoed(VetoReason),

    /// Gate 2e adds nothing.
    ///
    /// **This is not agreement.** It means only that the last commit touching
    /// this path is older than the window — which §6.18 measured to be
    /// *anti*-predictive of deadness (1.4% against a 6.4% base rate). Any
    /// deletion decision has to be carried entirely by other evidence.
    Abstain,
}

impl RecencyVerdict {
    /// Whether this verdict rescues the candidate.
    pub fn is_veto(&self) -> bool {
        matches!(self, RecencyVerdict::Vetoed(_))
    }

    /// The reason, when there is one.
    pub fn reason(&self) -> Option<&VetoReason> {
        match self {
            RecencyVerdict::Vetoed(reason) => Some(reason),
            RecencyVerdict::Abstain => None,
        }
    }
}

/// Gate 2e, configured with its window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecencyVeto {
    window: Duration,
}

impl RecencyVeto {
    /// §9.3 2e's default: seven days.
    ///
    /// A week covers the sprint-length gap between writing a helper and wiring
    /// it up, which is the false positive being defended against. The number is
    /// a judgement, not a measurement — §6.18 measured the *direction* of the
    /// age signal, never a cutoff — so it is exposed
    /// ([`RecencyVeto::with_window_days`]) rather than baked in.
    pub const DEFAULT_WINDOW_DAYS: u64 = 7;

    /// Gate 2e with an `N`-day window.
    ///
    /// A window of 0 still vetoes anything committed this second (the bound is
    /// inclusive), and every non-age veto — shallow clone, dirty tree, missing
    /// timestamp, failed query — is independent of the window and cannot be
    /// configured away.
    pub fn with_window_days(days: u64) -> RecencyVeto {
        RecencyVeto {
            window: Duration::from_secs(days.saturating_mul(86_400)),
        }
    }

    /// The configured window.
    pub fn window(&self) -> Duration {
        self.window
    }

    /// Judge one candidate path.
    ///
    /// Infallible **by design**: there is no error channel for a caller to
    /// misread as "no veto". Every failure — a broken repository, an
    /// unparseable timestamp, a path outside the working tree — becomes
    /// [`VetoReason::EvidenceUnavailable`] carrying the failure text, so the
    /// problem is reported *and* the candidate is rescued while it exists
    /// (§6.20).
    ///
    /// `path` may be absolute or relative to the working tree root.
    pub fn judge(&self, repo: &Repo, path: &Path) -> RecencyVerdict {
        let root = repo.root();
        let rel = match relative_to(root, path) {
            Ok(rel) => rel,
            Err(detail) => return unavailable(detail),
        };

        // Can this gate run at all? Asked first, because in a shallow clone
        // every answer below is the same answer for every file in the tree.
        match repo.is_shallow() {
            Ok(true) => return RecencyVerdict::Vetoed(VetoReason::ShallowHistory),
            Ok(false) => {}
            Err(source) => {
                return unavailable(format!(
                    "could not determine whether {} is a shallow clone: {source}",
                    root.display()
                ))
            }
        }

        // Working-tree state before history: an unstaged edit is newer than any
        // commit, and history cannot see it.
        match worktree_differs_from_head(root, &rel) {
            Ok(true) => return RecencyVerdict::Vetoed(VetoReason::UncommittedChange),
            Ok(false) => {}
            Err(detail) => return unavailable(detail),
        }

        let touched = match last_touch(root, &rel) {
            Ok(Some(seconds)) => seconds,
            Ok(None) => return RecencyVerdict::Vetoed(VetoReason::NoCommitTimestamp),
            Err(detail) => return unavailable(detail),
        };
        let now = match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(since) => i64::try_from(since.as_secs()).unwrap_or(i64::MAX),
            Err(source) => return unavailable(format!("system clock is before 1970: {source}")),
        };

        // Signed on purpose: a commit dated in the future (clock skew on a
        // build agent, a deliberate `--date`) gives a negative age, which is
        // inside the window and must veto rather than wrap around into
        // "ancient".
        let age_seconds = now.saturating_sub(touched);
        let window_seconds = i64::try_from(self.window.as_secs()).unwrap_or(i64::MAX);
        if age_seconds <= window_seconds {
            return RecencyVerdict::Vetoed(VetoReason::RecentCommit {
                committed_at: touched,
                age: Duration::from_secs(u64::try_from(age_seconds).unwrap_or(0)),
            });
        }
        RecencyVerdict::Abstain
    }
}

impl Default for RecencyVeto {
    fn default() -> RecencyVeto {
        RecencyVeto::with_window_days(RecencyVeto::DEFAULT_WINDOW_DAYS)
    }
}

fn unavailable(detail: String) -> RecencyVerdict {
    RecencyVerdict::Vetoed(VetoReason::EvidenceUnavailable { detail })
}

/// Unix seconds of the newest commit touching `rel`, or `None` when no commit
/// does.
///
/// Reads **both** dates and keeps the later one. `git am`, `git rebase` and
/// `git cherry-pick` preserve the author date from the original patch and stamp
/// the committer date now, so a file that landed this morning can carry a
/// two-year-old author date; the reverse (`--date` in the future) is rarer but
/// equally survivable. The newest evidence of a touch is the one that matters.
///
/// `--no-show-signature` because `log.showSignature=true` in the ambient config
/// would prepend gpg output to what we parse — and would block on an agent
/// prompt while doing it.
fn last_touch(root: &Path, rel: &Path) -> Result<Option<i64>, String> {
    let output = run_git(
        root,
        &[
            OsString::from("log"),
            OsString::from("-1"),
            OsString::from("--no-show-signature"),
            // NUL-separated so no locale or format setting can merge the two.
            OsString::from("--format=%ct%x00%at"),
            OsString::from("--"),
            literal_pathspec(rel),
        ],
    )?;
    if !output.status.success() {
        return Err(describe_failure("log", root, &output));
    }
    let stdout = std::str::from_utf8(&output.stdout)
        .map_err(|source| format!("`git log` printed non-UTF-8 output: {source}"))?;
    let record = stdout.trim();
    // An empty log is a *completed* search that found no commit touching this
    // path. That is not an age, so the caller vetoes — but it is a different
    // state from a failed query, and the two must not be merged (§6.20).
    if record.is_empty() {
        return Ok(None);
    }
    let (committer, author) = record
        .split_once('\0')
        .ok_or_else(|| format!("unparseable `git log` record: {record:?}"))?;
    let parse = |field: &str, label: &str| -> Result<i64, String> {
        field
            .trim()
            .parse::<i64>()
            .map_err(|source| format!("unparseable {label} timestamp {field:?}: {source}"))
    };
    Ok(Some(std::cmp::max(
        parse(committer, "committer")?,
        parse(author, "author")?,
    )))
}

/// Whether the working tree or index differs from `HEAD` at `rel`.
///
/// True for a modified tracked file, a staged change, a deletion, and for
/// untracked content (`--untracked-files=all`, so that a candidate *directory*
/// reports uncommitted work inside it rather than hiding it).
///
/// `--no-optional-locks` so that this read never takes `index.lock`: parallel
/// worktrees, several CI jobs and several agents on one checkout are the
/// documented normal case (§6.19), and a safety gate must not fail — or make
/// someone else fail — over a lock it only wanted in order to look.
fn worktree_differs_from_head(root: &Path, rel: &Path) -> Result<bool, String> {
    let output = run_git(
        root,
        &[
            OsString::from("--no-optional-locks"),
            OsString::from("status"),
            OsString::from("--porcelain"),
            OsString::from("-z"),
            OsString::from("--untracked-files=all"),
            OsString::from("--"),
            literal_pathspec(rel),
        ],
    )?;
    if !output.status.success() {
        return Err(describe_failure("status", root, &output));
    }
    Ok(!output.stdout.is_empty())
}

/// Resolve `path` to a path relative to the working tree root.
///
/// Gate 0c (§9.3) is a real-path containment check, so symlinks are resolved
/// where possible; `canonicalize` fails on paths that do not exist, and "judge
/// a path that is already gone" is a legitimate question, so an unresolvable
/// path falls back to its lexical form and git refuses anything that escapes.
///
/// The root itself is refused rather than judged: it reduces to the empty
/// pathspec, and `git log -1 -- :(literal)` with an empty path reports the
/// repository's newest commit (verified on git 2.50.1), so an unguarded root
/// would take the whole working tree's age from whatever was committed last —
/// and clear the entire tree once that aged past the window.
fn relative_to(root: &Path, path: &Path) -> Result<PathBuf, String> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    let resolved = std::fs::canonicalize(&absolute).unwrap_or(absolute);
    let rel = resolved.strip_prefix(root).map_err(|_| {
        format!(
            "{} is outside the working tree {}",
            path.display(),
            root.display()
        )
    })?;
    if rel.as_os_str().is_empty() {
        return Err(format!(
            "{} is the working tree root, not a candidate path",
            root.display()
        ));
    }
    Ok(rel.to_path_buf())
}

/// Run `git <args>` in `root`, with the ambient repository redirection scrubbed.
fn run_git(root: &Path, args: &[OsString]) -> Result<Output, String> {
    let mut cmd = Command::new("git");
    cmd.args(args)
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for var in INHERITED_GIT_ENV {
        cmd.env_remove(var);
    }
    // A gate that blocks on a credential prompt mid-scan is indistinguishable
    // from one that has crashed.
    cmd.env("GIT_TERMINAL_PROMPT", "0");
    cmd.output()
        .map_err(|source| format!("failed to run `git` in {}: {source}", root.display()))
}

/// A failed git invocation, described well enough to reproduce by hand.
fn describe_failure(subcommand: &str, root: &Path, output: &Output) -> String {
    format!(
        "`git {subcommand}` in {} exited with {}: {}",
        root.display(),
        match output.status.code() {
            Some(code) => code.to_string(),
            None => "a signal".to_string(),
        },
        String::from_utf8_lossy(&output.stderr).trim()
    )
}

/// Wrap `rel` as a literal pathspec.
///
/// Without `:(literal)`, git reads `report[1].txt` as a glob and can report the
/// modification time of a *different* file.
fn literal_pathspec(rel: &Path) -> OsString {
    let mut spec = OsString::from(":(literal)");
    spec.push(OsStr::new(rel.as_os_str()));
    spec
}
