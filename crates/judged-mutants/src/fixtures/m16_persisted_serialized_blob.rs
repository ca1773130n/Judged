//! Class 16 — a type whose only remaining consumer is a persisted serialized
//! blob *(§6.24; exactly what OpenRewrite's `serialVersionUID` bail-out protects)*.

use std::path::Path;

use judged_core::Result;

use crate::mutant::{Ecosystem, GroundTruth, Mutant};

/// A pickled cache entry on disk still names the class. Nothing in the
/// source does.
pub struct PersistedSerializedBlob;

impl Mutant for PersistedSerializedBlob {
    fn id(&self) -> &str {
        "m16"
    }
    fn ecosystem(&self) -> Ecosystem {
        Ecosystem::Python
    }
    fn mechanism(&self) -> &str {
        "type named only inside an on-disk pickled/serialized blob"
    }
    fn research_ref(&self) -> &str {
        "§10 E2 class 16"
    }
    fn materialize(&self, _dir: &Path) -> Result<GroundTruth> {
        todo!("m16: pickle a value on disk whose class has no source reference")
    }
}
