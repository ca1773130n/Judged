//! The ratchet itself: block what is new, tolerate what we already owed.

use std::collections::BTreeSet;

use judged_core::git::Repo;
use judged_core::sarif::{assess_run_health, BaselineState, Run, RunHealth, SarifResult};

use crate::baseline::{result_fingerprint, Baseline};
use crate::rot::{detect_rot, RotReason};

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
    /// The run was trusted enough to judge, but not entirely.
    ///
    /// §9.2 is explicit that partial degradation must **cap the tier for
    /// affected paths** rather than be discarded, so a degraded run still
    /// produces a verdict — and that verdict is carried inside rather than
    /// returned on its own, so that a caller cannot reach it without also
    /// having the reasons in hand. A green ratchet over half a repository is
    /// exactly the outcome §6.20 says nobody ever notices.
    ///
    /// `verdict` is never itself `Degraded`; [`ratchet`] wraps at most once.
    Degraded {
        reasons: Vec<String>,
        verdict: Box<RatchetOutcome>,
    },
}

/// Map an outcome onto a process exit code.
///
/// §9.2 names Ruff as the model contract: **0 = clean, 1 = violations, 2 =
/// abnormal termination**, and notes that Semgrep and cargo-machete match that
/// shape while knip, vulture, ts-prune, Go deadcode and Periphery conflate
/// "clean" with "crashed before doing anything". Judged is on the right side of
/// that line by construction: refusal is the only path to 2, and a failed
/// health assessment can take no other path.
///
/// Degradation does not move the code. It is surfaced in the outcome for the
/// operator to read, not used to fail a build that the analyzers did not fail.
pub fn exit_code(outcome: &RatchetOutcome) -> i32 {
    match outcome {
        RatchetOutcome::Clean => 0,
        // Rot sits with new findings rather than with refusal because both are
        // findings about the repository that a human must act on, and both are
        // things this run positively established. Refusal is the opposite: the
        // absence of an establishable answer.
        RatchetOutcome::NewFindings(_) | RatchetOutcome::Rot(_) => 1,
        RatchetOutcome::Refused { .. } => 2,
        RatchetOutcome::Degraded { verdict, .. } => exit_code(verdict),
    }
}

/// Where a single result stands relative to the baseline.
///
/// The other two SARIF states are reached elsewhere, or not at all:
///
/// - [`BaselineState::Absent`] describes a baseline entry with no result, which
///   is not a property of any result in this run. It surfaces as
///   [`RotReason::NeverMatched`] instead, because "absent" understates it — an
///   entry matching nothing is an amnesty protecting nothing.
/// - [`BaselineState::Updated`] means same finding, changed detail. A
///   [`crate::BaselineEntry`] stores no message and no line, deliberately (§9.2:
///   fingerprints must never be line-based), so v1 has nothing to compare it
///   against and cannot tell `Updated` from `Unchanged`. Reporting it would be
///   a guess.
pub fn baseline_state(baseline: &Baseline, result: &SarifResult) -> BaselineState {
    if is_baselined(&known_fingerprints(baseline), result) {
        BaselineState::Unchanged
    } else {
        BaselineState::New
    }
}

/// The baseline's join keys, in a form that can be probed in log time.
///
/// Built once per batch rather than rescanned per result: a monorepo baseline
/// and a monorepo run are both in the thousands, and the ratchet runs on every
/// CI job.
fn known_fingerprints(baseline: &Baseline) -> BTreeSet<&str> {
    baseline
        .entries()
        .iter()
        .map(|entry| entry.fingerprint.as_str())
        .collect()
}

/// The single definition of "the baseline already carries this finding".
/// Both the per-result query and the batch diff go through it so they cannot
/// disagree about what `new` means.
fn is_baselined(known: &BTreeSet<&str>, result: &SarifResult) -> bool {
    known.contains(result_fingerprint(result).as_str())
}

/// Compare a run against the baseline.
///
/// `expected_analysis_targets` is the number of files the caller believes the
/// tool should have scanned; it feeds the §9.2 positive control that decides
/// whether this run is trustworthy enough to fail a build over. `now` is an
/// RFC 3339 timestamp supplied by the caller so runs are reproducible.
///
/// Three stages, in this order, and the order is the design:
///
/// 1. **Health.** A run that failed its §9.2 health assessment is refused
///    before anything is compared. §6.20's failure mode is a crashed analyzer
///    emitting zero results and a ratchet recording that as "nothing new" —
///    permanently disarmed, and green, so nobody looks again.
/// 2. **Rot.** Reported before new findings because both fail the build, so the
///    only question is what the developer should do first. Pruning the baseline
///    is mechanical; fixing code is not, and a new-findings list computed
///    against a baseline already known to be stale has to be recomputed anyway.
/// 3. **New findings.** The only thing the developer is asked to fix, which is
///    the entire proposition of §9.14: block the inflow without demanding the
///    backlog.
pub fn ratchet(
    baseline: &Baseline,
    run: &Run,
    repo: &Repo,
    expected_analysis_targets: usize,
    now: &str,
) -> RatchetOutcome {
    let degradations = match assess_run_health(run, expected_analysis_targets) {
        RunHealth::Failed { reasons } => {
            return RatchetOutcome::Refused {
                reason: reasons.join("; "),
            }
        }
        RunHealth::Degraded { reasons } => reasons,
        RunHealth::Healthy => Vec::new(),
    };

    let rot = detect_rot(baseline, run, repo, now);
    let verdict = if !rot.is_empty() {
        RatchetOutcome::Rot(rot)
    } else {
        let new = new_findings(baseline, run);
        if new.is_empty() {
            RatchetOutcome::Clean
        } else {
            RatchetOutcome::NewFindings(new)
        }
    };

    if degradations.is_empty() {
        verdict
    } else {
        RatchetOutcome::Degraded {
            reasons: degradations,
            verdict: Box::new(verdict),
        }
    }
}

/// Results whose fingerprint the baseline does not carry, annotated with
/// `baselineState: new` so the caller can re-emit them as SARIF without
/// recomputing the diff.
fn new_findings(baseline: &Baseline, run: &Run) -> Vec<SarifResult> {
    let known = known_fingerprints(baseline);
    run.results
        .iter()
        .filter(|result| !is_baselined(&known, result))
        .map(|result| {
            let mut annotated = result.clone();
            annotated.baseline_state = Some(BaselineState::New);
            annotated
        })
        .collect()
}
