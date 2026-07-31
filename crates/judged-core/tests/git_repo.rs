//! Behavioural tests for [`judged_core::git`], run against **real repositories**.
//!
//! Nothing here is mocked. Gate 0g (§9.3) is the load-bearing safety decision in
//! this project — a wrong answer deletes work that git cannot give back (§8.1) —
//! and the only thing that can validate it is the `git` binary the tool actually
//! shells out to. A mock would encode our beliefs about git rather than git's
//! behaviour, and every hazard in §6.16/§6.17 is a case where those two differ.
//!
//! `judged-core` has no `tempfile` dev-dependency (the scaffold owns the
//! manifests), so this file carries a ~20-line temp directory of its own.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use judged_core::git::{RecoverabilityClass, Repo};

// ---------------------------------------------------------------------------
// test scaffolding
// ---------------------------------------------------------------------------

/// A directory under the system temp root, removed on drop.
struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(label: &str) -> TempDir {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "judged-core-git-{}-{}-{}",
            std::process::id(),
            n,
            label
        ));
        fs::create_dir_all(&path).expect("create temp dir");
        // Resolve symlinks (macOS hands out /var -> /private/var) so that the
        // paths we pass in compare equal to what `git rev-parse --show-toplevel`
        // reports back.
        let path = fs::canonicalize(&path).expect("canonicalize temp dir");
        TempDir { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

/// Run `git` in `dir` and return trimmed stdout, asserting success.
fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("spawn git");
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

fn write(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent dir");
    }
    fs::write(path, contents).expect("write file");
}

/// A repository with one commit, built entirely with the git CLI so that the
/// fixture never depends on the code under test.
fn repo_with_commit(dir: &Path) {
    fs::create_dir_all(dir).expect("create repo dir");
    git(dir, &["-c", "init.defaultBranch=main", "init", "-q", "."]);
    write(&dir.join("a.txt"), "alpha\n");
    git(dir, &["add", "-A"]);
    commit(dir, "initial");
}

fn commit(dir: &Path, message: &str) {
    git(
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
    );
}

/// Seed `<td>/origin.git` (bare) from `<td>/work` and return both paths.
fn repo_with_remote(td: &TempDir) -> (PathBuf, PathBuf) {
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
    repo_with_commit(&work);
    git(&work, &["remote", "add", "origin", &bare.to_string_lossy()]);
    git(&work, &["push", "-q", "origin", "main"]);
    (bare, work)
}

fn open(dir: &Path) -> Repo {
    Repo::discover(dir).expect("discover repo")
}

// ---------------------------------------------------------------------------
// discover
// ---------------------------------------------------------------------------

#[test]
fn discover_walks_up_to_the_working_tree_root() {
    let td = TempDir::new("discover");
    let root = td.path().join("repo");
    repo_with_commit(&root);
    let nested = root.join("src/deep");
    fs::create_dir_all(&nested).expect("create nested dir");

    let repo = open(&nested);

    assert_eq!(repo.root(), root.as_path());
}

#[test]
fn discover_outside_a_repository_is_an_error() {
    let td = TempDir::new("discover-none");

    let err = Repo::discover(td.path()).expect_err("must not invent a repository");

    // §12: "no repository here" and "git blew up" must never look like success.
    assert!(
        format!("{err}").contains("git"),
        "expected a git error, got: {err}"
    );
}

// ---------------------------------------------------------------------------
// Gate 0g — RecoverabilityClass (§9.3 0g, §8.1)
// ---------------------------------------------------------------------------

#[test]
fn tracked_file_reachable_from_a_remote_ref_is_tracked_pushed() {
    let td = TempDir::new("pushed");
    let (_bare, work) = repo_with_remote(&td);

    let repo = open(&work);

    assert_eq!(
        repo.recoverability(Path::new("a.txt")).expect("classify"),
        RecoverabilityClass::TrackedPushed
    );
}

#[test]
fn tracked_file_with_no_remote_is_tracked_unpushed_not_pushed() {
    let td = TempDir::new("unpushed");
    let root = td.path().join("repo");
    repo_with_commit(&root);

    let repo = open(&root);

    // §9.3 Gate 0d refuses to auto-act when HEAD is on no remote at all; the
    // classifier must not quietly upgrade "committed" to "pushed".
    assert_eq!(
        repo.recoverability(Path::new("a.txt")).expect("classify"),
        RecoverabilityClass::TrackedUnpushed
    );
}

#[test]
fn commit_not_yet_pushed_to_the_configured_remote_is_tracked_unpushed() {
    let td = TempDir::new("ahead");
    let (_bare, work) = repo_with_remote(&td);
    write(&work.join("b.txt"), "beta\n");
    git(&work, &["add", "-A"]);
    commit(&work, "second");

    let repo = open(&work);

    // A remote exists and HEAD~1 is on it, but HEAD is not: local-only content.
    assert_eq!(
        repo.recoverability(Path::new("b.txt")).expect("classify"),
        RecoverabilityClass::TrackedUnpushed
    );
}

#[test]
fn staged_but_never_committed_file_is_not_claimed_as_pushed() {
    let td = TempDir::new("unborn");
    let root = td.path().join("repo");
    fs::create_dir_all(&root).expect("create repo dir");
    git(&root, &["-c", "init.defaultBranch=main", "init", "-q", "."]);
    write(&root.join("staged.txt"), "s\n");
    git(&root, &["add", "staged.txt"]);

    let repo = open(&root);

    // HEAD is unborn. §8.2: the index is a GC root, so this is above Untracked,
    // but it is emphatically not on a remote.
    assert_eq!(
        repo.recoverability(Path::new("staged.txt"))
            .expect("classify"),
        RecoverabilityClass::TrackedUnpushed
    );
}

#[test]
fn untracked_file_is_untracked() {
    let td = TempDir::new("untracked");
    let root = td.path().join("repo");
    repo_with_commit(&root);
    write(&root.join("scratch.txt"), "not added\n");

    let repo = open(&root);

    // §8.1: nothing recovers this. It is the class we may never auto-delete.
    assert_eq!(
        repo.recoverability(Path::new("scratch.txt"))
            .expect("classify"),
        RecoverabilityClass::Untracked
    );
}

#[test]
fn ignored_file_is_ignored() {
    let td = TempDir::new("ignored");
    let root = td.path().join("repo");
    repo_with_commit(&root);
    write(&root.join(".gitignore"), "build/\n*.log\n");
    write(&root.join("build/out.bin"), "binary\n");
    write(&root.join("run.log"), "log\n");

    let repo = open(&root);

    assert_eq!(
        repo.recoverability(Path::new("build/out.bin"))
            .expect("classify"),
        RecoverabilityClass::Ignored
    );
    assert_eq!(
        repo.recoverability(Path::new("run.log")).expect("classify"),
        RecoverabilityClass::Ignored
    );
}

/// **The Magento case (§6.16, §6.17).** `.gitignore` excludes an entire
/// directory and then bang-negates specific paths back in. Ignore status is a
/// property of a *file*, never of the directory above it: a classifier that
/// answers per-directory deletes checked-in `.htaccess` files, `.gitkeep`
/// placeholders and editor configuration that no `git checkout` will restore,
/// because they were never tracked in the first place.
#[test]
fn bang_negated_file_inside_an_ignored_directory_is_untracked_not_ignored() {
    let td = TempDir::new("negation");
    let root = td.path().join("repo");
    repo_with_commit(&root);
    write(
        &root.join(".gitignore"),
        "/media/*\n!/media/customer\n!/media/customer/.htaccess\n",
    );
    write(&root.join("media/customer/.htaccess"), "deny from all\n");
    write(&root.join("media/cache/blob.bin"), "regenerable\n");

    let repo = open(&root);

    // Re-included by the negation: not ignored, not tracked => zero recovery.
    assert_eq!(
        repo.recoverability(Path::new("media/customer/.htaccess"))
            .expect("classify"),
        RecoverabilityClass::Untracked,
        "a bang-negated path must not inherit its parent directory's ignore status"
    );
    // Its sibling under the same ignored parent is genuinely ignored. The two
    // answers differing is the whole point: the decision is per file.
    assert_eq!(
        repo.recoverability(Path::new("media/cache/blob.bin"))
            .expect("classify"),
        RecoverabilityClass::Ignored
    );
}

#[test]
fn glob_metacharacters_in_a_filename_are_matched_literally() {
    let td = TempDir::new("glob");
    let root = td.path().join("repo");
    repo_with_commit(&root);
    write(&root.join("report[1].txt"), "untracked\n");

    let repo = open(&root);

    // If the path reached git as a pathspec pattern instead of a literal name,
    // `report[1].txt` could match some *other* tracked file and be misread as
    // recoverable.
    assert_eq!(
        repo.recoverability(Path::new("report[1].txt"))
            .expect("classify"),
        RecoverabilityClass::Untracked
    );
    assert!(!repo
        .is_tracked(Path::new("report[1].txt"))
        .expect("is_tracked"));
}

#[test]
fn absolute_and_repo_relative_paths_classify_identically() {
    let td = TempDir::new("abs");
    let root = td.path().join("repo");
    repo_with_commit(&root);

    let repo = open(&root);

    assert_eq!(
        repo.recoverability(&root.join("a.txt"))
            .expect("classify abs"),
        repo.recoverability(Path::new("a.txt"))
            .expect("classify rel")
    );
}

#[test]
fn a_path_outside_the_repository_is_refused() {
    let td = TempDir::new("outside");
    let root = td.path().join("repo");
    repo_with_commit(&root);
    let stranger = td.path().join("elsewhere.txt");
    write(&stranger, "not ours\n");

    let repo = open(&root);

    // §9.3 Gate 0c: a candidate whose real path is not a repo descendant is a
    // structural refusal, not a classification.
    repo.recoverability(&stranger)
        .expect_err("paths outside the working tree must be refused, not classified");
}

#[test]
fn the_working_tree_root_itself_is_not_a_candidate() {
    let td = TempDir::new("rootpath");
    let root = td.path().join("repo");
    repo_with_commit(&root);

    let repo = open(&root);

    // The root reduces to the empty pathspec, and `git ls-files -- :(literal)`
    // with an empty path matches *every tracked file* (verified on git 2.50.1).
    // Answering "tracked" here would tell a caller that deleting the entire
    // working tree is recoverable — the worst possible answer this type can
    // give, so it must be a refusal.
    repo.recoverability(&root)
        .expect_err("the working tree root must not classify as a recoverable file");
    repo.recoverability(Path::new(""))
        .expect_err("the empty path must not classify as a recoverable file");
}

// ---------------------------------------------------------------------------
// is_tracked
// ---------------------------------------------------------------------------

#[test]
fn is_tracked_distinguishes_index_membership() {
    let td = TempDir::new("tracked");
    let root = td.path().join("repo");
    repo_with_commit(&root);
    write(&root.join(".gitignore"), "*.log\n");
    write(&root.join("run.log"), "log\n");
    write(&root.join("scratch.txt"), "s\n");

    let repo = open(&root);

    assert!(repo.is_tracked(Path::new("a.txt")).expect("tracked"));
    assert!(!repo
        .is_tracked(Path::new("scratch.txt"))
        .expect("untracked"));
    assert!(!repo.is_tracked(Path::new("run.log")).expect("ignored"));
    assert!(!repo
        .is_tracked(Path::new("never-existed.txt"))
        .expect("absent"));
}

// ---------------------------------------------------------------------------
// is_shallow (§6.19: shallow clones are the CI default and void history signals)
// ---------------------------------------------------------------------------

#[test]
fn is_shallow_is_false_for_an_ordinary_clone() {
    let td = TempDir::new("full");
    let (_bare, work) = repo_with_remote(&td);

    assert!(!open(&work).is_shallow().expect("is_shallow"));
}

#[test]
fn is_shallow_detects_a_depth_one_clone() {
    let td = TempDir::new("shallow");
    let (bare, work) = repo_with_remote(&td);
    write(&work.join("b.txt"), "beta\n");
    git(&work, &["add", "-A"]);
    commit(&work, "second");
    git(&work, &["push", "-q", "origin", "main"]);
    // `--depth` is ignored for plain local clones; file:// forces the real
    // protocol, which is what CI actually does.
    let url = format!("file://{}", bare.display());
    let clone = td.path().join("shallowclone");
    git(
        td.path(),
        &["clone", "-q", "--depth", "1", &url, "shallowclone"],
    );
    assert!(
        clone.join(".git/shallow").exists(),
        "fixture is not shallow"
    );

    assert!(open(&clone).is_shallow().expect("is_shallow"));
}

#[test]
fn is_shallow_detects_a_partial_clone() {
    let td = TempDir::new("partial");
    let (bare, _work) = repo_with_remote(&td);
    git(&bare, &["config", "uploadpack.allowfilter", "true"]);
    let url = format!("file://{}", bare.display());
    let clone = td.path().join("partialclone");
    git(
        td.path(),
        &["clone", "-q", "--filter=blob:none", &url, "partialclone"],
    );
    assert_eq!(
        git(&clone, &["config", "--get", "remote.origin.promisor"]),
        "true",
        "fixture is not a partial clone"
    );

    // A blobless clone has full history but no blobs: "was this content ever
    // used" is as unanswerable as it is in a shallow clone (§6.19), so it must
    // trip the same abstention.
    assert!(open(&clone).is_shallow().expect("is_shallow"));
}

// ---------------------------------------------------------------------------
// blob_sha
// ---------------------------------------------------------------------------

#[test]
fn blob_sha_matches_git_hash_object() {
    let td = TempDir::new("blob");
    let root = td.path().join("repo");
    repo_with_commit(&root);

    let repo = open(&root);

    // Verified against git's own hasher, not against our own second opinion.
    let expected = git(&root, &["hash-object", "a.txt"]);
    assert_eq!(
        repo.blob_sha(Path::new("a.txt")).expect("blob_sha"),
        Some(expected)
    );
}

#[test]
fn blob_sha_is_none_for_paths_absent_from_head() {
    let td = TempDir::new("blob-none");
    let root = td.path().join("repo");
    repo_with_commit(&root);
    write(&root.join(".gitignore"), "*.log\n");
    write(&root.join("run.log"), "log\n");
    write(&root.join("scratch.txt"), "s\n");
    write(&root.join("sub/nested.txt"), "n\n");
    git(&root, &["add", "sub/nested.txt"]);
    commit(&root, "nested");

    let repo = open(&root);

    assert_eq!(
        repo.blob_sha(Path::new("scratch.txt")).expect("untracked"),
        None
    );
    assert_eq!(repo.blob_sha(Path::new("run.log")).expect("ignored"), None);
    assert_eq!(repo.blob_sha(Path::new("gone.txt")).expect("absent"), None);
    // A directory has a tree object, not a blob. Returning that tree SHA would
    // hand a fingerprint (§9.4) an identity it can never match.
    assert_eq!(repo.blob_sha(Path::new("sub")).expect("directory"), None);
}

#[test]
fn blob_sha_is_none_in_a_repository_without_commits() {
    let td = TempDir::new("blob-unborn");
    let root = td.path().join("repo");
    fs::create_dir_all(&root).expect("create repo dir");
    git(&root, &["-c", "init.defaultBranch=main", "init", "-q", "."]);
    write(&root.join("staged.txt"), "s\n");
    git(&root, &["add", "staged.txt"]);

    let repo = open(&root);

    // Unborn HEAD is a normal state, not a failure.
    assert_eq!(
        repo.blob_sha(Path::new("staged.txt")).expect("unborn HEAD"),
        None
    );
}

// ---------------------------------------------------------------------------
// init / add_all / commit — fixture construction for the E2 suite (§10)
// ---------------------------------------------------------------------------

#[test]
fn init_add_all_commit_produces_a_real_tracked_history() {
    let td = TempDir::new("init");
    let root = td.path().join("fixture");
    fs::create_dir_all(&root).expect("create fixture dir");
    write(&root.join("pkg/mod.py"), "def live():\n    pass\n");

    let repo = Repo::init(&root).expect("init");
    repo.add_all().expect("add_all");
    repo.commit("fixture").expect("commit");

    assert_eq!(repo.root(), root.as_path());
    assert!(repo
        .is_tracked(Path::new("pkg/mod.py"))
        .expect("is_tracked"));
    assert_eq!(
        repo.recoverability(Path::new("pkg/mod.py"))
            .expect("classify"),
        RecoverabilityClass::TrackedUnpushed
    );
    assert_eq!(
        repo.blob_sha(Path::new("pkg/mod.py")).expect("blob_sha"),
        Some(git(&root, &["hash-object", "pkg/mod.py"]))
    );
    // One commit, reachable from HEAD.
    assert_eq!(git(&root, &["rev-list", "--count", "HEAD"]), "1");
}

#[test]
fn commit_does_not_require_ambient_git_identity() {
    let td = TempDir::new("identity");
    let root = td.path().join("fixture");
    fs::create_dir_all(&root).expect("create fixture dir");
    write(&root.join("f.txt"), "x\n");

    let repo = Repo::init(&root).expect("init");
    // Simulate a machine with no usable identity: with an empty ident a plain
    // `git commit` dies with "Author identity unknown". E2 fixtures (§10) are
    // built on CI images where no identity is configured at all, so fixture
    // construction has to carry its own.
    git(&root, &["config", "user.name", ""]);
    git(&root, &["config", "user.email", ""]);

    repo.add_all().expect("add_all");
    repo.commit("fixture").expect("commit");

    assert_eq!(git(&root, &["rev-list", "--count", "HEAD"]), "1");
}

#[test]
fn add_all_stages_ignored_files_only_when_git_would() {
    let td = TempDir::new("addall");
    let root = td.path().join("fixture");
    fs::create_dir_all(&root).expect("create fixture dir");
    write(&root.join(".gitignore"), "*.log\n");
    write(&root.join("keep.txt"), "k\n");
    write(&root.join("noise.log"), "n\n");

    let repo = Repo::init(&root).expect("init");
    repo.add_all().expect("add_all");

    assert!(repo.is_tracked(Path::new("keep.txt")).expect("is_tracked"));
    // §8.2 makes `git add -f` a deliberate rung promotion. Fixture construction
    // must not perform it by accident, or every mutant would silently start
    // life one rung safer than the repository it models.
    assert!(!repo.is_tracked(Path::new("noise.log")).expect("is_tracked"));
}
