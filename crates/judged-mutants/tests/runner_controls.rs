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
/// be harder than the naive heuristic. The seven that pass it do so honestly:
/// they inject liveness a *stem-matching whole-repo grep* can see, which §6.2
/// marks mandatory, so a tool implementing only that counter-signal is supposed
/// to survive them.
///
/// **Set this to the observed count, not below it.** Measured with all 19
/// classes implemented: the naive cleaner is caught by 12 (m01, m02, m03, m08,
/// m09, m10, m12, m13, m14, m16, m18, m19). An earlier revision left the floor
/// at 5 against an observed 11, which meant six classes could quietly go soft —
/// a soft class contributes no false removals, and to an assertion counting
/// only the total that is indistinguishable from the class not existing. The
/// suite's own anti-rot guard would have stayed green through a 55% loss of
/// discriminating power. A floor that trails the truth is not a floor.
const NAIVE_MUST_FAIL_AT_LEAST: usize = 12;

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
    // Per class, not just in total. A count alone cannot tell "m03 went soft
    // and m11 got harder" from "nothing changed" — the two are the same
    // integer. §3.7 makes exactly this point about positive controls: asserted
    // at the wrong granularity, the control passes while the thing it guards
    // is broken. These five are the classes whose only reference is
    // definitionally not source, so a tool reading only source must miss them.
    // If one of them stops catching the naive cleaner, that fixture no longer
    // injects what its class name claims.
    const ALWAYS_CATCH: [&str; 5] = ["m03", "m08", "m09", "m13", "m18"];
    for id in ALWAYS_CATCH {
        assert!(
            caught.contains(id),
            "{id} did not catch the naive cleaner. Its class is defined by a \
             reference that is not source, so a source-only tool cannot see it; \
             if this fixture passes, it has stopped injecting its own mechanism. \
             Caught: {caught:?}"
        );
    }
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
