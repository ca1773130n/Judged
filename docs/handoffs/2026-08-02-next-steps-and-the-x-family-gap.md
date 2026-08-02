# Handoff — what to do next, and the one thing that blocks everything

**Date:** 2026-08-02 · **HEAD:** `8ce8cad` · **Tests:** 819 · **CI:** green

Read [`docs/decisions/2026-08-02-r1-determination.md`](../decisions/2026-08-02-r1-determination.md)
first. It is the standing determination on §11 R1 and it constrains what the next session is
allowed to do. Its §7 is an ordered list of what remains; this document does not repeat it. What
follows is the part that is not in there: which item to take, the decision it leaves open, how to
run the things that need external tools, and the traps that cost this session time.

---

## 1. Where it stands, in one screen

Four crates, four commands, and nothing that deletes anything.

| Command | Does |
| --- | --- |
| `judged ratchet` | Baselines SARIF findings, fails CI only on new ones. Refuses a failed run before recording. |
| `judged mutants` | Runs the §10 E2 catalogue — 19 injected liveness mechanisms — against a SUT. |
| `judged show-roots` | Materializes the root set (§5), grouped by provenance tier, each root citing its key. |
| `judged explain <path>` | Full gate trace for one path, including its Gate 0g recoverability class. |

Rescue layers, each independently switchable so a rescue can be attributed to the layer that
earned it: `--gate1` (never-touch inventory), `--veto` (Gate 2 reference veto), `--roots`.

The measured state, four real analyzers with every rescue layer on:

| SUT | classes graded | false removals | on |
| --- | ---: | ---: | --- |
| vulture 2.16 | 10 / 19 | 3 | m11 |
| knip 6.31.0 | 3 / 19 | 1 | m02 |
| deadcode v0.48.0 | 1 / 19 | 1 | m12 |
| cargo-shear 1.13.3 | 6 / 19 | 0 | — |

Five surviving false removals across three classes. Do not read that as a score. Every signal in
the project — all four analyzers and all three rescue layers — lives in **Family R**, and §9.5
requires a quorum of at least two of {B, R, X}. No configuration measured here could have produced
a Tier-0 action even scoring zero.

---

## 2. Take the X-family signal. It is the only blocking item.

§7 of the determination says this and it is right: nothing in this project observes execution.
§9.5 line 1217 makes Tier 0 unreachable by construction without an X signal, so R1 is not
answerable in either direction until one exists. Everything else on that list improves the tool;
only this one changes what can be concluded.

The determination names the item but does not pick an implementation. Here is the call I would
make, and the reasoning, so the next session can disagree with something concrete.

### 2.1 Ingest, do not collect

§11 R9 states this as an open question and §9.10 effectively answers it. Collecting coverage means
executing the target repository's code plus its entire transitive lockfile on the machine running
the cleaner — "clean my repo" becomes remote code execution. Ingesting an artifact CI already
produced has none of that surface.

Start with **lcov `.info`**. §2.1 calls `FNDA:0,<name>` the single most valuable cross-language
primitive in the survey — "this named function was never called", and it merges across runs with
`lcov -a`. JaCoCo, coverage.py's SQLite, and `go tool covdata` can follow; lcov alone reaches
several ecosystems at once.

### 2.2 It arrives as a veto, not an accuser

This matters more than the file format and it fits the architecture already built.

A coverage **hit** is proof of use. A coverage **miss** is bounded absence of evidence over one
window and one input distribution. §9.5 resolves the contradiction explicitly: a test-coverage miss
contributes **zero** toward deadness at any tier, because the miss is not merely weak but
systematically anti-correlated with the value of the code — error handlers, disaster-recovery
paths, platform branches. So the first X-family component is a fourth rescue layer beside
`--gate1`, `--veto` and `--roots`, wired the same way, with the same only-ever-removes-claims
invariant.

Production coverage may accuse, weakly (+0.5 bans, §9.5). Test coverage may not, ever.

### 2.3 Ship the positive control in the same commit

§3.7 is blunt: every catastrophic failure in the corpus presents identically as *"~0% covered
everywhere"*. Before trusting any coverage artifact, require a small declared set of always-live
symbols to appear executed, and discard the whole artifact loudly if they do not.

And specify it at the right granularity or it is theatre. In Python, Ruby and JS, `def`, `class`
and module-level lines execute at *import*, so under every documented failure mode you get
boot-only coverage in which a health-check handler's `def` line **is** covered while every function
body reads dead. Assert at function-body-line or `FNDA` granularity, plus a plausible floor.

### 2.4 The E2 problem you will hit immediately

The 19 fixtures do not currently produce coverage. To grade an X-family adapter you need either
runnable test suites in the fixtures or committed coverage artifacts alongside them. Both are
defensible; they are not equivalent.

Committed artifacts are hermetic and fast and make the fixture assert a *shape*. Runnable suites
are more honest and much slower, and they drag a language runtime into the harness for every
ecosystem. My inclination is committed lcov artifacts first — the adapter is the thing under test,
not the runtime — with a note in the fixture saying so. Decide deliberately and write down why.

---

## 3. What the determination forbids

There is a tripwire in §5 of the determination and it is easy to trip by accident.

Building Gate 3, adding a root source §5.2 names, writing the §6.24 serializer veto — that is
**implementation**. The design specifies them and they do not exist yet.

Widening a needle set, adding a root rule, adjusting a threshold, or editing a fixture **because
m02, m11 or m12 failed** is **tuning**, and the pre-commitment already answers tuning: the tier is
deleted, not tuned. The test is whether a change implements more of the specification or alters it.
A change that cannot be classified on that line is tuning.

This is not pedantry. Three surviving classes with three plausible fixes is exactly the situation
where a project talks itself into tuning one fixture at a time.

---

## 4. Running things

Two analyzer binaries live outside the repo and are needed by `--sut shear` and `--sut deadcode`:

```sh
export PATH="$HOME/go/bin:$HOME/.blackhole/Judged/2026-08-01-tools/bin:$PATH"
```

`cargo-shear` **cannot be installed by this project's own toolchain** — it needs rustc 1.95 through
`ra_ap_syntax`, and `rust-toolchain.toml` pins 1.94. It was installed with
`rustup run nightly cargo install cargo-shear --root ~/.blackhole/Judged/2026-08-01-tools`. Worth
remembering as a finding in its own right: an analyzer's usefulness is bounded by whether it
installs in the environment the cleaner runs in.

Vulture and knip are fetched per-run:

```sh
uv venv && uv pip install vulture     # under ~/.blackhole, never in the repo
npx knip@6                            # knip needs no install
```

Full stack for one analyzer:

```sh
cargo run -q -p judged-cli -- mutants --sut vulture --gate1 --veto --roots
```

---

## 5. Traps, all of which cost this session time

**`| tail` returns tail's exit status.** This produced a wrong answer three separate times,
including once in a document. Capture exit codes as `cmd >/dev/null 2>&1; echo $?` and never
through a pipe.

**Give concurrent agents one file each, and create the module skeleton first.** Two rounds were
damaged by several agents owning the same `mod.rs`. Writing the stub files and the `pub mod` lines
by hand before dispatching fixes it completely.

**Never forbid `Cargo.toml` edits to prevent collisions.** That instruction, meant to stop agents
colliding, produced 1,300 lines of hand-written TOML and YAML parsing that rejected valid manifests
in 7 of 9 real repositories. Own the manifest yourself and hand agents the dependency, or accept
the collision risk. The cost of the workaround was far higher than the collision would have been.

**Agents that own only `crates/` leave the documentation lying.** Every round where implementation
and documentation had different owners ended with the README describing a limitation that had been
removed. Put docs in the same agent's scope, or budget a correction pass and expect to need it.

**Verify the numbers yourself before committing.** Two measurements this session looked like
triumphs and were artifacts — the Gate 2 self-veto (8 of 10 prevented, mostly symbols rescued by
their own declaration) and the first corpus run. In both cases what exposed it was a column going
to zero, not a failing test.

**Watch the in-sample line.** Fixing a defect the corpus found and then re-measuring on the same
corpus is no longer out-of-sample for the half that changed. This was caught by a reviewer, not by
me, and it is easy to lose.

---

## 6. Backlog, none of it blocking

- Root-set hand-check accuracy is 85% (40 of 47 sampled), down from 97% when the set was smaller.
  A bigger root set that is less accurate is not straightforwardly an improvement.
- m13 (PHP) is read by no analyzer, so the catalogue measures 18 of 19.
- Gate 1 protects 8.6%–79.6% of tracked files depending on repository. The 79.6% case is probably
  over-firing; §6.17 measured only 3.6% of canonical gitignore patterns as explicitly
  irreplaceable.
- Six of Gate 1's sixteen classes have never fired in any measurement — the six about untracked and
  ignored state, which is the population the layer exists for.

---

## 7. If you only do one thing

Wire lcov ingestion as a fourth rescue layer, with a `FNDA`-granularity positive control, and
re-run the E2 suite. That produces the first configuration in this project's history that could
legitimately be called a signal combination, and §11 R1 becomes answerable for the first time.

Until then the determination stands: **not yet testable, tier presumed absent**, and the honest
product is report plus quarantine.
