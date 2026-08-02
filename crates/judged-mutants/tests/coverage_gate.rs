//! Observed execution measured as a rescue layer (§9.5, §3.7, §11 R1).
//!
//! This file pins the same two properties `veto_gate.rs` and `roots_gate.rs`
//! pin, because together they are what make a rescue layer safe rather than
//! merely useful, and they are what a future change would break silently:
//!
//! 1. **The layer may only ever remove claims.** Asserted on the claim *sets*,
//!    never on their sizes — a filter that dropped one claim and invented
//!    another keeps the count identical, and the count is what a summary line
//!    shows.
//! 2. **It is not a constant function.** A layer that rescues everything
//!    measures nothing (§3.7 on positive controls that always pass), so every
//!    rescue test also asserts what stayed claimed.
//!
//! And one property the other two layers do not have, because they read the
//! repository and this one reads somebody else's measurement of it: **an
//! artifact is never believed without its positive control.** The tests below
//! that withhold or break the control assert on an artifact that *would* have
//! rescued, so a regression that skipped the control would show up as a rescue
//! rather than as an absence.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use judged_core::Result;
use judged_mutants::coverage::{CoverageGap, CoveredSut, DEFAULT_ARTIFACT};
use judged_mutants::sut::{ClaimKind, Sut, SutVerdict, SymbolClaim};

/// A SUT that claims exactly what it was handed, whatever repository it is
/// pointed at — so anything the pair does differently is attributable to the
/// coverage layer and to nothing else.
struct FixedSut {
    claims: SutVerdict,
}

impl Sut for FixedSut {
    fn name(&self) -> &str {
        "fixed"
    }

    fn run(&self, _repo: &Path) -> Result<SutVerdict> {
        Ok(self.claims.clone())
    }
}

/// A tracefile in which `src/live.py` runs and `src/decoy.py` does not, and in
/// which `handle_request` is called while `unused_helper` is merely declared.
///
/// The `DA` lines on the decoy matter: they are what makes it a genuine record
/// of a file that was instrumented and never entered, rather than a file the
/// artifact forgot. Absence and zero are different claims (§6.20) and a fixture
/// that conflated them would be testing the wrong thing.
const TRACEFILE: &str = "TN:suite\n\
     SF:/home/runner/work/repo/src/live.py\n\
     FN:4,handle_request\n\
     FNDA:12,handle_request\n\
     FN:20,unused_helper\n\
     FNDA:0,unused_helper\n\
     DA:1,1\n\
     DA:4,1\n\
     DA:5,12\n\
     DA:21,0\n\
     end_of_record\n\
     SF:/home/runner/work/repo/src/decoy.py\n\
     FN:2,never_called\n\
     FNDA:0,never_called\n\
     DA:1,0\n\
     DA:2,0\n\
     end_of_record\n";

/// A control the tracefile above passes.
const CONTROL: &str = "# always-live\nsymbol handle_request\n";

struct Repo {
    _dir: tempfile::TempDir,
    root: PathBuf,
}

impl Repo {
    /// A scratch repository, optionally carrying a tracefile and a control.
    ///
    /// Not hidden, for the same reason `run_suite` does not hide its trees: a
    /// directory whose name starts with a dot is skipped outright by some
    /// toolchains.
    fn new(tracefile: Option<&str>, control: Option<&str>) -> Repo {
        let dir = tempfile::Builder::new()
            .prefix("judged-coverage-")
            .tempdir()
            .expect("create a scratch directory");
        let artifact = dir.path().join(DEFAULT_ARTIFACT);
        std::fs::create_dir_all(artifact.parent().expect("the artifact has a parent"))
            .expect("create the coverage directory");
        if let Some(tracefile) = tracefile {
            std::fs::write(&artifact, tracefile).expect("write the tracefile");
        }
        if let Some(control) = control {
            // Through `path_for` rather than by hand, so a test cannot pass
            // against a location the layer does not actually look in.
            std::fs::write(judged_core::coverage::Control::path_for(&artifact), control)
                .expect("write the control");
        }
        let root = dir.path().to_path_buf();
        Repo { _dir: dir, root }
    }
}

/// Everything in play, claimed dead: what ran, what did not, and both spellings
/// of each.
///
/// Deliberately the whole set rather than a plausible analyzer's output. A
/// filter is characterized by what it lets through, so the input has to contain
/// both kinds of thing.
fn claim_everything() -> SutVerdict {
    SutVerdict {
        claimed_dead_paths: vec![PathBuf::from("src/live.py"), PathBuf::from("src/decoy.py")],
        claimed_dead_symbols: vec![
            SymbolClaim::declared_in("handle_request", "src/live.py"),
            SymbolClaim::declared_in("unused_helper", "src/live.py"),
            SymbolClaim::declared_in("never_called", "src/decoy.py"),
        ],
    }
}

fn covered(repo: &Repo, claims: SutVerdict) -> (CoveredSut, SutVerdict) {
    let layer = CoveredSut::new(Box::new(FixedSut { claims }));
    let verdict = layer.run(&repo.root).expect("the layer runs");
    (layer, verdict)
}

fn paths(verdict: &SutVerdict) -> BTreeSet<String> {
    verdict
        .claimed_dead_paths
        .iter()
        .map(|path| path.display().to_string())
        .collect()
}

fn symbols(verdict: &SutVerdict) -> BTreeSet<String> {
    verdict
        .claimed_dead_symbols
        .iter()
        .map(|symbol| symbol.name().to_string())
        .collect()
}

/// Property 1, on the sets rather than on their sizes.
#[test]
fn the_layer_can_only_ever_remove_claims() {
    let repo = Repo::new(Some(TRACEFILE), Some(CONTROL));
    let before = claim_everything();
    let (_, after) = covered(&repo, before.clone());

    assert!(
        paths(&after).is_subset(&paths(&before)),
        "a coverage hit may only drop a path claim, never add one"
    );
    assert!(
        symbols(&after).is_subset(&symbols(&before)),
        "a coverage hit may only drop a symbol claim, never add one"
    );
}

/// Property 2. The file that ran and the function that was entered are rescued;
/// the file that did not and the function that was not are still claimed.
///
/// The second half is the one that keeps the first from being vacuous, and it is
/// also §9.5's rule in its operational form: a miss contributes **zero**. Not a
/// weak accusation, not a tiebreaker — the claim comes out exactly as it went
/// in.
#[test]
fn a_hit_rescues_and_a_miss_changes_nothing() {
    let repo = Repo::new(Some(TRACEFILE), Some(CONTROL));
    let (layer, after) = covered(&repo, claim_everything());

    assert_eq!(
        paths(&after),
        BTreeSet::from(["src/decoy.py".to_string()]),
        "the executed file is rescued and the instrumented-but-never-entered \
         one is not"
    );
    assert_eq!(
        symbols(&after),
        BTreeSet::from(["never_called".to_string(), "unused_helper".to_string()]),
        "FNDA:12 rescues; FNDA:0 leaves the claim exactly as the analyzer made it"
    );

    let run = &layer.runs()[0];
    assert!(run.had_coverage());
    assert_eq!(run.claimed, 5);
    assert_eq!(run.survived, 3);
    assert_eq!(run.rescued.len(), 2);
    assert!(run.gap.is_none());
}

/// The rescue has to carry evidence somebody can check, not a verdict.
#[test]
fn a_rescue_names_the_record_that_proved_it() {
    let repo = Repo::new(Some(TRACEFILE), Some(CONTROL));
    let (layer, _) = covered(&repo, claim_everything());
    let run = &layer.runs()[0];

    let symbol = run
        .rescued
        .iter()
        .find(|rescue| rescue.kind == ClaimKind::Symbol)
        .expect("the called function was rescued");
    assert_eq!(symbol.claim, "handle_request");
    assert_eq!(symbol.calls, Some(12));
    assert_eq!(
        symbol.source,
        PathBuf::from("/home/runner/work/repo/src/live.py"),
        "the SF: path is reported as the artifact spelled it, not rewritten to \
         look local"
    );

    let path = run
        .rescued
        .iter()
        .find(|rescue| rescue.kind == ClaimKind::Path)
        .expect("the executed file was rescued");
    assert_eq!(path.calls, None, "a path rescue is line-granular");
    assert!(
        path.detail.contains("of 4 recorded lines"),
        "{}",
        path.detail
    );
}

/// §2.3. A module that was imported and never called reports covered `def`
/// lines. That rescues the *file*, and it must rescue none of its functions.
#[test]
fn boot_only_coverage_does_not_rescue_a_single_symbol() {
    // Every line of the file covered by import alone; every function body dead.
    let tracefile = "SF:src/handlers.py\n\
         FN:4,handle_request\n\
         FNDA:0,handle_request\n\
         FN:12,health\n\
         FNDA:0,health\n\
         DA:1,1\n\
         DA:4,1\n\
         DA:12,1\n\
         end_of_record\n";
    // A control that passes on this artifact would have to assert nothing about
    // calls, so the artifact is checked against one naming a symbol that IS
    // called elsewhere — the point here is the rescue rule, not the control.
    let repo = Repo::new(
        Some(&format!(
            "{tracefile}SF:src/other.py\nFNDA:3,boot\nend_of_record\n"
        )),
        Some("symbol boot\n"),
    );

    let (layer, after) = covered(
        &repo,
        SutVerdict {
            claimed_dead_paths: vec![PathBuf::from("src/handlers.py")],
            claimed_dead_symbols: vec![
                SymbolClaim::declared_in("handle_request", "src/handlers.py"),
                SymbolClaim::declared_in("health", "src/handlers.py"),
            ],
        },
    );

    assert!(paths(&after).is_empty(), "the imported file is rescued");
    assert_eq!(
        symbols(&after),
        BTreeSet::from(["handle_request".to_string(), "health".to_string()]),
        "a def line executing at import is not a call, so no symbol is rescued"
    );
    assert_eq!(layer.runs()[0].rescued.len(), 1);
}

/// No artifact is the ordinary case across a suite, and it is not a finding.
#[test]
fn a_repository_with_no_artifact_rescues_nothing_and_says_so() {
    let repo = Repo::new(None, None);
    let before = claim_everything();
    let (layer, after) = covered(&repo, before.clone());

    assert_eq!(paths(&after), paths(&before));
    assert_eq!(symbols(&after), symbols(&before));

    let run = &layer.runs()[0];
    assert!(
        !run.had_coverage(),
        "the denominator: zero rescues with no artifact must not read like zero \
         rescues over a covered repository"
    );
    assert_eq!(run.gap.as_ref().map(CoverageGap::kind), Some("no-artifact"));
    assert!(run.control.is_none());
}

/// §3.7, and the test that would catch a regression skipping the control: the
/// artifact here is the one that rescues two claims in every other test.
#[test]
fn an_artifact_with_no_control_is_discarded_whole() {
    let repo = Repo::new(Some(TRACEFILE), None);
    let before = claim_everything();
    let (layer, after) = covered(&repo, before.clone());

    assert_eq!(
        paths(&after),
        paths(&before),
        "an artifact nobody declared a check for rescues nothing"
    );
    assert_eq!(symbols(&after), symbols(&before));

    let run = &layer.runs()[0];
    assert!(!run.had_coverage());
    assert_eq!(run.gap.as_ref().map(CoverageGap::kind), Some("no-control"));
    assert!(
        run.gap
            .as_ref()
            .expect("a gap")
            .to_string()
            .contains("§3.7"),
        "the gap says why, not merely that"
    );
}

/// The artifact §3.7 describes: written, parseable, and describing a run that
/// never happened. It must rescue nothing, and the numbers that refused it must
/// survive into the report.
#[test]
fn an_artifact_that_fails_its_control_is_discarded_whole() {
    // Boot-only: every def covered, every body dead. A control naming a symbol
    // that must have been *called* is what tells this apart from a repository
    // nothing uses.
    let repo = Repo::new(
        Some(
            "SF:/home/runner/work/repo/src/live.py\n\
             FN:4,handle_request\n\
             FNDA:0,handle_request\n\
             DA:1,1\n\
             DA:4,1\n\
             end_of_record\n",
        ),
        Some(CONTROL),
    );
    let before = claim_everything();
    let (layer, after) = covered(&repo, before.clone());

    assert_eq!(
        paths(&after),
        paths(&before),
        "src/live.py has covered lines, and is still not rescued: the artifact \
         as a whole was refused"
    );

    let run = &layer.runs()[0];
    assert!(!run.had_coverage());
    assert_eq!(
        run.gap.as_ref().map(CoverageGap::kind),
        Some("control-failed")
    );
    let outcome = run.control.as_ref().expect("the numbers that refused it");
    assert_eq!(outcome.symbols_uncalled(), ["handle_request".to_string()]);
    assert_eq!(outcome.functions_called(), 0);
}

/// A tracefile this parser cannot read is not a repository nothing uses.
#[test]
fn an_unreadable_artifact_is_a_gap_and_not_a_clean_run() {
    let repo = Repo::new(
        Some("SF:src/a.py\nFNDA:many,work\nend_of_record\n"),
        Some(CONTROL),
    );
    let before = claim_everything();
    let (layer, after) = covered(&repo, before.clone());

    assert_eq!(paths(&after), paths(&before));
    let run = &layer.runs()[0];
    assert_eq!(run.gap.as_ref().map(CoverageGap::kind), Some("unreadable"));
}

/// A control that is itself broken must not fall back to trusting the artifact.
#[test]
fn a_broken_control_discards_the_artifact_rather_than_waiving_the_check() {
    let repo = Repo::new(Some(TRACEFILE), Some("symbols handle_request\n"));
    let before = claim_everything();
    let (layer, after) = covered(&repo, before.clone());

    assert_eq!(paths(&after), paths(&before));
    assert_eq!(
        layer.runs()[0].gap.as_ref().map(CoverageGap::kind),
        Some("unreadable")
    );
}

/// An analyzer that spells its claims absolutely must still be matched against a
/// tracefile recorded on another machine.
#[test]
fn an_absolutely_spelled_claim_is_matched_against_the_artifact() {
    let repo = Repo::new(Some(TRACEFILE), Some(CONTROL));
    let (_, after) = covered(
        &repo,
        SutVerdict {
            claimed_dead_paths: vec![repo.root.join("src/live.py")],
            claimed_dead_symbols: Vec::new(),
        },
    );
    assert!(
        after.claimed_dead_paths.is_empty(),
        "a claim spelled with the local repository root still matches the \
         artifact's remote SF: path"
    );
}
