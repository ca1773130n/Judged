//! Class 9 — referenced only from a README code block that CI executes.
//!
//! **The mechanism.** `src/lib.rs` carries
//! `#![doc = include_str!("../README.md")]`, so the ```` ```rust ```` block in
//! the README becomes a doctest. `cargo test --doc` — a step in the fixture's
//! CI workflow — compiles and runs it. That block is the only thing in the
//! repository that names `badge::render_badge`.
//!
//! **Why every other signal misses it.** The call site is inside a Markdown
//! file, so no Rust parser sees it as code; the binary does not call it, so the
//! call graph from `main` does not reach it; and §4.1 records the specific tool
//! failure — **cargo-udeps cannot see doctests at all**, which is why a crate
//! used only from a doc example is reported as an unused dependency. A tool
//! that models "what does `cargo build` need" gets the same answer, because
//! `cargo build` genuinely does not need it. Only `cargo test --doc` does, and
//! it is the job nobody models.
//!
//! **What is supposed to catch it.** §0.9's rule that documentation is not in
//! the deletion path, plus the whole-repo literal veto (§6.20) reading Markdown
//! as text worth matching. The crate is `publish = false`, so "it is `pub`, it
//! must be someone's API" is not available as an escape hatch here — the item
//! is public solely because a doctest compiles as an external crate.

use std::path::{Path, PathBuf};

use judged_core::Result;

use crate::mutant::{Ecosystem, GroundTruth, Mutant};

/// Documentation that is also a test. §0.9 keeps docs out of the deletion
/// path entirely; this mutant checks the tool honours that even when the
/// doc is the *only* thing keeping code alive.
pub struct ReadmeExecutedBlock;

/// Repo-relative path of the artifact that is alive and looks dead.
const LIVE: &str = "src/badge.rs";

/// The symbol inside [`LIVE`] that only the README example calls.
const LIVE_SYMBOL: &str = "render_badge";

/// The one file that names [`LIVE_SYMBOL`] — and it is not source.
const MECHANISM: &str = "README.md";

/// The crate root, which turns [`MECHANISM`] into executable documentation.
const DOC_INCLUDE_SITE: &str = "src/lib.rs";

/// The include that makes the README a doctest rather than prose.
const DOC_INCLUDE: &str = "include_str!(\"../README.md\")";

/// The CI step that runs it. Without this the block is prose and the mutant
/// would be claiming a liveness mechanism it does not have.
const CI_WORKFLOW: &str = ".github/workflows/ci.yml";

/// The command in [`CI_WORKFLOW`] that executes the README block.
const CI_DOCTEST_STEP: &str = "cargo test --doc";

impl ReadmeExecutedBlock {
    /// Repo-relative paths of the genuinely-dead files planted here. Neither
    /// is declared with `mod`, so neither is even compiled.
    const DECOYS: [&'static str; 2] = ["src/orphan_sparkline.rs", "src/unused_palette.rs"];
}

impl Mutant for ReadmeExecutedBlock {
    fn id(&self) -> &str {
        "m09"
    }
    fn ecosystem(&self) -> Ecosystem {
        Ecosystem::Rust
    }
    fn mechanism(&self) -> &str {
        "API exercised only by a README example that CI runs"
    }
    fn research_ref(&self) -> &str {
        "§10 E2 class 9"
    }
    fn materialize(&self, _dir: &Path) -> Result<GroundTruth> {
        todo!("m09: doctest-style README block wired into the CI job")
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
        let truth = ReadmeExecutedBlock
            .materialize(dir.path())
            .expect("m09 materializes");
        (dir, truth)
    }

    #[test]
    fn m09_is_a_real_git_repository_whose_live_artifact_is_committed() {
        let (dir, _truth) = materialize_into_tempdir();
        let repo = Repo::discover(dir.path()).expect("fixture is a git working tree");

        assert!(
            repo.blob_sha(Path::new(LIVE))
                .expect("blob_sha query succeeds")
                .is_some(),
            "{LIVE} must be present in HEAD"
        );
    }

    #[test]
    fn m09_ground_truth_names_files_that_are_really_there() {
        let (dir, truth) = materialize_into_tempdir();

        assert_eq!(truth.live_paths, vec![Path::new(LIVE).to_path_buf()]);
        assert_eq!(truth.live_symbols, vec![LIVE_SYMBOL.to_string()]);
        assert_eq!(truth.decoy_dead_paths.len(), ReadmeExecutedBlock::DECOYS.len());

        for path in truth.live_paths.iter().chain(&truth.decoy_dead_paths) {
            assert!(
                dir.path().join(path).is_file(),
                "ground truth names {} but it is not on disk",
                path.display()
            );
        }
    }

    #[test]
    fn m09_no_source_file_names_the_documented_api() {
        let (dir, _truth) = materialize_into_tempdir();

        // The Markdown file and the definition, and nothing else. In
        // particular not `src/main.rs`: if the binary called it, the call graph
        // would rescue it and the mutant would be testing nothing.
        assert_eq!(
            files_mentioning(dir.path(), LIVE_SYMBOL),
            vec![MECHANISM.to_string(), LIVE.to_string()],
            "only the README example and the definition may name the API"
        );
    }

    #[test]
    fn m09_the_readme_block_is_actually_executed() {
        let (dir, _truth) = materialize_into_tempdir();

        // A README block that CI does not run is a comment, and this class
        // would then be indistinguishable from an ordinary dead function. Both
        // halves have to hold: the crate root includes the README as docs, and
        // CI runs the doctests.
        assert_eq!(
            files_mentioning(dir.path(), DOC_INCLUDE),
            vec![DOC_INCLUDE_SITE.to_string()],
            "the crate root must include the README as documentation"
        );
        assert_eq!(
            files_mentioning(dir.path(), CI_DOCTEST_STEP),
            vec![CI_WORKFLOW.to_string()],
            "CI must run the doctests, or the block is not executed"
        );
    }

    #[test]
    fn m09_decoys_are_named_nowhere_at_all() {
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
