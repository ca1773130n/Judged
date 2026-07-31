//! Class 11 — an ORM/serializer field touched only via reflection
//! *(Periphery's Codable case)*.

use std::path::Path;

use judged_core::Result;

use crate::mutant::{Ecosystem, GroundTruth, Mutant};

/// The field is never read by name anywhere. The serializer walks it
/// reflectively, and deleting it silently changes the wire format.
pub struct ReflectiveField;

impl Mutant for ReflectiveField {
    fn id(&self) -> &str {
        "m11"
    }
    fn ecosystem(&self) -> Ecosystem {
        Ecosystem::Python
    }
    fn mechanism(&self) -> &str {
        "model field enumerated reflectively by a serializer, never named"
    }
    fn research_ref(&self) -> &str {
        "§10 E2 class 11"
    }
    fn materialize(&self, _dir: &Path) -> Result<GroundTruth> {
        todo!("m11: serializer field reached only through reflection")
    }
}
