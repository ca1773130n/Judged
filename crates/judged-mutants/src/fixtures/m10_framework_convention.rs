//! Class 10 — loaded by framework convention.

use std::path::Path;

use judged_core::Result;

use crate::mutant::{Ecosystem, GroundTruth, Mutant};

/// Django's `AppConfig` autoload and Jest's `__mocks__` directory in one
/// repository: two frameworks, two conventions, neither expressed as an
/// import. Polyglot because the convention, not the language, is the thing
/// under test.
pub struct FrameworkConvention;

impl Mutant for FrameworkConvention {
    fn id(&self) -> &str {
        "m10"
    }
    fn ecosystem(&self) -> Ecosystem {
        Ecosystem::Polyglot
    }
    fn mechanism(&self) -> &str {
        "loaded by framework convention: Django AppConfig autoload, Jest __mocks__"
    }
    fn research_ref(&self) -> &str {
        "§10 E2 class 10"
    }
    fn materialize(&self, _dir: &Path) -> Result<GroundTruth> {
        todo!("m10: Django app autoload plus a Jest __mocks__ module")
    }
}
