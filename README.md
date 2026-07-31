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

Those two are what this workspace currently contains.

## The ratchet (`judged-ratchet`)

Baseline the findings a repository already has, then fail CI only on new ones.
Nothing gets deleted and nothing needs configuring. The prior art is Shopify's
`deprecation_toolkit`, which worked because it never demanded the backlog be
fixed first. The crate is structurally incapable of touching the working tree;
its only power is to fail a build, and it refuses to do even that when the
analyzer run behind it was degraded.

The failure mode it has to survive is baselines rotting into a permanent amnesty
list, which is what `judged-ratchet`'s rot detection is for.

## The E2 suite (`judged-mutants`)

Nineteen mutants, each injecting a known-live artifact reachable through exactly
one mechanism: a class named only in a YAML string, a plugin found by scanning a
directory, a `//go:linkname` alias, a worker named only in a job payload that was
already enqueued. Any "dead" verdict on one of them is a hard failure.

The suite carries its own controls. `NaiveSut` is a deliberately bad cleaner
that must fail; if it ever passes, the suite is theatre. `RefusingSut` claims
nothing is dead and must also fail, on the genuinely-dead decoy files every
mutant plants, because a tool that never speaks is not safe, it is useless.

The point of running it early is that the answer is pre-committed: if no signal
combination clears the catalogue at zero false removals, the auto-delete tier
gets deleted from the design rather than tuned.

## Layout

| Crate | What it holds |
| --- | --- |
| `judged-core` | The SARIF 2.1.0 subset adapters are held to, content-derived fingerprints, and git recoverability classification |
| `judged-ratchet` | Baseline, diff, rot detection |
| `judged-mutants` | The 19-class catalogue, the SUT contract, the runner |
| `judged-cli` | The `judged` binary |

`judged-core::git::RecoverabilityClass` is worth reading before anything else.
Git protects the object database, not the working tree: a file that was never
`git add`-ed leaves nothing behind when you delete it. The highest-volume
targets of any cleaner — build output, caches, logs, scratch files — are exactly
the ones git cannot restore, so "gitignored" correlates with irrecoverability
rather than against it.

## Running the tests

```sh
cargo test --workspace
```

The toolchain is pinned in `rust-toolchain.toml`. Most of the public API is
still `todo!()`; the tests that exist cover the parts that are real, which today
means the on-disk formats and the shape of the catalogue.
