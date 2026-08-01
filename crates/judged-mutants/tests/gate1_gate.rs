//! Gate 1 — the never-touch inventory — measured through the E2 suite (§9.3).
//!
//! Gate 2 and the root set both reason about **usefulness**: is this referenced,
//! was this declared an entry point. Gate 1 is the only layer that reasons about
//! the **cost of being wrong**, and §9.3 states the distinction exactly — its
//! refusals are *"justified by IRREVERSIBILITY, not uselessness"*.
//!
//! Three properties are pinned here, and they are not interchangeable:
//!
//! 1. **Gate 1 may only ever remove claims**, asserted on the claim *sets*. The
//!    same invariant `veto_gate.rs` and `roots_gate.rs` pin, for the same
//!    reason: a filter that dropped one claim and invented another keeps the
//!    count identical, and the count is what a summary line prints.
//! 2. **Gate 1 runs first, and its refusal is absorbing.** §9.3 orders the gates
//!    and says any veto is final. Ordering here is structural rather than
//!    conventional — Gate 1 wraps the analyzer directly, so a claim it refuses
//!    is never handed to Gate 2 at all — and the test asserts that by checking
//!    Gate 2's own blocked list does not mention it.
//! 3. **A refusal that cannot name its class and its evidence is a score.** §9.13
//!    invariant: show a conflict list, never a probability.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use judged_core::Result;
use judged_mutants::fixtures;
use judged_mutants::gate1::{Gate1Sut, RefusedClaim};
use judged_mutants::mutant::{GroundTruth, Mutant};
use judged_mutants::sut::{ClaimKind, Sut, SutVerdict, SymbolClaim, VetoedSut};

/// A SUT that claims exactly what it was handed, whatever repository it is
/// pointed at.
///
/// The accuser half of the pair, reduced to a constant so that anything the pair
/// does differently is attributable to Gate 1 and to nothing else.
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

fn materialize(mutant: &dyn Mutant) -> Materialized {
    let dir = tempfile::Builder::new()
        .prefix("judged-gate1-")
        .tempdir()
        .expect("create a scratch directory for the fixture");
    let truth = mutant
        .materialize(dir.path())
        .unwrap_or_else(|error| panic!("{} materializes: {error}", mutant.id()));
    let root = dir.path().to_path_buf();
    Materialized {
        _dir: dir,
        root,
        truth,
    }
}

fn fixture(id: &str) -> Materialized {
    let mutants = fixtures::all();
    let mutant = mutants
        .iter()
        .find(|m| m.id() == id)
        .unwrap_or_else(|| panic!("the catalogue contains {id}"));
    materialize(mutant.as_ref())
}

/// Everything the fixture declares, claimed dead: the live artifacts (which
/// Gate 1 may refuse) and the decoys (which it may also refuse — a decoy that is
/// a migration is still a migration).
///
/// Deliberately the *whole* ground truth rather than a plausible analyzer's
/// output. A filter is characterized by what it lets through, so the input has
/// to contain both kinds of thing.
fn claim_everything(truth: &GroundTruth, root: &Path) -> SutVerdict {
    let mut paths: Vec<PathBuf> = Vec::new();
    for path in truth.live_paths.iter().chain(truth.decoy_dead_paths.iter()) {
        paths.push(relative(path, root));
    }
    let mut symbols: Vec<SymbolClaim> = truth
        .live_symbols
        .iter()
        .map(SymbolClaim::unattributed)
        .collect();
    symbols.extend(
        truth
            .decoy_dead_symbols
            .iter()
            .filter(|symbol| !symbol.is_empty())
            .map(SymbolClaim::unattributed),
    );
    SutVerdict {
        claimed_dead_paths: paths,
        claimed_dead_symbols: symbols,
    }
}

fn relative(path: &Path, root: &Path) -> PathBuf {
    path.strip_prefix(root).unwrap_or(path).to_path_buf()
}

fn path_set(verdict: &SutVerdict) -> BTreeSet<PathBuf> {
    verdict.claimed_dead_paths.iter().cloned().collect()
}

fn symbol_set(verdict: &SutVerdict) -> BTreeSet<String> {
    verdict
        .claimed_dead_symbols
        .iter()
        .map(|claim| claim.name().to_string())
        .collect()
}

/// Run Gate 1 over a fixed claim set, and hand back both the surviving claims
/// and the record of what was refused.
fn gated(claims: SutVerdict, root: &Path) -> (SutVerdict, Gate1Sut) {
    let layer = Gate1Sut::new(Box::new(FixedSut {
        claims: claims.clone(),
    }));
    let survivors = layer
        .run(root)
        .expect("Gate 1 builds over a materialized fixture");
    (survivors, layer)
}

// ---------------------------------------------------------------------------
// The invariant
// ---------------------------------------------------------------------------

/// **The invariant.** Gate 1 may only ever remove claims.
///
/// Two assertions, and they belong in one test because either alone is passed by
/// something broken.
///
/// The subset half is the safety property. It is asserted on the sets rather
/// than on the counts because a filter that dropped one claim and invented
/// another would keep the count identical.
///
/// The strictness half is what stops the first from being vacuous. The identity
/// function satisfies a subset assertion perfectly, so a test holding only that
/// cannot tell a working layer from an absent one — which is this project's own
/// failure mode (§6.20: silence read as a clean result) committed by its own
/// test suite.
#[test]
fn gate_one_only_ever_removes_claims() {
    let mutants = fixtures::all();
    let mut removed_anywhere = 0usize;

    for mutant in &mutants {
        let fixture = materialize(mutant.as_ref());
        let claims = claim_everything(&fixture.truth, &fixture.root);
        let (survivors, _) = gated(claims.clone(), &fixture.root);

        let bare_paths = path_set(&claims);
        let gated_paths = path_set(&survivors);
        let bare_symbols = symbol_set(&claims);
        let gated_symbols = symbol_set(&survivors);

        assert!(
            gated_paths.is_subset(&bare_paths),
            "{}: Gate 1 added path claims the analyzer never made: {:?}",
            mutant.id(),
            gated_paths.difference(&bare_paths).collect::<Vec<_>>()
        );
        assert!(
            gated_symbols.is_subset(&bare_symbols),
            "{}: Gate 1 added symbol claims the analyzer never made: {:?}",
            mutant.id(),
            gated_symbols.difference(&bare_symbols).collect::<Vec<_>>()
        );

        removed_anywhere +=
            (bare_paths.len() - gated_paths.len()) + (bare_symbols.len() - gated_symbols.len());
    }

    assert!(
        removed_anywhere > 0,
        "Gate 1 refused nothing in any of the {} classes. A pass-through satisfies \
         the subset half of this test exactly, so a layer that never fires is \
         indistinguishable from one that is not wired in at all.",
        mutants.len()
    );
}

/// m13 plants `media/customer/.htaccess` and `.vscode/settings.json` as live
/// artifacts. Both are Gate 1 files by two independent readings of §9.3 — an
/// upload tree and an Apache routing contract; an editor session directory —
/// and both are exactly the shape §6.17 measured: matched by an ignore pattern,
/// un-ignored by a `!` negation, and irreplaceable either way.
///
/// The second half is what makes this a measurement rather than a demonstration:
/// the two decoys are ordinary PHP source, and Gate 1 must leave them claimed. A
/// gate that refuses everything is a constant function and measures nothing
/// (§3.7 on positive controls that always pass).
#[test]
fn gate_one_refuses_the_upload_tree_and_the_editor_state_and_leaves_the_decoys_claimed() {
    let fixture = fixture("m13");
    let claims = claim_everything(&fixture.truth, &fixture.root);
    let (survivors, layer) = gated(claims, &fixture.root);

    let surviving = path_set(&survivors);
    for refused in ["media/customer/.htaccess", ".vscode/settings.json"] {
        assert!(
            !surviving.contains(Path::new(refused)),
            "Gate 1 let {refused} through; surviving claims were {surviving:?}"
        );
    }
    for decoy in ["lib/OldShippingCalculator.php", "pub/legacy_dispatch.php"] {
        assert!(
            surviving.contains(Path::new(decoy)),
            "Gate 1 refused the decoy {decoy}, which is ordinary PHP source. A gate \
             that refuses every candidate has the same output as one that is not \
             wired in."
        );
    }

    let runs = layer.runs();
    assert_eq!(runs.len(), 1, "one call to the inner SUT, one recorded run");
}

/// §9.13, and §7.3's finding that the best-validated prior art in the research —
/// IntelliJ's Safe Delete — shows the *usage list*, not a probability. A refusal
/// that cannot say which §9.3 class fired and on what evidence is a score
/// wearing a longer name.
#[test]
fn every_refusal_names_its_class_and_its_evidence() {
    let mutants = fixtures::all();
    let mut seen = 0usize;

    for mutant in &mutants {
        let fixture = materialize(mutant.as_ref());
        let claims = claim_everything(&fixture.truth, &fixture.root);
        let (_, layer) = gated(claims, &fixture.root);

        for run in layer.runs() {
            for refused in &run.refused {
                seen += 1;
                assert!(
                    refused.class.starts_with('1') && refused.class.len() == 2,
                    "{}: {:?} carries {:?}, which is not a §9.3 Gate 1 class code",
                    mutant.id(),
                    refused.claim,
                    refused.class
                );
                assert!(
                    !refused.detail.trim().is_empty(),
                    "{}: {:?} was refused with an empty reason",
                    mutant.id(),
                    refused.claim
                );
                assert!(
                    matches!(refused.kind, ClaimKind::Path | ClaimKind::Symbol),
                    "{}: {:?} was refused without saying what kind of claim it was",
                    mutant.id(),
                    refused.claim
                );
            }
        }
    }

    assert!(
        seen > 0,
        "no refusal was recorded anywhere in the catalogue, so this test asserted \
         nothing at all"
    );
}

/// **§9.3's ordering, and the absorbing property, in one assertion.**
///
/// Gate 1 runs before the reference veto. That is not a convention here, it is
/// the composition: Gate 1 wraps the analyzer and Gate 2 wraps Gate 1, so a
/// claim Gate 1 refused is never handed to Gate 2 at all.
///
/// Which is exactly what makes the refusal absorbing, and it is testable without
/// a mutable "overridden" flag anywhere: if Gate 2 never saw the claim, no
/// evidence Gate 2 could have found — a reference, a recent commit, a CI
/// manifest — can put it back. The assertion is therefore that Gate 1's refusals
/// appear in neither the final claim set nor Gate 2's blocked list, because Gate
/// 2 was never asked about them.
#[test]
fn a_gate_one_refusal_is_absorbing_because_gate_two_is_never_asked() {
    let fixture = fixture("m13");
    let claims = claim_everything(&fixture.truth, &fixture.root);

    // The composition IS the ordering. `Rc` because both ends have to be
    // interrogated afterwards, and a layer that took ownership of the one below
    // it would leave the inner record unreachable — the same reason
    // `mutants_cmd` holds each layer in one.
    let inner = Rc::new(Gate1Sut::new(Box::new(FixedSut {
        claims: claims.clone(),
    })));
    let outer = VetoedSut::new(Box::new(Rc::clone(&inner)));
    let survivors = outer.run(&fixture.root).expect("the stack runs");

    let refused: BTreeSet<String> = inner
        .runs()
        .iter()
        .flat_map(|run| run.refused.iter().map(|r| r.claim.clone()))
        .collect();
    assert!(
        !refused.is_empty(),
        "Gate 1 refused nothing on m13, so the ordering assertion below is vacuous"
    );

    let surviving: BTreeSet<String> = survivors
        .claimed_dead_paths
        .iter()
        .map(|p| p.display().to_string())
        .chain(
            survivors
                .claimed_dead_symbols
                .iter()
                .map(|s| s.name().to_string()),
        )
        .collect();
    let blocked_by_gate_two: BTreeSet<String> = outer
        .runs()
        .iter()
        .flat_map(|run| run.blocked.iter().map(|b| b.claim.clone()))
        .collect();

    for claim in &refused {
        assert!(
            !surviving.contains(claim),
            "{claim} was refused by Gate 1 and is still claimed dead after the whole \
             stack ran. A Gate 1 refusal is absorbing (§9.3)."
        );
        assert!(
            !blocked_by_gate_two.contains(claim),
            "{claim} was refused by Gate 1 and Gate 2 was asked about it anyway. \
             Gate 1 runs FIRST (§9.3); a claim it refused must never reach a later \
             gate, or the later gate's conflict list double-counts a rescue that \
             was already final."
        );
    }

    // The mirror. Gate 2 has to still have work to do, or this test would pass
    // just as well against a Gate 1 that refused the entire claim set.
    assert!(
        !blocked_by_gate_two.is_empty() || !surviving.is_empty(),
        "Gate 1 consumed the whole claim set, so nothing here shows that Gate 2 ran \
         at all"
    );
}

/// A refusal carries the evidence a reader needs to check it *by hand*, and it
/// carries **every** class that objected rather than whichever one the
/// evaluation order reached first.
///
/// m13's `.vscode/settings.json` is the case that makes the difference visible.
/// Three §9.3 clauses cover it — 1h names `.vscode/` as session state, 1j covers
/// it because GitHub Linguist's `vendor.yml` lists `(^|/)\.vscode/`, and 1m
/// because m13's `.gitignore` re-includes it with a `!` negation — and the
/// verdict reports 1j and 1m. Not 1h: §9.12 makes the vendored/generated
/// classification a hard exclusion that runs before the other content classes,
/// so 1j answers first and 1h never gets asked. That ordering is a property of
/// the design worth pinning, because "which class fired" is what a human acts
/// on.
///
/// The 1m half is the load-bearing one. §6.17 measured 246 negation patterns
/// across 41 canonical templates, and a negation is the repository *explicitly*
/// saying it wants a file that its own ignore rules would otherwise discard. The
/// refusal has to quote the pattern and the line, or a reader cannot check it.
#[test]
fn a_refusal_names_every_class_that_objected_and_quotes_each_rule() {
    let fixture = fixture("m13");
    let claims = claim_everything(&fixture.truth, &fixture.root);
    let (_, layer) = gated(claims, &fixture.root);

    let runs = layer.runs();
    let refused: Vec<&RefusedClaim> = runs.iter().flat_map(|run| run.refused.iter()).collect();
    let editor = refused
        .iter()
        .find(|r| r.claim == ".vscode/settings.json")
        .unwrap_or_else(|| {
            panic!(
                "Gate 1 did not refuse .vscode/settings.json; it refused {:?}",
                refused.iter().map(|r| &r.claim).collect::<Vec<_>>()
            )
        });

    let classes: Vec<&str> = editor.findings.iter().map(|f| f.class).collect();
    assert_eq!(
        classes,
        vec!["1j", "1m"],
        "the conflict list is wrong. §9.12 runs the vendored/generated exclusion \
         first, so 1j leads; 1m follows because m13's .gitignore un-ignores the \
         file. Reported: {}",
        editor.detail
    );
    assert_eq!(
        editor.class, "1j",
        "the headline class must be the first in §9.3 order, not an arbitrary one"
    );

    let provenance = &editor.findings[0];
    assert!(
        provenance.evidence.contains(".vscode"),
        "1j does not quote the Linguist pattern that matched: {}",
        provenance.evidence
    );
    let negation = &editor.findings[1];
    assert!(
        negation.evidence.contains("!.vscode/settings.json")
            && negation.evidence.contains(".gitignore:14"),
        "1m must quote the negation verbatim and the line it lives on, or a reader \
         cannot check it: {}",
        negation.evidence
    );
}
