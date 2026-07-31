//! Class 12 — a symbol aliased via `//go:linkname` / `extern "C"` / `#[no_mangle]`.

use std::path::Path;

use judged_core::Result;

use crate::mutant::{Ecosystem, GroundTruth, Mutant};

/// `//go:linkname` binds a name across package boundaries at link time. The
/// Go call graph shows nothing; the program depends on it.
pub struct LinknameAlias;

impl Mutant for LinknameAlias {
    fn id(&self) -> &str {
        "m12"
    }
    fn ecosystem(&self) -> Ecosystem {
        Ecosystem::Go
    }
    fn mechanism(&self) -> &str {
        "symbol bound through a //go:linkname alias rather than an import"
    }
    fn research_ref(&self) -> &str {
        "§10 E2 class 12"
    }
    fn materialize(&self, _dir: &Path) -> Result<GroundTruth> {
        todo!("m12: //go:linkname aliased function with no direct caller")
    }
}
