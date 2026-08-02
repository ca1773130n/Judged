//! Generating each fixture's coverage artifact from its own declared truth.
//!
//! Settled in `docs/decisions/2026-08-02-e2-coverage-artifacts.md`, and the
//! reason it is generated rather than hand-written is the whole of that
//! document. Three classes currently survive with false removals, all three live
//! through a runtime mechanism; nineteen independent authoring decisions, each
//! made by somebody who can see the score, would produce exactly the artifacts
//! that rescue exactly those three. The rule has to be fixed before any artifact
//! exists, and this module is the rule.
//!
//! # What a fixture declares, and what it may not
//!
//! One question, per class, answered from the injected mechanism and from
//! nothing else:
//!
//! > Of this fixture's live artifacts, which does a test suite exercising its
//! > documented entry point actually enter?
//!
//! The answers differ per class *because the mechanisms differ*, and that
//! difference is the entire content of the measurement. m12's `//go:linkname`
//! alias is called through at runtime, so a test enters it. m05's recovery
//! handler is entered by no test that does not inject a fault. m08's script runs
//! in a pipeline and not in a test process. A catalogue on which coverage
//! rescued everything would be one written to flatter coverage, and the
//! pre-commitment says such a run must not be published.
//!
//! Three constraints, and [`Declaration::check`] enforces all three in code
//! rather than leaving them to discipline:
//!
//! 1. A covered path must be one of the fixture's own `live_paths`.
//! 2. A called symbol must be one of its own `live_symbols`, and the file said
//!    to declare it must be a live path.
//! 3. **A decoy is never covered.** A decoy is genuinely dead; an artifact
//!    showing one executed would be a false statement about the fixture, and
//!    decoy recall would stop meaning anything.
//!
//! # The suite entry record, and why every artifact has one
//!
//! A real tracefile contains the test suite itself. Measured: Coverage.py's
//! output for a three-function module carries a `test_ledger.py` record with
//! `FNDA:1,test_handles` beside the module's own
//! (`judged-core/tests/coverage_real_artifacts.rs`). So does this generator's,
//! and it is load-bearing rather than decorative.
//!
//! A positive control has to name a symbol that was **called** (§3.7), and a
//! class whose live symbols are all uncalled — m01's registry-loaded class, m11's
//! reflectively-read fields — has none to name. Without an entry record those
//! classes could carry no control, so their artifacts would be discarded and the
//! file-level evidence they do have would be lost for a reason that has nothing
//! to do with the mechanism under test. The entry symbol belongs to no fixture's
//! ground truth, so it can rescue nothing; all it does is give every artifact a
//! control that can fail.

use std::path::Path;

use judged_core::coverage::Control;
use judged_core::{Error, Result};

use crate::mutant::{Declaration, GroundTruth};

/// The file a generated artifact attributes the suite's own execution to, and
/// the function it records as having run.
///
/// Deliberately named so it cannot collide with anything a fixture declares: it
/// exists to be the control's anchor, and a symbol that also appeared in some
/// fixture's ground truth could rescue a claim on evidence the generator
/// manufactured.
const SUITE_ENTRY_FILE: &str = "tests/judged_e2_entry";
const SUITE_ENTRY_SYMBOL: &str = "judged_e2_suite_entry";

/// The root a generated tracefile pretends it was recorded under.
///
/// Absolute, and not this repository's path, because that is what a real
/// artifact looks like — it was written on a CI runner. It also means every
/// fixture exercises [`judged_core::coverage::Coverage::executed_file`]'s
/// suffix matching rather than an equality that would only ever work locally.
const RECORDED_ROOT: &str = "/ci/build";

/// Write `truth`'s declared execution into `repo` as an lcov tracefile and its
/// positive control.
///
/// A class that declares nothing plants nothing: no artifact, no control, and
/// the layer reports `no-artifact` for it. That is the honest rendering — an
/// empty tracefile and a missing one say different things, and only the missing
/// one is true here (§6.20).
pub fn plant(
    repo: &Path,
    mutant_id: &str,
    truth: &GroundTruth,
    declaration: &Declaration,
    artifact: &Path,
) -> Result<bool> {
    declaration.check(mutant_id, truth)?;
    if declaration.is_empty() {
        return Ok(false);
    }

    let tracefile = render(truth, declaration);
    let control = render_control(declaration);

    let artifact_path = repo.join(artifact);
    if let Some(parent) = artifact_path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| Error::Io {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    std::fs::write(&artifact_path, tracefile).map_err(|source| Error::Io {
        path: artifact_path.clone(),
        source,
    })?;
    let control_path = Control::path_for(&artifact_path);
    std::fs::write(&control_path, control).map_err(|source| Error::Io {
        path: control_path,
        source,
    })?;
    Ok(true)
}

/// The tracefile: one record per live path, one per decoy, and the suite entry.
///
/// Decoys are emitted at zero rather than omitted, and that is a statement
/// rather than filler. An omitted file is one the instrumenter never saw; a file
/// with `DA:1,0` is one it watched and never entered. The second is what is true
/// of a decoy, and emitting it is what makes the artifact exercise the miss path
/// the layer must never act on.
fn render(truth: &GroundTruth, declaration: &Declaration) -> String {
    let mut out = String::from("TN:judged-e2\n");

    for path in &truth.live_paths {
        let covered = declaration.covered_paths.contains(path);
        let functions: Vec<&str> = declaration
            .called_symbols
            .iter()
            .filter(|(file, _)| file == path)
            .map(|(_, symbol)| symbol.as_str())
            .collect();
        out.push_str(&record(path, covered, &functions));
    }

    for decoy in &truth.decoy_dead_paths {
        out.push_str(&record(decoy, false, &[]));
    }

    out.push_str(&record(
        Path::new(SUITE_ENTRY_FILE),
        true,
        &[SUITE_ENTRY_SYMBOL],
    ));
    out
}

/// One `SF:` record.
///
/// The `FN:<line>,<name>` dialect, which is c8's; Coverage.py's three-field form
/// is equally valid and the parser reads both
/// (`judged-core/tests/coverage_real_artifacts.rs`). Line numbers are synthetic
/// and ascending — nothing in this system decides anything with them, and a
/// generated artifact should not pretend to know where in a file a symbol sits.
fn record(path: &Path, covered: bool, functions: &[&str]) -> String {
    let mut out = format!("SF:{RECORDED_ROOT}/{}\n", path.display());
    let hits = u32::from(covered);

    for (index, name) in functions.iter().enumerate() {
        let line = 10 + index as u32 * 10;
        out.push_str(&format!("FN:{line},{name}\n"));
    }
    for name in functions {
        // Every function in this list is one the declaration says was called, so
        // the count is non-zero by construction. A function the fixture declares
        // live but uncalled is simply absent from `functions` and therefore has
        // no record at all — which is what an instrumenter writes for code it
        // did not resolve, and is weaker than `FNDA:0`. The file's own `DA`
        // lines carry whatever is true about it.
        out.push_str(&format!("FNDA:1,{name}\n"));
    }
    out.push_str(&format!("DA:1,{hits}\n"));
    out.push_str("end_of_record\n");
    out
}

/// The control: the suite entry, plus every symbol the declaration says ran.
///
/// The floor is the exact number of called functions the artifact should carry.
/// Exact rather than slack because a generated artifact has a known size — the
/// looseness a real repository needs is a hedge against a test suite whose
/// shape changes between runs, and nothing here changes between runs.
fn render_control(declaration: &Declaration) -> String {
    let mut out = String::from(
        "# Generated with the artifact beside it. See\n\
         # docs/decisions/2026-08-02-e2-coverage-artifacts.md\n",
    );
    out.push_str(&format!("symbol {SUITE_ENTRY_SYMBOL}\n"));
    for (_, symbol) in &declaration.called_symbols {
        out.push_str(&format!("symbol {symbol}\n"));
    }
    out.push_str(&format!(
        "min-called-functions {}\n",
        declaration.called_symbols.len() + 1
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use judged_core::coverage::Coverage;
    use std::path::PathBuf;

    fn truth() -> GroundTruth {
        GroundTruth {
            live_paths: vec![PathBuf::from("src/live.py"), PathBuf::from("src/also.py")],
            live_symbols: vec!["handle".to_string(), "quiet".to_string()],
            decoy_dead_paths: vec![PathBuf::from("src/decoy.py")],
            decoy_dead_symbols: vec!["dead".to_string()],
        }
    }

    fn planted(declaration: &Declaration) -> Coverage {
        let text = render(&truth(), declaration);
        Coverage::parse(Path::new("lcov.info"), &text).expect("the generator emits valid lcov")
    }

    /// The generated artifact says what the declaration says, and nothing more.
    #[test]
    fn a_called_symbol_is_a_call_and_an_uncalled_live_symbol_is_not() {
        let declaration = Declaration::loaded(["src/also.py"]).calling("src/live.py", "handle");
        let coverage = planted(&declaration);

        assert!(coverage.called_function("handle").is_some());
        assert!(
            coverage.called_function("quiet").is_none(),
            "a live symbol the declaration does not call gets no record, so nothing \
             rescues it"
        );
        assert!(coverage.executed_file("src/live.py").is_some());
        assert!(
            coverage.executed_file("src/also.py").is_some(),
            "declared loaded, so the file is executed even with nothing called in it"
        );
    }

    /// A decoy is watched and never entered — which is a record at zero, not an
    /// omission, and never a rescue.
    #[test]
    fn a_decoy_is_recorded_as_never_entered() {
        let coverage = planted(&Declaration::loaded(["src/live.py"]));

        assert!(
            coverage
                .files()
                .iter()
                .any(|file| file.source().ends_with("src/decoy.py")),
            "the decoy has a record: the instrumenter watched it"
        );
        assert!(
            coverage.executed_file("src/decoy.py").is_none(),
            "and never saw it run"
        );
    }

    /// The three constraints, each refused rather than trusted.
    #[test]
    fn a_declaration_that_does_not_describe_its_fixture_is_refused() {
        let truth = truth();

        let on_a_decoy = Declaration::loaded(["src/decoy.py"]);
        let error = on_a_decoy.check("m00", &truth).expect_err("refused");
        assert!(error.to_string().contains("decoy"), "{error}");

        let on_a_stranger = Declaration::loaded(["src/nowhere.py"]);
        assert!(on_a_stranger.check("m00", &truth).is_err());

        let calling_a_stranger = Declaration::default().calling("src/live.py", "invented");
        assert!(calling_a_stranger.check("m00", &truth).is_err());

        // The symbol is live and the file is live, but the file is not among the
        // fixture's live paths — caught, because an artifact that misplaces a
        // function is quietly false about where the code lives.
        let misplaced = Declaration::default().calling("src/elsewhere.py", "handle");
        assert!(misplaced.check("m00", &truth).is_err());

        let honest = Declaration::loaded(["src/also.py"]).calling("src/live.py", "handle");
        assert!(honest.check("m00", &truth).is_ok());
    }

    /// Every artifact carries a control that a broken artifact fails.
    #[test]
    fn the_generated_control_passes_on_its_own_artifact_and_fails_on_a_truncated_one() {
        let declaration = Declaration::default().calling("src/live.py", "handle");
        let control = Control::parse(
            Path::new("lcov.info.control"),
            &render_control(&declaration),
        )
        .expect("the generator emits a valid control");

        assert!(control.check(&planted(&declaration)).passed());

        // The §3.7 case: an artifact that kept its shape and lost its records.
        let boot_only = Coverage::parse(
            Path::new("lcov.info"),
            "SF:/ci/build/src/live.py\nDA:1,1\nend_of_record\n",
        )
        .expect("parses");
        let outcome = control.check(&boot_only);
        assert!(!outcome.passed());
        assert_eq!(outcome.functions_called(), 0);
    }

    /// A class that declares nothing plants nothing — a missing artifact and an
    /// empty one are different claims, and only the first is true.
    #[test]
    fn a_class_that_declares_nothing_plants_no_artifact() {
        let repo = tempfile::Builder::new()
            .prefix("judged-plant-")
            .tempdir()
            .expect("scratch");
        let planted = plant(
            repo.path(),
            "m00",
            &truth(),
            &Declaration::nothing(),
            Path::new("coverage/lcov.info"),
        )
        .expect("planting nothing is not an error");

        assert!(!planted);
        assert!(!repo.path().join("coverage/lcov.info").exists());
    }

    /// And a class that declares something plants both halves, where the layer
    /// looks for them.
    #[test]
    fn planting_writes_the_artifact_and_its_control_together() {
        let repo = tempfile::Builder::new()
            .prefix("judged-plant-")
            .tempdir()
            .expect("scratch");
        let artifact = Path::new("coverage/lcov.info");
        let planted = plant(
            repo.path(),
            "m00",
            &truth(),
            &Declaration::default().calling("src/live.py", "handle"),
            artifact,
        )
        .expect("plants");

        assert!(planted);
        let on_disk = repo.path().join(artifact);
        assert!(on_disk.is_file());
        assert!(Control::path_for(&on_disk).is_file());
    }
}
