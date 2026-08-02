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
//! One precondition sits in front of it, and it is not a softening. A SUT
//! declares which ecosystems it can read and the runner skips the rest, so a
//! report can now cover fewer than nineteen classes — and over *zero* graded
//! classes, `false_removal_count == 0` is the absence of a run wearing the
//! digits of a clean one. That is refused rather than gated; see [`gate`].
//! Every skipped class is still printed, marked, and counted in its own column,
//! because a skip that reads as a pass would make narrowing an adapter's
//! declared languages a way to raise a green (§6.20).
//!
//! That leaves a hole, and the report is built around admitting it: a system
//! under test that claims nothing is ever dead scores a perfect zero and passes
//! the gate. It is also useless. So decoy recall — how many genuinely-dead
//! files the SUT actually found — is printed on the line below the gate, every
//! time, in both renderings. §3.7 and §9.8 require a positive control on every
//! evidence artifact; this is the suite's own, printed rather than enforced,
//! because turning it into an exit code would let a fixture author raise a
//! green by planting easier decoys.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use judged_core::veto::literal::{NeedleKind, NeedleStrategy};
use judged_mutants::adapters::{deadcode, knip, shear, vulture};
use judged_mutants::coverage::{CoverageRescue, CoveredSut};
use judged_mutants::fixtures;
use judged_mutants::gate1::{Gate1Sut, RefusedClaim};
use judged_mutants::mutant::{Ecosystem, Mutant};
use judged_mutants::roots::{RescuedClaim, RootedSut};
use judged_mutants::runner::{reads_mutant, run_suite, Grade, MutantReport, SuiteReport};
use judged_mutants::sut::{
    BlockedClaim, CommandSut, GateSet, NaiveSut, RefusingSut, Sut, SutVerdict, VetoedSut,
};
use serde_json::{json, Value};

use crate::args::{MutantsArgs, SutChoice};

/// Exit codes that mean a vulture run finished its analysis.
///
/// Load-bearing, not decorative: [`CommandSut`] discards the stdout of a run
/// that ended on any other code, so getting this wrong in either direction is a
/// silent scoring error. Too narrow and every productive run is refused; too
/// wide and a vulture that died on a syntax error is graded as one that found
/// nothing.
///
/// Measured rather than assumed, against **vulture 2.16**:
///
/// | Condition | Exit |
/// | --- | --- |
/// | No dead code found | 0 |
/// | Dead code found | 3 |
/// | Syntax error, or a target that does not exist | 1 |
/// | Unrecognized argument | 2 |
///
/// So the productive case is 3, not 0, and 1 has to stay out even though a
/// crashed vulture prints a plausible-looking empty result.
const VULTURE_COMPLETED_EXIT_CODES: [i32; 2] = [0, 3];

/// Exit codes that mean a knip run finished its analysis.
///
/// Measured against **knip 6.31.0** (`npx --yes knip@6 --reporter sarif
/// --no-progress --directory <repo>`), on this machine, in the fixture
/// repositories this suite materializes:
///
/// | Condition | Exit | stdout |
/// | --- | --- | --- |
/// | Analysis ran, nothing unused | 0 | a SARIF log with `"results":[]` |
/// | Analysis ran, unused files or dependencies found | 1 | a SARIF log with results |
/// | Unknown option, or a positional argument | 1 | knip's usage text |
/// | No `package.json` anywhere above the directory | 2 | a one-line pointer to `--help` |
///
/// So the productive case is **1**, and 1 is also how knip reports being called
/// wrongly. Those two are told apart by the parser rather than by the code:
/// [`knip::parse`] rejects anything that is not a SARIF log, so the usage text
/// becomes a hard error instead of an empty verdict. That is the split §6.20
/// demands — *"no data" must be a distinct state from "zero executions"* — and
/// it is why 1 can be declared healthy without declaring a misconfigured run
/// clean.
///
/// 2 stays out. It is what knip returns for the fixture repositories that hold
/// no `package.json` at all, which is most of the catalogue, and it is
/// indistinguishable from the fatal errors knip also exits 2 on.
const KNIP_COMPLETED_EXIT_CODES: [i32; 2] = [0, 1];

/// Exit codes that mean a deadcode run finished its analysis.
///
/// Measured against **`golang.org/x/tools/cmd/deadcode`** with go1.26.2 on
/// darwin/arm64, using the argv [`crate::args`] declares:
///
/// | Condition | Exit | stdout |
/// | --- | --- | --- |
/// | Analysis ran, nothing dead | 0 | the four bytes `null` |
/// | Analysis ran, dead functions found | 0 | the `Package` JSON array |
/// | Target directory holds no Go files | 1 | empty |
/// | Target is not inside a Go module | 1 | empty |
/// | A package fails to parse or type-check | 1 | empty |
/// | Unknown flag, or no package pattern at all | 2 | usage text on stderr |
///
/// **Only 0 is a completed run**, and it covers both the productive and the
/// empty case — deadcode gives a caller no exit code by which to tell "I
/// analyzed your program and it is all reachable" from "I refused". The stdout
/// does: `null` versus an array. That distinction is the adapter's, not the
/// harness's.
///
/// Note what 1 collides with, and why nothing may be added to this list to make
/// the suite quieter. "This repository has no Go in it" and "your Go does not
/// compile" are the same code and the same empty stdout. Declaring 1 healthy
/// would hand [`deadcode::verdict_from_stdout`] an empty stream for a run that
/// never analyzed anything — and an empty stream parses to no claims, which is
/// zero false removals, which is a passing gate.
const DEADCODE_COMPLETED_EXIT_CODES: [i32; 1] = [0];

/// Exit codes that mean a cargo-shear run finished its analysis.
///
/// Measured against **cargo-shear** (`--format json <repo>`), built from source
/// for this round because it needs a newer rustc than this repository pins:
///
/// | Condition | Exit | stdout |
/// | --- | --- | --- |
/// | Analysis ran, nothing found | 0 | `{"summary":{"errors":0,...},"findings":[]}` |
/// | Analysis ran, warning-severity findings only (unlinked files) | 0 | the JSON document |
/// | Analysis ran, error-severity findings (unused dependencies) | 1 | the JSON document |
/// | Unknown or malformed command-line argument | 1 | empty |
/// | No `Cargo.toml`, or `cargo metadata` failed | 2 | `error: Metadata error: ...` |
///
/// Two things here are worth stating out loud, because both are the sort of
/// detail a remembered exit-code table gets wrong.
///
/// First, cargo-shear's exit code depends on the **severity** of what it found,
/// not on whether it found anything: an unlinked file is a warning and exits 0,
/// while an unused dependency is an error and exits 1. A list of `[0]` would
/// therefore discard exactly the findings §4.1 cares most about.
///
/// Second, 1 is shared with "you called me wrongly", as it is for knip, and is
/// separated the same way — by the parser. [`shear::parse_output`] rejects an
/// empty stream and rejects the plain-text `error:` line, so neither can arrive
/// as a clean verdict. 2 stays out for the same reason knip's does: it is what
/// every non-Rust fixture produces, and it is also what a broken `Cargo.toml`
/// produces.
const SHEAR_COMPLETED_EXIT_CODES: [i32; 2] = [0, 1];

/// Run the catalogue and render the result.
pub fn run(args: &MutantsArgs) -> (String, i32) {
    // Before the fixtures, before git, before anything that takes a second: an
    // analyzer that is not on this machine must stop the run here.
    //
    // This is the whole feature's failure mode, and it is not hypothetical.
    // `CommandSut::run` turns a spawn failure into `Ok(SutVerdict::default())`
    // — an empty verdict — so a missing binary would be graded as a SUT that
    // claimed nothing dead. Nineteen classes of nothing is a false-removal
    // count of zero, which is the gate's only input, which is exit 0 and
    // "GATE PASSED". A green build certifying that an absent tool is safe to
    // trust is §6.20's disarming failure exactly, and §3.7 records that this
    // shape — an artifact reporting ~0% for everything, then believed — is how
    // every catastrophic failure in this space presented.
    if let Err(refusal) = preflight(&args.sut) {
        return (render_refusal(&refusal, &args.sut, args.json), 2);
    }

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

    let sut = build_sut(&args.sut);

    let report = match run_suite(sut.as_ref(), &mutants) {
        Ok(report) => report,
        // A crashed harness is not a passing harness. §3.7: every catastrophic
        // failure in this space presented as an artifact reporting ~0% for
        // everything, which was then trusted.
        Err(error) => {
            return (
                render_refusal(
                    &Refusal {
                        headline: "the E2 suite did not complete".to_string(),
                        detail: error.to_string(),
                        remedy: foreign_ecosystem_hint(sut.as_ref(), &mutants),
                    },
                    &args.sut,
                    args.json,
                ),
                2,
            )
        }
    };

    // Without a rescue layer the run is over: `report` is what the bare accuser
    // did, which is the number this suite has always published.
    //
    // With one, the suite is run a **second** time with the layers wrapped
    // around the same analyzer, and the report becomes the difference between
    // the two. Two runs rather than one, and the extra analyzer time is the
    // price: "how many false removals did this layer prevent" is a question
    // about a pair of runs, and deriving it from one of them would mean assuming
    // the answer.
    //
    // The layers are stacked **root set first**, and the order is an argument
    // rather than a convenience: a candidate that IS a declared entry point is
    // not a candidate at all, so there is nothing left for a reference veto to
    // weigh. Each layer keeps its own per-claim record, which is what lets the
    // report say which layer earned a rescue instead of publishing one combined
    // number.
    let (report, rescue) = if args.gate1 || args.veto || args.roots || args.coverage {
        let mut stacked: Box<dyn Sut> = build_sut(&args.sut);
        let mut gate_one: Option<Rc<Gate1Sut>> = None;
        let mut covered: Option<Rc<CoveredSut>> = None;
        let mut rooted: Option<Rc<RootedSut>> = None;
        let mut vetoed: Option<Rc<VetoedSut>> = None;
        // Innermost, so it runs FIRST. §9.3 orders the gates and Gate 1 comes
        // before the reference veto; here that ordering is the composition
        // rather than a convention, which is also what makes a Gate 1 refusal
        // absorbing — a claim it drops is never handed to a later layer, so
        // there is no later evidence for anything to override it with.
        if args.gate1 {
            let layer = Rc::new(Gate1Sut::new(stacked));
            stacked = Box::new(Rc::clone(&layer));
            gate_one = Some(layer);
        }
        // Next, and ahead of both Family-R layers on purpose. Every rescue layer
        // here is a pure filter, so the order cannot change the final claim set
        // — but it decides which layer is on the record for a claim that more
        // than one of them would have dropped, and that attribution is what §11
        // R1 is asking about. "This function ran 4,281 times" and "some file
        // spells its name" are not the same evidence, and crediting the second
        // when the first was available would overstate what a needle earned.
        if args.coverage {
            let layer = Rc::new(CoveredSut::with_artifact(
                stacked,
                args.coverage_artifact.clone(),
            ));
            stacked = Box::new(Rc::clone(&layer));
            covered = Some(layer);
        }
        if args.roots {
            let layer = Rc::new(RootedSut::new(stacked));
            stacked = Box::new(Rc::clone(&layer));
            rooted = Some(layer);
        }
        if args.veto {
            let layer = Rc::new(VetoedSut::new(stacked).with_needles(args.needles));
            stacked = Box::new(Rc::clone(&layer));
            vetoed = Some(layer);
        }

        let gated = match run_suite(stacked.as_ref(), &mutants) {
            Ok(gated) => gated,
            Err(error) => {
                return (
                    render_refusal(
                        &Refusal {
                            headline: "the rescued E2 suite did not complete".to_string(),
                            detail: error.to_string(),
                            remedy: foreign_ecosystem_hint(sut.as_ref(), &mutants),
                        },
                        &args.sut,
                        args.json,
                    ),
                    2,
                )
            }
        };

        let mut layers: Vec<Layer> = Vec::new();
        if let Some(layer) = &gate_one {
            layers.push(gate1_layer(layer));
        }
        if let Some(layer) = &covered {
            layers.push(coverage_layer(layer));
        }
        if let Some(layer) = &rooted {
            layers.push(roots_layer(layer));
        }
        if let Some(layer) = &vetoed {
            layers.push(veto_layer(layer));
        }
        match compare(
            &report,
            &gated,
            &layers,
            vetoed.as_ref().map(|layer| layer.gates()),
            args.needles,
        ) {
            Ok(summary) => (gated, Some(summary)),
            Err(refusal) => return (render_refusal(&refusal, &args.sut, args.json), 2),
        }
    } else {
        (report, None)
    };

    // The gate, and only the gate (§10 E2, §11 R1) — but not before checking
    // that there is something for it to be a gate over.
    //
    // Under `--veto` this gates on the **gated** run, because the gated run is
    // the system that was measured. The bare run's numbers do not disappear:
    // they are printed beside it, and the exit code belongs to the combination
    // §11 R1 asks about, not to half of it.
    let code = match gate(&report) {
        Ok(code) => code,
        Err(refusal) => return (render_refusal(&refusal, &args.sut, args.json), 2),
    };

    let rendered = if args.json {
        render_json(&report, &catalogue, &args.sut, rescue.as_ref())
    } else {
        render_text(&report, &catalogue, &args.sut, rescue.as_ref())
    };
    (rendered, code)
}

/// Gate 1's accounting, normalized.
///
/// The unattributed count rides into the layer's `config` string rather than
/// being dropped, and it is the one number that can turn this layer into a
/// constant function: a symbol claim the analyzer attributed to no file has no
/// file for sixteen file classes to be evaluated against, so §9.3's 1p rule
/// refuses it. An analyzer that attributed nothing would therefore score a
/// perfect false-removal record by saying nothing at all — §3.7's positive
/// control that always passes. Every shipped adapter attributes, so the number
/// is normally zero; a report that did not print it could not show that.
fn gate1_layer(layer: &Gate1Sut) -> Layer {
    let runs = layer.runs();
    let gaps: usize = runs.iter().map(|run| run.gaps.len()).sum();
    let unattributed: usize = runs.iter().map(|run| run.unattributed).sum();
    Layer {
        name: GATE1,
        config: format!(
            "classes 1a-1p, {unattributed} claim(s) refused only for having no declaration \
             site, {gaps} scan gap(s)"
        ),
        runs: runs
            .iter()
            .map(|run| LayerRun {
                claimed: run.claimed,
                survived: run.survived,
                dropped: run.refused.iter().map(Dropped::from_refused).collect(),
            })
            .collect(),
    }
}

/// The root-set layer's accounting, normalized.
///
/// The gaps ride into the layer's `config` string rather than being dropped: a
/// rescue count printed beside an unreported gap is the §6.20 shape — a layer
/// that materialized five roots and missed a whole framework's convention set
/// looks exactly like one that materialized five roots.
fn roots_layer(layer: &RootedSut) -> Layer {
    let runs = layer.runs();
    let gaps: usize = runs.iter().map(|run| run.gaps.len()).sum();
    let materialized: usize = runs.iter().map(|run| run.roots_materialized).sum();
    Layer {
        name: ROOTS,
        config: format!(
            "tiers A+B+C, {materialized} root(s) materialized across the catalogue, \
             {gaps} unresolved"
        ),
        runs: runs
            .iter()
            .map(|run| LayerRun {
                claimed: run.claimed,
                survived: run.survived,
                dropped: run.rescued.iter().map(Dropped::from_rescued).collect(),
            })
            .collect(),
    }
}

/// The coverage layer's accounting, normalized.
///
/// The `config` string leads with the number of classes that had a **believed**
/// artifact, and that number is the whole reason this function is not two lines.
/// A rescue count with no denominator is §6.20's shape at its purest here: zero
/// rescues over nineteen classes with no tracefile, and zero rescues over
/// nineteen fully covered ones, are the same integer and opposite findings. The
/// gaps are printed by cause for the same reason — "no artifact" is an ordinary
/// fact about a fixture, while "failed its control" is a broken measurement
/// somebody has to go and fix.
fn coverage_layer(layer: &CoveredSut) -> Layer {
    let runs = layer.runs();
    let believed = runs.iter().filter(|run| run.had_coverage()).count();
    let mut causes: BTreeMap<&'static str, usize> = BTreeMap::new();
    for run in runs.iter().filter_map(|run| run.gap.as_ref()) {
        *causes.entry(run.kind()).or_insert(0) += 1;
    }
    let gaps = if causes.is_empty() {
        "no gaps".to_string()
    } else {
        causes
            .into_iter()
            .map(|(cause, count)| format!("{count} {cause}"))
            .collect::<Vec<_>>()
            .join(", ")
    };

    Layer {
        name: COVERAGE,
        config: format!(
            "lcov {}, {believed} of {} class(es) had an artifact that passed its control ({gaps})",
            layer.artifact().display(),
            runs.len()
        ),
        runs: runs
            .iter()
            .map(|run| LayerRun {
                claimed: run.claimed,
                survived: run.survived,
                dropped: run.rescued.iter().map(Dropped::from_covered).collect(),
            })
            .collect(),
    }
}

/// Gate 2's accounting, normalized.
fn veto_layer(layer: &VetoedSut) -> Layer {
    Layer {
        name: VETO,
        config: format!(
            "gates {}, needles {}",
            layer
                .gates()
                .gates()
                .into_iter()
                .map(|gate| gate.as_str())
                .collect::<Vec<_>>()
                .join(", "),
            describe_needles(layer.needles())
        ),
        runs: layer
            .runs()
            .iter()
            .map(|run| LayerRun {
                claimed: run.claimed,
                survived: run.survived,
                dropped: run.blocked.iter().map(Dropped::from_blocked).collect(),
            })
            .collect(),
    }
}

// ---------------------------------------------------------------------------
// The trade
// ---------------------------------------------------------------------------

/// One claim a rescue layer dropped, normalized across layers.
///
/// The two layers answer different questions and carry different evidence — a
/// veto names the needle that fired and the file it fired in, a root names the
/// §5.1 tier and the file and key that declared it — so the union of their
/// fields is nullable in both directions. What is never null is `layer`: a
/// report that cannot say **which layer earned a rescue** publishes a combined
/// number, and §11 R1's question is about which signals earn their place.
#[derive(Debug, Clone)]
struct Dropped {
    /// `gate1`, `roots` or `veto`.
    layer: &'static str,
    /// The claim, spelled exactly as the analyzer spelled it.
    claim: String,
    kind: &'static str,
    /// Which sub-gate or which convention fired: `literal`, `django/appconfig`.
    rule: String,
    /// The veto's sub-gate, kept under its own key for a consumer that reads it.
    /// `None` for a root-set rescue, which has no gate.
    gate: Option<&'static str>,
    /// The §5.1 tier a root came from. `None` for a veto rescue, which is not a
    /// statement about entry points at all.
    tier: Option<&'static str>,
    /// The §9.3 Gate 1 class that refused, e.g. `1b`. `None` for the other two
    /// layers, neither of which is making a claim about irreversibility.
    class: Option<&'static str>,
    /// The file and key that declared a root. `None` for a veto rescue.
    origin: Option<String>,
    needle: Option<String>,
    needle_kind: Option<String>,
    found_in: Option<PathBuf>,
    declared_in: Option<PathBuf>,
    detail: String,
}

impl Dropped {
    fn from_blocked(record: &BlockedClaim) -> Dropped {
        Dropped {
            layer: VETO,
            claim: record.claim.clone(),
            kind: record.kind.as_str(),
            rule: record.gate.as_str().to_string(),
            gate: Some(record.gate.as_str()),
            tier: None,
            class: None,
            origin: None,
            needle: record.needle.clone(),
            needle_kind: record.needle_kind.clone(),
            found_in: record.found_in.clone(),
            declared_in: record.declared_in.clone(),
            detail: record.detail.clone(),
        }
    }

    fn from_rescued(record: &RescuedClaim) -> Dropped {
        Dropped {
            layer: ROOTS,
            claim: record.claim.clone(),
            kind: record.kind.as_str(),
            rule: record.rule.clone(),
            gate: None,
            tier: Some(record.tier.label()),
            class: None,
            origin: Some(record.origin.clone()),
            needle: None,
            needle_kind: None,
            // The file the declaration lives in is the file a reader opens to
            // check the rescue — the same role `found_in` plays for a veto.
            found_in: record.origin_file.clone(),
            declared_in: None,
            detail: record.detail.clone(),
        }
    }

    /// A rescue by observed execution.
    ///
    /// `rule` is `fnda` or `lines`, which is the one distinction a reader of this
    /// layer most needs: `fnda` means the function was entered, and `lines`
    /// means only that something in the file ran — which in Python, Ruby and
    /// JavaScript happens at import (§2.3). Collapsing the two into "covered"
    /// would present the weaker fact with the stronger one's confidence.
    ///
    /// `found_in` is the `SF:` path exactly as the tracefile spelled it, which is
    /// a path on whichever machine ran the tests. Left un-rewritten deliberately:
    /// a reader checking a rescue needs to see that the evidence came from
    /// somewhere else, and a locally-plausible path would hide it.
    fn from_covered(record: &CoverageRescue) -> Dropped {
        Dropped {
            layer: COVERAGE,
            claim: record.claim.clone(),
            kind: record.kind.as_str(),
            rule: match record.calls {
                Some(_) => "fnda".to_string(),
                None => "lines".to_string(),
            },
            gate: None,
            tier: None,
            class: None,
            origin: None,
            needle: record.function.clone(),
            needle_kind: record.calls.map(|calls| format!("{calls} call(s)")),
            found_in: Some(record.source.clone()),
            declared_in: None,
            detail: record.detail.clone(),
        }
    }

    /// A Gate 1 refusal.
    ///
    /// `rule` carries the §9.3 class code, because the class IS the rule that
    /// fired — `1b` says secrets and identity, and there is no narrower rule
    /// underneath it. `class` repeats it under its own key so a consumer can
    /// group by class without parsing a string it was told is free-form.
    ///
    /// `declared_in` is the file that was actually judged for a symbol claim.
    /// Gate 1's classes are properties of files, so a symbol is judged by the
    /// file that declares it, and a reader who cannot see which file that was
    /// cannot check the refusal.
    fn from_refused(record: &RefusedClaim) -> Dropped {
        Dropped {
            layer: GATE1,
            claim: record.claim.clone(),
            kind: record.kind.as_str(),
            rule: record.class.to_string(),
            gate: None,
            tier: None,
            class: Some(record.class),
            origin: None,
            needle: None,
            needle_kind: None,
            found_in: None,
            declared_in: record.declared_in.clone(),
            detail: record.detail.clone(),
        }
    }
}

/// One rescue layer's accounting for one class.
#[derive(Debug, Clone)]
struct LayerRun {
    /// Claims this layer was handed. For the inner-most layer that is what the
    /// analyzer produced; for an outer one it is what the layer below passed
    /// through.
    claimed: usize,
    survived: usize,
    dropped: Vec<Dropped>,
}

/// One rescue layer, and everything it recorded across the run.
#[derive(Debug, Clone)]
struct Layer {
    /// `roots` or `veto`.
    name: &'static str,
    /// The configuration that produced these numbers. A number obtained under
    /// one configuration is not the number obtained under another (§11 R8).
    config: String,
    /// One entry per graded class, in run order.
    runs: Vec<LayerRun>,
}

/// The `layer` tags, spelled once. They appear in JSON keys, in report lines and
/// in the SUT name, and three spellings of the same layer is how a consumer ends
/// up matching on the one this build does not emit.
const GATE1: &str = "gate1";
const COVERAGE: &str = "coverage";
const ROOTS: &str = "roots";
const VETO: &str = "veto";

/// What one layer earned, over the whole run.
#[derive(Debug)]
struct LayerLine {
    name: &'static str,
    config: String,
    /// False removals **attributed to this layer**: the live artifacts that
    /// stopped being claimed because *this* layer dropped the claim naming them.
    prevented: usize,
    /// §11 R8's flag rate, both halves, for this layer. Exact rather than
    /// attributed — a layer knows what it was handed and what it dropped.
    claims_judged: usize,
    claims_dropped: usize,
}

/// What the rescue stack changed on one class.
#[derive(Debug)]
struct RescueClass {
    /// Live artifacts the bare run claimed and the rescued run did not, each
    /// with the layer that earned it — `None` when no layer's record names it,
    /// which is a defect rather than a category and is counted as such.
    prevented: Vec<(String, Option<&'static str>)>,
    /// Genuinely-dead decoys the bare run found and the rescued run did not.
    ///
    /// Stack-level, and it cannot be otherwise: decoy recall is a **count** and
    /// not a set, so there is no artifact name to attribute a loss to. Stated
    /// rather than split with a guess.
    decoys_lost: usize,
    /// How many claims the **accuser** made here — the denominator of §11 R8's
    /// flag rate, whose numerator is [`Self::blocked`].
    ///
    /// Published because R8's two requirements are *block on any hit* and *a
    /// tolerable flag rate*, and a rate cannot be checked against a report that
    /// carries only the claims that fired. Taken from the inner-most layer's own
    /// accounting, which [`compare_runs`] refuses to publish unless
    /// `survived + dropped` equals it.
    claims_judged: usize,
    /// Every claim any layer dropped on this class, with its evidence, each
    /// tagged with the layer that dropped it.
    blocked: Vec<Dropped>,
    /// What each layer was handed and what it passed on, in the order the
    /// layers ran.
    ///
    /// [`Self::claims_judged`] is the **accuser's** count, which is the right
    /// denominator for the stack and the wrong one for any layer but the first:
    /// under `--gate1 --veto` the veto is handed only what Gate 1 passed
    /// through, and publishing the accuser's number in the veto's row would
    /// understate its flag rate by exactly the claims it never saw. The
    /// composition check in [`compare_runs`] already proves these chain — one
    /// layer's `survived` is the next layer's `claimed` — so this is that same
    /// chain, published rather than merely asserted.
    per_layer: Vec<LayerClass>,
}

/// One layer's accounting on one class.
#[derive(Debug, Clone)]
struct LayerClass {
    name: &'static str,
    claimed: usize,
    survived: usize,
}

/// The whole trade, per class and in total.
#[derive(Debug)]
struct RescueSummary {
    /// The layers in force, inner-most first, each with what it earned.
    layers: Vec<LayerLine>,
    /// Which sub-gates ran, and which needles Gate 2a derived. A number produced
    /// under one configuration is not the number produced under another (§11 R8).
    /// Empty when the veto was not in the stack.
    gates: String,
    needles: String,
    /// Prevented false removals no layer's record accounts for. Zero in a
    /// healthy run: every artifact that stopped being claimed stopped because
    /// some layer dropped the claim naming it. Published rather than folded into
    /// the total, because a number nobody can attribute is exactly the combined
    /// number this build exists to avoid.
    unattributed: usize,
    /// Live artifacts rescued, and live artifacts still removed after the veto.
    prevented: usize,
    remaining: usize,
    /// Decoys found before and after, and the difference. **The price**, and it
    /// is a first-class field rather than a note, because a report that showed
    /// only the prevented column would be selling the veto instead of measuring
    /// it (§9.13: never sort by, or present, what flatters the tool).
    decoys_bare: usize,
    decoys_gated: usize,
    decoys_lost: usize,
    decoys_total: usize,
    /// §11 R8's flag rate, both halves, over the whole run.
    ///
    /// Stated here and not left to be summed out of [`Self::classes`]: a class
    /// where Gate 2 blocked nothing and cost nothing has no row, so summing the
    /// published rows counts only the classes where the gate fired and inflates
    /// every rate derived from it — in the direction that flatters the gate.
    claims_judged: usize,
    claims_blocked: usize,
    /// Keyed by mutant id, in catalogue order.
    classes: Vec<(String, RescueClass)>,
}

/// Diff a bare run against its rescued twin, and refuse to publish a comparison
/// that violates the one property every layer in the stack has.
///
/// **A rescue layer can only ever remove claims.** That is asserted in
/// `judged-mutants/tests/veto_gate.rs` and `tests/roots_gate.rs` on the claim
/// sets themselves; this is the same property re-checked on the graded output at
/// the moment a report is about to be printed, because the report is what
/// somebody acts on. A layer that added a false removal, or found a decoy the
/// bare run missed, would mean it nominated rather than rescued — and there is
/// no rendering of that which is safe to publish.
///
/// The subset check is on the false-removal **sets**, never on their sizes: two
/// runs can remove the same number of live artifacts without removing the same
/// ones, and the sets are what carry the meaning.
fn compare(
    bare: &SuiteReport,
    rescued: &SuiteReport,
    layers: &[Layer],
    gates: Option<GateSet>,
    needles: NeedleStrategy,
) -> Result<RescueSummary, Refusal> {
    compare_runs(bare, rescued, layers, gates, needles)
}

/// [`compare`] over the values it reads, so the refusals above can be provoked
/// in a test.
///
/// A seam rather than a convenience: the branches this splits out are the ones
/// that must never be reached in production, which is exactly why they cannot
/// be left unexercised.
fn compare_runs(
    bare: &SuiteReport,
    gated: &SuiteReport,
    layers: &[Layer],
    gates: Option<GateSet>,
    needles: NeedleStrategy,
) -> Result<RescueSummary, Refusal> {
    let violation = |detail: String| Refusal {
        headline: "a rescue layer did not behave as a veto".to_string(),
        detail,
        remedy: Some(
            "This is a defect in a rescue layer or in the wrapper around it, not \
             a finding about the analyzer. Nothing about this run may be reported \
             until it is fixed."
                .to_string(),
        ),
    };

    if bare.reports.len() != gated.reports.len() {
        return Err(violation(format!(
            "the bare run covered {} classes and the gated run {}",
            bare.reports.len(),
            gated.reports.len()
        )));
    }

    // Dropped claims are attributed to classes by run order: `run_suite` calls
    // the SUT exactly once per graded class, in catalogue order, and skips the
    // rest before it materializes anything. If those two sequences ever stop
    // agreeing, the evidence would be printed beside the wrong class — so the
    // lengths are checked rather than assumed, and a mismatch is a refusal.
    let graded: Vec<&MutantReport> = gated
        .reports
        .iter()
        .filter(|row| row.grade != Grade::NotRead)
        .collect();
    for layer in layers {
        if layer.runs.len() != graded.len() {
            return Err(violation(format!(
                "the {} layer recorded {} runs but the report grades {} classes, so a \
                 rescued claim cannot be attributed to the class it came from",
                layer.name,
                layer.runs.len(),
                graded.len()
            )));
        }
    }
    // Keyed by class: how many claims the accuser made, and which claims each
    // layer dropped. Both halves, because §11 R8's flag rate is a ratio and a
    // report that carries only the numerator publishes a number nobody can
    // check.
    let mut blocked_by_class: Vec<(String, usize, Vec<Dropped>, Vec<LayerClass>)> = Vec::new();
    for (index, row) in graded.iter().enumerate() {
        let mut dropped: Vec<Dropped> = Vec::new();
        let mut per_layer: Vec<LayerClass> = Vec::new();
        let mut handed: Option<usize> = None;
        for layer in layers {
            let run = &layer.runs[index];
            if run.survived + run.dropped.len() != run.claimed {
                return Err(violation(format!(
                    "{}: the {} layer was handed {} claims and accounted for {} of them",
                    row.mutant_id,
                    layer.name,
                    run.claimed,
                    run.survived + run.dropped.len()
                )));
            }
            // The stack has to compose: what one layer passed through is what
            // the next was handed. If those disagree, the layers did not see the
            // same claim set and no per-layer attribution below means anything.
            if let Some(passed) = handed {
                if passed != run.claimed {
                    return Err(violation(format!(
                        "{}: the {} layer was handed {} claims but the layer below it passed \
                         {passed} through",
                        row.mutant_id, layer.name, run.claimed
                    )));
                }
            }
            handed = Some(run.survived);
            per_layer.push(LayerClass {
                name: layer.name,
                claimed: run.claimed,
                survived: run.survived,
            });
            dropped.extend(run.dropped.iter().cloned());
        }
        // The accuser's own count: what the inner-most layer was handed.
        let claimed = layers.first().map_or(0, |layer| layer.runs[index].claimed);
        blocked_by_class.push((row.mutant_id.clone(), claimed, dropped, per_layer));
    }

    let mut classes: Vec<(String, RescueClass)> = Vec::new();
    for (before, after) in bare.reports.iter().zip(gated.reports.iter()) {
        if before.mutant_id != after.mutant_id {
            return Err(violation(format!(
                "the two runs disagree about the catalogue: {} against {}",
                before.mutant_id, after.mutant_id
            )));
        }

        let bare_removals: BTreeSet<&str> =
            before.false_removals.iter().map(String::as_str).collect();
        let gated_removals: BTreeSet<&str> =
            after.false_removals.iter().map(String::as_str).collect();
        if !gated_removals.is_subset(&bare_removals) {
            return Err(violation(format!(
                "{}: the gated run removed live artifacts the bare run did not: {:?}. \
                 A veto may only rescue.",
                before.mutant_id,
                gated_removals
                    .difference(&bare_removals)
                    .collect::<Vec<_>>()
            )));
        }
        if after.decoys_found > before.decoys_found {
            return Err(violation(format!(
                "{}: the gated run found {} decoys and the bare run {}. A veto \
                 cannot nominate, so it cannot find anything the accuser missed.",
                before.mutant_id, after.decoys_found, before.decoys_found
            )));
        }

        // A class Gate 2 never saw — one the SUT could not read — has no run and
        // therefore no denominator, which is zero claims judged rather than a
        // missing number.
        let (claims_judged, blocked, per_layer) = blocked_by_class
            .iter()
            .find(|(id, _, _, _)| id == &before.mutant_id)
            .map(|(_, claimed, blocked, per_layer)| (*claimed, blocked.clone(), per_layer.clone()))
            .unwrap_or_default();

        classes.push((
            before.mutant_id.clone(),
            RescueClass {
                // Attributed here rather than counted here: the layer that
                // dropped the claim naming an artifact is the layer that earned
                // it, and that is the whole answer §11 R1 asks for.
                prevented: bare_removals
                    .difference(&gated_removals)
                    .map(|name| ((*name).to_string(), attribute(name, &blocked)))
                    .collect(),
                decoys_lost: before.decoys_found - after.decoys_found,
                claims_judged,
                blocked,
                per_layer,
            },
        ));
    }

    let decoys_bare: usize = bare.reports.iter().map(|row| row.decoys_found).sum();
    let decoys_gated: usize = gated.reports.iter().map(|row| row.decoys_found).sum();
    let prevented: Vec<(&String, Option<&'static str>)> = classes
        .iter()
        .flat_map(|(_, class)| class.prevented.iter().map(|(name, layer)| (name, *layer)))
        .collect();
    Ok(RescueSummary {
        layers: layers
            .iter()
            .map(|layer| LayerLine {
                name: layer.name,
                config: layer.config.clone(),
                prevented: prevented
                    .iter()
                    .filter(|(_, earned)| *earned == Some(layer.name))
                    .count(),
                // Over every run this layer made, not over the rows that
                // survived into the report. See the field.
                claims_judged: layer.runs.iter().map(|run| run.claimed).sum(),
                claims_dropped: layer.runs.iter().map(|run| run.dropped.len()).sum(),
            })
            .collect(),
        gates: gates
            .map(|gates| {
                gates
                    .gates()
                    .into_iter()
                    .map(|gate| gate.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default(),
        needles: gates.map_or_else(String::new, |_| describe_needles(needles)),
        unattributed: prevented
            .iter()
            .filter(|(_, earned)| earned.is_none())
            .count(),
        prevented: prevented.len(),
        remaining: gated.false_removal_count,
        decoys_bare,
        decoys_gated,
        decoys_lost: decoys_bare - decoys_gated,
        decoys_total: gated.reports.iter().map(|row| row.decoys_total).sum(),
        claims_judged: layers
            .first()
            .map(|layer| layer.runs.iter().map(|run| run.claimed).sum())
            .unwrap_or_default(),
        claims_blocked: layers
            .iter()
            .flat_map(|layer| layer.runs.iter())
            .map(|run| run.dropped.len())
            .sum(),
        classes,
    })
}

/// Which layer's rescue explains a live artifact that stopped being claimed.
///
/// Ground truth spells an artifact bare — a repo-relative path, or an unqualified
/// symbol — and an analyzer spells its claim however its ecosystem does. The same
/// trailing-segment rule [`judged_mutants::runner`] grades with is what joins the
/// two, for the same reason: matching on equality alone would leave a rescue
/// unattributed on a spelling difference, and an unattributed rescue is the
/// combined number this report exists to avoid.
///
/// A claim dropped by one layer never reaches the next, so no single claim can
/// be attributed twice. What can happen is two *different* claims naming one
/// artifact — an analyzer claiming both `reporting/apps.py` and the symbol in it
/// — with a different layer dropping each. The inner-most layer is credited
/// then, which is the same order the stack ran in and is why the layer list is
/// ordered rather than a set.
fn attribute(artifact: &str, dropped: &[Dropped]) -> Option<&'static str> {
    dropped
        .iter()
        .find(|record| {
            record.claim == artifact
                || ["::", ".", "/", "#"]
                    .iter()
                    .any(|sep| record.claim.ends_with(&format!("{sep}{artifact}")))
        })
        .map(|record| record.layer)
}

impl RescueSummary {
    /// What the stack changed on one class, or `None` when it changed nothing
    /// and the class has no row of its own.
    fn class(&self, mutant_id: &str) -> Option<&RescueClass> {
        self.classes
            .iter()
            .find(|(id, _)| id == mutant_id)
            .map(|(_, class)| class)
            .filter(|class| {
                !class.prevented.is_empty() || class.decoys_lost > 0 || !class.blocked.is_empty()
            })
    }

    /// What to call the stack in a summary line: the layer's own name when there
    /// is one layer, and `rescue` when there are two.
    ///
    /// A single-layer run says `veto prevented:` because that is what prevented
    /// it. A two-layer run must not, because it would be naming one layer for
    /// the other's work.
    fn stack_label(&self) -> &str {
        match self.layers.as_slice() {
            [only] => only.name,
            _ => "rescue",
        }
    }
}

/// Which needles Gate 2a derived, spelled for a report.
///
/// §11 R8 records that the parent-directory needle is the one expected to
/// dominate the flag rate, so a false-removals-prevented number means nothing
/// without the strategy that produced it.
fn describe_needles(strategy: NeedleStrategy) -> String {
    [
        NeedleKind::Basename,
        NeedleKind::Stem,
        NeedleKind::ParentDir,
        NeedleKind::Symbol,
    ]
    .into_iter()
    .filter(|kind| strategy.includes(*kind))
    .map(NeedleKind::as_str)
    .collect::<Vec<_>>()
    .join("+")
}

// ---------------------------------------------------------------------------
// Refusing to run, which is not the same as running and finding nothing
// ---------------------------------------------------------------------------

/// Why the suite did not produce a verdict.
///
/// A structure rather than a formatted string, because the same refusal has to
/// be rendered twice — once for a person and once for whatever reads `--json` —
/// and the JSON rendering is the one that would otherwise quietly turn a
/// refusal into a result.
#[derive(Debug)]
struct Refusal {
    /// One line, in the log tail a human actually reads.
    headline: String,
    /// What was looked for and what was found instead.
    detail: String,
    /// What to do about it. §9.13's presentation rules are about what somebody
    /// can do next; "vulture is missing" without "install it like this" makes
    /// the reader go and find out.
    remedy: Option<String>,
}

/// The exit code a finished run earns, or a refusal to publish one.
///
/// §10 E2 gates on `false_removal_count` and on nothing else, and that stays
/// true — with one precondition that is not a softening of it. Zero false
/// removals over **zero graded classes** is not a clean run: it is the absence
/// of a run, wearing the same digits. §6.20 is explicit that "no data" must be
/// a distinct state from "zero executions" and must never flow into a score, so
/// a report with nothing in the denominator is refused rather than gated.
///
/// This is the arithmetic that makes skipping safe. A SUT declares which
/// ecosystems it reads and the runner skips the rest; without this check, the
/// narrowest possible declaration — reads nothing — would grade nothing, remove
/// nothing, and exit 0 with "GATE PASSED". Adding a language filter to an
/// adapter would then be a way to raise a green, which is worse than the defect
/// the filter was added to fix.
fn gate(report: &SuiteReport) -> Result<i32, Refusal> {
    if report.graded_count() == 0 {
        return Err(Refusal {
            headline: format!(
                "the E2 suite graded none of its {} classes",
                report.reports.len()
            ),
            detail: "Every class was skipped: the system under test declares it reads no \
                     ecosystem present in any of them, so no repository was built and the \
                     analyzer was never run. Its false-removal count is 0 because nothing was \
                     measured, not because nothing was wrong."
                .to_string(),
            remedy: Some(
                "Widen the ecosystems this SUT declares it reads, or grade it against a \
                 catalogue in a language it can load."
                    .to_string(),
            ),
        });
    }

    Ok(if report.false_removal_count == 0 {
        0
    } else {
        1
    })
}

/// Refuse to grade a SUT whose analyzer is not on this machine.
fn preflight(choice: &SutChoice) -> Result<(), Refusal> {
    // The two in-process controls start no subprocess, so there is nothing to
    // look for and nothing that can be missing.
    //
    // `probe_program` rather than `argv[0]`: for `--sut deadcode` the argv
    // begins with `sh`, and a preflight satisfied by a shell would report every
    // machine as having deadcode installed.
    let Some(program) = choice.probe_program() else {
        return Ok(());
    };

    if locate(&program).is_some() {
        return Ok(());
    }

    // Two failures that the word "missing" would blur into one. A name that is
    // not on PATH is a tool to install; a path that is not there is a typo, or
    // a build that has not run, or the wrong working directory. Reporting the
    // second as the first describes a search that never happened and sends the
    // reader to install something they already have.
    if program.contains(std::path::MAIN_SEPARATOR) {
        return Err(Refusal {
            headline: format!("there is no analyzer at `{program}`"),
            detail: format!(
                "`{program}` has a directory in it, so it was used as the path it is rather \
                 than searched for by name. Nothing is at that path, resolved against the \
                 directory judged was started in ({}).",
                std::env::current_dir()
                    .map(|d| d.display().to_string())
                    .unwrap_or_else(|_| "unknown".to_string())
            ),
            remedy: Some(format!(
                "Check the path, or build it first. A bare name — `{}` with no directory — is \
                 looked up on PATH instead.",
                Path::new(&program)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| program.clone())
            )),
        });
    }

    let searched = match std::env::var_os("PATH") {
        Some(path) => {
            let count = std::env::split_paths(&path).count();
            format!(
                "Looked for `{program}` in the {count} director{} on PATH; it is in none of them.",
                if count == 1 { "y" } else { "ies" }
            )
        }
        None => format!("Looked for `{program}`, but PATH is not set at all."),
    };

    Err(Refusal {
        headline: format!("the analyzer `{program}` is not installed"),
        detail: searched,
        remedy: Some(install_hint(&program)),
    })
}

/// The one thing a reader needs when a language-specific analyzer stops the
/// suite, and cannot get from the tool's own message.
///
/// Every class the analyzer is handed is now one it declared it can read
/// ([`judged_mutants::sut::Sut::reads`]), so a failure here is a failure
/// *inside* its own ecosystem — a broken fixture, a broken toolchain, a
/// genuine crash — and specifically **not** the language mismatch that used to
/// end these runs on `m01`. Saying which language it reads, and how many
/// classes were skipped for being outside it, is what stops a reader spending
/// the afternoon reinstalling a tool that is fine.
///
/// Kept in the conditional voice for the same reason it always was: this hint
/// is attached to every incomplete run of a language-specific SUT, and
/// asserting a cause it cannot check would send the reader past a real bug.
fn foreign_ecosystem_hint(sut: &dyn Sut, mutants: &[Box<dyn Mutant>]) -> Option<String> {
    let langs = sut.reads()?;
    // The runner's own predicate, not a second copy of it. A reimplementation
    // here could report a different number of skipped classes from the one the
    // run actually skipped, in a message whose whole job is to explain that
    // number.
    let skipped = mutants
        .iter()
        .filter(|mutant| !reads_mutant(sut, mutant.as_ref()))
        .count();

    let spoken: Vec<&str> = langs.iter().map(|lang| ecosystem(*lang)).collect();
    Some(format!(
        "`{}` reads {}, and {skipped} of {} classes in the catalogue are outside that. Those are \
         skipped before the analyzer is spawned — never materialized, never handed over, never \
         graded — so the class named above is one this analyzer declared it CAN read, and the \
         failure is inside its own ecosystem rather than a language mismatch. Note that the \
         skipped classes are not passes: they are counted in their own column and in neither the \
         numerator nor the denominator of anything (§6.20, \"no data\" is a distinct state from \
         \"zero executions\"). Declaring the refusing exit code healthy is not the alternative \
         fix — knip and cargo-shear exit 2 for a broken project as well as an absent one, and \
         deadcode's 1 covers \"no Go here\" and \"your Go does not compile\" alike, so accepting \
         them would score a crashed run as a clean one.",
        sut.name(),
        spoken.join(" and "),
        mutants.len(),
    ))
}

/// Where `program` is, if it is anywhere.
///
/// A name is looked up on `PATH`; anything containing a separator is taken as
/// the path it looks like, so `--sut command -- ./tools/analyze` works without
/// a flag to say so.
///
/// Existence, not executability. A file that is present but not executable
/// fails later at spawn time, and that failure is loud on its own; treating it
/// as "not installed" here would print an install command at somebody whose
/// problem is a permission bit.
fn locate(program: &str) -> Option<PathBuf> {
    let candidate = Path::new(program);
    if program.contains(std::path::MAIN_SEPARATOR) {
        return candidate.is_file().then(|| candidate.to_path_buf());
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|directory| directory.join(program))
        .find(|candidate| candidate.is_file())
}

/// How to get `program`, when we know. Only ever called with a bare name.
fn install_hint(program: &str) -> String {
    match program {
        "vulture" => "Install it with `pipx install vulture`, or `pip install vulture` into the \
                      environment judged runs in. It needs Python."
            .to_string(),
        // The missing thing is npx, not knip: `--sut knip` runs
        // `npx --yes knip@6`, and npx fetches knip itself. Saying "install
        // knip" would send the reader to `npm i -g knip`, which pins a
        // different version than the one the suite grades.
        "npx" => "`npx` ships with Node.js — install Node 20 or newer (`brew install node`, or \
                  https://nodejs.org). knip itself does not need installing: `--sut knip` runs \
                  `npx --yes knip@6`, which fetches the pinned version on first use and needs \
                  network access to do it."
            .to_string(),
        "deadcode" => "Install it with `go install golang.org/x/tools/cmd/deadcode@latest`, then \
                       put `$(go env GOPATH)/bin` on PATH — that last step is the one that is \
                       usually missing. It also needs the Go toolchain at run time, because it \
                       loads the program from source."
            .to_string(),
        "cargo-shear" => "Install it with `cargo install cargo-shear`. Judged runs the binary \
                          directly rather than as `cargo shear`, so it has to be on PATH by that \
                          name. Note that recent versions need a newer rustc than this repository \
                          pins; `cargo install --locked cargo-shear` with a toolchain of its own \
                          is the way round that."
            .to_string(),
        other => format!(
            "Install `{other}` and put it on PATH, or give its path instead: \
             `--sut command -- /path/to/{other} [args...]`."
        ),
    }
}

/// A refusal, rendered for whoever asked.
fn render_refusal(refusal: &Refusal, choice: &SutChoice, json: bool) -> String {
    // Deliberately worded to avoid the strings a gate result is made of. A log
    // scanner, or a human skimming, must not be able to find the words that
    // mean "the suite ran and cleared the bar" anywhere in a report where it
    // did neither.
    const WHY: &str = "No verdict was reached and no class was graded. This is a refusal rather \
                       than a result on purpose: an analyzer that never ran claims nothing dead, \
                       which is zero false removals, which is the number that clears the release \
                       gate. Grading it would certify a tool that was not here (§3.7, §6.20).";

    if json {
        let document = json!({
            "sut": choice.label(),
            "refused": true,
            "reason": refusal.headline,
            "detail": refusal.detail,
            "remedy": refusal.remedy,
            "why_this_is_not_a_result": WHY,
        });
        // Note what is absent: `gate_passed`, `false_removal_count`, `mutants`.
        // A consumer reaching for them gets nothing rather than a zero, because
        // a zero here and a zero from a real clean run are the same bytes.
        return match serde_json::to_string_pretty(&document) {
            Ok(text) => format!("{text}\n"),
            Err(error) => format!("{{\"refused\":true,\"reason\":\"{error}\"}}\n"),
        };
    }

    let mut out = format!(
        "REFUSED — {} (exit 2)\n\n  {}\n",
        refusal.headline, refusal.detail
    );
    if let Some(remedy) = &refusal.remedy {
        out.push_str(&format!("  {remedy}\n"));
    }
    out.push_str(&format!("\n{WHY}\n"));
    out
}

/// The SUT the report is about.
fn build_sut(choice: &SutChoice) -> Box<dyn Sut> {
    match choice {
        SutChoice::Naive => Box::new(NaiveSut),
        SutChoice::Refusing => Box::new(RefusingSut),
        SutChoice::Vulture => Box::new(
            CommandSut::new("vulture", "vulture", vulture::verdict_from_stdout)
                .with_success_exit_codes(VULTURE_COMPLETED_EXIT_CODES)
                // §9.2's other non-SARIF clause: every adapter declares the
                // finding classes it structurally cannot emit, so the
                // orchestrator knows when the tool's silence means anything.
                .with_cannot_emit([vulture::CAPABILITY_ENVELOPE])
                // The coarsest entry in that envelope, and the one the runner
                // acts on rather than prints: a whole language the tool cannot
                // open. Taken from the adapter rather than restated here, so
                // the CLI and the adapter cannot disagree about what a tool
                // reads.
                .with_reads(vulture::READS.iter().copied()),
        ),
        // The three below share a shape with vulture and differ in one respect
        // worth naming: each takes its argv from [`SutChoice::external_argv`]
        // rather than repeating the program here, because for `deadcode` the
        // program is a shell and the analyzer's name lives in the argv.
        SutChoice::Knip => Box::new(
            external(choice, knip::parse)
                .with_success_exit_codes(KNIP_COMPLETED_EXIT_CODES)
                .with_cannot_emit([knip::CAPABILITY_ENVELOPE])
                .with_reads(knip::READS.iter().copied()),
        ),
        SutChoice::Deadcode => Box::new(
            external(choice, deadcode::verdict_from_stdout)
                .with_success_exit_codes(DEADCODE_COMPLETED_EXIT_CODES)
                .with_cannot_emit([deadcode::CAPABILITY_ENVELOPE])
                .with_reads(deadcode::READS.iter().copied()),
        ),
        SutChoice::Shear => Box::new(
            external(choice, shear::verdict_from_stdout)
                .with_success_exit_codes(SHEAR_COMPLETED_EXIT_CODES)
                .with_cannot_emit([shear::CAPABILITY_ENVELOPE])
                .with_reads(shear::READS.iter().copied()),
        ),
        SutChoice::Command(argv) => {
            let (program, args) = argv
                .split_first()
                .expect("argv is non-empty by construction");
            // No `with_success_exit_codes`: an arbitrary analyzer's exit codes
            // are not interpretable, so the strict default stands and a
            // non-zero exit is treated as a run that failed rather than as a
            // run that found things. Somebody who knows better can say so out
            // loud with `-- sh -c 'mytool "$@"; true' --`.
            //
            // No `with_reads` either, and for the mirror-image reason. A
            // language guessed from an argv would let the harness skip classes
            // on a claim the analyzer never made, and a skipped class is a
            // false removal that never gets counted. Unknown competence is not
            // a claim in either direction, so the escape hatch is measured on
            // the whole catalogue.
            //
            // Its stdout is parsed as vulture's format because that is the only
            // adapter that exists. The usage text says so; guessing a format
            // from the output would be the adapter being cleverer than the
            // tool, which §9.2's adapter rules forbid in both directions.
            Box::new(
                CommandSut::new(
                    choice.label(),
                    program.clone(),
                    vulture::verdict_from_stdout,
                )
                .with_args(args.to_vec()),
            )
        }
    }
}

/// A [`CommandSut`] built from a named SUT's declared argv.
///
/// One place where `external_argv()` is split into program and arguments, so
/// that a SUT whose analyzer is not `argv[0]` — `deadcode`, which is run through
/// a one-line `sh -c` because it takes package patterns rather than a directory
/// — cannot end up with a different argv here than the one
/// [`SutChoice::probe_program`] was checked against.
fn external(choice: &SutChoice, parse: fn(&str) -> judged_core::Result<SutVerdict>) -> CommandSut {
    let argv = choice
        .external_argv()
        .expect("a named external SUT declares an argv");
    let (program, args) = argv
        .split_first()
        .expect("a named external SUT's argv is non-empty by construction");
    CommandSut::new(choice.label(), program.clone(), parse).with_args(args.to_vec())
}

/// What the reader has to know about the translation before they read a number
/// produced through it.
///
/// §9.2's second non-SARIF clause requires every adapter to declare the finding
/// classes it structurally cannot emit; the vulture adapter also states which
/// half of a verdict it fills and which it leaves empty, and calls the resulting
/// count a lower bound. A score reported without that is a score somebody will
/// read as vulture's blast radius when it is the adapter's floor on it.
struct Disclosure {
    envelope: &'static str,
    mapping: &'static str,
}

/// The escape hatch's envelope, which is that there isn't one.
const UNDECLARED_ENVELOPE: &str = "\
capability envelope: NOT DECLARED. This analyzer was named on the command line, \
so nothing is known about what it structurally cannot emit, and its silence is \
therefore not evidence about anything. A low false-removal count here bounds \
this run only (§9.2).";

fn disclosure(choice: &SutChoice) -> Option<Disclosure> {
    match choice {
        // The controls are this repository's own code, described where they are
        // defined; there is no third-party translation to disclose.
        SutChoice::Naive | SutChoice::Refusing => None,
        SutChoice::Vulture => Some(Disclosure {
            envelope: vulture::CAPABILITY_ENVELOPE,
            mapping: vulture::MAPPING_DECISION,
        }),
        SutChoice::Knip => Some(Disclosure {
            envelope: knip::CAPABILITY_ENVELOPE,
            mapping: knip::MAPPING_DECISION,
        }),
        SutChoice::Deadcode => Some(Disclosure {
            envelope: deadcode::CAPABILITY_ENVELOPE,
            mapping: deadcode::MAPPING_DECISION,
        }),
        SutChoice::Shear => Some(Disclosure {
            envelope: shear::CAPABILITY_ENVELOPE,
            mapping: shear::MAPPING_DECISION,
        }),
        // Its stdout is read by the vulture adapter, so the mapping decision
        // applies verbatim; the envelope cannot.
        SutChoice::Command(_) => Some(Disclosure {
            envelope: UNDECLARED_ENVELOPE,
            mapping: vulture::MAPPING_DECISION,
        }),
    }
}

/// One blocked claim, as the conflict list entry §9.13 asks for.
///
/// `needle` and `found_in` are `null` rather than absent when the veto came from
/// a search that did not complete: nothing fired, and the reason nothing fired
/// is that nothing looked (§6.20). A consumer that sees a null needle beside a
/// populated `detail` is looking at exactly that case.
///
/// `declared_in` sits next to `found_in` so the two can be compared, and that
/// comparison is the point. For a symbol claim, Gate 2a excludes the declaring
/// file and searches the rest, so the two fields differing is what makes a
/// rescue a *cross-file* reference rather than the symbol's own declaration read
/// back at it. Without both, the two are indistinguishable in the output — which
/// is how a Gate 2a that rescued every symbol claim in the suite passed review.
/// `null` means the analyzer named no file, so nothing was excluded and the
/// rescue is unchecked in exactly that way; it is `null` rather than absent for
/// the same reason as the fields above.
/// `layer`, `tier` and `origin` are the root set's half of the same contract.
/// A Tier B rescue is a guess about a framework, and a record that did not say
/// so would present it with a manifest's confidence — which `judged_core::roots`
/// names as worse than emitting no root at all.
fn blocked_json(record: &Dropped) -> Value {
    json!({
        "layer": record.layer,
        "claim": record.claim,
        "kind": record.kind,
        "rule": record.rule,
        "gate": record.gate,
        "tier": record.tier,
        "class": record.class,
        "origin": record.origin,
        "needle": record.needle,
        "needle_kind": record.needle_kind,
        "found_in": record.found_in.as_ref().map(|path| path.display().to_string()),
        "declared_in": record.declared_in.as_ref().map(|path| path.display().to_string()),
        "detail": record.detail,
    })
}

/// One layer's share of the stack's prevented false removals.
fn layer_share(rescue: &RescueSummary, name: &str) -> usize {
    rescue
        .layers
        .iter()
        .find(|layer| layer.name == name)
        .map_or(0, |layer| layer.prevented)
}

/// One layer's flag rate, both halves: claims handed to it, claims it dropped.
fn layer_claims(rescue: &RescueSummary, name: &str) -> (usize, usize) {
    rescue
        .layers
        .iter()
        .find(|layer| layer.name == name)
        .map_or((0, 0), |layer| (layer.claims_judged, layer.claims_dropped))
}

/// One layer's configuration, as the report states it back.
fn layer_config(rescue: &RescueSummary, name: &str) -> String {
    rescue
        .layers
        .iter()
        .find(|layer| layer.name == name)
        .map(|layer| layer.config.clone())
        .unwrap_or_default()
}

/// The JSON spelling of a grade. Lower-case and stable, because a consumer will
/// match on it.
fn grade_name(grade: Grade) -> &'static str {
    match grade {
        Grade::Passed => "passed",
        Grade::Failed => "failed",
        Grade::NotRead => "not_read",
    }
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
    let passed = report.passed_count();
    // Summed over the whole report, which needs no filtering: a class the SUT
    // could not read was never materialized, so it declared no decoys and
    // contributes zero to both halves. That is the point of skipping before
    // materialization rather than after — the exclusion is structural instead
    // of being a condition somebody has to remember to write here.
    let decoys_found = report.reports.iter().map(|r| r.decoys_found).sum();
    let decoys_total = report.reports.iter().map(|r| r.decoys_total).sum();
    (passed, decoys_found, decoys_total)
}

/// What the report calls the system it measured.
///
/// Under a rescue layer the measured system is the analyzer **and** that layer,
/// and the label says so, inner-most layer first — `naive+roots+veto`. A rescued
/// run publishes a smaller false-removal count than the same analyzer bare, and
/// two runs that reported the same `sut` would be compared as if the tool had
/// improved.
///
/// Built from the summary's own layer list rather than from the flags, so the
/// name and the numbers cannot disagree about which layers were in force.
fn sut_label(choice: &SutChoice, rescue: Option<&RescueSummary>) -> String {
    let mut label = choice.label();
    for layer in rescue.map(|rescue| rescue.layers.as_slice()).unwrap_or(&[]) {
        label.push('+');
        label.push_str(layer.name);
    }
    label
}

/// How many blocked claims a class shows in the text rendering before it stops
/// listing them.
///
/// A cap, not a filter: the count of what is not shown is printed, and `--json`
/// carries every one. §9.13 budgets a human ten seconds, and a control that
/// blocks forty claims on each of nineteen classes would push the gate line off
/// the bottom of the log.
const BLOCKED_SHOWN: usize = 6;

fn render_text(
    report: &SuiteReport,
    catalogue: &[(String, String, String)],
    choice: &SutChoice,
    rescue: Option<&RescueSummary>,
) -> String {
    let classes = report.reports.len();
    let (passed, decoys_found, decoys_total) = totals(report);

    let mut out = format!(
        "judged mutants — §10 E2, {classes} injected liveness mechanisms, SUT `{}`\n\n\
         \x20 Any \"dead\" verdict on an injected live artifact is a hard failure, not a tuning\n\
         \x20 opportunity (§10 E2). Decoys are genuinely-dead files planted beside them, so that\n\
         \x20 a tool which refuses to answer cannot score a perfect run.\n\n",
        sut_label(choice, rescue)
    );

    if let Some(rescue) = rescue {
        for layer in &rescue.layers {
            out.push_str(&match layer.name {
                ROOTS => format!(
                    "The root set (§5) was materialized for every fixture and ran on every claim:\n\
                     {}.\nA candidate that IS a declared or discovered root is never claimed \
                     dead. Every rescue\ncarries the §5.1 tier it came from, because a Tier B \
                     root is a guess about a framework.\n\n",
                    layer.config
                ),
                GATE1 => format!(
                    "Gate 1 (§9.3) — the never-touch inventory — ran BEFORE anything else, on \
                     every\nclaim this analyzer made: {}.\nIts refusals are about the cost of \
                     being wrong, not about usefulness: a claim it\nrefuses is never handed to a \
                     later layer, so there is no evidence to override it.\n\n",
                    layer.config
                ),
                COVERAGE => format!(
                    "Observed execution (§9.5, Family X) was ingested for every fixture and ran \
                     on\nevery claim: {}.\nA HIT is proof of use and drops the claim. A MISS \
                     contributes ZERO at any tier, because\nthe untested path is systematically \
                     the valuable one. An artifact with no positive\ncontrol beside it, or one \
                     that fails it, is discarded whole and rescues nothing (§3.7).\n\n",
                    layer.config
                ),
                // Gate 2. The catch-all rather than a named arm because the veto
                // is the layer every other one is described relative to, and a
                // fifth layer arriving with no headline of its own should read
                // as the generic case rather than silently claim to be Gate 1.
                _ => format!(
                    "Gate 2 (§9.3) ran on every claim this analyzer made: {}.\n\
                     A veto can only RESCUE, never nominate, so the gated claim set is a subset \
                     of the\nbare one. Both halves of the trade are below: what it prevented, and \
                     what it cost.\n\n",
                    layer.config
                ),
            });
        }
    }

    // Printed above the table rather than below the summary. §9.13 budgets a
    // human ten seconds and puts the numbers that decide something in the log
    // tail, so a page of adapter prose goes where it answers "which grading am
    // I looking at" — before the rows — and not where it would push the gate
    // line off the bottom of a CI log.
    if let Some(disclosure) = disclosure(choice) {
        out.push_str(&format!(
            "{}\n\n{}\n\n",
            disclosure.envelope, disclosure.mapping
        ));
    }

    for row in &report.reports {
        let (mechanism, research) = lookup(catalogue, &row.mutant_id);
        out.push_str(&mutant_line(row, mechanism));
        for removed in &row.false_removals {
            // Indented under its class and spelled out, because this line is
            // the finding: a live artifact the tool would have deleted, and the
            // documented incident class it came from.
            out.push_str(&format!("       removed live: {removed}   [{research}]\n"));
        }
        if let Some(class) = rescue.and_then(|rescue| rescue.class(&row.mutant_id)) {
            for (rescued, layer) in &class.prevented {
                // The layer leads the line. Which layer earned a rescue is the
                // finding — m10 is rescued by a Tier B convention and by nothing
                // else, and a line that only said "rescued" would lose exactly
                // that.
                out.push_str(&format!(
                    "       {} rescued live: {rescued}   [{research}]\n",
                    layer.unwrap_or("unattributed")
                ));
            }
            if class.decoys_lost > 0 {
                out.push_str(&format!(
                    "       the stack also rescued {} genuinely-dead decoy file(s) — the price\n",
                    class.decoys_lost
                ));
            }
            // The conflict list §9.13 asks for, and the usage list §7.3 records
            // IntelliJ Safe Delete showing instead of a probability: what fired,
            // and where.
            //
            // A symbol also states where it was declared, because `detail`
            // names the file the evidence was found in and those two being the
            // same file is a rescue that checked nothing. One line holding only
            // the second of them cannot be told from a genuine cross-file
            // reference — the gap that let a Gate 2a rescuing every symbol claim
            // read as a working gate.
            for record in class.blocked.iter().take(BLOCKED_SHOWN) {
                let declared = match &record.declared_in {
                    Some(path) => format!(" (declared in {})", path.display()),
                    // Nothing borrowed from `found_in` to fill the gap: the
                    // analyzer named no file, so Gate 2a excluded none, and
                    // saying so is the honest form of this row.
                    None => String::new(),
                };
                out.push_str(&format!(
                    "       [{layer}/{rule}] rescued {kind} {claim}{declared} — {detail}\n",
                    layer = record.layer,
                    rule = record.rule,
                    kind = record.kind,
                    claim = record.claim,
                    detail = record.detail,
                ));
            }
            if class.blocked.len() > BLOCKED_SHOWN {
                out.push_str(&format!(
                    "       … and {} more rescued claim(s) on this class; --json lists every one\n",
                    class.blocked.len() - BLOCKED_SHOWN
                ));
            }
        }
    }

    // Summary lines are unindented: they are what a CI log tail shows, and what
    // a human reads in the ten seconds §9.13 budgets.
    //
    // Three columns, because there are three states. The old two-column line
    // spent the unread classes as failures, which was wrong in the harmless
    // direction; folding them into `passed` instead would have been wrong in
    // the direction that ships an auto-act tier (§6.20).
    let unread = report.not_read_count();
    out.push_str(&format!(
        "\n{classes} classes: {} graded — {passed} passed, {} failed; {unread} not read\n",
        report.graded_count(),
        report.failed_count(),
    ));
    // Stated as its own line rather than a footnote, because it is the single
    // number most likely to be misread out of this report. A Python-only tool
    // scored against 19 classes has genuinely been measured on far fewer, and
    // a summary that does not say so invites "vulture only broke 4 of 19".
    if unread > 0 {
        out.push_str(&format!(
            "not measured: {unread} of {classes} classes are outside this SUT's languages — \
             they were never built and never handed to it, so they are in neither column above \
             and in neither half of the decoy line below\n"
        ));
    }
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

    // The two halves of the trade, on adjacent lines and in the same shape.
    // Printed together on purpose: a report showing only what the veto prevented
    // would be selling it, and §9.13 invariant 3 forbids presenting the flattering
    // number without the one that pays for it.
    if let Some(rescue) = rescue {
        // One `prevented` line per layer, so a two-layer run cannot report one
        // layer's rescues under the other's name. With a single layer this is
        // the line it has always been.
        for layer in &rescue.layers {
            out.push_str(&format!(
                "{} prevented: {} false removal(s) — {} bare, {} still removed after {}\n",
                layer.name,
                layer.prevented,
                rescue.prevented + rescue.remaining,
                rescue.remaining,
                rescue.stack_label(),
            ));
        }
        if rescue.unattributed > 0 {
            // Never folded into a layer's column. An artifact that stopped being
            // claimed with no layer's record naming it means the attribution
            // join is broken, and a report that hid that would publish exactly
            // the combined number this build exists to avoid.
            out.push_str(&format!(
                "unattributed: {} prevented false removal(s) no layer's record accounts for — \
                 this is a defect in the report, not a property of the analyzer\n",
                rescue.unattributed
            ));
        }
        out.push_str(&format!(
            "{} cost: {} decoy(s) lost — {} of {} found bare, {} of {} found after the stack\n",
            rescue.stack_label(),
            rescue.decoys_lost,
            rescue.decoys_bare,
            rescue.decoys_total,
            rescue.decoys_gated,
            rescue.decoys_total,
        ));
        if rescue.decoys_gated == 0 && rescue.decoys_bare > 0 {
            out.push_str(
                "note: the gated combination found no genuinely-dead file at all. It reached this \
                 false-removal count the way a tool that refuses to answer reaches it, and §11 R1 \
                 asks whether a signal combination is USABLE, not only whether it is safe.\n",
            );
        }
    }

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
        "  {id}  {verdict:4}  {ecosystem:10}  {false_removals} false  {found}/{total} decoys  {mechanism}{note}\n",
        id = row.mutant_id,
        // Three verdicts, and `----` rather than a word for the third. A class
        // that was never attempted has no verdict, and any word in this column
        // would be read as one — "skip" most of all, which sounds like a
        // decision the analyzer made about the code.
        verdict = match row.grade {
            Grade::Passed => "pass",
            Grade::Failed => "FAIL",
            Grade::NotRead => "----",
        },
        ecosystem = ecosystem(row.ecosystem),
        false_removals = row.false_removals.len(),
        found = row.decoys_found,
        total = row.decoys_total,
        // Spelled out beside the dashes, because the dashes alone are easy to
        // read as a rendering artifact. The zeros on this row are not findings;
        // they are the absence of a measurement.
        note = if row.grade == Grade::NotRead {
            "  [NOT READ by this SUT]"
        } else {
            ""
        },
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
    choice: &SutChoice,
    rescue: Option<&RescueSummary>,
) -> String {
    let (passed, decoys_found, decoys_total) = totals(report);
    let layer_names: Vec<&'static str> = rescue
        .map(|rescue| rescue.layers.iter().map(|layer| layer.name).collect())
        .unwrap_or_default();

    let mutants: Vec<Value> = report
        .reports
        .iter()
        .map(|row| {
            let (mechanism, research) = lookup(catalogue, &row.mutant_id);
            // One object per layer that ran, each carrying only that layer's own
            // records. A reader asking "what did the root set earn here" must not
            // have to filter a merged list, and a merged list is how a Tier B
            // guess ends up counted as a reference the veto found.
            let class = rescue.and_then(|rescue| rescue.class(&row.mutant_id));
            // The key holding the claim list differs by layer, and deliberately:
            // a veto BLOCKS a claim on evidence it went looking for, and a root
            // set RESCUES one because the artifact was declared an entry point.
            // Naming both "blocked" would flatten that back into one signal.
            let layer_row = |name: &'static str, claims_key: &'static str| {
                class.map(|class| {
                    let mut object = serde_json::Map::new();
                    object.insert(
                        "prevented_false_removals".to_string(),
                        json!(class
                            .prevented
                            .iter()
                            .filter(|(_, earned)| *earned == Some(name))
                            .map(|(artifact, _)| artifact.clone())
                            .collect::<Vec<String>>()),
                    );
                    // Stack-level, and the key says so: decoy recall is a count
                    // and not a set, so a lost decoy cannot be attributed to one
                    // layer.
                    object.insert(
                        "decoys_lost_by_the_stack".to_string(),
                        json!(class.decoys_lost),
                    );
                    // The flag rate's denominator (§11 R8), beside the numerator
                    // below, and it is THIS layer's denominator rather than the
                    // accuser's: a layer stacked above another is handed only
                    // what the one below passed through. Without both halves a
                    // published fire rate cannot be re-derived from this report.
                    let mine = class.per_layer.iter().find(|entry| entry.name == name);
                    object.insert(
                        "claims_judged".to_string(),
                        json!(mine.map_or(class.claims_judged, |entry| entry.claimed)),
                    );
                    object.insert(
                        "claims_survived".to_string(),
                        json!(mine.map_or(class.claims_judged, |entry| entry.survived)),
                    );
                    // Every one, uncapped. The text rendering shows the first
                    // few; a machine gets the whole conflict list, because §9.13
                    // asks for a list somebody can check rather than a score they
                    // have to believe.
                    object.insert(
                        claims_key.to_string(),
                        json!(class
                            .blocked
                            .iter()
                            .filter(|record| record.layer == name)
                            .map(blocked_json)
                            .collect::<Vec<Value>>()),
                    );
                    Value::Object(object)
                })
            };
            // `refused_claims`, not `blocked_claims` and not `rescued_claims`.
            // The three verbs are different acts: Gate 2 BLOCKS a claim on
            // evidence it went looking for, the root set RESCUES one because the
            // artifact was declared an entry point, and Gate 1 REFUSES one
            // because destroying it is not reversible — a statement about cost,
            // not about usefulness. Flattening them into one word would hide the
            // distinction §9.3 draws between Gate 1 and everything after it.
            let gate1_row = layer_names
                .contains(&GATE1)
                .then(|| layer_row(GATE1, "refused_claims"))
                .flatten();
            // A fourth verb, and it is the one word none of the other three can
            // carry: this layer does not weigh evidence about a claim, it
            // reports that the artifact WAS EXECUTED. Calling it "rescued"
            // alongside the root set would file direct observation under the
            // same word as a framework convention that was inferred.
            let coverage_row = layer_names
                .contains(&COVERAGE)
                .then(|| layer_row(COVERAGE, "executed_claims"))
                .flatten();
            let veto_row = layer_names
                .contains(&VETO)
                .then(|| layer_row(VETO, "blocked_claims"))
                .flatten();
            let roots_row = layer_names
                .contains(&ROOTS)
                .then(|| layer_row(ROOTS, "rescued_claims"))
                .flatten();
            json!({
                "id": row.mutant_id,
                "ecosystem": ecosystem(row.ecosystem),
                "mechanism": mechanism,
                "research_ref": research,
                // Both, and in this order. `passed` is what a consumer written
                // before this build already reads, and it is false for an
                // unread class — but false alone reads as "failed", so the
                // three-state field is emitted beside it rather than instead of
                // it. A consumer that ignores `grade` under-credits the tool;
                // one that inferred a pass from `not_read` would over-credit
                // it, and only the second error ships something.
                "grade": grade_name(row.grade),
                "passed": row.passed(),
                "false_removals": row.false_removals,
                "decoys_found": row.decoys_found,
                "decoys_total": row.decoys_total,
                // In the order the layers run (§9.3), so a reader following the
                // trace reads it the way the pipeline evaluated it.
                "gate1": gate1_row,
                "coverage": coverage_row,
                "veto": veto_row,
                "roots": roots_row,
            })
        })
        .collect();

    let document = json!({
        // `vulture+veto`, not `vulture`, when Gate 2 ran. The measured system is
        // the pair, and a consumer comparing two runs under the same name would
        // read the veto's rescues as the analyzer having improved.
        "sut": sut_label(choice, rescue),
        // Absent for the two in-process controls, present for anything that
        // went through an adapter. A consumer that records `false_removal_count`
        // without it has recorded a number stripped of what bounds it.
        "adapter": disclosure(choice).map(|d| json!({
            "capability_envelope": d.envelope,
            "mapping_decision": d.mapping,
        })),
        "classes": report.reports.len(),
        "graded_classes": report.graded_count(),
        "passed_classes": passed,
        "failed_classes": report.failed_count(),
        // Emitted whether or not it is zero, so a consumer can require the key
        // and notice a producer that predates it. A dashboard that reads
        // `false_removal_count` without this one has recorded a numerator with
        // no denominator (§6.20).
        "not_read_classes": report.not_read_count(),
        "false_removal_count": report.false_removal_count,
        "gate_passed": report.false_removal_count == 0,
        "decoys_found": decoys_found,
        "decoys_total": decoys_total,
        "classes_with_false_removals": failing_classes(report),
        // Absent without a rescue layer, so a consumer can tell a rescued report
        // from a bare one by the presence of a key rather than by parsing a
        // name. The stack's view, and the one place where every number is
        // consistent: `prevented + remaining == bare` always holds here, whereas
        // a layer's own `false_removals_prevented` is that layer's SHARE.
        "rescue": rescue.map(|rescue| json!({
            "layers": rescue.layers.iter().map(|layer| json!({
                "name": layer.name,
                "config": layer.config,
                "false_removals_prevented": layer.prevented,
                "claims_judged": layer.claims_judged,
                "claims_rescued": layer.claims_dropped,
            })).collect::<Vec<Value>>(),
            "false_removals_prevented": rescue.prevented,
            // Zero in a healthy run. Published rather than folded into a layer's
            // column, because a rescue nobody can attribute is the combined
            // number this report exists to avoid.
            "false_removals_prevented_unattributed": rescue.unattributed,
            "false_removals_remaining": rescue.remaining,
            "false_removals_bare": rescue.prevented + rescue.remaining,
            "decoys_lost": rescue.decoys_lost,
            "decoys_found_bare": rescue.decoys_bare,
            "decoys_found_rescued": rescue.decoys_gated,
            "decoys_total": rescue.decoys_total,
            "claims_judged": rescue.claims_judged,
            "claims_rescued": rescue.claims_blocked,
        })),
        // Absent without `--veto`. Its `false_removals_prevented` is Gate 2's
        // OWN share of the stack's rescues; everything else in it is the stack's
        // (a decoy loss cannot be attributed to one layer, see `RescueClass`).
        "veto": layer_names.contains(&VETO).then_some(()).and(rescue).map(|rescue| json!({
            "enabled": true,
            "gates": rescue.gates,
            "needles": rescue.needles,
            // Both columns, always, and neither is derivable from the other.
            "false_removals_prevented": layer_share(rescue, VETO),
            "false_removals_remaining": rescue.remaining,
            "false_removals_bare": rescue.prevented + rescue.remaining,
            "decoys_lost": rescue.decoys_lost,
            "decoys_found_bare": rescue.decoys_bare,
            "decoys_found_gated": rescue.decoys_gated,
            "decoys_total": rescue.decoys_total,
            // §11 R8's flag rate, both halves, over every claim Gate 2 saw.
            // Not summable from the per-class rows: a class where nothing
            // fired has no row and still had its claims judged.
            "claims_judged": layer_claims(rescue, VETO).0,
            "claims_blocked": layer_claims(rescue, VETO).1,
        })),
        // Absent without `--roots`, and shaped like the veto's so the two can be
        // read side by side in a sweep.
        "roots": layer_names.contains(&ROOTS).then_some(()).and(rescue).map(|rescue| json!({
            "enabled": true,
            "tiers": layer_config(rescue, ROOTS),
            "false_removals_prevented": layer_share(rescue, ROOTS),
            "false_removals_remaining": rescue.remaining,
            "false_removals_bare": rescue.prevented + rescue.remaining,
            "decoys_lost": rescue.decoys_lost,
            "decoys_found_bare": rescue.decoys_bare,
            "decoys_found_rescued": rescue.decoys_gated,
            "decoys_total": rescue.decoys_total,
            "claims_judged": layer_claims(rescue, ROOTS).0,
            "claims_rescued": layer_claims(rescue, ROOTS).1,
        })),
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
                grade: if count == 0 && decoys_found == decoys_total {
                    Grade::Passed
                } else {
                    Grade::Failed
                },
                false_removals,
                decoys_found,
                decoys_total,
            }],
            false_removal_count: count,
        }
    }

    /// The same, over two classes, so a run whose gate fired on one of them can
    /// be told from one that fired on both.
    fn two_class_suite(first: Vec<String>, second: Vec<String>) -> SuiteReport {
        let mut suite = suite(first, 1, 1);
        let mut row = suite.reports[0].clone();
        row.mutant_id = "m02".to_string();
        row.false_removals = second;
        row.grade = if row.false_removals.is_empty() {
            Grade::Passed
        } else {
            Grade::Failed
        };
        suite.false_removal_count += row.false_removals.len();
        suite.reports.push(row);
        suite
    }

    /// One Gate 2 run over one class, blocking `blocked` of `claimed` claims.
    fn veto_run(claimed: usize, blocked: Vec<BlockedClaim>) -> LayerRun {
        LayerRun {
            claimed,
            survived: claimed - blocked.len(),
            dropped: blocked.iter().map(Dropped::from_blocked).collect(),
        }
    }

    /// One root-set run over one class, rescuing `rescued` of `claimed` claims.
    fn roots_run(claimed: usize, rescued: Vec<RescuedClaim>) -> LayerRun {
        LayerRun {
            claimed,
            survived: claimed - rescued.len(),
            dropped: rescued.iter().map(Dropped::from_rescued).collect(),
        }
    }

    /// A Tier B convention root that rescued a claim — m10's shape, reduced.
    fn rescued_by_convention(
        claim: &str,
        kind: judged_mutants::sut::ClaimKind,
        rule: &str,
    ) -> RescuedClaim {
        RescuedClaim {
            claim: claim.to_string(),
            kind,
            tier: judged_mutants::roots::Tier::B,
            rule: rule.to_string(),
            origin: "pyproject.toml#django in [project] dependencies".to_string(),
            origin_file: Some(PathBuf::from("pyproject.toml")),
            target: claim.to_string(),
            detail: format!("django >=4.2 loads it by convention ({rule})"),
        }
    }

    /// The layer list `run` builds, for a run with Gate 2 only.
    fn veto_layer_of(runs: &[LayerRun]) -> Vec<Layer> {
        vec![Layer {
            name: VETO,
            config: "gates literal, reachability, needles basename+stem".to_string(),
            runs: runs.to_vec(),
        }]
    }

    fn blocked_path(claim: &str, needle: &str, found_in: &str) -> BlockedClaim {
        BlockedClaim {
            claim: claim.to_string(),
            kind: judged_mutants::sut::ClaimKind::Path,
            gate: judged_mutants::sut::Gate::Literal,
            needle: Some(needle.to_string()),
            needle_kind: Some("stem".to_string()),
            found_in: Some(PathBuf::from(found_in)),
            declared_in: None,
            detail: format!("{found_in} names it: the stem needle {needle:?} occurs at byte 12"),
        }
    }

    /// A symbol Gate 2a rescued. `declared_in` is what the analyzer said, so
    /// `None` spells the case where it said nothing.
    fn blocked_symbol(claim: &str, declared_in: Option<&str>, found_in: &str) -> BlockedClaim {
        BlockedClaim {
            claim: claim.to_string(),
            kind: judged_mutants::sut::ClaimKind::Symbol,
            gate: judged_mutants::sut::Gate::Literal,
            needle: Some(claim.to_string()),
            needle_kind: Some("symbol".to_string()),
            found_in: Some(PathBuf::from(found_in)),
            declared_in: declared_in.map(PathBuf::from),
            detail: format!("{found_in} names it: the symbol needle {claim:?} occurs at byte 12"),
        }
    }

    /// The bare/gated pair the CLI diffs, compared the way `run` compares it.
    fn summary(bare: &SuiteReport, gated: &SuiteReport, runs: &[LayerRun]) -> RescueSummary {
        compare_runs(
            bare,
            gated,
            &veto_layer_of(runs),
            Some(GateSet::default()),
            judged_mutants::sut::DEFAULT_NEEDLES,
        )
        .expect("the fixture pair satisfies the invariant")
    }

    /// A one-row suite the SUT could not read, in the ecosystem the caller
    /// names — the shape [`run_suite`] produces for a skipped class: no ground
    /// truth, no claims, and a grade that is neither of the other two.
    fn unread_suite(ecosystem: Ecosystem) -> SuiteReport {
        SuiteReport {
            sut_name: "test".to_string(),
            reports: vec![MutantReport {
                mutant_id: "m01".to_string(),
                ecosystem,
                grade: Grade::NotRead,
                false_removals: Vec::new(),
                decoys_found: 0,
                decoys_total: 0,
            }],
            false_removal_count: 0,
        }
    }

    #[test]
    fn a_class_the_sut_could_not_read_is_marked_and_counted_out() {
        // Vulture is a Python AST tool. Handed a Rust fixture it opens no file
        // and claims nothing, which without this marker renders identically to
        // a tool that read the code and correctly kept it. §6.20: "no data"
        // must be a distinct state from "zero executions".
        let text = render_text(
            &unread_suite(Ecosystem::Rust),
            &catalogue(),
            &SutChoice::Vulture,
            None,
        );

        assert!(
            text.contains("[NOT READ by this SUT]"),
            "an unread class must be marked; got {text}"
        );
        assert!(
            text.contains("not measured: 1 of 1 classes"),
            "the summary must carry the denominator, or '4 of 19' is the reading people take; \
             got {text}"
        );
        // And it must not appear in either verdict column. This is the whole
        // arithmetic of the feature: a skipped class that counted as passed
        // would make narrowing an adapter's languages a way to raise a green.
        assert!(
            text.contains("0 graded — 0 passed, 0 failed; 1 not read"),
            "the summary folded an unread class into a verdict column; got {text}"
        );
    }

    #[test]
    fn a_class_the_sut_read_carries_no_such_marker() {
        // The other half, and the one that keeps the marker meaningful: if it
        // appeared on rows the tool genuinely analyzed, it would stop carrying
        // information and start being noise a reader learns to skip.
        let text = render_text(
            &suite(Vec::new(), 1, 1),
            &catalogue(),
            &SutChoice::Vulture,
            None,
        );
        assert!(!text.contains("NOT READ"), "got {text}");
        assert!(!text.contains("not measured"), "got {text}");
        assert!(
            text.contains("1 graded — 1 passed, 0 failed; 0 not read"),
            "got {text}"
        );
    }

    #[test]
    fn every_named_analyzer_declares_the_languages_its_tool_can_load() {
        // The map that decides what gets skipped. It lives on the adapters now,
        // not here — one copy, next to the measurements that justify it — and
        // this pins that the CLI wires each SUT to its own adapter's
        // declaration rather than to a second list that can disagree.
        //
        // Both directions of error are damaging. Too wide and the analyzer is
        // handed a repository it cannot open, which is the abort this feature
        // exists to prevent; too narrow and a class it really does read is
        // dropped from the measurement, which turns an uncounted false removal
        // into a green.
        let expected: &[(SutChoice, &[Ecosystem])] = &[
            (SutChoice::Vulture, &[Ecosystem::Python]),
            (SutChoice::Knip, &[Ecosystem::TypeScript]),
            (SutChoice::Deadcode, &[Ecosystem::Go]),
            (SutChoice::Shear, &[Ecosystem::Rust]),
        ];

        for (choice, langs) in expected {
            let sut = build_sut(choice);
            assert_eq!(
                sut.reads(),
                Some(*langs),
                "`--sut {}` reads the wrong set of languages",
                choice.label()
            );
            assert!(
                !langs.contains(&Ecosystem::Polyglot),
                "`--sut {}` claims to read `Polyglot`. That is a property of a class's \
                 liveness mechanism, not a toolchain any analyzer can be pointed at — a \
                 fixture says which languages are actually in it, and matching on Polyglot \
                 hands the tool repositories with none of them (measured: knip exits 2 on \
                 m08, m13 and m18)",
                choice.label()
            );
        }

        // And the ones that declare nothing keep declaring nothing. Both
        // controls are language-agnostic by construction, and an arbitrary
        // command has unknown competence — which is not a claim in either
        // direction, so it is measured on everything.
        for choice in [
            SutChoice::Naive,
            SutChoice::Refusing,
            SutChoice::Command(vec!["mytool".to_string()]),
        ] {
            assert_eq!(
                build_sut(&choice).reads(),
                None,
                "got a language claim for {choice:?}"
            );
        }
    }

    #[test]
    fn a_report_that_graded_nothing_is_refused_rather_than_gated() {
        // The abuse case, at the surface that produces the exit code. An
        // analyzer declaring it reads no ecosystem present in any class grades
        // none of them, removes nothing live, and would otherwise print
        // "false removals: 0 — GATE PASSED" and exit 0 — a green build
        // certifying a tool that never opened a file (§6.20, §3.7).
        let nothing_graded = unread_suite(Ecosystem::Rust);
        assert_eq!(nothing_graded.false_removal_count, 0);

        let refusal = gate(&nothing_graded)
            .expect_err("a suite that graded nothing must not produce an exit code");
        assert!(
            refusal.headline.contains("graded none"),
            "the refusal must say what is missing: {}",
            refusal.headline
        );
        assert!(
            refusal.detail.contains("nothing was measured"),
            "the refusal must name the reason the zero is not a result: {}",
            refusal.detail
        );

        // And the rendering must not contain the words a gate result is made
        // of, in either direction.
        let rendered = render_refusal(&refusal, &SutChoice::Vulture, false);
        for forbidden in ["GATE PASSED", "GATE FAILED", "false removals:"] {
            assert!(
                !rendered.contains(forbidden),
                "a refusal printed `{forbidden}`: {rendered}"
            );
        }

        // The other side of the guard: one graded class is enough to gate on,
        // and the gate is still false removals and nothing else.
        assert_eq!(gate(&suite(Vec::new(), 1, 1)).ok(), Some(0));
        assert_eq!(
            gate(&suite(vec!["live.py".to_string()], 1, 1)).ok(),
            Some(1)
        );
    }

    #[test]
    fn the_declared_completed_exit_codes_are_the_measured_ones() {
        // Measured tables live on the constants; this pins the values so that a
        // later edit has to go and change the documented measurement too.
        //
        // The direction of each mistake is the reason this is a test rather
        // than a comment. A set that is too narrow refuses every productive run
        // — knip and cargo-shear both report findings *by* exiting 1. A set
        // that is too wide grades a crash as a clean scan: deadcode returns 1
        // for "this is not a Go module" and for "your Go does not compile"
        // alike, with empty stdout both times.
        assert_eq!(VULTURE_COMPLETED_EXIT_CODES, [0, 3]);
        assert_eq!(KNIP_COMPLETED_EXIT_CODES, [0, 1]);
        assert_eq!(DEADCODE_COMPLETED_EXIT_CODES, [0]);
        assert_eq!(SHEAR_COMPLETED_EXIT_CODES, [0, 1]);

        // knip is the one tool that states its own health bit, so the CLI's
        // copy must not disagree with it.
        assert_eq!(
            KNIP_COMPLETED_EXIT_CODES.as_slice(),
            knip::SUCCESS_EXIT_CODES,
            "the CLI and the knip adapter disagree about which exits are healthy"
        );

        // No named analyzer may declare 2 healthy. Every one of these tools
        // uses it for "I could not run here at all" — no package.json, no
        // Cargo.toml, an unparseable command line — and every one of those
        // states has an empty or non-report stdout that parses to no claims,
        // which is zero false removals, which is a passing gate.
        for codes in [
            VULTURE_COMPLETED_EXIT_CODES.as_slice(),
            KNIP_COMPLETED_EXIT_CODES.as_slice(),
            DEADCODE_COMPLETED_EXIT_CODES.as_slice(),
            SHEAR_COMPLETED_EXIT_CODES.as_slice(),
        ] {
            assert!(!codes.contains(&2), "exit 2 declared healthy in {codes:?}");
        }
    }

    #[test]
    fn every_named_analyzer_discloses_an_envelope_and_a_mapping() {
        // §9.2's second non-SARIF clause. A SUT wired without a disclosure
        // publishes a number with nothing bounding it, and the omission is
        // invisible in the report — there is simply one less paragraph.
        for choice in [
            SutChoice::Vulture,
            SutChoice::Knip,
            SutChoice::Deadcode,
            SutChoice::Shear,
        ] {
            let disclosure = disclosure(&choice)
                .unwrap_or_else(|| panic!("{} discloses nothing", choice.label()));
            assert!(
                !disclosure.envelope.trim().is_empty(),
                "{} has an empty capability envelope",
                choice.label()
            );
            assert!(
                !disclosure.mapping.trim().is_empty(),
                "{} has an empty mapping decision",
                choice.label()
            );
        }
    }

    #[test]
    fn a_clean_gate_with_no_decoy_recall_says_so_rather_than_reading_as_success() {
        // The hole the gate deliberately leaves. A report that printed only
        // "GATE PASSED" here would be endorsing a tool that has never called
        // anything dead.
        let text = render_text(
            &suite(Vec::new(), 0, 3),
            &catalogue(),
            &SutChoice::Refusing,
            None,
        );

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
            &SutChoice::Naive,
            None,
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
            &SutChoice::Naive,
            None,
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

    // -----------------------------------------------------------------------
    // The veto, and the trade it makes
    // -----------------------------------------------------------------------

    /// The whole point of `--veto` is a **trade**, and a report that printed
    /// only the prevented column would be selling the veto rather than
    /// measuring it (§9.13: never present what flatters the tool).
    ///
    /// Both numbers, at column zero, where CI shows them.
    #[test]
    fn a_veto_report_prints_what_it_cost_beside_what_it_prevented() {
        let bare = suite(vec!["ledger/dunning.py".to_string()], 2, 2);
        let gated = suite(Vec::new(), 1, 2);
        let runs = [veto_run(
            3,
            vec![
                blocked_path("ledger/dunning.py", "dunning", "ledger/apps.yaml"),
                blocked_path("ledger/old_report.py", "old_report", "docs/CHANGELOG.md"),
            ],
        )];
        let summary = summary(&bare, &gated, &runs);

        let text = render_text(&gated, &catalogue(), &SutChoice::Naive, Some(&summary));

        let prevented = text
            .lines()
            .find(|line| line.starts_with("veto prevented: "))
            .unwrap_or_else(|| panic!("no `veto prevented:` line at column zero; got {text}"));
        let cost = text
            .lines()
            .find(|line| line.starts_with("veto cost: "))
            .unwrap_or_else(|| panic!("no `veto cost:` line at column zero; got {text}"));

        assert!(prevented.contains('1'), "got {prevented}");
        assert!(
            cost.contains("1 decoy(s) lost"),
            "the price has to be a number on its own line, not a footnote; got {cost}"
        );
        assert!(
            text.find("veto prevented: ").unwrap() < text.find("veto cost: ").unwrap(),
            "the two lines are adjacent and in this order, so neither can be \
             read without the other"
        );
    }

    /// §9.13 asks for a conflict list rather than a score, and §7.3 records that
    /// IntelliJ Safe Delete — the best-validated prior art in the document —
    /// shows the *usage list*. A blocked claim has to say which needle fired and
    /// in which file.
    #[test]
    fn a_blocked_claim_names_its_needle_and_the_file_it_fired_in() {
        let bare = suite(vec!["ledger/dunning.py".to_string()], 1, 1);
        let gated = suite(Vec::new(), 1, 1);
        let runs = [veto_run(
            1,
            vec![blocked_path(
                "ledger/dunning.py",
                "dunning",
                "ledger/apps.yaml",
            )],
        )];
        let summary = summary(&bare, &gated, &runs);

        let text = render_text(&gated, &catalogue(), &SutChoice::Naive, Some(&summary));
        assert!(text.contains("ledger/apps.yaml"), "got {text}");
        assert!(text.contains("\"dunning\""), "got {text}");

        let json: Value = serde_json::from_str(&render_json(
            &gated,
            &catalogue(),
            &SutChoice::Naive,
            Some(&summary),
        ))
        .expect("the report is JSON");
        let record = &json["mutants"][0]["veto"]["blocked_claims"][0];
        assert_eq!(record["claim"], "ledger/dunning.py");
        assert_eq!(record["needle"], "dunning");
        assert_eq!(record["needle_kind"], "stem");
        assert_eq!(record["found_in"], "ledger/apps.yaml");
        assert_eq!(record["gate"], "literal");
        assert_eq!(record["kind"], "path");
    }

    /// **A rescue has to be checkable as cross-file at a glance.**
    ///
    /// `found_in` alone cannot be: a symbol rescued by a genuine reference from
    /// another module and a symbol "rescued" by its own declaration print the
    /// same shape, a plausible file name beside a plausible needle. That is not
    /// a cosmetic gap. A Gate 2a with nothing to exclude rescues *every* symbol
    /// claim — vulture 11 of 16 decoys to 0, deadcode 2 of 2 to 0, both landing
    /// on "GATE PASSED" by claiming nothing — and this report was the surface
    /// where that should have been obvious and was invisible.
    ///
    /// So the declaration site is emitted beside the evidence, and a reader
    /// compares two fields.
    #[test]
    fn a_blocked_symbol_shows_its_declaration_site_beside_the_evidence() {
        let bare = suite(vec!["RATES".to_string(), "dump_invoices".to_string()], 1, 1);
        let gated = suite(Vec::new(), 1, 1);
        let runs = [veto_run(
            2,
            vec![
                blocked_symbol(
                    "RATES",
                    Some("ledger/unused_currency_table.py"),
                    "docs/fx-runbook.md",
                ),
                // The other case, and the reason the field is nullable: the
                // analyzer named no file, so nothing was excluded and the
                // rescue may well be self-reference. A reader must be able to
                // tell that apart from the row above.
                blocked_symbol("dump_invoices", None, "ledger/legacy_invoice_dump.py"),
            ],
        )];
        let summary = summary(&bare, &gated, &runs);

        // The text rendering is where a reviewer looks first, so it carries the
        // same fact in the same line as the evidence.
        let text = render_text(&gated, &catalogue(), &SutChoice::Naive, Some(&summary));
        assert!(
            text.contains("declared in ledger/unused_currency_table.py"),
            "a blocked symbol must say where it was declared, or a self-rescue \
             reads exactly like a cross-file one; got {text}"
        );
        assert!(
            !text.contains("declared in ledger/legacy_invoice_dump.py"),
            "the analyzer named no file for dump_invoices, and the report must \
             not borrow the found_in file to fill the gap; got {text}"
        );

        let json: Value = serde_json::from_str(&render_json(
            &gated,
            &catalogue(),
            &SutChoice::Naive,
            Some(&summary),
        ))
        .expect("the report is JSON");

        let cross_file = &json["mutants"][0]["veto"]["blocked_claims"][0];
        assert_eq!(cross_file["claim"], "RATES");
        assert_eq!(cross_file["kind"], "symbol");
        assert_eq!(cross_file["declared_in"], "ledger/unused_currency_table.py");
        assert_eq!(cross_file["found_in"], "docs/fx-runbook.md");
        assert_ne!(
            cross_file["declared_in"], cross_file["found_in"],
            "this is the whole property: the two fields differ, so the rescue \
             is a real cross-file reference"
        );

        let unattributed = &json["mutants"][0]["veto"]["blocked_claims"][1];
        assert_eq!(unattributed["claim"], "dump_invoices");
        assert_eq!(
            unattributed["declared_in"],
            Value::Null,
            "null, not absent and not the found_in file: the analyzer said \
             nothing, so nothing was excluded, and a reader must not be able to \
             mistake this for a checked cross-file rescue"
        );
        assert_eq!(unattributed["found_in"], "ledger/legacy_invoice_dump.py");
    }

    /// A gated run publishes a smaller false-removal count than the same
    /// analyzer bare. Two runs reported under the same name would be compared as
    /// if the analyzer had improved, so the name says what was measured.
    #[test]
    fn the_measured_system_is_named_as_the_pair_it_is() {
        let bare = suite(vec!["x".to_string()], 1, 1);
        let gated = suite(Vec::new(), 1, 1);
        let runs = [veto_run(1, vec![blocked_path("x", "x", "y")])];
        let summary = summary(&bare, &gated, &runs);

        let text = render_text(&gated, &catalogue(), &SutChoice::Vulture, Some(&summary));
        assert!(text.contains("SUT `vulture+veto`"), "got {text}");

        let json: Value = serde_json::from_str(&render_json(
            &gated,
            &catalogue(),
            &SutChoice::Vulture,
            Some(&summary),
        ))
        .expect("the report is JSON");
        assert_eq!(json["sut"], "vulture+veto");
        assert_eq!(json["veto"]["false_removals_prevented"], 1);
        assert_eq!(json["veto"]["false_removals_bare"], 1);
        assert_eq!(json["veto"]["false_removals_remaining"], 0);

        // And a bare report carries no veto key at all, so a consumer tells the
        // two apart by presence rather than by parsing a name.
        let bare_json: Value =
            serde_json::from_str(&render_json(&bare, &catalogue(), &SutChoice::Vulture, None))
                .expect("the report is JSON");
        assert_eq!(bare_json["sut"], "vulture");
        assert!(bare_json["veto"].is_null(), "got {bare_json}");
    }

    /// A veto-gated run that cleared the gate by claiming nothing has not
    /// answered §11 R1's question, and the report says so rather than printing
    /// GATE PASSED and stopping.
    #[test]
    fn a_gated_run_that_found_no_decoy_at_all_is_told_it_scored_like_a_refusal() {
        let bare = suite(vec!["x".to_string()], 2, 2);
        let gated = suite(Vec::new(), 0, 2);
        let runs = [veto_run(3, vec![blocked_path("x", "x", "y")])];
        let summary = summary(&bare, &gated, &runs);

        let text = render_text(&gated, &catalogue(), &SutChoice::Deadcode, Some(&summary));
        assert!(text.contains("GATE PASSED"), "got {text}");
        assert!(
            text.contains("the way a tool that refuses to answer reaches it"),
            "a combination that rescued every decoy is safe and useless, and the \
             report has to say which; got {text}"
        );
    }

    /// **The invariant, at the moment a report is about to be published.**
    ///
    /// Vetoing can only ever remove claims. A gated run that removed a live
    /// artifact the bare run kept, or found a decoy the bare run missed, means
    /// the layer nominated rather than rescued — and there is no rendering of
    /// that which is safe to print, so it is a refusal rather than a report.
    #[test]
    fn a_gated_run_that_added_a_claim_is_refused_rather_than_reported() {
        let bare = suite(Vec::new(), 1, 1);
        let gated = suite(vec!["ledger/dunning.py".to_string()], 1, 1);

        let refusal = compare_runs(
            &bare,
            &gated,
            &veto_layer_of(&[veto_run(1, Vec::new())]),
            Some(GateSet::default()),
            judged_mutants::sut::DEFAULT_NEEDLES,
        )
        .expect_err("a gated run with a new false removal must not be reported");
        assert!(
            refusal.headline.contains("did not behave as a veto"),
            "got {}",
            refusal.headline
        );
        assert!(
            refusal.detail.contains("A veto may only rescue"),
            "got {}",
            refusal.detail
        );

        // The same rule in the other direction: a decoy the accuser never found
        // cannot appear because a rescue-only layer ran.
        let bare = suite(Vec::new(), 0, 2);
        let gated = suite(Vec::new(), 1, 2);
        let refusal = compare_runs(
            &bare,
            &gated,
            &veto_layer_of(&[veto_run(0, Vec::new())]),
            Some(GateSet::default()),
            judged_mutants::sut::DEFAULT_NEEDLES,
        )
        .expect_err("a gated run cannot find more decoys than the bare run");
        assert!(
            refusal.detail.contains("cannot nominate"),
            "got {}",
            refusal.detail
        );
    }

    /// **Two layers, and every rescue attributed to the one that made it.**
    ///
    /// This is the whole reason `--roots` is a separate flag from `--veto`. m10
    /// is the shape reduced here: a symbol nothing in the repository names, which
    /// only a Tier B convention can rescue, beside a path a literal search
    /// rescues the ordinary way. A report that summed the two would say "the
    /// stack prevented 2" and answer none of §11 R1's question.
    #[test]
    fn each_rescue_is_attributed_to_the_layer_that_made_it() {
        let bare = suite(
            vec![
                "reporting/apps.py".to_string(),
                "ReportingConfig".to_string(),
            ],
            1,
            1,
        );
        let rescued = suite(Vec::new(), 1, 1);

        // The root set is handed both claims and rescues the AppConfig symbol;
        // the veto is handed what survived and rescues the path.
        let layers = vec![
            Layer {
                name: ROOTS,
                config: "tiers A+B+C, 4 root(s) materialized, 0 unresolved".to_string(),
                runs: vec![roots_run(
                    2,
                    vec![rescued_by_convention(
                        "ReportingConfig",
                        judged_mutants::sut::ClaimKind::Symbol,
                        "django/appconfig",
                    )],
                )],
            },
            Layer {
                name: VETO,
                config: "gates literal, needles basename+stem".to_string(),
                runs: vec![veto_run(
                    1,
                    vec![blocked_path(
                        "reporting/apps.py",
                        "apps",
                        "billing/settings.py",
                    )],
                )],
            },
        ];

        let summary = compare_runs(
            &bare,
            &rescued,
            &layers,
            Some(GateSet::default()),
            judged_mutants::sut::DEFAULT_NEEDLES,
        )
        .expect("both layers only rescued");

        assert_eq!(summary.prevented, 2);
        assert_eq!(
            summary.unattributed, 0,
            "every prevented false removal has a layer's record naming it"
        );
        let by_layer: Vec<(&str, usize)> = summary
            .layers
            .iter()
            .map(|layer| (layer.name, layer.prevented))
            .collect();
        assert_eq!(
            by_layer,
            vec![(ROOTS, 1), (VETO, 1)],
            "one each, and the root set first: it is consulted before the veto"
        );

        let text = render_text(&rescued, &catalogue(), &SutChoice::Naive, Some(&summary));
        for expected in ["roots prevented: 1", "veto prevented: 1", "rescue cost: "] {
            assert!(
                text.lines().any(|line| line.starts_with(expected)),
                "no line starts with `{expected}`; got {text}"
            );
        }
        // And the per-class line names the layer, because "rescued" alone loses
        // the only thing this measurement is for.
        assert!(
            text.contains("roots rescued live: ReportingConfig"),
            "got {text}"
        );

        let json: Value = serde_json::from_str(&render_json(
            &rescued,
            &catalogue(),
            &SutChoice::Naive,
            Some(&summary),
        ))
        .expect("the report is JSON");
        assert_eq!(json["sut"], "naive+roots+veto");
        assert_eq!(json["roots"]["false_removals_prevented"], 1);
        assert_eq!(json["veto"]["false_removals_prevented"], 1);
        assert_eq!(json["rescue"]["false_removals_prevented"], 2);
        let record = &json["mutants"][0]["roots"]["rescued_claims"][0];
        assert_eq!(record["layer"], ROOTS);
        assert_eq!(record["tier"], "B");
        assert_eq!(record["rule"], "django/appconfig");
        assert_eq!(
            record["origin"],
            "pyproject.toml#django in [project] dependencies"
        );
        // The veto's own list holds only the veto's record. A merged list is how
        // a guess about a framework ends up counted as a reference somebody
        // found.
        let blocked = json["mutants"][0]["veto"]["blocked_claims"]
            .as_array()
            .expect("the veto's list is present");
        assert_eq!(blocked.len(), 1);
        assert_eq!(blocked[0]["claim"], "reporting/apps.py");
    }

    /// The layers have to compose, and a report may not assume they did.
    ///
    /// What one layer passes through is what the next is handed. If those
    /// disagree the two layers did not see the same claim set, and every
    /// per-layer number below is measured against a denominator that does not
    /// exist.
    #[test]
    fn layers_that_did_not_see_the_same_claim_set_are_refused() {
        let bare = suite(vec!["x".to_string()], 1, 1);
        let rescued = suite(Vec::new(), 1, 1);

        let layers = vec![
            Layer {
                name: ROOTS,
                config: "tiers A+B+C".to_string(),
                // Rescues one of two, so it hands one claim through.
                runs: vec![roots_run(
                    2,
                    vec![rescued_by_convention(
                        "ReportingConfig",
                        judged_mutants::sut::ClaimKind::Symbol,
                        "django/appconfig",
                    )],
                )],
            },
            Layer {
                name: VETO,
                config: "gates literal".to_string(),
                // ...and is handed two.
                runs: vec![veto_run(2, vec![])],
            },
        ];

        let refusal = compare_runs(
            &bare,
            &rescued,
            &layers,
            Some(GateSet::default()),
            judged_mutants::sut::DEFAULT_NEEDLES,
        )
        .expect_err("a stack that did not compose is not reportable");
        assert!(
            refusal.detail.contains("passed"),
            "the refusal has to name the disagreement; got {}",
            refusal.detail
        );
    }

    /// Blocked claims are attributed to classes by run order. If that ever stops
    /// holding, the evidence would be printed beside the wrong class — which is
    /// worse than printing none, because it is checkable and wrong.
    #[test]
    fn evidence_that_cannot_be_attributed_to_a_class_is_refused() {
        let bare = suite(vec!["x".to_string()], 1, 1);
        let gated = suite(Vec::new(), 1, 1);

        let refusal = compare_runs(
            &bare,
            &gated,
            // One graded class, two recorded runs.
            &veto_layer_of(&[veto_run(1, Vec::new()), veto_run(1, Vec::new())]),
            Some(GateSet::default()),
            judged_mutants::sut::DEFAULT_NEEDLES,
        )
        .expect_err("a run count that disagrees with the report is not reportable");
        assert!(
            refusal.detail.contains("cannot be attributed"),
            "got {}",
            refusal.detail
        );
    }

    /// §11 R8: the needle strategy is the biggest lever on the flag rate, so a
    /// prevented/lost pair produced under one configuration is not the pair
    /// produced under another. The report states which one it was.
    #[test]
    fn the_report_states_which_gates_and_which_needles_produced_its_numbers() {
        let bare = suite(vec!["x".to_string()], 1, 1);
        let gated = suite(Vec::new(), 1, 1);
        let runs = [veto_run(1, vec![blocked_path("x", "x", "y")])];
        let summary = summary(&bare, &gated, &runs);

        assert_eq!(summary.gates, "literal, reachability");
        assert_eq!(summary.needles, "basename+stem");

        let text = render_text(&gated, &catalogue(), &SutChoice::Naive, Some(&summary));
        assert!(text.contains("gates literal, reachability"), "got {text}");
        assert!(text.contains("needles basename+stem"), "got {text}");
    }

    /// §11 R8's **other** half: how often the gate fires.
    ///
    /// R8 records two requirements that conflict — §9.3 says block on any hit,
    /// and a usable tool needs a tolerable flag rate — and asks for a
    /// measurement rather than an argument. A flag rate is blocked over
    /// *claims judged*, and the report published only the numerator: every
    /// blocked claim, and no count of what Gate 2 was asked about. So a
    /// published fire rate could not be re-derived from `--json`, which for a
    /// swept table is the same as not having measured it.
    ///
    /// `claims_judged` is the denominator, per class, and it is the count Gate 2
    /// itself accounted for — `compare_runs` already refuses a run where
    /// `survived + blocked != claimed`, so the two cannot drift apart silently.
    #[test]
    fn a_gated_class_publishes_the_denominator_its_flag_rate_needs() {
        let bare = suite(vec!["ledger/dunning.py".to_string()], 1, 1);
        let gated = suite(Vec::new(), 1, 1);
        // Four claims reached Gate 2 and one of them was blocked.
        let runs = [veto_run(
            4,
            vec![blocked_path(
                "ledger/dunning.py",
                "dunning",
                "ledger/apps.yaml",
            )],
        )];
        let summary = summary(&bare, &gated, &runs);

        let json: Value = serde_json::from_str(&render_json(
            &gated,
            &catalogue(),
            &SutChoice::Naive,
            Some(&summary),
        ))
        .expect("the report is JSON");
        let veto = &json["mutants"][0]["veto"];
        assert_eq!(
            veto["claims_judged"],
            json!(4),
            "the denominator of the flag rate, or the rate is unpublishable; got {veto}"
        );
        assert_eq!(
            veto["blocked_claims"]
                .as_array()
                .expect("blocked_claims is a list")
                .len(),
            1,
            "and the numerator is the list already published beside it"
        );
    }

    /// A run's flag rate cannot be summed out of its per-class rows, so the run
    /// states it itself.
    ///
    /// A class where Gate 2 blocked nothing and cost nothing has no row —
    /// deliberately, because the text report would otherwise be a list of
    /// classes where nothing happened. Its claims were still judged. Summing the
    /// published `claims_judged` column therefore counts only the classes where
    /// the gate fired, which inflates every flag rate derived from it, and
    /// inflates it in the direction that flatters the gate.
    ///
    /// So the totals are emitted once, at the run level, over every claim Gate 2
    /// saw.
    #[test]
    fn the_run_states_its_own_flag_rate_because_the_class_rows_cannot_be_summed() {
        // Two classes: the gate fires on the first and is silent on the second,
        // which is the case the per-class column drops.
        let bare = two_class_suite(vec!["ledger/dunning.py".to_string()], Vec::new());
        let gated = two_class_suite(Vec::new(), Vec::new());
        let runs = [
            veto_run(
                4,
                vec![blocked_path(
                    "ledger/dunning.py",
                    "dunning",
                    "ledger/apps.yaml",
                )],
            ),
            veto_run(6, Vec::new()),
        ];
        let summary = summary(&bare, &gated, &runs);

        let json: Value = serde_json::from_str(&render_json(
            &gated,
            &catalogue(),
            &SutChoice::Naive,
            Some(&summary),
        ))
        .expect("the report is JSON");

        assert!(
            json["mutants"][1]["veto"].is_null(),
            "the silent class still has no row of its own"
        );
        let veto = &json["veto"];
        assert_eq!(
            veto["claims_judged"],
            json!(10),
            "every claim Gate 2 saw, including the six on the class with no row"
        );
        assert_eq!(
            veto["claims_blocked"],
            json!(1),
            "and the numerator beside it, so the ratio is read rather than assembled"
        );
    }
}
