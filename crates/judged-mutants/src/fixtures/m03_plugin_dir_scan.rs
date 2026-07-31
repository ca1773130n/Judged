//! Class 3 — registered by a directory-scanning plugin loader.

use std::path::Path;

use judged_core::Result;

use crate::mutant::{Ecosystem, GroundTruth, Mutant};

/// The loader iterates `plugins/*.py` at startup. Nothing names the plugin;
/// its liveness is a property of where the file sits on disk, which is
/// invisible to a call-graph.
pub struct PluginDirScan;

impl Mutant for PluginDirScan {
    fn id(&self) -> &str {
        "m03"
    }
    fn ecosystem(&self) -> Ecosystem {
        Ecosystem::Python
    }
    fn mechanism(&self) -> &str {
        "plugin discovered by directory scan at startup, never named in code"
    }
    fn research_ref(&self) -> &str {
        "§10 E2 class 3"
    }
    fn materialize(&self, _dir: &Path) -> Result<GroundTruth> {
        todo!("m03: plugins/ directory scanned at import time")
    }
}
