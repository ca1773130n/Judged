//! Where one repository stops and another begins (§9.3 Gate 0b).
//!
//! §9.3's 0b is one sentence — *"Refuse to descend into any directory containing
//! `.git` (nested repo / submodule)"* — and every walker in this crate
//! implemented it as `name == ".git"`. That is wrong in four ordinary layouts,
//! and it was wrong in seven places at once, because the mistake is not a typo:
//! it is a wrong belief about how git marks a repository, and a wrong belief
//! gets copied.
//!
//! # What `.git` actually looks like
//!
//! Verified against git 2.50.1 rather than taken from the spec:
//!
//! - An ordinary nested clone has `.git` as a **directory**. This is the only
//!   case the old test caught.
//! - A **linked worktree** has `.git` as a regular **file** holding
//!   `gitdir: …` — 63 bytes from `git worktree add`.
//! - A **submodule** checkout, likewise, pointing at
//!   `<super>/.git/modules/<name>`.
//! - A **bare repository** — `vendor/foo.git/`, the shape vendored dependencies
//!   and mirrors take — has no `.git` entry *at all*. Its marker is its own
//!   contents.
//!
//! # Why crossing one is not merely untidy
//!
//! Each walker that crossed a boundary drew a different wrong conclusion, all of
//! them confident: a Tier A root materialized from a nested repository's
//! manifest, a Gate 2 reference "found" in a vendored clone that has nothing to
//! do with this tree, an in-source `#[no_mangle]` root read out of a submodule,
//! Gate 1 content classes judging files another repository owns. And §8.3
//! records the cost of getting the containing directory wrong: the common
//! "complete removal" recipe for a submodule includes
//! `rm -rf $GIT_DIR/modules/<name>`, which destroys that submodule's entire
//! object database — its history and its reflog, local commits included.
//!
//! # An unreadable probe does not mean "not a boundary"
//!
//! [`Boundary::Unreadable`] is its own state and
//! [`Boundary::stops_the_walk`] is true for it. A directory we could not
//! classify might be another repository, and descending into it on the strength
//! of a failed `lstat` is §6.20's inversion — the walk would treat "I could not
//! look" as "there is nothing here". Callers that keep a gap list should record
//! one; the walk stops either way.

use std::fmt;
use std::path::Path;

/// What a directory is, as far as the walk is concerned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Boundary {
    /// Not a repository. Descend.
    None,
    /// A repository boundary of this kind. Do not descend.
    Repository(Kind),
    /// Could not be classified. **Do not descend** — see the module docs.
    Unreadable(String),
}

impl Boundary {
    /// Whether the walk must stop here.
    ///
    /// True for [`Boundary::Repository`] **and** [`Boundary::Unreadable`]. The
    /// two are kept apart in the type because a report should say which, and
    /// folded together here because the walk does the same thing for both.
    pub fn stops_the_walk(&self) -> bool {
        !matches!(self, Boundary::None)
    }

    /// The reason, for a caller with somewhere to record it. `None` when the
    /// walk may proceed.
    pub fn reason(&self) -> Option<String> {
        match self {
            Boundary::None => None,
            Boundary::Repository(kind) => Some(kind.to_string()),
            Boundary::Unreadable(why) => Some(why.clone()),
        }
    }
}

/// Which shape of repository was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// `.git` is a directory: an ordinary nested clone.
    NestedClone,
    /// `.git` is a file holding `gitdir: …`: a linked worktree or a submodule.
    /// The two are indistinguishable without reading the pointer, and nothing
    /// here needs to tell them apart — both are somebody else's repository.
    GitFile,
    /// No `.git` at all, but the directory is itself a git directory: a bare
    /// repository, e.g. a vendored `foo.git/`.
    Bare,
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Kind::NestedClone => "a nested repository (.git is a directory)",
            Kind::GitFile => "a linked worktree or submodule (.git is a file)",
            Kind::Bare => "a bare repository (HEAD, objects/ and refs/ present)",
        })
    }
}

/// Classify `dir` as a repository boundary.
///
/// **Never call this on the scan root.** The working tree root contains `.git`
/// by definition, so 0b read literally refuses every repository; the exemption
/// is structural — a walk starts *at* the root and classifies only what it
/// descends into.
pub fn classify(dir: &Path) -> Boundary {
    let marker = dir.join(".git");
    // `symlink_metadata`, not `metadata`: a `.git` that is a symlink is not
    // something to follow, and §9.3 0a forbids traversing one to find out.
    match std::fs::symlink_metadata(&marker) {
        Ok(meta) if meta.is_dir() => return Boundary::Repository(Kind::NestedClone),
        Ok(_) => return Boundary::Repository(Kind::GitFile),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Boundary::Unreadable(format!(
                "{} could not be classified: {error}",
                marker.display()
            ))
        }
    }
    bare_repository(dir)
}

/// Whether `dir` is itself a git directory, by git's own `is_git_directory()`
/// conjunction.
///
/// `HEAD` a regular file, `objects/` and `refs/` directories, and `HEAD`
/// beginning `ref: ` or holding a 40-character object id. All four, because any
/// one alone is ordinary: plenty of trees have a `refs/` directory that is not a
/// repository.
///
/// This is a **widening** of §9.3 0b, which names only `.git`. A bare repository
/// has no `.git` entry and the clause misses it entirely, while
/// `vendor/foo.git/` is exactly how vendored dependencies and mirrors are
/// checked in. Labelled as a widening rather than presented as the clause.
fn bare_repository(dir: &Path) -> Boundary {
    let head = dir.join("HEAD");
    match std::fs::symlink_metadata(&head) {
        Ok(meta) if meta.is_file() => {}
        Ok(_) => return Boundary::None,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Boundary::None,
        Err(error) => {
            return Boundary::Unreadable(format!(
                "{} could not be classified: {error}",
                head.display()
            ))
        }
    }
    for required in ["objects", "refs"] {
        match std::fs::symlink_metadata(dir.join(required)) {
            Ok(meta) if meta.is_dir() => {}
            Ok(_) | Err(_) => return Boundary::None,
        }
    }

    // Only now read HEAD, and only its first line — a bare repository's HEAD is
    // one short line, and a file that merely happens to be called HEAD could be
    // any size.
    let Ok(contents) = std::fs::read(&head) else {
        return Boundary::Unreadable(format!("{} could not be read", head.display()));
    };
    let first: Vec<u8> = contents.into_iter().take(64).collect();
    let text = String::from_utf8_lossy(&first);
    let line = text.lines().next().unwrap_or("").trim();
    let is_head = line.starts_with("ref: ")
        || (line.len() == 40 && line.bytes().all(|b| b.is_ascii_hexdigit()));

    if is_head {
        Boundary::Repository(Kind::Bare)
    } else {
        Boundary::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch() -> tempfile::TempDir {
        tempfile::Builder::new()
            .prefix("judged-boundary-")
            .tempdir()
            .expect("scratch")
    }

    /// The three `.git` shapes, and the ordinary directory that must be walked.
    #[test]
    fn every_shape_of_git_marker_stops_the_walk_and_nothing_else_does() {
        let dir = scratch();

        let nested = dir.path().join("nested");
        std::fs::create_dir_all(nested.join(".git")).expect("mkdir");
        assert_eq!(classify(&nested), Boundary::Repository(Kind::NestedClone));

        // A linked worktree and a submodule both look like this.
        let linked = dir.path().join("linked");
        std::fs::create_dir_all(&linked).expect("mkdir");
        std::fs::write(linked.join(".git"), "gitdir: /elsewhere/.git/worktrees/w\n")
            .expect("write");
        assert_eq!(classify(&linked), Boundary::Repository(Kind::GitFile));

        let ordinary = dir.path().join("src");
        std::fs::create_dir_all(&ordinary).expect("mkdir");
        assert_eq!(
            classify(&ordinary),
            Boundary::None,
            "an ordinary directory must be walked, or the gate is a constant function"
        );
    }

    /// A bare repository has no `.git` at all — the shape `vendor/foo.git/`
    /// takes, which §9.3 0b misses entirely.
    #[test]
    fn a_bare_repository_is_a_boundary_even_with_no_dot_git() {
        let dir = scratch();
        let bare = dir.path().join("vendor/foo.git");
        std::fs::create_dir_all(bare.join("objects")).expect("mkdir");
        std::fs::create_dir_all(bare.join("refs")).expect("mkdir");
        std::fs::write(bare.join("HEAD"), "ref: refs/heads/main\n").expect("write");

        assert_eq!(classify(&bare), Boundary::Repository(Kind::Bare));

        // And a detached bare repo, whose HEAD is a bare object id.
        std::fs::write(
            bare.join("HEAD"),
            "0123456789abcdef0123456789abcdef01234567\n",
        )
        .expect("write");
        assert_eq!(classify(&bare), Boundary::Repository(Kind::Bare));
    }

    /// All four conjuncts are required, or ordinary trees start refusing.
    #[test]
    fn a_directory_that_merely_resembles_a_git_directory_is_not_one() {
        let dir = scratch();

        // `refs/` and `objects/` without HEAD — an ordinary asset tree.
        let assets = dir.path().join("assets");
        std::fs::create_dir_all(assets.join("objects")).expect("mkdir");
        std::fs::create_dir_all(assets.join("refs")).expect("mkdir");
        assert_eq!(classify(&assets), Boundary::None);

        // A HEAD that is not a git HEAD — e.g. a fixture or a CSV column dump.
        let docs = dir.path().join("docs");
        std::fs::create_dir_all(docs.join("objects")).expect("mkdir");
        std::fs::create_dir_all(docs.join("refs")).expect("mkdir");
        std::fs::write(docs.join("HEAD"), "column,header\n1,2\n").expect("write");
        assert_eq!(classify(&docs), Boundary::None);
    }

    /// §6.20 in this module: a probe that could not read stops the walk, and
    /// says so, rather than being read as "nothing here".
    #[test]
    fn an_unclassifiable_directory_stops_the_walk_rather_than_being_descended_into() {
        let unreadable = Boundary::Unreadable("permission denied".to_string());
        assert!(unreadable.stops_the_walk());
        assert!(unreadable.reason().is_some());

        assert!(!Boundary::None.stops_the_walk());
        assert!(Boundary::None.reason().is_none());
        assert!(Boundary::Repository(Kind::Bare).stops_the_walk());
    }

    /// A `.git` symlink is not followed to decide what it is. §9.3 0a forbids
    /// traversing one, and a link pointing at a directory would otherwise be
    /// classified by its target.
    #[test]
    fn a_symlinked_git_marker_is_a_boundary_without_being_followed() {
        let dir = scratch();
        let target = dir.path().join("real-git");
        std::fs::create_dir_all(&target).expect("mkdir");
        let tree = dir.path().join("tree");
        std::fs::create_dir_all(&tree).expect("mkdir");

        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&target, tree.join(".git")).expect("symlink");
            assert_eq!(
                classify(&tree),
                Boundary::Repository(Kind::GitFile),
                "classified from the link itself, never from what it points at"
            );
        }
    }
}
