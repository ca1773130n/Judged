//! Class 19 — an exported symbol with no in-repo caller but a live ABI consumer
//! *(§6.24, §6.9)*.

use std::path::Path;

use judged_core::Result;

use crate::mutant::{Ecosystem, GroundTruth, Mutant};

/// Unfalsifiable from inside the repository **by construction**, which is
/// what makes it the right test: the only correct behaviour is to refuse,
/// and a tool that guesses here is guessing everywhere.
pub struct AbiConsumerExport;

impl Mutant for AbiConsumerExport {
    fn id(&self) -> &str {
        "m19"
    }
    fn ecosystem(&self) -> Ecosystem {
        Ecosystem::Rust
    }
    fn mechanism(&self) -> &str {
        "#[no_mangle] export whose only consumer is outside the repository"
    }
    fn research_ref(&self) -> &str {
        "§10 E2 class 19"
    }
    fn materialize(&self, _dir: &Path) -> Result<GroundTruth> {
        todo!("m19: #[no_mangle] extern export with no in-repo caller")
    }
}
