//! Wire compatibility for the SARIF subset.
//!
//! The logs below are hand-written to resemble what a ruff adapter and a knip
//! adapter actually emit, in Judged's *normalized projection* (the adapter owns
//! the mapping down from nested wire SARIF — see the `sarif` module docs). They
//! carry fields we do not model on purpose: real tools emit far more than this
//! subset, and a parser that rejects a log because it contains `$schema` or
//! `ruleIndex` turns a healthy run into no run at all — which §6.20 says is the
//! failure mode that ends in mass deletion.
//!
//! The mirror-image rule also holds: leniency belongs in the *parser*, never in
//! the *assessment*. A log missing `invocations` parses fine and then assesses
//! as `Failed`, because absence is not success.

use judged_core::sarif::{
    assess_run_health, BaselineState, Level, RunHealth, SarifLog, SuppressionKind,
    SuppressionStatus, ROLE_ANALYSIS_TARGET,
};

/// Resembles `ruff --output-format sarif` over a small Python package, after
/// adapter normalization. Ruff is §9.2's model contract: it distinguishes
/// "violations found" (exit 1) from "abnormal termination" (exit 2), so its
/// adapter can compute a real health bit. Note the omitted empty collections
/// and the unmodelled keys.
const RUFF_LIKE_LOG: &str = r#"{
  "$schema": "https://json.schemastore.org/sarif-2.1.0.json",
  "version": "2.1.0",
  "runs": [
    {
      "tool": { "name": "ruff", "version": "0.5.6", "informationUri": "https://docs.astral.sh/ruff" },
      "invocations": [
        { "executionSuccessful": true, "exitCode": 1, "commandLine": "ruff check --output-format sarif ." }
      ],
      "artifacts": [
        { "locationUri": "src/pkg/__init__.py", "roles": ["analysisTarget"], "length": 0 },
        { "locationUri": "src/pkg/cli.py", "roles": ["analysisTarget"], "length": 812 },
        { "locationUri": "src/pkg/legacy.py", "roles": ["analysisTarget", "traced"], "length": 240 }
      ],
      "results": [
        {
          "ruleId": "F401",
          "ruleIndex": 3,
          "level": "warning",
          "message": "`os` imported but unused",
          "locations": [{ "uri": "src/pkg/legacy.py", "startLine": 4, "startColumn": 8 }],
          "partialFingerprints": { "judged/v1": "e3b0c44298fc1c149afbf4c8996fb924" }
        }
      ]
    }
  ]
}"#;

/// Resembles a knip run. knip is one of the tools §9.2 names as conflating
/// "clean" with "crashed before doing anything", so its adapter must lean on
/// the notification channel and the analysisTarget set. This log is the
/// realistic degraded case: it finished, it reported one finding, and it also
/// admits it could not load a config file.
const KNIP_LIKE_LOG: &str = r#"{
  "version": "2.1.0",
  "runs": [
    {
      "tool": { "name": "knip", "version": "5.27.0" },
      "invocations": [
        {
          "executionSuccessful": true,
          "toolExecutionNotifications": [
            {
              "level": "error",
              "message": "Error loading vite.config.ts: Cannot find module '@/alias'",
              "descriptor": { "id": "knip/plugin-load-failure" }
            },
            { "level": "note", "message": "analyzed 2 workspaces" }
          ]
        }
      ],
      "artifacts": [
        { "locationUri": "src/index.ts", "roles": ["analysisTarget"] },
        { "locationUri": "src/unused.ts", "roles": ["analysisTarget"] },
        { "locationUri": "dist/index.js", "roles": ["generated"] }
      ],
      "results": [
        {
          "ruleId": "unused-export",
          "level": "warning",
          "message": "Unused export: renderLegacyBanner",
          "locations": [{ "uri": "src/unused.ts", "startLine": 17 }],
          "partialFingerprints": { "judged/v1": "9f86d081884c7d659a2feaa0c55ad015" },
          "baselineState": "unchanged",
          "suppressions": [
            {
              "kind": "external",
              "status": "accepted",
              "justification": "public API of the embed bundle; see ADR-0031",
              "guid": "b0a5f6c2-0000-4000-8000-000000000001"
            }
          ]
        }
      ],
      "baselineGuid": "7f9c2ba4-e88f-509e-9b4a-1c5f0a3e9d21"
    }
  ]
}"#;

fn parse(log: &str) -> SarifLog {
    serde_json::from_str::<SarifLog>(log).expect("fixture log must deserialize")
}

/// Deserialize, re-serialize, deserialize again: proves nothing we model is
/// lost on the way out, independent of key ordering.
fn assert_stable_round_trip(log: &SarifLog) {
    let json = serde_json::to_string(log).expect("log must serialize");
    let reparsed: SarifLog = serde_json::from_str(&json).expect("our own output must parse");
    assert_eq!(log, &reparsed, "round trip changed the log");
}

#[test]
fn ruff_like_log_round_trips_and_ignores_unmodelled_fields() {
    let log = parse(RUFF_LIKE_LOG);
    assert_stable_round_trip(&log);

    let run = &log.runs[0];
    assert_eq!(run.tool.name, "ruff");
    assert_eq!(run.tool.version.as_deref(), Some("0.5.6"));
    assert!(run.invocations[0].execution_successful);
    // Omitted entirely in the fixture: absence of degradation is not an error.
    assert!(run.invocations[0].tool_execution_notifications.is_empty());
    assert_eq!(run.artifacts.len(), 3);
    assert_eq!(run.results[0].rule_id, "F401");
    assert_eq!(run.results[0].level, Level::Warning);
    assert_eq!(run.results[0].locations[0].start_line, Some(4));
    assert_eq!(
        run.results[0]
            .partial_fingerprints
            .get("judged/v1")
            .map(String::as_str),
        Some("e3b0c44298fc1c149afbf4c8996fb924")
    );
    // Absent in the fixture, and absent is not an error for these.
    assert_eq!(run.results[0].baseline_state, None);
    assert!(run.results[0].suppressions.is_empty());
    assert_eq!(run.baseline_guid, None);
}

#[test]
fn knip_like_log_round_trips_with_notifications_and_suppressions() {
    let log = parse(KNIP_LIKE_LOG);
    assert_stable_round_trip(&log);

    let run = &log.runs[0];
    let notifications = &run.invocations[0].tool_execution_notifications;
    assert_eq!(notifications.len(), 2);
    assert_eq!(notifications[0].level, Level::Error);
    assert!(notifications[0].message.contains("vite.config.ts"));

    assert_eq!(
        run.results[0].baseline_state,
        Some(BaselineState::Unchanged)
    );
    let suppression = &run.results[0].suppressions[0];
    assert_eq!(suppression.kind, SuppressionKind::External);
    assert_eq!(suppression.status, SuppressionStatus::Accepted);
    assert!(suppression.justification.is_some());
    assert_eq!(
        run.baseline_guid.as_deref(),
        Some("7f9c2ba4-e88f-509e-9b4a-1c5f0a3e9d21")
    );
}

/// The knip fixture is the canonical §6.20 shape: the tool exited successfully
/// and produced findings, while telling us in a channel most consumers discard
/// that a config file failed to load — so the roots it would have contributed
/// are missing. It must not read as a trustworthy run.
#[test]
fn knip_like_log_with_a_load_failure_notification_assesses_as_degraded() {
    let log = parse(KNIP_LIKE_LOG);
    let analysis_targets = log.runs[0]
        .artifacts
        .iter()
        .filter(|a| a.roles.iter().any(|r| r == ROLE_ANALYSIS_TARGET))
        .count();
    assert_eq!(analysis_targets, 2, "fixture sanity");

    let health = assess_run_health(&log.runs[0], analysis_targets);

    match health {
        RunHealth::Degraded { reasons } => assert!(
            reasons.iter().any(|r| r.contains("vite.config.ts")),
            "the load failure must be surfaced verbatim, got {reasons:?}"
        ),
        other => panic!("expected Degraded, got {other:?}"),
    }
}

/// The degenerate log: a tool that died before writing anything but its header.
/// It must parse — refusing to parse would only move the failure somewhere the
/// orchestrator handles less carefully — and it must assess as `Failed`, never
/// as a clean run with zero findings.
#[test]
fn log_without_invocations_or_artifacts_parses_and_assesses_as_failed() {
    let log = parse(r#"{ "runs": [ { "tool": { "name": "vulture" } } ] }"#);

    let run = &log.runs[0];
    assert_eq!(run.tool.version, None);
    assert!(run.invocations.is_empty());
    assert!(run.artifacts.is_empty());
    assert!(run.results.is_empty());

    assert!(
        matches!(assess_run_health(run, 40), RunHealth::Failed { .. }),
        "a log with no invocation must be Failed"
    );
}

/// `executionSuccessful` is required by the SARIF spec, so a log without it is
/// malformed — but §6.20 forbids resolving the ambiguity in the tool's favour.
/// It parses as "not successful", which assesses as `Failed`.
#[test]
fn missing_execution_successful_is_not_read_as_success() {
    let log = parse(
        r#"{
          "runs": [
            {
              "tool": { "name": "ts-prune" },
              "invocations": [ { "toolExecutionNotifications": [] } ],
              "artifacts": [ { "locationUri": "src/a.ts", "roles": ["analysisTarget"] } ]
            }
          ]
        }"#,
    );

    assert!(!log.runs[0].invocations[0].execution_successful);
    assert!(matches!(
        assess_run_health(&log.runs[0], 1),
        RunHealth::Failed { .. }
    ));
}

/// A role we do not model must survive a round trip. Dropping it would discard
/// contract information the next version of Judged may need to read.
#[test]
fn unmodelled_artifact_roles_survive_a_round_trip() {
    let log = parse(KNIP_LIKE_LOG);
    let json = serde_json::to_string(&log).expect("log must serialize");
    assert!(json.contains(r#""generated""#), "got {json}");
}

/// Wire spellings are load-bearing: these strings appear in files other tools
/// write and in the committed baseline. Pinned in both directions.
#[test]
fn enum_wire_spellings_are_camel_case_in_both_directions() {
    let log = parse(KNIP_LIKE_LOG);
    let json = serde_json::to_string(&log).expect("log must serialize");

    for expected in [
        r#""level":"error""#,
        r#""level":"note""#,
        r#""level":"warning""#,
        r#""baselineState":"unchanged""#,
        r#""kind":"external""#,
        r#""status":"accepted""#,
        r#""executionSuccessful":true"#,
        r#""toolExecutionNotifications""#,
        r#""partialFingerprints""#,
        r#""locationUri""#,
        r#""baselineGuid""#,
    ] {
        assert!(json.contains(expected), "missing {expected} in {json}");
    }
}

/// Unknown *fields* are ignored; unknown *enum values* are not. A `level` we
/// cannot interpret must fail loudly rather than degrade to something milder —
/// silently downgrading an unrecognized severity is how a fatal notification
/// becomes invisible.
#[test]
fn unrecognized_level_value_is_rejected() {
    let err = serde_json::from_str::<SarifLog>(
        r#"{
          "runs": [
            {
              "tool": { "name": "knip" },
              "invocations": [
                { "executionSuccessful": true,
                  "toolExecutionNotifications": [ { "level": "catastrophic", "message": "x" } ] }
              ]
            }
          ]
        }"#,
    )
    .expect_err("an unknown severity must not parse");

    assert!(
        err.to_string().contains("catastrophic") || err.to_string().contains("unknown variant"),
        "error should name the bad variant, got {err}"
    );
}
