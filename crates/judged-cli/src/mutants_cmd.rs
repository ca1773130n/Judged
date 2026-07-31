//! `judged mutants` — §10 E2, the suite that decides whether an auto-act tier
//! may exist at all.
//!
//! Nineteen known-live artifacts, each reachable through exactly **one**
//! mechanism, injected into nineteen throwaway repositories. The methodology is
//! muSE / Bonett et al. (ACM TOSEM 3439802), and §10 E2 states the grading rule
//! without softening it: *any "dead" verdict on an injected artifact is a hard
//! failure* — not a tuning opportunity.
//!
//! # What the exit code means, and what it deliberately does not
//!
//! Exit 0 requires `false_removal_count == 0` and nothing else. §11 R1
//! pre-commits that if that number is not zero, the auto-act tier is **deleted
//! from the design rather than tuned**, so it is the only number the gate may
//! depend on.
//!
//! That leaves a hole, and the report is built around admitting it: a system
//! under test that claims nothing is ever dead scores a perfect zero and passes
//! the gate. It is also useless. So decoy recall — how many genuinely-dead
//! files the SUT actually found — is printed on the line below the gate, every
//! time, in both renderings. §3.7 and §9.8 require a positive control on every
//! evidence artifact; this is the suite's own, printed rather than enforced,
//! because turning it into an exit code would let a fixture author raise a
//! green by planting easier decoys.

use judged_mutants::fixtures;
use judged_mutants::mutant::Ecosystem;
use judged_mutants::runner::{run_suite, MutantReport, SuiteReport};
use judged_mutants::sut::{NaiveSut, RefusingSut, Sut};
use serde_json::{json, Value};

use crate::args::{MutantsArgs, SutChoice};

/// Run the catalogue and render the result.
pub fn run(args: &MutantsArgs) -> (String, i32) {
    let mutants = fixtures::all();
    // Captured before the run, because grading returns ids and ecosystems but
    // not the mechanism — and the mechanism is the whole point of a failure.
    // "m08 failed" is a bug report; "m08 failed: referenced only from a CI
    // workflow step" is a design finding.
    let catalogue: Vec<(String, String, String)> = mutants
        .iter()
        .map(|m| {
            (
                m.id().to_string(),
                m.mechanism().to_string(),
                m.research_ref().to_string(),
            )
        })
        .collect();

    let sut: Box<dyn Sut> = match args.sut {
        SutChoice::Naive => Box::new(NaiveSut),
        SutChoice::Refusing => Box::new(RefusingSut),
    };

    let report = match run_suite(sut.as_ref(), &mutants) {
        Ok(report) => report,
        // A crashed harness is not a passing harness. §3.7: every catastrophic
        // failure in this space presented as an artifact reporting ~0% for
        // everything, which was then trusted.
        Err(error) => {
            return (
                format!(
                    "REFUSED — the E2 suite did not complete (exit 2)\n\n  {error}\n\n\
                     No verdict was reached. A suite that did not run is not a suite that \
                     found nothing (§3.7, §6.20).\n"
                ),
                2,
            )
        }
    };

    // The gate, and only the gate (§10 E2, §11 R1).
    let code = if report.false_removal_count == 0 {
        0
    } else {
        1
    };

    let rendered = if args.json {
        render_json(&report, &catalogue, args.sut)
    } else {
        render_text(&report, &catalogue, args.sut)
    };
    (rendered, code)
}

/// The catalogue's own spelling for an ecosystem.
fn ecosystem(ecosystem: Ecosystem) -> &'static str {
    match ecosystem {
        Ecosystem::Python => "python",
        Ecosystem::TypeScript => "typescript",
        Ecosystem::Rust => "rust",
        Ecosystem::Go => "go",
        Ecosystem::Polyglot => "polyglot",
    }
}

/// Mechanism and research reference for a mutant id.
fn lookup<'a>(catalogue: &'a [(String, String, String)], id: &str) -> (&'a str, &'a str) {
    catalogue
        .iter()
        .find(|(candidate, _, _)| candidate == id)
        .map(|(_, mechanism, research)| (mechanism.as_str(), research.as_str()))
        .unwrap_or(("(mechanism not declared)", "(unreferenced)"))
}

/// Classes that removed something live, in catalogue order.
///
/// Ordered by mutant id, never by how much each one removed — §9.13 invariant 3
/// applies to this report exactly as it applies to the ratchet's.
fn failing_classes(report: &SuiteReport) -> Vec<&str> {
    report
        .reports
        .iter()
        .filter(|r| !r.false_removals.is_empty())
        .map(|r| r.mutant_id.as_str())
        .collect()
}

fn totals(report: &SuiteReport) -> (usize, usize, usize) {
    let passed = report.reports.iter().filter(|r| r.passed).count();
    let decoys_found = report.reports.iter().map(|r| r.decoys_found).sum();
    let decoys_total = report.reports.iter().map(|r| r.decoys_total).sum();
    (passed, decoys_found, decoys_total)
}

fn render_text(
    report: &SuiteReport,
    catalogue: &[(String, String, String)],
    choice: SutChoice,
) -> String {
    let classes = report.reports.len();
    let (passed, decoys_found, decoys_total) = totals(report);

    let mut out = format!(
        "judged mutants — §10 E2, {classes} injected liveness mechanisms, SUT `{}`\n\n\
         \x20 Any \"dead\" verdict on an injected live artifact is a hard failure, not a tuning\n\
         \x20 opportunity (§10 E2). Decoys are genuinely-dead files planted beside them, so that\n\
         \x20 a tool which refuses to answer cannot score a perfect run.\n\n",
        choice.as_str()
    );

    for row in &report.reports {
        let (mechanism, research) = lookup(catalogue, &row.mutant_id);
        out.push_str(&mutant_line(row, mechanism));
        for removed in &row.false_removals {
            // Indented under its class and spelled out, because this line is
            // the finding: a live artifact the tool would have deleted, and the
            // documented incident class it came from.
            out.push_str(&format!("       removed live: {removed}   [{research}]\n"));
        }
    }

    // Summary lines are unindented: they are what a CI log tail shows, and what
    // a human reads in the ten seconds §9.13 budgets.
    out.push_str(&format!(
        "\n{classes} classes: {passed} passed, {} failed\n",
        classes - passed
    ));
    out.push_str(&format!(
        "decoy recall: {decoys_found} of {decoys_total} genuinely-dead files found\n"
    ));
    out.push_str(&format!(
        "false removals: {} — {}\n",
        report.false_removal_count,
        if report.false_removal_count == 0 {
            "GATE PASSED (§10 E2 gates releases on this number, and on nothing else)"
        } else {
            "GATE FAILED (§11 R1: if this is not zero, the auto-act tier is deleted \
             from the design rather than tuned)"
        }
    ));

    let failing = failing_classes(report);
    if failing.is_empty() {
        if decoys_found == 0 {
            out.push_str(
                "note: this SUT removed nothing at all, so it cleared the gate without \
                 demonstrating it can find anything. Zero false removals is also the score of a \
                 tool that refuses to answer.\n",
            );
        }
    } else {
        out.push_str(&format!(
            "classes with false removals: {}\n",
            failing.join(", ")
        ));
    }
    out
}

/// One row of the table: id, verdict, ecosystem, the two counts, the mechanism.
fn mutant_line(row: &MutantReport, mechanism: &str) -> String {
    format!(
        "  {id}  {verdict:4}  {ecosystem:10}  {false_removals} false  {found}/{total} decoys  {mechanism}\n",
        id = row.mutant_id,
        verdict = if row.passed { "pass" } else { "FAIL" },
        ecosystem = ecosystem(row.ecosystem),
        false_removals = row.false_removals.len(),
        found = row.decoys_found,
        total = row.decoys_total,
    )
}

/// The same report, for something that is not a person.
///
/// Emitted as the whole of stdout so it can be piped straight into `jq`. Keys
/// are snake_case rather than the SARIF-style camelCase used on the wire
/// elsewhere, because this is Judged's own report about its own suite and not
/// an interchange format anyone else defines.
fn render_json(
    report: &SuiteReport,
    catalogue: &[(String, String, String)],
    choice: SutChoice,
) -> String {
    let (passed, decoys_found, decoys_total) = totals(report);

    let mutants: Vec<Value> = report
        .reports
        .iter()
        .map(|row| {
            let (mechanism, research) = lookup(catalogue, &row.mutant_id);
            json!({
                "id": row.mutant_id,
                "ecosystem": ecosystem(row.ecosystem),
                "mechanism": mechanism,
                "research_ref": research,
                "passed": row.passed,
                "false_removals": row.false_removals,
                "decoys_found": row.decoys_found,
                "decoys_total": row.decoys_total,
            })
        })
        .collect();

    let document = json!({
        "sut": choice.as_str(),
        "classes": report.reports.len(),
        "passed_classes": passed,
        "false_removal_count": report.false_removal_count,
        "gate_passed": report.false_removal_count == 0,
        "decoys_found": decoys_found,
        "decoys_total": decoys_total,
        "classes_with_false_removals": failing_classes(report),
        "mutants": mutants,
    });

    match serde_json::to_string_pretty(&document) {
        Ok(text) => format!("{text}\n"),
        // Unreachable for a document built from owned strings and integers, and
        // reported rather than unwrapped so that an impossible failure is still
        // a message instead of a panic (AGENTS.md rule 12).
        Err(error) => format!("{{\"error\":\"could not serialize the E2 report: {error}\"}}\n"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalogue() -> Vec<(String, String, String)> {
        vec![(
            "m01".to_string(),
            "a dotted path in a settings string".to_string(),
            "§10 E2 class 1".to_string(),
        )]
    }

    fn suite(false_removals: Vec<String>, decoys_found: usize, decoys_total: usize) -> SuiteReport {
        let count = false_removals.len();
        SuiteReport {
            sut_name: "test".to_string(),
            reports: vec![MutantReport {
                mutant_id: "m01".to_string(),
                ecosystem: Ecosystem::Python,
                passed: count == 0 && decoys_found == decoys_total,
                false_removals,
                decoys_found,
                decoys_total,
            }],
            false_removal_count: count,
        }
    }

    #[test]
    fn a_clean_gate_with_no_decoy_recall_says_so_rather_than_reading_as_success() {
        // The hole the gate deliberately leaves. A report that printed only
        // "GATE PASSED" here would be endorsing a tool that has never called
        // anything dead.
        let text = render_text(&suite(Vec::new(), 0, 3), &catalogue(), SutChoice::Refusing);

        assert!(text.contains("false removals: 0"), "got {text}");
        assert!(text.contains("GATE PASSED"), "got {text}");
        assert!(text.contains("decoy recall: 0 of 3"), "got {text}");
        assert!(text.contains("removed nothing at all"), "got {text}");
    }

    #[test]
    fn a_false_removal_names_the_class_the_mechanism_and_the_artifact() {
        let text = render_text(
            &suite(vec!["app/tasks/nightly.py".to_string()], 1, 1),
            &catalogue(),
            SutChoice::Naive,
        );

        assert!(text.contains("GATE FAILED"), "got {text}");
        assert!(
            text.contains("classes with false removals: m01"),
            "got {text}"
        );
        assert!(text.contains("app/tasks/nightly.py"), "got {text}");
        assert!(text.contains("§10 E2 class 1"), "got {text}");
        assert!(
            text.contains("a dotted path in a settings string"),
            "the mechanism is the finding; without it a failure is just an id"
        );
    }

    #[test]
    fn the_summary_lines_start_at_column_zero() {
        // What CI shows is the tail of the log, and what a human greps for is a
        // line start. Indenting these would bury the only two numbers that
        // decide anything.
        let text = render_text(
            &suite(vec!["x".to_string()], 0, 1),
            &catalogue(),
            SutChoice::Naive,
        );

        for expected in [
            "false removals: ",
            "decoy recall: ",
            "classes with false removals: ",
        ] {
            assert!(
                text.lines().any(|line| line.starts_with(expected)),
                "no line starts with `{expected}`; got {text}"
            );
        }
    }
}
