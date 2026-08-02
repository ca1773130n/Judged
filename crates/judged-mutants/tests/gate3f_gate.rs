//! Gate 3f measured through the E2 suite (§9.3, §6.24, §11 R1).
//!
//! The same two properties `veto_gate.rs`, `roots_gate.rs` and
//! `coverage_gate.rs` pin, and for 3f they matter more than for any of them,
//! because §9.3 ends this conjunct with *"No ban count overrides this."* An
//! absorbing gate that silently refused everything would be invisible in every
//! downstream number while disabling the entire pipeline:
//!
//! 1. **It may only ever remove claims** — asserted on the claim *sets*, not
//!    their sizes.
//! 2. **It is not a constant function** — every refusal test says what stayed
//!    claimed.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use judged_core::gate3f::Condition;
use judged_core::Result;
use judged_mutants::fixtures;
use judged_mutants::gate3f::Gate3fSut;
use judged_mutants::mutant::GroundTruth;
use judged_mutants::sut::{Sut, SutVerdict, SymbolClaim};

/// A SUT that claims exactly what it was handed, so anything the pair does
/// differently is attributable to 3f and to nothing else.
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

struct Materialized {
    _dir: tempfile::TempDir,
    root: PathBuf,
    truth: GroundTruth,
}

fn fixture(id: &str) -> Materialized {
    let mutants = fixtures::all();
    let mutant = mutants
        .iter()
        .find(|m| m.id() == id)
        .unwrap_or_else(|| panic!("the catalogue contains {id}"));
    let dir = tempfile::Builder::new()
        .prefix("judged-gate3f-")
        .tempdir()
        .expect("scratch");
    let truth = mutant
        .materialize(dir.path())
        .unwrap_or_else(|error| panic!("{id} materializes: {error}"));
    let root = dir.path().to_path_buf();
    Materialized {
        _dir: dir,
        root,
        truth,
    }
}

/// Everything the fixture declares, claimed dead: the live artifacts 3f may
/// refuse and the decoys it must not.
///
/// Deliberately the whole ground truth rather than a plausible analyzer's
/// output. A filter is characterized by what it lets through, so the input has
/// to contain both kinds of thing.
fn claim_everything(root: &Path, truth: &GroundTruth) -> SutVerdict {
    let mut symbols: Vec<SymbolClaim> = Vec::new();
    // Each symbol is attributed to the live path whose contents actually name
    // it, which is what a real analyzer does — vulture prints `path:line:`,
    // deadcode carries a `Position`, knip carries an artifact `uri`.
    //
    // Attributing every symbol to the fixture's *first* live path instead is
    // what this helper did originally, and it is wrong in a way that looks like
    // a gate defect: m12 declares `drain` in `internal/sampler/drain.go` and
    // `TelemetryFlush` in `cmd/libtelemetry/abi.go`, so the export was judged
    // against a file that does not export it and the class read as unrefused.
    // The gate was right; the harness was lying to it.
    for name in &truth.live_symbols {
        let declaring = truth
            .live_paths
            .iter()
            .find(|path| {
                std::fs::read_to_string(root.join(path))
                    .is_ok_and(|text| text.contains(name.as_str()))
            })
            .or_else(|| truth.live_paths.first());
        if let Some(file) = declaring {
            symbols.push(SymbolClaim::declared_in(name, file));
        }
    }
    for (index, decoy) in truth.decoy_dead_paths.iter().enumerate() {
        if let Some(symbol) = truth.decoy_dead_symbols.get(index) {
            if !symbol.is_empty() {
                symbols.push(SymbolClaim::declared_in(symbol, decoy));
            }
        }
    }
    SutVerdict {
        claimed_dead_paths: truth
            .live_paths
            .iter()
            .chain(truth.decoy_dead_paths.iter())
            .cloned()
            .collect(),
        claimed_dead_symbols: symbols,
    }
}

fn run(fixture: &Materialized) -> (Gate3fSut, SutVerdict, SutVerdict) {
    let before = claim_everything(&fixture.root, &fixture.truth);
    let layer = Gate3fSut::new(Box::new(FixedSut {
        claims: before.clone(),
    }));
    let after = layer.run(&fixture.root).expect("3f runs");
    (layer, before, after)
}

fn names(verdict: &SutVerdict) -> BTreeSet<String> {
    verdict
        .claimed_dead_paths
        .iter()
        .map(|p| p.display().to_string())
        .chain(
            verdict
                .claimed_dead_symbols
                .iter()
                .map(|s| s.name().to_string()),
        )
        .collect()
}

/// Property 1, across every class in the catalogue rather than one.
#[test]
fn gate_3f_can_only_ever_remove_claims_on_every_class() {
    for mutant in fixtures::all() {
        let materialized = fixture(mutant.id());
        let (_, before, after) = run(&materialized);
        assert!(
            names(&after).is_subset(&names(&before)),
            "{}: 3f invented a claim",
            mutant.id()
        );
    }
}

/// Property 2, stated as the number that would prove the gate broken.
///
/// If 3f refused every claim in every class it would show as zero false removals
/// everywhere — the number §11 R1 reads — while having disabled the pipeline. So
/// the catalogue must contain claims it leaves alone, and most of them must be
/// left alone.
#[test]
fn gate_3f_leaves_most_of_the_catalogue_alone() {
    let mut claimed = 0usize;
    let mut refused = 0usize;
    for mutant in fixtures::all() {
        let materialized = fixture(mutant.id());
        let (layer, _, _) = run(&materialized);
        let run = &layer.runs()[0];
        claimed += run.claimed;
        refused += run.refused.len();
    }

    assert!(
        refused > 0,
        "3f fired on nothing at all: it is not wired in"
    );
    assert!(
        refused * 2 < claimed,
        "3f refused {refused} of {claimed} claims. An absorbing gate that refuses most \
         of what it sees reports zero false removals by refusing to have an opinion, \
         which is the shape §3.7 calls a control that always passes."
    );
}

/// The ABI condition, on the two classes §6.24 describes and §7 item 4 names.
///
/// m12's `//export TelemetryFlush` and m19's `#[no_mangle]` are the shapes
/// *"already-linked consumers that were never rebuilt"* refers to, and both were
/// predicted to refuse before this was run.
#[test]
fn an_abi_export_is_refused_and_its_decoys_are_not() {
    for (id, exported) in [("m12", "TelemetryFlush"), ("m19", "ledger_amortize")] {
        let materialized = fixture(id);
        let (layer, _, after) = run(&materialized);
        let run = &layer.runs()[0];

        assert!(
            run.refused
                .iter()
                .any(|r| r.claim == exported && r.conditions.contains(&Condition::AbiExport)),
            "{id}: {exported} is exported across an ABI boundary and must be refused; \
             refused {:?}",
            run.refused.iter().map(|r| &r.claim).collect::<Vec<_>>()
        );

        // The mirror: the decoys are still claimed, so the gate has not simply
        // swallowed the class.
        let survivors = names(&after);
        for decoy in &materialized.truth.decoy_dead_paths {
            assert!(
                survivors.contains(&decoy.display().to_string()),
                "{id}: 3f refused the decoy {}, which is genuinely dead",
                decoy.display()
            );
        }
    }
}

/// m16 is §6.24's own worked example: *"the class definition is the schema for
/// data already written to disk."*
#[test]
fn a_type_whose_only_consumer_is_a_pickled_blob_is_refused() {
    let materialized = fixture("m16");
    let (layer, _, _) = run(&materialized);

    assert!(layer.runs()[0]
        .refused
        .iter()
        .any(|r| r.conditions.contains(&Condition::Serializable)));
}

/// **m11 is not refused, and that is a finding rather than a defect.**
///
/// §7 of the R1 determination expected 3f to *"speak to m11"*. Implemented from
/// §6.24's own enumerated counter-signals, it does not, and the reason is worth
/// keeping: m11's serializer walks `type(model).model_fields` by hand. There is
/// no `pickle`, no `serialVersionUID`, no `#[derive(Deserialize)]`, no
/// `__getstate__`, and no `.proto`/`.avsc`/`.graphql` schema file — none of the
/// markers §6.24 lists.
///
/// So m11 sits inside §6.24's *described* hazard, wire-format field deletion,
/// and outside every counter-signal §6.24 *enumerates* for it. That is a gap in
/// the specification rather than in this code.
///
/// This test pins the gap deliberately, so that closing it is a decision
/// somebody makes rather than a marker somebody quietly adds. Widening the list
/// to catch a reflective `model_fields` walk **because m11 failed** is tuning by
/// the determination's §5 test — the instrument would be changed to fit the
/// score — and the pre-commitment answers tuning by deleting the tier rather
/// than adjusting it. If the list should grow, it grows because §6.24 is amended
/// for reflective serializers in general, and this test is what fails to say so.
#[test]
fn m11_is_outside_every_counter_signal_6_24_enumerates() {
    let materialized = fixture("m11");
    let (layer, before, after) = run(&materialized);

    assert_eq!(
        names(&after),
        names(&before),
        "3f has nothing to say about a hand-rolled reflective serializer, and this \
         test exists to make closing that gap a deliberate act rather than a quiet one"
    );
    assert!(layer.runs()[0].refused.is_empty());
}

/// A symbol the analyzer attributed to no file is one 3f could not read, which
/// is neither a refusal nor a clearance (§6.20).
#[test]
fn an_unattributed_symbol_is_counted_as_unjudgeable_rather_than_cleared() {
    let materialized = fixture("m19");
    let layer = Gate3fSut::new(Box::new(FixedSut {
        claims: SutVerdict {
            claimed_dead_paths: Vec::new(),
            claimed_dead_symbols: vec![SymbolClaim::unattributed("ledger_amortize")],
        },
    }));
    let after = layer.run(&materialized.root).expect("3f runs");

    assert_eq!(after.claimed_dead_symbols.len(), 1, "not refused");
    assert_eq!(
        layer.runs()[0].unattributed,
        1,
        "and not silently counted as a clean judgement either"
    );
}
