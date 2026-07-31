//! Class 13 — a file un-ignored by a `!` gitignore negation.

use std::path::Path;

use judged_core::Result;

use crate::mutant::{Ecosystem, GroundTruth, Mutant};

/// `.vscode/settings.json` and `var/logs/.gitkeep`: deliberately rescued
/// from a broad ignore rule, which makes the negation itself the statement
/// of intent. §6.17 and Gate 0g both live here.
pub struct GitignoreNegation;

impl Mutant for GitignoreNegation {
    fn id(&self) -> &str {
        "m13"
    }
    fn ecosystem(&self) -> Ecosystem {
        Ecosystem::Polyglot
    }
    fn mechanism(&self) -> &str {
        "file rescued from a broad ignore rule by an explicit ! negation"
    }
    fn research_ref(&self) -> &str {
        "§10 E2 class 13"
    }
    fn materialize(&self, _dir: &Path) -> Result<GroundTruth> {
        todo!("m13: ignore rule plus ! negation rescuing a tracked config")
    }
}
