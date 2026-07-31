//! Class 15 — a worker class named only in an already-enqueued job payload *(§6.24)*.

use std::path::Path;

use judged_core::Result;

use crate::mutant::{Ecosystem, GroundTruth, Mutant};

/// Constructed by enqueuing, then deleting the class, then draining. §10 E2
/// is specific about the bar: the test suite must stay green **and** the
/// tool must still refuse. Green tests are not evidence here.
pub struct EnqueuedJobPayload;

impl Mutant for EnqueuedJobPayload {
    fn id(&self) -> &str {
        "m15"
    }
    fn ecosystem(&self) -> Ecosystem {
        Ecosystem::Python
    }
    fn mechanism(&self) -> &str {
        "worker class named only inside a job payload already sitting in the queue"
    }
    fn research_ref(&self) -> &str {
        "§10 E2 class 15"
    }
    fn materialize(&self, _dir: &Path) -> Result<GroundTruth> {
        todo!("m15: enqueue a Celery payload naming a class with no live call site")
    }
}
