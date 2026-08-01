//! Behavioural tests for Gate 2e, the recent-modification veto, run against
//! **real repositories** built by the `git` binary.
//!
//! Nothing here is mocked, for the same reason `git_repo.rs` mocks nothing: the
//! two hazards this gate exists to survive (§6.19) are *properties of git and of
//! the filesystem*, not properties of our beliefs about them. A shallow clone
//! that answers every history query successfully while holding one commit, and a
//! checkout that stamps today's mtime onto a file last edited in 2019, cannot be
//! faked by a stub without first assuming the answer.
//!
//! Two of these tests are therefore the point of the file:
//!
//! - [`shallow_clone_vetoes_a_file_a_full_clone_clears`] clones the *same bare
//!   repository* twice, once at `--depth 1`, and asserts the verdicts differ.
//! - [`ancient_mtime_does_not_defeat_a_recent_commit`] and
//!   [`fresh_mtime_does_not_manufacture_a_recent_commit`] drive mtime and commit
//!   time in opposite directions and pin the verdict to the commit.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use judged_core::git::Repo;
use judged_core::veto::recency::{RecencyVerdict, RecencyVeto, VetoReason};

// ---------------------------------------------------------------------------
// test scaffolding
// ---------------------------------------------------------------------------

/// A [`tempfile::TempDir`] whose path has been canonicalized once.
///
/// macOS hands out temp roots under `/var/folders/…`, a symlink to
/// `/private/var/folders/…`, while `git rev-parse --show-toplevel` answers with
/// the resolved form. Comparing an unresolved fixture path against
/// [`Repo::root`] fails on exactly the platform these tests run on.
struct TempDir {
    /// Held for its `Drop`: removing the directory tree is tempfile's job.
    _guard: tempfile::TempDir,
    path: PathBuf,
}

impl TempDir {
    fn new(label: &str) -> TempDir {
        let guard = tempfile::Builder::new()
            .prefix(&format!("judged-core-recency-{label}-"))
            .tempdir()
            .expect("create temp dir");
        let path = fs::canonicalize(guard.path()).expect("canonicalize temp dir");
        TempDir {
            _guard: guard,
            path,
        }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

/// Run `git` in `dir` with extra environment, returning trimmed stdout.
fn git_env(dir: &Path, args: &[&str], env: &[(&str, String)]) -> String {
    let mut cmd = Command::new("git");
    cmd.args(args).current_dir(dir);
    for (key, value) in env {
        cmd.env(key, value);
    }
    let out = cmd.output().expect("spawn git");
    assert!(
        out.status.success(),
        "git {:?} failed in {}: {}",
        args,
        dir.display(),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout)
        .expect("git stdout is utf-8")
        .trim()
        .to_string()
}

fn git(dir: &Path, args: &[&str]) -> String {
    git_env(dir, args, &[])
}

fn write(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent dir");
    }
    fs::write(path, contents).expect("write file");
}

fn now_epoch() -> i64 {
    i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock is after 1970")
            .as_secs(),
    )
    .expect("epoch seconds fit in i64")
}

fn days_ago(days: i64) -> i64 {
    now_epoch() - days * 86_400
}

/// git's raw-timestamp date format: `@<unix seconds> <utc offset>`.
fn git_date(epoch: i64) -> String {
    format!("@{epoch} +0000")
}

fn init(dir: &Path) {
    fs::create_dir_all(dir).expect("create repo dir");
    git(dir, &["-c", "init.defaultBranch=main", "init", "-q", "."]);
}

/// Commit the index with author *and* committer date pinned to `epoch`.
fn commit_at(dir: &Path, message: &str, epoch: i64) {
    commit_split(dir, message, epoch, epoch);
}

/// Commit with the two dates driven independently — the `git am` / patch-import
/// shape, where the author date is old and the committer date is now.
fn commit_split(dir: &Path, message: &str, author_epoch: i64, committer_epoch: i64) {
    git_env(
        dir,
        &[
            "-c",
            "user.name=Judged Test",
            "-c",
            "user.email=test@judged.invalid",
            "commit",
            "-q",
            "--no-gpg-sign",
            "-m",
            message,
        ],
        &[
            ("GIT_AUTHOR_DATE", git_date(author_epoch)),
            ("GIT_COMMITTER_DATE", git_date(committer_epoch)),
        ],
    );
}

/// `<name>` committed at `epoch`, in a repository created for the purpose.
fn repo_with_file_committed_at(dir: &Path, name: &str, epoch: i64) {
    init(dir);
    write(&dir.join(name), "content\n");
    git(dir, &["add", "-A"]);
    commit_at(dir, "seed", epoch);
}

fn open(dir: &Path) -> Repo {
    Repo::discover(dir).expect("discover repo")
}

/// Assert a veto and return its reason, so callers can pin *why*.
///
/// "Vetoed for some reason" is not good enough for this gate: a shallow clone
/// and a file edited this morning are both vetoes, and only one of them means
/// the gate ran.
fn expect_veto(verdict: RecencyVerdict) -> VetoReason {
    match verdict {
        RecencyVerdict::Vetoed(reason) => reason,
        RecencyVerdict::Abstain => panic!("expected a veto, got Abstain"),
    }
}

fn assert_abstains(verdict: &RecencyVerdict) {
    assert_eq!(
        verdict,
        &RecencyVerdict::Abstain,
        "expected the gate to abstain, got {verdict:?}"
    );
    // Abstain is the *absence of a rescue*, never an accusation. Gate 2e can
    // only ever add a veto (§9.1), so nothing here may read as "delete it".
    assert!(!verdict.is_veto());
}

// ---------------------------------------------------------------------------
// the window itself (§9.3 2e, §6.18)
// ---------------------------------------------------------------------------

#[test]
fn the_default_window_is_seven_days() {
    assert_eq!(RecencyVeto::DEFAULT_WINDOW_DAYS, 7);
    assert_eq!(
        RecencyVeto::default().window(),
        std::time::Duration::from_secs(7 * 86_400)
    );
}

#[test]
fn a_commit_inside_the_window_is_vetoed() {
    let td = TempDir::new("inside");
    let root = td.path().join("repo");
    repo_with_file_committed_at(&root, "wip.rs", days_ago(1));

    let verdict = RecencyVeto::default().judge(&open(&root), Path::new("wip.rs"));

    // The work-in-progress false positive: static analysis, coverage and
    // production evidence can all agree this file is unused and all be wrong,
    // because the code that will use it was written yesterday and has not been
    // wired up yet (§6.18).
    match expect_veto(verdict) {
        VetoReason::RecentCommit { committed_at, .. } => {
            assert!(committed_at > days_ago(2), "wrong commit timestamp read");
        }
        other => panic!("expected RecentCommit, got {other:?}"),
    }
}

#[test]
fn a_commit_older_than_the_window_abstains() {
    let td = TempDir::new("outside");
    let root = td.path().join("repo");
    repo_with_file_committed_at(&root, "old.rs", days_ago(400));

    let verdict = RecencyVeto::default().judge(&open(&root), Path::new("old.rs"));

    // **This is the assertion someone will eventually try to strengthen into an
    // accusation.** §6.18 measured that direction: files untouched >4y were
    // deleted at 1.4% against a 6.4% base rate — 70 wrong deletions per right
    // one. Abstain is the strongest thing age may ever say.
    assert_abstains(&verdict);
}

#[test]
fn the_window_boundary_sits_at_seven_days_within_the_hour() {
    let td = TempDir::new("boundary");
    let root = td.path().join("repo");
    init(&root);
    // One hour either side of the seven-day line, and deliberately not *on* it:
    // a commit dated exactly `now - 7d` lands on the inclusive bound only if
    // the test and the gate read the same integer second, which is a coin
    // flip. An hour of slack pins the boundary to ±1h — tight enough that no
    // off-by-a-day window survives, loose enough to be deterministic.
    write(&root.join("stale.rs"), "stale\n");
    git(&root, &["add", "-A"]);
    commit_at(&root, "stale", days_ago(7) - 3_600);
    write(&root.join("fresh.rs"), "fresh\n");
    git(&root, &["add", "-A"]);
    commit_at(&root, "fresh", days_ago(7) + 3_600);

    let veto = RecencyVeto::default();
    let repo = open(&root);

    assert!(
        veto.judge(&repo, Path::new("fresh.rs")).is_veto(),
        "6d23h must be inside a seven-day window"
    );
    assert_abstains(&veto.judge(&repo, Path::new("stale.rs")));
}

#[test]
fn the_window_is_configurable_and_applies_to_the_same_history() {
    let td = TempDir::new("window");
    let root = td.path().join("repo");
    repo_with_file_committed_at(&root, "f.rs", days_ago(20));

    let repo = open(&root);

    assert_abstains(&RecencyVeto::default().judge(&repo, Path::new("f.rs")));
    assert!(RecencyVeto::with_window_days(30)
        .judge(&repo, Path::new("f.rs"))
        .is_veto());
}

#[test]
fn age_is_read_per_path_not_from_the_repository_tip() {
    let td = TempDir::new("perpath");
    let root = td.path().join("repo");
    init(&root);
    write(&root.join("ancient.rs"), "ancient\n");
    git(&root, &["add", "-A"]);
    commit_at(&root, "ancient", days_ago(400));
    write(&root.join("today.rs"), "today\n");
    git(&root, &["add", "-A"]);
    commit_at(&root, "today", now_epoch());

    let veto = RecencyVeto::default();
    let repo = open(&root);

    // HEAD is seconds old. A gate that read the tip commit's date — or the
    // repository's — would veto every file in every active repository, which is
    // a gate that never runs.
    assert!(veto.judge(&repo, Path::new("today.rs")).is_veto());
    assert_abstains(&veto.judge(&repo, Path::new("ancient.rs")));
}

#[test]
fn a_recent_committer_date_vetoes_despite_an_ancient_author_date() {
    let td = TempDir::new("amdate");
    let root = td.path().join("repo");
    init(&root);
    write(&root.join("imported.rs"), "from a patch\n");
    git(&root, &["add", "-A"]);
    // `git am` preserves the author date from the patch and stamps the
    // committer date now; so does every rebase and cherry-pick. Reading only
    // the author date would call a file landed this morning two years old.
    commit_split(&root, "import", days_ago(700), now_epoch());

    let verdict = RecencyVeto::default().judge(&open(&root), Path::new("imported.rs"));

    assert!(
        verdict.is_veto(),
        "an ancient author date must not hide a commit made today: {verdict:?}"
    );
}

// ---------------------------------------------------------------------------
// hazard 1 — shallow clones are the CI default (§6.19)
// ---------------------------------------------------------------------------

#[test]
fn shallow_clone_vetoes_a_file_a_full_clone_clears() {
    let td = TempDir::new("shallow");
    let bare = td.path().join("origin.git");
    let work = td.path().join("work");
    git(
        td.path(),
        &[
            "-c",
            "init.defaultBranch=main",
            "init",
            "-q",
            "--bare",
            "origin.git",
        ],
    );
    repo_with_file_committed_at(&work, "old.rs", days_ago(400));
    git(&work, &["remote", "add", "origin", &bare.to_string_lossy()]);
    git(&work, &["push", "-q", "origin", "main"]);

    // `--depth` is ignored for plain local clones; `file://` forces the real
    // protocol, which is what `actions/checkout` does.
    let url = format!("file://{}", bare.display());
    git(td.path(), &["clone", "-q", &url, "full"]);
    git(td.path(), &["clone", "-q", "--depth", "1", &url, "shallow"]);
    let full = td.path().join("full");
    let shallow = td.path().join("shallow");
    assert!(
        shallow.join(".git/shallow").exists(),
        "fixture is not shallow"
    );

    let veto = RecencyVeto::default();

    // Same bare repository, same commit, same file, same 400-day-old timestamp
    // — which the shallow clone still reports correctly. The verdicts differ
    // only because one clone can answer history questions and the other cannot.
    assert_abstains(&veto.judge(&open(&full), Path::new("old.rs")));
    assert_eq!(
        expect_veto(veto.judge(&open(&shallow), Path::new("old.rs"))),
        VetoReason::ShallowHistory,
        "a gate that cannot run is not a gate that found nothing"
    );
}

#[test]
fn the_shallow_veto_says_why() {
    let td = TempDir::new("shallow-why");
    let bare = td.path().join("origin.git");
    let work = td.path().join("work");
    git(
        td.path(),
        &[
            "-c",
            "init.defaultBranch=main",
            "init",
            "-q",
            "--bare",
            "origin.git",
        ],
    );
    repo_with_file_committed_at(&work, "old.rs", days_ago(400));
    git(&work, &["remote", "add", "origin", &bare.to_string_lossy()]);
    git(&work, &["push", "-q", "origin", "main"]);
    let url = format!("file://{}", bare.display());
    git(td.path(), &["clone", "-q", "--depth", "1", &url, "shallow"]);

    let verdict =
        RecencyVeto::default().judge(&open(&td.path().join("shallow")), Path::new("old.rs"));

    // A veto nobody can explain gets overridden by the next person to read the
    // output. The reason has to name the environment fault and the fix.
    let rendered = expect_veto(verdict).to_string();
    assert!(
        rendered.contains("shallow"),
        "veto reason must name the fault: {rendered}"
    );
    assert!(
        rendered.contains("fetch-depth"),
        "veto reason must name the fix: {rendered}"
    );
}

// ---------------------------------------------------------------------------
// hazard 2 — mtime is void after any checkout (§6.19)
// ---------------------------------------------------------------------------

#[test]
fn ancient_mtime_does_not_defeat_a_recent_commit() {
    let td = TempDir::new("old-mtime");
    let root = td.path().join("repo");
    repo_with_file_committed_at(&root, "keystore.p12", days_ago(1));
    // The `.env`/keystore shape: written once, never touched again, so it holds
    // the *oldest* mtime in the repository while being live configuration.
    let out = Command::new("touch")
        .args(["-t", "200001010000"])
        .arg(root.join("keystore.p12"))
        .output()
        .expect("spawn touch");
    assert!(out.status.success(), "touch failed");
    let mtime = fs::metadata(root.join("keystore.p12"))
        .expect("stat")
        .modified()
        .expect("mtime");
    assert!(
        mtime < SystemTime::now() - std::time::Duration::from_secs(365 * 86_400),
        "fixture mtime was not backdated"
    );

    let verdict = RecencyVeto::default().judge(&open(&root), Path::new("keystore.p12"));

    assert!(
        verdict.is_veto(),
        "a 26-year-old mtime must not override a commit made yesterday: {verdict:?}"
    );
}

#[test]
fn fresh_mtime_does_not_manufacture_a_recent_commit() {
    let td = TempDir::new("new-mtime");
    let root = td.path().join("repo");
    repo_with_file_committed_at(&root, "old.rs", days_ago(400));
    // Clone, rsync, `docker COPY`, CI checkout and Time Machine restore all do
    // exactly this: rewrite mtime to now without touching content.
    let out = Command::new("touch")
        .arg(root.join("old.rs"))
        .output()
        .expect("spawn touch");
    assert!(out.status.success(), "touch failed");
    let mtime = fs::metadata(root.join("old.rs"))
        .expect("stat")
        .modified()
        .expect("mtime");
    assert!(
        mtime > SystemTime::now() - std::time::Duration::from_secs(600),
        "fixture mtime was not refreshed"
    );

    let verdict = RecencyVeto::default().judge(&open(&root), Path::new("old.rs"));

    // If this vetoed, the gate would fire on every file of every CI checkout —
    // indistinguishable from a gate that vetoes unconditionally, and no signal
    // at all.
    assert_abstains(&verdict);
}

// ---------------------------------------------------------------------------
// content git has no timestamp for
// ---------------------------------------------------------------------------

#[test]
fn an_uncommitted_edit_vetoes_a_file_whose_last_commit_is_ancient() {
    let td = TempDir::new("dirty");
    let root = td.path().join("repo");
    repo_with_file_committed_at(&root, "old.rs", days_ago(400));
    write(&root.join("old.rs"), "content\nbeing rewritten right now\n");

    let verdict = RecencyVeto::default().judge(&open(&root), Path::new("old.rs"));

    // The purest work-in-progress case: history says two years, the working
    // tree says thirty seconds, and the working tree is right.
    assert_eq!(
        expect_veto(verdict),
        VetoReason::UncommittedChange,
        "an unstaged edit is the most recent modification there is"
    );
}

#[test]
fn an_untracked_file_is_vetoed_rather_than_cleared() {
    let td = TempDir::new("untracked");
    let root = td.path().join("repo");
    repo_with_file_committed_at(&root, "old.rs", days_ago(400));
    write(&root.join("scratch.rs"), "brand new\n");

    let verdict = RecencyVeto::default().judge(&open(&root), Path::new("scratch.rs"));

    // git has no timestamp for content it has never seen, and "no timestamp"
    // must never read as "old". §8.1: this is also the class git cannot give
    // back, so clearing it would be the worst available answer.
    assert!(
        verdict.is_veto(),
        "content git has no history for must not clear this gate: {verdict:?}"
    );
}

#[test]
fn a_path_no_commit_ever_touched_is_vetoed() {
    let td = TempDir::new("nohistory");
    let root = td.path().join("repo");
    repo_with_file_committed_at(&root, "old.rs", days_ago(400));

    // A candidate naming a path that is not on disk and not in history — a
    // stale finding, a typo, an already-deleted path. `git log` completes and
    // reports nothing, and nothing is not an age.
    let verdict = RecencyVeto::default().judge(&open(&root), Path::new("never/existed.rs"));

    assert_eq!(expect_veto(verdict), VetoReason::NoCommitTimestamp);
}

// ---------------------------------------------------------------------------
// the rule that outranks everything: a failed search is a HIT (§6.20)
// ---------------------------------------------------------------------------

#[test]
fn a_failed_git_invocation_vetoes_instead_of_abstaining() {
    let td = TempDir::new("broken");
    let root = td.path().join("repo");
    repo_with_file_committed_at(&root, "old.rs", days_ago(400));
    let repo = open(&root);
    // The repository this gate was about to interrogate disappears underneath
    // it — the concurrency and shared-workspace hazards of §6.19, and the shape
    // of every §6.20 self-failure: the query does not answer "nothing", it
    // fails to answer at all.
    fs::remove_dir_all(root.join(".git")).expect("remove .git");

    let verdict = RecencyVeto::default().judge(&repo, Path::new("old.rs"));

    // Before `.git` was removed this exact call abstained. An errored search
    // must not produce the same verdict as a completed one.
    match expect_veto(verdict) {
        VetoReason::EvidenceUnavailable { detail } => {
            assert!(
                !detail.is_empty(),
                "the failure must be reported, not eaten"
            );
        }
        other => panic!("expected EvidenceUnavailable, got {other:?}"),
    }
}

#[test]
fn a_path_outside_the_working_tree_is_vetoed_not_cleared() {
    let td = TempDir::new("outside-tree");
    let root = td.path().join("repo");
    repo_with_file_committed_at(&root, "old.rs", days_ago(400));
    let stranger = td.path().join("elsewhere.rs");
    write(&stranger, "not ours\n");

    let verdict = RecencyVeto::default().judge(&open(&root), &stranger);

    // This repository's history says nothing about a path outside it. That is
    // an absence of evidence, so it is a veto — never an answer.
    assert!(
        verdict.is_veto(),
        "a path this repo has no history for must not clear the gate: {verdict:?}"
    );
}

#[test]
fn the_working_tree_root_itself_is_vetoed() {
    let td = TempDir::new("root");
    let root = td.path().join("repo");
    repo_with_file_committed_at(&root, "old.rs", days_ago(400));

    let repo = open(&root);

    // An empty pathspec matches every tracked file (verified on git 2.50.1), so
    // an unguarded root would take the *repository's* newest commit as its age
    // and, once that aged past the window, clear the whole working tree.
    assert!(RecencyVeto::default().judge(&repo, &root).is_veto());
    assert!(RecencyVeto::default().judge(&repo, Path::new("")).is_veto());
}
