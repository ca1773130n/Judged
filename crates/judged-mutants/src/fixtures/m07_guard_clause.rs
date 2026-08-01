//! Class 7 — a guard clause with no observable effect *(debloat Issue 3)*.
//!
//! **The mechanism.** `descendable` in `src/lib.rs` plans a recursive walk over
//! a directory listing and skips any entry for which
//! `dot_entry::is_self_or_parent_link` is true. `readdir` returns the self and
//! parent links on every POSIX filesystem; a test fixture never does. So on
//! every input the suite supplies, the guard is a branch that is taken zero
//! times and changes nothing.
//!
//! **Why every other signal misses it.** This is the class the debloating study
//! called out as fundamentally unsound to reason about dynamically. §3.4
//! Issue 3: Blade removed exactly this guard from `rm-8.4`'s `fts_build`
//! (gnulib), and running the test suite against the debloated program made the
//! broken traversal **delete the container's `/bin`**, crashing the debloating
//! process itself. The paper's conclusion is the sharpest line in the corpus:
//! *"Such guard logic has no observable effect under normal conditions as its
//! failure modes are environment-dependent which are unlikely to be covered by
//! any practical test suite… a removal can silently pass all tests while
//! introducing catastrophic behavior."*
//!
//! **What is supposed to catch it.** Only the static call edge, plus a refusal
//! to treat "removing it changed no test outcome" as evidence of anything.
//!
//! **This fixture is inert by construction.** It takes an in-memory listing and
//! returns a list of names: there is no filesystem call anywhere in it, so no
//! mutation of it can delete anything. The shape is worth reproducing; the
//! blast radius is not. One of the tests below enforces that inertness, so it
//! cannot be lost in a later edit.

use std::path::{Path, PathBuf};

use judged_core::git::Repo;
use judged_core::{Error, Result};

use crate::mutant::{Ecosystem, GroundTruth, Mutant};

/// The hardest class to argue with, because under normal conditions the
/// guard genuinely does nothing. That is what a guard is.
pub struct GuardClause;

/// Repo-relative path of the artifact that is alive and looks dead.
const LIVE: &str = "src/dot_entry.rs";

/// The symbol inside [`LIVE`] whose removal no test can observe.
const LIVE_SYMBOL: &str = "is_self_or_parent_link";

/// The one file that calls [`LIVE_SYMBOL`].
const MECHANISM: &str = "src/lib.rs";

/// The parent-link literal, as it appears in source. It must exist in exactly
/// one place in the fixture — inside the guard — because a test input
/// containing it would make the guard observable and dissolve the mutant.
///
/// `cfg(test)`, because it names an invariant rather than any file's contents.
#[cfg(test)]
const PARENT_LINK_LITERAL: &str = "\"..\"";

/// Files written into the mutant repository, as `(repo-relative path, body)`.
///
/// Every body here is pure data-in, data-out. See the module docs: the class
/// derives from a debloated `rm` that deleted `/bin`, so the fixture models the
/// shape of that guard and is structurally incapable of the consequence.
const FILES: &[(&str, &str)] = &[
    (
        "Cargo.toml",
        r#"[package]
name = "m07-walkplan"
version = "0.1.0"
edition = "2021"
publish = false

[dependencies]
"#,
    ),
    (
        MECHANISM,
        r#"//! Plan a recursive walk from a directory listing.
//!
//! The input is an in-memory listing and the output is a list of names. This
//! crate performs no filesystem operation of any kind, deliberately.

mod dot_entry;

/// One directory's worth of names, as `readdir` would hand them over.
pub struct Listing<'a> {
    pub dir: &'a str,
    pub entries: &'a [&'a str],
}

/// The children a recursive walk should descend into, in listing order.
pub fn descendable(listing: &Listing<'_>) -> Vec<String> {
    let mut planned = Vec::new();
    for entry in listing.entries {
        // Every POSIX directory listing begins with the self link and the
        // parent link. Without this line the plan descends into its own
        // parent -- an unbounded ascent out of the tree it was asked about.
        // No practical test suite supplies a listing containing them, so
        // removing the guard is invisible to the tests and catastrophic in
        // production: a removal can silently pass all tests while introducing
        // catastrophic behaviour.
        if dot_entry::is_self_or_parent_link(entry) {
            continue;
        }
        planned.push(format!("{}/{}", listing.dir, entry));
    }
    planned
}
"#,
    ),
    (
        LIVE,
        r#"//! LIVE. The guard, alone in its own module -- which is exactly how it comes
//! to be deleted: a one-file diff that turns no test red.

/// Whether `name` is the self link or the parent link that every directory
/// listing contains.
pub(crate) fn is_self_or_parent_link(name: &str) -> bool {
    name == "." || name == ".."
}
"#,
    ),
    (
        "tests/walk.rs",
        r#"//! The whole test suite. Every listing here is hand-written, and
//! hand-written listings never contain the self or parent link, so this suite
//! passes identically with the guard and without it. That is the finding, not
//! a gap in the fixture.

use m07_walkplan::{descendable, Listing};

#[test]
fn plans_a_descent_into_every_child() {
    let listing = Listing {
        dir: "var/log",
        entries: &["nginx", "postgres", "syslog"],
    };

    assert_eq!(
        descendable(&listing),
        vec!["var/log/nginx", "var/log/postgres", "var/log/syslog"]
    );
}

#[test]
fn an_empty_directory_plans_nothing() {
    let listing = Listing {
        dir: "srv",
        entries: &[],
    };

    assert!(descendable(&listing).is_empty());
}
"#,
    ),
    (
        "src/orphan_glob_cache.rs",
        r#"//! DEAD DECOY. The glob cache was dropped when patterns moved to the
//! caller; no `mod` declares this file, so it is not compiled and nothing
//! names it.

pub fn cache_key(pattern: &str, depth: usize) -> String {
    format!("{pattern}#{depth}")
}
"#,
    ),
    (
        "src/unused_depth_limit.rs",
        r#"//! DEAD DECOY. A second one on purpose: decoy recall is a rate, and one
//! decoy cannot tell a tool that reasoned from a tool that guessed once.

pub const MAX_DEPTH: usize = 64;
"#,
    ),
];

impl GuardClause {
    /// Repo-relative paths of the genuinely-dead files planted here. Neither
    /// is declared with `mod`, so neither is even compiled.
    const DECOYS: [&'static str; 2] = ["src/orphan_glob_cache.rs", "src/unused_depth_limit.rs"];

    /// The symbol each decoy defines, index-aligned with [`Self::DECOYS`].
    /// Without these a symbol-level analyzer scores zero decoys here and reads
    /// as having found nothing (see `GroundTruth::decoy_dead_symbols`).
    const DECOY_SYMBOLS: [&'static str; 2] = ["cache_key", "MAX_DEPTH"];

    /// Filesystem primitives that must not appear anywhere in the fixture.
    ///
    /// The research provenance of this class is a debloated `rm` that deleted
    /// `/bin` while its own test suite ran. A fixture modelling that shape must
    /// be incapable of touching a filesystem, and "incapable" has to be checked
    /// rather than intended.
    #[cfg(test)]
    const FORBIDDEN_EFFECTS: [&'static str; 5] =
        ["std::fs", "remove_file", "remove_dir", "read_dir", "unlink"];
}

impl Mutant for GuardClause {
    fn id(&self) -> &str {
        "m07"
    }
    fn ecosystem(&self) -> Ecosystem {
        Ecosystem::Rust
    }
    fn mechanism(&self) -> &str {
        "bounds/precondition guard with no effect until the input is hostile"
    }
    fn research_ref(&self) -> &str {
        "§10 E2 class 7"
    }
    fn materialize(&self, dir: &Path) -> Result<GroundTruth> {
        let repo = Repo::init(dir)?;
        for (relative, body) in FILES {
            let path = repo.root().join(relative);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(|source| Error::Io {
                    path: parent.to_path_buf(),
                    source,
                })?;
            }
            std::fs::write(&path, body).map_err(|source| Error::Io { path, source })?;
        }
        repo.add_all()?;
        repo.commit("m07: walk planner whose dot-entry guard no test can observe")?;

        Ok(GroundTruth {
            // Repo-relative, because the runner keys ground truth and SUT
            // claims on the same repo-relative rendering and the fixture's own
            // canonicalized root is not the path the runner holds.
            live_paths: vec![PathBuf::from(LIVE)],
            // The file is live because the symbol in it is. §3.4 Issue 3 is
            // about the symbol: the whole point is that deleting one `if` is
            // as catastrophic and as invisible as deleting the module.
            live_symbols: vec![LIVE_SYMBOL.to_string()],
            decoy_dead_paths: Self::DECOYS.iter().copied().map(PathBuf::from).collect(),
            decoy_dead_symbols: Self::DECOY_SYMBOLS
                .iter()
                .map(|symbol| (*symbol).to_string())
                .collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::support;

    #[test]
    fn m07_is_a_real_git_repository_whose_live_artifact_is_committed() {
        let (_dir, repo, _truth) = support::materialize(&GuardClause);
        support::assert_committed(&repo, &[LIVE]);
    }

    #[test]
    fn m07_ground_truth_names_files_that_are_really_there() {
        let (_dir, repo, truth) = support::materialize(&GuardClause);

        assert_eq!(truth.live_paths, vec![Path::new(LIVE).to_path_buf()]);
        assert_eq!(truth.live_symbols, vec![LIVE_SYMBOL.to_string()]);
        assert_eq!(truth.decoy_dead_paths.len(), GuardClause::DECOYS.len());

        support::assert_ground_truth_is_on_disk(&repo, &truth);
    }

    #[test]
    fn m07_the_guard_is_called_from_exactly_one_place() {
        let (_dir, repo, _truth) = support::materialize(&GuardClause);

        assert_eq!(
            support::files_mentioning(repo.root(), LIVE_SYMBOL),
            vec![LIVE.to_string(), MECHANISM.to_string()],
            "only the definition and the one traversal may name the guard"
        );
    }

    #[test]
    fn m07_the_guard_module_is_never_named_by_its_filename() {
        let (_dir, repo, _truth) = support::materialize(&GuardClause);
        let live = Path::new(LIVE);
        let basename = live
            .file_name()
            .and_then(|n| n.to_str())
            .expect("LIVE has a UTF-8 basename");
        let stem = live
            .file_stem()
            .and_then(|s| s.to_str())
            .expect("LIVE has a UTF-8 stem");

        // Rust links a module with `mod dot_entry;`, never with a filename, so
        // the basename occurs nowhere in the repository. A cleaner that greps
        // for `dot_entry.rs` before deleting it finds nothing to stop it —
        // which is the point of the class, and has to be true for the mutant
        // to be measuring anything.
        assert!(
            support::files_mentioning(repo.root(), basename).is_empty(),
            "{basename} must be spelled nowhere; the only link is a `mod` declaration"
        );

        // One mechanism, per §10 E2: exactly one file names the module. If a
        // second did, a rescue here would not tell us which signal did it.
        assert_eq!(
            support::files_mentioning(repo.root(), stem),
            vec![MECHANISM.to_string()],
            "only the traversal may name the guard's module"
        );
    }

    #[test]
    fn m07_no_input_in_the_repository_makes_the_guard_fire() {
        let (_dir, repo, _truth) = support::materialize(&GuardClause);

        // The guard is only a *guard* if nothing the suite feeds it is hostile.
        // If a test input ever contains the parent link, the guard becomes
        // observable, the suite starts protecting it, and the mutant stops
        // encoding §3.4 Issue 3.
        assert_eq!(
            support::files_mentioning(repo.root(), PARENT_LINK_LITERAL),
            vec![LIVE.to_string()],
            "the parent link may appear only inside the guard itself"
        );
    }

    #[test]
    fn m07_the_fixture_cannot_touch_a_filesystem() {
        let (_dir, repo, _truth) = support::materialize(&GuardClause);

        // Enforced, not merely intended: the incident this class encodes is a
        // debloated traversal that deleted /bin while its own tests ran.
        for effect in GuardClause::FORBIDDEN_EFFECTS {
            assert!(
                support::files_mentioning(repo.root(), effect).is_empty(),
                "{effect} appears in the fixture; the traversal model must be inert"
            );
        }
    }

    #[test]
    fn m07_decoys_are_named_nowhere_at_all() {
        let (_dir, repo, truth) = support::materialize(&GuardClause);
        support::assert_decoys_are_unreferenced(&repo, &truth);
    }
}
