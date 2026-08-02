# Handoff — Family X exists now, and Family B is the only thing left in the way

**Date:** 2026-08-02 · **HEAD:** `4eb0a40` on `feat/x-family-lcov-ingest` ·
**PR:** [#3](https://github.com/ca1773130n/Judged/pull/3), green, unmerged · **Tests:** 866

Read [`docs/decisions/2026-08-02-e2-coverage-artifacts.md`](../decisions/2026-08-02-e2-coverage-artifacts.md)
first — its §6 is the measurement and §7 is what remains. The
[previous handoff](./2026-08-02-next-steps-and-the-x-family-gap.md) is still current for
everything it says about toolchains, the tuning tripwire, and the traps; this document does not
repeat it. What follows is what changed, and what the next session should pick up.

---

## 1. The one-line version

`judged mutants --coverage` ingests an lcov tracefile and lets observed execution rescue claims.
It is the first non-Family-R signal in the project. Five surviving false removals across four
analyzers became four; knip is now clean.

That is a smaller number than it sounds, and §2 is why.

---

## 2. Family B is now the blocking item, in the place the X gap held

§9.5 needs a quorum of two of {B, R, X}. There is now one X. There is still no B — no build-system
or deploy-time evidence of any kind — so **Tier 0 is still unreachable by construction, and §11 R1
still stands where the determination left it.** Nothing measured this session changes that, and no
amount of improving the coverage layer will.

If you only do one thing: find the cheapest honest B signal and build it the way this one was
built — ingest rather than collect, rescue rather than accuse, positive control in the same commit,
and the fixture declarations written before anything is measured.

---

## 3. What the measurement actually showed

Two properties of coverage did most of the work, and neither belongs to this catalogue:

- **`FNDA` records functions.** Classes, model fields and module names get no function record
  however thoroughly they run. Most of the catalogue's live symbols are classes.
- **Import-time execution is language-specific.** A Python or JavaScript module that is merely
  imported has covered lines; a Rust or Go file whose functions are never entered has none.

Seven of nineteen classes declare any execution; three declare a called symbol. The three classes
that were failing behaved exactly as their declarations predicted, which is the part worth trusting
— not that the number improved, but that it moved where the mechanism said and stayed still where
the mechanism said:

| class | before | after | why |
| --- | --- | --- | --- |
| m02 (knip) | false removal | **cleared** | a path claim on the dynamically imported transport; the import runs, the module loads |
| m11 (vulture) | 3 false removals | unchanged | no live paths at all, and three live model **fields** — no function record at any granularity |
| m12 (deadcode) | 2 claims, 1 surviving | 1 surviving | `drain` is called through the alias; `TelemetryFlush` is an ABI export consumed outside the repo |

m11 is the honest half. The class an execution signal looked most likely to rescue is the one it
structurally cannot.

---

## 4. Do the real-instrumenter check first, whatever you build next

This was the highest value-per-minute thing in the session and it nearly got done in the wrong
order. Before the generator existed, two tracefiles were produced by running real code under real
tools and committed byte-exact
(`crates/judged-core/tests/data/`, read by `judged-core/tests/coverage_real_artifacts.rs`).

They immediately caught something a generated fixture never could. **Coverage.py and c8 use
different `FN:` dialects** — `FN:<start>,<end>,<name>` against `FN:<line>,<name>` — and guessing one
would have silently lost every function record from half the ecosystem while every test in the
project went on passing, because the same misunderstanding that read the fixtures would have
written them.

That generalises to any adapter whose fixtures this project authors. Twenty minutes producing one
real artifact from the real tool, before the generator rather than after it, buys a check nothing
downstream can.

They also corrected a claim the code had asserted without measuring. Coverage.py reports `DA:7,1`
on a `def` line whose body is `DA:8,0` — the definition genuinely executes at import — while c8
reports `DA:9,0` for the equivalent JavaScript declaration, because a hoisted function is not an
executed statement in its model. Same conclusion by two different mechanisms.

---

## 5. Things that will bite

**`runner_suts::command_sut::stdout_is_handed_to_the_supplied_parser` is flaky on Linux CI.** It
failed once this session with `Text file busy (os error 26)` spawning a script the test had just
written, and passed on re-run and in the sibling job on the same commit. It is not related to any
change here.

The mechanism is the standard fork/exec race. `fs::write` closes its descriptor, but a *different*
test thread forking in that instant hands the child a copy, and the child holds it until its own
exec — during which our exec of that file returns `ETXTBSY`. Small window, real one.

Two fixes are reasonable, and choosing between them is why this PR did not: retry the spawn on
`ETXTBSY` at the ~15 call sites in `mod command_sut`, or serialize write-then-spawn within the
binary behind a mutex. The second is sufficient — other test binaries are separate processes and
never inherit the descriptor.

**The coverage artifact is planted for both runs of a `--coverage` invocation, never one.** The
bare run is the baseline the gated run is subtracted from, so a tracefile present in only one would
put the analyzer's opinions about that file into the difference as though a rescue layer had caused
them. It does mean a `--coverage` bare run is not the same measurement as a bare run without it —
the repository genuinely has one more file — and the layer's `config` line says so.

**A new fixture fails `tests/coverage_declarations.rs` until it declares.** Deliberate. The default
is "nothing entered", which is conservative and also silent, and a class that never declared must
not be indistinguishable from one whose mechanism no test reaches.

---

## 6. Running it

Everything in the previous handoff's §4 still applies. Vulture is per-run and was installed this
session under `~/.blackhole/Judged/2026-08-02/analyzers/.venv`; `cargo-shear` and `deadcode` are
still in `~/.blackhole/Judged/2026-08-01-tools/bin`.

```sh
export PATH="$HOME/.blackhole/Judged/2026-08-02/analyzers/.venv/bin:$HOME/go/bin:$HOME/.blackhole/Judged/2026-08-01-tools/bin:$PATH"
cargo run -q -p judged-cli -- mutants --sut knip --gate1 --veto --roots --coverage
```

---

## 7. Backlog, unchanged except where noted

- **Family B.** §2. The only blocking item.
- The lcov 2.x index form (`FNL`/`FNA`) is not parsed. Such an artifact yields zero functions, fails
  the control's floor, and is discarded whole — the safe failure. Widening the parser is recall,
  not correctness.
- Production coverage as a weak accuser (+0.5 bans, §9.5). Nothing accuses today and nothing should
  until a production source can be told apart from a test run.
- Root-set hand-check accuracy is 85% (40 of 47 sampled), down from 97% when the set was smaller.
- m13 (PHP) is read by no analyzer, so the catalogue measures 18 of 19 — including under coverage.
- Six of Gate 1's sixteen classes have never fired in any measurement: the six about untracked and
  ignored state, which is the population the layer exists for.
