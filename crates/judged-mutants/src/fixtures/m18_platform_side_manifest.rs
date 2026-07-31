//! Class 18 — an entry point declared only in a platform-side manifest *(§5.2)*.

use std::path::Path;

use judged_core::Result;

use crate::mutant::{Ecosystem, GroundTruth, Mutant};

/// An Android `<receiver>`, a `.pth` file, an `NSExtensionPrincipalClass`, a
/// `META-INF/…AutoConfiguration.imports` line, a `[ModuleInitializer]`. The
/// platform, not the program, does the calling.
pub struct PlatformSideManifest;

impl Mutant for PlatformSideManifest {
    fn id(&self) -> &str {
        "m18"
    }
    fn ecosystem(&self) -> Ecosystem {
        Ecosystem::Polyglot
    }
    fn mechanism(&self) -> &str {
        "entry point declared only in a platform manifest the platform reads"
    }
    fn research_ref(&self) -> &str {
        "§10 E2 class 18"
    }
    fn materialize(&self, _dir: &Path) -> Result<GroundTruth> {
        todo!("m18: entry point declared only in a platform-side manifest")
    }
}
