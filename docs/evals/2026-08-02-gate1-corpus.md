# What Gate 1 protects: the never-touch inventory on nine real repositories

**Date:** 2026-08-02 · **Toolchain:** rustc 1.94.1, git 2.50.1 · **Analyzers:** vulture 2.16,
knip 6.31.0, `x/tools` deadcode v0.48.0, cargo-shear 1.13.3, node v24.14.0 ·
**Corpus:** the same nine shallow clones at the same SHAs as
[`2026-08-02-out-of-sample-corpus.md`](./2026-08-02-out-of-sample-corpus.md) §1

This round put §9.3's Gate 1 into the pipeline ahead of the reference veto, gave it a human
interface (`judged explain <path>`), and measured it on 3,751 tracked files nobody wrote a
fixture for.

**Three results, and the second one is the one that matters.**

1. **On the E2 catalogue, Gate 1 is redundant.** Stacked on top of `--veto --roots` it prevents
   **zero additional false removals** for all four real analyzers and costs zero decoys. The
   four-analyzer total is 5 over 3 classes with or without it. It prevents exactly one thing no
   other layer does, and only against the `naive` control. That is not a defect in Gate 1; it
   is a fact about §10 E2, whose nineteen classes are built to exercise *reference mechanisms*
   and therefore land squarely in Gate 2's domain. The catalogue contains no `.env`, no
   `terraform.tfstate` and no analyst's `.RData`, so it cannot measure the layer that exists
   for them.
2. **On the corpus, Gate 1 protects 28.4% of tracked files, and 380 of the 1,064 protections
   come from a single sub-rule that is wrong.** §6.17 measured only **3.6%** of canonical
   gitignore patterns as explicitly irreplaceable; a layer protecting 28% of a repository is
   over-firing, and the hand check localises almost all of the excess. Class 1i (legal) treats
   *"declares an SPDX-License-Identifier"* as making a file a compliance artifact. In
   `open-telemetry/opentelemetry-demo`, which mandates a per-file SPDX header, that refuses
   **364 of 584 files** — including every ordinary React component. Removing that one sub-rule
   takes otel-demo from **79.6% to 18.7%** and the corpus from **28.4% to 18.4%**.
3. **The hand check found it, and only the hand check could have.** 47 protected files sampled
   at stride 23: **30 defensible, 17 wrong.** All 17 are the same sub-rule, in the same
   repository. Every aggregate in section 3 looks healthy without it — a 28% protection rate on
   a corpus containing a vendored tree, a codegen-heavy Kubernetes controller and 70 Django
   migrations is not obviously wrong until you read the rows.

**A fourth result was found by running the measurement, not by reviewing the code.**
`judged explain` mis-resolved every path given from outside the working tree, and reported
`UNTRACKED` — rung R9, the most alarming answer it has — for files that were committed. Eleven
green CLI tests did not catch it because all eleven ran from inside the repository. Section 6.

---

## 1. What was built

Three things, all under TDD with the red captured before each implementation.

**Gate 1 in the pipeline** — `judged-mutants/src/gate1.rs`. `judged_core::gate1` holds the
sixteen classes as three modules with three vocabularies (1a–1f `state`, 1g–1k `content`,
1l–1p `contracts`), each answering about its own classes and abstaining about the rest. `Gate1`
is the assembler that asks all sixteen in §9.3's order and returns every class that fired;
`Gate1Sut` is the layer, selected with `judged mutants --gate1`.

**The ordering is the composition, not a convention.** `Gate1Sut` wraps the analyzer and every
later layer wraps *it*, so a claim Gate 1 refuses is never handed to Gate 2 at all. That is what
makes the refusal absorbing, and it is testable without an "overridden" flag anywhere:
`gate1_gate.rs::a_gate_one_refusal_is_absorbing_because_gate_two_is_never_asked` asserts that a
Gate 1 refusal appears in neither the final claim set **nor Gate 2's blocked list**. It also
carries the mirror assertion, because a Gate 1 that consumed the whole claim set would satisfy
the first half perfectly.

The same subset invariant the other two rescue layers carry is asserted here on the claim
**sets**, over all nineteen classes, with a strictness half — a layer that never fires satisfies
a subset assertion exactly, and would be indistinguishable from one that is not wired in.

**`judged explain <path>`** — §9.13 asks for it by name alongside `--why-alive` and
`show-roots`. It prints, in the order §9.3 evaluates them:

```
$ judged explain .env

.env
  in /tmp/demo

RECOVERABILITY (Gate 0g, §8.1) — what git could give back
  class    IGNORED
  rung     R7 at best, R9 by default
  meaning  matched by an ignore rule and never `git add`-ed: zero recovery
           path, and the class §6.17 measured as most likely to hold the
           only copy of something — .env, a dev SQLite database,
           terraform.tfstate.backup
  ignored  `.env` at .gitignore:2

GATE 1 — the never-touch inventory (§9.3)
  INELIGIBLE — 2 of the sixteen classes refuse this path.
  A Gate 1 refusal is absorbing: it is justified by IRREVERSIBILITY, not by
  uselessness, so no later evidence of uselessness moves it.

  1b secrets and identity
      the path matches the dotenv name `.env`, which is not one of the
      template forms (.env.example, .env.sample, .env.template, .env.dist)
  1p the unknown
      no extension, magic signature or path convention determined what this
      file is, and §9.3's 1p rule is that the unknown defaults to keep

EVIDENCE READ
  magic bytes   none matched the signature table
  file type     UNDETERMINED — …which is class 1p above
  ignore rule   `.env` at .gitignore:2

NOT RUN by this command
  Gate 0a–0f  the boundary refusals: symlink traversal, nested repositories, …
  Gate 2      the reference veto — whether anything in the repository NAMES
              this path… `judged mutants --veto` measures it; nothing here has
              asked it.
  Gate 3      artifact and deadness promotion.
  A gate this command did not run is not a gate that abstained. Nothing above
  is evidence that this path is unused (§6.20).
```

**Gate 0g has existed in `judged_core::git` since the first commit and nothing had consumed it
in five rounds.** This is where it surfaces, and it leads the report rather than trailing it,
because §9.3 is explicit that the ordering is the point: *usefulness is irrelevant until
recoverability is known, because the cost of being wrong is set by the rung, not the tier.* The
rung is printed beside the class — `TrackedPushed` → R2–R4, `TrackedUnpushed` → R4 local only,
`Untracked` and `Ignored` → R7 at best, R9 by default — because the class alone is the cheap
half of the answer.

The command never says a file is safe to delete, and it ends by naming the gates it did **not**
run (0a–0f, 2, 3). §6.20 applies to this command's own output: a trace that silently omits a
gate is indistinguishable from one in which that gate abstained.

---

## 2. Gate 1 against the E2 catalogue

Every figure is read out of `--json`: per-run totals from `rescue.false_removals_bare`,
`.false_removals_remaining`, `.decoys_found_bare`, `.decoys_found_rescued`, and per-layer
figures from `rescue.layers[]`. Per-claim evidence comes from `mutants[].gate1.refused_claims[]`,
whose `class` names the §9.3 class that fired and whose `detail` quotes the rule.

### 2.1 Four analyzers, four configurations

| Configuration | False removals | Classes still failing | Decoys recovered |
| --- | ---: | ---: | ---: |
| bare | **10** | 7 | 26 / 33 |
| `--gate1` | 9 | 6 | 25 / 33 |
| `--veto` | 6 | 4 | 19 / 33 |
| `--veto --roots` | **5** | 3 | 19 / 33 |
| `--gate1 --veto --roots` | **5** | 3 | 19 / 33 |

Per analyzer, bare → full stack: vulture 6 → 3, knip 2 → 1, deadcode 2 → 1, cargo-shear 0 → 0.
The three classes that still remove something live are m02 (dynamic import), m11 (reflective
ORM field) and m12 (link-name alias) — the same three
[`2026-08-02-gate2-veto.md`](./2026-08-02-gate2-veto.md) §5 identifies as leaving no literal
trace outside their own declaration.

### 2.2 Gate 1's unique contribution is zero, and that is worth stating plainly

The last two rows above are identical. Measured directly by running `--veto --roots` with and
without `--gate1`:

| SUT | `--veto --roots` | `+ --gate1` | Gate 1's unique prevention | Decoys |
| --- | ---: | ---: | ---: | --- |
| vulture | 3 | 3 | **0** | 10/16 → 10/16 |
| knip | 1 | 1 | **0** | 2/6 → 2/6 |
| deadcode | 1 | 1 | **0** | 2/2 → 2/2 |
| cargo-shear | 0 | 0 | **0** | 5/9 → 5/9 |
| naive (control) | 4 | 3 | **1** | 26/31 → 26/31 |

Run alone, `--gate1` prevents one of knip's two false removals. It refuses both of m14's
`dist/widget.*.js` files under 1j — Linguist's `vendor.yml` lists `(^|/)dist/` — and one of the
two is the live asset (a prevented false removal) while the other is the planted decoy (a lost
find). m14 goes from failing to clear, and knip's decoy recall from 4/6 to 3/6. Gate 2 clears
m14 as well. Because the stack attributes a rescue to the layer that ran **first**, and Gate 1
runs first, the combined report credits Gate 1 with claims the veto would also have caught; the
table above is the counterfactual that corrects for it.

Against the `naive` control the stack goes 20 → 3 false removals (Gate 1 5, roots 4, veto 8),
which is the best figure the suite has recorded. Gate 1's fourteen refusals there are worth
reading because they are the shape it exists for: `media/customer/thumb_00042.png` and
`media/catalog/product/placeholder.jpg` under 1g, `var/broker/celery-default.jsonl` under 1f,
`.vscode/settings.json` under 1j, `internal/collector/asm_stub.s` under 1p.

### 2.3 Two collisions the E2 run exposed

**`vendor/site-packages/zzz_ledger_bootstrap.pth` is refused under 1e — models, weights and
checkpoints — because `.pth` is PyTorch's checkpoint extension.** It is a Python path
configuration file. The refusal is right and the class is wrong, and a report that told an
operator to go and check a model registry for it would waste their time. 1j also fires on the
same file, so the conflict list is not misleading, but the headline class is.

**`.vscode/ipch.db` is refused under 1d for the `.db` extension.** That file is a *decoy*: a
genuinely dead Visual Studio precompiled-header cache. Gate 1 refusing it is the designed cost
— a tool cannot tell a build cache's `.db` from a production SQLite file without opening it, and
opening it is what 1d's magic-byte rule does when there are bytes to read. It is the single
decoy the naive control loses to Gate 1 (31 → 30).

---

## 3. The corpus census

Nine repositories, the SHAs in
[`2026-08-02-out-of-sample-corpus.md`](./2026-08-02-out-of-sample-corpus.md) §1, every tracked
file (`git ls-files`) judged by all sixteen classes. 3,751 files. **Zero errors and zero scan
gaps** — `StateGate::survey` completed on every tree.

| Repo | Tracked | Protected | % | Without the 1i SPDX-header sub-rule | % |
| --- | ---: | ---: | ---: | ---: | ---: |
| `psf/requests` | 130 | 52 | 40.0% | 52 | 40.0% |
| `pypa/pip` | 1043 | 92 | 8.8% | 76 | 7.3% |
| `django/djangoproject.com` | 645 | 136 | 21.1% | 136 | 21.1% |
| `colinhacks/zod` | 583 | 50 | 8.6% | 50 | 8.6% |
| `spf13/cobra` | 66 | 8 | 12.1% | 8 | 12.1% |
| `prometheus/node_exporter` | 405 | 153 | 37.8% | 153 | 37.8% |
| `kubernetes/sample-controller` | 58 | 35 | 60.3% | 35 | 60.3% |
| `open-telemetry/opentelemetry-demo` | 584 | **465** | **79.6%** | 109 | 18.7% |
| `BurntSushi/ripgrep` | 237 | 73 | 30.8% | 73 | 30.8% |
| **total** | **3751** | **1064** | **28.4%** | **692** | **18.4%** |
| **median** | | | **30.8%** | | **21.1%** |

### 3.1 Which class does the work

| Class | Files where it is the **only** reason | Files where it fires at all |
| --- | ---: | ---: |
| 1b secrets and identity | 4 | 12 |
| 1f downloaded or acquired data | 12 | 12 |
| 1i legal | **416** | **449** |
| 1j vendored / generated / submodule / LFS | 104 | 211 |
| 1k migrations | 70 | 70 |
| 1l platform contracts | 14 | 121 |
| 1m un-ignored by a `!` negation | 0 | 1 |
| 1p the unknown | **296** | 337 |

**Six of the sixteen classes never fired at all:** 1a external effectors, 1c infrastructure
state, 1d local databases, 1e models and checkpoints, 1g user-generated content, 1h session and
scratch state. That is not a defect and it is not a clean bill of health — it is what the corpus
is. These six are overwhelmingly about *untracked and ignored* files, and this census walks
`git ls-files`. A public repository does not commit its `.env`, its `terraform.tfstate` or its
analyst's `.RData`, which are the files §8.1 says are unrecoverable and §6.17 says are the ones
canonical gitignore templates discard. **The classes Gate 1 exists for are the ones this
measurement structurally cannot reach.** Measuring them needs working trees with real untracked
state, which no public corpus provides; that is section 7's open item.

### 3.2 Gate 0g over the corpus

Every one of the 3,751 files classifies as `TRACKED_UNPUSHED` — rung **R4, local only**. The
clones are shallow fetches of a recorded SHA at detached `FETCH_HEAD`, so `HEAD` is on no remote
branch. That is not an artifact of the harness: §6.19 says shallow is the CI default, and it
means the same repository that would classify at R2–R4 on a developer's laptop classifies one
rung lower in CI. It composes with the Gate 2e shallow-clone refusal that
[`2026-08-02-out-of-sample-corpus.md`](./2026-08-02-out-of-sample-corpus.md) §6 measured
refusing all 27 candidates across these same nine clones.

---

## 4. The hand check

**Population:** the 1,064 protected files, ordered by repository (corpus-document order) then by
path. **Stride 23, offset 0**, giving 47 rows — the same sample size the root-set hand check
used, for comparability. The sample and every class it reports reproduce exactly through the
shipped command: all 47 were re-derived with `judged explain --json` and matched the census
row-for-row, 0 mismatches.

**Criterion:** a protection is *defensible* if a careful engineer, shown the evidence Gate 1
prints, would agree the file should not be deleted by an automated cleaner.

**Result: 30 of 47 defensible, 17 wrong.**

### 4.1 The 30 that are right

They divide cleanly:

- **Compliance artifacts by name** (3): `requests/LICENSE`,
  `pip/src/pip/_vendor/distlib/LICENSE.txt`, `ripgrep/crates/globset/COPYING`. 1i doing exactly
  its job.
- **Migrations** (4): every `djangoproject/*/migrations/*` row, including
  `releases/migrations/__init__.py` — refused because it *sits in* a migrations directory of 4
  entries. Deleting that file breaks Django's migration discovery outright, and no reference
  analyzer would flag it as anything but dead.
- **Platform contracts** (10): `Procfile` (Heroku process types), `.github/CODEOWNERS`
  (*"deleting it silently removes required-review enforcement — a security control"*),
  `.github/workflows/*.yml`, `djangoproject/.../security.txt`, `requests/.coveragerc`, and four
  `.gitignore` files whose 1l reason is the best sentence in the whole registry: *"read by git,
  on every future `add` — deleting it the next `git add -A` commits .env and every other ignored
  secret."*
- **Checked-in codegen** (2): `sample-controller/pkg/generated/.../doc.go` (`// Code generated by
  client-gen. DO NOT EDIT.`) and `otel-demo/src/currency/build/generated/.../health.grpc.pb.cc`
  (`// Generated by the gRPC C++ plugin.`). Both by content, not by path.
- **Private keys** (1): `requests/tests/certs/mtls/client/client.pem`. A test fixture, so 1b's
  *remediation* — "escalate for rotation" — is wrong for it. The refusal is still right: a
  cleaner cannot tell a test key from a live one, and this is the direction to be wrong in.
- **Acquired data** (1): `ripgrep/benchsuite/runs/2016-12-24-.../raw.csv`. Timing measurements
  recorded in 2016 on machines that no longer exist. §9.3 1f, and a good catch — though the rule
  that found it is *"the path matches the `.csv` extension"*, which would fire on a generated CSV
  just as readily.
- **Genuinely undeterminable types** (9): `pip/tools/vendoring/patches/pygments.patch` (a
  vendoring patch re-applied on every vendor refresh), four `node_exporter/collector/fixtures/
  proc/*` kernel captures, a Prometheus exposition fixture, `zod/.../site.webmanifest`,
  `node_exporter/.../use.libsonnet` and one `.md.j2` template. Right outcome, reached by
  ignorance rather than by knowledge — see 4.3.

### 4.2 The 17 that are wrong, and they are one rule

Every failure is class 1i firing on *"declares `SPDX-License-Identifier: Apache-2.0` on line 2"*,
and every one is in `open-telemetry/opentelemetry-demo`:

```
src/frontend/components/CartItems/CartItems.tsx        [1i] declares SPDX-License-Identifier: Apache-2.0 on line 2
src/frontend/pages/index.tsx                           [1i] ...
src/cart/src/services/HealthCheckService.cs            [1i] ...
src/ad/src/main/java/oteldemo/problempattern/CPULoad.java  [1i] ...
src/quote/src/Application/Settings/Settings.php        [1i] ...
test/telemetry/test_traces.py                          [1i] ...
    … 11 more, spanning .tsx .ts .java .cs .php .py .ex .exs .yml .yaml Dockerfile
```

`CartItems.tsx` is an ordinary React component. §9.3 lists *"SPDX headers"* under 1i, and the
implementation reads that as *a file carrying an SPDX header is a legal artifact*. The correct
reading is the other one: **the legal artifact is the header, not the file.** A cleaner must not
strip the header while editing; that does not make the file it sits in ineligible for deletion.

The consequence is exactly the failure mode §6.17's 3.6% figure warns about. Any repository
practising REUSE-style per-file licensing — which is the entire CNCF and Linux Foundation
ecosystem — has its whole source tree declared never-touch, on a property that has nothing to do
with the file. And the direction of the error is the expensive one for a *safety* layer: it is
not that Gate 1 permits too much, it is that it protects on a reason that is unfalsifiable, so
an operator reading the report learns nothing they can act on.

Corpus-wide the sub-rule accounts for **380 of 449 1i hits (85%)**, in two repositories:
otel-demo 364, pip 16. Every other 1i hit is a named compliance file — `LICENSE` 43, `UNLICENSE`
11, `COPYING` 5, `AUTHORS` 3, `NOTICE` 2, `MAINTAINERS` 2, `OWNERS` 1, `COPYRIGHT` 1, one
`*.cdx.json` SBOM.

**Recommendation, not applied here:** 1i should protect a file for *being* a compliance
artifact, and protect an SPDX header as an edit-level constraint on the file that carries it.
The change belongs to whoever owns `judged-core/src/gate1/content.rs`; this document does not
touch it, and every number above is measured against the code as it stands.

### 4.3 1p is the second-largest source, and its over-firing is by design

1p — *the unknown defaults to keep* — is the sole reason for 296 files, 7.9% of the corpus,
ranging from 2% of cobra to **34% of node_exporter**. That is not the same kind of finding as
4.2. §9.3 chooses this: a file whose type cannot be determined is not a candidate, and the
module doc says the other fifteen classes exist to buy back the recall it costs.

What the census shows is where the cost actually falls:

The largest groups, of 337 files where 1p fires:

| Extension | 1p hits | What they are |
| --- | ---: | --- |
| *(no extension)* | 149 | `node_exporter/collector/fixtures/proc/*` kernel captures; `ripgrep/benchsuite/runs/*/summary` |
| `.po` | 33 | gettext translation catalogues |
| `.in` `.mdx` `.prom` `.out` | 66 | autoconf inputs, MDX docs, Prometheus exposition fixtures |
| `.patch` `.libsonnet` `.j2` `.jsonnet` | 26 | vendoring patches, jsonnet libraries, Jinja templates |
| `.key` `.csr` `.cnf` | 11 | already refused by 1b as well |

Several of these are well-known types absent from `KNOWN_EXTENSIONS` — `.po`, `.libsonnet`,
`.mdx`, `.patch`. Adding them would move those files out of 1p and into whatever the other
fifteen classes say, which for most is *nothing*. That is a recall improvement with a real
safety cost and it is a decision for whoever owns `contracts.rs`, not a bug. The number to carry
forward is that **1p alone would protect 7.9% of a real corpus**, against §6.17's 3.6% for
"explicitly irreplaceable".

---

## 5. Reproducing this

The census used a throwaway binary over `judged_mutants::gate1::Gate1`, which is not shipped and
was deleted after the run. Every row it produced is reproducible from the shipped command, and
the 47-row sample was verified that way — all 47 re-derived through `judged explain --json`,
zero mismatches against the census:

```sh
# the corpus — the pin() recipe and SHAs are in 2026-08-02-out-of-sample-corpus.md §1
cd corpus/otel-demo
git ls-files -z | xargs -0 -n1 judged explain --json \
  | jq -r 'select(.gate1.disposition=="INELIGIBLE")
           | [.path, (.gate1.findings | map(.class) | join(","))] | @tsv'
```

`judged explain --json` carries `recoverability.class`, `recoverability.rung`,
`gate1.disposition`, `gate1.findings[].class/.title/.evidence`, `evidence.magic`,
`evidence.type_signal`, `evidence.ignore_rule`, `scan_gaps` and `gates_not_run`.

E2 figures: `judged mutants --sut <sut> [--gate1] [--veto] [--roots] --json`. The layer rows are
`rescue.layers[]` and the per-class conflict list is `mutants[].gate1.refused_claims[]`. Each
layer's `claims_judged` is **that layer's** denominator, not the accuser's — under `--gate1
--veto` the veto is handed only what Gate 1 passed through, and the composition check in
`compare_runs` refuses to publish a report where one layer's `survived` is not the next layer's
`claimed`.

---

## 6. A bug the measurement found and the tests did not

`judged explain` joined the path it was given onto the discovered working tree root, which is
correct only when the two are already relative to each other. Run from a directory above the
repository — which is how the corpus measurement runs it — `judged explain corpus/requests/
LICENSE` asked the gates about `<root>/corpus/requests/LICENSE`, a path that does not exist.

The output was not an error. It was:

```
RECOVERABILITY (Gate 0g, §8.1)
  class    UNTRACKED
  rung     R7 at best, R9 by default
GATE 1
  1d local databases and persistence
      the head of this file could not be read (No such file or directory (os error 2)),
      so the content sniff that 1b, 1c, 1d, 1e and 1f all rely on could not run
```

Both lines are confident and both are wrong: the file is committed, and it is a `LICENSE`.
Nothing in the report said the file it was describing was not the file it was asked about — the
§6.20 shape, produced by the command written to prevent it.

Eleven CLI tests covered `explain` and all eleven passed, because all eleven ran from inside the
repository with a repo-relative path. The regression test
(`explain_resolves_a_path_given_from_outside_the_working_tree`) runs from the parent directory
and asserts on the rung, and the fix canonicalises the path — or its parent, when the file does
not exist, which is precisely the case somebody deciding whether to restore something is in.

---

## 7. What this does not establish

- **Gate 1's most important classes are unmeasured.** Six of sixteen never fired, and they are
  the six about untracked and ignored state — the population §8.1 calls unrecoverable. A corpus
  of tracked files in public repositories cannot contain them. Until Gate 1 is measured on
  working trees with real `.env` files, real `terraform.tfstate` and real local databases, the
  numbers here describe the classes that happen to be reachable, not the ones that matter.
- **The E2 result is not evidence that Gate 1 is unnecessary.** It is evidence that §10 E2 does
  not test it. Reading 2.2 as "Gate 1 earns nothing" would be the same category error as reading
  a tool's silence as a clean result.
- **The 47-row hand check is one sample at one stride.** It found a defect worth 380 protections
  because that defect is concentrated; a defect spread thinly across nine repositories at 1% each
  would pass a stride-23 sample untouched.
- **Nothing here measures the cost of a *missing* protection.** The complement — files Gate 1
  should protect and does not — was not sampled, and a safety layer's false-negative rate is the
  number that eventually matters most.
- **§11 R1 is not moved.** The full stack still leaves 5 false removals over 3 classes across
  four analyzers. m02, m11 and m12 remain, for the structural reason
  [`2026-08-02-gate2-veto.md`](./2026-08-02-gate2-veto.md) §5 gives.

---

## 8. Where this sits beside the other evaluations

| Document | Question it answers | Relationship |
| --- | --- | --- |
| [`2026-08-01-vulture-e2-baseline.md`](./2026-08-01-vulture-e2-baseline.md) | What one analyzer does unprotected | The bare column here |
| [`2026-08-01-four-analyzers-e2.md`](./2026-08-01-four-analyzers-e2.md) | What four analyzers do unprotected | Section 2.1's bare row (10 over 7 classes) is the same measurement |
| [`2026-08-02-gate2-veto.md`](./2026-08-02-gate2-veto.md) | What the reference veto is worth, in sample | Section 2.1's `--veto` row reproduces it; 2.2 shows Gate 1 adds nothing on top |
| [`2026-08-02-out-of-sample-corpus.md`](./2026-08-02-out-of-sample-corpus.md) | Gate 2a's flag rate and the root set, out of sample | Same nine repositories, same SHAs, same stride-sampled hand-check method |
| **this document** | What Gate 1 protects, and whether the protections are right | The first measurement of the layer that reasons about cost rather than usefulness |

The out-of-sample document's second result was *"the hand check got worse as the root set got
bigger, and only the hand check found it"* — 7 wrong of 47, all one defect, one rule, one
repository. This round reproduces that shape exactly: 17 wrong of 47, one defect, one rule,
one repository, invisible in every aggregate. Two rounds is not a pattern, but it is enough to
say the stride-sampled hand check is currently the only instrument in this project that has
ever found a defect the test suite did not.
