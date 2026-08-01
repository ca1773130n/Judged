# Out of sample: Gate 2a's flag rate and the root set, on nine real repositories

**Date:** 2026-08-02 · **Toolchain:** rustc 1.94.1, git 2.50.1 ·
**Corpus:** nine shallow clones, §10 E5's shapes, pinned by SHA in section 1

**Judged, two states, both measured here:**

- **before** — `ce0d97f`, the commit that landed the root set. Its Tier A manifest readers were
  hand-written TOML and YAML subsets, about 1300 lines.
- **after** — `ce0d97f` plus this round's replacement of those subsets with `toml` 1.1.4
  (toml-rs, the parser Cargo is built on) and `saphyr-parser` 0.0.11. That change is in the
  working tree this document was written from and is uncommitted as of writing; it is the
  commit this document ships in. No other behaviour differs between the two states.

> Everything Judged had measured before the first version of this document lived on nineteen
> fixtures written from the same research document that specifies Judged's rules. §10 E5 says
> six popular Python libraries are far too homogeneous and names what a defensible corpus
> needs. This is the second pass over code nobody wrote a fixture for: the first pass found
> the root set empty on most of it, and this one re-runs after the cause was fixed.

**Three results.**

1. **The Tier A root scan went from 2 of 9 repositories to 9 of 9.** Before, one unreadable
   manifest voided every machine-declared root in the repository, and 7 of the 9 contained a
   manifest the hand-written parsers rejected even though the file was valid — so 5 of 9
   returned nothing at all and 7 of 9 returned no Tier A root. After, every repository
   produces Tier A roots, the corpus total goes from **264 roots to 854**, and the eight gaps
   fall to one — which is a real gap, a framework with no plugin, not a parse failure.
2. **The hand check got worse as the root set got bigger, and only the hand check found it.**
   47 roots sampled at stride 20 across all nine repositories: **37 correct, 3 correct-but-
   unresolved, 7 wrong** — against 29/2/1 of 32 last round. All 7 are one defect, in one rule,
   in one repository, and it was invisible before because that repository emitted no Tier A
   root to be wrong. Section 4.3 measures it: 99 of otel-demo's 130 `packaged_file` roots name
   a path that does not exist.
3. **§11 R8's flag-rate half is not closable by picking a needle set** — on the population an
   analyzer actually claims, the shipped `basename+stem` blocks a median **84.6%** of
   candidates and `+parent-dir` blocks **100% on seven of the nine repositories**. R8 asks for
   the strategy whose fire rate is "tolerable" and carries no numeric threshold for the word,
   so this does not settle R8; what it establishes is that every strategy in R8's space has a
   high flag rate on real code, which is a fact R8's framing did not anticipate.

Gate 2e's shallow-clone refusal is in section 6. It refused all 27 candidates across the nine
shallow clones and did not refuse in a full clone.

---

## 1. The corpus

Cloned into a scratch directory outside the repository and deleted at the end of the round.
Every clone is shallow, which is deliberate: §6.19 says shallow clones are the CI default and
that they silently void every VCS signal, so the corpus doubles as a test of the refusal
(section 6).

The first version of this document recorded `git clone --depth 1 --single-branch`. That
recipe no longer reproduces the corpus, because each repository's default branch has moved
since. Fetching the recorded SHA directly gives the same shallow shape at the same commit,
and is what was run this round:

```sh
pin() {  # dir url sha
  mkdir -p "$1" && git -C "$1" init -q && git -C "$1" remote add origin "$2"
  git -C "$1" fetch -q --depth 1 origin "$3" && git -C "$1" checkout -q FETCH_HEAD
}
pin requests          https://github.com/psf/requests.git                      414f0513c33883adf6f2b46901d4f0b38a455851
pin pip               https://github.com/pypa/pip.git                          6236392d41f0623476b9dbca2f1c55b832ee7e43
pin djangoproject     https://github.com/django/djangoproject.com.git          4744e9ef8f27c113586fd062d7028e70d6554340
pin zod               https://github.com/colinhacks/zod.git                    912f0f51b0ced654d0069741e7160834dca742ee
pin cobra             https://github.com/spf13/cobra.git                       adbc8813901bba65827259daa8e22ff94ec1f30e
pin node_exporter     https://github.com/prometheus/node_exporter.git          b401dcfc667cee0a5d29232bab51a8ce1c58ec07
pin sample-controller https://github.com/kubernetes/sample-controller.git      3e50bfd72c521dd4b1b9d832b9f2e6254b4ff148
pin otel-demo         https://github.com/open-telemetry/opentelemetry-demo.git f7408a50aac60bd848dda33f7df5db43503e4b7a
pin ripgrep           https://github.com/BurntSushi/ripgrep.git                435f59fc4b43af3ab32f34d53fa34978f393fe52
```

| Repo | Commit | Tracked files | E5 shape it covers |
| --- | --- | ---: | --- |
| `psf/requests` | `414f0513` | 130 | Python library |
| `pypa/pip` | `6236392d` | 1043 | Python, **vendored tree** (297 files under `src/pip/_vendor/`) |
| `django/djangoproject.com` | `4744e9ef` | 645 | **Django-shaped app** (4 `apps.py`, real `INSTALLED_APPS`) |
| `colinhacks/zod` | `912f0f51` | 583 | **TypeScript**, pnpm workspace, 8 packages |
| `spf13/cobra` | `adbc8813` | 66 | **Go** library, flat layout |
| `prometheus/node_exporter` | `b401dcfc` | 405 | Go application, deep `collector/` tree |
| `kubernetes/sample-controller` | `3e50bfd7` | 58 | Go, **heavy codegen** (31 of 58 files generated) |
| `open-telemetry/opentelemetry-demo` | `f7408a50` | 584 | **Polyglot monorepo**, protobuf codegen, 3 frameworks detected |
| `BurntSushi/ripgrep` | `435f59fc` | 237 | **Rust** workspace |

`node_exporter` was picked expecting a `vendor/` directory and no longer has one; Go modules
removed vendoring from most of the ecosystem. `pip` carries the vendored tree instead, which
is why the corpus is nine repositories rather than eight.

**The reconstruction was checked before it was used.** Running the `ce0d97f` binary over these
nine clones reproduces the first version's section 5 exactly — 264 roots, 197 Tier A, 67 Tier
B, 8 gaps, the same five repositories at zero — so the "before" column below is the same
measurement, not a similar one.

---

## 2. Reproducing this

Two programs. `judged show-roots` is in the repository. The fire-rate sweep is not — nothing
in the shipped CLI measures a flag rate over a whole repository, so the sweep is a throwaway
crate outside the repository that links `judged-core` and calls the same `LiteralVeto::query`
the gate calls, with `ScanLimits::default()` and no other configuration.

```sh
# Root set, per repo, with each binary. Timings in section 5 are the minimum of five runs.
for r in corpus/*/; do judged show-roots --json "$r" > "roots.$(basename $r).json"; done

# Fire rate, per repo. 100 candidates per population, sampled deterministically.
firerate corpus/*/ > firerate.tsv
```

**The sample is deterministic and reconstructible.** `git ls-files`, sorted, then every
*k*-th entry with *k* = ⌊total / 100⌋; repositories with fewer than 100 tracked files are
taken whole (`cobra` 66, `sample-controller` 58). Three populations are reported separately:

- **path-all** — every sampled tracked file, including docs, images and fixtures.
- **path-source** — the subset with a source extension (`.py .rs .go .ts .tsx .js .jsx .mjs
  .cjs .java .c .cc`). **This is the decision-relevant population**, because it is what an
  analyzer claims. It is a re-slice of the same verdicts, not a second run.
- **symbol** — up to 100 `(defining file, symbol name)` pairs, extracted by a line-prefix
  scan (`def `, `class `, `pub fn `, `func `, `export function `, …) over the sampled source
  files. Crude on purpose: it is a sample of declared symbols, not a parse.

**A "fire" is a BLOCK, not an error.** No ground-truth labels were collected and none are
inferred. Section 9 states what follows from that and what does not.

**What was re-derived this round and what was not.** The percentage columns of sections 3.1
and 3.2 were re-measured from scratch with a rebuilt sweep and reproduce the first version's
figures **exactly**, every cell, including every `n` — so those two tables are verified rather
than inherited. Section 3.2's *kinds fired* column and the whole of section 3.3 are carried
from the first run and were **not** re-derived: the symbol extractor was a throwaway and the
"…" in its prefix list is not a specification, so re-running it would produce a different
sample rather than a check on the old one. Treat 3.3 as the weaker table of the three.

---

## 3. §11 R8: the fire rate on real code

### 3.1 path-source — what an analyzer would actually claim

| Repo | `basename` | `basename+stem` | `+parent-dir` | `+parent-dir+symbol` | n |
| --- | ---: | ---: | ---: | ---: | ---: |
| cobra | 5.6% | 19.4% | 47.2% | 47.2% | 36 |
| node_exporter | 4.2% | 12.5% | **100.0%** | **100.0%** | 48 |
| zod | 15.9% | 49.3% | **100.0%** | **100.0%** | 69 |
| sample-controller | 15.0% | 70.0% | 97.5% | 97.5% | 40 |
| otel-demo | 43.6% | 84.6% | **100.0%** | **100.0%** | 39 |
| ripgrep | 27.3% | 93.2% | **100.0%** | **100.0%** | 44 |
| djangoproject | 28.2% | 94.9% | **100.0%** | **100.0%** | 39 |
| pip | 54.5% | 87.9% | **100.0%** | **100.0%** | 66 |
| requests | 39.1% | **100.0%** | **100.0%** | **100.0%** | 23 |
| **median** | **27.3%** | **84.6%** | **100.0%** | **100.0%** | |
| **range** | 4.2 – 54.5 | 12.5 – 100 | 47.2 – 100 | 47.2 – 100 | |

### 3.2 path-all — every tracked file, with the needle kinds that fired

| Repo | `basename` | `+stem` | `+parent-dir` | kinds fired at `+parent-dir` | n |
| --- | ---: | ---: | ---: | --- | ---: |
| zod | 25.0% | 57.0% | 97.0% | basename 24, stem 49, **parent-dir 93** | 100 |
| cobra | 27.3% | 39.4% | 59.1% | basename 15, stem 22, parent-dir 27 | 66 |
| sample-controller | 29.3% | 69.0% | 89.7% | basename 17, stem 29, **parent-dir 44** | 58 |
| requests | 39.0% | 78.0% | 96.0% | basename 30, stem 58, **parent-dir 77** | 100 |
| djangoproject | 42.0% | 74.0% | 96.0% | basename 34, stem 49, **parent-dir 93** | 100 |
| node_exporter | 48.0% | 53.0% | 98.0% | basename 43, stem 20, **parent-dir 90** | 100 |
| pip | 55.0% | 89.0% | 100.0% | basename 33, stem 71, **parent-dir 86** | 100 |
| otel-demo | 57.0% | 81.0% | 99.0% | basename 33, stem 58, **parent-dir 88** | 100 |
| ripgrep | 62.0% | 94.0% | 97.0% | basename 61, stem 69, **parent-dir 78** | 100 |
| **median** | **42.0%** | **74.0%** | **97.0%** | | |

### 3.3 symbol claims

Carried from the first run; see the note at the end of section 2.

| Repo | `basename` | `+stem` | `+parent-dir` | `+symbol` | symbol needle fired |
| --- | ---: | ---: | ---: | ---: | ---: |
| node_exporter | 0.0% | 4.0% | **100.0%** | **100.0%** | 33 |
| zod | 2.0% | 95.0% | **100.0%** | **100.0%** | 39 |
| otel-demo | 2.0% | 36.0% | **100.0%** | **100.0%** | 75 |
| cobra | 3.0% | 34.0% | 44.0% | 53.0% | 35 |
| sample-controller | 22.0% | 77.0% | 92.0% | 94.0% | 41 |
| ripgrep | 27.0% | 98.0% | **100.0%** | **100.0%** | 47 |
| djangoproject | 29.0% | **100.0%** | **100.0%** | **100.0%** | 48 |
| requests | 31.0% | **100.0%** | **100.0%** | **100.0%** | 41 |
| pip | 68.0% | 95.0% | **100.0%** | **100.0%** | 25 |
| **median** | **22.0%** | **95.0%** | **100.0%** | **100.0%** | |

### 3.4 What the tables say

**`+parent-dir` is not a tuning option, it is a constant function.** 100% on seven of nine
repositories in path-source, ≥ 89.7% on eight of nine, and the exception — cobra at 47.2% —
is not a reprieve: cobra keeps 37 of its 66 files in the repository root, and a root-level
file derives no parent-directory needle at all. Where there is a directory name to search
for, it is found. The three repositories with the deepest trees (node_exporter's
`collector/`, zod's `packages/*/src/`, otel-demo's `src/*/`) are all at exactly 100%.

**`+symbol` is byte-identical to `+parent-dir` on path claims** — every repository, both path
populations, no exceptions, in both the original run and this round's re-derivation. A path
candidate carries no symbol name, so the fourth needle derives nothing, and R8's four-way
sweep is a three-way sweep out of sample exactly as it was in sample. It separates only on
symbol claims, and there by very little.

**The shipped default is not in a different regime from the strategy it was chosen over.**
`basename+stem` blocks a median 84.6% of source-file claims — 100% on requests, 94.9% on
djangoproject, 93.2% on ripgrep, 87.9% on pip. Four of the nine are above 87%. The
in-sample round found `+stem` "free at this scale" and it was, on ten-file fixtures; on real
code it is the needle that carries most of the blocking.

**Even `basename` alone blocks a median 27.3% of source-file claims**, and 54.5% of pip's.
The floor of Gate 2a — the part §9.3 makes structurally impossible to remove — is already a
double-digit flag rate on real code.

**The stem is doing the work, and generic stems are why.** node_exporter is the clean
counter-example that proves the mechanism: its Go files are `arp_linux.go`,
`bcache_linux.go`, `bonding_linux.go` — long, compound, specific — and `+stem` moves it only
from 4.2% to 12.5%. requests' files are `models.py`, `utils.py`, `status_codes.py`, and
`+stem` takes it from 39.1% straight to 100%. The needle that fires is the one whose text is
a word the repository uses for other reasons.

---

## 4. The root set: what the parsers cost, and what is still wrong

Nothing crashed and nothing timed out, in either state. Every `judged show-roots` exited 0;
every sweep exited 0 with an empty stderr; no scan hit `ScanLimits`, and **zero** of the
blocks in section 3 came from an incomplete scan (the incomplete column is 0 in all 18 sweep
rows), so every figure there is a literal hit rather than an abstention counted as one.

### 4.1 Before: one unreadable manifest voided every Tier A root in the repository

Seven of the nine repositories contained at least one manifest the hand-written parsers
rejected, and in every case the file was valid. The scan is all-or-nothing, and its own
message said so — this is the gap `ripgrep` produced at `ce0d97f`, quoted from the run:

> ``Cargo.toml:123: unknown escape `\<newline>`. One unreadable manifest fails the whole Tier
> A scan, so EVERY machine-declared root is missing from this list — not just this
> package's.``

**Failing closed is correct and did not change.** §6.20 makes "no data" a distinct state from
"zero", and emitting the other packages' roots while silently dropping the unreadable one's
would produce a root list that looks complete and is not. The bug was never the policy. It
was that valid input was being rejected, and the blast radius of one rejection is a
repository.

The rejected file, and what each repository recovered once the parser could read it:

| Repo | Manifest rejected at `ce0d97f` | Tier A before | Tier A after | First run's floor |
| --- | --- | ---: | ---: | ---: |
| otel-demo | `.github/workflows/checks.yml:25` | 0 | **299** | ≥ 261 |
| ripgrep | ``Cargo.toml:123`` | 0 | **85** | ≥ 76 |
| node_exporter | `.github/workflows/golangci-lint.yml:18` | 0 | **73** | 68 |
| pip | `.github/workflows/ci.yml:311` | 0 | **72** | 28 |
| requests | `.github/workflows/run-tests.yml:20` | 0 | **39** | 27 |
| cobra | `.github/workflows/test.yml#jobs.test-win.defaults.run` | 0 | **17** | 2 |
| sample-controller | `go.mod:7` | 0 | **5** | ≥ 4 |

The first run estimated the loss by deleting the offending file and re-running, and said those
figures were floors rather than estimates. Every one of them held: each recovered Tier A count
is at or above its floor, and the two marked `≥` — where deleting `Cargo.toml` or a `go.mod`
also deleted the roots that manifest itself declares — are the two that exceeded it by most.

Five defects accounted for all seven repositories: a YAML key whose only value is a trailing
comment; `jobs.<id>.defaults.run`, which is a mapping, read as if it were a step's scalar
`run`; a TOML line continuation inside a multi-line basic string; the `godebug` directive,
which `go.mod` has had since Go 1.23; and YAML flow mappings, which were not read at all.

**None of them is a defect a subset parser can be patched out of.** Each is ordinary syntax
that the format's own specification requires, and the list of things a hand-written subset
gets wrong is not enumerable from inside the subset — which is the argument for the fix that
was applied rather than for a sixth patch. TOML is now read by `toml` 1.1.4 (toml-rs, what
Cargo itself parses manifests with, so a manifest Cargo accepts is one Judged accepts) and
YAML by `saphyr-parser` 0.0.11, used as an event stream so that neither YAML's type resolution
nor anchor expansion runs over a GitHub workflow. All five are pinned by regression tests
carrying the offending file verbatim at the SHA in section 1 —
`ripgreps_debian_extended_description_is_a_valid_multi_line_string`,
`sample_controllers_go_mod_declares_a_godebug_directive`,
`node_exporters_permissions_key_carries_only_a_trailing_comment`,
`cobras_job_default_run_is_a_mapping_not_a_command`,
`requests_excludes_a_matrix_combination_with_a_flow_mapping` — and the tests that encode the
fail-closed policy on genuinely malformed input all still pass.

### 4.2 What the fix did not fix: Tier B

Tier B is unchanged by this round: 67 roots before, 67 after, from django and pytest only.
Three things it should have found, it still does not.

**Next.js produces zero roots in both repositories that declare it.** `next` is detected in
zod (declared `15.5.15`) and otel-demo (declared `16.2.12`), both marked `covered: true`, and
neither emits a single Next.js root. zod holds **11** app-router entry points under
`packages/docs/app/` (`(doc)/[[...slug]]/page.tsx`, `api/search/route.ts`, `layout.tsx`,
`not-found.tsx`, `og.png/route.tsx`, …); otel-demo holds **16** pages-router entry points
under `src/frontend/pages/` (`_app.tsx`, `_document.tsx`, `api/cart.ts`, …). Every one of them
is Tier B's exact subject: a file a framework turns into an entry point with no source
reference anywhere. Two independent causes:

- **`app/**` is anchored at the repository root.** zod's app router lives at
  `packages/docs/app/`, and the rules stop matching. §10 E5 names the polyglot monorepo as a
  corpus shape the design has to survive; a workspace is where a JavaScript framework normally
  lives.
- **The pages router has no rule.** `Rule` has four `NextAppRouter*` variants and nothing for
  `pages/`. Next 14, 15 and 16 all still support it and otel-demo uses it.

**pytest is not detected in pip at all**, so **140 test modules and 2 conftests** never become
roots. (`git ls-files` at the pinned SHA: 140 paths matching `test_*.py`, all under `tests/`,
plus `tests/conftest.py` and `tests/unit/resolution_resolvelib/conftest.py`.) The cause is that
pip declares pytest under PEP 735 `[dependency-groups]` in `pyproject.toml` rather than in a
`requirements*.txt`. After the parser fix pip's gap list is empty and says nothing about
pytest.

**The claim the first version of this document made here was too broad, and is withdrawn.** It
said Tier B "reports `covered: true` and `gaps: 0` while emitting nothing" on three
repositories. That is true of exactly one — **zod**: `next` covered, `gap_count` 0, zero Tier
B roots. It is false of otel-demo, which reports one gap and does emit 7 Tier B roots (pytest);
and false of pip, where pytest is never detected, so there is no `covered: true` to be wrong —
pip's failure is silence, not a false claim of coverage. The corrected statement is narrower
and still worth making: **on zod, a framework Judged recognized, named and marked covered
contributed zero roots, next to a gap count of zero.** §9.5 caps tiers on unresolved hints
because a fabricated root hides a real gap; that is the same failure with the sign flipped.

**And the version was never resolved, on any framework, in any repository.** §5.1 makes Tier
B "correct only if framework **and version** detected correctly", so this is the load-bearing
caveat on all 67 Tier B roots and the first version did not disclose it. What `show-roots`
reports is the *declared requirement*, taken off the manifest, not a resolved version — the
field is named `declared_version` and the report says so, but the count beside it does not:

| Repo | Framework | `declared_version` | What it actually is |
| --- | --- | --- | --- |
| djangoproject | django | `null` | Detected from the `manage.py` marker file; **no version information at all** |
| requests | pytest | `>=2.8.0,<10` | A range spanning eight major versions |
| otel-demo | pytest | `==9.0.3 \` | A pip-compile pin, with the line-continuation backslash still attached |
| otel-demo | fastapi | `==0.140.7 \` | Same; not covered, and the corpus's one remaining gap |
| otel-demo | next | `16.2.12` | The `dependencies` entry in `package.json`, not the lockfile's resolution |
| zod | next | `15.5.15` | Same |

Nothing here consults a lockfile. The 50 django roots in section 5 rest on a version that is
`null`, and the trailing ` \` on two of these is direct evidence that the string is raw
manifest text rather than anything resolved.

### 4.3 After: a new defect, and only the hand check found it

`packaged_file` roots derived from a `Dockerfile COPY` are rebased onto the Dockerfile's own
directory. That is right when the build context is the Dockerfile's directory and wrong when
it is not, and in otel-demo it is not: every service's Dockerfile is built from the repository
root, so its `COPY` sources are already repo-relative and get the prefix a second time.

```
src/ad/Dockerfile:8   COPY ./src/ad/gradlew* ./src/ad/settings.gradle* ./src/ad/build.gradle ./
  -> root target      src/ad/src/ad/settings.gradle*
```

Across otel-demo's 130 `packaged_file` roots, **99 carry a doubled prefix and only 3 name a
path that exists.** They are 33% of that repository's Tier A roots and 12.6% of the corpus's
854.
No other repository shows the pattern — djangoproject's and zod's Dockerfiles sit where their
context does — which is exactly why it stayed hidden: before this round otel-demo emitted zero
Tier A roots, so the rule with the defect had nothing to be wrong about.

**This is left unfixed, deliberately and for the same reason as last round.** The file that
owns it, `roots/manifest.rs`, is the file this round rewrote; correcting the context rule is a
separate change with its own tests, and a build context is not knowable from a Dockerfile
alone — it is declared in `docker-compose.yml` or on the `docker build` command line, neither
of which this module reads today. Recorded here so the next round has it, rather than fixed
under cover of a documentation pass.

### 4.4 A root with an empty target, still present

```
A	packaged_file	Dockerfile#copy@41[0]	<empty>
```

djangoproject's `Dockerfile:41` is `COPY . .`. `COPY ./requirements ./requirements` on line 29
resolves correctly to `requirements`; `COPY . .` still yields a root whose target is the empty
string. An empty target is not a root — either it is `.` or it is a gap saying a whole-tree
copy cannot be resolved to seeds. Unchanged since the first run and unfixed for the same
reason as 4.3.

---

## 5. The root set, per repo

Timings are the minimum of five wall-clock runs of `judged show-roots --json`, warm cache,
process start included.

| Repo | Roots before | Roots after | A | B | C | Gaps | Files scanned | Time |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| otel-demo | 7 | **306** | 299 | 7 | 0 | 1 | 574 | 15.9 ms |
| zod | 175 | 175 | 175 | 0 | 0 | 0 | 580 | 8.0 ms |
| ripgrep | 0 | **85** | 85 | 0 | 0 | 0 | 236 | 6.5 ms |
| node_exporter | 0 | **73** | 73 | 0 | 0 | 0 | 404 | 9.2 ms |
| djangoproject | 72 | 72 | 22 | 50 | 0 | 0 | 645 | 12.3 ms |
| pip | 0 | **72** | 72 | 0 | 0 | 0 | 1037 | 14.5 ms |
| requests | 10 | **49** | 39 | 10 | 0 | 0 | 128 | 3.9 ms |
| cobra | 0 | **17** | 17 | 0 | 0 | 0 | 66 | 3.3 ms |
| sample-controller | 0 | **5** | 5 | 0 | 0 | 0 | 58 | 4.2 ms |
| **total** | **264** | **854** | **787** | **67** | **0** | **1** | | |

**Repositories with a non-empty Tier A root set: 2 of 9 before, 9 of 9 after.** Repositories
returning nothing at all: 5 of 9 before, 0 of 9 after.

**The one remaining gap is a real one.** otel-demo declares `fastapi` in
`src/agent/requirements.txt`, Judged recognizes it, has no plugin for it, and says so —
`framework detected, no plugin — its convention roots are missing — tier capped (§9.5)`. That
is the gap channel doing its job: a framework whose entry points are missing from the list is
named in the list of what is missing. Eight gaps became one, and the one that survived is the
kind that should.

**Speed is not the problem.** 15.9 ms is the worst of the nine, over a 574-file repository
producing 306 roots. No configuration of this module is going to be too slow to run in CI.

**No repository produced a Tier C root, and that is correct.** Tier C is solicited from a
human and committed; none of these repositories has been asked. An empty Tier C column on a
corpus nobody has interviewed is the honest answer, and it is worth noting that the module
did not invent one.

---

## 6. §6.19: the shallow-clone refusal behaved

The corpus is nine shallow clones, in which every file reports the same grafted commit. Gate
2e is supposed to refuse rather than answer.

The first version of this document quoted `cargo test` output for two tests
(`a_full_clone_does_not_report_shallow_history`,
`every_shallow_corpus_repo_refuses_with_shallow_history`) that do not exist in this repository
and never did. **That output was fabricated and is deleted.** The claim itself was checkable,
so it was checked: below is real captured output from a throwaway harness that calls the
shipped `RecencyVeto::default().judge` on three files per repository — sorted `git ls-files`,
indices 0, ⌊n/3⌋, ⌊2n/3⌋ — across the nine clones and, as a contrast, the Judged working
clone, which is complete.

```
cobra              tracked=66    shallow=true
      [0] .github/dependabot.yml -> Vetoed(ShallowHistory)
      [22] bash_completions_test.go -> Vetoed(ShallowHistory)
      [44] flag_groups.go -> Vetoed(ShallowHistory)
...
sample-controller  tracked=58    shallow=true
      [0] .github/PULL_REQUEST_TEMPLATE.md -> Vetoed(ShallowHistory)
      [19] hack/update-codegen.sh -> Vetoed(ShallowHistory)
      [38] pkg/generated/clientset/versioned/typed/samplecontroller/v1alpha1/doc.go -> Vetoed(ShallowHistory)
Judged             tracked=90    shallow=false
      [0] .github/workflows/ci.yml -> Vetoed(RecentCommit { committed_at: 1785529348, age: 65478s })
      [30] crates/judged-core/tests/fingerprint_normalize.rs -> Vetoed(RecentCommit { committed_at: 1785529348, age: 65478s })
      [60] crates/judged-mutants/src/fixtures/m15_enqueued_job_payload.rs -> Vetoed(RecentCommit { committed_at: 1785549733, age: 45093s })

30 candidates: 27 Vetoed(ShallowHistory), 3 other
```

All 27 shallow candidates returned `Vetoed(ShallowHistory)` — not a stale timestamp read as
recency, not an `EvidenceUnavailable` shrug, the specific refusal. `Repo::is_shallow` answered
`true` for all nine and `false` for the full clone, whose three candidates are vetoed for
`RecentCommit` instead: a veto reached by running the gate rather than by declining to. So the
assertion is not passing because the gate refuses unconditionally.

The behaviour is also covered by two tests that do exist, on fixtures rather than on the
corpus: `shallow_clone_vetoes_a_file_a_full_clone_clears` and `the_shallow_veto_says_why` in
`crates/judged-core/tests/veto_recency.rs`.

**This is the one layer the corpus was expected to break and it did not.** It is also the
only layer here that gets no credit for being right, because refusing is all it does; the
value of the refusal is that the other gates never see a history-derived number that was
never computed.

---

## 7. The hand-checked sample

A count of roots says nothing about whether they are roots, and this round the count went up
5.7× — which is exactly the situation in which a bigger number can be a worse answer. So the
sample was widened to cover all nine repositories rather than the four that produced roots
last time.

**Stride 20, starting at index 0, over each repository's roots in the order `show-roots`
emits them** — 47 roots of 854. Every one was checked against the file it claims to come
from, and the mechanical part of that check was done by re-resolving the cited key with an
*independent* parser: Judged reads YAML with saphyr-parser and JSON with serde, and the check
re-reads the same key path with PyYAML 6.0.3 and Python's `json`, then compares values.

**37 correct, 3 correct but unresolved, 7 wrong.**

| Verdict | n | What was checked |
| --- | ---: | --- |
| Correct | 37 | The origin file exists, the cited key holds the cited value under an independent parser, and the target names something real |
| Correct declaration, unresolved target | 3 | All zod: `files[4]` → `packages/zod/**/*.d.ts`, `exports["./mini"].require` → `packages/zod/mini/index.cjs`, `exports["./v4/core"].require` → `packages/zod/v4/core/index.cjs`. Faithful transcriptions, correctly rebased from package-relative to repo-relative; the glob is never expanded and the two `.cjs` files are build output, gitignored by `packages/zod/.gitignore:3` |
| Wrong | 7 | All otel-demo `packaged_file`, all the doubled-prefix defect of section 4.3 |

Compare with last round: 29 correct, 2 unresolved, 1 wrong of 32. The defensible share fell
from 31/32 (96.9%) to 40/47 (85.1%), and every one of the seven new errors is the same defect
in the same rule in the same repository.

Spot checks worth quoting, because they are the ones that could have been fabricated and were
not:

- **Array indices are real indices.** `node_exporter`
  `.github/workflows/ci.yml#jobs.build.steps[4].run` re-resolves under PyYAML to the exact
  two-branch `promu codesign` shell block Judged reports, newlines included. All 29 sampled
  `ci_action` and `command` roots matched their key's value byte for byte.
- **The formerly-fatal files are now the ones producing roots.** cobra's single sampled root
  comes from `.github/workflows/labeler.yml`, and `jobs.triage.steps[0].uses` is
  `actions/labeler@v5`; sample-controller's comes from the `go.mod` whose `godebug` line used
  to void the repository, and line 3 really is `module k8s.io/sample-controller`.
- **Tier B is not guessing at Django.** Both sampled django roots resolve:
  `dashboard/management/commands/update_metrics.py` and `legacy/urls.py` both exist and are
  what they are claimed to be.
- **pytest Tier B points at real files.** Both sampled pytest roots —
  `otel-demo test/telemetry/test_agentic.py`, `requests tests/test_adapters.py` — exist.
- **Container entry points are read correctly.** `src/opamp-server/Dockerfile#entrypoint@42`
  claims `/opamp-server`; line 42 is `ENTRYPOINT ["/opamp-server"]`.

**The failure mode has changed, and that is the finding.** Last round it was silence: five
repositories with nothing, and a caller unable to tell an empty root set produced by a clean
repository from one produced by a parse error two directories away. This round the silence is
gone and what replaced it is one loud, localized wrongness — 99 roots naming paths that do not
exist, in one rule, in one repository. That is the better failure of the two, because it is
visible to anyone who opens the file it cites; but a corpus root list with 99 of its 854
entries naming nothing is not a list anything should act on, and section 4.3 is the work that
follows from this section.

---

## 8. §11 R8: what to do

R8 records two requirements that conflict — §9.3's *block on any hit*, and a tolerable flag
rate — and asks for a measurement rather than an argument.

**1. Do not ship `+parent-dir`, and do not keep treating `+symbol` as a distinct row.**
`+parent-dir` blocks 100% of source-file claims on seven of nine real repositories and ≥ 89.7%
on eight of nine. The 100% figure from Judged's own files was not a small-repository artifact.
`+symbol` is byte-identical to it on every path claim in every repository, so for path claims
the sweep R8 asks for has three distinct rows, not four.

**2. Keep the default at `basename+stem`, and stop describing its flag rate as tolerable
without saying what tolerable means.** Moving it does not help. `basename` alone was rejected
in sample for missing E2 classes 1, 8 and 13, and it still blocks a median 27.3% of
source-file claims out of sample — so the narrowest strategy that satisfies R8's rescue half
is also, on real code, a strategy that blocks 84.6% of claims at the median and 100% on one
repository in nine.

**R8's flag-rate half is not settled by this, and calling it settled would outrun the
evidence.** R8 asks for the strategy whose fire rate is tolerable and never says what number
that is; without a threshold, no measurement can discharge it. What is established is the
input such a threshold would have to be set against: **every strategy in R8's space has a high
flag rate on real code**, the spread between the narrowest and the widest is 27.3% to 100% at
the median, and there is no low-flag-rate corner of that space to retreat to. If the threshold
is ever written down above 27%, R8's two halves are unsatisfiable together and the
requirement, not the needle set, is what has to change.

**3. Therefore the next experiment is not another needle sweep.** What this round can support
is narrow: a Gate 2a hit is not, on real code, discriminating enough to be a terminal block on
its own, and the evidence needed to make it discriminating is already carried on every `Hit` —
which needle kind fired, in which file, at which offset. A block citing a `basename` hit in
one file and a block citing a `stem` hit that is a common English word are indistinguishable
in today's report and are not the same evidence. Ranking them is a design question this
document does not answer and did not measure.

**4. And the honest framing for §11 R1.** R1 asks whether an auto-act tier can exist at all,
and names E2 as what answers it. E2 measured what Gate 2 rescues. This measures what it
blocks, and at the shipped configuration it blocks most of what a source-file analyzer would
claim on a real repository. If both numbers hold, the surviving claim set on real code is
small — a finding about the size of any auto-act tier, arrived at from the flag-rate side, and
it needs a labelled corpus before it is more than a strong hint.

> **The re-measurement is no longer out-of-sample for the half that changed.** The five
> parser defects in §4.1 were *discovered by this corpus*, fixed against the exact bytes
> of these repositories, and then re-measured on the same nine. For the parser fixes the
> corpus is now a training set, and the honest reading of "9 of 9 repositories produce
> roots" is that the parsers handle the manifests they were repaired against. The fire-
> rate sweep and the recency refusal are untouched by that and remain out-of-sample. A
> genuinely independent check needs repositories nothing here was fitted to.

**5. The root set can now be measured, and a 47-root sample checks out at 85%.** Only 47 of
the 854 roots were checked at all — stride 20 across all nine repositories, verified against
an independent parser — of which 40 were defensible (37 correct, 3 correct-but-unresolved)
and 7 wrong. Nothing licenses a claim about the other 807. Note also that accuracy FELL from
last round's 31 of 32 (96.9%) to 40 of 47 (85.1%): the root set got much bigger and somewhat
less accurate, and a bigger root set that is less accurate is not straightforwardly an
improvement. Section
4.1's parser defects are closed and "how many roots does Judged materialize on a real
repository" has an answer for the first time: 854 across nine repositories, 787 of them Tier
A, of which 99 are known wrong. Section 4.3's defect, section 4.4's empty target and section
4.2's three missing Tier B conventions are what stand between that number and one worth acting
on.

---

## 9. Reconciliation with the earlier rounds

- [`2026-08-01-vulture-e2-baseline.md`](2026-08-01-vulture-e2-baseline.md) — vulture alone,
  bare, and already superseded by the four-analyzer round. Unaffected: nothing here touches
  E2 grading.
- [`2026-08-01-four-analyzers-e2.md`](2026-08-01-four-analyzers-e2.md) — all four analyzers,
  bare. Unaffected, same reason. It remains the reference for the bare column.
- [`2026-08-02-gate2-veto.md`](2026-08-02-gate2-veto.md) — Gate 2 measured **in sample**,
  against the 19-class catalogue. This document is the out-of-sample counterpart and the two
  measure opposite quantities: that one measures what the veto **rescues** on fixtures written
  from the same research document, this one measures what it **blocks** on code nobody wrote
  for it. Its §7 needle sweep and section 3 here agree on direction and cannot be compared
  cell-for-cell — different corpora, different denominators. Where it says `basename+stem` is
  "the narrowest strategy meeting R8's own criterion", read that as the **rescue** half of R8
  only; section 8 above is the flag-rate half, and it does not close.
- **The first version of this document** measured `ce0d97f` and is superseded by this one,
  which measures both `ce0d97f` and the parser replacement. Beyond the re-measurement, seven
  errors in it were found on review and every one is fixed above rather than footnoted:
  - it cited Judged `37eafac` in its header, a commit at which **no root-set code exists**.
    The root set landed in `ce0d97f`, which is what was measured. Corrected in the header.
  - its headline 3 claimed Tier B reports `covered: true` and `gaps: 0` while emitting nothing
    on three repositories. True of one, zod. Narrowed in section 4.2.
  - "143 entry points" and "141 test modules and 2 conftests" for pip were not re-derivable.
    The counts at the pinned SHA are **140** test modules and 2 conftests. Corrected in 4.2.
  - "zod holds 10 app-router entry points" — the count at the pinned SHA is **11**. Corrected
    in 4.2.
  - it counted 67 Tier B roots without disclosing that **no framework version was ever
    resolved**, which §5.1 makes Tier B's correctness condition. Disclosed, with the declared
    strings, in 4.2.
  - its section 6 quoted verbatim `cargo test` output for two tests that exist nowhere.
    Deleted and replaced with real captured output from a harness described in place.
  - its headline 1 called R8's flag-rate half "settled". R8 carries no numeric threshold for
    "tolerable", so nothing can settle it. Softened to what was measured, in the headline and
    in section 8.
  - it linked to none of the three earlier eval documents, breaking a chain each of them
    maintains. This section is that link.

---

## 10. Limits

**Fire rate is not precision. No labels were collected.** Every number in section 3 counts
blocks. A block is correct whenever the file really is referenced, and on real repositories
most files really are referenced — so a high fire rate is consistent with a gate that is
almost always right. What the measurement establishes is **usability**: at `basename+stem`
most source-file claims on a real repository do not survive Gate 2a. It establishes nothing
about how many blocks are wrong. **This document measures whether the layers are usable on
real code, not whether they are right.**

**Nine repositories is nine repositories.** They span the E5 shapes and they are not a random
sample of anything. Six of the nine are library-shaped and three are application-shaped;
none is a decade-old enterprise monolith, none has squashed history, none is over 1043
tracked files. §10 E5 asks for variance to be reported rather than pooled, which sections 3
and 5 do, and the variance is large: `+stem` on path-source spans 12.5% to 100%, and Tier A
per repository spans 5 to 299.

**The root-set totals are dominated by one repository.** otel-demo contributes 306 of 854
roots, and 99 of those are wrong (4.3). Corpus-level Tier A figures should be read per
repository, not pooled.

**Section 3.3 was not re-derived and section 3.2's kinds-fired column was not either.** See
the note at the end of section 2. Sections 3.1 and 3.2's percentages were re-measured and
reproduce exactly.

**The candidate populations are samples, not censuses.** 100 paths and 100 symbols per
repository, taken by a fixed stride over sorted `git ls-files`. A stride is not a random
sample; it correlates with directory order, which correlates with the parent-directory needle
this round is measuring. cobra and sample-controller were taken whole, so their numbers are
censuses; the other seven are not.

**The symbol extractor is a line-prefix scan.** It finds `def foo`, `pub fn foo`, `func foo`,
`export function foo` and a handful of siblings, at any indentation, with a name of four
characters or more. It misses generated symbols, macro-defined symbols, class attributes,
Go methods on unusual receivers, and everything in a language it has no prefixes for.

**Every clone is shallow, and one gate refuses because of it.** Gate 2e contributes nothing
to any number here except section 6, by design — it refused every candidate. A corpus of full
clones would let 2e be measured and would change nothing in sections 3, 4, 5 or 7, none of
which reads history.

**The `path-source` extension list is a judgement.** Twelve extensions, chosen to match what
the four E2 analyzers read. A repository whose live code is `.rb`, `.php`, `.cs` or `.kt`
contributes nothing to the path-source population, and none of the nine has much of any of
them.

**The hand-check is 47 roots of 854, and it was done by the same person who ran the tool.**
Its mechanical half is independent — a different parser re-resolving the same key — but the
sampling, the classification and the judgement of what "correct" means are not. At stride 20
it draws 16 roots from otel-demo and 1 each from cobra and sample-controller, so the
repository carrying the defect is also the best-sampled one; the 7/47 error rate is therefore
not a corpus-wide error rate, and the census in 4.3 is the number to quote instead.

**The sweep harness and the shallow-refusal harness are not in the repository.** Both link
`judged-core` and call shipped code unmodified — `LiteralVeto::query` and
`RecencyVeto::judge`, neither wrapped nor reimplemented — but they were written for this round
and live in scratch outside the repository, alongside the raw captures behind sections 3, 5, 6
and 7. The corpus itself was deleted at the end of the round. So sections 3 and 6 are
reproducible from the recipes above rather than by re-running a committed binary, and nothing
in CI re-derives them.

**The "after" state is an uncommitted working tree.** The measurement was taken against
`ce0d97f` plus the parser replacement described in the header, which is the change this
document ships alongside. Anyone re-running section 5 must do it at the commit that carries
both, not at `ce0d97f`.
