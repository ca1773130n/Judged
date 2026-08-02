//! Gate 0e against the layouts §9.3's nine words do not survive (§6.13, §8.3).
//!
//! Two properties, and the second is what stops the first being worthless:
//! it refuses everything inside the git directory *however* that directory is
//! reached, and it refuses nothing in an ordinary tree.

use std::path::{Path, PathBuf};

use judged_core::gate0e::{Gate0e, Region, Relation, Verdict};
use judged_core::git::Repo;

struct Fixture {
    _dir: tempfile::TempDir,
    root: PathBuf,
}

fn repo(label: &str) -> (Fixture, Repo) {
    let dir = tempfile::Builder::new()
        .prefix(&format!("judged-0e-{label}-"))
        .tempdir()
        .expect("scratch");
    let root = dir.path().to_path_buf();
    let repo = Repo::init(&root).expect("init");
    std::fs::write(root.join("README.md"), "x\n").expect("write");
    repo.add_all().expect("add");
    repo.commit("initial").expect("commit");
    (Fixture { _dir: dir, root }, repo)
}

fn regions(gate: &Gate0e) -> Vec<Region> {
    gate.regions().iter().map(|(r, _)| *r).collect()
}

/// A plain repository: every region located, everything inside it refused, and
/// ordinary content untouched.
#[test]
fn the_git_directory_is_refused_and_ordinary_content_is_not() {
    let (fixture, repo) = repo("plain");
    let gate = Gate0e::build(&repo);
    assert_eq!(
        regions(&gate),
        vec![
            Region::GitDir,
            Region::CommonDir,
            Region::LfsStore,
            Region::AnnexStore
        ]
    );

    let git_dir = repo.absolute_git_dir().expect("git dir");
    for inside in [
        git_dir.clone(),
        git_dir.join("objects/pack"),
        git_dir.join("HEAD"),
    ] {
        let verdict = gate.judge(&inside);
        assert!(
            !verdict.permits_action(),
            "{} must be refused",
            inside.display()
        );
    }

    // The half that keeps this from being a constant function.
    for ordinary in ["README.md", "src/lib.rs", "docs/design.md"] {
        let verdict = gate.judge(&fixture.root.join(ordinary));
        assert!(
            verdict.permits_action(),
            "{ordinary} is ordinary content and 0e must have nothing to say about it: \
             {verdict:?}"
        );
    }
}

/// §8.3's shape: the ancestor. `rm -rf repo/` takes the object database with it,
/// and the clause reads as though the hazard were only a path *inside* `.git`.
#[test]
fn a_candidate_that_contains_the_git_directory_is_refused() {
    let (fixture, repo) = repo("ancestor");
    let gate = Gate0e::build(&repo);

    let verdict = gate.judge(&fixture.root);
    assert!(!verdict.permits_action());
    assert!(
        verdict
            .findings()
            .iter()
            .any(|f| f.relation == Relation::Contains),
        "the working tree root contains the git directory: {verdict:?}"
    );
}

/// A linked worktree, where `.git` is a FILE and the two git directories differ.
/// A prefix match on `.git/` protects nothing here.
#[test]
fn both_git_directories_are_protected_in_a_linked_worktree() {
    let (fixture, _main) = repo("worktree");
    let linked = fixture.root.parent().expect("parent").join("judged-0e-wt");
    let added = std::process::Command::new("git")
        .args(["worktree", "add", "-q"])
        .arg(&linked)
        .arg("HEAD")
        .current_dir(&fixture.root)
        .output()
        .expect("spawn git");
    assert!(
        added.status.success(),
        "could not build the fixture, so nothing was exercised: {}",
        String::from_utf8_lossy(&added.stderr)
    );
    assert!(linked.join(".git").is_file(), "the premise: .git is a file");

    let worktree = Repo::discover(&linked).expect("a repository");
    let gate = Gate0e::build(&worktree);

    let own = worktree.absolute_git_dir().expect("own git dir");
    let common = worktree.common_dir().expect("common dir");
    assert_ne!(own, common, "the premise: the two directories differ here");

    for protected in [&own, &common] {
        assert!(
            !gate.judge(&protected.join("objects")).permits_action(),
            "{} must be protected",
            protected.display()
        );
    }
    assert!(
        gate.judge(&linked.join("README.md")).permits_action(),
        "and the worktree's own content is still ordinary"
    );

    std::process::Command::new("git")
        .args(["worktree", "remove", "--force"])
        .arg(&linked)
        .current_dir(&fixture.root)
        .output()
        .expect("cleanup");
}

/// §6.13: `lfs.storage` moves the object store, and an absolute value moves it
/// outside the git directory entirely — where a `.git/` prefix match cannot
/// reach it.
#[test]
fn an_lfs_store_relocated_outside_the_git_directory_is_still_protected() {
    let (fixture, repo) = repo("lfs");
    let elsewhere = fixture
        .root
        .parent()
        .expect("parent")
        .join("judged-0e-lfs-store");
    std::fs::create_dir_all(elsewhere.join("objects")).expect("mkdir");

    let set = std::process::Command::new("git")
        .args(["config", "lfs.storage"])
        .arg(&elsewhere)
        .current_dir(&fixture.root)
        .output()
        .expect("spawn git");
    assert!(
        set.status.success(),
        "{}",
        String::from_utf8_lossy(&set.stderr)
    );

    let gate = Gate0e::build(&repo);
    let verdict = gate.judge(&elsewhere.join("objects"));
    assert!(
        !verdict.permits_action(),
        "the store is outside .git and must still be refused: {verdict:?}"
    );
    assert!(
        verdict
            .findings()
            .iter()
            .any(|f| f.region == Region::LfsStore),
        "and named as the LFS store rather than as something else"
    );

    std::fs::remove_dir_all(&elsewhere).ok();
}

/// §6.20 in this gate: a probe that failed refuses, and says which probe.
#[test]
fn a_gate_that_could_not_locate_its_regions_refuses_rather_than_clears() {
    let dir = tempfile::Builder::new()
        .prefix("judged-0e-norepo-")
        .tempdir()
        .expect("scratch");
    // A Repo handle whose working tree has been removed underneath it: every
    // probe now fails, which is the case that must not read as "clear".
    let (fixture, repo) = repo("vanishing");
    drop(fixture);

    let gate = Gate0e::build(&repo);
    let verdict = gate.judge(&dir.path().join("anything"));
    assert!(
        !verdict.permits_action(),
        "a gate that could not locate the git directory has established nothing"
    );
    assert!(matches!(verdict, Verdict::Unreadable(_)), "{verdict:?}");
}

/// A relative candidate is not guessed at.
#[test]
fn a_relative_candidate_is_refused_rather_than_resolved_against_a_guess() {
    let (_fixture, repo) = repo("relative");
    let gate = Gate0e::build(&repo);
    assert!(!gate.judge(Path::new("src/lib.rs")).permits_action());
}
