//! The lcov parser against tracefiles real instrumenters actually wrote.
//!
//! Every other coverage test in this workspace uses a tracefile this project
//! authored, which means all of them share one blind spot: a misunderstanding of
//! the format is invisible, because the same misunderstanding writes the fixture
//! and reads it. The E2 fixtures will make that worse rather than better, since
//! they are generated. These two files close the gap in the one way it can be
//! closed — they were produced by running real code under real tools, and are
//! committed byte-exact.
//!
//! # Provenance
//!
//! Both were generated on 2026-08-02 from a three-function module
//! (`compute`, `handle_request`, `never_called`) driven by a caller that
//! exercises the first two and never the third:
//!
//! - `coverage-py-7.lcov.info` — Coverage.py 7.15.2 with C extension, via
//!   `coverage run -m pytest` then `coverage lcov -o lcov.info`.
//! - `c8-lcov.info` — c8 12.0.0 on Node v24.14.0, via
//!   `c8 --reporter=lcovonly node run.js`.
//!
//! Not edited afterwards, including to add a provenance header — a fixture whose
//! point is "this is what the tool wrote" stops being that the moment a line is
//! added to it. The provenance lives here instead.
//!
//! # What they caught
//!
//! The two tools disagree about the format in three ways that a single
//! hand-written fixture would have hidden, and one of them was a live risk in
//! the parser rather than a hypothetical:
//!
//! 1. **Both `FN:` dialects are real, and in the two most common tools.**
//!    Coverage.py writes `FN:<start>,<end>,<name>`; c8 writes `FN:<line>,<name>`.
//!    Supporting only the one this project would have guessed at would have lost
//!    every function record from half the ecosystem.
//! 2. **Record order differs.** Coverage.py interleaves `FN`/`FNDA` per
//!    function; c8 emits every `FN`, then every `FNDA`. Accumulating by name
//!    rather than by position is what makes both read the same.
//! 3. **The boot-only phenomenon is real and language-dependent.** See
//!    [`a_def_line_is_not_a_call_and_the_two_languages_prove_it_differently`].

use std::path::{Path, PathBuf};

use judged_core::coverage::{Control, Coverage};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data")
        .join(name)
}

fn parse(name: &str) -> Coverage {
    let path = fixture(name);
    Coverage::read(&path).unwrap_or_else(|error| panic!("{name} parses: {error}"))
}

/// Coverage.py's dialect, end to end.
#[test]
fn coverage_py_start_end_dialect_parses_with_its_line_numbers_intact() {
    let coverage = parse("coverage-py-7.lcov.info");

    let ledger = coverage
        .files()
        .iter()
        .find(|file| file.source() == Path::new("src/ledger.py"))
        .expect("the module under test has a record");

    // `FN:7,8,never_called` — the start line survives the three-field form.
    let never = ledger.function("never_called").expect("declared");
    assert_eq!(never.start_line(), Some(7));
    assert_eq!(never.calls(), 0);

    assert_eq!(ledger.function("compute").expect("declared").calls(), 1);
    assert_eq!(
        ledger.function("handle_request").expect("declared").calls(),
        1
    );
}

/// c8's dialect, which is the other one.
#[test]
fn c8_two_field_dialect_parses_even_though_every_fn_precedes_every_fnda() {
    let coverage = parse("c8-lcov.info");

    let ledger = coverage
        .files()
        .iter()
        .find(|file| file.source() == Path::new("src/ledger.js"))
        .expect("the module under test has a record");

    // `FN:9,neverCalled` twelve lines above `FNDA:0,neverCalled`. Accumulating
    // by name is what pairs them.
    let never = ledger.function("neverCalled").expect("declared");
    assert_eq!(never.start_line(), Some(9));
    assert_eq!(never.calls(), 0);
    assert_eq!(
        ledger.function("handleRequest").expect("declared").calls(),
        1
    );
}

/// §2.3, measured rather than assumed — and the measurement is more interesting
/// than the claim.
///
/// The design says a `def` line executes at import, so line coverage must never
/// be read as proof that a function ran. Both artifacts bear that out, and they
/// do it by **different routes**:
///
/// - Coverage.py reports `DA:7,1` for `never_called`'s `def` line and `DA:8,0`
///   for its body. The definition really did execute, at import, exactly as
///   §2.3 describes. A line-granularity check on line 7 would pass on a function
///   nothing ever called.
/// - c8 reports `DA:9,0` for the JavaScript declaration. A hoisted function
///   declaration is not an executed statement in its model, so the line is
///   simply uncovered.
///
/// Two languages, two mechanisms, one conclusion: `FNDA` is the only record that
/// answers "was this entered". The parser must reach the same verdict from both,
/// and that is what this pins.
#[test]
fn a_def_line_is_not_a_call_and_the_two_languages_prove_it_differently() {
    for (name, module, symbol) in [
        ("coverage-py-7.lcov.info", "src/ledger.py", "never_called"),
        ("c8-lcov.info", "src/ledger.js", "neverCalled"),
    ] {
        let coverage = parse(name);

        assert!(
            coverage.executed_file(module).is_some(),
            "{name}: the module was loaded, so a claim that the FILE is dead is dropped"
        );
        assert!(
            coverage.called_function(symbol).is_none(),
            "{name}: and a claim that {symbol} is dead is NOT, because nothing called it"
        );
    }

    // The half that is specific to Python, spelled out so a future change to the
    // line rule cannot pass by quietly agreeing with c8 alone.
    let python = parse("coverage-py-7.lcov.info");
    let ledger = python
        .files()
        .iter()
        .find(|file| file.source() == Path::new("src/ledger.py"))
        .expect("record");
    assert_eq!(
        (ledger.lines_hit(), ledger.lines_found()),
        (5, 6),
        "five of six lines executed while one of three functions was never entered — \
         the exact gap §2.3 is about"
    );
}

/// A record with nothing in it is legal, and Coverage.py emits one for every
/// empty `__init__.py`.
///
/// It has to parse as a file that was *not* executed rather than as a parse
/// error or as a file that was: an empty module is the most ordinary thing in a
/// Python package, and either wrong answer would be a rescue layer that fails
/// on the first real repository it meets.
#[test]
fn an_empty_record_is_a_file_with_no_evidence_rather_than_an_error() {
    let coverage = parse("coverage-py-7.lcov.info");

    let empty = coverage
        .files()
        .iter()
        .find(|file| file.source() == Path::new("src/__init__.py"))
        .expect("`SF:` immediately followed by `end_of_record` is still a record");
    assert_eq!(empty.lines_found(), 0);
    assert!(empty.functions().is_empty());
    assert!(
        coverage.executed_file("src/__init__.py").is_none(),
        "no evidence is not evidence of execution"
    );
}

/// The summary and branch keys both tools emit are skipped without complaint,
/// and the totals are recomputed from `DA` rather than read from `LF`/`LH`.
#[test]
fn summary_and_branch_keys_are_skipped_and_the_totals_are_recomputed() {
    let coverage = parse("c8-lcov.info");
    let ledger = coverage
        .files()
        .iter()
        .find(|file| file.source() == Path::new("src/ledger.js"))
        .expect("record");

    // The artifact's own `LF:13 LH:10`, arrived at independently. Agreement is
    // the point: it says the recomputation is right, without the parser having
    // trusted the tool's arithmetic to get there.
    assert_eq!((ledger.lines_found(), ledger.lines_hit()), (13, 10));
}

/// A control written against a real artifact, behaving as it will in a real
/// repository.
#[test]
fn a_control_passes_on_a_real_run_and_fails_on_the_function_nothing_called() {
    let coverage = parse("coverage-py-7.lcov.info");

    let good = Control::parse(
        Path::new("lcov.info.control"),
        "symbol handle_request\nsymbol compute\nmin-called-functions 3\n",
    )
    .expect("parses");
    assert!(
        good.check(&coverage).passed(),
        "three functions were entered — handle_request, compute, and the test itself"
    );

    // The mistake a real operator makes: declaring a symbol that looks always-live
    // and is not covered by this suite. It must fail loudly rather than be
    // rounded off.
    let bad =
        Control::parse(Path::new("lcov.info.control"), "symbol never_called\n").expect("parses");
    let outcome = bad.check(&coverage);
    assert!(!outcome.passed());
    assert_eq!(outcome.symbols_uncalled(), ["never_called".to_string()]);
}
