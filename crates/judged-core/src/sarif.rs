//! A deliberately minimal SARIF 2.1.0 subset — Judged's normalized projection.
//!
//! §9.2 makes SARIF *the* integration contract: adapters emit it, and the
//! orchestrator never reads a raw exit code or a tool's native output. This
//! module models only the fields that section says matter, which are precisely
//! the fields most SARIF consumers ignore:
//!
//! - [`Invocation::execution_successful`] — the adapter's health bit. The SARIF
//!   spec's own note exists because "not all programs exit with an exit code of
//!   0 on success and non-0 on failure". knip, vulture, ts-prune, Go deadcode
//!   and Periphery all conflate "clean" with "crashed before doing anything",
//!   so this bit plus a positive control is the only usable signal.
//! - [`Artifact::roles`] containing [`ROLE_ANALYSIS_TARGET`] — the scanned
//!   universe, and per §9.2 "the single most valuable contract clause".
//! - [`Invocation::tool_execution_notifications`] — partial degradation, which
//!   must cap the tier for affected paths rather than be discarded.
//! - [`SarifResult::partial_fingerprints`] — ledger identity across our own
//!   algorithm changes. Content-derived, never line-based.
//! - [`SarifResult::baseline_state`] — the stability window, natively; and the
//!   ratchet (§9.14).
//! - [`SarifResult::suppressions`] — the keep DSL (§5.3).
//!
//! `result.rank` is deliberately absent: §9.2 flags that the spec itself warns
//! rank values from different tools "are in general not commensurable", so the
//! interchange format contains a direct warning against the naive score fusion
//! we must not do. Leaving the field unmodelled makes that mistake unspellable.
//!
//! **This is a normalized projection, not the wire shape.** SARIF nests
//! `tool.driver.name`, `artifact.location.uri`, `result.message.text` and
//! `result.locations[].physicalLocation.artifactLocation.uri`; those are
//! flattened here because every consumer in this workspace wants the leaf.
//! Ingest adapters own the mapping from wire SARIF onto these types.
//!
//! # Leniency belongs in the parser, never in the assessment
//!
//! Deserialization is deliberately permissive in two directions and strict in a
//! third:
//!
//! - Fields we do not model are ignored. Real tools emit far more than this
//!   subset, and rejecting a log because it carries `$schema` or `ruleIndex`
//!   turns a healthy run into *no* run — which is the §6.20 failure mode.
//! - Absent collections deserialize to empty. "The tool reported nothing" is a
//!   meaningful state that [`assess_run_health`] already handles; refusing to
//!   parse it would only relocate the decision somewhere with less context.
//! - Unknown *values* of a modelled enum still fail loudly. Quietly demoting an
//!   unrecognized `level` would make a fatal notification invisible.
//!
//! None of that leniency reaches the verdict: a log missing `invocations`
//! parses cleanly and then assesses as [`RunHealth::Failed`].

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The artifact role that marks a file as one the tool was actually instructed
/// to scan. §9.2: hard-gate on `|analysisTarget| >= 0.8 x |candidate files|`,
/// and mark the subtree UNKNOWN otherwise — this is the static positive control
/// that catches silent degradation before it becomes a mass deletion.
pub const ROLE_ANALYSIS_TARGET: &str = "analysisTarget";

/// The floor from §9.2 for the ratio of declared analysis targets to expected
/// candidate files. Below this, the run has not seen enough of the repository
/// for its silence to mean anything.
pub const ANALYSIS_TARGET_RATIO_FLOOR: f64 = 0.8;

/// The root of a SARIF log.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SarifLog {
    /// One run per adapter invocation.
    pub runs: Vec<Run>,
}

/// A single analyzer invocation and everything it claimed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Run {
    /// The analyzer that produced this run.
    pub tool: Tool,
    /// Health and degradation records. SARIF permits several; §9.2 requires at
    /// least one so that `execution_successful` is always answerable. An empty
    /// list is therefore a *contract violation*, and [`assess_run_health`]
    /// treats it as failure rather than as an absence of bad news.
    #[serde(default)]
    pub invocations: Vec<Invocation>,
    /// The scanned universe. See [`ROLE_ANALYSIS_TARGET`].
    #[serde(default)]
    pub artifacts: Vec<Artifact>,
    /// The accusations. Never verdicts — §9.4: store evidence, never verdicts.
    #[serde(default)]
    pub results: Vec<SarifResult>,
    /// Identifies the baseline that [`SarifResult::baseline_state`] is relative
    /// to. Absent when the run was not diffed against anything.
    pub baseline_guid: Option<String>,
}

/// The analyzer behind a run, normalized from SARIF's `tool.driver`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tool {
    /// Tool name, e.g. `knip`, `vulture`, `cargo-machete`.
    pub name: String,
    /// Tool version. Evidence is invalidated when this changes (§9.4 records
    /// `tool_version` alongside every observation).
    pub version: Option<String>,
}

/// One execution of the analyzer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Invocation {
    /// The adapter's computed health bit — **not** a raw exit code (§9.2).
    ///
    /// Defaults to `false` when the field is absent. SARIF requires it, so a
    /// log without it is malformed — but §6.20 forbids resolving that ambiguity
    /// in the tool's favour, and "absent" must never be readable as "ran fine".
    #[serde(default)]
    pub execution_successful: bool,
    /// Per-rule failures with "rule disabled; run continues" semantics. §9.2:
    /// partial degradation must cap the tier for affected paths, not be
    /// discarded.
    #[serde(default)]
    pub tool_execution_notifications: Vec<Notification>,
}

/// A degradation record emitted by the tool during a run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Notification {
    /// Severity. `Level::Error` here means a rule was disabled mid-run.
    pub level: Level,
    /// Human-readable description of what degraded.
    pub message: String,
}

/// A file the tool touched, and in what capacity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Artifact {
    /// Repo-relative URI. Flattened from SARIF's `artifact.location.uri`.
    pub location_uri: String,
    /// Raw SARIF role strings. Compared against [`ROLE_ANALYSIS_TARGET`]; kept
    /// as strings because a tool may legitimately emit roles we do not model,
    /// and dropping them would silently discard contract information. An
    /// artifact with no roles is legal SARIF; it simply is not a scan target.
    #[serde(default)]
    pub roles: Vec<String>,
}

/// One accusation. Nothing in this type asserts that anything is dead.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SarifResult {
    /// Stable rule identifier within the emitting tool.
    pub rule_id: String,
    /// Severity as the tool reported it.
    pub level: Level,
    /// Human-readable finding text, flattened from `result.message.text`.
    pub message: String,
    /// Where the finding is. May be empty for repo-level findings.
    #[serde(default)]
    pub locations: Vec<Location>,
    /// Versioned, content-derived fingerprints keyed by algorithm name, e.g.
    /// `judged/v1`. A `BTreeMap` so serialization is deterministic: the
    /// baseline file is committed and diffed by humans (§9.4).
    #[serde(default)]
    pub partial_fingerprints: BTreeMap<String, String>,
    /// New / unchanged / updated / absent relative to [`Run::baseline_guid`].
    pub baseline_state: Option<BaselineState>,
    /// Suppressions carried on the finding — the keep DSL (§5.3).
    #[serde(default)]
    pub suppressions: Vec<Suppression>,
}

/// A location within an artifact.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Location {
    /// Repo-relative URI of the artifact this finding is in.
    pub uri: String,
    /// 1-based start line, **for human display only**. §9.2 is explicit that
    /// fingerprints must be content-derived and never line-based, or every
    /// reformat resets the stability clock. Do not feed this into an identity.
    pub start_line: Option<u32>,
}

/// A suppression attached to a finding.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Suppression {
    /// Whether the suppression came from the source or from an external store.
    pub kind: SuppressionKind,
    /// Review state. `UnderReview` and `Rejected` are not amnesty.
    pub status: SuppressionStatus,
    /// Why. Required by the keep DSL for anything that should survive review.
    pub justification: Option<String>,
}

/// SARIF result severity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Level {
    None,
    Note,
    Warning,
    Error,
}

/// A finding's state relative to a baseline. §9.2: this *is* the stability
/// window natively, and it is also the ratchet (§9.14).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BaselineState {
    New,
    Unchanged,
    Updated,
    Absent,
}

/// Where a suppression is recorded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SuppressionKind {
    InSource,
    External,
}

/// Review state of a suppression.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SuppressionStatus {
    Accepted,
    UnderReview,
    Rejected,
}

/// Whether a run's output may be trusted, and how far.
///
/// The distinction is the point: `Degraded` is not `Failed`. §9.2 requires that
/// partial degradation *caps the tier* for affected paths rather than
/// discarding the run, while a failed run must contribute nothing at all. §3.7
/// is why the third state has to exist at all: *"every catastrophic failure mode
/// in this space shares one signature: coverage reports ~0% for everything"* — a
/// run that looks clean because it did nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunHealth {
    /// Ran to completion over enough of the repository to be believed.
    Healthy,
    /// Usable, but something was lost: a disabled rule, a short scanned
    /// universe. Reasons are human-readable and are surfaced verbatim.
    Degraded { reasons: Vec<String> },
    /// Must not contribute evidence in either direction.
    Failed { reasons: Vec<String> },
}

/// The SARIF wire spelling of a level, so that a reason string quotes the same
/// word the tool emitted rather than a Rust-flavoured rendering of it.
fn wire_level(level: Level) -> &'static str {
    match level {
        Level::None => "none",
        Level::Note => "note",
        Level::Warning => "warning",
        Level::Error => "error",
    }
}

/// Classify a run against the §9.2 contract.
///
/// `expected_analysis_targets` is the number of candidate files the caller
/// believes the tool should have scanned; the ratio of declared
/// `analysisTarget` artifacts to that number is checked against
/// [`ANALYSIS_TARGET_RATIO_FLOOR`].
///
/// Three gates, applied to every run, in the order the research demands:
///
/// 1. **The health bit.** Any invocation with `executionSuccessful == false`
///    fails the run, and so does a run that recorded no invocation at all —
///    §6.20's central rule is that "no data" is a distinct state from "zero
///    findings", and absence is the purest form of no data.
/// 2. **Notifications.** Any `error` or `warning` notification degrades the
///    run and its message is carried verbatim into `reasons`. §9.2: partial
///    degradation must cap the tier, not be discarded. `note` and `none` are
///    informational; degrading on them would make every run degraded and
///    destroy the signal the tier carries.
/// 3. **The positive control.** The declared `analysisTarget` set must cover at
///    least [`ANALYSIS_TARGET_RATIO_FLOOR`] of the expected candidates. This is
///    §9.2's "single most valuable contract clause" and the only defence
///    against the shape that actually deletes code: a tool that ran to
///    completion over almost nothing and therefore found almost nothing.
///
/// `Failed` outranks `Degraded`, and a failed run still carries the degradation
/// reasons — they are usually *why* it died.
///
/// This function never inspects [`Run::results`]. Health is a property of the
/// run, not of what it found: zero findings from a healthy run is evidence,
/// and zero findings from a failed run is nothing at all.
pub fn assess_run_health(run: &Run, expected_analysis_targets: usize) -> RunHealth {
    let tool = &run.tool.name;
    let mut failures: Vec<String> = Vec::new();
    let mut degradations: Vec<String> = Vec::new();

    // Gate 1 — the health bit. Note what is *not* here: any reading of an exit
    // code. §9.2 quotes the SARIF spec's own note that "not all programs exit
    // with an exit code of 0 on success and non-0 on failure", with a worked
    // example of exitCode:1 alongside executionSuccessful:true. The adapter
    // computes this bit; the orchestrator only ever reads it.
    if run.invocations.is_empty() {
        failures.push(format!(
            "tool `{tool}` recorded no invocation, so `executionSuccessful` was never asserted; \
             absence is not success (§6.20)"
        ));
    }
    for (index, invocation) in run.invocations.iter().enumerate() {
        if !invocation.execution_successful {
            // One bad invocation poisons the run. Believing the others would
            // let a tool hide a crashed pass behind a clean one.
            failures.push(format!(
                "tool `{tool}` invocation {index} reported `executionSuccessful`: false"
            ));
        }

        // Gate 2 — degradation. The message is carried verbatim because it is
        // the operator's only clue about *which* part of the repository the run
        // stopped covering (§6.20: "ERROR: Error loading vite.config.ts").
        for notification in &invocation.tool_execution_notifications {
            match notification.level {
                Level::Error | Level::Warning => degradations.push(format!(
                    "tool `{tool}` emitted a {} notification: {}",
                    wire_level(notification.level),
                    notification.message
                )),
                Level::Note | Level::None => {}
            }
        }
    }

    // Gate 3 — the static positive control (§9.2). Count only artifacts the
    // tool declared it was *instructed to scan*; roles such as `traced` or
    // `generated` say nothing about coverage.
    let declared = run
        .artifacts
        .iter()
        .filter(|artifact| {
            artifact
                .roles
                .iter()
                .any(|role| role == ROLE_ANALYSIS_TARGET)
        })
        .count();

    if expected_analysis_targets == 0 {
        // No expectation means no control: there is nothing to compare the
        // scanned set against, so the run's silence cannot be given meaning.
        // §6.20 requires an explicit positive assertion that the artifact was
        // collected before a zero may be counted, and an unknown universe can
        // never supply one. Deliberately unreachable as `Healthy`.
        degradations.push(format!(
            "tool `{tool}` declared {declared} `{ROLE_ANALYSIS_TARGET}` artifacts but the expected \
             candidate count is 0, so coverage cannot be validated (§9.2)"
        ));
    } else {
        // The gate is `|analysisTarget| >= floor x |candidates|`, so exactly
        // 80% passes; the boundary is pinned by test because an off-by-one here
        // silently changes how much of a repository may go unscanned in silence.
        let required = ANALYSIS_TARGET_RATIO_FLOOR * expected_analysis_targets as f64;
        if (declared as f64) < required {
            let ratio = declared as f64 / expected_analysis_targets as f64;
            degradations.push(format!(
                "tool `{tool}` declared {declared} of {expected_analysis_targets} expected \
                 `{ROLE_ANALYSIS_TARGET}` artifacts (ratio {ratio:.2}, floor \
                 {ANALYSIS_TARGET_RATIO_FLOOR:.1}); its silence covers only what it scanned (§9.2)"
            ));
        }
    }

    if !failures.is_empty() {
        failures.extend(degradations);
        RunHealth::Failed { reasons: failures }
    } else if !degradations.is_empty() {
        RunHealth::Degraded {
            reasons: degradations,
        }
    } else {
        RunHealth::Healthy
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn result_uses_sarif_wire_names_for_the_fields_the_orchestrator_reads() {
        let mut fingerprints = BTreeMap::new();
        fingerprints.insert("judged/v1".to_string(), "deadbeef".to_string());
        let result = SarifResult {
            rule_id: "unused-export".to_string(),
            level: Level::Warning,
            message: "export `foo` is never imported".to_string(),
            locations: vec![Location {
                uri: "src/foo.ts".to_string(),
                start_line: Some(12),
            }],
            partial_fingerprints: fingerprints,
            baseline_state: Some(BaselineState::New),
            suppressions: vec![Suppression {
                kind: SuppressionKind::InSource,
                status: SuppressionStatus::UnderReview,
                justification: None,
            }],
        };

        let json = serde_json::to_string(&result).expect("SarifResult must serialize");

        assert!(json.contains(r#""ruleId":"unused-export""#), "got {json}");
        assert!(json.contains(r#""partialFingerprints""#), "got {json}");
        assert!(json.contains(r#""baselineState":"new""#), "got {json}");
        assert!(json.contains(r#""kind":"inSource""#), "got {json}");
        assert!(json.contains(r#""status":"underReview""#), "got {json}");
    }

    #[test]
    fn run_uses_sarif_wire_names_for_the_health_and_universe_fields() {
        let run = Run {
            tool: Tool {
                name: "knip".to_string(),
                version: Some("5.0.0".to_string()),
            },
            invocations: vec![Invocation {
                execution_successful: true,
                tool_execution_notifications: vec![Notification {
                    level: Level::Error,
                    message: "rule disabled; run continues".to_string(),
                }],
            }],
            artifacts: vec![Artifact {
                location_uri: "src/foo.ts".to_string(),
                roles: vec![ROLE_ANALYSIS_TARGET.to_string()],
            }],
            results: vec![],
            baseline_guid: Some("11111111-2222-3333-4444-555555555555".to_string()),
        };

        let json = serde_json::to_string(&run).expect("Run must serialize");

        assert!(json.contains(r#""executionSuccessful":true"#), "got {json}");
        assert!(
            json.contains(r#""toolExecutionNotifications""#),
            "got {json}"
        );
        assert!(json.contains(r#""baselineGuid""#), "got {json}");
        assert!(json.contains(r#""roles":["analysisTarget"]"#), "got {json}");
    }
}
