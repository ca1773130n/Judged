//! Class 9 — referenced only from a README code block that CI executes.

use std::path::Path;

use judged_core::Result;

use crate::mutant::{Ecosystem, GroundTruth, Mutant};

/// Documentation that is also a test. §0.9 keeps docs out of the deletion
/// path entirely; this mutant checks the tool honours that even when the
/// doc is the *only* thing keeping code alive.
pub struct ReadmeExecutedBlock;

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
