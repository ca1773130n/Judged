# Four analyzers against the E2 catalogue

**Date:** 2026-08-01 · **Tools:** vulture 2.16, knip 6.31.0, `x/tools` deadcode v0.48.0, cargo-shear 1.13.3

**Result: 10 false removals across the four tools. Five of the nineteen classes are
false-removed by every tool that can read them, and a sixth — m13 — is read by none of them,
so this round measures 18 of the 19 classes the catalogue defines. One tool, cargo-shear,
clears the gate at 0 false removals with 9/9 decoy recall; it earns that inside its scope,
and its scope excludes every claim that would have been wrong.**

> **Still current, and it measures the analyzers UNPROTECTED.** Every number here is a bare
> accuser's, which is not a system anybody proposed shipping. The same four analyzers with
> §9.3's Gate 2 behind them are measured in
> [`2026-08-02-gate2-veto.md`](2026-08-02-gate2-veto.md), which re-verified this document's
> bare column byte-for-byte and is the only current source for any *gated* figure. Gate 2
> prevents 4 of the 10 false removals below, for 7 of the 26 decoys. The prediction near the
> end of this file — that a grep veto would catch the classes naming their live artifact in
> plain text — was measured: right about m01 and m14, half right about m12, where `drain` is
> rescued by the `//go:linkname` comment and `TelemetryFlush` is named nowhere outside the
> file that declares it.

This supersedes [`2026-08-01-vulture-e2-baseline.md`](2026-08-01-vulture-e2-baseline.md),
which measured one Python tool against the whole 19-class catalogue, because the runner of the
day handed every class to every analyzer. Under the current runner vulture is handed the 10
classes it can read. Read this one.
That file is kept only as the record of the first run against a shipped analyzer; every
number in it was produced by an earlier runner and none of them reproduce today.

Vulture itself did not change between the two runs — two things around it did. The decoys
now carry symbols, so vulture, a symbol-level tool, gets credit for finding them. And the
runner now skips classes outside a SUT's declared ecosystems instead of grading them, which
moves the denominator: vulture is graded on 10 of the 19 classes here, and its decoy recall
is **11 of 16** counted over those 10. The older document's 0 of 31 was counted over all
nineteen classes, including the nine vulture never opens, so the two ratios are not
comparable in either direction and neither is evidence about the other. The number that *is*
comparable, because it is counted the same way in both runs, is the false-removal count: 6,
on m01, m10, m11 and m16, unchanged.

---

## 1. What was run

| Tool | Version | How it was obtained |
| --- | --- | --- |
| vulture | `2.16` | `uv venv` + `uv pip install vulture` in a throwaway directory outside the repo, deleted afterwards |
| knip | `6.31.0` | `npx --yes knip@6`, node v24.14.0 |
| deadcode | `golang.org/x/tools v0.48.0`, built with `go1.26.2` | pre-installed at `~/go/bin/deadcode` |
| cargo-shear | `1.13.3` | pre-built from source outside the repo; it needs a newer rustc than this repository pins (1.94) |

`cargo-shear --version` prints `Version: dev` — the version string is injected at release
time and is absent from a source build. `1.13.3` is read from the binary itself
(`strings … | grep cargo-shear-`), and it matches the newer of the two `cargo-shear-*`
directories in the local cargo registry.

Exact invocations, all read-only, all at each tool's own defaults:

```
vulture                                                   <repo>
npx --yes knip@6 --reporter sarif --no-progress --directory <repo>
sh -c 'exec deadcode -json "$1/..."' deadcode              <repo>
cargo-shear --format json                                  <repo>
```

Each runs with the materialized fixture repository as its working directory and receives
that repository as its final argument. No `--fix` mode, no tuned threshold, no whitelist.
`--sut command -- vulture --min-confidence 100` exists for tuning and is a different
experiment.

---

## 2. Verbatim headlines from `judged mutants`

All four tools run through the shipped CLI and all four produce a graded result. Each block
below is the tail of `judged mutants --sut <tool>`, verbatim, with the nineteen per-class
lines above it elided. Exit codes were read from `$?` on a run with no pipe in it.

```
$ judged mutants --sut vulture
19 classes: 10 graded — 3 passed, 7 failed; 9 not read
not measured: 9 of 19 classes are outside this SUT's languages — they were never built and never handed to it, so they are in neither column above and in neither half of the decoy line below
decoy recall: 11 of 16 genuinely-dead files found
false removals: 6 — GATE FAILED (§11 R1: if this is not zero, the auto-act tier is deleted from the design rather than tuned)
classes with false removals: m01, m10, m11, m16
```

Exit code **1**.

```
$ judged mutants --sut knip
19 classes: 3 graded — 0 passed, 3 failed; 16 not read
not measured: 16 of 19 classes are outside this SUT's languages — they were never built and never handed to it, so they are in neither column above and in neither half of the decoy line below
decoy recall: 4 of 6 genuinely-dead files found
false removals: 2 — GATE FAILED (§11 R1: if this is not zero, the auto-act tier is deleted from the design rather than tuned)
classes with false removals: m02, m14
```

Exit code **1**.

```
$ judged mutants --sut deadcode
19 classes: 1 graded — 0 passed, 1 failed; 18 not read
not measured: 18 of 19 classes are outside this SUT's languages — they were never built and never handed to it, so they are in neither column above and in neither half of the decoy line below
decoy recall: 2 of 2 genuinely-dead files found
false removals: 2 — GATE FAILED (§11 R1: if this is not zero, the auto-act tier is deleted from the design rather than tuned)
classes with false removals: m12
```

Exit code **1**.

```
$ judged mutants --sut shear
19 classes: 6 graded — 6 passed, 0 failed; 13 not read
not measured: 13 of 19 classes are outside this SUT's languages — they were never built and never handed to it, so they are in neither column above and in neither half of the decoy line below
decoy recall: 9 of 9 genuinely-dead files found
false removals: 0 — GATE PASSED (§10 E2 gates releases on this number, and on nothing else)
```

Exit code **0** — the first third-party analyzer to clear the gate. §5 and §7 are where that
gets read carefully; it is a narrower result than the exit code makes it look.

### What "not read" means, and why it is a third state

A SUT declares the ecosystems it reads. `run_suite` never builds a fixture outside them, so
those classes are neither passed nor failed but **not read**, counted on their own line.
A not-read class contributes to neither half of the decoy ratio and can never be scored as a
pass, which is the property that matters: a tool cannot improve its score by being unable to
open a file.

This is why the denominators here are smaller than in any earlier write-up. Vulture's decoy
recall is **11 of 16**, where earlier versions of this document reported 11 of 31. The
numerator did not move; the 15 decoys that left the denominator are the ones planted in the
nine classes vulture is never handed. A drop in that ratio's denominator is not a change in
what any tool found, and neither ratio is evidence about the other.

Before this, `run_suite` handed every class to the analyzer, and a language-specific tool
exits non-zero on a repository in the wrong language — an exit code it shares with a genuine
analysis failure, which is exactly why it could not simply be declared healthy. `--sut knip`,
`--sut deadcode` and `--sut shear` therefore exited 2 and graded nothing, and their per-class
numbers had to be obtained from a scratch driver calling `run_suite` one class at a time
outside the repository. That is history, recorded here because an earlier version of this
document reported it as the current state. Every number in this document now comes from the
CLI.

---

## 3. A harness defect found on the way, which had silently disabled the Go row

Before this round, deadcode returned `deadcode: no packages` on **m12, the only Go class**,
and `deadcode: packages contain errors` on the other eighteen. Both are exit 1 with empty
stdout. (The other eighteen are no longer handed to it at all — they are outside its declared
ecosystem and are now *not read*. The defect below concerned m12, and would have survived the
skip feature untouched.)

The cause was in Judged, not in deadcode. `tempfile::TempDir::new()` names its directory
with the default prefix `.tmp`, so every fixture repository was handed to every analyzer as
a **hidden directory**. `go help packages` states that the Go tool ignores path elements
beginning with `.` or `_`, so the pattern `<repo>/...` matched zero packages.

Reproduced by hand against the same tree, same tool:

```
.tmpABC123/  →  deadcode: no packages          exit 1
 tmpABC123/  →  [ {"Name":"main", …} ]         exit 0
```

This is §6.20's failure occurring inside the instrument built to detect it: a tool that
scanned nothing reports nothing, nothing is zero false removals, and zero false removals is
what clears the §11 R1 gate. It cost the suite the one class §4.1 predicts deadcode fails,
and nothing in the output said so.

Fixed in `crates/judged-mutants/src/runner.rs` — the repo is now created with an explicit
non-hidden prefix — and pinned by `no_component_of_a_fixture_repo_path_is_hidden` in
`crates/judged-mutants/tests/runner_harness.rs`. The assertion is about every SUT rather
than about Go, because "a hidden working directory changes what the tool looks at" is not a
Go-specific hazard.

---

## 4. Per class, per analyzer

`FR n` = n false removals, a hard failure. `clean` = read it, claimed nothing live dead, but
missed at least one decoy. `pass` = clean *and* full decoy recall. `—` = **not read**: the
class is outside the tool's declared ecosystems, so its fixture was never built and never
handed to the tool. `d` = decoys found / decoys planted, counted only where the tool read.

| # | Eco | Mechanism | vulture | knip | deadcode | cargo-shear |
| --- | --- | --- | --- | --- | --- | --- |
| m01 | python | dotted class path only in a YAML app list | **FR 1** `DunningConfig` d2/2 | — | — | — |
| m02 | polyglot | module name computed at runtime, `importlib`/`require` | clean d1/2 | **FR 1** `src/transports/websocketTransport.ts` d1/2 | — | — |
| m03 | python | plugin found by directory scan | **pass** d1/1 | — | — | — |
| m04 | rust | subcommand reachable only when a human types it | — | — | — | **pass** d1/1 |
| m05 | python | recovery handler on the failure path | **pass** d2/2 | — | — | — |
| m06 | rust | lock helper used only under contention | — | — | — | **pass** d2/2 |
| m07 | rust | guard clause inert until input is hostile | — | — | — | **pass** d2/2 |
| m08 | polyglot | script invoked only from CI / Dockerfile / k8s | clean d0/2 | — | — | — |
| m09 | rust | API exercised only by a README example CI runs | — | — | — | **pass** d2/2 |
| m10 | polyglot | framework convention: AppConfig, `__mocks__` | **FR 1** `ReportingConfig` d1/2 | clean d1/2 | — | — |
| m11 | python | model field enumerated reflectively | **FR 3** `legal_hold_until`, `retention_days`, `tenant_slug` d1/1 | — | — | — |
| m12 | go | symbol bound through `//go:linkname` | — | — | **FR 2** `TelemetryFlush`, `drain` d2/2 | — |
| m13 | polyglot | file rescued by an explicit `!` negation | — | — | — | — |
| m14 | typescript | committed build output consumed by a CDN path | — | **FR 1** `dist/widget.7f3a91c.js` d2/2 | — | — |
| m15 | python | worker named only in a queued job payload | **pass** d1/1 | — | — | — |
| m16 | python | type named only in a pickled blob | **FR 1** `RateSnapshot` d1/1 | — | — | — |
| m17 | rust | `inventory::submit!`, empty call graph | — | — | — | **pass** d1/1 |
| m18 | polyglot | entry point only in a platform manifest | clean d1/2 | — | — | — |
| m19 | rust | `#[no_mangle]` export consumed outside the repo | — | — | — | **pass** d1/1 |

**m13 is the one row with no entry in any column.** No analyzer here reads it. §5 and §7
return to that.

| | vulture | knip | deadcode | cargo-shear |
| --- | --- | --- | --- | --- |
| classes graded | 10 | 3 | 1 | 6 |
| classes not read | 9 | 16 | 18 | 13 |
| **false removals** | **6** | **2** | **2** | **0** |
| classes with a false removal | m01, m10, m11, m16 | m02, m14 | m12 | none |
| decoy recall over graded classes | 11/16 | 4/6 | 2/2 | 9/9 |
| passed | 3 | 0 | 0 | 6 |
| exit code | 1 | 1 | 1 | **0** |

Both tables are read from `judged mutants --sut <tool> --json`, which reports the grade, the
per-class decoy pair and the false-removal list for every class.

### The evidence behind the three non-Python rows

The blocks below are each tool's own output, captured from the tool rather than from Judged.
What the CLI independently confirms is the graded outcome — which live artifacts were claimed
dead, and how many decoys were found — and it agrees with every one of them.

**deadcode on m12** — the `deadcode -json` findings, rendered by hand for legibility (this
block is a summary, not verbatim stdout):

```
PKG example.com/m12/telemetry/cmd/libtelemetry
   DEAD: TelemetryFlush @ cmd/libtelemetry/abi.go 18     ← LIVE (cgo //export)
   DEAD: runtime_throw  @ cmd/libtelemetry/abi.go 27
   DEAD: _cgo_cmalloc   @ cmd/libtelemetry/abi.go 30
PKG example.com/m12/telemetry/internal/collector
   DEAD: unusedPercentile @ internal/collector/unused_percentile.go 5   ← decoy, correct
PKG example.com/m12/telemetry/internal/sampler
   DEAD: drain           @ internal/sampler/drain.go 9   ← LIVE (//go:linkname)
   DEAD: legacyHistogram @ internal/sampler/legacy_histogram.go 6       ← decoy, correct
```

Both decoys found, and both live symbols claimed dead. This is the tool doing exactly what
its own `-help` text says it will do, on the exact shape the fixture is built from.

**knip on m14** — the live artifact is `dist/widget.7f3a91c.js`, whose only consumer is
`<script src="/dist/widget.7f3a91c.js" defer></script>` in `public/index.html`. At default
configuration knip reports:

```
knip/files -> dist/widget.0c9e142.js     ← decoy, correct
knip/files -> dist/widget.7f3a91c.js     ← LIVE
knip/files -> src/unusedFeatureFlags.ts  ← decoy, correct
```

**cargo-shear on m17 and m19** — it is not silent on either class. It emits a correct
finding on each:

```
m17: shear/unlinked_files  "1 unlinked file in `schema-migrator`\nsrc/checksum_v1.rs"   help: delete this file
m19: shear/unlinked_files  "1 unlinked file in `ledger-abi`\nsrc/deprecated_rounding.rs" help: delete this file
```

Both name the decoy and neither names the live file. But the reason is not that cargo-shear
understands `inventory::submit!` or `#[no_mangle]`: the live files are reachable by `mod`
declaration (`mod migrations;` → `mod m0007;`, and `mod ffi;`), and the decoys are not
declared by any `mod`. cargo-shear checks module linkage and unused manifest dependencies.
It never asks whether a symbol has callers, and `backfill_missing_avatars` has none —
"none we found" is not the situation; none exist. **Its pass on the two hardest Rust classes
is a fact about its capability envelope, not about the mechanisms.**

---

## 5. Across all four

### Is there a class no tool survives?

Yes — **five**, taking "survives" as *some tool graded it and claimed nothing live dead*:

**m01** (YAML string reference), **m11** (reflective model field), **m12** (`//go:linkname`),
**m14** (committed build output), **m16** (pickled blob).

On each of these, exactly one tool could read the class and that tool false-removed on it.
Nothing here is redundancy: with four tools spanning four ecosystems, most classes have
exactly one possible reader, so "no tool survives" and "the only reader failed" are the same
sentence for m01, m11, m12, m14 and m16.

And **m13 is not survived either, for a different and worse reason: no tool reads it.** It is
the only one of the nineteen with an empty row. Its live artifact is PHP — rescued from a
broad ignore rule by an explicit `!` negation, beside a `composer.json` and a checked-in
media file — and none of the four adapters covers PHP. m13 is not a class the analyzers
failed; it is a class they were never asked about.

A stricter reading tightens this further. One class has a tool that stayed clean but found
**zero** of the decoys planted there: vulture on **m08**, 0/2. A tool that found nothing dead
in a repository containing two genuinely-dead files has not demonstrated it read anything,
and its silence is not evidence (that is the first clause of vulture's own capability
envelope). Counting m08 alongside m13 gives **seven of nineteen** classes with no result that
means anything: m01, m08, m11, m12, m13, m14, m16.

### Is there a tool with zero false removals across the classes it can read?

**Yes: cargo-shear. 0 false removals across the 6 Rust classes it grades, with 9/9 decoy
recall, and `judged mutants --sut shear` exits 0 — GATE PASSED.** It is the first
third-party analyzer to clear the gate, and the 9/9 is what makes that worth something: it is
not the refusing-SUT degenerate case, because it demonstrably found every genuinely-dead file
planted in front of it. Within its scope that is competence, and it should be credited as
competence.

It should also not be read as "cargo-shear is the safest of the four". The gate is a
per-tool result over whatever that tool read, and cargo-shear read 6 of 19 classes. It is not
being compared with vulture over the same 19 classes; there is no class that both of them
grade. Read narrowly, then. cargo-shear
answers two questions — is a declared dependency unused, and is a file unreachable by `mod`
declaration — and neither question can produce the claim that fails m04, m06, m07, m09, m17
or m19. It is not that it evaluated the guard clause, the contention path, the README
example, the registry and the ABI export and judged them live; it never formed an opinion
about any of them. **A tool scores zero false removals on this catalogue either by being
right about reachability or by never claiming anything about reachability, and E2's own
decoy mechanism does not separate those two.** The decoys separate "found nothing" from
"found something"; they do not separate "found the right kind of something".

### What this does and does not say about §11 R1

§11 R1 pre-commits that **if no signal combination clears the catalogue at zero false
removals, the auto-act tier is deleted from the design rather than tuned.**

What the evidence supports:

- No single tool here clears the catalogue. The union of all four grades **18 of the 19
  classes** (vulture 10 + knip's m14 + deadcode's m12 + shear's 6; m13 is read by none of
  them), and that union carries **10 false removals across 7 distinct classes** (m01, m02,
  m10, m11, m12, m14, m16). Combining signals by union of claims cannot
  reduce a false removal — every claim any member makes is a claim the union makes — so
  **no subset of these four clears even the 18 it can reach at zero false removals.** The only
  zero-false-removal subset is `{cargo-shear}`, which reads 6 of 19 — leaving all thirteen
  classes it does not read, among them every class another tool false-removed (m01, m02, m10,
  m11, m12, m14, m16), with no reader at all. Whatever the union does, m13 stays unmeasured.
- The one tool with a clean sheet is clean because of what it structurally cannot say. That
  is evidence *against* the hypothesis that precision comes free from picking a better
  analyzer, and it is the second time this catalogue has produced that shape: the same
  sentence was true of vulture's zeroes on the Rust classes before this round could read them.

What the evidence does **not** support:

- **Four off-the-shelf analyzers is not "no signal combination", and this document does not
  discharge the pre-commitment.** Every tool here is a single-signal static analyzer run at
  its defaults. The design's signal set as described is broader — grep veto over
  non-source artifacts, config and manifest parsing, coverage or runtime traces, VCS
  history, an explicit keep-list. None of those were measured, and at least three of the
  five unsurvived classes (m01 a YAML string, m14 an HTML `<script src>`, m12 a compiler
  directive) name their live artifact in plain text somewhere in the repository, which is
  exactly what a grep veto exists to catch.
- Nothing here measures whether a *combination* would clear the catalogue, because no
  combination was run. The union argument above is arithmetic on claims already collected,
  not an experiment.
- **The catalogue was not fully measured.** m13 has no reader, so R1's condition is being
  evaluated against 18 of the 19 classes it is defined over. That cuts against a clean
  verdict in either direction.

The honest summary: **this round supplies no counterexample to R1's deletion condition and
several results consistent with it, on the strongest evidence the project has so far. It is
not yet the exhaustive search R1's wording requires.** The next thing that would move R1 is
a signal that is not a language-specific dead-code analyzer.

---

## 6. Prediction scorecard

| # | Prediction | Verdict |
| --- | --- | --- |
| 1 | deadcode false-removes on m12 (`//go:linkname`), per §4.1 | **CONFIRMED, and stronger than stated** |
| 2 | knip is configuration-sensitive and degrades rather than being wrong, per §6.20 and its FAQ | **SPLIT — first half CONFIRMED, second half REFUTED** |
| 3 | shear says nothing about m17/m19, which is not the same as passing them | **CONFIRMED in substance, REFUTED in the literal claim** |
| 4 | vulture's decoy recall is no longer 0 now that decoys carry symbols | **CONFIRMED** |

**1 — CONFIRMED.** deadcode claims `drain` dead, which is bound through `//go:linkname`,
which is the documented failure the prediction names. It also claims `TelemetryFlush` dead —
a cgo `//export` — which §4.1 does not separately predict. Two false removals where the
prediction implies one. Note that this was only measurable after the §3 harness fix; on the
code as it stood, deadcode could not read m12 at all.

**2 — SPLIT.** *Configuration-sensitive:* confirmed sharply. The same repository, same tool,
same version, four configurations:

*Provenance:* this sweep was run by hand against a scratch copy of the m14 fixture, varying
`knip.json` between runs. Only the first row is re-derivable from a shipped command
(`judged mutants --sut knip`, which uses knip's defaults and false-removes
`dist/widget.7f3a91c.js` on m14); the other four rows are recorded as observed and cannot
be reproduced from this repository as it stands. Treat them as supporting detail, not as a
measurement anyone can check.

| Config | files claimed unused |
| --- | --- |
| default (no `knip.json`) | `dist/widget.0c9e142.js`, **`dist/widget.7f3a91c.js`**, `src/unusedFeatureFlags.ts` |
| `entry: [src/main.ts, public/index.html]`, `project: [src/**, dist/**]` | `dist/widget.0c9e142.js`, **`dist/widget.7f3a91c.js`**, `src/unusedFeatureFlags.ts` |
| `entry: [src/main.ts, public/index.html]`, `project: [src/**, dist/**, public/**]` | `dist/widget.0c9e142.js`, **`dist/widget.7f3a91c.js`**, `src/unusedFeatureFlags.ts` |
| `entry: [src/main.ts, dist/*.js]` | `src/unusedFeatureFlags.ts` |
| `entry: [src/main.ts]`, `project: [src/**/*.ts]` | `src/unusedFeatureFlags.ts` |

*Degrades rather than being wrong:* refuted where it matters. knip does degrade loudly when
it cannot start at all: handed one of the 16 classes with no `package.json` it exits 2 with
`ERROR: Unable to find package.json`, unambiguous and correct — that is what the suite saw
before it stopped handing knip those classes, and it is why knip's refusal could not be
waved through as a clean run. But on all three classes it **can** read it produced a
confident SARIF log, and on two of them that log named a live file dead, at exit 1, with
nothing marking the answer as degraded. Loud refusal outside its ecosystem buys nothing
inside it.

The configuration sweep adds a finding the prediction does not anticipate and which is worse
than either half of it: **no configuration tested both keeps the live file and finds the dead
one.** Declaring `dist/*.js` an entry rescues `widget.7f3a91c.js` and simultaneously loses
`widget.0c9e142.js`, which is genuinely dead — decoy recall on m14 drops 2/2 → 1/2.
Configuration moves the error between a false positive and a false negative rather than
removing it. Declaring `public/index.html` an entry changes nothing at all: knip does not
follow `<script src>` from an HTML entry to a committed bundle.

**3 — CONFIRMED in substance, REFUTED literally.** The substance — a pass on m17/m19 is not
evidence the mechanism was understood — is exactly right, and §4 shows why: the live files
are `mod`-declared and the decoys are not, which is the whole of what cargo-shear checked.
But the literal claim is wrong: cargo-shear does **not** say nothing about m17 and m19. It
emits a `shear/unlinked_files` finding on each, correctly naming the decoy, with
`help: delete this file`. The distinction matters because "silent" and "correct about
something else" grade identically here and are different tools to trust.

**4 — CONFIRMED.** Vulture now finds 11 decoys where it previously found none: **11 of 16**,
counted over the 10 classes it grades. Vulture is unchanged; the decoys carry symbols now and
vulture is a symbol-level tool. All 11 are in classes with Python decoys.

Note what is *not* being compared. The superseded baseline's 0 of 31 counted every decoy in
the catalogue, including the 15 planted in the nine classes vulture is never handed; the 16
here counts only the decoys it was actually shown. The two ratios have different denominators
and putting them side by side would overstate the change. What settles the prediction is the
numerator, which is counted identically in both: 0 became 11.

---

## 7. LIMITS

Read this section before quoting any number above.

1. **One of the nineteen classes has no analyzer coverage at all.** m13 — a PHP file rescued
   from a broad ignore rule by an explicit `!` negation, beside a `composer.json` and a
   checked-in media asset — is read by none of the four tools, because no adapter here covers
   PHP. Every other class is graded by at least one of them. **This round therefore measures
   18 of the 19 mechanisms the catalogue defines**, and nothing above — which classes survive,
   what a union of the four would do, what any of it bears on R1 — extends to m13. That is a
   gap in the evaluation rather than a footnote to it, and closing it needs a PHP adapter, not
   a rerun of these four.

2. **"Not read" is most of this table.** Of the 76 tool×class cells, only **20 are graded
   results**; the other **56 were never read** (vulture 9, knip 16, deadcode 18,
   cargo-shear 13), because the class falls outside that tool's declared ecosystems and its
   fixture was never built. Every per-tool false-removal count is a count over a small,
   language-determined subset: deadcode's entire result is one class.

3. **A zero is not a pass, and an exit 0 is not a clean bill of health.** cargo-shear's 0
   false removals over 6 classes, and vulture's clean cell on m08, are produced by tools that
   could not make the failing claim. E2 grades what a tool claims; it does not grade what a
   tool declined to think about. The capability envelopes exist precisely to stop a zero being
   read as competence, and they should be read alongside every row here — most of all the row
   that passed the gate.

4. **Single configuration, single platform, single point in time.** darwin/arm64, one
   GOOS/GOARCH — deadcode's own help text notes its analysis is valid for one build
   configuration and a function dead in one may be live in another. Tool versions are pinned
   above and all four move.

5. **cargo-shear is a source build reporting `Version: dev`.** The `1.13.3` attribution is
   inferred from the binary and the local registry, not from the tool's own output. It also
   reached the network (`Updating crates.io index`) on each of the six Rust classes; the runs
   are not hermetic.

6. **The decoys measure that a tool found *something* dead, not that it looked for the right
   thing.** This is the load-bearing limitation for §5's second question and it is a property
   of the catalogue, not of this run.

7. **`n=1` per class.** Each class is one fixture exercising one mechanism, hand-built. A
   tool that fails m14 fails this construction of "committed build output consumed by a CDN
   path"; the generalization to the class is an argument, not a measurement.

8. **The knip configuration sweep is not part of the graded run.** It was done on a scratch
   copy to test prediction 2. No fixture in the repository was modified, and the graded knip
   numbers are all at default configuration.

9. **Vulture's 6 is a lower bound, not a total.** The adapter maps vulture's findings to
   symbol claims and never to file claims, because vulture reports names and never names a
   file — its `MAPPING_DECISION`, printed above every run, states this and calls the resulting
   count a lower bound. A finding that lands inside a file the ground truth declares LIVE goes
   ungraded whenever the class's live artifact is a file rather than a symbol, and
   `unreachable code after 'return'` produces no claim at all because it names a keyword. The
   superseded baseline works through where that happens, class by class. No equivalent
   analysis exists for the other three tools.
