//! The ratchet itself: block what is new, tolerate what we already owed.

use judged_core::git::Repo;
use judged_core::sarif::{Run, SarifResult};

use crate::baseline::Baseline;
use crate::rot::RotReason;

/// What the ratchet decided.
///
/// Note what is *not* here: no deletion, no quarantine, no verdict about
/// liveness. §0.5 — ship the ratchet before the reaper — makes this component
/// deliberately incapable of touching the working tree. Its entire power is to
/// fail a build.
#[derive(Debug, Clone, PartialEq)]
pub enum RatchetOutcome {
    /// Nothing new, nothing rotten. CI passes.
    Clean,
    /// Findings not present in the baseline. CI fails, and these are the only
    /// things the developer has to deal with.
    NewFindings(Vec<SarifResult>),
    /// The baseline itself needs pruning. Reported separately from
    /// [`RatchetOutcome::NewFindings`] because the remediation is the opposite:
    /// delete lines from the baseline rather than fix code.
    Rot(Vec<RotReason>),
    /// The ratchet declined to judge this run at all.
    ///
    /// A degraded or failed analyzer run (§9.2 `executionSuccessful`, the
    /// `analysisTarget` ratio floor) cannot distinguish "no new findings" from
    /// "found nothing because it crashed". Passing CI on that evidence would
    /// teach everyone that a green ratchet means nothing. Refusing loudly is
    /// the only honest option — §12: silent failures are bugs.
    Refused { reason: String },
}

/// Compare a run against the baseline.
///
/// `expected_analysis_targets` is the number of files the caller believes the
/// tool should have scanned; it feeds the §9.2 positive control that decides
/// whether this run is trustworthy enough to fail a build over. `now` is an
/// RFC 3339 timestamp supplied by the caller so runs are reproducible.
pub fn ratchet(
    _baseline: &Baseline,
    _run: &Run,
    _repo: &Repo,
    _expected_analysis_targets: usize,
    _now: &str,
) -> RatchetOutcome {
    todo!("ratchet: assess run health first, then rot, then new findings")
}
