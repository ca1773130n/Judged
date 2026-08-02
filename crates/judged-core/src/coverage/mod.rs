//! Family X — observed execution (§2.1, §9.5).
//!
//! Everything else in this repository is Family R: it reasons about references.
//! Four analyzers, three rescue layers, and not one of them has ever watched the
//! program run. §9.5 requires a quorum of at least two of {B, R, X} before any
//! Tier-0 action, so an all-R build cannot reach Tier 0 no matter how well it
//! scores — §11 R1 is unanswerable in either direction until something here
//! observes execution.
//!
//! # Ingest, never collect
//!
//! §11 R9 poses this as open and §9.10 settles it. Collecting coverage means
//! executing the target repository's code, plus its entire transitive lockfile,
//! on the machine running the cleaner — "clean my repo" becomes remote code
//! execution. Ingesting an artifact CI already produced has none of that
//! surface. So this module parses; it never spawns.
//!
//! # A hit is proof. A miss is not evidence.
//!
//! This is the asymmetry the whole module is built around, and it is not a
//! conservative preference — §9.5 states it as a rule with a mechanism. A
//! coverage **hit** is direct proof of use. A coverage **miss** is bounded
//! absence of evidence over one window and one input distribution, and it is
//! *systematically anti-correlated* with the value of the code it describes:
//! error handlers, disaster-recovery paths, platform branches and migration
//! shims are exactly the population that a test suite never enters and that you
//! most want to survive. So a test-coverage miss contributes **zero** toward
//! deadness at any tier. Production coverage may accuse, weakly (+0.5 bans,
//! §9.5); test coverage may not, ever.
//!
//! The consequence for the code here: nothing in this module returns "dead". The
//! only question it answers is *was this executed*, and the only thing a caller
//! may do with a `yes` is drop a claim.
//!
//! # Nothing is trusted without a positive control
//!
//! §3.7 is blunt about the failure mode: every catastrophic failure in the
//! surveyed corpus presents identically, as an artifact reporting *"~0% covered
//! everywhere"* that is then believed. An artifact that was never written, was
//! written by a run that crashed on boot, or was produced by an lcov dialect
//! this parser does not read all look the same from here — and all three read as
//! "nothing is used".
//!
//! [`control`] is the answer, and it is required rather than optional: a repo
//! declares a handful of symbols that must appear executed, and an artifact that
//! cannot show them is discarded whole and loudly. See its module docs for why
//! the granularity is `FNDA` and not lines.

pub mod control;
pub mod lcov;

pub use control::{Control, ControlOutcome};
pub use lcov::{Coverage, FileCoverage, FunctionCoverage};
