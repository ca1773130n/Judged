//! Every class's coverage declaration, pinned as a table (§9.5, Family X).
//!
//! [`Mutant::coverage_declaration`] defaults to "nothing entered", which is the
//! conservative reading and also a silent one: a class that simply never
//! declared looks exactly like a class whose mechanism no test process reaches.
//! That is §6.20's pair — "had nothing to say" and "nobody asked" — and it must
//! not go unnoticed, so the whole catalogue's answers live here. A new fixture
//! fails this file until somebody states what a test suite does with it.
//!
//! The same shape `runner_capability.rs` uses for [`Mutant::languages`], for the
//! same reason.
//!
//! # The rule these answers come from
//!
//! `docs/decisions/2026-08-02-e2-coverage-artifacts.md`, fixed before any
//! artifact existed. Two consequences do most of the work in the table below,
//! and both are properties of coverage rather than of this catalogue:
//!
//! - **`FNDA` records functions.** Classes, model fields and module names have
//!   no function record however thoroughly they are exercised, so a symbol claim
//!   about one gets no coverage evidence at all. Most of this catalogue's live
//!   symbols are classes.
//! - **Import-time execution is language-specific.** A Python or JavaScript
//!   module that is merely imported has executed lines. A Rust or Go file whose
//!   functions are never entered has none, so it can only be covered by way of a
//!   call.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use judged_mutants::fixtures;

/// One row: the class, the live paths a test run loads, and the live symbols it
/// calls with the file declaring each.
struct Row {
    id: &'static str,
    covered: &'static [&'static str],
    called: &'static [(&'static str, &'static str)],
}

/// The catalogue's answers, in class order.
///
/// The per-class reasoning lives on each fixture's `coverage_declaration`, where
/// it sits beside the mechanism it was derived from. This is the index.
const DECLARATIONS: &[Row] = &[
    Row {
        id: "m01",
        covered: &["ledger/dunning.py"],
        called: &[],
    },
    Row {
        id: "m02",
        covered: &[
            "app/backends/redis_backend.py",
            "src/transports/websocketTransport.ts",
        ],
        called: &[],
    },
    Row {
        id: "m03",
        covered: &["pluginhost/plugins/tsvwriter.py"],
        called: &[],
    },
    Row {
        id: "m04",
        covered: &[],
        called: &[],
    },
    Row {
        id: "m05",
        covered: &[],
        called: &[],
    },
    Row {
        id: "m06",
        covered: &[],
        called: &[],
    },
    Row {
        id: "m07",
        covered: &["src/dot_entry.rs"],
        called: &[("src/dot_entry.rs", "is_self_or_parent_link")],
    },
    Row {
        id: "m08",
        covered: &[],
        called: &[],
    },
    Row {
        id: "m09",
        covered: &["src/badge.rs"],
        called: &[("src/badge.rs", "render_badge")],
    },
    Row {
        id: "m10",
        covered: &["reporting/apps.py", "__mocks__/redis.js"],
        called: &[],
    },
    Row {
        id: "m11",
        covered: &[],
        called: &[],
    },
    Row {
        id: "m12",
        covered: &["internal/sampler/drain.go"],
        called: &[("internal/sampler/drain.go", "drain")],
    },
    Row {
        id: "m13",
        covered: &[],
        called: &[],
    },
    Row {
        id: "m14",
        covered: &[],
        called: &[],
    },
    Row {
        id: "m15",
        covered: &[],
        called: &[],
    },
    Row {
        id: "m16",
        covered: &[],
        called: &[],
    },
    Row {
        id: "m17",
        covered: &[],
        called: &[],
    },
    Row {
        id: "m18",
        covered: &[],
        called: &[],
    },
    Row {
        id: "m19",
        covered: &[],
        called: &[],
    },
];

/// Every class declares, and declares what this table says.
#[test]
fn the_whole_catalogue_has_declared_what_a_test_suite_enters() {
    let catalogue = fixtures::all();
    assert_eq!(
        catalogue.len(),
        DECLARATIONS.len(),
        "a class was added or removed without an entry here — which would leave it \
         silently declaring nothing"
    );

    for (mutant, row) in catalogue.iter().zip(DECLARATIONS) {
        assert_eq!(mutant.id(), row.id, "the table is in catalogue order");
        let declared = mutant.coverage_declaration();

        let expected_paths: Vec<PathBuf> = row.covered.iter().map(PathBuf::from).collect();
        let expected_calls: Vec<(PathBuf, String)> = row
            .called
            .iter()
            .map(|(file, symbol)| (PathBuf::from(file), (*symbol).to_string()))
            .collect();

        // Sets, because `Declaration::calling` adds a file to `covered_paths` as
        // a side effect and the order it lands in is an implementation detail.
        let declared_paths: BTreeSet<&Path> = declared
            .covered_paths
            .iter()
            .map(PathBuf::as_path)
            .collect();
        let expected_set: BTreeSet<&Path> = expected_paths.iter().map(PathBuf::as_path).collect();
        assert_eq!(declared_paths, expected_set, "{}: covered paths", row.id);
        assert_eq!(
            declared.called_symbols, expected_calls,
            "{}: called symbols",
            row.id
        );
    }
}

/// Every declaration describes the fixture it belongs to.
///
/// The table above could agree with the fixtures and still be wrong about the
/// repositories they build — a path renamed in `materialize` and not here, a
/// symbol that is no longer live. `Declaration::check` is what catches that, and
/// running it against the **materialized** truth rather than against the table
/// is what makes it a check rather than a restatement.
#[test]
fn every_declaration_describes_the_repository_its_fixture_builds() {
    for mutant in fixtures::all() {
        let dir = tempfile::Builder::new()
            .prefix("judged-declarations-")
            .tempdir()
            .expect("scratch");
        let truth = mutant
            .materialize(dir.path())
            .unwrap_or_else(|error| panic!("{} materializes: {error}", mutant.id()));

        mutant
            .coverage_declaration()
            .check(mutant.id(), &truth)
            .unwrap_or_else(|error| panic!("{error}"));
    }
}

/// The pre-commitment, enforced.
///
/// `docs/decisions/2026-08-02-e2-coverage-artifacts.md` §5, written before any
/// artifact existed: *"If the declaration comes out as 'every live artifact is
/// entered by a test', the rule was applied wrongly. A catalogue on which
/// coverage is a perfect oracle is a catalogue that has been written to flatter
/// it."*
///
/// This is that sentence as an assertion. It is deliberately not a threshold on
/// how much coverage should reach — that would be a number to tune. It says only
/// that the catalogue must contain live artifacts an execution signal cannot
/// see, which is the property that makes the measurement worth taking.
#[test]
fn the_catalogue_is_not_a_catalogue_coverage_can_see_all_of() {
    let catalogue = fixtures::all();
    let declaring = catalogue
        .iter()
        .filter(|mutant| !mutant.coverage_declaration().is_empty())
        .count();
    let calling = catalogue
        .iter()
        .filter(|mutant| !mutant.coverage_declaration().called_symbols.is_empty())
        .count();

    assert!(
        declaring < catalogue.len(),
        "every class declares execution: the catalogue has been written to flatter \
         coverage and the run must not be published"
    );
    assert!(
        declaring > 0,
        "no class declares any execution, so the layer cannot be measured at all"
    );
    assert!(
        calling > 0 && calling < declaring,
        "the catalogue must contain both classes an execution signal reaches by a \
         CALL and classes it reaches only by an import — collapsing the two would \
         make the FNDA-versus-line distinction (§2.3) untestable here"
    );
}
