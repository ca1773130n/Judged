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
argues the product is the layers around those analyzers, and that three of them
should exist before anything is allowed to delete a byte: something that stops
new findings without demanding the backlog be fixed, something that measures how
often an analyzer is wrong about live code, and something that writes down the
root set a human can audit instead of one a tool inferred.

Judged deletes nothing. Not by default, not behind a flag. There is no `--fix`,
and passing one is refused before the subcommand is even parsed. The ratchet's
only power is to fail a build; the mutant suite writes exclusively to throwaway
directories. That is the design (§9.14: the ratchet has zero deletion risk), not
a milestone that hasn't landed yet.

## Status: there is no auto-act tier, and none is assumed to be coming

Read this before the feature list, because it decides what the features are for.

**Judged reports. It does not act.** Whether an auto-act tier — anything allowed
to delete a byte without a human in the loop — may exist at all is the single
highest-risk open question in the research (§11 R1), and the answer is
pre-committed in both directions: if no signal combination clears the mutation
catalogue at zero false removals, the tier is **deleted from the design rather
than tuned**, and the honest product is report and quarantine.

That question is **not yet testable**, so the pre-commitment is **not triggered**
and the tier is **presumed absent until proven otherwise**. Best measured
configuration to date — four real analyzers behind all three rescue layers that
exist — still calls **5 live artifacts dead, across 3 of the 19 classes**. But
the reason the question is untestable is more basic than that number: §9.5 makes
an auto-act tier formally unreachable without a signal that observes execution,
and **nothing in this project observes execution**. Every signal here reads
repository text. None of the configurations measured so far could have produced
an auto-act decision even at zero false removals.

So the posture is the one that costs nothing if the tier eventually clears and
everything if it does not. Nothing ships that auto-acts. Nothing downstream gets
built assuming the tier arrives. §11 R1 budgets for exactly this outcome and
calls report+quarantine the honest product, not a shortfall.

To be exact about what "today" means: **Judged today is report only.** The
quarantine half of report+quarantine is not built either — no ledger, no
stability window, no reaping — and §11 R1 names those as the downstream that
"assumes the answer is yes". They should be written to survive the tier's
deletion.

The full determination — every number with the command that produced it,
inference labelled as inference, and the specific evidence that would reverse the
call in either direction — is
[`docs/decisions/2026-08-02-r1-determination.md`](docs/decisions/2026-08-02-r1-determination.md).

## The commands

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
judged mutants --sut naive|refusing|vulture|knip|deadcode|shear [--veto [--needles <strategy>]] [--json]
judged mutants --sut command [--veto] [--json] -- <analyzer> [args...]
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

### `judged show-roots` — print the root set, and what could not be resolved

```sh
judged show-roots [--json] [<path>]
```

ProGuard's `-printseeds` and Nix's `--print-roots`, which §9.13 asks for by
name. It decides nothing and deletes nothing: it materializes what was
*declared* — with the file and the key that declared it, so a human can check
each line against the repository before anything acts on it. §1.2 is the reason
the command has this shape rather than a better one: you cannot infer the closed
world, you can only have it declared.

Roots come in three provenance tiers, and the tier travels with every root
because a caller that cannot tell them apart will trust a guessed convention as
though a manifest had declared it:

| Tier | Where it comes from | Confidence |
| --- | --- | --- |
| **A — machine-declared** | A build system already reads this file to find roots: `Cargo.toml [[bin]]`, `package.json` exports/bin/scripts, `go.mod`, `pyproject`, `Dockerfile` CMD/ENTRYPOINT, GitHub workflow `run`/`uses` | High |
| **B — convention-inferable** | A framework's file layout turns a file into an entry point with no source reference: Django `INSTALLED_APPS` and management commands, pytest `test_*.py` | Medium — correct only if framework **and version** were detected correctly |
| **C — human-declared** | Solicited from a person and committed | Whatever the person knew |

The other half of the output is the half that gets dropped everywhere else: every
framework recognized with no plugin behind it, every manifest that would not
parse, every declared entry that matched nothing. §6.20 requires "no data" to be
a distinct state from "zero", and a report printing only successes renders a
framework whose entire convention is missing identically to one that genuinely
has no roots. Both counts are always printed, and `--json` always carries `gaps`
beside `roots`.

Exit 0 whenever a root set was materialized, whatever it contains — a repository
with few roots is a normal repository. Exit 2 only when nothing could be read at
all, because zero roots over zero files is the absence of a scan wearing the
digits of an empty repository.

Measured out of sample on nine real repositories in
[`docs/evals/2026-08-02-out-of-sample-corpus.md`](docs/evals/2026-08-02-out-of-sample-corpus.md):
854 roots, 787 of them Tier A, worst case 15.9 ms over a 574-file repository. That
document is also where the layer's current defects are recorded, and they are not
small — 99 of those roots name a path that does not exist. Read it before
trusting the output.

### `judged explain` — why one path is or is not eligible, gate by gate

```sh
judged explain [--json] <path>
```

§9.13 asks for this alongside `show-roots`. It prints the gates in the order
§9.3 evaluates them: recoverability first (Gate 0g — what git could give back,
and at which rung), then Gate 1, the never-touch inventory, with every class that
refused the path and the rule that did it.

The ordering is the point rather than a convention. Usefulness is irrelevant
until recoverability is known, because the cost of being wrong is set by the rung
and not by the tier. A Gate 1 refusal is **absorbing and justified by
irreversibility, not by uselessness** — no later evidence that a file is unused
moves it, and equally, a refusal is not a claim that the file is used.

The command never says a path is safe to delete, and it ends by naming the gates
it did *not* run (0a–0f, 2, 3). §6.20 applies to its own output: a trace that
silently omits a gate is indistinguishable from one in which that gate abstained.

What Gate 1 protects, and how often it is wrong, is measured on 3,751 files
across nine real repositories in
[`docs/evals/2026-08-02-gate1-corpus.md`](docs/evals/2026-08-02-gate1-corpus.md).
Read it before trusting this output too: Gate 1 currently refuses **28.4%** of
tracked files against §6.17's 3.6% baseline for "explicitly irreplaceable", and a
47-row hand check found **17 wrong**, all from one sub-rule that reads an SPDX
header as making its whole file a compliance artifact.

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

### `--veto`: the same four analyzers, with Gate 2 behind them

Those ten false removals are what an analyzer does **unprotected**, and no
architecture in the research proposes shipping one that way. §9.1 orchestrates
analyzers as bounded accusers with a veto layer behind them, and §11 R1 asks
whether any signal *combination* clears the catalogue — not whether any tool
does. `--veto` runs §9.3's Gate 2 over every claim the analyzer makes. It can
only ever **rescue**; a veto is absorbing, no later evidence overrides it, and
nothing in the layer can cause a candidate to be claimed dead. Because the
number that matters is the difference, the suite runs twice — once bare, once
gated — and prints both halves of the trade.

| SUT | False removals bare → gated | Decoy recall bare → gated |
| --- | --- | --- |
| vulture 2.16 | 6 → **4** | 11/16 → 10/16 |
| knip 6.31.0 | 2 → **1** | 4/6 → 2/6 |
| deadcode v0.48.0 | 2 → **1** | 2/2 → 2/2 |
| cargo-shear 1.13.3 | 0 → 0 | 9/9 → 5/9 |
| all four | **10 → 6** | 26/29 → 19/29 |

**Gate 2 prevents 4 of the 10 false removals at the shipped needle strategy, for
7 of the 26 decoys the four tools recover.** Three of the seven affected classes
are cleared; four still remove something live. Every rescue is cross-file: the
gate excludes a claim's own file before searching, so a symbol found only in its
own declaration is not rescued by it — **provided the analyzer said where it was
declared.** When a tool names a symbol without attributing it to a file, there is
nothing to exclude and the symbol is still rescued by its own declaration. That
is deliberate (a gate that may only rescue is allowed to err toward rescuing) and
it is a real remaining cost, not a closed defect: every such claim is a decoy the
suite will never see found. The four are m01 (a dotted class path in
`apps.yaml`), m12's `drain` (a `//go:linkname` comment), m16 (a class name inside
a committed pickle — binaries are searched) and m14 (`widget.7f3a91c.js` in
`public/index.html`).

The six that survive are the more useful half of the result. Five of them —
`ReportingConfig`, three reflection-touched model fields and a `//export`ed ABI
symbol — are named **nowhere outside the file that declares them**. A whole-repo
literal search rescues what is written down twice and cannot rescue what is
written down once, and no needle setting changes that. The sixth, a module reached
by a specifier assembled at runtime, is reachable only through the directory
needle, at the cost the sweep measures.

The `all four` row's denominator is 29 distinct decoy files, not the 33 you get by
summing the per-tool columns: m02 and m10 are each graded by two analyzers, which
counts their four decoys twice.

The §11 R8 needle sweep is in the same document and is reproducible with
`--needles`. `basename+stem` is what ships and is the narrowest strategy meeting
the **rescue** half of R8's criterion; `basename` alone fails it. `+parent-dir` is
the only strategy that reaches the tenth false removal, and it buys it by also
blocking on the words `src` and `dist` — taking knip's decoy recall to zero. Full
write-up, per-class needle trace, sweep and limits in
[`docs/evals/2026-08-02-gate2-veto.md`](docs/evals/2026-08-02-gate2-veto.md).

**R8's other half — a tolerable flag rate — is not met by any strategy measured
here.** R8 sets no numeric threshold for "tolerable", so this is a measurement
and not a verdict against the criterion. On nine real repositories, over the population an
analyzer actually claims, the shipped `basename+stem` blocks a median **84.6%** of
candidates and `+parent-dir` blocks **100% on seven of the nine**. Even `basename`
alone, the part §9.3 makes structurally impossible to remove, blocks a median
27.3%. R8 never writes down what "tolerable" means, so no measurement can settle
it; what the measurement does establish is that R8's space contains no
low-flag-rate strategy to retreat to, which is not what R8 assumed when it framed
the question as a choice of needle set.
[`docs/evals/2026-08-02-out-of-sample-corpus.md`](docs/evals/2026-08-02-out-of-sample-corpus.md).

That document also records a defect worth reading on its own account. Until it was
fixed, `SutVerdict` carried symbol claims as bare names with no location, so Gate
2a had no declaring file to exclude, found every symbol in its own declaration,
and rescued **every symbol claim in the suite** — vulture 11/16 decoys to 0/16,
deadcode 2/2 to 0/2, both printing `GATE PASSED` by claiming nothing. A veto that
fires on every input is a constant function, and a constant function measures
nothing. It survived a full review because `--veto --json` printed only where a
string was found and not where it was declared, so a genuine cross-file rescue and
a tautology looked identical. Both fields are emitted now.

### The full stack: Gate 1, Gate 2 and the root set together

Three rescue layers now exist, and each composes independently so all eight
combinations are measurable. `--gate1` runs §9.3's Gate 1, the never-touch
inventory, ahead of everything else. Four analyzers, aggregated:

| Configuration | False removals | Classes failing | Decoys |
| --- | ---: | ---: | --- |
| bare | **10** | 7 | 26/33 |
| `--gate1` | 9 | 6 | 25/33 |
| `--veto` | 6 | 4 | 19/33 |
| `--roots` | 9 | 6 | 26/33 |
| `--veto --roots` | **5** | 3 | 19/33 |
| `--gate1 --veto --roots` | **5** | 3 | 19/33 |

**Gate 1's marginal contribution is exactly zero** — the last two rows are
identical in every column, and per analyzer the counterfactual is 3→3, 1→1, 1→1,
0→0. That is the expected result and not a defect. Gate 1 refuses on
**irreversibility**, not on usefulness: it makes a wrong answer cheaper, it does
not make the answer more correct, so it cannot clear a class that a reference
analyzer gets wrong. §10 E2's nineteen classes exercise *reference* mechanisms,
which is Gate 2's domain; the catalogue contains no `.env`, no
`terraform.tfstate` and no analyst's `.RData`, so it cannot measure the layer
that exists for them.

Where Gate 1 *did* move something, the reason matters more than the number. Run
alone it clears knip's m14 — and the trace shows why that is not a result:

```
  m14  FAIL  typescript  0 false  1/2 decoys  committed build output whose only consumer is a CDN path
       gate1 rescued live: dist/widget.7f3a91c.js   [§10 E2 class 14]
       the stack also rescued 1 genuinely-dead decoy file(s) — the price
       [gate1/1j] rescued path dist/widget.0c9e142.js — 1j vendored, generated, submodule or LFS-tracked: matches GitHub Linguist vendor.yml `(^|/)dist/`, so it is not this repository's code
       [gate1/1j] rescued path dist/widget.7f3a91c.js — 1j vendored, generated, submodule or LFS-tracked: matches GitHub Linguist vendor.yml `(^|/)dist/`, so it is not this repository's code
```

`…7f3a91c.js` is the live asset; `…0c9e142.js` is the planted decoy, genuinely
dead. The justification is byte-identical. What makes the live one live is a
`<script src=…>` in `public/index.html`, which Gate 1 never read — it matched a
directory pattern and refused both. One false removal prevented, one decoy
destroyed, 1:1, and the layer cannot tell you which it just did. **A rescue like
that is a coincidence of shape, not evidence that anything got better at
identifying dead code.** The fixture predicted it: *"a tool that roots all of
`dist/` is safe and scores zero decoy recall."* Gate 2 clears m14 too, by finding
the filename in the HTML — a rescue that *is* connected to the liveness.

**Five false removals survive, in three classes, and Gate 1 fired on none of
them:** m02's runtime-computed module specifier (knip), m11's three reflectively
enumerated model fields (vulture), m12's `//go:linkname` alias (deadcode). All
sixteen Gate 1 classes were asked about all three and none refused a single
claim.

> **Read every number here as an upper bound.** The evaluation is **in-sample**:
> the rescue vocabulary of all three layers and the 19 classes come from the same
> research document, so it is graded on the exam it studied for.

None of this discharges §11 R1 — it points the other way, and Gate 1's arrival
did not change that in either direction. Five false removals remain across three
classes; no measured combination clears the **18 of 19 classes any analyzer here
can read** (m13 is PHP and nothing reads it); and every signal in the stack reads
repository text, so §9.5's two-family quorum is unsatisfiable and Tier 0 is
formally unreachable regardless of the count. The determination, with what would
reverse it, is
[`docs/decisions/2026-08-02-r1-determination.md`](docs/decisions/2026-08-02-r1-determination.md).
Gate 1 measured on its own terms — 3,751 files across nine real repositories,
where it protects 28.4% of them and a hand check finds 17 of 47 protections wrong
— is
[`docs/evals/2026-08-02-gate1-corpus.md`](docs/evals/2026-08-02-gate1-corpus.md).

## Layout

| Crate | What it holds |
| --- | --- |
| `judged-core` | The SARIF 2.1.0 subset adapters are held to, content-derived fingerprints, git recoverability classification, Gate 1's sixteen never-touch classes under `gate1/`, Gate 2's vetoes, and the Tier A/B/C root readers under `roots/` |
| `judged-ratchet` | Baseline, diff, rot detection |
| `judged-mutants` | The 19-class catalogue, the SUT contract, the runner, and the root-set materializer that assembles the three tiers |
| `judged-cli` | The `judged` binary: `ratchet`, `mutants`, `show-roots`, `explain` |

The root readers under `judged-core/src/roots/` parse other people's file
formats, so they use those ecosystems' own parsers — `toml` (toml-rs, what Cargo
parses manifests with) and `saphyr-parser` for YAML — rather than subsets of
them. That is not a style preference. Hand-written subsets shipped first, and a
parser for a *subset* of a format is a list of valid files you reject: measured
out of sample, they emptied the Tier A root set of 7 of 9 real repositories.

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
