//! End-to-end behaviour of the ratchet, exercised only through the public API.
//!
//! These live outside `src/` deliberately: the ratchet's contract is what CI
//! and the CLI can observe, and every test here is written against that surface
//! rather than against internals.

use judged_core::git::Repo;
use judged_core::sarif::{
    Artifact, BaselineState, Invocation, Level, Location, Notification, Run, SarifResult, Tool,
    ROLE_ANALYSIS_TARGET,
};
use judged_ratchet::baseline::BASELINE_PATH;
use judged_ratchet::{
    baseline_state, detect_rot, exit_code, ratchet, Baseline, BaselineEntry, RatchetOutcome,
    RotReason,
};
use tempfile::{Builder, TempDir};

// ---------------------------------------------------------------------------
// Scratch space
// ---------------------------------------------------------------------------

/// A throwaway directory that deletes itself when the test ends.
///
/// The label is only a prefix on the directory name, so a test that leaves one
/// behind after a hard abort still says which case it came from.
fn scratch(label: &str) -> TempDir {
    Builder::new()
        .prefix(&format!("judged-ratchet-{label}-"))
        .tempdir()
        .expect("scratch directory must be creatable")
}

// ---------------------------------------------------------------------------
// Builders
// ---------------------------------------------------------------------------

fn result(rule_id: &str, uri: &str, start_line: Option<u32>, message: &str) -> SarifResult {
    SarifResult {
        rule_id: rule_id.to_string(),
        level: Level::Warning,
        message: message.to_string(),
        locations: vec![Location {
            uri: uri.to_string(),
            start_line,
        }],
        partial_fingerprints: Default::default(),
        baseline_state: None,
        suppressions: Vec::new(),
    }
}

/// Attach an adapter-supplied `judged/v1` fingerprint, as a well-behaved
/// adapter would.
fn fingerprinted(mut r: SarifResult, hex: &str) -> SarifResult {
    r.partial_fingerprints
        .insert("judged/v1".to_string(), hex.to_string());
    r
}

fn entry(fingerprint: &str, uri: &str) -> BaselineEntry {
    BaselineEntry {
        fingerprint: fingerprint.to_string(),
        rule_id: "unused-export".to_string(),
        uri: uri.to_string(),
        symbol: None,
        first_seen: "2026-07-31T00:00:00Z".to_string(),
        expires: None,
        justification: None,
    }
}

/// A run whose invocation succeeded and whose declared `analysisTarget` set
/// clears the §9.2 ratio floor, so `assess_run_health` returns `Healthy`.
fn healthy_run(results: Vec<SarifResult>) -> Run {
    Run {
        tool: Tool {
            name: "knip".to_string(),
            version: Some("5.0.0".to_string()),
        },
        invocations: vec![Invocation {
            execution_successful: true,
            tool_execution_notifications: Vec::new(),
        }],
        artifacts: (0..EXPECTED_TARGETS)
            .map(|n| Artifact {
                location_uri: format!("src/f{n}.ts"),
                roles: vec![ROLE_ANALYSIS_TARGET.to_string()],
            })
            .collect(),
        results,
        baseline_guid: None,
    }
}

/// The §6.20 scenario: the tool ran, reported that it did not succeed, and
/// therefore its result set — however plausible it looks — means nothing.
fn crashed_run(results: Vec<SarifResult>) -> Run {
    let mut run = healthy_run(results);
    run.invocations = vec![Invocation {
        execution_successful: false,
        tool_execution_notifications: vec![Notification {
            level: Level::Error,
            message: "Error loading vite.config.ts".to_string(),
        }],
    }];
    run
}

/// A run that succeeded but declared only half the `analysisTarget` set it was
/// expected to, so the §9.2 positive control caps it at `Degraded`.
fn degraded_run(results: Vec<SarifResult>) -> Run {
    let mut run = healthy_run(results);
    run.artifacts.truncate(EXPECTED_TARGETS / 2);
    run
}

/// The number of candidate files [`healthy_run`] declares it scanned. Passed as
/// `expected_analysis_targets` so that a healthy run really is healthy.
const EXPECTED_TARGETS: usize = 10;

/// An instant every fixture treats as "now".
const NOW: &str = "2026-07-31T12:00:00Z";

fn repo(scratch: &TempDir) -> Repo {
    Repo::init(scratch.path()).expect("git init must succeed in scratch space")
}

fn touch(scratch: &TempDir, rel: &str) {
    let path = scratch.path().join(rel);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("fixture parent directory must be creatable");
    }
    std::fs::write(&path, b"// fixture\n").expect("fixture file must be writable");
}

// ---------------------------------------------------------------------------
// The committed baseline file (§9.4)
// ---------------------------------------------------------------------------

#[test]
fn baseline_lives_at_the_committed_path() {
    assert_eq!(BASELINE_PATH, ".judged/baseline.jsonl");
}

#[test]
fn a_missing_baseline_file_is_an_empty_baseline() {
    // First run on a repository must not need a setup step.
    let scratch = scratch("missing");

    let baseline = Baseline::load(&scratch.path().join(BASELINE_PATH))
        .expect("a missing baseline file must not be an error");

    assert_eq!(baseline.entries(), &[] as &[BaselineEntry]);
}

#[test]
fn save_then_load_round_trips() {
    let scratch = scratch("roundtrip");
    let path = scratch.path().join(BASELINE_PATH);
    let mut with_options = entry("judged/v1:beef", "src/b.ts");
    with_options.symbol = Some("pkg.Widget".to_string());
    with_options.expires = Some("2026-12-31".to_string());
    with_options.justification = Some("pending API removal".to_string());
    let original = Baseline::new(vec![entry("judged/v1:00ff", "src/a.ts"), with_options]);

    original
        .save(&path)
        .expect("save must create .judged/ as needed");
    let loaded = Baseline::load(&path).expect("a baseline we just wrote must load");

    assert_eq!(loaded.entries(), original.entries());
}

#[test]
fn saved_baseline_is_one_line_per_entry_and_byte_stable() {
    // The file is committed and reviewed (§9.4). Byte instability across runs
    // would put unrelated churn in every PR that touches it, which is how a
    // baseline stops being read and becomes the permanent amnesty list §9.14
    // warns about.
    let scratch = scratch("stable");
    let baseline = Baseline::new(vec![
        entry("judged/v1:00ff", "src/a.ts"),
        entry("judged/v1:beef", "src/b.ts"),
    ]);

    let first = scratch.path().join("first.jsonl");
    let second = scratch.path().join("second.jsonl");
    baseline.save(&first).expect("save must succeed");
    baseline.save(&second).expect("save must succeed");
    let first_bytes = std::fs::read(&first).expect("written baseline must be readable");
    let second_bytes = std::fs::read(&second).expect("written baseline must be readable");

    assert_eq!(
        first_bytes, second_bytes,
        "two saves must agree byte for byte"
    );
    let text = String::from_utf8(first_bytes).expect("baseline must be UTF-8");
    assert!(text.ends_with('\n'), "got {text:?}");
    assert_eq!(text.lines().count(), 2, "got {text:?}");

    // And a load/save cycle must be a fixed point, or `judged ratchet --write`
    // would rewrite lines it did not change.
    let third = scratch.path().join("third.jsonl");
    Baseline::load(&first)
        .expect("baseline must load")
        .save(&third)
        .expect("save must succeed");
    assert_eq!(
        std::fs::read(&first).expect("readable"),
        std::fs::read(&third).expect("readable"),
        "load/save must be a fixed point"
    );
}

#[test]
fn a_malformed_line_is_an_error_not_a_silent_drop() {
    // Silently dropping an unparseable line un-accepts findings and fails CI
    // for reasons nobody can explain (AGENTS.md rule 12, fail loudly).
    let scratch = scratch("malformed");
    let path = scratch.path().join("baseline.jsonl");
    std::fs::write(
        &path,
        b"{\"fingerprint\":\"judged/v1:00ff\",\"rule_id\":\"r\",\"uri\":\"src/a.ts\",\"first_seen\":\"2026-07-31T00:00:00Z\"}\n<<<<<<< HEAD\n",
    )
    .expect("fixture must be writable");

    let err = Baseline::load(&path).expect_err("a conflict marker must not parse as a baseline");

    let rendered = err.to_string();
    assert!(rendered.contains("line 2"), "got {rendered}");
}

#[test]
fn an_entry_without_a_fingerprint_is_rejected() {
    // The fingerprint is the join key. An empty one silently matches nothing,
    // so the entry would be indistinguishable from rot forever.
    let scratch = scratch("nofingerprint");
    let path = scratch.path().join("baseline.jsonl");
    std::fs::write(
        &path,
        b"{\"fingerprint\":\"\",\"rule_id\":\"r\",\"uri\":\"src/a.ts\",\"first_seen\":\"2026-07-31T00:00:00Z\"}\n",
    )
    .expect("fixture must be writable");

    let err = Baseline::load(&path).expect_err("an empty fingerprint must be rejected");

    assert!(err.to_string().contains("fingerprint"), "got {err}");
}

#[test]
fn from_results_prefers_the_adapters_own_fingerprint() {
    let results = vec![fingerprinted(
        result(
            "unused-export",
            "src/a.ts",
            Some(12),
            "`foo` is never imported",
        ),
        "00ff",
    )];

    let baseline = Baseline::from_results(&results, "2026-07-31T00:00:00Z");

    assert_eq!(baseline.entries().len(), 1);
    assert_eq!(baseline.entries()[0].fingerprint, "judged/v1:00ff");
    assert_eq!(baseline.entries()[0].rule_id, "unused-export");
    assert_eq!(baseline.entries()[0].uri, "src/a.ts");
    assert_eq!(baseline.entries()[0].first_seen, "2026-07-31T00:00:00Z");
}

#[test]
fn from_results_derives_a_fingerprint_when_the_adapter_emitted_none() {
    let results = vec![result(
        "unused-export",
        "src/a.ts",
        Some(12),
        "`foo` is never imported",
    )];

    let baseline = Baseline::from_results(&results, "2026-07-31T00:00:00Z");

    let derived = &baseline.entries()[0].fingerprint;
    assert!(derived.starts_with("judged/v1:"), "got {derived}");
    assert_eq!(derived.len(), "judged/v1:".len() + 64, "got {derived}");
}

#[test]
fn from_results_does_not_write_the_same_finding_twice() {
    // Two adapters, or one adapter run over overlapping targets, can report the
    // same finding twice. A committed file with duplicate lines is review noise
    // and makes the rot report double-count.
    let one = fingerprinted(result("unused-export", "src/a.ts", Some(12), "m"), "00ff");
    let two = fingerprinted(result("unused-export", "src/a.ts", Some(99), "m"), "00ff");

    let baseline = Baseline::from_results(&[one, two], "2026-07-31T00:00:00Z");

    assert_eq!(baseline.entries().len(), 1);
}

// ---------------------------------------------------------------------------
// Rot detection (§5.3 generalized, named as the ratchet's failure mode in §9.14)
// ---------------------------------------------------------------------------

#[test]
fn an_entry_that_still_matches_is_not_rot() {
    let scratch = scratch("rot-live");
    let repo = repo(&scratch);
    touch(&scratch, "src/a.ts");
    let run = healthy_run(vec![fingerprinted(
        result("unused-export", "src/a.ts", Some(12), "m"),
        "00ff",
    )]);
    let baseline = Baseline::new(vec![entry("judged/v1:00ff", "src/a.ts")]);

    assert_eq!(detect_rot(&baseline, &run, &repo, NOW), Vec::new());
}

#[test]
fn an_entry_no_finding_carries_is_rot() {
    // Either the finding was fixed or our own analysis stopped producing it.
    // Both mean the amnesty now protects nothing, and §5.3 is explicit: "a
    // suppression list without rot detection is the off switch".
    let scratch = scratch("rot-unmatched");
    let repo = repo(&scratch);
    touch(&scratch, "src/a.ts");
    let run = healthy_run(Vec::new());
    let baseline = Baseline::new(vec![entry("judged/v1:00ff", "src/a.ts")]);

    assert_eq!(
        detect_rot(&baseline, &run, &repo, NOW),
        vec![RotReason::NeverMatched {
            fingerprint: "judged/v1:00ff".to_string()
        }]
    );
}

#[test]
fn an_entry_whose_file_is_gone_is_rot() {
    let scratch = scratch("rot-gone");
    let repo = repo(&scratch);
    let run = healthy_run(Vec::new());
    let baseline = Baseline::new(vec![entry("judged/v1:00ff", "src/deleted.ts")]);

    assert_eq!(
        detect_rot(&baseline, &run, &repo, NOW),
        vec![RotReason::ReferentGone {
            uri: "src/deleted.ts".to_string()
        }],
        "a deleted referent must be named as such, not reported as an unmatched fingerprint"
    );
}

#[test]
fn an_entry_past_its_expiry_is_rot() {
    let scratch = scratch("rot-expired");
    let repo = repo(&scratch);
    touch(&scratch, "src/a.ts");
    let run = healthy_run(vec![fingerprinted(
        result("unused-export", "src/a.ts", Some(12), "m"),
        "00ff",
    )]);
    let mut e = entry("judged/v1:00ff", "src/a.ts");
    e.expires = Some("2026-07-30".to_string());
    let baseline = Baseline::new(vec![e]);

    assert_eq!(
        detect_rot(&baseline, &run, &repo, NOW),
        vec![RotReason::Expired {
            fingerprint: "judged/v1:00ff".to_string(),
            expires: "2026-07-30".to_string()
        }]
    );
}

#[test]
fn an_expiry_still_in_the_future_is_not_rot() {
    let scratch = scratch("rot-not-yet");
    let repo = repo(&scratch);
    touch(&scratch, "src/a.ts");
    let run = healthy_run(vec![fingerprinted(
        result("unused-export", "src/a.ts", Some(12), "m"),
        "00ff",
    )]);
    let mut e = entry("judged/v1:00ff", "src/a.ts");
    e.expires = Some("2026-08-01".to_string());
    let baseline = Baseline::new(vec![e]);

    assert_eq!(detect_rot(&baseline, &run, &repo, NOW), Vec::new());
}

#[test]
fn an_expiry_that_cannot_be_evaluated_is_rot() {
    // The baseline is hand-edited. `2026/08/01` and `next quarter` will be
    // typed into it. Comparing them as ISO-8601 would silently grant a longer
    // amnesty than the author asked for, so an unevaluable expiry is rot: the
    // entry has stopped earning its place until a human fixes the line.
    let scratch = scratch("rot-garbage-expiry");
    let repo = repo(&scratch);
    touch(&scratch, "src/a.ts");
    let run = healthy_run(vec![fingerprinted(
        result("unused-export", "src/a.ts", Some(12), "m"),
        "00ff",
    )]);
    let mut e = entry("judged/v1:00ff", "src/a.ts");
    e.expires = Some("next quarter".to_string());
    let baseline = Baseline::new(vec![e]);

    assert_eq!(
        detect_rot(&baseline, &run, &repo, NOW),
        vec![RotReason::Expired {
            fingerprint: "judged/v1:00ff".to_string(),
            expires: "next quarter".to_string()
        }]
    );
}

#[test]
fn a_deleted_referent_outranks_the_unmatched_fingerprint_it_implies() {
    // A deleted file always drags its fingerprint out of the run too. Emitting
    // both reasons would double every deletion in the report; the file being
    // gone is the actionable one, because it can never match again.
    let scratch = scratch("rot-precedence");
    let repo = repo(&scratch);
    let run = healthy_run(Vec::new());
    let mut e = entry("judged/v1:00ff", "src/deleted.ts");
    e.expires = Some("2026-07-30".to_string());
    let baseline = Baseline::new(vec![e]);

    assert_eq!(
        detect_rot(&baseline, &run, &repo, NOW),
        vec![RotReason::ReferentGone {
            uri: "src/deleted.ts".to_string()
        }],
        "exactly one reason per entry, most specific first"
    );
}

#[test]
fn an_entry_with_no_uri_is_never_reported_as_a_missing_file() {
    // Project-scoped findings (an unused dependency, say) have no artifact.
    // Joining `repo_root` with "" yields the repo root, which exists, but the
    // check must not depend on that accident.
    let scratch = scratch("rot-no-uri");
    let repo = repo(&scratch);
    let run = healthy_run(vec![fingerprinted(
        result("unused-dependency", "", None, "crate `prost` is unused"),
        "00ff",
    )]);
    let baseline = Baseline::new(vec![entry("judged/v1:00ff", "")]);

    assert_eq!(detect_rot(&baseline, &run, &repo, NOW), Vec::new());
}

#[test]
fn an_empty_baseline_has_no_rot() {
    // The run immediately after `--write` must be clean, or the ratchet is
    // unusable on day one.
    let scratch = scratch("rot-empty");
    let repo = repo(&scratch);
    let run = healthy_run(vec![result("unused-export", "src/a.ts", Some(1), "m")]);

    assert_eq!(
        detect_rot(&Baseline::default(), &run, &repo, NOW),
        Vec::new()
    );
}

// ---------------------------------------------------------------------------
// Refusing unhealthy input (§6.20, §9.2) — the reason the ratchet exists
// ---------------------------------------------------------------------------

#[test]
fn a_baseline_written_from_a_crashed_run_is_refused_not_believed() {
    // The disarming scenario, end to end. A crashed analyzer emits zero
    // results; recording that as the baseline and then checking against it
    // would leave a permanently green gate that nobody ever looks at again.
    let scratch = scratch("disarm");
    let repo = repo(&scratch);
    let crashed = crashed_run(Vec::new());

    let written = Baseline::from_results(&crashed.results, NOW);
    let path = scratch.path().join(BASELINE_PATH);
    written.save(&path).expect("save must succeed");
    let loaded = Baseline::load(&path).expect("load must succeed");
    let outcome = ratchet(&loaded, &crashed, &repo, EXPECTED_TARGETS, NOW);

    match &outcome {
        RatchetOutcome::Refused { reason } => {
            assert!(reason.contains("executionSuccessful"), "got {reason}")
        }
        other => panic!("a crashed run must be refused, got {other:?}"),
    }
    assert_eq!(exit_code(&outcome), 2);
}

#[test]
fn a_crashed_run_cannot_report_clean_against_a_good_baseline() {
    // The sharper half of the same failure: the tool reports the findings it
    // already knew about but flags that it did not succeed. Diffing alone would
    // call that Clean and pass every PR for as long as the tool stays broken.
    let scratch = scratch("disarm-check");
    let repo = repo(&scratch);
    touch(&scratch, "src/a.ts");
    let finding = fingerprinted(result("unused-export", "src/a.ts", Some(12), "m"), "00ff");
    let baseline = Baseline::new(vec![entry("judged/v1:00ff", "src/a.ts")]);

    let outcome = ratchet(
        &baseline,
        &crashed_run(vec![finding]),
        &repo,
        EXPECTED_TARGETS,
        NOW,
    );

    assert!(
        matches!(outcome, RatchetOutcome::Refused { .. }),
        "got {outcome:?}"
    );
}

#[test]
fn a_run_with_no_invocation_is_refused() {
    // Absence is not success (§6.20): a run that never asserted
    // `executionSuccessful` has told us nothing about whether it ran.
    let scratch = scratch("no-invocation");
    let repo = repo(&scratch);
    let mut run = healthy_run(Vec::new());
    run.invocations.clear();

    let outcome = ratchet(&Baseline::default(), &run, &repo, EXPECTED_TARGETS, NOW);

    assert!(
        matches!(outcome, RatchetOutcome::Refused { .. }),
        "got {outcome:?}"
    );
}

#[test]
fn a_degraded_run_still_produces_a_verdict_and_carries_the_reasons() {
    // §9.2: partial degradation caps the tier, it does not discard the run.
    // But it must never be silent — half a repository scanned, reported green,
    // is the §6.20 failure in slow motion.
    let scratch = scratch("degraded");
    let repo = repo(&scratch);
    touch(&scratch, "src/a.ts");
    let finding = fingerprinted(result("unused-export", "src/a.ts", Some(12), "m"), "00ff");
    let baseline = Baseline::new(vec![entry("judged/v1:00ff", "src/a.ts")]);

    let outcome = ratchet(
        &baseline,
        &degraded_run(vec![finding]),
        &repo,
        EXPECTED_TARGETS,
        NOW,
    );

    match &outcome {
        RatchetOutcome::Degraded { reasons, verdict } => {
            assert_eq!(**verdict, RatchetOutcome::Clean);
            assert_eq!(reasons.len(), 1, "got {reasons:?}");
            assert!(reasons[0].contains(ROLE_ANALYSIS_TARGET), "got {reasons:?}");
        }
        other => panic!("a degraded run must surface its reasons, got {other:?}"),
    }
    // Degradation is reported, not punished: the analyzers did not fail.
    assert_eq!(exit_code(&outcome), 0);
}

// ---------------------------------------------------------------------------
// The diff (§9.2 `baselineState`)
// ---------------------------------------------------------------------------

#[test]
fn a_run_that_matches_the_baseline_is_clean() {
    let scratch = scratch("clean");
    let repo = repo(&scratch);
    touch(&scratch, "src/a.ts");
    let finding = fingerprinted(result("unused-export", "src/a.ts", Some(12), "m"), "00ff");
    let baseline = Baseline::new(vec![entry("judged/v1:00ff", "src/a.ts")]);

    let outcome = ratchet(
        &baseline,
        &healthy_run(vec![finding]),
        &repo,
        EXPECTED_TARGETS,
        NOW,
    );

    assert_eq!(outcome, RatchetOutcome::Clean);
    assert_eq!(exit_code(&outcome), 0);
}

#[test]
fn a_finding_the_baseline_does_not_carry_fails_the_gate() {
    let scratch = scratch("new");
    let repo = repo(&scratch);
    touch(&scratch, "src/a.ts");
    touch(&scratch, "src/b.ts");
    let known = fingerprinted(result("unused-export", "src/a.ts", Some(12), "m"), "00ff");
    let fresh = fingerprinted(result("unused-export", "src/b.ts", Some(3), "m"), "beef");
    let baseline = Baseline::new(vec![entry("judged/v1:00ff", "src/a.ts")]);

    let outcome = ratchet(
        &baseline,
        &healthy_run(vec![known, fresh]),
        &repo,
        EXPECTED_TARGETS,
        NOW,
    );

    match &outcome {
        RatchetOutcome::NewFindings(findings) => {
            assert_eq!(findings.len(), 1, "only the unbaselined finding is new");
            assert_eq!(findings[0].locations[0].uri, "src/b.ts");
            assert_eq!(
                findings[0].baseline_state,
                Some(BaselineState::New),
                "the returned result must be annotated for SARIF re-emission"
            );
        }
        other => panic!("expected new findings, got {other:?}"),
    }
    assert_eq!(exit_code(&outcome), 1);
}

#[test]
fn baseline_state_maps_onto_sarif() {
    let known = fingerprinted(result("unused-export", "src/a.ts", Some(12), "m"), "00ff");
    let fresh = fingerprinted(result("unused-export", "src/b.ts", Some(3), "m"), "beef");
    let baseline = Baseline::new(vec![entry("judged/v1:00ff", "src/a.ts")]);

    assert_eq!(baseline_state(&baseline, &known), BaselineState::Unchanged);
    assert_eq!(baseline_state(&baseline, &fresh), BaselineState::New);
}

#[test]
fn reformatting_a_file_does_not_manufacture_new_findings() {
    // §9.2: "fingerprints must be content-derived, never line-based, or every
    // reformat resets the stability clock". A formatter moves every line in the
    // file and moves the line numbers the tool bakes into its own prose; none
    // of that is a new finding.
    let scratch = scratch("reformat");
    let repo = repo(&scratch);
    touch(&scratch, "src/a.ts");
    let before = result(
        "unused-export",
        "src/a.ts",
        Some(12),
        "export `foo` is never imported (line 12)",
    );
    let after = result(
        "unused-export",
        "src/a.ts",
        Some(480),
        "export `foo` is never imported (line 480)",
    );
    let baseline = Baseline::from_results(&[before], NOW);

    let outcome = ratchet(
        &baseline,
        &healthy_run(vec![after]),
        &repo,
        EXPECTED_TARGETS,
        NOW,
    );

    assert_eq!(
        outcome,
        RatchetOutcome::Clean,
        "a reformat must not look like new dead code"
    );
}

// ---------------------------------------------------------------------------
// Rot through the gate, and the Ruff exit-code contract (§9.2)
// ---------------------------------------------------------------------------

#[test]
fn rot_fails_the_gate_as_hard_as_a_new_finding() {
    // §5.3: "a suppression list without rot detection is the off switch".
    let scratch = scratch("gate-rot");
    let repo = repo(&scratch);
    let baseline = Baseline::new(vec![entry("judged/v1:00ff", "src/deleted.ts")]);

    let outcome = ratchet(
        &baseline,
        &healthy_run(Vec::new()),
        &repo,
        EXPECTED_TARGETS,
        NOW,
    );

    assert_eq!(
        outcome,
        RatchetOutcome::Rot(vec![RotReason::ReferentGone {
            uri: "src/deleted.ts".to_string()
        }])
    );
    assert_eq!(exit_code(&outcome), 1);
}

#[test]
fn a_rotten_baseline_is_reported_before_new_findings() {
    // Both fail the build, so the order is about what the developer is told to
    // do first. Pruning the baseline is mechanical; fixing code is not, and a
    // new-findings list computed against a baseline that is known to be stale
    // is a list you have to recompute anyway.
    let scratch = scratch("rot-first");
    let repo = repo(&scratch);
    touch(&scratch, "src/b.ts");
    let fresh = fingerprinted(result("unused-export", "src/b.ts", Some(3), "m"), "beef");
    let baseline = Baseline::new(vec![entry("judged/v1:00ff", "src/deleted.ts")]);

    let outcome = ratchet(
        &baseline,
        &healthy_run(vec![fresh]),
        &repo,
        EXPECTED_TARGETS,
        NOW,
    );

    assert!(matches!(outcome, RatchetOutcome::Rot(_)), "got {outcome:?}");
}

#[test]
fn exit_codes_follow_the_ruff_contract() {
    assert_eq!(exit_code(&RatchetOutcome::Clean), 0);
    assert_eq!(exit_code(&RatchetOutcome::NewFindings(Vec::new())), 1);
    assert_eq!(exit_code(&RatchetOutcome::Rot(Vec::new())), 1);
    assert_eq!(
        exit_code(&RatchetOutcome::Refused {
            reason: "crashed".to_string()
        }),
        2
    );
    // Degradation reports; the wrapped verdict decides.
    assert_eq!(
        exit_code(&RatchetOutcome::Degraded {
            reasons: vec!["half the repo".to_string()],
            verdict: Box::new(RatchetOutcome::NewFindings(Vec::new())),
        }),
        1
    );
}
