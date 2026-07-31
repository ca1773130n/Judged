# Judged

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
judged mutants --sut naive|refusing [--json]
```

Nineteen mutants, each materialized into a temporary repository, each planting
an artifact that is genuinely live but reachable through exactly one mechanism:
a class named only in a YAML string, a plugin found by scanning a directory, a
`//go:linkname` alias, a worker named only in a job payload already sitting in
the queue. Any "dead" verdict on one of them is a false removal, and the gate is
zero false removals, nothing else. Every mutant also plants genuinely-dead
decoys, so a tool that refuses to answer cannot score a perfect run.

Exit 0 only when the false-removal count is zero; 1 otherwise.

## The headline result

Both systems under test are controls that ship with the suite. The summary lines
below are verbatim; the nineteen per-class lines above each are elided.

```
$ judged mutants --sut refusing
19 classes: 0 passed, 19 failed
decoy recall: 0 of 31 genuinely-dead files found
false removals: 0 — GATE PASSED (§10 E2 gates releases on this number, and on nothing else)
note: this SUT removed nothing at all, so it cleared the gate without demonstrating it can find anything. Zero false removals is also the score of a tool that refuses to answer.
$ echo $?
0

$ judged mutants --sut naive
19 classes: 7 passed, 12 failed
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
its 19 classes are discriminating enough to separate them. It says nothing about
knip, vulture, or any shipped tool, because none has been graded yet. E2
answering R1 needs a real signal combination run through it.

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
