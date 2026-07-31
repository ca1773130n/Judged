//! Class 8 — referenced only from a Dockerfile / CI workflow / k8s manifest.

use std::path::Path;

use judged_core::Result;

use crate::mutant::{Ecosystem, GroundTruth, Mutant};

/// The reference is real, executable, and written in a language no code
/// analyzer for the project's ecosystem reads.
pub struct CiManifestRef;

impl Mutant for CiManifestRef {
    fn id(&self) -> &str {
        "m08"
    }
    fn ecosystem(&self) -> Ecosystem {
        Ecosystem::Polyglot
    }
    fn mechanism(&self) -> &str {
        "script invoked only from a CI workflow, Dockerfile or k8s manifest"
    }
    fn research_ref(&self) -> &str {
        "§10 E2 class 8"
    }
    fn materialize(&self, _dir: &Path) -> Result<GroundTruth> {
        todo!("m08: script referenced only from .github/workflows and a Dockerfile")
    }
}
