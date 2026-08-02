//! Gate 0a against real links (§9.3, §6.13, §6.16).
//!
//! Every fixture here is a symlink actually created on disk, because 0a's whole
//! subject is what `lstat` answers and that cannot be reasoned about — the
//! trailing-separator case in particular returns the *target's* metadata with
//! `is_symlink = false`, which no amount of reading the clause reveals.

#![cfg(unix)]

use std::os::unix::fs::symlink;
use std::path::PathBuf;

use judged_core::gate0a::{Condition, Gate0a, StorePresence, Verdict};
use judged_core::git::Repo;

struct Fixture {
    _dir: tempfile::TempDir,
    root: PathBuf,
    repo: Repo,
}

fn fixture(label: &str) -> Fixture {
    let dir = tempfile::Builder::new()
        .prefix(&format!("judged-0a-{label}-"))
        .tempdir()
        .expect("scratch");
    // Canonical, because git reports canonical paths and 0a bounds its ancestor
    // walk at the root — on macOS the scratch root is reached through
    // /var -> /private/var, and mixing the two spellings makes every candidate
    // look like it is outside the tree.
    let root = dir.path().canonicalize().expect("canonical");
    let repo = Repo::init(&root).expect("init");
    std::fs::write(root.join("README.md"), "x\n").expect("write");
    repo.add_all().expect("add");
    repo.commit("initial").expect("commit");
    Fixture {
        _dir: dir,
        root,
        repo,
    }
}

fn conditions(verdict: &Verdict) -> Vec<Condition> {
    verdict.findings().iter().map(|f| f.condition).collect()
}

/// A resolving link is never a candidate, and ordinary content is untouched.
#[test]
fn a_resolving_link_is_refused_and_ordinary_content_is_not() {
    let f = fixture("resolving");
    std::fs::create_dir_all(f.root.join("target")).expect("mkdir");
    std::fs::write(f.root.join("target/file.txt"), "x\n").expect("write");
    symlink(f.root.join("target"), f.root.join("link")).expect("symlink");

    let gate = Gate0a::build(&f.repo);
    assert_eq!(
        conditions(&gate.judge(&f.root.join("link"))),
        vec![Condition::ResolvingLink]
    );

    // The half that keeps this from being a constant function.
    for ordinary in ["README.md", "target", "target/file.txt"] {
        assert!(
            gate.judge(&f.root.join(ordinary)).permits_action(),
            "{ordinary} involves no link and 0a must have nothing to say"
        );
    }
}

/// §6.16, measured: `rm -rf LINK/` deletes the target's contents. The spelling
/// itself is the hazard, and `lstat` cannot see it.
#[test]
fn a_link_spelled_with_a_trailing_separator_is_refused_for_that_reason_too() {
    let f = fixture("separator");
    std::fs::create_dir_all(f.root.join("target")).expect("mkdir");
    symlink(f.root.join("target"), f.root.join("link")).expect("symlink");
    let gate = Gate0a::build(&f.repo);

    // The premise, restated as an assertion so the test documents why the strip
    // exists: lstat on the slashed spelling answers about the TARGET.
    let slashed = format!("{}/", f.root.join("link").display());
    assert!(
        !std::fs::symlink_metadata(&slashed)
            .expect("lstat")
            .file_type()
            .is_symlink(),
        "lstat on a trailing-separator spelling reports the target, not the link"
    );

    let verdict = gate.judge(&PathBuf::from(&slashed));
    let fired = conditions(&verdict);
    assert!(
        fired.contains(&Condition::TrailingSeparatorOnLink),
        "the spelling is the §6.16 hazard: {fired:?}"
    );
    assert!(
        fired.contains(&Condition::ResolvingLink),
        "and it is still a link, which the strip is what let us see: {fired:?}"
    );

    // An ordinary directory spelled the same way is not refused — the finding is
    // about links, not about slashes.
    let dir_slashed = format!("{}/", f.root.join("target").display());
    assert!(gate.judge(&PathBuf::from(dir_slashed)).permits_action());
}

/// §6.13: after `git annex drop` a broken symlink is the NORMAL state, and
/// reporting it deletes the only pointer to content `git annex get` would
/// restore.
#[test]
fn a_dangling_link_is_reportable_only_when_no_store_exists() {
    let without = fixture("nostore");
    symlink("nowhere", without.root.join("broken")).expect("symlink");
    let gate = Gate0a::build(&without.repo);
    assert_eq!(gate.store(), &StorePresence::Absent);
    assert!(
        gate.judge(&without.root.join("broken")).permits_action(),
        "with no store, a broken link is reportable — that is what the clause permits"
    );

    // The same link, in a repository that has an annex.
    let with = fixture("annexed");
    symlink("nowhere", with.root.join("broken")).expect("symlink");
    let git_dir = with.repo.common_dir().expect("common dir");
    std::fs::create_dir_all(git_dir.join("annex/objects")).expect("annex");

    let gate = Gate0a::build(&with.repo);
    assert_eq!(gate.store(), &StorePresence::Present("git-annex"));
    assert_eq!(
        conditions(&gate.judge(&with.root.join("broken"))),
        vec![Condition::DanglingLinkWithStore],
        "an annexed repository's broken links are un-fetched content, not garbage"
    );
}

/// A DVC repository, whose default cache is not symlinks at all — which is why
/// the repository-level probe is the load-bearing half.
#[test]
fn a_dvc_repository_also_suppresses_reporting() {
    let f = fixture("dvc");
    std::fs::create_dir_all(f.root.join(".dvc/cache")).expect("mkdir");
    symlink("nowhere", f.root.join("broken")).expect("symlink");

    let gate = Gate0a::build(&f.repo);
    assert_eq!(gate.store(), &StorePresence::Present("DVC"));
    assert!(!gate.judge(&f.root.join("broken")).permits_action());
}

/// Reaching a candidate through a link is traversing one, which the clause
/// forbids outright.
#[test]
fn a_candidate_under_a_linked_ancestor_is_refused() {
    let f = fixture("ancestor");
    std::fs::create_dir_all(f.root.join("real/nested")).expect("mkdir");
    std::fs::write(f.root.join("real/nested/file.txt"), "x\n").expect("write");
    symlink(f.root.join("real"), f.root.join("via")).expect("symlink");

    let gate = Gate0a::build(&f.repo);
    let verdict = gate.judge(&f.root.join("via/nested/file.txt"));
    assert_eq!(conditions(&verdict), vec![Condition::LinkedAncestor]);

    // The same file by its real path is ordinary.
    assert!(gate
        .judge(&f.root.join("real/nested/file.txt"))
        .permits_action());
}

/// §6.20: a probe that could not run does not license reporting.
#[test]
fn a_store_probe_that_failed_is_treated_as_a_store_being_present() {
    let f = fixture("vanishing");
    symlink("nowhere", f.root.join("broken")).expect("symlink");
    let broken = f.root.join("broken");
    let repo = Repo::discover(&f.root).expect("repo");
    // Remove the working tree underneath the handle, so every git probe fails.
    let dir = f._dir;
    let root = f.root.clone();
    std::fs::rename(root.join(".git"), root.join(".git-moved")).expect("hide the git dir");

    let gate = Gate0a::build(&repo);
    assert!(
        matches!(gate.store(), StorePresence::Unreadable(_)),
        "{:?}",
        gate.store()
    );
    assert!(
        !gate.judge(&broken).permits_action(),
        "a probe that failed has not established that no annex exists"
    );
    drop(dir);
}

/// A relative candidate is not guessed at.
#[test]
fn a_relative_candidate_is_refused_rather_than_resolved_against_a_guess() {
    let f = fixture("relative");
    let gate = Gate0a::build(&f.repo);
    assert!(!gate.judge(&PathBuf::from("README.md")).permits_action());
}
