//! Class 5 — an error-handling module reached only on failure *(debloat Issue 5)*.

use std::path::Path;

use judged_core::Result;

use crate::mutant::{Ecosystem, GroundTruth, Mutant};

/// Coverage-guided debloaters removed exactly this and shipped it. The
/// handler runs on the day the system is already broken, which is the worst
/// possible day to discover it was deleted.
pub struct ErrorPathOnly;

impl Mutant for ErrorPathOnly {
    fn id(&self) -> &str {
        "m05"
    }
    fn ecosystem(&self) -> Ecosystem {
        Ecosystem::Python
    }
    fn mechanism(&self) -> &str {
        "recovery handler reached only on the failure path"
    }
    fn research_ref(&self) -> &str {
        "§10 E2 class 5"
    }
    fn materialize(&self, _dir: &Path) -> Result<GroundTruth> {
        todo!("m05: error handler invoked only from an except/catch branch")
    }
}
