//! Class 4 — a CLI subcommand invoked only by humans.

use std::path::Path;

use judged_core::Result;

use crate::mutant::{Ecosystem, GroundTruth, Mutant};

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
    fn materialize(&self, _dir: &Path) -> Result<GroundTruth> {
        todo!("m04: registered subcommand with no in-repo caller")
    }
}
