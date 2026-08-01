//! What a SUT declares it can read, and what the suite does with a class it
//! cannot.
//!
//! §9.2's first non-SARIF clause is the capability envelope: *"every adapter
//! declares which finding classes it can and structurally cannot emit … This is
//! what lets the orchestrator know when silence means anything."* A
//! language-specific analyzer's largest structural blind spot is a whole
//! language, and until this build the suite had no way to be told about one — it
//! handed every fixture to whatever analyzer was selected, and a tool given a
//! repository in the wrong language either refused it (aborting the run) or
//! tolerated it and scored a class it never opened.
//!
//! Skipping is the fix, and it carries the one risk this file exists to close.
//! §6.20: *"'no data' must be a distinct state from 'zero executions,' and it
//! must never flow into a deadness score."* If a skipped class silently became a
//! passed one, narrowing an adapter's declared languages would be a way to raise
//! a green, and an adapter declaring it reads nothing would score a perfect run.
//! So a skip is a third state, counted in its own column and in no other.

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use judged_core::Result;
use judged_mutants::fixtures;
use judged_mutants::mutant::{Ecosystem, GroundTruth, Mutant};
use judged_mutants::runner::{run_suite, Grade, SuiteReport};
use judged_mutants::sut::{NaiveSut, RefusingSut, Sut, SutVerdict};

/// A mutant that records every time it is asked to build a repository.
///
/// Materialization is the observable half of "the class was never attempted".
/// A runner that built the tree and then declined to run the tool would still
/// be doing work whose only purpose is to be thrown away, and — worse — would
/// leave the door open to grading it later.
struct CountingMutant {
    id: &'static str,
    ecosystem: Ecosystem,
    languages: &'static [Ecosystem],
    materializations: Arc<AtomicUsize>,
}

impl CountingMutant {
    fn new(id: &'static str, ecosystem: Ecosystem, languages: &'static [Ecosystem]) -> Self {
        CountingMutant {
            id,
            ecosystem,
            languages,
            materializations: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// A handle on the counter that survives boxing into `dyn Mutant`.
    fn counter(&self) -> Arc<AtomicUsize> {
        Arc::clone(&self.materializations)
    }
}

impl Mutant for CountingMutant {
    fn id(&self) -> &str {
        self.id
    }
    fn ecosystem(&self) -> Ecosystem {
        self.ecosystem
    }
    fn languages(&self) -> &'static [Ecosystem] {
        self.languages
    }
    fn mechanism(&self) -> &str {
        "a counting stand-in"
    }
    fn research_ref(&self) -> &str {
        "(test fixture)"
    }
    fn materialize(&self, dir: &Path) -> Result<GroundTruth> {
        self.materializations.fetch_add(1, Ordering::SeqCst);
        let live = dir.join("live.txt");
        std::fs::write(&live, "live\n").expect("fixture writes");
        let decoy = dir.join("decoy.txt");
        std::fs::write(&decoy, "decoy\n").expect("fixture writes");
        Ok(GroundTruth {
            live_paths: vec![live],
            live_symbols: Vec::new(),
            decoy_dead_paths: vec![decoy],
            decoy_dead_symbols: vec![String::new()],
        })
    }
}

/// A SUT that reads exactly the ecosystems it is given, and records which
/// repositories it was actually handed.
struct DeclaringSut {
    name: &'static str,
    reads: Option<&'static [Ecosystem]>,
    /// Claimed dead in every repository it is shown, so that a class it was
    /// wrongly handed shows up as a false removal rather than as silence.
    claims: &'static [&'static str],
    invocations: Mutex<Vec<String>>,
}

impl DeclaringSut {
    fn new(name: &'static str, reads: Option<&'static [Ecosystem]>) -> Self {
        DeclaringSut {
            name,
            reads,
            claims: &[],
            invocations: Mutex::new(Vec::new()),
        }
    }

    fn claiming(mut self, claims: &'static [&'static str]) -> Self {
        self.claims = claims;
        self
    }

    fn invocation_count(&self) -> usize {
        self.invocations.lock().expect("not poisoned").len()
    }
}

impl Sut for DeclaringSut {
    fn name(&self) -> &str {
        self.name
    }

    fn reads(&self) -> Option<&[Ecosystem]> {
        self.reads
    }

    fn run(&self, repo: &Path) -> Result<SutVerdict> {
        self.invocations
            .lock()
            .expect("not poisoned")
            .push(repo.display().to_string());
        Ok(SutVerdict {
            claimed_dead_paths: self.claims.iter().map(Path::new).map(Into::into).collect(),
            claimed_dead_symbols: Vec::new(),
        })
    }
}

fn grades(report: &SuiteReport) -> Vec<(&str, Grade)> {
    report
        .reports
        .iter()
        .map(|row| (row.mutant_id.as_str(), row.grade))
        .collect()
}

#[test]
fn a_sut_that_reads_nothing_grades_nothing_and_cannot_present_a_clean_run() {
    // THE abuse case. A skipped class must never be able to improve a score, so
    // the limit of the feature — an adapter that declares it reads no language
    // at all — must come out of the suite with nothing graded rather than with
    // a perfect record.
    //
    // Run against the real catalogue, because the number that matters is the
    // one a release would be gated on.
    let mutants = fixtures::all();
    let blind = DeclaringSut::new("reads-nothing", Some(&[]));

    let report = run_suite(&blind, &mutants).expect("the suite still produces a report");

    assert_eq!(
        blind.invocation_count(),
        0,
        "an analyzer that declares it reads nothing was still spawned; whatever \
         it printed is about to be graded"
    );
    assert_eq!(
        report.graded_count(),
        0,
        "a SUT that reads nothing graded {} classes",
        report.graded_count()
    );
    assert_eq!(
        report.not_read_count(),
        mutants.len(),
        "every class must be accounted for as unread, not dropped: a class that \
         vanishes from the report cannot be told apart from one that was never \
         in the catalogue"
    );
    assert_eq!(report.reports.len(), mutants.len());
    for row in &report.reports {
        assert_eq!(
            row.grade,
            Grade::NotRead,
            "{} was graded by a SUT that reads nothing",
            row.mutant_id
        );
        assert!(
            !row.passed(),
            "{} is reported as passed by a SUT that never opened it. This is the \
             whole risk of skipping: if `not read` reads as `passed`, declaring a \
             narrower language set becomes a way to raise a green (§6.20).",
            row.mutant_id
        );
        assert_eq!(
            (row.decoys_found, row.decoys_total),
            (0, 0),
            "{} contributed to decoy recall without being attempted; a skipped \
             class must be in neither the numerator nor the denominator",
            row.mutant_id
        );
    }

    // And the number a release is gated on is still zero — which is exactly why
    // zero cannot be the whole gate. `graded_count` is what tells the caller
    // that this zero means "nothing was measured" rather than "nothing was
    // wrong"; §6.20's rule is that those must be distinct states.
    assert_eq!(report.false_removal_count, 0);
    assert_eq!(
        report.passed_count(),
        0,
        "a SUT that reads nothing must pass no class"
    );
    assert_eq!(report.failed_count(), 0);
}

#[test]
fn a_skipped_class_is_counted_in_its_own_column_and_in_no_other() {
    // The arithmetic, stated explicitly. Every class lands in exactly one of
    // three columns, and the two that gate anything are computed over the
    // graded ones alone.
    let mutants: Vec<Box<dyn Mutant>> = vec![
        Box::new(CountingMutant::new(
            "k01",
            Ecosystem::Python,
            &[Ecosystem::Python],
        )),
        Box::new(CountingMutant::new("k02", Ecosystem::Go, &[Ecosystem::Go])),
        Box::new(CountingMutant::new(
            "k03",
            Ecosystem::Rust,
            &[Ecosystem::Rust],
        )),
    ];

    // Reads Python only, and removes the live file wherever it is let in — so
    // the Python class fails and the other two are never attempted.
    let sut = DeclaringSut::new("python-only", Some(&[Ecosystem::Python])).claiming(&["live.txt"]);
    let report = run_suite(&sut, &mutants).expect("suite runs");

    assert_eq!(
        grades(&report),
        vec![
            ("k01", Grade::Failed),
            ("k02", Grade::NotRead),
            ("k03", Grade::NotRead),
        ]
    );
    assert_eq!(
        report.passed_count() + report.failed_count() + report.not_read_count(),
        report.reports.len(),
        "the three columns must partition the catalogue exactly; anything else \
         means a class is being counted twice or not at all"
    );
    assert_eq!(report.graded_count(), 1);
    assert_eq!(report.not_read_count(), 2);
    assert_eq!(
        report.false_removal_count, 1,
        "only the class that was actually analyzed may contribute to the gate"
    );
}

#[test]
fn a_class_outside_the_suts_languages_is_never_built_and_never_handed_over() {
    // Skipping has to happen before materialization, not after. A runner that
    // built the repository and then declined to grade it would be doing work
    // whose only product is a tree nobody looks at — and it would leave the
    // verdict one line of code away from being collected anyway.
    let python = CountingMutant::new("k01", Ecosystem::Python, &[Ecosystem::Python]);
    let go = CountingMutant::new("k02", Ecosystem::Go, &[Ecosystem::Go]);
    let (python_built, go_built) = (python.counter(), go.counter());
    let mutants: Vec<Box<dyn Mutant>> = vec![Box::new(python), Box::new(go)];

    let sut = DeclaringSut::new("python-only", Some(&[Ecosystem::Python]));
    run_suite(&sut, &mutants).expect("suite runs");

    assert_eq!(
        sut.invocation_count(),
        1,
        "the analyzer was spawned on a class it declared it cannot read"
    );
    assert_eq!(
        python_built.load(Ordering::SeqCst),
        1,
        "the class the SUT reads was not built"
    );
    assert_eq!(
        go_built.load(Ordering::SeqCst),
        0,
        "a class the SUT cannot read was materialized anyway. The tree is then \
         one line of code away from being handed over and graded, and building \
         it is work whose only product is a directory nobody reads."
    );
}

#[test]
fn a_language_agnostic_sut_still_sees_every_class() {
    // The controls must be untouched by this feature. Both declare no language
    // set at all — `NaiveSut` walks every source extension the catalogue uses
    // and `RefusingSut` declines uniformly rather than out of incapacity — so
    // nothing may be skipped for either, and the numbers §9.8 relies on to
    // prove the gate can still fail are unchanged.
    assert!(
        NaiveSut.reads().is_none(),
        "the positive control declared a language set; it would stop being a \
         faithful reproduction of §7.5's whole-repository heuristic"
    );
    assert!(RefusingSut.reads().is_none());

    let mutants = fixtures::all();
    for sut in [&NaiveSut as &dyn Sut, &RefusingSut as &dyn Sut] {
        let report = run_suite(sut, &mutants).expect("suite runs");
        assert_eq!(
            report.not_read_count(),
            0,
            "`{}` skipped a class despite declaring no language limit",
            sut.name()
        );
        assert_eq!(report.graded_count(), mutants.len());
        assert!(
            report.reports.iter().all(|row| row.grade != Grade::NotRead),
            "`{}` produced an unread row",
            sut.name()
        );
    }
}

#[test]
fn every_fixture_declares_the_languages_actually_in_its_repository() {
    // The map that decides what gets skipped, pinned as a full matrix rather
    // than as spot checks — because the failure mode is a *missing* or *extra*
    // entry and a spot check on the entries that exist can see neither.
    //
    // Both directions are damaging and they are damaging differently. Too wide
    // and a tool is handed a repository it cannot analyze, which is the abort
    // this feature exists to prevent. Too narrow and a class the tool really
    // does read is dropped from the measurement — and a dropped class is a
    // false removal that never gets counted, which is the direction that
    // quietly turns a red into a green.
    //
    // Measured 2026-08-01 against the materialized catalogue, not inferred from
    // the class names: `find <repo> -type f` on each of the nineteen fixtures.
    let expected: &[(&str, &[Ecosystem])] = &[
        ("m01", &[Ecosystem::Python]),
        // Python `app/` plus a TypeScript `src/` with package.json and
        // tsconfig.json: both toolchains load it.
        ("m02", &[Ecosystem::Python, Ecosystem::TypeScript]),
        ("m03", &[Ecosystem::Python]),
        ("m04", &[Ecosystem::Rust]),
        ("m05", &[Ecosystem::Python]),
        ("m06", &[Ecosystem::Rust]),
        ("m07", &[Ecosystem::Rust]),
        // Polyglot by mechanism — a CI workflow referencing a shell script —
        // but the only *language* toolchain that can load it is Python. There
        // is no package.json, and knip exits 2 on it (measured).
        ("m08", &[Ecosystem::Python]),
        ("m09", &[Ecosystem::Rust]),
        // Django/Python plus a JavaScript half with package.json.
        ("m10", &[Ecosystem::Python, Ecosystem::TypeScript]),
        ("m11", &[Ecosystem::Python]),
        ("m12", &[Ecosystem::Go]),
        // PHP and checked-in media. None of the four analyzers Judged adapts
        // reads PHP, so every one of them skips it — which is the honest
        // answer, not a gap: a tool that cannot parse the language cannot have
        // an opinion about it.
        ("m13", &[]),
        ("m14", &[Ecosystem::TypeScript]),
        ("m15", &[Ecosystem::Python]),
        ("m16", &[Ecosystem::Python]),
        ("m17", &[Ecosystem::Rust]),
        // Python plus a Kotlin/Gradle Android half. No JS, no package.json.
        ("m18", &[Ecosystem::Python]),
        ("m19", &[Ecosystem::Rust]),
    ];

    let mutants = fixtures::all();
    assert_eq!(mutants.len(), expected.len());
    for (mutant, (id, languages)) in mutants.iter().zip(expected) {
        assert_eq!(mutant.id(), *id, "catalogue order changed");
        assert_eq!(
            mutant.languages(),
            *languages,
            "{id} declares the wrong languages",
        );
        assert!(
            !mutant.languages().contains(&Ecosystem::Polyglot),
            "{id} declares `Polyglot` as one of its languages. Polyglot is a \
             property of a class's liveness mechanism, not a toolchain any \
             analyzer can be pointed at; a SUT matching on it would be claiming \
             to read a language that does not exist"
        );
        for (index, language) in mutant.languages().iter().enumerate() {
            assert!(
                !mutant.languages()[..index].contains(language),
                "{id} lists {language:?} twice"
            );
        }
    }
}

#[test]
fn a_single_language_fixture_does_not_have_to_repeat_itself() {
    // The default: a fixture in one ecosystem is loadable by that ecosystem's
    // toolchain and no other, so it says so once. Only a polyglot class has an
    // answer that cannot be derived, and `Polyglot` therefore defaults to the
    // empty set — the conservative reading, and the one the matrix above exists
    // to stop anybody relying on by accident.
    struct Bare;
    impl Mutant for Bare {
        fn id(&self) -> &str {
            "bare"
        }
        fn ecosystem(&self) -> Ecosystem {
            Ecosystem::Rust
        }
        fn mechanism(&self) -> &str {
            "none"
        }
        fn research_ref(&self) -> &str {
            "(test fixture)"
        }
        fn materialize(&self, _dir: &Path) -> Result<GroundTruth> {
            Ok(GroundTruth::default())
        }
    }
    assert_eq!(Bare.languages(), &[Ecosystem::Rust]);

    struct BarePolyglot;
    impl Mutant for BarePolyglot {
        fn id(&self) -> &str {
            "bare-polyglot"
        }
        fn ecosystem(&self) -> Ecosystem {
            Ecosystem::Polyglot
        }
        fn mechanism(&self) -> &str {
            "none"
        }
        fn research_ref(&self) -> &str {
            "(test fixture)"
        }
        fn materialize(&self, _dir: &Path) -> Result<GroundTruth> {
            Ok(GroundTruth::default())
        }
    }
    assert_eq!(
        BarePolyglot.languages(),
        &[],
        "a polyglot class that does not say what is in it must be read by no \
         language-specific analyzer, rather than by all of them"
    );
}
