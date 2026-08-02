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
use judged_core::Result;

use crate::fixtures::write;
use crate::mutant::{Declaration, Ecosystem, GroundTruth, Mutant};

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
    /// Reachable only when a human types the subcommand. No test process does.
    fn coverage_declaration(&self) -> Declaration {
        Declaration::nothing()
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
            // Index-aligned with the decoy above: a symbol-level analyzer never
            // claims a path, so without this it is never asked a question it
            // can answer (see `GroundTruth::decoy_dead_symbols`).
            decoy_dead_symbols: vec!["major".to_string()],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::support;

    #[test]
    fn m04_materializes_a_real_git_repo_with_one_commit() {
        let (_dir, repo, _truth) = support::materialize(&HumanCliSubcommand);
        support::assert_committed(&repo, &["Cargo.toml"]);
    }

    #[test]
    fn m04_ground_truth_paths_all_exist_on_disk() {
        let (_dir, repo, truth) = support::materialize(&HumanCliSubcommand);
        assert!(
            !truth.live_paths.is_empty(),
            "m04's live artifact is a file"
        );
        support::assert_ground_truth_is_on_disk(&repo, &truth);
    }

    /// One mechanism only: the dispatch table in `src/main.rs`. If the module
    /// stem turned up anywhere else — a test, another module, a build script —
    /// something other than the human could be said to reach it.
    #[test]
    fn m04_the_subcommand_module_is_named_only_by_the_dispatch_table() {
        let (_dir, repo, truth) = support::materialize(&HumanCliSubcommand);
        let live = truth
            .live_paths
            .first()
            .expect("m04 declares a live subcommand module");
        let stem = live
            .file_stem()
            .expect("live module has a file name")
            .to_string_lossy()
            .into_owned();

        let naming: Vec<String> = support::tree(repo.root())
            .into_iter()
            .filter(|(_, bytes)| support::mentions(bytes, &stem))
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
    fn m04_a_runbook_documents_the_subcommand_for_a_human() {
        let (_dir, repo, _truth) = support::materialize(&HumanCliSubcommand);
        let runbooks: Vec<(String, Vec<u8>)> = support::tree(repo.root())
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
                .any(|(_, bytes)| support::mentions(bytes, "rotate-signing-key")),
            "the runbook must tell a human the exact command to type"
        );
    }

    /// No test exercises the subcommand, so a coverage-fused score reports it
    /// at zero — the precise trap §6.6 describes. Assert the asymmetry exists.
    #[test]
    fn m04_no_test_in_the_fixture_exercises_the_subcommand() {
        let (_dir, repo, truth) = support::materialize(&HumanCliSubcommand);
        let live = truth
            .live_paths
            .first()
            .expect("m04 declares a live subcommand module");
        for (path, bytes) in support::tree(repo.root()) {
            if Path::new(&path) == live.as_path() {
                continue;
            }
            assert!(
                !support::mentions(&bytes, "#[test]") || !support::mentions(&bytes, "rotate"),
                "{path} tests the subcommand; m04 requires it to be uncovered"
            );
        }
    }

    #[test]
    fn m04_the_decoy_is_named_by_nothing() {
        let (_dir, repo, truth) = support::materialize(&HumanCliSubcommand);
        support::assert_decoys_are_unreferenced(&repo, &truth);
    }
}
