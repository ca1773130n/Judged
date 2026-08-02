# Handoff — Family X exists now, and what actually blocks a quorum

> **Superseded** by
> [`2026-08-02-five-layers-a-ledger-and-why-everything-is-tier-3.md`](./2026-08-02-five-layers-a-ledger-and-why-everything-is-tier-3.md).
> Kept as a record. §2's correction here was right as far as it went and still incomplete: the
> ledger was built afterwards and found that **Gate 0a–0f does not exist**, so no candidate reaches
> Tier 2 either. Read the newer document for what blocks and in what order.

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

## 2. What is actually blocking — corrected

**This section originally said "Family B is the only thing left in the way." That was wrong, and
it was wrong in a way worth spelling out, because it would have sent the next session to build the
wrong thing.** Reading §9.5's definitions properly gives three corrections:

**There is more than one route to a quorum.** §9.5 definition 1 ends: *"the only two-family
combinations that exist are {B,R}, {B,X}, {R,X}."* `{R,X}` is available in principle, so Family B
is not uniquely required.

**But this X cannot take it.** A family ACCUSES only when its maximum accuse-polarity evidence
reaches **+0.5 bans**. The X table gives +0.5 to *"zero hits, full window, **production profiling
present**"* — and test coverage is pinned at **0.0**, veto only, by the resolved contradiction at
§9.5. What shipped this session is test coverage. It can rescue forever and never accuse, so it can
never be one of the two families. Reaching `{R,X}` means production-sourced evidence with a declared
window and expiry, not a better lcov parser.

**And underneath both: nothing can accuse yet at all.** There is no ban ledger and no §9.6 tier
model in this codebase — no `+0.5`, no accumulation, no tier assignment. `grep` for accuse-polarity
in `crates/*/src` finds only doc comments saying Gate 1 does *not* accuse. The `Tier` enum that
exists is §5.1 root provenance (A/B/C), unrelated. So "which two families accuse" is not a question
the code can answer today, for any family, and building a second signal does not change that.

Which means the next step is not "add a signal". It is the determination's §7 order, whose item 1
— *an X-family signal, at all* — this session discharged. **Item 2 is Gate 3, and 3f specifically:
the only specified gate that does not exist, and the one §7 says would speak to m11 and to classes
15–19 as a group.** Family B is item 3; root-set completion for the §5.2 sources is item 4.

### 2.1 Item 2, since: 3f built, and the rest of Gate 3 is blocked on the ledger

3f shipped and it refuses m12's `//export`, m19's `#[no_mangle]`, m16's pickle and m15's Celery
worker. deadcode went to zero. Only vulture's m11 survives across the whole catalogue.

**Item 2 is not fully discharged, and the remainder is not more of the same work.** 3a–3d are
directory conjuncts — a known build-artifact directory, its toolchain present, no Gate-1 content
inside, no non-ignored file inside — and they are *Tier-0 promotion preconditions*. There is no tier
assignment for them to be preconditions of, so building them now produces a gate with nothing to
gate. 3e is the family quorum outright and needs the ban ledger by definition.

3f was different, and that difference is the reason it was buildable alone: it is an absorbing
refusal, so it stands on its own exactly as Gate 1 does. The rest of Gate 3 does not.

So the honest reading of §7 after this session: the items with standalone value are **item 4**
(root-set completion for the §5.2 sources — concrete, named, and it speaks to m18, m13 and m19) and
**the ban ledger with §9.6's tier model**, which is not on §7's list in its own right but which item
8 depends on entirely — *"until a tier is assigned to anything, none is computable at all."*

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

- **Gate 3a–3e.** 3f is built (§2.1). The rest are Tier-0 promotion preconditions and 3e is the
  family quorum, so both wait on the ledger below.
- **A finding-by-review lesson worth keeping.** 3f's queue condition missed m15 — the very class
  it exists for — because the naive SUT never claims m15's live artifacts, so every measurement
  ran past it and the catalogue stayed green. A gate that silently does not fire on the class it
  was built for is invisible to a suite that never asks. Assert per-class, at fixture level, that
  a gate fires where its specification says it should.
- **A ban ledger and the §9.6 tier model.** Nothing computes bans, so no family can accuse. §7
  item 8 depends on it: until a tier is assigned to anything, none of §10's headline metrics is
  computable.
- **Family B** (§7 item 3, regenerate-and-diff) and **root-set completion** for the §5.2 sources
  the implementation lacks (§7 item 4: `//go:linkname`, `//export`, `.pth`, `#[no_mangle]`,
  `AndroidManifest.xml`, `composer.json`).
- The lcov 2.x index form (`FNL`/`FNA`) is not parsed. Such an artifact yields zero functions, fails
  the control's floor, and is discarded whole — the safe failure. Widening the parser is recall,
  not correctness.
- Production coverage as a weak accuser (+0.5 bans, §9.5). Nothing accuses today and nothing should
  until a production source can be told apart from a test run.
- Root-set hand-check accuracy is 85% (40 of 47 sampled), down from 97% when the set was smaller.
- m13 (PHP) is read by no analyzer, so the catalogue measures 18 of 19 — including under coverage.
- Six of Gate 1's sixteen classes have never fired in any measurement: the six about untracked and
  ignored state, which is the population the layer exists for.
