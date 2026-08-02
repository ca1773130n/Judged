//! E2 — mutation injection, the precision eval nobody runs (§10).
//!
//! §10 E2 says build this **first**, before any analyzer integration. The
//! method comes from the Android static-analysis literature (muSE / Bonett et
//! al., ACM TOSEM 3439802): inject known-live artifacts reachable through one
//! mechanism each, and treat any "dead" verdict on one as a hard failure.
//!
//! The consequence is pre-committed and it is not a threshold to tune. §10 E2:
//! "if no signal combination clears all 14 at zero false removals, the product
//! is report+quarantine and the auto-act tier must be DELETED from the design
//! rather than tuned." This crate exists to make that question answerable in
//! weeks rather than after an incident.

pub mod adapters;
pub mod coverage;
pub mod fixtures;
pub mod gate1;
pub mod gate3f;
pub mod mutant;
pub mod roots;
pub mod runner;
pub mod sut;

pub use mutant::{Ecosystem, GroundTruth, Mutant};
pub use runner::{run_suite, run_suite_with, MutantReport, SuiteOptions, SuiteReport};
// `SymbolClaim` rides along with `SutVerdict` because it is the type of one of
// its two fields: re-exporting the verdict without it would leave a consumer
// unable to name what it holds.
pub use sut::{NaiveSut, RefusingSut, Sut, SutVerdict, SymbolClaim};
