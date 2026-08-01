# Out of sample: Gate 2a's flag rate and the root set, on nine real repositories

**Date:** 2026-08-02 · **Judged:** `37eafac` · **Toolchain:** rustc 1.94.1, git 2.50.1 ·
**Corpus:** nine `--depth 1` clones, §10 E5's shapes, listed in section 1

> Everything Judged had measured before this document lived on nineteen fixtures written
> from the same research document that specifies Judged's rules. §10 E5 says six popular
> Python libraries are far too homogeneous and names what a defensible corpus needs. This is
> the first round on code nobody wrote a fixture for.

**Three results, and the second and third are the ones that matter.**

1. **§11 R8's flag-rate half is settled, against every strategy in it.** On the population an
   analyzer actually claims — source files — the shipped `basename+stem` blocks a median
   **84.6%** of candidates, and `+parent-dir` blocks **100% on seven of the nine repositories**.
   The in-repo finding that `parent-dir` fires at 100% on Judged's own 70 files was not an
   artifact of a small repository. It is what that needle does. R8 asks for the strategy whose
   fire rate is tolerable; in the strategy space R8 defines there is no such strategy.
2. **The Tier A root scan returns zero roots on five of the nine repositories**, and it does so
   because a single unparseable manifest voids the whole scan. Seven of the nine repositories
   contain at least one manifest the parsers reject, and in every case the file is valid.
   Five distinct parser defects account for all seven; each is reproduced below on a file of
   under twelve lines.
3. **Tier B reports `covered: true` and `gaps: 0` while emitting nothing**, on repositories
   holding 10, 16 and 143 entry points of exactly the shape Tier B exists to materialize. A
   fabricated root hides a real gap, which §9.5 says; a fabricated *absence* of gaps is the
   same failure with the sign flipped, and it is the one this round found.

Gate 2e's shallow-clone refusal, the one thing here that was expected to break and did not,
is in section 6. It refused on all 27 candidates in all nine clones, and abstained in a full
clone.

---

## 1. The corpus

Cloned into a scratch directory outside the repository and deleted at the end of the round.
Every clone is `--depth 1 --single-branch`, which is deliberate: §6.19 says shallow clones are
the CI default and that they silently void every VCS signal, so the corpus doubles as a test
of the refusal (section 6).

```sh
cd /Users/neo/.blackhole/Judged/2026-08-02/corpus
git clone --depth 1 --single-branch https://github.com/psf/requests.git              requests
git clone --depth 1 --single-branch https://github.com/pypa/pip.git                  pip
git clone --depth 1 --single-branch https://github.com/django/djangoproject.com.git  djangoproject
git clone --depth 1 --single-branch https://github.com/colinhacks/zod.git            zod
git clone --depth 1 --single-branch https://github.com/spf13/cobra.git               cobra
git clone --depth 1 --single-branch https://github.com/prometheus/node_exporter.git  node_exporter
git clone --depth 1 --single-branch https://github.com/kubernetes/sample-controller.git sample-controller
git clone --depth 1 --single-branch https://github.com/open-telemetry/opentelemetry-demo.git otel-demo
git clone --depth 1 --single-branch https://github.com/BurntSushi/ripgrep.git        ripgrep
```

| Repo | Commit | Tracked files | Tracked bytes | E5 shape it covers |
| --- | --- | ---: | ---: | --- |
| `psf/requests` | `414f0513c33883adf6f2b46901d4f0b38a455851` | 130 | 4.5 MB | Python library |
| `pypa/pip` | `6236392d41f0623476b9dbca2f1c55b832ee7e43` | 1043 | 15.4 MB | Python, **vendored tree** (297 files under `src/pip/_vendor/`) |
| `django/djangoproject.com` | `4744e9ef8f27c113586fd062d7028e70d6554340` | 645 | 9.1 MB | **Django-shaped app** (4 `apps.py`, real `INSTALLED_APPS`) |
| `colinhacks/zod` | `912f0f51b0ced654d0069741e7160834dca742ee` | 583 | 13.9 MB | **TypeScript**, pnpm workspace, 8 packages |
| `spf13/cobra` | `adbc8813901bba65827259daa8e22ff94ec1f30e` | 66 | 0.7 MB | **Go** library, flat layout |
| `prometheus/node_exporter` | `b401dcfc667cee0a5d29232bab51a8ce1c58ec07` | 405 | 2.8 MB | Go application, deep `collector/` tree |
| `kubernetes/sample-controller` | `3e50bfd72c521dd4b1b9d832b9f2e6254b4ff148` | 58 | 0.4 MB | Go, **heavy codegen** (31 of 58 files generated) |
| `open-telemetry/opentelemetry-demo` | `f7408a50aac60bd848dda33f7df5db43503e4b7a` | 584 | 8.5 MB | **Polyglot monorepo**, protobuf codegen, 3 frameworks detected |
| `BurntSushi/ripgrep` | `435f59fc4b43af3ab32f34d53fa34978f393fe52` | 237 | 3.3 MB | **Rust** workspace |

`node_exporter` was picked expecting a `vendor/` directory and no longer has one; Go modules
removed vendoring from most of the ecosystem. `pip` carries the vendored tree instead, which
is why the corpus is nine repositories rather than eight.

---

## 2. Reproducing this

Two programs. `judged show-roots` is in the repository. The fire-rate sweep is not — nothing
in the shipped CLI measures a flag rate over a whole repository, so the sweep is a throwaway
crate outside the repository that links `judged-core` and calls the same
`LiteralVeto::query` the gate calls, with `ScanLimits::default()` and no other configuration.

```sh
# Root set, per repo. Timings in section 5 are the minimum of five runs.
for r in corpus/*/; do judged show-roots --json "$r" > "roots.$(basename $r).json"; done

# Fire rate, per repo. 100 candidates per population, sampled deterministically.
for r in corpus/*/; do firerate "$r" 100 > "sweep.$(basename $r).tsv"; done
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
inferred. Section 8 states what follows from that and what does not.

The harness is graded before it is trusted: `tests/known_repo.rs` builds a five-file
repository whose verdicts are worked out by hand in the test's own doc comment — basename
1/4, `+stem` 1/4, `+parent-dir` 3/4 — and asserts them. It reproduces all three.

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
populations, no exceptions. A path candidate carries no symbol name, so the fourth needle
derives nothing, and R8's four-way sweep is a three-way sweep out of sample exactly as it was
in sample. It separates only on symbol claims, and there by very little: seven of nine
repositories are already at 100% before it is added, and the two that are not move 44% → 53%
(cobra) and 92% → 94% (sample-controller).

**The shipped default is not in a different regime from the strategy it was chosen over.**
`basename+stem` blocks a median 84.6% of source-file claims — 100% on requests, 94.9% on
djangoproject, 93.2% on ripgrep, 87.9% on pip. Four of the nine are above 87%. The
in-sample round found `+stem` "free at this scale" and it was, on ten-file fixtures; on real
code it is the needle that carries most of the blocking. On symbol claims it is worse: a
median 95%, and 100% on three repositories.

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

## 4. What crashed, timed out, or produced obviously wrong output

Nothing crashed. Nothing timed out. Every `judged show-roots` exited 0; every sweep exited 0
with an empty stderr; no scan hit `ScanLimits`, and **zero** of the blocks in section 3 came
from an incomplete scan (the `vetoed_incomplete` column is 0 in all 108 sweep rows), so every
figure there is a literal hit rather than an abstention counted as one.

The wrong output is all in the root set.

### 4.1 One unreadable manifest voids every Tier A root in the repository

Seven of the nine repositories contain at least one manifest the parsers reject. The scan is
all-or-nothing, and its own message says so:

> `Cargo.toml:123: unknown escape `\<newline>`. One unreadable manifest fails the whole Tier A
> scan, so EVERY machine-declared root is missing from this list — not just this package's.`

The consequence, measured by deleting the offending files and re-running:

| Repo | Rejected manifest(s) | Roots as-is | Roots once the file is deleted | Tier A lost |
| --- | --- | ---: | ---: | ---: |
| ripgrep | `Cargo.toml` | 0 | 76 (A 76) | **≥ 76** |
| node_exporter | `.github/workflows/golangci-lint.yml` | 0 | 68 (A 68) | **68** |
| pip | `.github/workflows/ci.yml` | 0 | 28 (A 28) | **28** |
| requests | `.github/workflows/run-tests.yml` | 10 (B 10) | 37 (A 27, B 10) | **27** |
| sample-controller | `go.mod` | 0 | 4 (A 4) | **≥ 4** |
| cobra | `.github/workflows/test.yml` | 0 | 2 (A 2) | **2** |
| otel-demo | 6 workflows + `src/checkout/go.mod` + `src/product-catalog/go.mod` | 7 (B 7) | 268 (A 261, B 7) | **≥ 261** |

The two `≥` rows are floors: deleting `Cargo.toml` or a `go.mod` removes the roots that
manifest itself declares, so a working parser yields more than the recovery figure, not fewer.

**Five of nine repositories report zero roots.** cobra, node_exporter, pip, ripgrep and
sample-controller all have a `main` entry point, a package manifest, and CI that runs
commands, and `show-roots` prints nothing but a gap.

Five defects account for all seven repositories. Each is reproduced below on a file written
from scratch; the repro directories were built, run, and deleted with the corpus.

**(a) A YAML key whose only value is a trailing comment.** node_exporter
`golangci-lint.yml:17`, pip `ci.yml:310`, otel-demo `checks.yml:24` — three of the seven.

```yaml
name: t
on: push
permissions:  # a trailing comment
  contents: read
jobs:
  gate:
    runs-on: ubuntu-latest
    steps:
      - run: echo hi
```

```
{"root_count":0,"gaps":[".github/workflows/ci.yml:4: unexpected indentation. ..."]}
```

Delete four words — the comment on line 3, nothing else — and re-commit:

```
{"root_count":1,"gaps":[]}
```

The comment is being read as the scalar value of `permissions:`, so the nested block on the
next line arrives where no block is expected.

**(b) `jobs.<id>.defaults.run` is a mapping, `jobs.<id>.steps[].run` is a scalar.** cobra
`test.yml:98`.

```yaml
jobs:
  gate:
    defaults:
      run:
        shell: bash
    steps:
      - run: echo hi
```

```
{"root_count":0,"g":[".github/workflows/ci.yml#jobs.gate.defaults.run: must be a s..."]}
```

Both keys are spelled `run` and the parser treats every one of them as a step command.

**(c) TOML line continuation inside a multi-line basic string.** ripgrep `Cargo.toml:122`,
where the Debian `extended-description` opens with `"""\`. A trailing backslash before a
newline is valid TOML 1.0 — it trims the newline and the leading whitespace of the next line.

```toml
[package]
name = "x"
version = "0.1.0"
description = """\
line one
line two
"""
```

```
{"root_count":0,"g":["Cargo.toml:5: unknown escape `\<newline>`. One unreadable manifest f..."]}
```

**(d) `godebug` is a real `go.mod` directive.** sample-controller `go.mod:7`, and
`go.mod` there is itself generated (`// This is a generated file. Do not edit directly.`).
The directive has existed since Go 1.23.

```
module example.com/x

go 1.24.0

godebug default=go1.24
```

```
{"root_count":0,"g":["go.mod:5: `godebug` is not a go.mod directive. One unreadabl..."]}
```

**(e) YAML flow mappings are not read.** requests `run-tests.yml:20`, which excludes one
matrix combination with `- { python-version: "pypy-3.11", os: "windows-latest" }`.

```yaml
jobs:
  gate:
    strategy:
      matrix:
        exclude:
        - { os: "windows-latest" }
    runs-on: ubuntu-latest
    steps:
      - run: echo hi
```

```
flow-map:  {"root_count":0,"g":[".github/workflows/ci.yml:8: trailing text after a quoted scala..."]}
block-map: {"root_count":1,"g":[]}
```

The second line is the same data written as a block mapping. Nothing else in the file changed.

**On the all-or-nothing design itself.** Refusing wholesale rather than emitting a partial
root set is the right instinct — a root set silently missing a package is the §6.20 shape,
and the gap message is exact about what happened. But the blast radius is a repository, the
trigger is any one of hundreds of files, and on this corpus it fires 78% of the time. Every
figure in section 5's Tier A column is a figure about the parsers, not about Tier A.

### 4.2 Tier B reports full coverage while emitting nothing

`next` is detected in zod (15.5.15) and otel-demo (16.2.12), both marked `covered: true`, and
**zero** Next.js roots are emitted from either. zod's report ends:

```
frameworks: next 15.5.15
manifests read: 14
files scanned: 580
roots: 175 — 175 tier A, 0 tier B, 0 tier C
could not resolve: 0 — every framework recognized was covered, every manifest parsed, and
every declared entry decided something.
```

zod holds 10 app-router entry points (`packages/docs/app/(doc)/[[...slug]]/page.tsx`,
`app/api/search/route.ts`, `app/layout.tsx`, …). otel-demo holds 16 pages-router entry points
(`src/frontend/pages/_app.tsx`, `pages/api/cart.ts`, …). Every one of them is Tier B's exact
subject: a file a framework turns into an entry point with no source reference anywhere.

Two independent causes, both reproduced:

```
root layout      app/page.tsx, app/layout.tsx
  -> {"tierB":["next/app-router-layout","next/app-router-page"],"gaps":0,"covered":true}
nested layout    packages/docs/app/page.tsx, app/layout.tsx
  -> {"tierB":[],"gaps":0,"covered":true}
pages router     pages/index.tsx, pages/_app.tsx, pages/api/cart.ts
  -> {"tierB":[],"gaps":0,"covered":true}
```

- **`app/**` is anchored at the repository root.** Move the same two files to
  `packages/docs/app/` — with `next` in both the root and the package manifest — and the
  rules stop matching. §10 E5 names the polyglot monorepo as a corpus shape the design has to
  survive; a workspace is where a JavaScript framework normally lives.
- **The pages router has no rule.** `Rule` has four `NextAppRouter*` variants and nothing for
  `pages/`. Next 14 and 15 both still support it and otel-demo uses it.

The same shape appears in pip from a different direction: pytest is not detected at all, so
**141 test modules and 2 conftests never become roots**, because pip declares pytest under
PEP 735 `[dependency-groups]` in `pyproject.toml` rather than in a `requirements*.txt`. pip's
gap list contains one entry, the unreadable workflow, and nothing about pytest.

**In all four cases `gap_count` is 0 and, where a framework was recognized, `covered` is
`true`.** §9.5 caps tiers on unresolved hints because a fabricated root hides a real gap.
These are the mirror image: a real gap hidden by a fabricated claim of completeness. The
trailing sentence of the report — *"That is not the same as having recognized everything"* —
is doing more work than a reader can reasonably be expected to give it, because the sentence
before it says every recognized framework was covered, and that sentence is false.

### 4.3 A root with an empty target

```
A	packaged_file	Dockerfile#copy@41[0]	<empty>
```

djangoproject's `Dockerfile:41` is `COPY . .`. `COPY ./requirements ./requirements` on line 29
resolves correctly to `requirements`; `COPY . .` yields a root whose target is the empty
string. One root in 264, and it is the only malformed one on the corpus, but an empty target
is not a root — either it is `.` or it is a gap saying a whole-tree copy cannot be resolved
to seeds.

---

## 5. The root set, per repo

Timings are the minimum of five runs of `judged show-roots --json`, warm cache.

| Repo | Roots | A | B | C | Gaps | Files scanned | Time |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| zod | 175 | 175 | 0 | 0 | 0 | 580 | 14 ms |
| djangoproject | 72 | 22 | 50 | 0 | 0 | 645 | 16 ms |
| requests | 10 | 0 | 10 | 0 | 1 | 128 | 5 ms |
| otel-demo | 7 | 0 | 7 | 0 | 2 | 574 | 17 ms |
| cobra | 0 | 0 | 0 | 0 | 1 | 66 | 3 ms |
| node_exporter | 0 | 0 | 0 | 0 | 1 | 404 | 8 ms |
| pip | 0 | 0 | 0 | 0 | 1 | 1037 | 17 ms |
| ripgrep | 0 | 0 | 0 | 0 | 1 | 236 | 7 ms |
| sample-controller | 0 | 0 | 0 | 0 | 1 | 58 | 4 ms |
| **total** | **264** | **197** | **67** | **0** | **8** | | |

**Speed is not the problem.** 17 ms over a 1043-file repository. No configuration of this
module is going to be too slow to run in CI.

**No repository produced a Tier C root, and that is correct.** Tier C is solicited from a
human and committed; none of these repositories has been asked. An empty Tier C column on a
corpus nobody has interviewed is the honest answer, and it is worth noting that the module
did not invent one.

**Only two of the nine repositories produced any Tier A root at all**, and the seven zeros
are section 4.1's, not a property of the repositories.

---

## 6. §6.19: the shallow-clone refusal behaved

The corpus is nine `--depth 1` clones, in which every file reports the same grafted commit.
Gate 2e is supposed to refuse rather than answer.

```
running 2 tests
test a_full_clone_does_not_report_shallow_history ... ok
refused 27 candidates across the shallow corpus
test every_shallow_corpus_repo_refuses_with_shallow_history ... ok

test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.58s
```

Three files per repository, spread across the tree by the same stride the sweep uses. All 27
returned `Vetoed(ShallowHistory)` — not a stale timestamp read as recency, not an
`EvidenceUnavailable` shrug, the specific refusal. `Repo::is_shallow` answered `true` for all
nine. The contrast case is the Judged working clone, which is complete: `is_shallow` is
`false` and no candidate is refused for shallow history, so the assertion is not passing
because the gate refuses unconditionally.

**This is the one layer the corpus was expected to break and it did not.** It is also the
only layer here that gets no credit for being right, because refusing is all it does; the
value of the refusal is that the other gates never see a history-derived number that was
never computed.

---

## 7. The hand-checked sample

A count of roots says nothing about whether they are roots. 32 were sampled deterministically
— every 14th of zod's 175, every 7th of djangoproject's 72, every 2nd of requests' 10, every
3rd of otel-demo's 7 — and each was checked against the file it claims to come from.

**29 of 32 correct, 2 correct but unresolved, 1 wrong.**

| Verdict | n | What was checked |
| --- | ---: | --- |
| Correct | 29 | The origin file exists, the cited key holds the cited value, and the target names something real |
| Correct declaration, unresolved target | 2 | zod `exports["./v4/locales/*"].require` → `packages/zod/v4/locales/*`, and `files[1]` → `packages/zod/**/*.js`. Faithful transcriptions of the manifest; the glob is never expanded, so nothing on disk carries either name |
| Wrong | 1 | `Dockerfile#copy@41[0]` with an empty target (section 4.3) |

Spot checks worth quoting, because they are the ones that could have been fabricated and were
not:

- **Array indices are real indices.** `.github/workflows/release.yml#jobs.build_and_publish.steps[8].run` →
  `pnpm run prepublishOnly`. Counting the steps in that job by hand: checkout, Set up Node,
  Upgrade npm, Install pnpm, `pnpm install`, `pnpm build`, `pnpm test`,
  `pnpm run --filter @zod/resolution test:all`, `pnpm run prepublishOnly`. Index 8, exact
  text. The same held for every one of the nine `ci_action` and `command` roots sampled.
- **Tier B is not guessing at Django.** All 7 sampled Django roots check out:
  `INSTALLED_APPS` in `djangoproject/settings/common.py` really does list `aggregator` and
  `fundraising`; `aggregator/management/commands/send_pending_approval_email.py` and
  `fundraising/management/commands/create_stripe_plans.py` both exist;
  `djangoproject/settings/prod.py`, `docs/urls.py` and `manage.py` all exist and are what
  they are claimed to be.
- **pytest Tier B is exact, not approximate.** requests has exactly 10 files matching
  `tests/(test_*|conftest).py` and exactly 10 pytest roots were emitted — not the 15 files in
  `tests/`, and not a superset. All 8 sampled pytest roots (5 requests, 3 otel-demo) point at
  files that exist.

**So the roots that exist are overwhelmingly real.** The failure mode on this corpus is not
fabrication, it is silence: five repositories with nothing, three frameworks' worth of Tier B
missing with `gaps: 0` next to it. That is the better failure of the two to have, and it is
still the one §9.5 warns about, because a caller cannot tell an empty root set produced by a
clean repository from one produced by a parse error two directories away.

---

## 8. §11 R8: what to do

R8 records two requirements that conflict — §9.3's *block on any hit*, and a tolerable flag
rate — and asks for a measurement rather than an argument. Here is the measurement's answer.

**1. Do not ship `+parent-dir`, and do not keep treating `+symbol` as a distinct row.**
`+parent-dir` blocks 100% of source-file claims on seven of nine real repositories and ≥ 89.7%
on eight of nine. The 100% figure from Judged's own 70 files was not a small-repository
artifact. `+symbol` is byte-identical to it on every path claim in every repository, so for
path claims the sweep R8 asks for has three distinct rows, not four — now an out-of-sample
fact rather than an inference from the flag's own documentation.

**2. Keep the default at `basename+stem`, and stop describing it as a tolerable flag rate.**
Moving it does not help. `basename` alone was rejected in sample for missing E2 classes 1, 8
and 13, and it still blocks a median 27.3% of source-file claims out of sample — so the
narrowest strategy that satisfies R8's rescue half is also, on real code, a strategy that
blocks 84.6% of claims at the median and 100% on one repository in nine. **There is no
strategy in R8's space whose fire rate is tolerable.** R8's flag-rate half cannot be closed by
picking a needle set, which is the opposite of what R8 assumed when it framed the question.

**3. Therefore the next experiment is not another needle sweep.** What this round can support
is narrow: a Gate 2a hit is not, on real code, discriminating enough to be a terminal block on
its own, and the evidence needed to make it discriminating is already carried on every `Hit`
— which needle kind fired, in which file, at which offset. A block citing a `basename` hit in
one file and a block citing a `stem` hit that is a common English word are indistinguishable
in today's report and are not the same evidence. Ranking them is a design question this
document does not answer and did not measure.

**4. And the honest framing for §11 R1.** R1 asks whether an auto-act tier can exist at all,
and names E2 as what answers it. E2 measured what Gate 2 rescues. This measures what it
blocks, and at the shipped configuration it blocks most of what a source-file analyzer would
claim on a real repository. If both numbers hold, the surviving claim set on real code is
small — which is a finding about the size of any auto-act tier, arrived at from the flag-rate
side, and it needs a labelled corpus before it is more than a strong hint.

**5. The root set has to be fixed before it can be measured.** Sections 4.1 and 4.2 are not
tuning; they are four parser defects and two coverage gaps, each reproduced on a file under
ten lines. Until they are closed, "how many roots does Judged materialize on a real
repository" has no measurable answer: five of nine repositories return zero for a reason that
has nothing to do with their root sets. **These were left unfixed on purpose.** The files
involved — `roots/manifest.rs`, `roots/convention.rs` — belong to concurrent work in flight
at the time of the run, and this round's job was to measure the layers as they stand, not to
edit them out from under their author.

---

## 9. Limits

**Fire rate is not precision. No labels were collected.** Every number in section 3 counts
blocks. A block is correct whenever the file really is referenced, and on real repositories
most files really are referenced — so a high fire rate is consistent with a gate that is
almost always right. What the measurement establishes is **usability**: at `basename+stem`
most source-file claims on a real repository do not survive Gate 2a, so an analyzer behind it
has few surviving claims regardless of whether the blocks are correct. It establishes nothing
about how many blocks are wrong. **This document measures whether the layers are usable on
real code, not whether they are right.**

**Nine repositories is nine repositories.** They span the E5 shapes and they are not a random
sample of anything. Six of the nine are library-shaped and three are application-shaped;
none is a decade-old enterprise monolith, none has squashed history, none is over 1043
tracked files, and the largest is 15 MB. §10 E5 asks for variance to be reported rather than
pooled, which sections 3 and 5 do, and the variance is large: `+stem` on path-source spans
12.5% to 100%.

**The candidate populations are samples, not censuses.** 100 paths and 100 symbols per
repository, taken by a fixed stride over sorted `git ls-files`. A stride is not a random
sample; it correlates with directory order, which correlates with the parent-directory needle
this round is measuring. cobra and sample-controller were taken whole, so their numbers are
censuses; the other seven are not.

**The symbol extractor is a line-prefix scan.** It finds `def foo`, `pub fn foo`, `func foo`,
`export function foo` and a handful of siblings, at any indentation, with a name of four
characters or more. It misses generated symbols, macro-defined symbols, class attributes,
Go methods on unusual receivers, and everything in a language it has no prefixes for. The
symbol population in section 3.3 is a sample of declared symbols, and one biased toward
conventionally-declared ones.

**Every clone is shallow, and one gate refuses because of it.** Gate 2e contributes nothing
to any number here except section 6, by design — it refused every candidate. A corpus of full
clones would let 2e be measured and would change nothing in sections 3, 4, 5 or 7, none of
which reads history.

**The Tier A recovery figures in 4.1 are floors, not estimates.** They were obtained by
deleting the unreadable manifest, which for `Cargo.toml` and `go.mod` deletes the roots that
manifest itself declares. What a corrected parser yields is more than the number in the
table, by an unmeasured amount.

**The `path-source` extension list is a judgement.** Twelve extensions, chosen to match what
the four E2 analyzers read. A repository whose live code is `.rb`, `.php`, `.cs` or `.kt`
contributes nothing to the path-source population, and none of the nine has much of any of
them.

**The hand-check is 32 roots of 264, and it was done by the same person who ran the tool.**
It samples zod, djangoproject, requests and otel-demo, which are the only four repositories
that produced roots at all. It says nothing about the correctness of roots on the five
repositories that produced none, because there were none to check.

**The sweep harness is not in the repository.** It links `judged-core` and calls the shipped
`LiteralVeto::query` unmodified, and it is graded against a hand-counted five-file repository
before use — but it was written for this round and deleted with the corpus, so section 3 is
reproducible from the recipe in section 2 rather than by re-running a committed binary.
