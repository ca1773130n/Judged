//! What a mutant is, and what it promises about the repository it builds.
//!
//! §10 E2 borrows the mutation-based soundness methodology from the Android
//! static-analysis literature (muSE / Bonett et al., ACM TOSEM 3439802):
//! systematically inject known-live artifacts reachable only through **one
//! mechanism each**. Any "dead" verdict on an injected artifact is a hard
//! failure — not a tuning opportunity.

use std::path::{Path, PathBuf};

use judged_core::Result;

/// The language ecosystem a mutant exercises.
///
/// `Polyglot` is not a catch-all: several §10 classes are only expressible
/// across a language boundary, e.g. a CI manifest referencing a script, or a
/// gitignore negation that no language server ever sees.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ecosystem {
    Python,
    TypeScript,
    Rust,
    Go,
    Polyglot,
}

/// What a materialized mutant guarantees about itself.
///
/// The decoys are the load-bearing part. §3.7 and §9.8 require a positive
/// control on every evidence artifact, and the same logic applies one level up
/// to the suite: without genuinely-dead files planted in the repository, a tool
/// that refuses to call anything dead scores a perfect zero false removals and
/// looks indistinguishable from a tool that works. The decoys are what make
/// [`crate::sut::RefusingSut`] fail.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GroundTruth {
    /// Files that are genuinely reachable. Claiming any of these is dead is a
    /// false removal and fails the suite outright.
    pub live_paths: Vec<PathBuf>,
    /// Symbols that are genuinely reachable, same rule.
    pub live_symbols: Vec<String>,
    /// Files that really are dead. A tool that finds none of these has told us
    /// nothing, however safe it looks.
    pub decoy_dead_paths: Vec<PathBuf>,
}

/// One injected liveness mechanism.
pub trait Mutant {
    /// Stable identifier, `m01`..`m19`. Used in reports and in release gating.
    fn id(&self) -> &str;

    /// The ecosystem the mutant is written in.
    fn ecosystem(&self) -> Ecosystem;

    /// The single mechanism by which the live artifact is reachable. One
    /// mechanism per mutant is the whole methodology — a mutant reachable two
    /// ways cannot tell you which signal caught it.
    fn mechanism(&self) -> &str;

    /// Where this class comes from in the research document, so a failure can
    /// be traced back to the documented real-world incident it encodes.
    fn research_ref(&self) -> &str;

    /// Build the mutant repository under `dir` and declare its ground truth.
    fn materialize(&self, dir: &Path) -> Result<GroundTruth>;
}
