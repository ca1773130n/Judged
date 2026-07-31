//! `assess_run_health` — the §9.2 / §6.20 contract that keeps "no data" from
//! being read as "zero findings".
//!
//! Every shape asserted here is a real failure mode from §6.20: knip failing to
//! load `vite.config.ts` and contributing no roots, Periphery indexing one
//! scheme, Go `deadcode` pointed at a library, cargo-udeps under the wrong
//! feature set. All of them present as a clean run with zero results. The tests
//! named `*_is_never_clean_*` are the three shapes that produce mass deletion
//! in the field, and they are the reason this function exists.

use judged_core::sarif::{
    assess_run_health, Artifact, Invocation, Level, Location, Notification, Run, RunHealth,
    SarifResult, Tool, ROLE_ANALYSIS_TARGET,
};
use std::collections::BTreeMap;

/// A run with no findings, no degradation, and `n` files declared as scanned.
/// The starting point for every test: the shape a caller is most tempted to
/// read as "clean".
fn silent_run(execution_successful: bool, analysis_targets: usize) -> Run {
    Run {
        tool: Tool {
            name: "knip".to_string(),
            version: Some("5.0.0".to_string()),
        },
        invocations: vec![Invocation {
            execution_successful,
            tool_execution_notifications: vec![],
        }],
        artifacts: (0..analysis_targets)
            .map(|i| Artifact {
                location_uri: format!("src/mod{i}.ts"),
                roles: vec![ROLE_ANALYSIS_TARGET.to_string()],
            })
            .collect(),
        results: vec![],
        baseline_guid: None,
    }
}

fn reasons_of(health: &RunHealth) -> &[String] {
    match health {
        RunHealth::Healthy => &[],
        RunHealth::Degraded { reasons } | RunHealth::Failed { reasons } => reasons,
    }
}

fn joined_reasons(health: &RunHealth) -> String {
    reasons_of(health).join(" | ")
}

// ---------------------------------------------------------------------------
// The three shapes that produce mass deletion (§6.20)
// ---------------------------------------------------------------------------

/// Adversarial shape 1: zero results *because the analyzer died*. §9.2 forbids
/// reading a raw exit code; `executionSuccessful` is the only health signal,
/// and false must never be reachable as anything but `Failed`.
#[test]
fn zero_results_with_execution_failure_is_never_clean() {
    let health = assess_run_health(&silent_run(false, 100), 100);

    assert!(
        matches!(health, RunHealth::Failed { .. }),
        "executionSuccessful=false must be Failed, got {health:?}"
    );
    assert!(
        joined_reasons(&health).contains("executionSuccessful"),
        "the reason must name the health bit that tripped, got {health:?}"
    );
}

/// Adversarial shape 2: the analyzer ran to completion but only ever saw 3 of
/// the 100 candidate files — knip after `ERROR: Error loading vite.config.ts`,
/// Periphery on one scheme, `deadcode` aimed at a library. Silence over 3% of
/// the repository is not evidence about the other 97%.
#[test]
fn scanning_three_of_a_hundred_expected_files_is_never_clean() {
    let health = assess_run_health(&silent_run(true, 3), 100);

    assert!(
        matches!(health, RunHealth::Degraded { .. }),
        "a 3/100 analysisTarget set must be Degraded, got {health:?}"
    );
    let reasons = joined_reasons(&health);
    assert!(
        reasons.contains('3') && reasons.contains("100"),
        "the reason must name the observed and expected counts, got {reasons}"
    );
    assert!(
        reasons.contains("0.8"),
        "the reason must name the §9.2 floor, got {reasons}"
    );
}

/// Adversarial shape 3: a rule failed and was disabled mid-run, so the run has
/// zero results *for that rule* by construction. §9.2: partial degradation caps
/// the tier, it is not discarded.
#[test]
fn rule_level_error_notification_with_zero_results_is_never_clean() {
    let mut run = silent_run(true, 100);
    run.invocations[0].tool_execution_notifications = vec![Notification {
        level: Level::Error,
        message: "rule `unused-export` disabled: failed to load vite.config.ts".to_string(),
    }];

    let health = assess_run_health(&run, 100);

    assert!(
        matches!(health, RunHealth::Degraded { .. }),
        "an error notification must Degrade even with 0 results, got {health:?}"
    );
    assert!(
        joined_reasons(&health).contains("failed to load vite.config.ts"),
        "the notification message must be carried verbatim, got {health:?}"
    );
}

// ---------------------------------------------------------------------------
// Absence is not success
// ---------------------------------------------------------------------------

/// §6.20: "no data" is a distinct state from "zero executions". A run that
/// never recorded an invocation never asserted that it ran.
#[test]
fn absent_invocation_is_failure_not_success() {
    let mut run = silent_run(true, 100);
    run.invocations.clear();

    let health = assess_run_health(&run, 100);

    assert!(
        matches!(health, RunHealth::Failed { .. }),
        "a run with no invocation must be Failed, got {health:?}"
    );
}

/// SARIF permits several invocations; one failure poisons the run. Trusting the
/// first or the last would let a tool hide a crashed pass behind a clean one.
#[test]
fn any_failed_invocation_fails_the_whole_run() {
    let mut run = silent_run(true, 100);
    run.invocations.push(Invocation {
        execution_successful: false,
        tool_execution_notifications: vec![],
    });

    let health = assess_run_health(&run, 100);

    assert!(
        matches!(health, RunHealth::Failed { .. }),
        "one failed invocation among many must fail the run, got {health:?}"
    );
}

// ---------------------------------------------------------------------------
// The analysisTarget positive control (§9.2's "single most valuable clause")
// ---------------------------------------------------------------------------

/// The gate is `|analysisTarget| >= 0.8 x |candidates|`, so exactly 80% passes.
/// Pinned because an off-by-one here silently changes how much of a repository
/// may go unscanned before anyone is told.
#[test]
fn analysis_target_floor_passes_at_exactly_eighty_percent() {
    assert_eq!(
        assess_run_health(&silent_run(true, 80), 100),
        RunHealth::Healthy,
        "80/100 is exactly the floor and must pass"
    );
    assert!(
        matches!(
            assess_run_health(&silent_run(true, 79), 100),
            RunHealth::Degraded { .. }
        ),
        "79/100 is below the floor and must degrade"
    );
}

/// Only artifacts carrying the `analysisTarget` role count. A tool that lists
/// every file it merely *traced* would otherwise satisfy the positive control
/// without having been instructed to scan anything.
#[test]
fn artifacts_without_the_analysis_target_role_do_not_count() {
    let mut run = silent_run(true, 0);
    run.artifacts = (0..100)
        .map(|i| Artifact {
            location_uri: format!("src/mod{i}.ts"),
            // "traced" is a real SARIF role and is not an instruction to scan.
            roles: vec!["traced".to_string()],
        })
        .collect();

    let health = assess_run_health(&run, 100);

    assert!(
        matches!(health, RunHealth::Degraded { .. }),
        "100 non-analysisTarget artifacts must not satisfy the control, got {health:?}"
    );
}

/// A role list may legitimately contain roles we do not model; the target role
/// still counts when it appears alongside them.
#[test]
fn analysis_target_role_counts_alongside_other_roles() {
    let mut run = silent_run(true, 0);
    run.artifacts = (0..100)
        .map(|i| Artifact {
            location_uri: format!("src/mod{i}.py"),
            roles: vec!["traced".to_string(), ROLE_ANALYSIS_TARGET.to_string()],
        })
        .collect();

    assert_eq!(assess_run_health(&run, 100), RunHealth::Healthy);
}

/// With no expectation to check against there is no positive control at all —
/// the ratio is unevaluable, so the run cannot be certified. §6.20 requires an
/// explicit positive assertion that the artifact was collected *before*
/// counting a zero; an unknown universe can never supply one.
#[test]
fn zero_expected_analysis_targets_is_never_healthy() {
    assert!(
        matches!(
            assess_run_health(&silent_run(true, 0), 0),
            RunHealth::Degraded { .. }
        ),
        "an unknown universe must not be Healthy"
    );
    assert!(
        matches!(
            assess_run_health(&silent_run(true, 500), 0),
            RunHealth::Degraded { .. }
        ),
        "a large analysisTarget set does not substitute for a known universe"
    );
}

// ---------------------------------------------------------------------------
// Tier ordering and the healthy path
// ---------------------------------------------------------------------------

/// `Failed` outranks `Degraded` — a dead run contributes nothing in either
/// direction — but the degradation reasons stay attached, because they are what
/// tells an operator *why* the run died.
#[test]
fn failure_outranks_degradation_and_keeps_every_reason() {
    let mut run = silent_run(false, 3);
    run.invocations[0].tool_execution_notifications = vec![Notification {
        level: Level::Error,
        message: "rule `no-unused` disabled".to_string(),
    }];

    let health = assess_run_health(&run, 100);

    assert!(
        matches!(health, RunHealth::Failed { .. }),
        "failure must win over degradation, got {health:?}"
    );
    let reasons = joined_reasons(&health);
    assert!(reasons.contains("executionSuccessful"), "got {reasons}");
    assert!(
        reasons.contains("rule `no-unused` disabled"),
        "got {reasons}"
    );
    assert!(reasons.contains(ROLE_ANALYSIS_TARGET), "got {reasons}");
}

/// `warning` notifications degrade too. §6.20's `-coverpkg` case is exactly a
/// warning that instruments nothing and is "trivially lost in CI logs".
#[test]
fn warning_notification_degrades() {
    let mut run = silent_run(true, 100);
    run.invocations[0].tool_execution_notifications = vec![Notification {
        level: Level::Warning,
        message: "no packages being built depend on matches for pattern main".to_string(),
    }];

    let health = assess_run_health(&run, 100);

    assert!(
        matches!(health, RunHealth::Degraded { .. }),
        "a warning notification must cap the tier, got {health:?}"
    );
    assert!(
        joined_reasons(&health).contains("no packages being built"),
        "got {health:?}"
    );
}

/// `note` and `none` are informational. Degrading on them would make every run
/// degraded, which destroys the signal the tier is supposed to carry.
#[test]
fn note_and_none_notifications_do_not_degrade() {
    let mut run = silent_run(true, 100);
    run.invocations[0].tool_execution_notifications = vec![
        Notification {
            level: Level::Note,
            message: "analyzing 100 files".to_string(),
        },
        Notification {
            level: Level::None,
            message: "cache hit".to_string(),
        },
    ];

    assert_eq!(assess_run_health(&run, 100), RunHealth::Healthy);
}

/// The only Healthy shape: it ran, nothing degraded, and it demonstrably saw
/// the repository. Findings themselves are irrelevant to health.
#[test]
fn complete_run_over_a_known_universe_is_healthy() {
    let mut run = silent_run(true, 100);
    run.results = vec![SarifResult {
        rule_id: "unused-export".to_string(),
        level: Level::Warning,
        message: "export `foo` is never imported".to_string(),
        locations: vec![Location {
            uri: "src/mod0.ts".to_string(),
            start_line: Some(12),
        }],
        partial_fingerprints: BTreeMap::new(),
        baseline_state: None,
        suppressions: vec![],
    }];

    assert_eq!(assess_run_health(&run, 100), RunHealth::Healthy);
}

/// Health is a property of the run, not of its findings: a *failed* run that
/// somehow emitted results is still failed, and those results must not be read
/// as evidence.
#[test]
fn results_do_not_rescue_a_failed_run() {
    let mut run = silent_run(false, 100);
    run.results = vec![SarifResult {
        rule_id: "unused-export".to_string(),
        level: Level::Warning,
        message: "export `foo` is never imported".to_string(),
        locations: vec![],
        partial_fingerprints: BTreeMap::new(),
        baseline_state: None,
        suppressions: vec![],
    }];

    assert!(matches!(
        assess_run_health(&run, 100),
        RunHealth::Failed { .. }
    ));
}
