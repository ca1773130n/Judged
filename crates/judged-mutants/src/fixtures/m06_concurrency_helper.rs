//! Class 6 — a synchronization helper used only under concurrency *(debloat Issue 4)*.

use std::path::Path;

use judged_core::Result;

use crate::mutant::{Ecosystem, GroundTruth, Mutant};

/// Single-threaded observation never touches it. Deleting it does not break
/// the build or the tests; it corrupts data under load.
pub struct ConcurrencyHelper;

impl Mutant for ConcurrencyHelper {
    fn id(&self) -> &str {
        "m06"
    }
    fn ecosystem(&self) -> Ecosystem {
        Ecosystem::Rust
    }
    fn mechanism(&self) -> &str {
        "lock helper exercised only when two threads contend"
    }
    fn research_ref(&self) -> &str {
        "§10 E2 class 6"
    }
    fn materialize(&self, _dir: &Path) -> Result<GroundTruth> {
        todo!("m06: mutex helper on a path only taken under contention")
    }
}
