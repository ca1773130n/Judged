//! Class 1 — referenced only by a string in a YAML/JSON config.

use std::path::Path;

use judged_core::Result;

use crate::mutant::{Ecosystem, GroundTruth, Mutant};

/// A Celery task class named only as a dotted string in `celery.yaml`. No
/// import, no call site: the reference exists in a file no Python analyzer
/// parses as code.
pub struct YamlStringRef;

impl Mutant for YamlStringRef {
    fn id(&self) -> &str {
        "m01"
    }
    fn ecosystem(&self) -> Ecosystem {
        Ecosystem::Python
    }
    fn mechanism(&self) -> &str {
        "dotted class path appearing only as a string in a YAML config"
    }
    fn research_ref(&self) -> &str {
        "§10 E2 class 1"
    }
    fn materialize(&self, _dir: &Path) -> Result<GroundTruth> {
        todo!("m01: task class referenced only from celery.yaml, plus dead decoys")
    }
}
