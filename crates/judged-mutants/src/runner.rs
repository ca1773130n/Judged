//! Running the suite and grading the result.

use crate::mutant::{Ecosystem, Mutant};
use crate::sut::Sut;
use judged_core::Result;

/// How one SUT did on one mutant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MutantReport {
    /// Which mutant, e.g. `m07`.
    pub mutant_id: String,
    /// Its ecosystem, carried so a report can be read without the catalogue.
    pub ecosystem: Ecosystem,
    /// Passed means zero false removals **and** at least the decoys the suite
    /// requires. Both halves are necessary; see [`crate::sut::RefusingSut`].
    pub passed: bool,
    /// Live paths and symbols the SUT claimed were dead. Any entry here is a
    /// hard failure — §10 E2: "any 'dead' verdict is a hard failure."
    pub false_removals: Vec<String>,
    /// Genuinely-dead decoys the SUT correctly identified.
    pub decoys_found: usize,
    /// Decoys planted by this mutant.
    pub decoys_total: usize,
}

/// How one SUT did on the whole catalogue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuiteReport {
    /// The SUT that was graded.
    pub sut_name: String,
    /// One report per mutant, in catalogue order.
    pub reports: Vec<MutantReport>,
    /// Total false removals across the catalogue.
    ///
    /// Kept at the top level because it is the release gate: §10 E2 requires
    /// releases be gated on zero failures, and §11 R1 pre-commits that if this
    /// number is not zero, the auto-act tier is **deleted from the design
    /// rather than tuned**. A number that decides that deserves to be
    /// impossible to miss.
    pub false_removal_count: usize,
}

/// Materialize every mutant, run the SUT over each, and grade it.
///
/// Each mutant gets its own throwaway repository; a mutant that fails to
/// materialize is an error, never a skip, because a silently skipped mutant
/// turns into a pass the SUT did not earn.
pub fn run_suite(_sut: &dyn Sut, _mutants: &[Box<dyn Mutant>]) -> Result<SuiteReport> {
    todo!("run_suite: materialize each mutant into a temp repo, run the SUT, grade")
}
