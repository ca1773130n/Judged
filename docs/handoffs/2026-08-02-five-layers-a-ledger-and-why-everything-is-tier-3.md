# Handoff — five layers, a ledger, and why every candidate is Tier 3

**Date:** 2026-08-02 (evening) · **HEAD:** `50dea7d` on `main` · **Tests:** 915 · **CI:** green

Supersedes [`2026-08-02-family-x-landed-and-what-actually-blocks.md`](./2026-08-02-family-x-landed-and-what-actually-blocks.md)
and, through it, [`2026-08-02-next-steps-and-the-x-family-gap.md`](./2026-08-02-next-steps-and-the-x-family-gap.md).
Both stay in place as a record. Neither is current guidance — though the traps in the older one's §5
are all still true, and still worth the five minutes.

Five PRs merged since. Read
[`docs/decisions/2026-08-02-ban-ledger-and-tier-model.md`](../decisions/2026-08-02-ban-ledger-and-tier-model.md)
first — its §5.2 is the finding that reorders everything else.

---

## 1. Where it stands

| Layer | Flag | Does |
| --- | --- | --- |
| Gate 1 | `--gate1` | The never-touch inventory (§9.3 1a–1p). Refuses on irreversibility. |
| Gate 3f | `--gate3f` | §6.24: serializable type, queue payload, ABI export. *No ban count overrides this.* |
| Coverage | `--coverage` | Family X. Ingests lcov; a hit rescues, a miss contributes zero. |
| Root set | `--roots` | §5 Tiers A/B/C, now including §5.2's in-source markers. |
| Gate 2 | `--veto` | The reference veto (§9.3 2a/2b/2c/2e). |

Four analyzers, full stack, measured at `50dea7d`:

| SUT | graded | false removals |
| --- | ---: | --- |
| vulture 2.16 | 10 / 19 | 3 — m11 |
| knip 6.31.0 | 3 / 19 | 0 |
| deadcode v0.48.0 | 1 / 19 | 0 |
| cargo-shear 1.13.3 | 6 / 19 | 0 |

Five surviving false removals this morning; three now, all of them m11's reflectively-read model
fields. knip cleared by coverage, deadcode by Gate 3f.

---

## 2. The blocking item, corrected twice, now measured

This is the third statement of what blocks a quorum. The first two were wrong and the corrections
are worth carrying, because both were confident.

**"Family B is the only thing in the way"** — wrong. §9.5 definition 1 ends *"the only two-family
combinations that exist are {B,R}, {B,X}, {R,X}"*, so B is not uniquely required.

**"Then it's the {R,X} route"** — also wrong, for a subtler reason. A family accuses only at
**MAX ≥ +0.5 bans**, the X table gives +0.5 only to *"zero hits, full window, production profiling
present"*, and test coverage is pinned at **0.0, veto only**. What shipped is test coverage. It can
rescue forever and never accuse.

**And underneath both, now built and measured: nothing can be promoted at all, because Gate 0
barely exists.** §9.3's Gate 0 has seven conjuncts and **only 0g is implemented.** 0a (never
traverse a symlink), 0b (refuse to descend into a nested repository), 0c (reject a realpath that
escapes the repo), 0d (refuse to act during a rebase, with a dirty tree, with no remote, in a
shallow clone), 0e (never touch `.git/`), 0f (advisory lock, refuse while a build runs) — none is
anywhere in `crates/`.

§9.6 makes **Tier 2** conditional on "Gates 0–2 pass", so every candidate on every repository is
**Tier 3** — one tier below what the ledger's own pre-commitment assumed before it was wired, and
for a reason that pre-commitment could not have known.

None of this is on the R1 determination's §7 list. Eight items, and not one of them says "Gate 0
barely exists", because until something tried to *assign* a tier nothing had to ask whether its
preconditions were computable at all.

---

## 3. What to do next, in the order the evidence supports

**1. Gate 0a–0f.** Two of its conjuncts are the cheapest safety work outstanding anywhere in this
project — **0e never touch `.git/`** and **0a never traverse a symlink** — and both are refusals
with no measurement behind them, so their absence is the difference between a bug and an
unrecoverable one. 0d and 0f cost nearly as little, and they are what stops a run acting on a tree
somebody is mid-rebase in. Per line written, nothing else here changes what the tool is permitted
to do by as much.

**2. The per-adapter §9.5 row mapping.** The ledger currently earns exactly one R row — Gate 2a's
+1.0 for a completed zero-hit search — and that is a floor rather than a score: deadcode's
compiler-backed analysis earns nothing extra, because the +1.5 row carries a *"zero dynamism
detected"* qualifier nobody computes. Which row each adapter earns is a real decision under §9.2's
*"not more careful than the tool, and not less"*, and it needs its own record.

**3. A second accusing family.** Family B (§7 item 3, regenerate-and-diff) or production-sourced X.
This is the only thing that lifts the quorum, but it is third because with Gate 0 missing there is
nothing for a quorum to unlock.

**4. The two CI flakes** (§5). Small, well understood, and each has already cost a rerun.

---

## 4. Three things that were found by measurement, not by reasoning

Worth repeating because in each case the reasoning was confident and wrong.

**Coverage's reach is much narrower than the design assumes.** `FNDA` records *functions*, and most
of this catalogue's live symbols are classes, model fields and module names — none of which carries
a function record however thoroughly it runs. So m11, the class an execution signal looked most
likely to rescue, is the one it structurally cannot.

**A generated fixture cannot catch a format misunderstanding.** Coverage.py and c8 use *different*
`FN:` dialects. Guessing one would have lost every function record from half the ecosystem while
every test passed, because the same misunderstanding that read the fixtures would have written
them. Twenty minutes producing one real artifact from the real tool, before the generator, bought a
check nothing downstream could.

**A rule at 0% precision nearly shipped.** The in-source root scanner reported five roots on this
repository and all five were wrong — read out of string literals in test fixtures, one with a
line-continuation backslash in the symbol name. Judged has no FFI. The out-of-sample eval already
warned that a bigger, less accurate root set is not an improvement.

---

## 5. Things that will bite

**Two CI flakes, both filesystem races in `judged-mutants` on Linux, both diagnosed.**

- `runner_suts::command_sut::*` — `ETXTBSY` spawning a script the test just wrote. A different test
  thread forks while the write descriptor is open, and the child holds it until its own exec. Fix:
  retry the spawn on `ETXTBSY`, or serialize write-then-spawn behind a mutex (sufficient — other
  test binaries are separate processes and never inherit the descriptor).
- `runner_controls::*` — `DirectoryNotEmpty` from `TempDir::close()` in `run_suite`. Fixtures call
  `Repo::init`, so every mutant repo contains a `.git/`, and `remove_dir_all` races. Fix: retry the
  close, preserving the reason it is explicit — a discarded error is a leaked tree per mutant,
  nineteen per run.

Both pass on re-run. Neither is caused by any change here.

**A gate that does not fire on the class it was built for is invisible.** Gate 3f's queue condition
shipped without firing on m15 — §6.24's canonical shape — because the naive SUT never claims m15's
live artifacts, so every catalogue measurement ran past it. Found by review. **Assert per class, at
fixture level, that a rule fires where its specification says it should.**

**Scaffolding has been less rigorous than the work.** Four separate times a check reported success
having verified nothing: `| tail` swallowing an exit code, a Python `str.replace` that silently did
not match after `cargo fmt` reformatted its target, a poll loop that latched onto a stale job id,
and a shell glob stored in a variable that zsh never expanded. None broke the work, because a real
test or measurement caught each afterwards. Assert that an edit matched; never trust a watcher that
cannot name what it is watching.

---

## 6. Running it

```sh
export PATH="$HOME/.blackhole/Judged/2026-08-02/analyzers/.venv/bin:$HOME/go/bin:$HOME/.blackhole/Judged/2026-08-01-tools/bin:$PATH"
cargo run -q -p judged-cli -- mutants --sut knip --gate1 --gate3f --veto --roots --coverage
cargo run -q -p judged-cli -- explain <path>          # now ends with what §9.6 would require
```

`cargo-shear` still cannot be installed by this project's own toolchain — it needs rustc 1.95
through `ra_ap_syntax` and `rust-toolchain.toml` pins 1.94.

---

## 7. Backlog

- Gate 3a–3e. 3f exists; 3a–3d are directory conjuncts and 3e is the family quorum, which the
  ledger now answers.
- The stability window and the deadness invariant (§9.5 definition 3) need a store of prior runs
  keyed by tree SHA.
- `scanned_universe_ratio`, `ladder_rung`, `has_external_effector`, `is_distributable` — §9.6
  criteria nothing computes, each reported as `NotEvaluable` rather than assumed.
- lcov 2.x's `FNL`/`FNA` index form is unparsed. Such an artifact yields zero functions, fails the
  control's floor, and is discarded whole — the safe failure.
- §5.2 in-source sources still unimplemented: `//go:embed`, `//go:wasmexport`, `#[wasm_bindgen]`,
  `#[pyo3::pymodule]`. Pinned by a test that fails if one is added.
- Root-set hand-check accuracy was 85% (40 of 47) before today's additions and has not been
  re-sampled since.
- m13 (PHP) is read by no analyzer, so the catalogue measures 18 of 19.
- Six of Gate 1's sixteen classes have never fired in any measurement — the six about untracked and
  ignored state, which is the population the layer exists for.
