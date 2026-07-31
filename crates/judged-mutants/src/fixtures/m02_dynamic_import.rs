//! Class 2 — loaded via `importlib` / `require(variable)` / `Class.forName`.

use std::path::Path;

use judged_core::Result;

use crate::mutant::{Ecosystem, GroundTruth, Mutant};

/// Polyglot on purpose: the Python half uses `importlib.import_module` with
/// a name built at runtime, the TypeScript half uses `require(variable)`.
/// Both defeat static import resolution, and a tool that handles one and
/// not the other should not score as if it handled the class.
pub struct DynamicImport;

impl Mutant for DynamicImport {
    fn id(&self) -> &str {
        "m02"
    }
    fn ecosystem(&self) -> Ecosystem {
        Ecosystem::Polyglot
    }
    fn mechanism(&self) -> &str {
        "module name computed at runtime and passed to importlib / require"
    }
    fn research_ref(&self) -> &str {
        "§10 E2 class 2"
    }
    fn materialize(&self, _dir: &Path) -> Result<GroundTruth> {
        todo!("m02: importlib.import_module and require(variable) halves")
    }
}
