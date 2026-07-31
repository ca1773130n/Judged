//! Class 17 — a symbol reachable only through a link-time registry *(§6.1)*.

use std::path::Path;

use judged_core::Result;

use crate::mutant::{Ecosystem, GroundTruth, Mutant};

/// `inventory::submit!` / `linkme` / a self-registering `static Registrar` /
/// `__attribute__((constructor))`. §10 E2 puts it exactly: the call graph is
/// genuinely empty and the code genuinely runs.
pub struct LinkTimeRegistry;

impl Mutant for LinkTimeRegistry {
    fn id(&self) -> &str {
        "m17"
    }
    fn ecosystem(&self) -> Ecosystem {
        Ecosystem::Rust
    }
    fn mechanism(&self) -> &str {
        "registered at link time via inventory::submit!, with an empty call graph"
    }
    fn research_ref(&self) -> &str {
        "§10 E2 class 17"
    }
    fn materialize(&self, _dir: &Path) -> Result<GroundTruth> {
        todo!("m17: inventory::submit! registration with no direct caller")
    }
}
