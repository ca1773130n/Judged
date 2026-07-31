//! The system under test, and the two controls the suite needs to be meaningful.

use std::path::{Path, PathBuf};

use judged_core::Result;

/// What a cleaner claims is dead after looking at a repository.
///
/// There is no field for "confidence" and no field for "score". §9.2 records
/// that the SARIF spec itself warns rank values from different tools "are in
/// general not commensurable"; the suite grades on claims, not on how sure the
/// tool felt.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SutVerdict {
    /// Repo-relative paths the tool says can be removed.
    pub claimed_dead_paths: Vec<PathBuf>,
    /// Symbols the tool says can be removed.
    pub claimed_dead_symbols: Vec<String>,
}

/// A cleaner the suite can grade.
pub trait Sut {
    /// Name used in [`crate::runner::SuiteReport`].
    fn name(&self) -> &str;

    /// Analyze `repo` and return what it would remove. Implementations must not
    /// mutate `repo` — §9.2: adapters are read-only, the orchestrator owns 100%
    /// of mutations.
    fn run(&self, repo: &Path) -> Result<SutVerdict>;
}

/// A deliberately bad cleaner: reachability from obvious entry points, nothing
/// else. No grep veto, no config parsing, no framework conventions.
///
/// **This is the suite's own positive control.** §3.7 and §9.8 establish the
/// principle for evidence artifacts — if known-live symbols do not appear,
/// discard the artifact loudly — and the suite needs the same guarantee about
/// itself. `NaiveSut` must FAIL, loudly and on many mutants. **If a naive
/// cleaner ever passes the suite, the suite is theatre** and its green results
/// on a real tool mean nothing.
pub struct NaiveSut;

impl Sut for NaiveSut {
    fn name(&self) -> &str {
        "naive"
    }

    fn run(&self, _repo: &Path) -> Result<SutVerdict> {
        todo!("NaiveSut::run: naive reachability, no grep veto, no config awareness")
    }
}

/// A cleaner that claims nothing is ever dead.
///
/// The negative control, and the reason [`crate::mutant::GroundTruth`] carries
/// decoys. This SUT has a perfect false-removal record and is completely
/// useless; a suite that cannot tell it apart from a working tool is measuring
/// nothing. It must fail on decoy recall while passing on false removals.
pub struct RefusingSut;

impl Sut for RefusingSut {
    fn name(&self) -> &str {
        "refusing"
    }

    fn run(&self, _repo: &Path) -> Result<SutVerdict> {
        todo!("RefusingSut::run: always an empty verdict")
    }
}
