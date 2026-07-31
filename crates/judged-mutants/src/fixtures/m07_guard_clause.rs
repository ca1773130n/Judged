//! Class 7 — a guard clause with no observable effect *(debloat Issue 3)*.

use std::path::Path;

use judged_core::Result;

use crate::mutant::{Ecosystem, GroundTruth, Mutant};

/// The hardest class to argue with, because under normal conditions the
/// guard genuinely does nothing. That is what a guard is.
pub struct GuardClause;

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
    fn materialize(&self, _dir: &Path) -> Result<GroundTruth> {
        todo!("m07: guard clause whose removal changes nothing observable")
    }
}
