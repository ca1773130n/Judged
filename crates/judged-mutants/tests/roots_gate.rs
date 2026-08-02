//! The root set measured through the E2 suite (§5, §9.13, §11 R1).
//!
//! Gate 2 is a reference veto: it asks whether anything in the repository names
//! the candidate. Two classes in the catalogue are built so that nothing does.
//! `ReportingConfig` occurs in `reporting/apps.py` and in no other file, because
//! Django finds it by scanning that file for an `AppConfig` subclass; a Jest
//! manual mock is substituted by directory name alone. §5.1 Tier B is the only
//! layer that can rescue either — **they are not veto failures, they are root-set
//! failures.**
//!
//! What this file pins is the same property `veto_gate.rs` pins about Gate 2,
//! because it is what makes a rescue layer safe rather than merely useful:
//! **materializing roots may only ever remove claims.** It is asserted on the
//! claim *sets* rather than on their sizes — a filter that dropped one claim and
//! invented another keeps the count identical, and the count is what a summary
//! line shows.
//!
//! And the mirror property, which is what stops the first from being vacuous: a
//! root set that rescues *everything* is a constant function, and a constant
//! function measures nothing (§3.7 on positive controls that always pass). So
//! every rescue test here also asserts what stayed claimed.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use judged_core::Result;
use judged_mutants::fixtures;
use judged_mutants::mutant::{GroundTruth, Mutant};
use judged_mutants::roots::{self, GapKind, RootedSut, Tier};
use judged_mutants::sut::{ClaimKind, Sut, SutVerdict, SymbolClaim};

/// A SUT that claims exactly what it was handed, whatever repository it is
/// pointed at.
///
/// The accuser half of the pair, reduced to a constant so that anything the
/// pair does differently is attributable to the root set and to nothing else.
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
    // Not hidden, for the same reason `run_suite` does not hide it: a directory
    // whose name starts with a dot is skipped by some toolchains outright.
    let dir = tempfile::Builder::new()
        .prefix("judged-roots-")
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

/// Everything the fixture declares, claimed dead: the live artifacts (which the
/// root set may rescue) and the decoys (which it must not).
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

/// Run the root-set layer over a fixed claim set, and hand back both the
/// surviving claims and the record of what was rescued.
fn rooted(claims: SutVerdict, root: &Path) -> (SutVerdict, RootedSut) {
    let layer = RootedSut::new(Box::new(FixedSut {
        claims: claims.clone(),
    }));
    let survivors = layer
        .run(root)
        .expect("the root set materializes over a materialized fixture");
    (survivors, layer)
}

// ---------------------------------------------------------------------------
// The invariant
// ---------------------------------------------------------------------------

/// **The invariant.** The root set may only ever remove claims.
///
/// Two assertions, and they belong in one test because either alone is passed by
/// something broken.
///
/// The subset half is the safety property: a root rescues, and nothing in the
/// layer may cause a candidate to be claimed dead. It is asserted on the sets
/// rather than on the counts because a filter that dropped one claim and
/// invented another would keep the count identical.
///
/// The strictness half is what stops the first half from being vacuous. The
/// identity function satisfies a subset assertion perfectly, so a test holding
/// only that cannot tell a working layer from an absent one — which is this
/// project's own failure mode (§6.20: silence read as a clean result) committed
/// by its own test suite.
#[test]
fn materializing_roots_only_ever_removes_claims() {
    let mutants = fixtures::all();
    let mut removed_anywhere = 0usize;

    for mutant in &mutants {
        let fixture = materialize(mutant.as_ref());
        let claims = claim_everything(&fixture.truth, &fixture.root);
        let (survivors, _) = rooted(claims.clone(), &fixture.root);

        let bare_paths = path_set(&claims);
        let rooted_paths = path_set(&survivors);
        let bare_symbols = symbol_set(&claims);
        let rooted_symbols = symbol_set(&survivors);

        assert!(
            rooted_paths.is_subset(&bare_paths),
            "{}: the root set added path claims the analyzer never made: {:?}",
            mutant.id(),
            rooted_paths.difference(&bare_paths).collect::<Vec<_>>()
        );
        assert!(
            rooted_symbols.is_subset(&bare_symbols),
            "{}: the root set added symbol claims the analyzer never made: {:?}",
            mutant.id(),
            rooted_symbols.difference(&bare_symbols).collect::<Vec<_>>()
        );

        removed_anywhere +=
            (bare_paths.len() - rooted_paths.len()) + (bare_symbols.len() - rooted_symbols.len());
    }

    assert!(
        removed_anywhere > 0,
        "the root set rescued nothing in any of the {} classes. A pass-through \
         satisfies the subset half of this test exactly, so a layer that never \
         fires is indistinguishable from one that is not wired in at all.",
        mutants.len()
    );
}

/// A root that does not say which tier it came from is worse than no root: it
/// invites a caller to trust a guessed convention as though a manifest had
/// declared it (`roots::mod`). So every rescue has to carry the tier **and** a
/// file a human can open.
#[test]
fn every_rescue_names_its_tier_and_a_file_that_exists() {
    let mutants = fixtures::all();
    let mut seen = 0usize;

    for mutant in &mutants {
        let fixture = materialize(mutant.as_ref());
        let claims = claim_everything(&fixture.truth, &fixture.root);
        let (_, layer) = rooted(claims, &fixture.root);

        for run in layer.runs() {
            for rescue in &run.rescued {
                seen += 1;
                assert!(
                    matches!(rescue.tier, Tier::A | Tier::B | Tier::C),
                    "{}: {rescue:?} carries no §5.1 tier",
                    mutant.id()
                );
                assert!(
                    !rescue.rule.is_empty(),
                    "{}: {rescue:?} does not say which rule fired",
                    mutant.id()
                );
                let file = rescue
                    .origin_file
                    .as_ref()
                    .unwrap_or_else(|| panic!("{}: {rescue:?} names no origin file", mutant.id()));
                assert!(
                    fixture.root.join(file).exists(),
                    "{}: {rescue:?} points at {}, which is not in the repository — a \
                     provenance a reader cannot check is a score wearing a longer name",
                    mutant.id(),
                    file.display()
                );
            }
        }
    }

    assert!(
        seen > 0,
        "no class produced a single rescue, so this test asserted nothing"
    );
}

// ---------------------------------------------------------------------------
// The prediction, recorded so it can be wrong
// ---------------------------------------------------------------------------

/// m10 should be rescued: a Django `AppConfig` is a Tier B convention root, and
/// so is a Jest manual mock.
///
/// This is the class Gate 2 structurally cannot reach. `ReportingConfig` occurs
/// in exactly one file — its own declaration — so a literal veto that excludes
/// the declaring file has nothing left to find, however the needles are tuned.
#[test]
fn m10_is_rescued_by_tier_b_conventions_and_the_decoys_are_not() {
    let fixture = fixture("m10");
    let claims = claim_everything(&fixture.truth, &fixture.root);
    let (survivors, layer) = rooted(claims, &fixture.root);

    let paths = path_set(&survivors);
    let symbols = symbol_set(&survivors);

    for live in ["reporting/apps.py", "__mocks__/redis.js"] {
        assert!(
            !paths.contains(Path::new(live)),
            "{live} is a Tier B convention root; the root set must rescue it. \
             Survivors: {paths:?}"
        );
    }
    assert!(
        !symbols.contains("ReportingConfig"),
        "Django instantiates ReportingConfig by scanning apps.py, and the name \
         occurs nowhere else in the repository — the root set is the only layer \
         that can rescue it. Survivors: {symbols:?}"
    );

    // The other half, and it is not optional. A layer that rescued the decoys
    // too would clear this class the way a tool that refuses to answer clears
    // it.
    for decoy in ["reporting/textwrap_helper.py", "src/color_utils.js"] {
        assert!(
            paths.contains(Path::new(decoy)),
            "{decoy} is genuinely dead and no root names it; rescuing it would \
             make the layer a constant function. Survivors: {paths:?}"
        );
    }
    for decoy in ["hang_indent", "toHex"] {
        assert!(
            symbols.contains(decoy),
            "{decoy} is a genuinely-dead symbol and no root names it. \
             Survivors: {symbols:?}"
        );
    }

    // The rescue has to say *which* convention fired, not merely that one did.
    let runs = layer.runs();
    let rescued = &runs[0].rescued;
    let appconfig = rescued
        .iter()
        .find(|r| r.claim == "ReportingConfig")
        .expect("the AppConfig symbol was rescued");
    assert_eq!(appconfig.tier, Tier::B);
    assert_eq!(appconfig.kind, ClaimKind::Symbol);
    assert_eq!(
        appconfig.rule, "django/appconfig",
        "the rule is the attribution: §11 R2 wants the fire rate of each rule \
         measured rather than guessed, which is unanswerable without it"
    );
}

/// m11 should **not** be rescued, and if it ever is, that is a finding rather
/// than a success.
///
/// A reflectively-read ORM field is a reflection hazard (§6.1), not an entry
/// point. Nothing in the repository declares `tenant_slug` a root: Django is not
/// even present, and the framework that is (`pydantic`) has no convention that
/// makes a *field* an entry point. A root set that rescued these would be
/// over-firing — rescuing on a rule that does not describe the case — and the
/// cost of that is exactly the decoys it would rescue alongside.
#[test]
fn m11_reflective_fields_are_not_roots_and_the_layer_says_nothing_about_them() {
    let fixture = fixture("m11");
    let claims = claim_everything(&fixture.truth, &fixture.root);
    let (survivors, layer) = rooted(claims, &fixture.root);

    let symbols = symbol_set(&survivors);
    for field in ["tenant_slug", "retention_days", "legal_hold_until"] {
        assert!(
            symbols.contains(field),
            "{field} is read by reflection over the model's shape, which is a \
             §6.1 hazard and not an entry point. The root set materializes what \
             was DECLARED; nothing declares a pydantic field a root, so a rescue \
             here would be the layer over-firing. Survivors: {symbols:?}"
        );
    }
    assert!(
        symbols.contains("to_hex"),
        "the decoy is dead and nothing declares it either"
    );

    let runs = layer.runs();
    assert!(
        runs[0].rescued.is_empty(),
        "the root set claimed to rescue something in m11: {:?}. Every entry here \
         is a rule firing on a case it does not describe.",
        runs[0].rescued
    );
}

// ---------------------------------------------------------------------------
// What it could not resolve
// ---------------------------------------------------------------------------

/// §6.20, applied to root discovery: *"no data" must be a distinct state from
/// "zero"*. A framework detected with no plugin contributes no roots, and a
/// consumer that cannot tell that apart from "this framework has no roots" will
/// delete a convention-loaded file.
///
/// m15 declares `celery>=5.3`. Celery is recognized and not covered, so the root
/// set must report the gap rather than return a shorter list in silence.
#[test]
fn a_detected_framework_with_no_plugin_is_reported_as_a_gap() {
    let fixture = fixture("m15");
    let set = roots::materialize(&fixture.root, &[] as &[String]);

    let gap = set
        .gaps()
        .iter()
        .find(|gap| gap.kind == GapKind::FrameworkWithoutPlugin)
        .unwrap_or_else(|| {
            panic!(
                "m15 declares celery, which is recognized and uncovered; the \
                 root set reported no gap at all: {:?}",
                set.gaps()
            )
        });
    assert!(
        gap.subject.contains("celery"),
        "the gap must name the framework a reader has to go and check: {gap:?}"
    );
    assert!(
        set.files_scanned() > 0,
        "a root set reporting over a corpus of zero files has not looked (§6.20)"
    );
}

/// The other half of the same rule, and the more dangerous one: a list of roots
/// that is **short** rather than absent.
///
/// m01 loads `INSTALLED_APPS` from a YAML file at import time. The root set
/// cannot follow that, and what it must not do is return the roots it did find
/// and say nothing — because the entry it could not resolve
/// (`ledger.dunning.DunningConfig`) is precisely m01's live artifact. A caller
/// reading a short list as a complete one deletes it.
#[test]
fn a_root_list_that_could_not_be_followed_says_so_rather_than_being_short() {
    let fixture = fixture("m01");
    let set = roots::materialize(&fixture.root, &[] as &[String]);

    let gap = set
        .gaps()
        .iter()
        .find(|gap| gap.kind == GapKind::UnresolvedRootList)
        .unwrap_or_else(|| {
            panic!(
                "m01's INSTALLED_APPS is data, not a literal; the root set \
                 returned a short list in silence: {:?}",
                set.gaps()
            )
        });
    assert!(
        gap.detail.contains("INSTALLED_APPS"),
        "the gap must name the list a human has to resolve by hand: {gap:?}"
    );
}

/// The seed dump §9.13 asks for by name, checked for the one property that makes
/// it auditable: every line says which tier it came from.
#[test]
fn printseeds_labels_every_root_with_its_tier_and_prints_the_gaps() {
    let fixture = fixture("m10");
    let set = roots::materialize(&fixture.root, &[] as &[String]);
    let dump = set.printseeds();

    assert!(
        !set.roots().is_empty(),
        "m10 ships a Django app and a Jest mock; the root set found nothing: {dump}"
    );
    for root in set.roots() {
        let line = format!("{}\t{}", root.tier().label(), root.rule());
        assert!(
            dump.contains(&line),
            "every root leads with its tier and rule, so a reader can audit the \
             classification before anything acts on it; {line:?} is missing from:\n{dump}"
        );
    }
    assert!(
        dump.contains("reporting/apps.py"),
        "the Tier B AppConfig root is the whole point of this class:\n{dump}"
    );
}

// ---------------------------------------------------------------------------
// In-source Tier A roots (§5.2, determination §7 item 4)
//
// Asserted per class, at fixture level, and that shape is a lesson rather than a
// preference. Gate 3f's queue condition shipped without firing on m15 — the very
// class it was built for — because the naive SUT never claims m15's live
// artifacts, so every catalogue measurement ran straight past it and the suite
// stayed green. A rule that silently does not fire on the class it exists for is
// invisible to any measurement that never asks about that class. So each new
// root source is asked directly, about the class §5.2 names it for.
// ---------------------------------------------------------------------------

/// m12 is `//go:linkname` in fixture form, and §4.1 records that directive as
/// exactly why `x/tools/cmd/deadcode` reports a symbol *"spuriously as dead"*.
#[test]
fn go_linkname_and_export_are_roots_in_m12() {
    let fixture = fixture("m12");
    let set = roots::materialize(&fixture.root, &[] as &[String]);

    for symbol in ["drain", "TelemetryFlush"] {
        let root = set
            .rescues_symbol(symbol)
            .unwrap_or_else(|| panic!("{symbol} is declared a root by a §5.2 Go directive"));
        assert_eq!(root.tier(), Tier::A, "a directive is not a guess");
        assert!(
            root.rule().starts_with("go/"),
            "{symbol} was rescued by {} rather than by the directive that declares it",
            root.rule()
        );
    }
}

/// §2.6.4 of the R1 determination records that m18's `.pth` was *"rescued only
/// by an extension collision inside a cost gate, on one control SUT"*. It is a
/// declared root now, which is what §5.2 says it always was.
#[test]
fn the_pth_file_in_m18_is_a_declared_root_rather_than_an_accident() {
    let fixture = fixture("m18");
    let pth = "vendor/site-packages/zzz_ledger_bootstrap.pth";
    let set = roots::materialize(&fixture.root, &[pth.to_string()]);

    let root = set
        .rescues_path(pth)
        .expect("a .pth file is an entry point with no caller anywhere (§5.2)");
    assert_eq!(root.tier(), Tier::A);
    assert_eq!(root.rule(), "python/pth");

    // And the module its `import` line names, which is the other half of the
    // `site` semantics and the reason the file matters at all.
    assert!(
        set.rescues_symbol("ledger_startup_hook").is_some(),
        "the module a .pth imports is executed at interpreter start"
    );
}

/// m19's `#[no_mangle]` export, whose only consumer is outside the repository.
#[test]
fn the_no_mangle_export_in_m19_is_a_root() {
    let fixture = fixture("m19");
    let set = roots::materialize(&fixture.root, &[] as &[String]);

    let root = set
        .rescues_symbol("ledger_amortize")
        .expect("#[no_mangle] instructs the linker to emit it under that name");
    assert_eq!(root.tier(), Tier::A);
    assert_eq!(root.rule(), "rust/no-mangle");
}

/// The mirror, and the one that keeps the three above from being vacuous: the
/// new sources must not materialize roots for a class that declares none.
///
/// m05 is Python with no `.pth`, no `sitecustomize.py` and no directives.
#[test]
fn a_class_with_no_in_source_markers_gains_no_in_source_roots() {
    let fixture = fixture("m05");
    let set = roots::materialize(&fixture.root, &[] as &[String]);

    let in_source: Vec<&str> = set
        .roots()
        .iter()
        .map(roots::Root::rule)
        .filter(|rule| {
            rule.starts_with("go/") || rule.starts_with("rust/") || rule.starts_with("python/")
        })
        .collect();
    assert!(
        in_source.is_empty(),
        "m05 declares no §5.2 in-source root and must gain none: {in_source:?}"
    );
}
