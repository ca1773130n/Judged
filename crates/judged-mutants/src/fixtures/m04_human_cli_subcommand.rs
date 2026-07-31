//! Class 4 — a CLI subcommand invoked only by humans.
//!
//! **Mechanism.** `keyctl rotate-signing-key` is registered in a dispatch
//! table in `src/main.rs` and invoked from nowhere in the repository. The only
//! thing that ever supplies the selecting string is a person following
//! `docs/runbooks/signing-key-rotation.md`.
//!
//! **Why every other signal misses it.** A call graph is not the trap here —
//! it does reach the handler through the table. The traps are the two signals a
//! cleaner is most tempted to fuse in:
//!
//! - *Coverage.* Nothing exercises the handler, so a coverage-fused score reads
//!   zero. §6.6 is categorical about why that is worse than noisy: coverage is
//!   **systematically anti-correlated with the value of the code**, and the miss
//!   surfaces only during the emergency the code existed for.
//! - *History.* The subcommand runs once every key-rotation cycle, so nothing
//!   in the recent log or in any observed fleet mentions it either.
//!
//! §6.6 gives the counter-signals, and the fixture plants all three so that the
//! mutant is fair rather than impossible: the `rotate` path lexicon, the
//! `docs/runbooks/` reference, and a top-level entry point with no in-repo
//! caller ("the archetype"). §6.6's rule is that this class is **hard-excluded
//! from any auto-act tier regardless of how many signals agree**, because the
//! signals are correlated through the same cause.

use std::path::Path;

use judged_core::git::Repo;
use judged_core::{Error, Result};

use crate::mutant::{Ecosystem, GroundTruth, Mutant};

/// The root set is unknowable precisely because humans are in it (§0.1).
/// This subcommand is registered, documented, and never called from
/// anywhere inside the repository.
pub struct HumanCliSubcommand;

impl Mutant for HumanCliSubcommand {
    fn id(&self) -> &str {
        "m04"
    }
    fn ecosystem(&self) -> Ecosystem {
        Ecosystem::Rust
    }
    fn mechanism(&self) -> &str {
        "subcommand reachable only when a human types it"
    }
    fn research_ref(&self) -> &str {
        "§10 E2 class 4"
    }
    fn materialize(&self, dir: &Path) -> Result<GroundTruth> {
        let repo = Repo::init(dir)?;
        let root = repo.root().to_path_buf();

        write(
            &root,
            "Cargo.toml",
            "[package]\nname = \"keyctl\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n\
             [dependencies]\n",
        )?;

        // The mechanism. `rotate-signing-key` is spelled here and in the
        // runbook; nothing in the tree ever *passes* it.
        write(
            &root,
            "src/main.rs",
            r#"mod rotate_signing_key;
mod status;

/// Subcommand table. The only thing that can select an entry is argv, and the
/// only thing that writes argv for `rotate-signing-key` is a human with the
/// runbook open.
const COMMANDS: &[(&str, fn() -> i32)] = &[
    ("status", status::run),
    ("rotate-signing-key", rotate_signing_key::run),
];

fn main() {
    let name = std::env::args().nth(1).unwrap_or_else(|| "status".to_string());
    let code = match COMMANDS.iter().find(|(candidate, _)| *candidate == name) {
        Some((_, run)) => run(),
        None => {
            eprintln!("unknown subcommand: {name}");
            2
        }
    };
    std::process::exit(code);
}
"#,
        )?;

        // The routinely-exercised sibling, with the only test in the crate.
        // Its presence is what makes the coverage asymmetry legible: `status`
        // reads as hot, the rotation handler reads as dead.
        write(
            &root,
            "src/status.rs",
            r#"pub fn run() -> i32 {
    println!("signing key: ok");
    0
}

#[cfg(test)]
mod tests {
    #[test]
    fn reports_success() {
        assert_eq!(super::run(), 0);
    }
}
"#,
        )?;

        // THE LIVE ARTIFACT. Registered, documented, never called, never
        // covered.
        write(
            &root,
            "src/rotate_signing_key.rs",
            r#"//! Run during a key-rotation window, roughly twice a year, by hand.

pub fn run() -> i32 {
    let previous = std::fs::read_to_string("etc/signing.key").unwrap_or_default();
    if previous.is_empty() {
        eprintln!("no key to rotate");
        return 1;
    }
    println!("retiring key {}", &previous[..8.min(previous.len())]);
    0
}
"#,
        )?;

        // §6.6's counter-signal, spelled the way a human reads it: the CLI
        // string, not the module path. A tool that only greps identifiers will
        // not connect this document to `src/rotate_signing_key.rs`.
        write(
            &root,
            "docs/runbooks/signing-key-rotation.md",
            "# Rotating the signing key\n\n\
             Twice a year, and during any suspected key compromise.\n\n\
             1. Announce the maintenance window.\n\
             2. Run `keyctl rotate-signing-key` on the bastion host.\n\
             3. Confirm with `keyctl status`.\n",
        )?;

        // THE DECOY. A stray module never declared with `mod`, so cargo does
        // not even compile it. Nothing anywhere refers to it.
        write(
            &root,
            "src/parse_semver.rs",
            r#"pub fn major(version: &str) -> Option<u32> {
    version.split('.').next()?.parse().ok()
}
"#,
        )?;

        repo.add_all()?;
        repo.commit("m04: keyctl with a human-only rotation subcommand")?;

        Ok(GroundTruth {
            live_paths: vec!["src/rotate_signing_key.rs".into()],
            live_symbols: vec!["rotate_signing_key::run".to_string()],
            decoy_dead_paths: vec!["src/parse_semver.rs".into()],
        })
    }
}

/// Write one fixture file, creating parents, attaching the path to any failure.
///
/// Duplicated in each mutant module rather than shared: `fixtures/mod.rs` is
/// complete and declares only the nineteen class modules, so there is nowhere
/// to put a shared helper without changing it.
fn write(root: &Path, rel: &str, contents: &str) -> Result<()> {
    let path = root.join(rel);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| Error::Io {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    std::fs::write(&path, contents).map_err(|source| Error::Io { path, source })
}

#[cfg(test)]
mod tests {
    use super::*;
    use judged_core::git::Repo;
    use tempfile::TempDir;

    fn materialize() -> (TempDir, Repo, GroundTruth) {
        let dir = TempDir::new().expect("create tempdir");
        let truth = HumanCliSubcommand
            .materialize(dir.path())
            .expect("m04 materializes");
        let repo = Repo::discover(dir.path()).expect("fixture is a git repo");
        (dir, repo, truth)
    }

    fn tree(root: &Path) -> Vec<(String, Vec<u8>)> {
        let mut out = Vec::new();
        walk(root, root, &mut out);
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    fn walk(dir: &Path, root: &Path, out: &mut Vec<(String, Vec<u8>)>) {
        for entry in std::fs::read_dir(dir).expect("read fixture directory") {
            let path = entry.expect("read directory entry").path();
            if path.file_name().is_some_and(|n| n == ".git") {
                continue;
            }
            if path.is_dir() {
                walk(&path, root, out);
            } else {
                let rel = path
                    .strip_prefix(root)
                    .expect("path is under the fixture root")
                    .to_string_lossy()
                    .into_owned();
                out.push((rel, std::fs::read(&path).expect("read fixture file")));
            }
        }
    }

    fn mentions(haystack: &[u8], needle: &str) -> bool {
        let needle = needle.as_bytes();
        haystack.windows(needle.len()).any(|w| w == needle)
    }

    #[test]
    fn materializes_a_real_git_repo_with_one_commit() {
        let (_dir, repo, _truth) = materialize();
        assert!(repo.root().join(".git").is_dir(), "expected a git directory");
        assert!(
            repo.is_tracked(Path::new("Cargo.toml"))
                .expect("query the index"),
            "the fixture must be committed, not just written to disk"
        );
    }

    #[test]
    fn ground_truth_paths_all_exist_on_disk() {
        let (_dir, repo, truth) = materialize();
        assert!(!truth.live_paths.is_empty(), "m04's live artifact is a file");
        assert!(
            !truth.decoy_dead_paths.is_empty(),
            "without a decoy, a tool that claims nothing passes m04 for free"
        );
        for path in truth.live_paths.iter().chain(&truth.decoy_dead_paths) {
            assert!(path.is_relative(), "{path:?} must be repo-relative");
            assert!(repo.root().join(path).is_file(), "{path:?} is missing");
        }
    }

    /// One mechanism only: the dispatch table in `src/main.rs`. If the module
    /// stem turned up anywhere else — a test, another module, a build script —
    /// something other than the human could be said to reach it.
    #[test]
    fn the_subcommand_module_is_named_only_by_the_dispatch_table() {
        let (_dir, repo, truth) = materialize();
        let live = truth
            .live_paths
            .first()
            .expect("m04 declares a live subcommand module");
        let stem = live
            .file_stem()
            .expect("live module has a file name")
            .to_string_lossy()
            .into_owned();

        let naming: Vec<String> = tree(repo.root())
            .into_iter()
            .filter(|(_, bytes)| mentions(bytes, &stem))
            .map(|(path, _)| path)
            .collect();

        assert!(
            naming.iter().any(|p| p == "src/main.rs"),
            "the dispatch table must register {stem:?}, otherwise it is not live"
        );
        for path in &naming {
            assert!(
                path == "src/main.rs" || Path::new(path) == live.as_path(),
                "{path} also names {stem:?}; m04 allows exactly one registration \
                 site, or the mutant no longer isolates the human as the caller"
            );
        }
    }

    /// §6.6's counter-signals are what a tool is *supposed* to notice here:
    /// a runbook mention and a `rotate` in the path lexicon. The mutant is only
    /// a fair test if they are actually present.
    #[test]
    fn a_runbook_documents_the_subcommand_for_a_human() {
        let (_dir, repo, _truth) = materialize();
        let runbooks: Vec<(String, Vec<u8>)> = tree(repo.root())
            .into_iter()
            .filter(|(path, _)| path.starts_with("docs/runbooks/"))
            .collect();
        assert!(
            !runbooks.is_empty(),
            "§6.6 names docs/runbooks/ as the counter-signal; plant one"
        );
        assert!(
            runbooks
                .iter()
                .any(|(_, bytes)| mentions(bytes, "rotate-signing-key")),
            "the runbook must tell a human the exact command to type"
        );
    }

    /// No test exercises the subcommand, so a coverage-fused score reports it
    /// at zero — the precise trap §6.6 describes. Assert the asymmetry exists.
    #[test]
    fn no_test_in_the_fixture_exercises_the_subcommand() {
        let (_dir, repo, truth) = materialize();
        let live = truth
            .live_paths
            .first()
            .expect("m04 declares a live subcommand module");
        for (path, bytes) in tree(repo.root()) {
            if Path::new(&path) == live.as_path() {
                continue;
            }
            assert!(
                !mentions(&bytes, "#[test]") || !mentions(&bytes, "rotate"),
                "{path} tests the subcommand; m04 requires it to be uncovered"
            );
        }
    }

    #[test]
    fn the_decoy_is_named_by_nothing() {
        let (_dir, repo, truth) = materialize();
        for decoy in &truth.decoy_dead_paths {
            let stem = decoy
                .file_stem()
                .expect("decoy has a file name")
                .to_string_lossy()
                .into_owned();
            for (path, bytes) in tree(repo.root()) {
                if Path::new(&path) == decoy.as_path() {
                    continue;
                }
                assert!(
                    !mentions(&bytes, &stem),
                    "{path} references the decoy {stem:?}, so it is not dead"
                );
            }
        }
    }
}
