//! Harness mechanics for [`run_suite`], exercised with stub mutants and stub SUTs.
//!
//! These tests deliberately do *not* use the real §10 catalogue. The catalogue
//! answers "is this cleaner safe"; this file answers the prior question "does the
//! harness grade honestly", and a harness bug that inflates a pass would be
//! invisible if both were tested together.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use judged_core::{Error, Result};
use judged_mutants::mutant::{Ecosystem, GroundTruth, Mutant};
use judged_mutants::runner::run_suite;
use judged_mutants::sut::{Sut, SutVerdict};

/// Every directory `run_suite` handed to a [`RecordingMutant`], in order.
static MATERIALIZED_IN: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

/// A mutant with ground truth supplied by the test. `materialize` really writes
/// the declared files so that the directory it is given is a plausible repo.
struct StubMutant {
    id: &'static str,
    truth: GroundTruth,
}

impl StubMutant {
    fn boxed(id: &'static str, truth: GroundTruth) -> Box<dyn Mutant> {
        Box::new(StubMutant { id, truth })
    }
}

impl Mutant for StubMutant {
    fn id(&self) -> &str {
        self.id
    }
    fn ecosystem(&self) -> Ecosystem {
        Ecosystem::Polyglot
    }
    fn mechanism(&self) -> &str {
        "test stub"
    }
    fn research_ref(&self) -> &str {
        "test stub"
    }
    fn materialize(&self, dir: &Path) -> Result<GroundTruth> {
        for rel in self
            .truth
            .live_paths
            .iter()
            .chain(self.truth.decoy_dead_paths.iter())
        {
            let target = dir.join(rel);
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).map_err(|source| Error::Io {
                    path: parent.to_path_buf(),
                    source,
                })?;
            }
            fs::write(&target, b"stub\n").map_err(|source| Error::Io {
                path: target.clone(),
                source,
            })?;
        }
        Ok(self.truth.clone())
    }
}

/// A mutant that records the directory it was handed, so the test can check
/// isolation and cleanup from the outside.
struct RecordingMutant {
    id: &'static str,
}

impl Mutant for RecordingMutant {
    fn id(&self) -> &str {
        self.id
    }
    fn ecosystem(&self) -> Ecosystem {
        Ecosystem::Polyglot
    }
    fn mechanism(&self) -> &str {
        "test stub"
    }
    fn research_ref(&self) -> &str {
        "test stub"
    }
    fn materialize(&self, dir: &Path) -> Result<GroundTruth> {
        let mut seen = MATERIALIZED_IN.lock().unwrap_or_else(|e| e.into_inner());
        seen.push(dir.to_path_buf());
        let marker = dir.join(format!("{}.marker", self.id));
        fs::write(&marker, b"x").map_err(|source| Error::Io {
            path: marker.clone(),
            source,
        })?;
        Ok(GroundTruth::default())
    }
}

/// A mutant that cannot build its repository.
struct BrokenMutant;

impl Mutant for BrokenMutant {
    fn id(&self) -> &str {
        "broken"
    }
    fn ecosystem(&self) -> Ecosystem {
        Ecosystem::Polyglot
    }
    fn mechanism(&self) -> &str {
        "test stub"
    }
    fn research_ref(&self) -> &str {
        "test stub"
    }
    fn materialize(&self, _dir: &Path) -> Result<GroundTruth> {
        Err(Error::Fixture {
            mutant_id: "broken".into(),
            message: "cannot build".into(),
        })
    }
}

/// A SUT that returns the same claims whatever repository it is shown.
struct ScriptedSut {
    verdict: SutVerdict,
}

impl Sut for ScriptedSut {
    fn name(&self) -> &str {
        "scripted"
    }
    fn run(&self, _repo: &Path) -> Result<SutVerdict> {
        Ok(self.verdict.clone())
    }
}

/// A SUT that records every repository path it is shown, in its own state
/// rather than in a process-wide static, so a test using it cannot race
/// another test in this binary.
#[derive(Default)]
struct PathRecordingSut {
    seen: Mutex<Vec<PathBuf>>,
}

impl Sut for PathRecordingSut {
    fn name(&self) -> &str {
        "path-recording"
    }
    fn run(&self, repo: &Path) -> Result<SutVerdict> {
        self.seen
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(repo.to_path_buf());
        Ok(SutVerdict::default())
    }
}

/// A SUT that crashes.
struct CrashingSut;

impl Sut for CrashingSut {
    fn name(&self) -> &str {
        "crashing"
    }
    fn run(&self, _repo: &Path) -> Result<SutVerdict> {
        Err(Error::Sut {
            sut: "crashing".into(),
            message: "exited 139".into(),
        })
    }
}

/// Ground truth whose decoys are reachable by path only — no symbol route.
///
/// `decoy_dead_symbols` is index-aligned with `decoy_dead_paths` and `""` means
/// "this decoy defines nothing a symbol-level tool could name", which is the
/// right declaration for a stub file whose whole content is `stub\n`.
fn truth(live: &[&str], decoys: &[&str]) -> GroundTruth {
    GroundTruth {
        live_paths: live.iter().map(PathBuf::from).collect(),
        live_symbols: Vec::new(),
        decoy_dead_paths: decoys.iter().map(PathBuf::from).collect(),
        decoy_dead_symbols: decoys.iter().map(|_| String::new()).collect(),
    }
}

fn claims(paths: &[&str], symbols: &[&str]) -> SutVerdict {
    SutVerdict {
        claimed_dead_paths: paths.iter().map(PathBuf::from).collect(),
        claimed_dead_symbols: symbols.iter().map(|s| (*s).to_string()).collect(),
    }
}

#[test]
fn false_removals_are_the_intersection_of_claims_and_live_ground_truth() {
    let mutants = vec![StubMutant::boxed(
        "s01",
        GroundTruth {
            live_paths: vec![PathBuf::from("app/live.py")],
            live_symbols: vec!["LiveTask".to_string()],
            decoy_dead_paths: vec![PathBuf::from("app/dead.py")],
            decoy_dead_symbols: vec![String::new()],
        },
    )];
    // Claims one live path, one live symbol, one genuine decoy, and one path
    // that is in neither set — only the first two are false removals.
    let sut = ScriptedSut {
        verdict: claims(
            &["app/live.py", "app/dead.py", "app/unknown.py"],
            &["LiveTask", "SomethingElse"],
        ),
    };

    let report = run_suite(&sut, &mutants).expect("suite runs");

    assert_eq!(report.sut_name, "scripted");
    assert_eq!(report.reports.len(), 1);
    assert_eq!(
        report.reports[0].false_removals,
        vec!["LiveTask".to_string(), "app/live.py".to_string()],
        "only claims that intersect live ground truth count, and they are sorted"
    );
    assert_eq!(report.false_removal_count, 2);
}

#[test]
fn a_qualified_spelling_is_the_same_symbol_and_a_longer_name_is_not() {
    // Ground truth spells symbols bare (`DunningConfig`); a real tool spells
    // them the way its ecosystem does. Under exact equality a SUT could delete
    // a live symbol and be graded clean purely for having qualified the name —
    // the gate silently under-reporting the one number §11 R1 keys on, which is
    // the direction a safety gate must never be wrong in.
    //
    // The second half is what keeps that widening honest. A bare `ends_with`
    // would score `MyDunningConfig` as a hit on `DunningConfig`, and a gate that
    // manufactures false removals is one nobody keeps running. The trailing
    // segment must be a whole segment: preceded by a separator, or nothing.
    let mutants = vec![StubMutant::boxed(
        "s01",
        GroundTruth {
            live_paths: Vec::new(),
            live_symbols: vec![
                "DunningConfig".to_string(),
                "render_badge".to_string(),
                "drain".to_string(),
                "Widget".to_string(),
                // Nothing below is claimed. Each is a suffix of one of the
                // claims above without being a segment of it, so a bare
                // `ends_with` reports it as removed and a segment match does
                // not. They have to be symbols nothing else already flags:
                // a false positive on a symbol some other claim legitimately
                // matches is invisible in a set of live symbol names.
                "Config".to_string(),
                "badge".to_string(),
            ],
            decoy_dead_paths: Vec::new(),
            decoy_dead_symbols: Vec::new(),
        },
    )];
    let sut = ScriptedSut {
        verdict: claims(
            &[],
            &[
                // Qualified spellings of a live symbol: same symbol, caught.
                "ledger.dunning.DunningConfig", // python
                "badge::render_badge",          // rust
                "pkg/sampler.drain",            // go
                "src/ui/Widget.tsx#Widget",     // typescript, uri-fragment style
                // Merely ends with the same letters: a different symbol, and
                // claiming it is not a false removal.
                "MyDunningConfig",
                "xrender_badge",
                "predrain",
            ],
        ),
    };

    let report = run_suite(&sut, &mutants).expect("suite runs");

    assert_eq!(
        report.reports[0].false_removals,
        vec![
            "DunningConfig".to_string(),
            "Widget".to_string(),
            "drain".to_string(),
            "render_badge".to_string(),
        ],
        "a qualified spelling names the same symbol and must be caught; a name \
         that merely ends with the same letters is a different symbol and must \
         not be. `Config` and `badge` are live, unclaimed, and are what a bare \
         `ends_with` would invent"
    );
    assert_eq!(report.false_removal_count, 4);
}

#[test]
fn any_false_removal_fails_that_mutant() {
    // §10 E2: "Any 'dead' verdict is a hard failure." One is enough, even when
    // the SUT also found every decoy.
    let mutants = vec![StubMutant::boxed(
        "s01",
        truth(&["app/live.py"], &["app/dead.py"]),
    )];
    let sut = ScriptedSut {
        verdict: claims(&["app/live.py", "app/dead.py"], &[]),
    };

    let report = run_suite(&sut, &mutants).expect("suite runs");

    assert!(
        !report.reports[0].passed,
        "one false removal must fail the mutant regardless of decoy recall"
    );
    assert_eq!(report.reports[0].decoys_found, 1);
    assert_eq!(report.reports[0].decoys_total, 1);
}

#[test]
fn decoy_recall_is_recorded_and_required_for_a_pass() {
    // A tool that never claims anything has zero false removals and is useless.
    // The pass condition therefore has two halves.
    let mutants = vec![StubMutant::boxed(
        "s01",
        truth(&["app/live.py"], &["app/dead_a.py", "app/dead_b.py"]),
    )];

    let silent = ScriptedSut {
        verdict: SutVerdict::default(),
    };
    let silent_report = run_suite(&silent, &mutants).expect("suite runs");
    assert_eq!(silent_report.false_removal_count, 0);
    assert_eq!(silent_report.reports[0].decoys_found, 0);
    assert_eq!(silent_report.reports[0].decoys_total, 2);
    assert!(
        !silent_report.reports[0].passed,
        "zero false removals is not a pass when no decoy was found"
    );

    let working = ScriptedSut {
        verdict: claims(&["app/dead_b.py", "app/dead_a.py"], &[]),
    };
    let working_report = run_suite(&working, &mutants).expect("suite runs");
    assert_eq!(working_report.reports[0].decoys_found, 2);
    assert!(working_report.reports[0].passed);
}

/// Ground truth with a symbol route on every decoy, paired by index.
fn truth_with_decoy_symbols(decoys: &[(&str, &str)]) -> GroundTruth {
    GroundTruth {
        live_paths: Vec::new(),
        live_symbols: Vec::new(),
        decoy_dead_paths: decoys.iter().map(|(path, _)| PathBuf::from(path)).collect(),
        decoy_dead_symbols: decoys
            .iter()
            .map(|(_, symbol)| (*symbol).to_string())
            .collect(),
    }
}

#[test]
fn a_decoy_is_found_when_the_sut_names_a_symbol_defined_in_it() {
    // The defect this exists to fix. Decoy recall was path-only, and a
    // symbol-level analyzer never claims a path — so vulture scored 0 of 31
    // decoys, which reads on the scoreboard as "found nothing" when the truth
    // is "was never asked a question it could answer". That is §6.20's category
    // error ("no data" is not "zero executions") committed by the suite's own
    // positive control.
    let mutants = vec![StubMutant::boxed(
        "s01",
        truth_with_decoy_symbols(&[("app/dead.py", "hang_indent")]),
    )];
    let sut = ScriptedSut {
        verdict: claims(&[], &["hang_indent"]),
    };

    let report = run_suite(&sut, &mutants).expect("suite runs");

    assert_eq!(
        report.reports[0].decoys_found, 1,
        "naming the only symbol a dead file defines is finding that dead file"
    );
    assert_eq!(report.reports[0].decoys_total, 1);
    assert!(
        report.reports[0].passed,
        "zero false removals plus full decoy recall is a pass, whichever route \
         the recall came through"
    );
}

#[test]
fn a_qualified_spelling_of_a_decoy_symbol_still_finds_it() {
    // Same reasoning as the live-symbol side, and deliberately the same
    // matcher: ground truth spells symbols bare, real tools spell them the way
    // their ecosystem does. A second implementation here would be a second
    // thing to keep in step.
    let mutants = vec![StubMutant::boxed(
        "s01",
        truth_with_decoy_symbols(&[
            ("app/dead.py", "hang_indent"),
            ("src/dead.rs", "cache_key"),
            ("pkg/dead.go", "legacyHistogram"),
            ("src/Dead.tsx", "FLAGS"),
        ]),
    )];
    let sut = ScriptedSut {
        verdict: claims(
            &[],
            &[
                "pluginhost.textwrap_helper.hang_indent",
                "judged::globs::cache_key",
                "internal/sampler.legacyHistogram",
                "src/flags.ts#FLAGS",
            ],
        ),
    };

    let report = run_suite(&sut, &mutants).expect("suite runs");

    assert_eq!(
        report.reports[0].decoys_found, 4,
        "a qualified spelling names the same symbol here exactly as it does for \
         a live symbol; under-counting recall would make a working tool look \
         like one that refuses to answer"
    );
}

#[test]
fn a_decoy_found_by_both_routes_is_counted_once() {
    // The denominator is decoy FILES, so the two routes are two ways to find
    // the same thing and not two things to find. A tool that reports an unused
    // file *and* its unused export — knip does exactly this — would otherwise
    // score 4 of 2.
    let mutants = vec![StubMutant::boxed(
        "s01",
        truth_with_decoy_symbols(&[("app/dead.py", "hang_indent"), ("app/gone.py", "to_hex")]),
    )];
    let sut = ScriptedSut {
        verdict: claims(&["app/dead.py", "app/gone.py"], &["hang_indent", "to_hex"]),
    };

    let report = run_suite(&sut, &mutants).expect("suite runs");

    assert_eq!(report.reports[0].decoys_total, 2);
    assert_eq!(
        report.reports[0].decoys_found, 2,
        "recall counts decoy files found, not claims that hit; a rate above 100% \
         is a broken instrument"
    );
}

#[test]
fn a_decoy_with_no_symbol_route_is_reachable_by_its_path_alone() {
    // Four of the catalogue's decoys define nothing nameable — a bash script,
    // an nginx config, a PHP file that only echoes, a minified bundle. They
    // declare `""`, and `""` must never match a claim: `names_same_symbol` on an
    // empty needle would score a claim of `""`, or of anything ending in a
    // separator, as having found them. A decoy credited for a claim about
    // nothing is the same defect as a decoy nobody can find, pointed the other
    // way.
    let mutants = vec![StubMutant::boxed(
        "s01",
        truth_with_decoy_symbols(&[("scripts/old_benchmark.sh", "")]),
    )];

    let guessing = ScriptedSut {
        verdict: claims(&[], &["", "anything.", "old_benchmark"]),
    };
    let guessing_report = run_suite(&guessing, &mutants).expect("suite runs");
    assert_eq!(
        guessing_report.reports[0].decoys_found, 0,
        "a decoy with no symbol route must not be credited to any symbol claim"
    );

    let working = ScriptedSut {
        verdict: claims(&["scripts/old_benchmark.sh"], &[]),
    };
    let working_report = run_suite(&working, &mutants).expect("suite runs");
    assert_eq!(
        working_report.reports[0].decoys_found, 1,
        "the path route is the only one such a decoy has, and it must still work"
    );
}

#[test]
fn the_recall_denominator_is_decoy_files_not_paths_plus_symbols() {
    // Documented choice, asserted so it cannot drift: recall is out of decoy
    // FILES, with either route counting as finding one. A decoy is a file; a
    // tool that names its only symbol has found that file, and counting the
    // path and the symbol as two separate things to find would halve the score
    // of every tool that can only take one of the two routes — reintroducing,
    // in the denominator, exactly the defect the numerator just fixed.
    let mutants = vec![StubMutant::boxed(
        "s01",
        truth_with_decoy_symbols(&[("app/dead.py", "hang_indent"), ("app/gone.py", "to_hex")]),
    )];
    let path_only = ScriptedSut {
        verdict: claims(&["app/dead.py", "app/gone.py"], &[]),
    };
    let symbol_only = ScriptedSut {
        verdict: claims(&[], &["hang_indent", "to_hex"]),
    };

    for sut in [path_only, symbol_only] {
        let report = run_suite(&sut, &mutants).expect("suite runs");
        assert_eq!(
            (
                report.reports[0].decoys_found,
                report.reports[0].decoys_total
            ),
            (2, 2),
            "a file-level tool and a symbol-level tool that each find both decoy \
             files must score the same, and that score must be 2 of 2"
        );
    }
}

#[test]
fn a_failing_mutant_does_not_stop_later_mutants_from_being_graded() {
    let mutants = vec![
        StubMutant::boxed("s01", truth(&["app/live.py"], &[])),
        StubMutant::boxed("s02", truth(&["other/live.py"], &["other/dead.py"])),
        StubMutant::boxed("s03", truth(&["third/live.py"], &[])),
    ];
    let sut = ScriptedSut {
        verdict: claims(&["app/live.py", "third/live.py", "other/dead.py"], &[]),
    };

    let report = run_suite(&sut, &mutants).expect("suite runs");

    let ids: Vec<&str> = report
        .reports
        .iter()
        .map(|r| r.mutant_id.as_str())
        .collect();
    assert_eq!(
        ids,
        vec!["s01", "s02", "s03"],
        "every mutant is graded and reports stay in catalogue order"
    );
    assert!(!report.reports[0].passed);
    assert!(report.reports[1].passed);
    assert!(!report.reports[2].passed);
    assert_eq!(report.false_removal_count, 2);
}

#[test]
fn report_is_deterministic_across_runs() {
    let mutants = vec![
        StubMutant::boxed(
            "s01",
            GroundTruth {
                live_paths: vec![PathBuf::from("z.py"), PathBuf::from("a.py")],
                live_symbols: vec!["Zeta".into(), "Alpha".into()],
                decoy_dead_paths: vec![PathBuf::from("d2.py"), PathBuf::from("d1.py")],
                decoy_dead_symbols: vec!["Dead2".to_string(), "Dead1".to_string()],
            },
        ),
        StubMutant::boxed("s02", truth(&["b.py"], &["d3.py"])),
    ];
    let sut = ScriptedSut {
        verdict: claims(&["z.py", "b.py", "a.py", "d1.py"], &["Zeta", "Alpha"]),
    };

    let first = run_suite(&sut, &mutants).expect("suite runs");
    let second = run_suite(&sut, &mutants).expect("suite runs");
    assert_eq!(
        first, second,
        "two runs of the same input must be identical"
    );

    let mut sorted = first.reports[0].false_removals.clone();
    sorted.sort();
    assert_eq!(
        first.reports[0].false_removals, sorted,
        "false removals are emitted sorted, not in claim or ground-truth order"
    );
}

#[test]
fn duplicate_claims_are_counted_once() {
    let mutants = vec![StubMutant::boxed("s01", truth(&["app/live.py"], &[]))];
    let sut = ScriptedSut {
        verdict: claims(&["app/live.py", "app/live.py"], &[]),
    };

    let report = run_suite(&sut, &mutants).expect("suite runs");

    assert_eq!(report.reports[0].false_removals, vec!["app/live.py"]);
    assert_eq!(
        report.false_removal_count, 1,
        "the release-gate number must count distinct artifacts, not claim events"
    );
}

#[test]
fn absolute_ground_truth_paths_are_normalized_to_repo_relative() {
    // A fixture may naturally return `dir.join(...)`. `SutVerdict` is documented
    // as repo-relative. If the harness compared them raw, the intersection would
    // be empty and every mutant would pass — a silent, total loss of the gate.
    struct AbsoluteTruthMutant;
    impl Mutant for AbsoluteTruthMutant {
        fn id(&self) -> &str {
            "abs"
        }
        fn ecosystem(&self) -> Ecosystem {
            Ecosystem::Polyglot
        }
        fn mechanism(&self) -> &str {
            "test stub"
        }
        fn research_ref(&self) -> &str {
            "test stub"
        }
        fn materialize(&self, dir: &Path) -> Result<GroundTruth> {
            Ok(GroundTruth {
                live_paths: vec![dir.join("app/live.py")],
                live_symbols: Vec::new(),
                decoy_dead_paths: vec![dir.join("app/dead.py")],
                decoy_dead_symbols: vec![String::new()],
            })
        }
    }

    let mutants: Vec<Box<dyn Mutant>> = vec![Box::new(AbsoluteTruthMutant)];
    let sut = ScriptedSut {
        verdict: claims(&["app/live.py", "app/dead.py"], &[]),
    };

    let report = run_suite(&sut, &mutants).expect("suite runs");

    assert_eq!(report.reports[0].false_removals, vec!["app/live.py"]);
    assert_eq!(report.reports[0].decoys_found, 1);
}

#[test]
fn a_mutant_that_cannot_materialize_is_an_error_naming_the_mutant() {
    // Scaffold contract: a mutant that fails to materialize is an error, never a
    // skip — a silently skipped mutant becomes a pass the SUT did not earn.
    let mutants: Vec<Box<dyn Mutant>> = vec![Box::new(BrokenMutant)];
    let sut = ScriptedSut {
        verdict: SutVerdict::default(),
    };

    match run_suite(&sut, &mutants) {
        Err(Error::Fixture { mutant_id, message }) => {
            assert_eq!(mutant_id, "broken");
            assert!(
                message.contains("cannot build"),
                "the underlying reason must survive, got {message:?}"
            );
        }
        other => panic!("expected Error::Fixture, got {other:?}"),
    }
}

#[test]
fn a_sut_that_crashes_is_an_error_naming_the_sut_and_the_mutant() {
    // Recording a crash as "claimed nothing" would score a perfect zero false
    // removals — the §3.7 signature of every catastrophic failure in this space.
    let mutants = vec![StubMutant::boxed("s01", truth(&["app/live.py"], &[]))];

    match run_suite(&CrashingSut, &mutants) {
        Err(Error::Sut { sut, message }) => {
            assert_eq!(sut, "crashing");
            assert!(
                message.contains("s01") && message.contains("exited 139"),
                "must name the mutant and keep the cause, got {message:?}"
            );
        }
        other => panic!("expected Error::Sut, got {other:?}"),
    }
}

#[test]
fn each_mutant_gets_a_fresh_repo_and_it_is_removed_afterwards() {
    MATERIALIZED_IN
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clear();

    let mutants: Vec<Box<dyn Mutant>> = vec![
        Box::new(RecordingMutant { id: "r01" }),
        Box::new(RecordingMutant { id: "r02" }),
    ];
    let sut = ScriptedSut {
        verdict: SutVerdict::default(),
    };

    run_suite(&sut, &mutants).expect("suite runs");

    let dirs = MATERIALIZED_IN
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    assert_eq!(dirs.len(), 2);
    assert_ne!(dirs[0], dirs[1], "each mutant needs its own repository");
    for dir in &dirs {
        assert!(
            !dir.exists(),
            "temp repo {dir:?} outlived the suite; a run of the full catalogue \
             would leak nineteen trees"
        );
    }
}

#[test]
fn no_component_of_a_fixture_repo_path_is_hidden() {
    // Measured, 2026-08-01, and it silently cost the suite an entire ecosystem.
    //
    // `tempfile::TempDir::new()` names its directory with the default prefix
    // `.tmp`, so every fixture repo was handed to the analyzer as a HIDDEN
    // directory. The Go toolchain documents that it ignores path elements
    // beginning with `.` or `_` (`go help packages`), so the pattern
    // `<repo>/...` matched zero packages and `deadcode` printed
    //
    //     deadcode: no packages
    //
    // and exited 1 — indistinguishable, at the exit code, from "this
    // repository has no Go in it". m12 is the catalogue's only Go class and
    // the one §4.1 predicts deadcode false-removes on, so the harness was
    // structurally incapable of grading the prediction it was built to test.
    //
    // Reproduced by hand against the same fixture, deadcode/x-tools v0.48.0:
    // the identical tree under `.tmpABC123/` yields `deadcode: no packages`,
    // and under `tmpABC123/` yields the package JSON array and exit 0.
    //
    // This is a property of the harness, not of any one analyzer: a hidden
    // working directory changes what SOME tools will look at, and a tool that
    // looks at nothing reports nothing, which grades as zero false removals —
    // §6.20's "clean" that is really "never ran". So the assertion is about
    // every component of the path, for every SUT, not about Go.
    // Deliberately not `MATERIALIZED_IN`: that recorder is a process-wide
    // static, cargo runs the tests in this binary on parallel threads, and a
    // second test clearing it mid-run would make either test read the other's
    // directory. The SUT is handed the same path the mutant was, so recording
    // it here needs no shared state at all.
    let sut = PathRecordingSut::default();
    let mutants: Vec<Box<dyn Mutant>> = vec![StubMutant::boxed("h01", truth(&["live.py"], &[]))];

    run_suite(&sut, &mutants).expect("suite runs");

    let dirs = sut.seen.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let repo = dirs.first().expect("one repo was shown to the SUT");

    // The repo's own directory name is the one the harness chooses. Components
    // above it belong to the machine's TMPDIR and are not ours to police, so
    // the assertion is scoped to the leaf — which is exactly what was wrong.
    let leaf = repo
        .file_name()
        .expect("a temp repo has a final component")
        .to_string_lossy()
        .into_owned();
    assert!(
        !leaf.starts_with('.') && !leaf.starts_with('_'),
        "fixture repo {repo:?} is a hidden directory: the Go tool, and any \
         other tool that skips dot-directories, will scan nothing inside it \
         and report nothing, which grades as a clean run it never earned"
    );
}
