# How the E2 catalogue gets coverage — and the rule that has to be fixed first

**Date:** 2026-08-02 · **Status:** decided and implemented; measured in §6 · **Supersedes:** nothing

The X-family layer landed today: lcov ingestion, an `FNDA`-granularity positive control, and a
fourth rescue layer wired beside `--gate1`, `--veto` and `--roots`. At the time this was written it
rescued nothing, because the nineteen fixtures shipped no coverage artifacts, and `judged mutants
--sut naive --coverage` said so in the only way that is honest: **0 of 19 class(es) had an artifact
that passed its control (19 no-artifact)**.

The handoff's §2.4 named that gap and asked for a deliberate decision. This is it. §§1–5 were
written before anything was implemented and are left as they were, except where the work refuted
them — those corrections are marked, because a premise that survived contact is worth telling apart
from one that did not. §6 is what happened.

---

## 1. The trap, stated before the options

Three classes survive with false removals: m11 under vulture, m02 under knip, m12 under deadcode.
All three are live through a **runtime** mechanism — a reflective field read, a dynamic import, a
`go:linkname` alias — so a test suite that exercises any of those fixtures executes the live
artifact. Which means a coverage artifact for those three would rescue exactly the three claims the
project currently fails on. That is not a reason to write them; it is the reason to be careful about
writing them.

> **Corrected by §6.** The middle step is wrong for m11, and finding that out is most of what the
> implementation was worth. A test really does execute the reflective read — but the live artifacts
> are model *fields*, and `FNDA` records functions. There is no record for a field at any coverage
> granularity, so the class an execution signal was most plausibly going to rescue is the one it
> structurally cannot. The trap was real; it was just narrower than it looked.

The determination's §5 draws the line. Implementing more of the specification is implementation;
changing the instrument because m02, m11 or m12 failed is tuning, and the pre-commitment answers
tuning by deleting the tier rather than adjusting it. So three artifacts authored to rescue the
three failures would pass every test in this repository and tell us nothing at all, because the
input was chosen by looking at the output. The catalogue is the measuring instrument, and
everything below is about not bending it.

---

## 2. What was considered

**Runnable test suites in the fixtures.** The most honest option. Each fixture grows a real test,
the harness runs it, and coverage is produced by an actual execution — but it also drags a language
runtime into the harness for every ecosystem the catalogue touches, Python and Node and Go and Rust
and PHP, and makes a suite run depend on five toolchains being present and correct at once. Recall
that `cargo-shear` could not be installed under this project's own pinned toolchain. That is a
preview of how it goes.

**Committed artifacts, hand-authored per fixture.** Hermetic, fast, no runtimes. And the failure
mode is the one in §1: nineteen independent authoring decisions, each made by someone who can see
the score.

**Committed artifacts, generated from declared ground truth by one fixed rule.** What follows.

---

## 3. The decision

Fixture coverage is **generated**, never hand-written, from one new piece of declared ground truth,
and the rule is fixed before a single artifact exists.

Each fixture already declares `live_paths`, `live_symbols`, `decoy_dead_paths` and
`decoy_dead_symbols`. It gains one more declaration:

> **Of this fixture's live artifacts, which does a test suite exercising its documented entry point
> actually enter?**

That is a property of the injected mechanism, not a tuning knob. m12's aliased function is called
through at runtime, so a test enters it. m05's error path is entered by no test that does not inject
a fault. m08's CI manifest reference names a script that runs in a pipeline and not in a test
process. m18's entry points are read by the platform — CPython's `site` module, Android's broadcast
dispatch — and not by anything inside the test process. The answers differ per class **because the
mechanisms differ**, and that difference is the whole content of the measurement.

> **Two examples here were wrong, and writing the declarations is what surfaced it.** m09's README
> block is not "executed by a human reading documentation": `#![doc = include_str!("../README.md")]`
> makes it a doctest, and `cargo test --doc` runs it — so it is one of only three classes whose live
> symbol a test suite genuinely calls. And m11's reflective field is not entered in any sense
> coverage can record, for the reason in §1. Deriving each answer from the fixture rather than from
> memory is exactly what the rule was for.

Three constraints on how it is done, and they are the point:

1. **All nineteen, in one pass, before any measurement is re-run.** A rule applied to a subset is
   the hand-authoring option wearing a rule's clothes.
2. **The declaration is about the mechanism, not about the analyzer.** If writing it requires
   knowing what vulture did, it is the wrong field.
3. **Decoys are never covered.** A decoy is genuinely dead; an artifact showing one executed would
   be a false statement about the fixture, and the catalogue's decoy recall column would stop
   meaning anything.

Each generated artifact ships with its control, generated by the same rule, and with a note in the
fixture saying the artifact asserts a *shape* rather than recording a run.

---

## 4. What the result may not be read as

**Not a precision estimate for coverage in general.** The catalogue over-represents runtime
dispatch, because that is what makes an artifact invisible to static analysis and therefore worth
injecting. Runtime dispatch is precisely the population an execution signal sees. The catalogue
therefore flatters coverage relative to a real repository, where much of what survives is
configuration, platform branches and error handling that no test enters. A number from this suite
bounds the adapter, not the technique.

**Not evidence that the adapter reads real artifacts.** A generated tracefile is written by the
same project that parses it, so a shared misunderstanding of the format is invisible to the whole
suite. The one thing that closes that gap is a tracefile produced by a real instrumenter — done
before the generator rather than after it, and it earned its place immediately: Coverage.py and c8
turn out to use *different* `FN:` dialects, so guessing one would have lost every function record
from half the ecosystem (`judged-core/tests/coverage_real_artifacts.rs`).

**Not a Tier-0 clearance.** §9.5's quorum needs two of {B, R, X}. This adds the first X. Family B —
build-system and deploy-time evidence — is still entirely absent, and R1 stays where the
determination left it until a second family exists and the catalogue is clean under the pair.

---

## 5. The pre-commitment, written before the measurement

Stated now, so it cannot be chosen afterwards:

- If the declaration in §3 comes out as *"every live artifact is entered by a test"*, the rule was
  applied wrongly. A catalogue on which coverage is a perfect oracle is a catalogue that has been
  written to flatter it, and the correct response is to re-derive the field from the mechanisms and
  not to publish the run.
- If coverage rescues the three surviving false removals **and** the classes it leaves untouched are
  the ones whose mechanisms genuinely bypass a test process, that is a real result, and it is still
  a result about eighteen readable classes and one family.
- If coverage rescues nothing on a correctly declared catalogue, that is also a real result, and it
  says the E2 fixtures do not exercise the population an execution signal reaches. It would not be a
  reason to widen the layer.

---

## 6. The measurement

Implemented the same day. Every prediction below was written into the fixtures, and the pinning
table and its pre-commitment assertion were green, before any analyzer was run.

**Two properties of coverage did most of the work, and neither is a property of this catalogue.**
`FNDA` records **functions** — classes, model fields and module names have no function record
however thoroughly they are exercised — and most of this catalogue's live symbols are classes.
Import-time execution is **language-specific**: a Python or JavaScript module that is merely
imported has executed lines, while a Rust or Go file whose functions are never entered has none.
Seven of nineteen classes declare any execution; three of those declare a called symbol.

Four analyzers, full stack (`--gate1 --veto --roots`), with and without the layer:

| SUT | graded | false removals, no coverage | with coverage |
| --- | ---: | --- | --- |
| vulture 2.16 | 10 / 19 | 3 — m11 | 3 — m11 |
| knip 6.31.0 | 3 / 19 | 1 — m02 | **0** |
| deadcode v0.48.0 | 1 / 19 | 1 — m12 | 1 — m12 |
| cargo-shear 1.13.3 | 6 / 19 | 0 | 0 |

Five surviving false removals became four, no decoy was lost anywhere, and the three classes
behaved exactly as declared:

- **m02 cleared.** knip's false removal was a *path* claim on the dynamically imported transport.
  The import runs, so the module loads, so the claim is dropped — and no Family-R layer could reach
  it, which is why the class existed.
- **m11 untouched.** It declares no live paths at all, and its three live symbols are model fields.
  A field is not a function, so an execution signal has nothing to say about it at any granularity.
  This is the honest half of the result: the class an execution signal was most plausibly going to
  rescue is the one it structurally cannot.
- **m12 halved.** `drain` is called through the `//go:linkname` alias and is rescued; the surviving
  false removal is exactly `TelemetryFlush`, the ABI export whose consumer is outside the
  repository — m19's situation, and it gets m19's answer.

The prediction-to-outcome match is the load-bearing part. Not that the number improved, but that
it improved *where the mechanism said it would* and stayed still where the mechanism said it would.

---

## 7. What this leaves undone

- **Family B is still absent**, so §9.5's quorum is still unreachable and R1 still stands where the
  determination left it. That is now the single blocking item, in the place §2 of the previous
  handoff held.
- The lcov 2.x index form — `FNL`/`FNA` — is not parsed. An artifact using it yields zero functions,
  fails the control's floor, and is discarded whole rather than believed. That is the safe failure,
  and widening the parser is a recall improvement rather than a correctness fix.
- Production coverage as a weak **accuser** (+0.5 bans, §9.5). Nothing here accuses, and nothing
  should until a source can be told apart from a test run.
- m13 (PHP) is still read by no analyzer, so the catalogue measures 18 of 19 whatever this layer does.
