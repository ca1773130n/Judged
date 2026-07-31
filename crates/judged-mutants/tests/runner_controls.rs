//! The two controls, run against the real §10 E2 catalogue.
//!
//! This file is the point of the crate. §10 E2 gates releases on zero false
//! removals; §9.8 adds the thing almost nobody does, which is to check that the
//! gate can still fail: *"if breaking the build does not break the gate, the
//! gate is not a gate."*
//!
//! So there are two directions to prove, and neither is sufficient alone:
//!
//! * [`RefusingSut`] must come out at zero false removals. If it does not, the
//!   harness invents failures and every future red is untrustworthy.
//! * [`NaiveSut`] must come out at *many* false removals. If it does not, the
//!   fixtures have gone soft and every future green is untrustworthy.

use std::collections::BTreeSet;

use judged_mutants::fixtures;
use judged_mutants::runner::{run_suite, SuiteReport};
use judged_mutants::sut::{NaiveSut, RefusingSut};

/// A naive cleaner must be caught by at least this many distinct classes.
///
/// The floor counts only the classes a basename grep over source files *cannot*
/// see however the fixture is built, because the reference is definitionally not
/// in a source file: m08 (Dockerfile / CI workflow / k8s manifest), m09 (README
/// block), m13 (gitignore negation), m18 (platform-side manifest), plus m03,
/// whose plugin file is named by no literal anywhere.
///
/// It is deliberately not set to "most of the catalogue". Fixtures are free to
/// be harder than the naive heuristic and several already are — m01 hides its
/// dotted string inside `settings.py`, which *is* Python, so it survives a tool
/// that parses every file in the repository. That is a better mutant, and a
/// floor that punished it would push fixture authors toward weaker ones. The
/// floor's job is only to fail loudly if the catalogue stops injecting
/// unparsed-reference liveness at all.
///
/// Measured while the catalogue was still landing: with 7 of the 19 classes
/// implemented, 4 of them (m02, m03, m08, m10) caught the naive cleaner for 7
/// false removals. Raise this floor to the observed count once all 19 exist —
/// a floor that trails the truth by six classes is weaker than it needs to be.
const NAIVE_MUST_FAIL_AT_LEAST: usize = 5;

fn failing_classes(report: &SuiteReport) -> BTreeSet<String> {
    report
        .reports
        .iter()
        .filter(|r| !r.false_removals.is_empty())
        .map(|r| r.mutant_id.clone())
        .collect()
}

#[test]
fn refusing_sut_produces_no_false_removals_anywhere() {
    let mutants = fixtures::all();
    let expected = mutants.len();

    let report = run_suite(&RefusingSut, &mutants).expect("suite runs");

    assert_eq!(
        report.reports.len(),
        expected,
        "every registered mutant must appear in the report; a skipped mutant \
         reads as a pass the SUT never earned"
    );
    assert_eq!(
        report.false_removal_count,
        0,
        "a cleaner that claims nothing cannot possibly remove something live; \
         a non-zero count here means the harness invents failures. Offenders: {:?}",
        failing_classes(&report)
    );
    for r in &report.reports {
        assert_eq!(
            r.decoys_found, 0,
            "mutant {} reported decoy recall for a cleaner that claimed nothing",
            r.mutant_id
        );
    }
}

#[test]
fn refusing_sut_still_fails_the_suite_because_it_finds_no_decoys() {
    // Zero false removals is the score of a tool that refuses to answer. If the
    // suite cannot tell that apart from a working tool, it is measuring nothing
    // — which is what the decoys in `GroundTruth` exist to prevent.
    let mutants = fixtures::all();
    let report = run_suite(&RefusingSut, &mutants).expect("suite runs");

    let with_decoys: Vec<&str> = report
        .reports
        .iter()
        .filter(|r| r.decoys_total > 0)
        .map(|r| r.mutant_id.as_str())
        .collect();
    assert!(
        !with_decoys.is_empty(),
        "no mutant planted a genuinely-dead decoy, so a tool that never speaks \
         scores a perfect suite. Fixtures must plant decoys."
    );
    assert!(
        report.reports.iter().any(|r| !r.passed),
        "the do-nothing control passed the whole catalogue; decoy recall is not \
         being required"
    );
}

#[test]
fn naive_sut_is_caught_by_the_catalogue() {
    // The positive control. §9.8, applied to the suite itself.
    let mutants = fixtures::all();
    let report = run_suite(&NaiveSut, &mutants).expect("suite runs");

    let caught = failing_classes(&report);
    assert!(
        report.false_removal_count > 0,
        "a cleaner that claims a file dead whenever its basename does not appear \
         in some other source file — the exact heuristic §7.5 documents in every \
         shipped tool — passed the entire catalogue with zero false removals. \
         The fixtures have gone soft: they are no longer injecting artifacts \
         reachable only through an unparsed reference. Every green result this \
         suite has ever produced is now unsupported."
    );
    assert!(
        caught.len() >= NAIVE_MUST_FAIL_AT_LEAST,
        "the naive cleaner was caught by only {} of {} classes ({:?}), below the \
         floor of {}. The classes that must always catch it are the ones whose \
         only reference is definitionally not source — m03 plugin dir scan, m08 \
         CI manifest, m09 README block, m13 gitignore negation, m18 platform \
         manifest — with m14 generated asset, m15 enqueued payload, m16 \
         serialized blob and m19 ABI export normally joining them. If those are \
         passing, the fixture no longer injects what its class name claims and \
         the suite has stopped being able to fail.",
        caught.len(),
        report.reports.len(),
        caught,
        NAIVE_MUST_FAIL_AT_LEAST,
    );
}

#[test]
fn the_full_catalogue_report_is_deterministic() {
    // Two runs over freshly materialized repositories must be byte-identical.
    // Temp-directory names differ between runs, so anything that leaked an
    // absolute path into the report — or any set iterated in hash order — shows
    // up here rather than as an unexplainable CI diff months later.
    let mutants = fixtures::all();

    let first = run_suite(&NaiveSut, &mutants).expect("suite runs");
    let second = run_suite(&NaiveSut, &mutants).expect("suite runs");

    assert_eq!(first, second);
    assert_eq!(
        first
            .reports
            .iter()
            .map(|r| &r.mutant_id)
            .collect::<Vec<_>>(),
        second
            .reports
            .iter()
            .map(|r| &r.mutant_id)
            .collect::<Vec<_>>(),
        "reports must stay in catalogue order"
    );
}
