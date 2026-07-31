//! Class 14 — a checked-in generated artifact served directly by a CDN.

use std::path::Path;

use judged_core::Result;

use crate::mutant::{Ecosystem, GroundTruth, Mutant};

/// It looks like build output, which is what makes it dangerous: the whole
/// point is that the consumer is outside the repository.
pub struct CheckedInGeneratedAsset;

impl Mutant for CheckedInGeneratedAsset {
    fn id(&self) -> &str {
        "m14"
    }
    fn ecosystem(&self) -> Ecosystem {
        Ecosystem::TypeScript
    }
    fn mechanism(&self) -> &str {
        "committed build output whose only consumer is a CDN path"
    }
    fn research_ref(&self) -> &str {
        "§10 E2 class 14"
    }
    fn materialize(&self, _dir: &Path) -> Result<GroundTruth> {
        todo!("m14: committed dist/ bundle referenced only from a CDN URL")
    }
}
