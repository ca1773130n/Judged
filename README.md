# Judged

[![CI](https://github.com/ca1773130n/Judged/actions/workflows/ci.yml/badge.svg)](https://github.com/ca1773130n/Judged/actions/workflows/ci.yml)

A repository cleaner that starts from an uncomfortable premise: there is no
sound, general way to prove a file or a symbol is unused. Entry points get
invoked by humans, config strings, schedulers, and other repositories. The root
set is unknowable, and that's Rice's theorem plus an open world rather than a
gap somebody will close with a better parser.

So Judged is not a better analyzer. Existing analyzers answer "unreachable from
root set R under resolver X"; none of them answers "is deleting this safe". The
research this repository is built from
([docs/research](docs/research/2026-07-31-universal-safe-repo-cleaner-research.md))
argues the product is the layers around those analyzers, and that two of them
should exist before anything is allowed to delete a byte.

Judged deletes nothing. Not by default, not behind a flag. There is no `--fix`,
and passing one is refused before the subcommand is even parsed. The ratchet's
only power is to fail a build; the mutant suite writes exclusively to throwaway
directories. That is the design (§9.14: the ratchet has zero deletion risk), not
a milestone that hasn't landed yet. Whether an auto-act tier is ever allowed to
exist is the single highest-risk open question in the research (§11 R1), and the
suite below is the experiment that decides it, with the answer pre-committed: if
no signal combination clears the catalogue at zero false removals, the tier is
deleted from the design rather than tuned.

## The two commands

### `judged ratchet` — baseline today's findings, fail CI only on new ones

```sh
judged ratchet --sarif run.sarif [--baseline .judged/baseline.jsonl] [--update]
```

Reads analyzer output as SARIF, diffs it against a committed baseline by
content-derived fingerprint, and reports what is new. Nothing is configured and
nothing is fixed. The prior art is Shopify's `deprecation_toolkit`, which worked
because it never demanded the backlog be fixed first.

Two guards sit on top of the diff. It will not judge a *failed* run: a log whose
`executionSuccessful` is false or absent exits 2 instead of recording "nothing
new", because an analyzer that died reports zero findings and a green ratchet
built on that is permanently disarmed. A merely *degraded* run (a rule disabled,
a scanned universe below the 0.8 `analysisTarget` floor) still produces a
verdict, but one labelled as not covering the whole repository: degradation caps
what a result means rather than discarding it. And it flags baselines that have
rotted into a permanent amnesty list, since entries matching nothing are the
documented failure mode of every tool in this shape.

| Exit | Meaning |
| --- | --- |
| 0 | No new findings and no rot, or `--update` wrote the baseline. |
| 1 | New findings, or baseline rot. |
| 2 | Refused to judge: unreadable log, failed run, no git repository, refused flag. |

The `--sarif` input is the SARIF 2.1.0 *subset* modelled in
`judged_core::sarif`, with `tool.driver`, `artifact.location.uri` and
`result.message.text` already flattened. Adapters that produce it from raw ruff
or knip output do not exist yet; the contract they will be held to does.

### `judged mutants` — inject known-live artifacts and see what gets called dead

```sh
judged mutants --sut naive|refusing|vulture|knip|deadcode|shear [--json]
judged mutants --sut command [--json] -- <analyzer> [args...]
```

Nineteen mutants, each materialized into a temporary repository when graded, each planting
an artifact that is genuinely live but reachable through exactly one mechanism:
a class named only in a YAML string, a plugin found by scanning a directory, a
`//go:linkname` alias, a worker named only in a job payload already sitting in
the queue. Any "dead" verdict on one of them is a false removal, and the gate is
zero false removals, nothing else. Every mutant also plants genuinely-dead
decoys, so a tool that refuses to answer cannot score a perfect run.

Exit 0 only when the false-removal count is zero; 1 otherwise; 2 when the suite
could not be run.

| `--sut` | What gets graded |
| --- | --- |
| `naive` | A deliberately bad cleaner, shipped with the suite. The default, and the positive control. |
| `refusing` | Calls nothing dead. The negative control. |
| `vulture` | The installed `vulture`, invoked at its own defaults. |
| `knip` | `npx knip@6`, reporting SARIF. |
| `deadcode` | `golang.org/x/tools/cmd/deadcode`, reporting JSON. |
| `shear` | `cargo-shear`, reporting JSON. |
| `command` | Whatever argv follows `--`. |

Everything below `refusing` is how a real analyzer gets in: four by name, and
`command` for anything else, so adding a tool needs no code change. The analyzer
is run once per fixture repository, from inside it, and its stdout is read —
that is the entire interaction.

**A SUT declares the ecosystems it reads, and the suite skips the rest.** A
language-specific analyzer handed a repository in the wrong language exits
non-zero, and that exit code is one it shares with a genuine analysis failure —
so it cannot be waved through without also scoring a crashed run as a clean one.
The runner instead never builds those fixtures. A class outside the SUT's
declared ecosystems is **not read**: a third state beside pass and fail, counted
on its own line, excluded from both halves of the decoy ratio, and incapable of
being scored as a pass. The summary line carries all three numbers:

```
19 classes: 10 graded — 3 passed, 7 failed; 9 not read
```

Skipping moves the denominator, not the grade. All four named analyzers produce
a graded result — over 10, 3, 1 and 6 of the 19 classes respectively — and the
decoy ratio is computed only over what each one actually read.

Judged never passes an analyzer a `--fix` mode: a deletion-shaped flag is
refused wherever it appears, including inside the argv after `--`. That is a
claim about what Judged does, and it holds. It is **not** a claim that the
analyzer does not write, and one of the four breaks that: `cargo shear` begins
by running `cargo metadata`, which resolves the dependency graph and writes
`Cargo.lock` — observed, not inferred, and no flag combination avoids it
(`--frozen` prevents the write but then refuses to run). §9.2 forbids invoking a
tool's fix mode and assumes the read path is inert; an analyzer that mutates
while merely reading is a category it does not name. Judged discloses it rather
than claiming it away. `--sut vulture` uses vulture's own defaults rather than a tuned
`--min-confidence`, because a score obtained after picking the threshold that
suits our own fixtures would be comparable to nothing; tuning is spelled
`--sut command -- vulture --min-confidence 100`, which is honest about being a
different experiment.

**A missing analyzer exits 2 and names what to install.** This is the one place
the feature could have gone quietly wrong, so it is worth stating plainly: an
analyzer that is not on the machine claims nothing dead, which is zero false
removals, which is the number that clears the gate. Reporting that as a pass
would be a green build certifying a tool that was never there. So the run stops
before the fixtures are built:

```
$ judged mutants --sut vulture
REFUSED — the analyzer `vulture` is not installed (exit 2)

  Looked for `vulture` in the 47 directories on PATH; it is in none of them.
  (The directory count is read from the running PATH, so it differs per machine.)
  Install it with `pipx install vulture`, or `pip install vulture` into the environment judged runs in. It needs Python.

No verdict was reached and no class was graded. This is a refusal rather than a result on purpose: an analyzer that never ran claims nothing dead, which is zero false removals, which is the number that clears the release gate. Grading it would certify a tool that was not here (§3.7, §6.20).
$ echo $?
2
```

Every report produced through an adapter carries that adapter's capability
envelope — the finding classes the tool structurally *cannot* emit — and the
decision about which half of a verdict its findings were mapped to, printed
above the table. Without those, a low false-removal count reads as a good score
when it may only be a narrow one.

## The headline result

Both systems under test are controls that ship with the suite. The summary lines
below are verbatim; the nineteen per-class lines above each are elided.

```
$ judged mutants --sut refusing
19 classes: 19 graded — 0 passed, 19 failed; 0 not read
decoy recall: 0 of 31 genuinely-dead files found
false removals: 0 — GATE PASSED (§10 E2 gates releases on this number, and on nothing else)
note: this SUT removed nothing at all, so it cleared the gate without demonstrating it can find anything. Zero false removals is also the score of a tool that refuses to answer.
$ echo $?
0

$ judged mutants --sut naive
19 classes: 19 graded — 7 passed, 12 failed; 0 not read
decoy recall: 31 of 31 genuinely-dead files found
false removals: 20 — GATE FAILED (§11 R1: if this is not zero, the auto-act tier is deleted from the design rather than tuned)
classes with false removals: m01, m02, m03, m08, m09, m10, m12, m13, m14, m16, m18, m19
$ echo $?
1
```

`RefusingSut` calls nothing dead. It passes the gate and exits 0, which is the
point of running it: zero false removals is also the score of a tool that says
nothing, so the report states that in as many words and the decoy line is what
exposes it. `NaiveSut` is a deliberately bad cleaner (grep the identifier, delete
if unreferenced) and 12 of the 19 classes catch it, for 20 false removals. If it
ever exited 0, the suite would be theatre.

Be careful what that bounds. These are two reference SUTs, not real third-party
analyzers, so what has been measured is the harness: it catches both failure
directions, the cleaner that over-deletes and the cleaner that never speaks, and
its 19 classes are discriminating enough to separate them.

**Four third-party analyzers have now been graded** — vulture 2.16 (Python),
knip 6.31.0 (JS/TS), `x/tools` deadcode v0.48.0 (Go) and cargo-shear 1.13.3
(Rust). That is four of the ecosystems the catalogue injects into, not all of
them:

| Tool | Graded | Not read | Passed | False removals | Decoy recall | Exit |
| --- | --- | --- | --- | --- | --- | --- |
| vulture 2.16 | 10 | 9 | 3 | **6** — m01, m10, m11, m16 | 11/16 | 1 |
| knip 6.31.0 | 3 | 16 | 0 | **2** — m02, m14 | 4/6 | 1 |
| deadcode v0.48.0 | 1 | 18 | 0 | **2** — m12 | 2/2 | 1 |
| cargo-shear 1.13.3 | 6 | 13 | 6 | **0** | 9/9 | 0 |

**Ten false removals across the four. Five classes — m01, m11, m12, m14, m16 —
are false-removed by every tool that can read them.** Full write-up, with raw
output, the configuration sweep and the limits, in
[`docs/evals/2026-08-01-four-analyzers-e2.md`](docs/evals/2026-08-01-four-analyzers-e2.md).

**And one class, m13, is read by none of the four.** Its live artifact is PHP
rescued from a broad ignore rule by a `!` negation, beside a `composer.json` and
a checked-in media file, and no adapter here covers PHP. The union of all four
analyzers grades 18 of the 19 classes, so the catalogue currently measures 18 of
the 19 mechanisms it defines — the nineteenth has no reader at all.

cargo-shear is the one tool that clears the gate, and both halves of that need
saying. The decoy recall is real competence: 9 of 9 genuinely-dead files found
inside its scope, so this is not the refusing control's degenerate zero. But it
answers two questions — is a declared dependency unused, is a file unreachable
by `mod` declaration — and neither can produce the claim that would be wrong on
the classes it read. On m17 and m19 it names the decoy correctly and stays
silent about the live artifact, which is `mod`-declared; cargo-shear never asks
whether a symbol has callers. A catalogue of mostly symbol-level mechanisms, a
tool that reads declared dependencies and module linkage, and 6 of the 19
classes graded: an exit 0 here is a fact about its capability envelope before it
is a fact about the mechanisms, and it says nothing about whether an auto-act
tier could exist.

Read each row with its denominator. Only 20 of the 76 tool×class cells are
graded results; the other 56 were never read, because the class is outside the
analyzer's declared ecosystems. Every per-tool false-removal count is a count
over a small, language-determined subset. §4.1's prior figure for vulture on
other corpora — 44 true positives against 644 false positives across 9 projects,
59 of them on httpx, which contains no dead code at all — is a different
experiment and is not this number.

A bad score is a result, not a bug to tune out. Nothing in the fixtures, the
adapter or the grading was adjusted after seeing it, and §11 R1's consequence is
pre-committed in both directions. Four off-the-shelf analyzers at their defaults
are not "no signal combination", so nothing here discharges R1; what they
establish is that the harness grades reality rather than only the two SUTs we
wrote ourselves.

## Layout

| Crate | What it holds |
| --- | --- |
| `judged-core` | The SARIF 2.1.0 subset adapters are held to, content-derived fingerprints, git recoverability classification |
| `judged-ratchet` | Baseline, diff, rot detection |
| `judged-mutants` | The 19-class catalogue, the SUT contract, the runner |
| `judged-cli` | The `judged` binary |

`judged_core::git::RecoverabilityClass` is worth reading before anything else.
Git protects the object database, not the working tree: a file that was never
`git add`-ed leaves nothing behind when you delete it. The highest-volume
targets of any cleaner — build output, caches, logs, scratch files — are exactly
the ones git cannot restore, so "gitignored" correlates with irrecoverability
rather than against it.

## Running the tests

```sh
cargo test --workspace
```

The toolchain is pinned in `rust-toolchain.toml`. Nothing is mocked in the two
places where a mock would encode our beliefs rather than reality: the git tests
shell out to the `git` binary and build real repositories, and the mutant suite
materializes real files on disk. Both need `git` on `PATH`.

CI runs the same commands plus `cargo fmt --check` and `clippy -D warnings`, and
then runs both headline commands and asserts their exit codes — including that
`--sut naive` **fails** with exit 1. A gate that cannot fail is not a gate, and
a naive cleaner passing the suite would mean the fixtures had gone soft rather
than that the cleaner had got good. There is no CI job for `--sut vulture`:
pinning a Python analyzer reproducibly enough to gate on has not been done, and
a red build nobody can fix gets muted along with everything next to it.
