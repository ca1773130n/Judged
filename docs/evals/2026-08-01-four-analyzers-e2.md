# Four analyzers against the E2 catalogue

**Date:** 2026-08-01 · **Tools:** vulture 2.16, knip 6.31.0, `x/tools` deadcode v0.48.0, cargo-shear 1.13.3

**Result: 10 false removals across the four tools. Five of the nineteen classes are
false-removed by every tool that can read them. One tool — cargo-shear — has a clean sheet,
and the reason it is clean is that it cannot make the claim that would be wrong.**

This supersedes [`2026-08-01-vulture-e2-baseline.md`](2026-08-01-vulture-e2-baseline.md),
which measured one Python tool against a catalogue that is 7/19 not-Python. Read this one.
The vulture numbers here differ from that document in one respect — decoy recall is 11/31
rather than 0/31 — because the decoys now carry symbols; nothing about vulture changed.

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

## 2. Verbatim headlines from `judged mutants`, and why three of them are refusals

```
$ judged mutants --sut vulture
19 classes: 3 passed, 16 failed
not measured: 8 of 19 classes are outside this SUT's languages — it opened no file in them, so neither its passes nor its failures there are results
decoy recall: 11 of 31 genuinely-dead files found
false removals: 6 — GATE FAILED (§11 R1: if this is not zero, the auto-act tier is deleted from the design rather than tuned)
classes with false removals: m01, m10, m11, m16
```

Exit code **1**.

```
$ judged mutants --sut deadcode
REFUSED — the E2 suite did not complete (exit 2)
  … `sh` exited with status 1 … Last stderr line: deadcode: packages contain errors

$ judged mutants --sut shear
REFUSED — the E2 suite did not complete (exit 2)
  … `cargo-shear` exited with status 2 … Last stderr line: error: could not find `Cargo.toml` in `…`

$ judged mutants --sut knip
REFUSED — the E2 suite did not complete (exit 2)
  … `npx` exited with status 2 … Last stderr line: ERROR: Unable to find package.json
```

Exit code **2** for all three.

**This is the shipped CLI's honest answer, and it is a real limitation of the harness, not a
broken install.** `run_suite` runs the whole catalogue as one unit and treats any class the
analyzer refuses as a failure of the run. Vulture finds no Python and exits 0, so its run
completes and the unreadable classes are marked *not measured*. knip, cargo-shear and
deadcode instead exit non-zero on a repository in the wrong language, and those exit codes
are deliberately not declared healthy because each is shared with a genuine analysis failure
whose output is equally empty. So **`judged mutants` can currently produce a graded result
only for an analyzer that exits 0 on foreign repositories — one of the four.**

The per-class numbers in §4 were therefore obtained by driving `judged_mutants::run_suite`
one class at a time, from a throwaway driver outside the repository, using the same
`CommandSut`, the same argv, the same declared success exit codes and the same adapters the
CLI uses. Cross-checked: driving vulture that way reproduces the CLI's report exactly —
3 passed, 16 failed, 11/31 decoys, 6 false removals on m01/m10/m11/m16.

---

## 3. A harness defect found on the way, which had silently disabled the Go row

Before this round, deadcode returned `deadcode: no packages` on **m12, the only Go class**,
and `deadcode: packages contain errors` on the other eighteen. Both are exit 1 with empty
stdout.

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

`FR n` = n false removals, a hard failure. `clean` = read it, claimed nothing live dead.
`—` = the run did not complete on this class (the tool refused the repository). Vulture never
refuses, so its unreadable classes are marked `not read` on the strength of its declared
ecosystems rather than its exit code. `d` = decoys found / decoys planted.

| # | Eco | Mechanism | vulture | knip | deadcode | cargo-shear |
| --- | --- | --- | --- | --- | --- | --- |
| m01 | python | dotted class path only in a YAML app list | **FR 1** `DunningConfig` d2/2 | — | — | — |
| m02 | polyglot | module name computed at runtime, `importlib`/`require` | clean d1/2 | **FR 1** `src/transports/websocketTransport.ts` d1/2 | — | — |
| m03 | python | plugin found by directory scan | **pass** d1/1 | — | — | — |
| m04 | rust | subcommand reachable only when a human types it | not read d0/1 | — | — | **pass** d1/1 |
| m05 | python | recovery handler on the failure path | **pass** d2/2 | — | — | — |
| m06 | rust | lock helper used only under contention | not read d0/2 | — | — | **pass** d2/2 |
| m07 | rust | guard clause inert until input is hostile | not read d0/2 | — | — | **pass** d2/2 |
| m08 | polyglot | script invoked only from CI / Dockerfile / k8s | clean d0/2 | — | — | — |
| m09 | rust | API exercised only by a README example CI runs | not read d0/2 | — | — | **pass** d2/2 |
| m10 | polyglot | framework convention: AppConfig, `__mocks__` | **FR 1** `ReportingConfig` d1/2 | clean d1/2 | — | — |
| m11 | python | model field enumerated reflectively | **FR 3** `legal_hold_until`, `retention_days`, `tenant_slug` d1/1 | — | — | — |
| m12 | go | symbol bound through `//go:linkname` | not read d0/2 | — | **FR 2** `TelemetryFlush`, `drain` d2/2 | — |
| m13 | polyglot | file rescued by an explicit `!` negation | clean d0/2 | — | — | — |
| m14 | typescript | committed build output consumed by a CDN path | not read d0/2 | **FR 1** `dist/widget.7f3a91c.js` d2/2 | — | — |
| m15 | python | worker named only in a queued job payload | **pass** d1/1 | — | — | — |
| m16 | python | type named only in a pickled blob | **FR 1** `RateSnapshot` d1/1 | — | — | — |
| m17 | rust | `inventory::submit!`, empty call graph | not read d0/1 | — | — | **pass** d1/1 |
| m18 | polyglot | entry point only in a platform manifest | clean d1/2 | — | — | — |
| m19 | rust | `#[no_mangle]` export consumed outside the repo | not read d0/1 | — | — | **pass** d1/1 |

| | vulture | knip | deadcode | cargo-shear |
| --- | --- | --- | --- | --- |
| classes the run completed on | 19 (11 in its languages) | 3 | 1 | 6 |
| **false removals** | **6** | **2** | **2** | **0** |
| classes with a false removal | m01, m10, m11, m16 | m02, m14 | m12 | none |
| decoy recall over completed classes | 11/31 | 4/6 | 2/2 | 9/9 |
| passed | 3 | 0 | 0 | 6 |

### The evidence behind the three non-Python rows

**deadcode on m12** — `deadcode -json` output, verbatim, reduced to names:

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

Yes — **five**, taking "survives" as *some tool completed on it and claimed nothing live
dead*:

**m01** (YAML string reference), **m11** (reflective model field), **m12** (`//go:linkname`),
**m14** (committed build output), **m16** (pickled blob).

On each of these, exactly one tool could read the class and that tool false-removed on it.
Nothing here is redundancy: with four tools spanning four ecosystems, most classes have
exactly one possible reader, so "no tool survives" and "the only reader failed" are the same
sentence for m01, m11, m12, m14 and m16.

A stricter reading tightens this. Two further classes have a tool that stayed clean but
found **zero** of the planted decoys there — vulture on **m08** and **m13**, both 0/2. A
tool that found nothing dead in a repository that contains two genuinely-dead files has not
demonstrated it read anything, and its silence is not evidence (that is the first clause of
vulture's own capability envelope). Counting those as unsurvived too gives **seven of
nineteen**: m01, m08, m11, m12, m13, m14, m16.

### Is there a tool with zero false removals across the classes it can read?

**Yes: cargo-shear. 0 false removals across 6 Rust classes, with 9/9 decoy recall** — so it
is not the refusing-SUT degenerate case; it demonstrably found every genuinely-dead file
planted in front of it.

That is the strongest single result in this round and it should be read narrowly. cargo-shear
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

- No single tool here clears the catalogue. The union of all four completes on all 19
  classes (vulture 11 + knip's m14 + deadcode's m12 + shear's 6), and that union carries
  **10 false removals across 7 distinct classes** (m01, m02, m10, m11, m12, m14, m16).
  Combining signals by union of claims cannot
  reduce a false removal — every claim any member makes is a claim the union makes — so
  **no subset of these four clears all 19 at zero false removals.** The only zero-false-removal
  subset is `{cargo-shear}`, which reads 6 of 19 and leaves m01, m11, m12, m14 and m16 with
  no reader at all.
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

| Config | files claimed unused |
| --- | --- |
| default (no `knip.json`) | `dist/widget.0c9e142.js`, **`dist/widget.7f3a91c.js`**, `src/unusedFeatureFlags.ts` |
| `entry: [src/main.ts, public/index.html]`, `project: [src/**, dist/**]` | `dist/widget.0c9e142.js`, **`dist/widget.7f3a91c.js`**, `src/unusedFeatureFlags.ts` |
| `entry: [src/main.ts, public/index.html]`, `project: [src/**, dist/**, public/**]` | `dist/widget.0c9e142.js`, **`dist/widget.7f3a91c.js`**, `src/unusedFeatureFlags.ts` |
| `entry: [src/main.ts, dist/*.js]` | `src/unusedFeatureFlags.ts` |
| `entry: [src/main.ts]`, `project: [src/**/*.ts]` | `src/unusedFeatureFlags.ts` |

*Degrades rather than being wrong:* refuted where it matters. knip degrades loudly only when
it cannot start at all — 16 of 19 classes have no `package.json` and it exits 2 with
`ERROR: Unable to find package.json`, which is unambiguous and correct behaviour. But on all
three classes it **could** read it produced a confident SARIF log, and on two of them that
log named a live file dead, at exit 1, with nothing marking the answer as degraded.

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

**4 — CONFIRMED.** 11 of 31, against 0 of 31 in the superseded baseline. Vulture is
unchanged; the decoys now carry symbols and vulture is a symbol-level tool. All 11 come from
classes with Python decoys.

---

## 7. LIMITS

Read this section before quoting any number above.

1. **Three of the four tools cannot be run by the shipped CLI against this catalogue.**
   `judged mutants --sut knip|deadcode|shear` exits 2 and grades nothing. The per-class
   numbers come from a scratch driver calling `run_suite` one class at a time with the same
   SUT construction. It is verified to reproduce the CLI exactly for vulture, which is the
   only tool where both paths produce a result — so the cross-check covers the driver's
   fidelity but not, independently, the three tools it was written for.

2. **"Not read" is most of this table.** Of 76 tool×class cells, 47 are runs that did not
   complete at all (knip 16, deadcode 18, cargo-shear 13), and a further 8 are vulture runs
   that completed without opening a file in the class. **55 of 76 cells are not results.**
   Every per-tool false-removal count is a count over a small, language-determined subset:
   deadcode's entire result is one class.

3. **A zero is not a pass.** cargo-shear's 0/6 and vulture's clean cells on m08 and m13 are
   produced by tools that could not make the failing claim. E2 grades what a tool claims;
   it does not grade what a tool declined to think about. The capability envelopes exist
   precisely to stop a zero being read as competence, and they should be read alongside
   every row here.

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
