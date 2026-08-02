//! Gate 2 measured through the E2 suite (§9.3, §11 R1).
//!
//! §11 R1 does not ask whether any *analyzer* clears the catalogue. It asks
//! whether any **signal combination** does, and the architecture §9.1 describes
//! is an accuser plus a veto — analyzers orchestrated as bounded accusers, never
//! as oracles. Everything the suite measured before this file was a bare
//! accuser, which is not the thing R1 is about.
//!
//! [`VetoedSut`] is that combination made gradeable: any SUT, with Gate 2 run
//! over every claim it made. The property these tests exist to pin is the one
//! that makes the layer safe rather than merely useful — **vetoing can only ever
//! remove claims** — and it is asserted on the claim *sets*, not on their sizes.
//! Counts can coincide while the sets differ, and it is the sets that carry the
//! meaning.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use judged_core::Result;
use judged_mutants::fixtures;
use judged_mutants::mutant::{GroundTruth, Mutant};
use judged_mutants::runner::run_suite;
use judged_mutants::sut::{Gate, GateSet, NaiveSut, Sut, SutVerdict, SymbolClaim, VetoedSut};

/// A SUT that claims exactly what it was handed, whatever repository it is
/// pointed at.
///
/// The accuser half of the pair, reduced to a constant so that anything the
/// pair does differently is attributable to Gate 2 and to nothing else.
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

/// A materialized fixture, and the claim set that names everything in it.
struct Materialized {
    _dir: tempfile::TempDir,
    root: PathBuf,
    truth: GroundTruth,
}

fn materialize(mutant: &dyn Mutant) -> Materialized {
    // Not hidden, for the same reason `run_suite` does not hide it: a directory
    // whose name starts with a dot is skipped by some toolchains outright.
    let dir = tempfile::Builder::new()
        .prefix("judged-veto-")
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

/// Everything the fixture declares, claimed dead: the live artifacts (which a
/// veto should rescue) and the decoys (which it should mostly not).
///
/// Deliberately the *whole* ground truth rather than a plausible analyzer's
/// output. A filter is characterized by what it lets through, so the input has
/// to contain both kinds of thing.
fn claim_everything(truth: &GroundTruth, root: &Path) -> SutVerdict {
    let mut paths: Vec<PathBuf> = Vec::new();
    for path in truth.live_paths.iter().chain(truth.decoy_dead_paths.iter()) {
        paths.push(relative(path, root));
    }
    // Unattributed, deliberately. Ground truth names symbols and not the files
    // they live in, so inventing a declaration site here would be the test
    // handing Gate 2a a fact no analyzer supplied. This claim set is the
    // pessimal one: nothing to exclude, so every rescue the gate can make it
    // will make.
    let mut symbols: Vec<SymbolClaim> = truth
        .live_symbols
        .iter()
        .map(SymbolClaim::unattributed)
        .collect();
    // `""` declares that a decoy has no symbol route at all; claiming it would
    // be claiming nothing.
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

/// `SutVerdict` promises repo-relative paths; a fixture naturally has `dir` in
/// hand and returns `dir.join(..)`.
fn relative(path: &Path, root: &Path) -> PathBuf {
    path.strip_prefix(root).unwrap_or(path).to_path_buf()
}

fn path_set(verdict: &SutVerdict) -> BTreeSet<PathBuf> {
    verdict.claimed_dead_paths.iter().cloned().collect()
}

fn symbol_set(verdict: &SutVerdict) -> BTreeSet<SymbolClaim> {
    verdict.claimed_dead_symbols.iter().cloned().collect()
}

/// **The invariant.** Gate 2 may only ever shrink a claim set.
///
/// Two assertions, and they belong in one test because either alone is passed
/// by something broken.
///
/// The subset half is the safety property: a veto rescues, and nothing in the
/// layer may cause a candidate to be claimed dead. It is asserted on the sets
/// rather than on the counts because a filter that dropped one claim and
/// invented another would keep the count identical, and the count is what a
/// summary line shows.
///
/// The strictness half is what stops the first half from being vacuous. The
/// identity function satisfies a subset assertion perfectly, so a test holding
/// only that cannot tell a working gate from an absent one — which is this
/// project's own failure mode (§6.20: silence read as a clean result) committed
/// by its own test suite.
#[test]
fn vetoing_only_ever_removes_claims() {
    let mutants = fixtures::all();
    let mut removed_anywhere = 0usize;

    for mutant in &mutants {
        let fixture = materialize(mutant.as_ref());
        let accuser = FixedSut {
            claims: claim_everything(&fixture.truth, &fixture.root),
        };

        let bare = accuser.run(&fixture.root).expect("the accuser answers");
        let gated = VetoedSut::new(Box::new(FixedSut {
            claims: bare.clone(),
        }))
        .run(&fixture.root)
        .expect("Gate 2 runs over a materialized fixture");

        let bare_paths = path_set(&bare);
        let gated_paths = path_set(&gated);
        let bare_symbols = symbol_set(&bare);
        let gated_symbols = symbol_set(&gated);

        assert!(
            gated_paths.is_subset(&bare_paths),
            "{}: Gate 2 added path claims the analyzer never made: {:?}",
            mutant.id(),
            gated_paths.difference(&bare_paths).collect::<Vec<_>>()
        );
        assert!(
            gated_symbols.is_subset(&bare_symbols),
            "{}: Gate 2 added symbol claims the analyzer never made: {:?}",
            mutant.id(),
            gated_symbols.difference(&bare_symbols).collect::<Vec<_>>()
        );

        removed_anywhere +=
            (bare_paths.len() - gated_paths.len()) + (bare_symbols.len() - gated_symbols.len());
    }

    assert!(
        removed_anywhere > 0,
        "Gate 2 removed nothing from any of the {} classes. A pass-through \
         satisfies the subset half of this test exactly, so a veto that never \
         fires is indistinguishable from one that is not wired in at all.",
        mutants.len()
    );
}

/// §9.13 asks for a conflict list rather than a score, and IntelliJ Safe Delete
/// — §7.3's best-validated prior art — shows the usage list, not a probability.
/// A blocked claim that cannot say *what* fired and *where* is a score.
#[test]
fn a_blocked_claim_names_the_needle_that_fired_and_the_file_it_fired_in() {
    let mutants = fixtures::all();
    let m01 = mutants
        .iter()
        .find(|m| m.id() == "m01")
        .expect("the catalogue contains m01");
    let fixture = materialize(m01.as_ref());

    // m01's live artifact is reachable only through a YAML string, so the
    // whole-repo literal search is exactly the signal that should rescue it.
    let live = relative(
        fixture
            .truth
            .live_paths
            .first()
            .expect("m01 declares a live path"),
        &fixture.root,
    );

    let gated = VetoedSut::new(Box::new(FixedSut {
        claims: SutVerdict {
            claimed_dead_paths: vec![live.clone()],
            claimed_dead_symbols: Vec::new(),
        },
    }));
    let verdict = gated.run(&fixture.root).expect("Gate 2 runs");

    assert!(
        verdict.claimed_dead_paths.is_empty(),
        "m01's live artifact is named by a YAML string in the same repository, \
         so Gate 2a must rescue it; it survived instead: {:?}",
        verdict.claimed_dead_paths
    );

    let runs = gated.runs();
    assert_eq!(runs.len(), 1, "one call to the inner SUT, one recorded run");
    let blocked = &runs[0].blocked;
    assert_eq!(blocked.len(), 1, "exactly the one claim that was dropped");

    let record = &blocked[0];
    assert_eq!(record.claim, live.display().to_string());
    assert_eq!(
        record.gate,
        Gate::Literal,
        "the whole-repo literal search is what fired"
    );
    let needle = record
        .needle
        .as_deref()
        .expect("a literal veto names the needle that fired");
    assert!(
        live.display().to_string().contains(needle),
        "the needle {needle:?} is derived from the claimed path {}",
        live.display()
    );
    let found_in = record
        .found_in
        .as_deref()
        .expect("a literal veto names the file the needle fired in");
    assert_ne!(
        found_in, live,
        "a file may not be evidence about itself; the corpus excludes the \
         candidate"
    );
    assert!(
        fixture.root.join(found_in).exists(),
        "{} is a real file in the fixture",
        found_in.display()
    );
}

/// The same invariant one level up, where the numbers a release is gated on are
/// produced. A veto-gated run may never have more false removals than the bare
/// run, and may never find decoys the bare run did not.
///
/// Asserted on the false-removal *sets* per class, not on the totals: two runs
/// can remove the same number of live artifacts and not the same live
/// artifacts.
#[test]
fn a_veto_gated_run_never_grades_worse_than_the_bare_run() {
    let mutants = fixtures::all();

    let bare = run_suite(&NaiveSut, &mutants).expect("the bare control runs");
    let gated_sut = VetoedSut::new(Box::new(NaiveSut));
    let gated = run_suite(&gated_sut, &mutants).expect("the gated control runs");

    assert_eq!(
        bare.reports.len(),
        gated.reports.len(),
        "both runs cover the whole catalogue"
    );

    for (before, after) in bare.reports.iter().zip(gated.reports.iter()) {
        assert_eq!(before.mutant_id, after.mutant_id, "rows stay aligned");

        let before_removals: BTreeSet<&str> =
            before.false_removals.iter().map(String::as_str).collect();
        let after_removals: BTreeSet<&str> =
            after.false_removals.iter().map(String::as_str).collect();
        assert!(
            after_removals.is_subset(&before_removals),
            "{}: the veto introduced false removals the bare run did not make: {:?}",
            before.mutant_id,
            after_removals
                .difference(&before_removals)
                .collect::<Vec<_>>()
        );

        assert!(
            after.decoys_found <= before.decoys_found,
            "{}: the veto found decoys the bare run missed ({} > {}), which \
             would mean it nominated rather than rescued",
            before.mutant_id,
            after.decoys_found,
            before.decoys_found
        );
        assert_eq!(
            before.decoys_total, after.decoys_total,
            "{}: the denominator is the fixture's, not the gate's",
            before.mutant_id
        );
    }

    assert!(
        gated.false_removal_count <= bare.false_removal_count,
        "the gated run removed more live artifacts ({}) than the bare run ({})",
        gated.false_removal_count,
        bare.false_removal_count
    );
}

/// Why Gate 2e is not in the default set, recorded as a measurement rather than
/// as an opinion.
///
/// Every file in a fixture is committed by the fixture, seconds before the
/// analyzer is spawned, so the newest commit touching any of them is always
/// inside any window. Gate 2e therefore rescues 100% of claims in 100% of
/// classes — it measures the age of the scratch directory, not the content of
/// the repository — and a suite run through it reports a veto that prevented
/// everything at the cost of everything.
#[test]
fn gate_2e_rescues_every_claim_in_a_fixture_because_the_fixture_was_committed_seconds_ago() {
    let mutants = fixtures::all();
    let m01 = mutants
        .iter()
        .find(|m| m.id() == "m01")
        .expect("the catalogue contains m01");
    let fixture = materialize(m01.as_ref());

    let claims = claim_everything(&fixture.truth, &fixture.root);
    assert!(
        !claims.claimed_dead_paths.is_empty(),
        "the fixture declares paths to claim"
    );

    let gated = VetoedSut::with_gates(
        Box::new(FixedSut {
            claims: claims.clone(),
        }),
        GateSet::ALL,
    );
    let verdict = gated.run(&fixture.root).expect("Gate 2 runs");

    assert!(
        verdict.claimed_dead_paths.is_empty(),
        "Gate 2e rescues every path in a repository committed seconds ago; \
         these survived: {:?}",
        verdict.claimed_dead_paths
    );
    assert!(
        GateSet::ALL.includes(Gate::Recency),
        "GateSet::ALL is the set that includes Gate 2e"
    );
    assert!(
        !GateSet::default().includes(Gate::Recency),
        "the default set excludes Gate 2e, because in this suite it answers \
         the same way for every claim in every class"
    );
}

/// Gate 2a is structurally mandatory: a caller must not be able to run a
/// configuration that looks like Gate 2 and searches nothing.
///
/// The same shape as `NeedleStrategy::without(Basename)` in the veto module, and
/// for the same reason.
#[test]
fn the_literal_gate_cannot_be_switched_off() {
    for set in [GateSet::LITERAL_ONLY, GateSet::CONTENT, GateSet::ALL] {
        assert!(
            set.includes(Gate::Literal),
            "{set:?} must include the whole-repo literal search"
        );
    }
    assert_eq!(
        GateSet::default(),
        GateSet::CONTENT,
        "the default is the pair of content-derived gates"
    );
}

/// A symbol claim arrives without a location — `SutVerdict` has no field for
/// one, because vulture and deadcode both report bare names. Gate 2a is still
/// run over it, with the declaration site unknown and therefore nothing
/// excluded from the corpus, which is the direction that can only add rescues.
#[test]
fn a_symbol_claim_is_judged_by_the_symbol_needle_over_the_whole_corpus() {
    let mutants = fixtures::all();
    let m01 = mutants
        .iter()
        .find(|m| m.id() == "m01")
        .expect("the catalogue contains m01");
    let fixture = materialize(m01.as_ref());

    let live_symbol = fixture
        .truth
        .live_symbols
        .first()
        .cloned()
        .expect("m01 declares a live symbol");

    let gated = VetoedSut::new(Box::new(FixedSut {
        claims: SutVerdict {
            claimed_dead_paths: Vec::new(),
            claimed_dead_symbols: vec![
                SymbolClaim::unattributed(&live_symbol),
                // A name nothing in the repository spells. The corpus really was
                // searched and really found nothing, which is the one answer
                // Gate 2a is allowed to give without rescuing.
                SymbolClaim::unattributed("ThisIdentifierIsNotInTheFixture"),
            ],
        },
    }));
    let verdict = gated.run(&fixture.root).expect("Gate 2 runs");

    assert!(
        !verdict
            .claimed_dead_symbols
            .iter()
            .any(|claim| claim.name() == live_symbol),
        "the live symbol {live_symbol} is spelled somewhere in the repository, \
         so the symbol needle fires and Gate 2a rescues it"
    );
    assert_eq!(
        verdict.claimed_dead_symbols,
        vec![SymbolClaim::unattributed("ThisIdentifierIsNotInTheFixture")],
        "a completed search that found nothing is the only thing that clears \
         Gate 2a, and it must clear it — a gate that rescues unconditionally is \
         not measuring anything"
    );
}

/// **The price of an unattributed claim, pinned so that nobody can quietly stop
/// paying it.**
///
/// A symbol that really is dead still occurs once: in its own declaration.
/// Gate 2a excludes that file when the accuser says which file it is — the test
/// above is that property — and when the accuser does not say, there is nothing
/// to exclude. So the declaration is read as a reference, the decoy is rescued,
/// and decoy recall falls.
///
/// That reasoning is right for this case and was, until provenance existed,
/// applied to every case; see [`judged_mutants::sut::SymbolClaim`]. What stays
/// true is that a claim carrying no location must land here rather than have a
/// location guessed for it.
///
/// This test exists because the tempting repair — reading the hit list and
/// deciding for oneself that a single-file hit is "only the declaration" — would
/// re-derive the evidence-to-verdict mapping outside the one function that owns
/// it, and would turn Gate 2 into something that can decline to rescue. The cost
/// is a column in the report instead.
#[test]
fn an_unattributed_decoy_symbol_is_rescued_by_its_own_declaration_and_that_is_the_documented_cost()
{
    let mutants = fixtures::all();
    let m01 = mutants
        .iter()
        .find(|m| m.id() == "m01")
        .expect("the catalogue contains m01");
    let fixture = materialize(m01.as_ref());

    let decoy = fixture
        .truth
        .decoy_dead_symbols
        .iter()
        .find(|symbol| !symbol.is_empty())
        .cloned()
        .expect("m01 plants a decoy with a symbol route");

    let gated = VetoedSut::new(Box::new(FixedSut {
        claims: SutVerdict {
            claimed_dead_paths: Vec::new(),
            claimed_dead_symbols: vec![SymbolClaim::unattributed(&decoy)],
        },
    }));
    let verdict = gated.run(&fixture.root).expect("Gate 2 runs");

    assert!(
        verdict.claimed_dead_symbols.is_empty(),
        "a genuinely-dead symbol survived Gate 2a. Nothing told Gate 2a where \
         its declaration was, so there was nothing to exclude and the \
         declaration itself had to fire: {:?}",
        verdict.claimed_dead_symbols
    );

    let runs = gated.runs();
    let record = runs[0]
        .blocked
        .first()
        .expect("the dropped claim is recorded with its evidence");
    assert_eq!(record.claim, decoy);
    assert_eq!(record.gate, Gate::Literal);
    assert_eq!(record.needle.as_deref(), Some(decoy.as_str()));
    assert_eq!(
        record.declared_in, None,
        "the analyzer named no file, and the report must not invent one: \
         `declared_in` is what tells a reader this rescue could not have \
         excluded anything"
    );
    let found_in = record
        .found_in
        .as_deref()
        .expect("the rescue names the file it read the symbol out of");
    assert!(
        fixture.root.join(found_in).exists(),
        "{} is a real file in the fixture, so the rescue is checkable",
        found_in.display()
    );
}

/// **The difference between a veto and a constant function.**
///
/// A function that returns the same answer for every input measures nothing,
/// and that is what Gate 2a over symbols was: told nothing about where a symbol
/// lives, it read every declaration as a reference to itself and rescued every
/// claim. §3.7 makes the same point about positive controls — a control that
/// always passes is theatre — and it applies to a gate exactly as it applies to
/// a control.
///
/// The pair below is the whole property, and it has to be one test because
/// either half alone is passed by something broken. A gate that never fires
/// passes the first assertion. A gate that always fires passes the second. Only
/// a gate that is actually reading the corpus passes both.
///
/// `RATES` is m01's second decoy: declared in `ledger/unused_currency_table.py`
/// and spelled in no other file the fixture plants. So with the declaration site
/// known there is genuinely nothing to find, and the only way the second half
/// can differ from the first is if Gate 2a really searched.
#[test]
fn a_symbol_declared_in_one_file_is_vetoed_only_by_a_second_file_naming_it() {
    let mutants = fixtures::all();
    let m01 = mutants
        .iter()
        .find(|m| m.id() == "m01")
        .expect("the catalogue contains m01");
    let fixture = materialize(m01.as_ref());

    let declaring = PathBuf::from("ledger/unused_currency_table.py");
    assert!(
        fixture.root.join(&declaring).exists(),
        "m01 plants {}; this test is keyed on that file and must fail loudly if \
         the fixture is renamed rather than quietly measure something else",
        declaring.display()
    );

    // Provenance, which is the whole point: the name AND the file the tool
    // attributed it to.
    let claim = SymbolClaim::declared_in("RATES", &declaring);
    let claims = SutVerdict {
        claimed_dead_paths: Vec::new(),
        claimed_dead_symbols: vec![claim.clone()],
    };

    let gated = VetoedSut::new(Box::new(FixedSut {
        claims: claims.clone(),
    }));
    let verdict = gated
        .run(&fixture.root)
        .expect("Gate 2 runs over a materialized fixture");

    assert_eq!(
        verdict.claimed_dead_symbols,
        vec![claim.clone()],
        "RATES occurs once in m01, in the file that declares it. A declaration \
         is not a reference to itself, so with the declaration site known there \
         is nothing left for Gate 2a to find — and a gate that rescues this \
         claim rescues every symbol claim ever made, which is a constant \
         function wearing a gate's name. Blocked: {:?}",
        gated.runs()[0].blocked
    );

    // The same claim, in a repository where one more file names it. Nothing
    // else changes.
    let mentions = fixture.root.join("docs/fx-runbook.md");
    std::fs::create_dir_all(mentions.parent().expect("docs/ has a parent")).expect("create docs/");
    std::fs::write(
        &mentions,
        "When a currency is added, extend the RATES table before deploying.\n",
    )
    .expect("write the second file");
    let repo = judged_core::git::Repo::discover(&fixture.root).expect("the fixture is a repo");
    repo.add_all().expect("stage the second file");
    repo.commit("mention RATES from a second file")
        .expect("commit the second file");

    let gated = VetoedSut::new(Box::new(FixedSut { claims }));
    let verdict = gated
        .run(&fixture.root)
        .expect("Gate 2 runs over a materialized fixture");

    assert!(
        verdict.claimed_dead_symbols.is_empty(),
        "a second file names RATES, which is exactly the cross-file evidence \
         Gate 2a exists to act on. A gate that does not fire here is not a gate. \
         Survived: {:?}",
        verdict.claimed_dead_symbols
    );
    let runs = gated.runs();
    let record = runs[0]
        .blocked
        .first()
        .expect("the rescued claim is recorded with its evidence");
    assert_eq!(
        record.found_in.as_deref(),
        Some(Path::new("docs/fx-runbook.md")),
        "the rescue must name the OTHER file. Naming the declaration site would \
         mean the exclusion did not happen and the pass above was luck"
    );
    assert_eq!(
        record.declared_in.as_deref(),
        Some(declaring.as_path()),
        "a reader has to be able to see at a glance that the rescue is \
         cross-file rather than self-reference, which needs both sites side by \
         side"
    );
}

// ---------------------------------------------------------------------------
// §9.5 R-family evidence (§9.4, §9.6)
//
// Gate 2a is the only thing in this build that can license a §9.5 R row, and it
// licenses exactly one: "zero textual occurrences, complete non-truncated
// search", +1.0. The qualifier is not re-checked here because the type system
// already carries it — `Verdict::Clear` IS the complete-corpus zero-hit case,
// and an incomplete search is a veto (§6.20), so a claim whose scan did not
// finish is blocked rather than cleared.
// ---------------------------------------------------------------------------

/// A survivor earns the +1.0 row; a rescued claim earns nothing.
#[test]
fn only_a_claim_that_survived_a_complete_search_earns_r_family_evidence() {
    let mutants = fixtures::all();
    let mutant = mutants
        .iter()
        .find(|m| m.id() == "m05")
        .expect("the catalogue contains m05");
    let materialized = materialize(mutant.as_ref());
    let before = claim_everything(&materialized.truth, &materialized.root);
    let layer = VetoedSut::new(Box::new(FixedSut {
        claims: before.clone(),
    }));
    let after = layer.run(&materialized.root).expect("the veto runs");
    let run = &layer.runs()[0];

    assert!(
        !run.blocked.is_empty() && run.survived > 0,
        "the fixture must exercise both sides for this test to mean anything"
    );

    for survivor in &run.complete_search_survivors {
        let evidence = run
            .evidence_for(survivor)
            .unwrap_or_else(|| panic!("{survivor} survived, so it earned the +1.0 row"));
        assert_eq!(evidence.family(), judged_core::ledger::Family::R);
        assert!((evidence.bans() - 1.0).abs() < 1e-9);
    }

    for blocked in &run.blocked {
        assert!(
            run.evidence_for(&blocked.claim).is_none(),
            "{} was rescued, so there is no accusation left to weigh",
            blocked.claim
        );
    }

    // The survivor list and the returned claim set are the same population,
    // which is what makes the evidence attributable to a claim the caller holds.
    let returned: BTreeSet<String> = after
        .claimed_dead_paths
        .iter()
        .map(|p| p.display().to_string())
        .chain(
            after
                .claimed_dead_symbols
                .iter()
                .map(|s| s.name().to_string()),
        )
        .collect();
    assert_eq!(
        run.complete_search_survivors
            .iter()
            .cloned()
            .collect::<BTreeSet<String>>(),
        returned
    );
}

/// One family accusing is not a quorum, so the evidence Gate 2a produces cannot
/// promote anything on its own — §9.5's rule, arriving here as arithmetic.
#[test]
fn gate_2a_evidence_alone_never_reaches_a_quorum() {
    let mutants = fixtures::all();
    let mutant = mutants
        .iter()
        .find(|m| m.id() == "m05")
        .expect("the catalogue contains m05");
    let materialized = materialize(mutant.as_ref());
    let layer = VetoedSut::new(Box::new(FixedSut {
        claims: claim_everything(&materialized.truth, &materialized.root),
    }));
    layer.run(&materialized.root).expect("the veto runs");
    let run = &layer.runs()[0];

    let survivor = run
        .complete_search_survivors
        .first()
        .expect("at least one claim survived");
    let mut ledger = judged_core::ledger::Ledger::new();
    ledger.record(run.evidence_for(survivor).expect("evidence"));

    assert!(
        ledger.accuses(judged_core::ledger::Family::R),
        "1.0 >= the +0.5 floor"
    );
    assert_eq!(
        ledger.accusing().len(),
        1,
        "and one family is not the two §9.5 requires"
    );
}
