# Pre-Initiative Research: A Universal, Safe Repository Cleaner

**Date:** 2026-07-31
**Status:** Pre-initiative research. No code written. Decision-enabling.
**Audience:** The engineer about to start building this.

---

## ⚠ READ THIS BEFORE ANY OTHER SECTION — the irreversibility inversion

> **Git protects the object database, not the working tree. A file that was never `git add`-ed leaves NOTHING behind when deleted — no blob, no reflog entry, no `lost-found`.** (Verified on git 2.50.1; see §8.1 for the experiment and §8 for the full ladder.)
>
> Therefore the intuitive risk ordering is exactly backwards:
>
> | Class | Safe to *classify*? | Safe to *delete*? | Ladder rung |
> |---|---|---|---|
> | **Tracked and pushed** | No — it is source, and misclassification is a behavioural change | **Yes** — `git revert` / `git checkout <sha>^ -- <path>` restores it | R2–R4 |
> | **Untracked, not ignored** | Yes — it is not source | **No** — this is uncommitted human work with zero recovery path | R7 at best, **R9 by default** |
> | **Ignored** | Yes | **No** — `.env`, dev SQLite DBs, `terraform.tfstate.backup`, IDE run configs, model weights, patched `node_modules` live here | R7 at best, **R9 by default** |
>
> **The highest-volume, most tempting targets of any repo cleaner — build output, caches, logs, scratch files — are precisely the files git cannot restore.** "Gitignored" is *positively* correlated with irrecoverability, not negatively (§6.17: only 5.9% of github/gitignore patterns are confidently regenerable; 3.6% are explicitly irreplaceable; 90.5% unclassifiable).
>
> Every consequence of this is load-bearing and appears in five places, all of which must agree: §6.17 (gitignore inversion), §8.1 (the proof), §8.2 (`git add` as a one-command R9→R6 promotion), §9.3 Gate 0/Gate 1 (structural refusals), §9.6 (tier eligibility). **If an implementation ever auto-deletes an untracked or ignored path without first promoting its rung, it has reintroduced the exact defect this document exists to prevent.**

---

## 0. Executive summary (read this, then decide whether to keep reading)

1. There is no sound, general way to prove a file or symbol is unused. Entry points are invoked by humans, config strings, schedulers, and other repositories; the root set is unknowable. This is not a tooling gap — it is Rice's theorem plus an open world.
2. Therefore the product is **not** a better analyzer. Every per-language analyzer answers "unreachable from root set R under resolver X." None answers "is deleting this safe."
3. The product is the four layers no existing tool has: a **materialized, auditable root set**; a **veto lattice** where liveness evidence is absorbing and deadness evidence merely accumulates; an **evidence ledger with expiry and stability windows**; and an **enforced reversibility ladder** where the tool refuses to act above the rung the environment can actually support.
4. Orchestrate existing analyzers as *bounded accusers*, never as oracles. Contract them over SARIF 2.1.0, using the fields nobody uses: `invocation.executionSuccessful`, `artifact.roles:["analysisTarget"]` (a scanned-universe positive control), `partialFingerprints`, `baselineState`, `suppressions`.
5. **Ship the ratchet before the reaper.** Baseline current state; fail CI only on *new* junk. Zero deletion risk, zero config burden, best prior art (Shopify `deprecation_toolkit`, Google Tricorder + build-visibility whitelists). A reaper that never stops the inflow is bailing a boat.
6. The measured precision ceiling for multi-signal fusion on real code is ~88% with recall collapsing to ~54%. Fusion gets you a good *ranked queue for a human*, not an auto-delete tier. **This is in direct tension with §9.6 shipping a Tier 0 at all** — deliberately so: §11 R1 makes the existence of an auto-act tier the single highest-risk open question, resolvable in weeks by E2 (§10), with the pre-committed answer that if E2 does not come back clean the tier is **deleted from the design rather than tuned**. Note also that the ~88% figure is 39 TodoMVC apps (§11 "claims to stop propagating"): too pessimistic for the artifact/duplicate tier where content hashing is proof-grade, too optimistic for dynamic-language symbol deletion.
7. Runtime evidence is asymmetric and must be wired that way structurally: a hit is a proof of use; a miss is bounded absence of evidence over one window and one input distribution. Sampling profilers must be structurally incapable of voting "dead."
8. Test coverage contributes **zero** toward deadness. Its only sound uses are as a hard liveness veto and to identify the "alive only in tests" class, which is deletable as a *pair*.
9. Documentation leaves the deletion path entirely. Its blast radius (inbound links, bookmarks, agent context) is unobservable from inside the repo. Docs get FLAG and PROPOSE-PATCH only.
10. The highest-volume delete targets — untracked, gitignored, build output — are exactly the files git *cannot* restore. "Gitignored" is positively correlated with irrecoverability. Two file classes, opposite recovery profiles, must not share a tier system. **This is the single most consequential finding in the document; see the READ-THIS-FIRST box above §0, the proof in §8.1, the free rung promotions in §8.2, Gate 0g in §9.3, and the Tier-0 eligibility resolution in §9.6. If you implement one thing from this research, implement Gate 0g.**
11. Some deletions have **external effectors**: in GitOps/IaC repos the file *is* the desired state of a live system. Terraform's `prevent_destroy` explicitly does not apply when you remove the resource block. `git revert` restores the HCL, not the database.
12. The single cheapest high-value safety mechanism found anywhere: a **positive control** on every evidence artifact — if known-always-live symbols do not appear, discard the artifact loudly. Every catastrophic failure in the corpus presents identically as "~0% covered everywhere."
13. The second cheapest: a **whole-repo raw-byte grep veto** over every file type including binaries, on basenames *and parent directory names*. Meta runs it (BigGrep) and accepts the recall cost explicitly.
14. Meta's SCARF — compiler graphs + production telemetry + grep veto + 300+ pattern detectors + human review — still ships wrong deletions to production. Its safety net is fast revert, not correct analysis. Yours must be strictly stronger because you cannot assume their rollback infrastructure.
15. Recommended first six months: ratchet + root-set materializer + never-touch inventory + quarantine ladder + a 14-class mutation-injection safety suite. Gate any auto-act tier on that suite showing zero false removals. If it can't, ship with no auto-act tier and say so.

---

## 1. Problem statement, and why this is hard

### 1.1 The predicate

Fix `P(x) = "x is used"` for an artifact `x` (file, symbol, dependency, data blob). We want an analysis `A` that reports `dead(x)`.

- `A` is **sound for deadness** iff `A(x) = dead ⟹ ¬P(x)`. No false deletions. Requires **over-approximating** the set of uses.
- `A` is **complete for deadness** iff `¬P(x) ⟹ A(x) = dead`. No missed dead code. Requires **under-approximating** uses.

Rice's theorem forbids both simultaneously for any non-trivial semantic property. Every real tool picks one and lies about the other.

**The polarity trap.** Most static-analysis literature and tooling says "sound" meaning *sound for bug-finding* — over-approximate errors, produce false alarms, better safe than sorry. A cleaner's false alarm is a **deletion**. A cleaner wants the *opposite* over-approximation from the one most static analysis infrastructure is built for. Bolting a cleaner onto a bug-finder's call graph is structurally wrong, and this explains why so many cleaners inherit exactly the wrong error direction.

Full soundness is unreachable in practice, and the honest posture is documented: Livshits et al., *In Defense of Soundiness: A Manifesto* (CACM Feb 2015) — be **soundy**: mostly sound with a specific, enumerated, published list of unsound choices. *"In published papers, sources of unsoundness often lurk in the shadows, with caveats only mentioned in an off-hand manner."* For a cleaner, the unsoundness list **is** the user-facing safety documentation.

### 1.2 The unknowable root set, stated properly

Reachability analysis is only meaningful under a **closed-world assumption**. Real repositories are open-world in at least five independent directions:

| Direction | Example | Why no static analysis closes it |
|---|---|---|
| Human invocation | `scripts/restore_from_backup.sh` run once every 18 months at 3am | The caller is a person and a runbook, possibly a wiki outside the repo |
| Other repositories | `org/shared-schemas` consumed by 12 repos via raw URL + codegen | Consumers are not in the analyzed corpus |
| Runtime data | Rails STI subclass names stored in a DB `type` column | The root list literally lives in a database row |
| Deploy-time config | A `Class.forName(prefix + name)` where `prefix` comes from a k8s ConfigMap | The string never exists in the repo |
| Published artifacts | A library's `pub` API; a mobile app version still in the field | Consumers are outside your control and outside time |

You cannot infer the closed world. You can only have it **declared**. GraalVM `native-image` is the industrial proof: an entire ecosystem (`reachability-metadata.json`, a tracing agent, a shared community metadata repository) exists because *"determining dynamically-accessed elements via static analysis is infeasible as reachability depends on data available only at run time."* Nix's GC is the other proof: roots must be `nix-store --add-root`-registered, and `--print-roots` / `--print-live` / `--print-dead` let you audit the classification before collecting.

### 1.3 The asymmetric cost of false positives

The two error types are not comparable and must not be traded off with a symmetric metric.

- **False negative** (missed dead code): cost is some disk, some cognitive load, some build time. Recoverable at any time. Dead code is also *rarely revived* (Caivano/Cassieri/Romano/Scanniello, EMSE 2023), so a miss stays missable.
- **False positive** (deleted live code): cost ranges from a build break (cheap, loud, immediate) to a silent behavioral change discovered eleven months later during the incident the deleted code existed for.

Meta states the trade explicitly for its BigGrep layer: *"This approach can cause false negatives, but avoids false positives. When automating the removal of dead code, those are a more serious problem."* Codemod.com: *"zero false positives — better to miss a pattern than to transform incorrectly."* `staticcheck`'s `unused` rule 2.5 on type parameters: *"Unused type parameters are probably useless, but they're a brand new feature and we don't want to introduce false positives because we couldn't anticipate some novel use-case."*

**Formal framing: Neyman–Pearson, not Bayes-optimal classification.** Classical classification minimizes a weighted sum of type-I and type-II error. NP classification minimizes type-II error *subject to a hard constraint* type-I ≤ α. That is exactly the requirement: 40% recall at zero false deletions beats 95% recall with one catastrophic deletion. Two consequences the NP literature is explicit about:

- You cannot achieve NP control by thresholding empirical training error. *"Common practices that directly limit the empirical type I error to no more than α do not satisfy the type I error control objective."* You need an order-statistics / umbrella-algorithm threshold with a finite-sample high-probability bound.
- The headline metric must be **recall-at-α**, reported with a lower confidence bound on precision. **Never F1** — F1 prices a catastrophic false deletion identically to a missed cleanup, which is precisely the trade the product exists to refuse. The ground-truth debloating study (arXiv:2604.17717) reports False-Removal and False-Retention separately and uses F1 only "to showcase when a tool leans towards one extreme."

### 1.4 And yet: not deleting is also unsafe

The counter-argument must be stated because "just leave it" is the default failure mode.

**Knight Capital, 1 Aug 2012.** The "Power Peg" code path was correctly deprecated in 2003 — flag marked deprecated, users switched away, UI option disabled — but never removed. In 2012 a new NYSE Retail Liquidity Program feature *reused the same configuration flag*. One of eight SMARS servers did not receive the new deploy. The eight-year-old dead code activated with no risk controls (its safety counter had been relocated in 2005). ~4 million unintended orders, ~$7B in unwanted positions across 154 stocks, ~$440–460M loss in 45 minutes; the firm was effectively destroyed. SEC Admin Proceeding 34-70694.

Dead code that is *shipped* is a loaded gun whose trigger is a reused flag or a partial deploy. The right framing is therefore not "deletion is dangerous, don't" but **"deletion must be cheaply and completely reversible, and its blast radius must be bounded."**

### 1.5 Prevalence — how much is actually there

| Source | Measurement |
|---|---|
| Romano et al., TSE 2020 | 5–10% of methods dead in Java desktop apps |
| Eder et al., ICSE 2012 (Munich Re, .NET, 2yr production profiling) | 25% of 25,390 method genealogies never executed |
| Boomsma, Hostnet & Gross, ICSM 2012 — *Dead code elimination for web systems written in PHP: lessons learned from an industry case* (DOI [10.1109/ICSM.2012.6405314](https://doi.org/10.1109/ICSM.2012.6405314)) | ~30% of files in a PHP subsystem (2,740 files removed). Method: file-level *runtime* logging in production, not static analysis — i.e. it is an X-family measurement and inherits §3.6's observation-universe caveat |
| ~~Brown et al.~~ — **UNSOURCED; citation could not be located.** A Crossref bibliographic search (2026-07-31) returned no matching work | *Claimed:* 30–50% of an industrial system not understood by any current developer. **Treat as folklore until a citation is produced; do not cite this number.** The defensible neighbouring claim is Meta's developer-survey row below, which is sourced |
| Google Sensenmann | ~5% of *all Google C++* deleted, >1000 CLs/week |
| Meta SCARF | 100M+ (elsewhere 104M+) LOC removed, 370,000+ change requests, 5 years |
| Meta developer survey | 30% of engineers who said "I find it challenging to work in the codebase" cited dead code |
| DocPrism (arXiv:2511.00215) | ≥11% of non-trivial documented methods contain a genuine code-or-doc error |

But Eder's counterpoint matters: only **7.6%** of maintenance actions touched the unused 25%. Dead code's cost is mostly cognitive and migration-tax, not direct maintenance. And of 27 sampled never-executed cases reviewed with developers, only **9 (33%)** were genuinely unnecessary. *"Never ran in prod for 24 months"* is a ~1-in-3 predictor of *deletable*. (Caveat: 9/27, 95% CI ≈ 17–54%.)

---

## 2. The evidence taxonomy

Every signal, rated. **Polarity** is the load-bearing column: `VETO` signals can only rescue a candidate; `ACCUSE` signals can only nominate; `ABSTAIN` means the signal has no information (which is *not* the same as "no evidence of use").

### 2.1 Comparison table

| Signal | Proves | Polarity | FP modes (says dead, is live) | FN modes | Cost | Portability |
|---|---|---|---|---|---|---|
| **Linker section GC** (`--gc-sections` + `--print-gc-sections`, `-dead_strip`, `/OPT:REF`) | Nothing reachable from this binary's GC roots references this section — a genuine proof of a negative | ACCUSE (proof-grade, per-link) | Reachable in another build config/target/feature set; needed by a static-lib consumer | Enormous: GC roots include everything in `.dynsym`, `SHF_GNU_RETAIN`, `INIT_ARRAY`, `KEEP()`, eh_frame personality routines, `__start_`/`__stop_` C-ident sections | ~free, 2 compiler flags + 1 linker flag, CI-trivial | ELF/Mach-O/PE: C, C++, Rust, Zig, Swift, Fortran, D |
| **Shipped-artifact symbol presence** (nm/objdump/Bloaty) | Exactly which symbols/CUs are in the release artifact | ACCUSE (proof about the artifact) | Repo builds >1 artifact/target/profile; ICF folds identical functions; LTO internalizes/renames; inlined-only symbols | Presence ≠ execution | seconds, one command | C/C++/Rust/Go/Swift/Zig |
| **Build-graph reachability** (`bazel query rdeps`, Blaze, Nix, Nx `affected`) | Not in the transitive closure of any binary/test target | ACCUSE (strong, with witness path) | dlopen/plugin registries; consumers outside this build graph; data files outside the graph | Coarse: one used symbol keeps the whole target | free where the build system exists | Bazel/Buck2/Pants/Nix/Nx/Turborepo only |
| **Compiler index / semantic graph** (Swift index store, SCIP, Glean, Go RTA) | No reference in the index for the build config that was compiled | ACCUSE (moderate–strong) | Reflection, DI, string dispatch, `#if`/build-tag branches not compiled, cross-language boundaries, **buggy indexers** | Over-approximating dispatch keeps too much alive | full build per config; hours at scale | Per-language; SCIP has ~10 indexers |
| **Whole-program RTA** (`golang.org/x/tools/cmd/deadcode`) | Not callable from any main/init, *even through func values, interface dispatch, and reflection* | ACCUSE (proof-grade within its scope) | `//go:linkname` aliasing; assembly/cgo callers; single GOOS/GOARCH/tags only; libraries have no roots | Conservative reflection model; marker interface methods | build-time, minutes | Go |
| **Scope-local liveness** (Ruff F401/F841, ESLint, Roslyn IDE0051, PMD UnusedPrivate*) | Not referenced within a closed scope (`private`, file-local) | ACCUSE (near-proof) | Reflection into private members, annotation-driven invocation, serialization callbacks, Lombok, `@MethodSource` | Blind past the visibility boundary | free (already in your linter) | Universal |
| **Language-enforced module boundary** (Go `internal/`, Rust `pub(crate)`, C# `internal`+`InternalsVisibleTo`, JPMS `exports`, Bazel `visibility`) | The universe of possible importers is a *known bounded subtree* | ACCUSE (proof-grade, underused) | Only reflection within the subtree | Only covers non-exported surface | free (`go list -deps -json`) | Go, Rust, C#, Kotlin, Java, Bazel |
| **Module-graph reachability from declared entries** (Knip, unimported, dpdm) | Unreachable from the configured entry set under static resolution | ACCUSE (moderate) | Every unresolvable reference: template-string `import()`, CJS member access, HTML `<script src>`, auto-imports/auto-mocks, missing framework plugin, cross-workspace relative paths | Over-broad entry globs hide everything | very cheap, cached | JS/TS (excellent), Python/Rust (good) |
| **Global name-set difference** (Vulture, ctags+ripgrep) | This identifier string appears nowhere as a load | ACCUSE (weak) | Framework-driven invocation: 260 FPs on Flask, 102 on FastAPI, 59 on httpx which had *zero* real dead items | Any same-named symbol anywhere marks all of them used | trivial, no build | Universal — which is why it is most reached for and most damaging |
| **Production runtime coverage** (Coverband, JaCoCo agent, `NODE_V8_COVERAGE`, `sys.monitoring`, `dotnet-coverage connect`) | This location executed at least once, in the observed processes | **VETO** (hit) / weak ACCUSE (miss) | Window < natural period; instrumentation silently disabled; partial fleet; code-identity drift; traffic-shaped blind spots | Import-time coverage marks `def`/`class` lines; tree-shaken exports never seen | agent + dump/ship/merge pipeline; runtime cost now ~0 | Ruby/JVM best; Node/Python good; PHP effectively unavailable |
| **Function-level runtime evidence** (`Coverage.start(methods:)`, V8 fn ranges, lcov `FNDA`, JaCoCo method counters) | This *named function* was invoked | **VETO** | Name collisions for overloads/closures/generated methods; identity keys shift on edit | Health-check/introspection invocation counts as live | free where native; derived in Python | Ruby/JS/JVM/.NET native; Python derived; PHP absent |
| **Whole-file "never loaded"** (`-Xlog:class+load`, `coverage --source`, `c8 --all`, JaCoCo class-file analysis) | A file/class was never loaded | ACCUSE (moderate) | Enumerator is lossy *toward* "unused" (coverage.py only discovers importable files, issue #1708); tree-shaken JS produces no record | Loaded ≠ used (classpath scanning); blind to partially-dead files | very cheap; `-verbose:class` is free | JVM strong, Python/Node good |
| **Sampling profilers** (Parca 19Hz, Pyroscope 100Hz, Datadog, JFR, py-spy) | This symbol was on-CPU at least once | **VETO ONLY** | — (safe direction) | Catastrophic if inverted: see §3.3 arithmetic. Inlined functions never appear at all. Off-CPU work invisible | ~free if already deployed | Universal |
| **Tombstones / gravestones** | Exhaustively, per invocation, that this suspect code executed in production | **VETO** (fires) / strong ACCUSE (silent) | Window shorter than the business cycle; not deployed to every environment | Log delivery can drop under load (Nestoria fails silently by design) | code change + deploy + months of waiting | Universal (it's a log line) |
| **Test-suite coverage** | The tests exercised this | **VETO only** | — | Systematically anti-correlated with the value of the code: error handlers, DR paths, platform branches, admin tooling | free (already in CI) | Universal |
| **Whole-corpus textual grep** (Meta BigGrep) | This literal string appears somewhere in the corpus | **VETO** (mandatory) on a hit; weak ACCUSE (+1.0 bans, §9.5) on a *complete, non-truncated, zero-hit* search — **and ABSTAIN, never ACCUSE, if the search was truncated, timed out, errored, or ran over an incomplete file set.** The two directions are not symmetric and must be separate code paths | Coincidental name collisions (cost: recall only) | Computed names (`"tbl_"+region`, `f"handlers.{kind}"`); names outside the repo; **truncated searches read as "no matches"** | ripgrep, milliseconds | Perfect — the only fully language-agnostic signal |
| **Manifest / entry-point discovery** | This path is a declared root | **VETO** | Roots living in shell history, runbooks, cron on a box | Over-broad manifests (`files: ["**"]`) mark everything a root | cheap to run, expensive to *build* | Universal in shape, per-ecosystem in content |
| **Shrinker output ingestion** (`-printusage`/`-printseeds`/`-whyareyoukeeping`, ILLink `_TrimmerDumpDependencies`, bundler stats) | What your *production* reachability pass already removed, from the *true* production root set | ACCUSE (strong) | Only as good as hand-written `-keep` rules | Over-keeping (`-keep class **.* { *; }`) hides real dead code | free — CI already computes it and throws it away | JVM/Android, .NET, JS bundlers |
| **Content-hash duplication** (`git ls-files -s`, jdupes, rmlint `-p`) | These paths hold byte-identical content | ACCUSE (proof about *content*) | Measured 6/6 unsafe on a real repo: multi-harness mirrors, per-package LICENSE, empty `__init__.py` sharing blob `e69de29b` | Any whitespace/EOL/version difference | free in git | Universal |
| **Magic-byte content sniff** | This is a SQLite DB / PEM key / HDF5 / safetensors / pg_dump | **VETO** (never-touch) | Over-vetoes `.jar`/`.whl`/`.docx` (cost: disk only) | Blind to plain-text irreplaceables: `.env`, `.tfstate`, `.npmrc`, `.Rhistory`, CSV | one 32-byte pread per file | Perfect |
| **Regenerate-and-diff** | This artifact is byte-reproducible by the build | ACCUSE (proof, when it works) | Non-hermetic builds; tree-shaking makes identity trivially true; runtime-loaded assets aren't build inputs | Reproducibility measured: npm 100%, Cargo 100%, PyPI 12.2%, Maven 2.1%, RubyGems 0% (ICSE 2025) | two full builds per verification | Poor without `SOURCE_DATE_EPOCH` |
| **Declared-output manifests** (`turbo.json#outputs`, Bazel `aquery`, Nix derivations) | The build system's own statement of what it produces | ACCUSE (strong, one-directional) | Symlink escape (Bazel `bazel-out` → `~/.cache`); worktree cache redirection | **Asymmetric:** absence proves nothing — "If you do not declare file outputs, Turborepo will not cache them" | free to read | Where modern build systems exist |
| **VCS history** (age, churn, single-commit) | When this path last changed | **VETO only** (recency), heuristic otherwise | **Measured anti-predictive:** >4y untouched → 1.4% subsequent deletion vs 6.4% base rate | Bulk reformats reset everything; squash/rebase/filter-repo destroys it; shallow CI clones make it constant | free (`git log`) | Universal |
| **Dynamic-construct density** (Semgrep/ast-grep for `getattr`, `Class.forName`, `require(var)`) | This region is unanalyzable | **TIER-CEILING MODIFIER** | n/a (never asserts deadness) | Missing an idiom means failing to down-weight | one pass, very cheap | 30+ languages |
| **LLM incorrectness detection** (LCEF-style) | A doc and its code disagree | ACCUSE (weak, docs only) | 0.63 precision even at best; 35% of FPs from "lack of API knowledge" (no callee body); naive prompting flags 82–97% of functions | 3.7% schema-invalid → silently reports clean; Type-4 semantic recall 0.32 (GPT-4), 0.07 (GPT-3.5) | one 70B inference per function | Excellent in principle, expensive in practice |

### 2.2 Signal independence — the modelling problem nobody solves

Naive Bayes over these signals is wrong in a specific, documented way. Static reachability and test coverage share one confounder: **repo dynamism**. If a module is reached only via `getattr` / `require(var)` / a DI container, the static graph misses it *and* the test suite (which exercises production paths through the same indirection, or fails to) misses it too. Their agreement is close to one observation reported twice. Multiplying them produces the known pathology: *"when features are correlated or duplicated, the classifier exhibits severe overconfidence… probability estimates tend to be more extreme (closer to zero or one) than they should otherwise be."*

**Correlation families** (take MAX within family, SUM across families):

- **Family R — reads repository text:** static reachability, grep veto, manifest roots, name heuristics. All fail on dynamic dispatch.
- **Family X — observes execution:** production coverage, tombstones, profiler samples, class-load logs. Independent of R.
- **Family B — build/artifact identity:** linker GC, shipped-symbol presence, declared outputs, regenerate-and-diff.
- **Family H — history:** VCS age, churn, co-change.

Only cross-family agreement earns a tier promotion. And the confounder should be modelled explicitly: introduce a per-repo latent `D = "this repo dispatches dynamically"`, estimated from dynamic-construct density, and let a high `D` collapse the entire R family's weight at once rather than each signal independently.

### 2.3 Representation: three-valued, not scalar

The valuable idea from Dempster–Shafer is the **representation**, not the combination rule. Mass over `Θ = {alive, dead}` plus explicit ignorance `m(Θ)`:

- "No call site found" → mass on `m(Θ)` (ignorance), **not** `m({dead})`.
- Only a positive proof (build regenerates this byte-identically; this is OS metadata; this is syntactically unreachable) puts real mass on `m({dead})`.

Avoid Dempster's rule itself: it assumes independent sources (violated), and Zadeh's paradox shows conflicting sources can combine to certainty in a hypothesis both considered negligible. The veto-lattice architecture in §9 is DS with `m(Θ)=1` as the default plus a hard floor — simpler and better behaved.

**Base rate for arithmetic.** A defensible prior for a tracked source file being dead is ~10%, i.e. prior log₁₀-odds ≈ −0.95. Reaching P(dead)=0.999 requires ~+4 bans of likelihood ratio. No combination of VCS and naming signals can supply that, and the design should make the arithmetic visible rather than hiding it.

---

## 3. Runtime evidence

### 3.1 The asymmetry, restated as an engineering rule

A coverage **hit** is a proof of use (sound). A coverage **miss** is bounded absence of evidence over one observation window and one input distribution (unsound). Design so hits can only *remove* candidates and no negative signal alone can promote one.

### 3.2 Per-ecosystem mechanisms, with flags

| Ecosystem | Mechanism | Key flags / API | Notes |
|---|---|---|---|
| Python 3.12+ | `sys.monitoring` (PEP 669) | Callback returns `sys.monitoring.DISABLE` → permanently unregisters *that location*; native speed thereafter | Naturally a one-shot set-membership test — exactly the shape needed. Tool IDs 0–5 coexist |
| Python | coverage.py | `[run] core = sysmon\|ctrace\|pytrace` (`COVERAGE_CORE`), `source`, `concurrency`, `sigterm`, `[paths]` aliasing, `coverage combine` | SQLite `line_bits` numbits, unionable in SQL. **`concurrency` unset → "very wrong results"** |
| Python | py-spy / austin | attach by PID, no restart | **Rescue-only.** Never a deadness vote |
| Python | `python -X importtime` | stderr | Proves a module was imported. Zero instrumentation |
| Ruby | stdlib `Coverage` | `Coverage.start(lines:, branches:, methods:, eval:, oneshot_lines:)` | `methods: true` → `{[Object,:fib,1,0,9,3] => 177}`: **per-method call counts, natively**. `oneshot_lines` fires once then zero cost (Feature #15022). `lines` and `oneshot_lines` mutually exclusive. Eval'd code without an explicit filename is unmeasurable |
| Ruby | Coverband | `use_oneshot_lines_coverage`, Redis Hash store, `config.ignore`, `rake coverband:dead_methods`, view/route/i18n trackers | Splits **eager-load vs runtime** coverage — the difference between "class body evaluated" and "method called" |
| Node | `NODE_V8_COVERAGE` + `v8.takeCoverage()` / `stopCoverage()` | Auto-propagates to `child_process.spawn` (set empty to stop) | Only zero-dep way to get production JS coverage on a schedule. Flush at exit unless you call `takeCoverage()` |
| Node | c8 / nyc | `--all --src --include --exclude --exclude-after-remap` | `--all` is mandatory: "v8 will only give us coverage for files that were loaded" |
| Node | istanbul (babel-plugin-istanbul) | AST instrumentation pre-transpile | `fnMap`/`f` counters against **original source coordinates** — immune to source-map back-mapping error. Not attachable to a built artifact |
| JVM | JaCoCo agent | `-javaagent:jacocoagent.jar`, `output=file\|tcpserver\|tcpclient\|none`, `append`, `dumponexit`, `sessionid`, JMX `org.jacoco:type=Runtime`, `jacococli merge` | Classes identified by **CRC64 of the raw class definition**. Report step analyses class files, so it *can* prove "never loaded". No authentication on tcpserver/JMX |
| JVM | `-Xlog:class+load=info:file=classes.log` | JDK9+ (`-verbose:class` older) | Zero-instrumentation, zero-risk file-level liveness |
| JVM | teamscale-jacoco-agent | periodic interval dumps + upload | The productionized answer to `dumponexit` fragility |
| .NET | coverlet | `--single-hit`, `--skipautoprops`, `--merge-with`, `--exclude-by-attribute`, `--use-source-link` | SDK-style projects only; test-harness oriented |
| .NET | `dotnet-coverage` | `collect`, **`connect`** (attach to a running server), `merge -f cobertura **/*.coverage` | Only first-party route to coverage from a live server |
| .NET | `dotnet-trace` / EventPipe | Jit/Loader keywords → `MethodLoadVerbose`, `ModuleLoad` | **ReadyToRun/NativeAOT executes without a JIT event** — "never JITted" ≠ "never executed" |
| Go | `go build -cover` | `GOCOVERDIR`, `go tool covdata percent\|func\|textfmt\|merge\|subtract\|intersect` | Only main-module packages instrumented by default. `-coverpkg=main` warns and instruments *nothing* (it takes import paths). Long-running servers must call `runtime/coverage.WriteCountersDir()` |
| C/C++/Rust/Swift | LLVM source-based | `-fprofile-instr-generate -fcoverage-mapping`, `LLVM_PROFILE_FILE` with `%p`/`%m`/**`%c`**, `llvm-profdata merge`, `llvm-cov export` | `%c` = continuous mode: "if the instrumented program crashes, or is killed by a signal, perfect coverage information can still be recovered." Long Darwin-only; `-fprofile-continuous` driver flag merged Feb 2025 (llvm-project #124353); Linux needs `-mllvm -runtime-counter-relocation` |
| C/C++ | gcov/lcov | `--coverage`, `-fprofile-update=atomic` | `.gcda` written only on `exit()`. **Signals update the app's `.gcda` but not shared libraries'** unless each `.so` exposes `__gcov_dump` |
| Rust | cargo-llvm-cov | `--no-report` + `report` to merge across feature sets; `--include-ffi`; `clean --workspace` between runs | Branch coverage unstable; doctests unstable (so `pub` items used only in doc examples become FPs) |
| PHP | pcov / Xdebug | `pcov.directory`, `pcov.exclude`; `XDEBUG_MODE=coverage`, `XDEBUG_CC_DEAD_CODE`, `XDEBUG_CC_UNUSED` | pcov: **line coverage only** (cannot separate definition from invocation), unmaintained since 2021, README says `pcov.enabled=0` in production. Xdebug: 16s suite → 215.95s |
| Interchange | lcov `.info` | `SF:`, `DA:<line>,<count>`, **`FN:`/`FNDA:<count>,<name>`**, `BRDA:` | `FNDA:0,<name>` is the single most valuable cross-language primitive: "this named function was never called," and it merges with `lcov -a` |
| Multi-format merge | grcov | consumes gcov/lcov/JaCoCo/coveralls | Reuse rather than rebuild. Path normalization is still yours |

**Overhead is no longer the objection — for some ecosystems.** Picnic measured **0.03%** average request-duration overhead for JaCoCo across two 24-hour production periods. coverage.py's sysmon core is "often lower than 5%" and cut PyPI's suite from 163s → 30s. Ruby `oneshot_lines` and PEP 669 `DISABLE` are one-shot with zero steady-state cost. **But**: instrumented LLVM builds run 2–4× slower, and PHP under Xdebug ~13×. Do not generalize the JVM/Ruby/Python-3.12 result.

### 3.3 The sampling-bias math

A sampling profiler at frequency *f* sees a function iff a sample lands while it is on-CPU. Expected samples over a window = *f* × (total CPU-seconds consumed by that function). Need E ≥ 3 for ~95% confidence of at least one hit.

- At 100 Hz: the function must consume **≥30 ms of CPU across the entire window**.
- At Parca's documented 19 Hz: **≥158 ms**.

`p(sampled per call) = 1 − exp(−f·T)`; `p(never in window) = (1−p)^N`. Over 90 days:

| f | per-call CPU T | frequency | p(NEVER sampled) |
|---|---|---|---|
| 100 Hz | 100 µs | weekly (N≈12.9) | **87.94%** |
| 19 Hz | 100 µs | weekly | **97.59%** |
| 100 Hz | 100 µs | **daily** (N=90) | 40.66% |
| 100 Hz | 1 ms | weekly | 27.65% |
| 100 Hz | 10 ms | weekly | 0.0003% |
| 100 Hz | 10 µs | hourly | 11.53% |

Degradations multiply: 25% duty cycle turns 87.9% → 96.8%; 5% duty → 99.4%. Profiling 10% of the fleet turns 87.9% → 98.7%; 1% → 99.87%. Coverband's documented high-traffic config of **1% request sampling** is the same 1-in-100 factor.

**The rule-of-three inversion is a trap.** Over 90 days a 1000-host × 16-core fleet at 100 Hz collects ~1.24×10¹³ samples, giving a 95% upper bound of 2.4×10⁻¹³ on the CPU fraction of a never-sampled function — which *sounds* like proof. But a weekly 100 µs call on that fleet has a true CPU fraction of ~1.03×10⁻¹⁴, an order of magnitude *below* the bound. The statistic is satisfied by a function that IS running. Bigger fleets do not help; the denominator grows with the evidence.

Two deterministic (not probabilistic) blind spots make it worse: **inlining** (a small hot function never appears as its own frame, at probability 1) and **off-CPU work** (I/O, lock waits, sleeps burn no CPU). "Not in the CPU profile" conflates "never ran" with "ran but did no CPU work."

**Conclusion: sampling profilers must be structurally incapable of contributing a deadness accusation.** Wire them as veto-only.

### 3.4 "Tests are not usage" — the strongest warning in the corpus

Test coverage and production usage are close to orthogonal. Typical suites cover 40–80% of lines; the uncovered remainder is dominated by production-only paths.

The ground-truth debloating study (Bilal et al., arXiv:2604.17717v2; 11 hand-curated programs, 5K–76K LoC, 8 tools) measured what happens when you equate "tests pass" with "safe to remove":

- Dynamic debloaters falsely removed **up to 94%** of must-retain code. Per-program False-Removal at LoC granularity — Blade: `mkdir-5.2.1` 90%, `uniq-8.16` 89%, `rm-8.4` 80%, `sort-8.16` 72%; Chisel: 78/68/67/66%; Cov: 58/66/63/56%.
- Conservative static tools sit at the opposite extreme: ~100% False Retention at function granularity (Lmcas 100% FRt on `mkdir` and `uniq`).
- **Issue 3 (Unsafe Intermediate State):** Blade removed the guard in `rm-8.4`'s `fts_build` (gnulib) that skips `.` and `..`. Running the test suite on that state caused the broken traversal to **delete the container's `/bin`**, crashing the debloating process itself. The authors' conclusion is the sharpest statement in the literature: *"Such guard logic has no observable effect under normal conditions as its failure modes are environment-dependent which are unlikely to be covered by any practical test suite. The paradigm's equation of 'test passes' with 'removal is safe' is therefore fundamentally unsound for guard logic: a removal can silently pass all tests while introducing catastrophic behavior."*
- **Issue 4:** Blade removed pthread mutex ops, the `queued` flag, and a condition-variable signal from `sort-8.16`'s `queue_insert`. *"Race conditions and deadlocks often appear only under specific timing conditions or heavy load."*
- **Issue 5:** *"Essential error logging and handling functions are frequently removed since they are very difficult to exercise with test cases."*
- **Issue 2 (Residual Path):** `gzip-1.2.4` debloated for decompression retained partial compression code; invoking it with compression flags reaches `unlink(ifname)` and **deletes the input file** with no output. Partial removal is worse than none.

**The structural trap (Sensenmann's insight).** If test execution counts as liveness, every tested-but-dead module stays alive forever and *"we would only be able to clean up untested code, which would severely hamper our efforts."* Google's fix: make each library and its test mutually dependent so they form a strongly connected component (Tarjan), sharing fate. The residual hard case is unsolved even at Google: an LZW test covering *both* a compressor and a decompressor has the *identical graph shape* as a `web_test` that merely uses `url_encoder_lib` as a support dependency — same topology, opposite correct treatment. Their current matcher is **edit distance on target names** plus a `testonly` convention, and they name coverage-based matching as "not yet explored."

### 3.5 Tombstones — the only exhaustive answer for rare code

Instrument only the *suspects*. A `tombstone(date, author, label)` call logs on invocation and does nothing else. Anything that fires is a **"vampire."**

Provenance: David Schnepper (Box), Velocity Santa Clara 2014 Ignite → Nestoria/Lokku `Lokku::Tombstone` (Perl, 9 Apr 2015) → `scheb/tombstone` (PHP, still maintained) → `lewispb/tombstone` (Ruby), `tombstone-py`.

Nestoria's implementation details **are the design spec**:
- Takes `(date_added, author)` inline so reports can age and attribute.
- Writes to a **local file** named `vampires_<date>_<user>_<tombstone-date>_<author>.log` — not a network call, so a firing tombstone can never take production down.
- Written to be "as fast as possible to avoid slowing production systems down if called in a tight loop."
- "Has several safeguards, such as failing silently if the log file is getting too big." → **a tombstone must never be able to become the outage**, which also means the *first* hit must be recorded durably before any rate limiting.

Nobody publishes the number you want (what fraction eventually fire, longest-latency vampire). The literature is unanimous on mechanism and silent on base rates. The honest framing: the required wait is set by the longest business cycle (annual tax/billing/compliance, yearly cert rotation, quarterly close, leap day), not by statistics — a structural argument for **≥13 months**, not 90 days.

### 3.6 Observation *universe*, not just window

Window length is the wrong axis for the worst cases. A vendor shipping both SaaS and on-prem has telemetry covering SaaS only: the LDAP, SAML, proxy, and offline-license paths have zero hits over **any** window, forever, because the users are outside the observed universe. Same shape: mobile apps on old versions still in the field; region-locked features; an air-gapped customer's integration; a flag at 0% today, ramping next quarter.

Required in the data model: a `telemetry_covers_all_deployments` flag, and a hard rule that runtime-based deadness claims are **void for any repo producing a distributable artifact**.

### 3.7 Positive-control validation (the cheapest safety mechanism in this document)

Every catastrophic failure mode in this space shares one signature: **coverage reports ~0% for everything.**

- Coverband under Scout APM `AUTO_INSTRUMENT=true`: *"it stops reporting any coverage, it will show one or two files that have been loaded at the start but everything else will show up as having 0% coverage."*
- coverage.py with `concurrency` unset under gevent/multiprocessing: "very wrong results," silently.
- JaCoCo CRC64 class-id mismatch after any recompile or bytecode rewriting (EJB container, Mockito, another agent): the class reads as NOT COVERED although it executed.
- `dumponexit` that never fired because the pod was SIGKILLed / OOM-killed / evicted.
- Empty `NODE_V8_COVERAGE` dir because the process was SIGKILLed.
- `.gcda` never written because the process didn't `exit()`.

**One assertion eliminates the entire class:** before trusting any artifact, require a small declared set of always-live symbols (HTTP entry point, main loop, health-check handler, `if __name__ == "__main__"`) to appear executed. If not, **discard the whole artifact with a loud error**. Essentially no existing tool does this.

**And it must be specified at the right granularity.** In Python/Ruby/JS, `def`, `class`, decorators, and module-level lines execute at *import*. Under every one of the five failures above you get boot-only coverage in which the health-check handler's `def` line IS covered — a line-granularity positive control **passes** while every function body reads dead. Specify the control at **function-body-line or `FNDA` granularity**, and additionally assert a plausible floor (e.g. ≥5% of function bodies executed). Otherwise it is theatre.

---

## 4. Static evidence

### 4.1 Per-language tool survey

| Tool | Ecosystem | Technique | Machine-readable output | Known FP modes |
|---|---|---|---|---|
| **Knip** | JS/TS monorepos | Module graph from entry set; `unused files = project files − (entry + resolved)`; ~178 framework plugins | `--reporter json\|sarif\|codeclimate`; custom reporters; **preprocessors**; `--trace-export/-file/-dependency` reverse graph | Template-string `import()`; CJS member access (`m.fn()` untraced, destructuring traced); HTML `<script src>`; unknown CLI args; auto-mocks/auto-imports; cross-workspace relative paths; conditional deps in *executed* config files; missing/incomplete plugin |
| **ts-prune** | TS | tsc API, exported symbols with no cross-file refs | text only; `-e` exit code | Dynamic imports; `require('name')` strings. **Archived**, points to Knip |
| **unimported** | JS/TS | follows imports from entry | text | Falls back to `package.json#main` (often `dist/`): "analyzing a bundled asset is likely to result in false positives." **Archived** |
| **depcheck** | npm | parsers + ~19 "specials" for tool configs | `--json` | README has a section titled *"False Alert"*. "The logic of a special is not perfect." **Archived Feb 2025, 116 open issues** |
| **madge / dpdm** | JS/TS | dependency graph; `--orphans`, `--circular`; dpdm `--detect-unused-files-from`, `--skip-dynamic-imports tree\|circular` | `--json` | Silent resolution failure (`--warning` exists for this reason). Orphan ≠ unused (entry points are orphans) |
| **Vulture** | Python | Global AST name-set difference. `DEFAULT_CONFIDENCE=60`; 90 imports; 100 args/unreachable | text + flake8 codes; Python API `Vulture().get_unused_code()` | **44 TP / 644 FP across 9 popular repos (~6% precision)**; 59 FPs on httpx which has *zero* dead items; 260 on Flask (`@app.template_global`); 102 on FastAPI (Pydantic fields); all Django model fields (#110); `globals()` (#373); dataclasses (#362); TypedDict (#335); Protocol (#313); **#422 is an open 100%-confidence FP** (`async def f(): return; yield` — removing the flagged `yield` silently converts an async generator to a coroutine) |
| **deadcode (albertas)** | Python | AST + rich ignore algebra: `--ignore-names-if-inherits-from`, `--ignore-names-if-decorated-with`, `--ignore-bodies-of` | text | No confidence model. AGPL |
| **Ruff** | Python | Per-file scope (pyflakes in Rust): F401/F811/F841/F842/F859/ARG001-005/B007/ERA001 | `--output-format json\|sarif\|github\|gitlab\|junit\|rdjson\|azure\|pylint\|concise\|grouped\|full` — richest matrix surveyed | **F401 autofix is documented unsafe in `__init__.py`**; string annotations under `TYPE_CHECKING` (#4654) |
| **deptry** | Python deps | imports vs declared deps + venv metadata. DEP001–DEP004 | `--json-output` | Entry-points-only packages (pytest/mypy/setuptools plugins) are structurally invisible |
| **Skylos** | Python | Framework-aware (Flask/FastAPI/Django/pytest/Pydantic) | — | Still 220 FPs vs 51 TPs on the same 9 repos — ~4× more FPs than TPs even framework-aware |
| **`x/tools/cmd/deadcode`** | Go | RTA from every `main`+`init`; `-test` adds test mains; `-whylive=fn` witness path | `-json` (`Package{Name,Path,Funcs}`, `Function{Name,Position,Generated,Marker}`), `-f=<template>` | `//go:linkname` aliasing → "spuriously reported as dead"; assembly/cgo callers; **valid for one GOOS/GOARCH/-tags only**; libraries have no roots. Docs: *"a dead method may be required to satisfy an interface that is never called. Some judgement is required."* |
| **staticcheck U1000** | Go | Ownership/use graph with ~40 numbered documented over-approximations | `-f json`; via golangci-lint `sarif\|checkstyle\|code-climate` | **#48 open since 2016**: build-tag-guarded usage → FP. **#1648**: field reached via `reflect.TypeFor` + `Field(i).Name`. Source comment: *"we cannot observe function calls in assembly files"* |
| **`go mod tidy`** | Go | recompute requirements from imports | — | golang/go#65054, #39570, #58216: removes deps needed behind build tags / other GOOS |
| **cargo-machete** | Rust | greps for import names; `--with-metadata` | `--json`; exit 0/1/2 | build.rs-generated usage (`prost`); package name ≠ import name (`rustls-webpki`→`webpki`). `--with-metadata` **may modify Cargo.lock** |
| **cargo-udeps** | Rust | real `cargo check` dep-info | — | "Some unused crates might not be detected"; per-name only (two versions collide); **cannot see doc-tests**; needs nightly |
| **cargo-shear** | Rust | rust-analyzer parser; also **unlinked `.rs` files** (not in any `mod` tree) | `--format=json`, `--fix`, `--deny-warnings` | Macro-expansion imports invisible without `--expand` (nightly, slow). *Unlinked-file detection is near-proof because Rust requires explicit `mod`* — the strongest file-level signal in any language surveyed |
| **rustc `dead_code`** | Rust | reachability from `pub` | `cargo build --message-format=json` (free) | FN: doesn't track trait impls — `#[derive(Clone)]` silences it (#57613); fails in dylib crates. FP: const-generic args (#128617), supertrait bounds (#121040) |
| **Periphery** | Swift | Builds project → **compiler index store** → declaration/reference graph; parses .xib/.storyboard | `--format json\|csv\|checkstyle\|codeclimate\|github-actions\|github-markdown\|gitlab-codequality\|xcode` | Only *built* targets indexed; `#if DEBUG` branches invisible (README's own `releaseName` example); "cannot analyze Objective-C since types may be dynamically typed"; Codable/Equatable/Hashable synthesis invisible; raw-value enums undecidable; `@_exported`; **four open Swift index-store bugs (apple/swift #56541, #56327, #56189, #56165)** where the compiler records wrong relations. 105 issues titled "false positive" |
| **UCDetector** | Java | JDT search for references | Eclipse markers/HTML/XML | No model of reflection, Spring, JPA, JAXB, ServiceLoader |
| **PMD** | Java + ~20 | AST rules, private/local scope | `--format sarif\|json\|codeclimate\|xml\|csv` | `UnusedPrivateMethod` ships `ignoredAnnotations` defaulting to `@Deprecated, @PostConstruct, @PreDestroy, lombok.EqualsAndHashCode.Include` — an admission that annotations defeat it |
| **Error Prone** | Java | javac plugin; UnusedVariable/UnusedMethod/UnusedNestedClass | javac diagnostics; `-XepPatchChecks:` emits a **unified diff** | Private/local only |
| **ProGuard / R8** | JVM bytecode | Shrink from hand-written `-keep` seeds | `-printusage` (what was removed), `-printseeds` (**the materialized root set**), `-whyareyoukeeping` (shortest chain) | The industry's largest FP class: reflection-only classes removed → `ClassNotFoundException`/`NoSuchMethodException`/`BadParcelableException`/Gson `JsonSyntaxException`, **release builds only**. Also kotlinx.coroutines #983/#3111: shrinker strips `META-INF/services` → "Module with the Main dispatcher is missing"; correct fix was `-adaptresourcefilenames`, not `-keep`. Caveat: `-whyareyoukeeping` "may sometimes contain circular deductions" |
| **ILLink / .NET trim** | .NET IL | mark-and-sweep with **declared** dynamism | MSBuild IL2xxx diagnostics → SARIF via `/p:ErrorLog=`; `_TrimmerDumpDependencies=true` XML keep-graph | Docs refuse to guarantee correctness with any warning outstanding. `[DynamicDependency]` keeps the target **without silencing the warning** — deliberately |
| **Roslyn IDE0051/IDE0052** | C#/VB | private members with no read/write refs | `dotnet build /p:ErrorLog=out.sarif` | Explicitly does **not** fire on `internal` or `public` |
| **PHPStan + tomasvotruba/unused-public** | PHP | private (core) / public (extension) with `template_paths:` for Twig | `--error-format=json\|checkstyle\|gitlab\|github\|junit\|prettyJson`; SARIF extension | *"when used only in templates, apart from Twig paths, it's not possible to detect them."* Gradual adoption via percentage thresholds (`methods: 2.5`) |
| **composer-unused** | PHP deps | resolves *symbols* each package provides | `--output-format` (verify per release) | DI-container string wiring invisible |
| **debride** | Ruby | ruby_parser static analysis | text | Output header is the most honest in the survey: *"These methods MIGHT not be called:"*. **`debride_rails_whitelist routes.txt log/production.log`** builds the retain set from `rake routes` + 28 days of real production logs — runtime fusion, shipped in 2015 |
| **`unused` (joshuaclayton)** | **Language-agnostic** | ctags definitions × ripgrep occurrence counts → `Removal { rLikelihood :: High\|Medium\|Low\|Unknown, rReason }`; app-vs-test occurrence split; `GitContext` | text/CSV | Checked-in build artifacts inflate counts → silent **false negatives**. **Archived 2020** — but its architecture is the closest existing prototype |
| **Erlang `xref`** | Erlang/OTP | Cross-reference server, in stdlib since the 1990s | Erlang terms | Predefined analyses `undefined`, `unused`, `locals_not_used`, `exports_not_used`, `deprecated`; a query algebra (`closure`, `\|`/`\|\|`/`\|\|\|`, `E`/`X`/`L`/`XC`/`LC`, `Mod`/`App`/`Rel` coercion). **Requires `debug_info` for local functions — an explicit, checkable evidence precondition** |
| **weeder** | Haskell | GHC `.hie` files (`-fwrite-ide-info`) → symbol-resolved graph, traversed from **regex roots in `weeder.toml`** | — | `type-class-roots` is a single explicit soundness dial for the instance-method problem. Ships `^Paths_.*` as a default root purely to suppress Cabal's generated module — even this tool needed a generated-code escape hatch on day one |
| **cppcheck** *(added — C/C++ was absent from this survey)* | C/C++ | `--enable=unusedFunction` builds a whole-program symbol-usage table across all TUs given to it | `--xml --xml-version=2`, `--template=`, SARIF via `cppcheck-sarif` wrappers | Documented by its own manual as needing the **whole program** in one invocation: it is disabled under `-j` (multi-threaded) analysis, and any TU not passed in reads as "no caller." Blind to `dlsym`, exported symbols, and library APIs. **Its silence is not evidence unless the `analysisTarget` set equals the full `compile_commands.json`** |
| **include-what-you-use / `clang -Wunused-*`** *(added)* | C/C++ | Clang AST; per-TU unused parameters/variables/functions (`-Wunused-function` fires only on `static` functions — the visibility boundary again) | compiler diagnostics; IWYU `--output=` mappings | Scope-local only, exactly like Ruff/Roslyn. IWYU answers a *different* question (unused #includes) and its removals routinely break other TUs that were transitively relying on the include |
| **`nm` / `objdump` / `readelf` / Bloaty McBloatface** *(added)* | any ELF/Mach-O/PE | Symbol table and section sizes of the **shipped artifact** | Bloaty `--csv`; `nm --defined-only --demangle` | The B-family proof for C/C++/Rust/Go/Swift, and the natural pairing with `--gc-sections`+`--print-gc-sections` (§2.1). Confounded by ICF folding, LTO internalization, and inlining — absence of a symbol proves it was not *emitted*, never that it was not *executed* |
| **detekt / IntelliJ inspections** *(added — Kotlin)* | Kotlin | `UnusedPrivateMember`, `UnusedPrivateProperty`, `UnusedParameter`; `internal` visibility closes the module world | `--report sarif:`/`xml:`/`html:` | Private/internal only. Blind to Spring/Android reflection, `@Serializable` synthesis, and Compose `@Preview` |
| **`scalac -Wunused` / scalafix `RemoveUnused`** *(added — Scala)* | Scala 2.13+/3 | Compiler-driven unused imports/privates/locals/params; scalafix applies the fix from the compiler's own diagnostics | scalafix patches; compiler diagnostics | Requires `-Wunused:all` (and `-Ywarn-unused` on 2.12); implicit/given resolution and macro expansion routinely make a "unused" import load-bearing |
| **RuboCop `Lint/UselessAssignment`, `Lint/UnusedMethodArgument`** *(added — Ruby)* | Ruby | Per-scope liveness (the Ruby analogue of Ruff F841) | `--format json`/`sarif` (via `rubocop-sarif`) | Scope-local. The cross-file question in Ruby is answered only by `debride` + `debride_rails_whitelist` (below), because Zeitwerk resolves names from paths at runtime |
| **Semgrep / ast-grep** | 30+ / any tree-sitter | pattern matching | `--json`, `--sarif`, `--gitlab-sast`; ast-grep `--json=stream` | Use as a **negative** signal: count dynamic-dispatch idioms to lower the tier ceiling. Never as a positive one |
| **scc / tokei** | all | LOC + complexity | `--format json\|csv\|html\|sql\|openmetrics` | Nothing about usage. Supplies **size**, the correct risk-weighting input (cf. debride `--minimum 30`, Vulture `--sort-by-size`) |

### 4.2 Portable substrates

| Substrate | What it gives | Verdict |
|---|---|---|
| **SCIP** (Sourcegraph) | Protobuf index of occurrences with a `SymbolRole` bitfield: `Definition=0x1, Import=0x2, WriteAccess=0x4, ReadAccess=0x8, Generated=0x10, **Test=0x20**, ForwardDefinition=0x40`. Indexers: Java/Scala/Kotlin, TS/JS, Rust (rust-analyzer), C/C++ (scip-clang), Ruby, Python, C#/VB, Dart, PHP. CLI: `scip print --json`, `scip stats`, `scip lint`, `scip expt-convert` → SQLite | **The best cross-language primitive found.** The `Test` role gives you "never referenced" vs "referenced only by tests" vs "referenced only in generated code" *portably*. Requires a build per language. No built-in root-set concept — you supply roots |
| **LSIF** | Same class, larger clumsier format | Superseded by SCIP |
| **Glean** (Meta) | **BSD-licensed, open source since 2021** — the corpus repeatedly and incorrectly calls it internal. Angle query language over a scalable fact store. Full indexers: C++/C, Hack, Haskell, JS+Flow, Python. **Ingests SCIP and LSIF**, covering Rust, Go, TypeScript, Java, Python, .NET. Public Docker demo | Materially changes the design space: the "compiler-derived cross-language fact database" half of SCARF is off-the-shelf. Not-yet-open indexers: Java, Kotlin, Erlang, Thrift, Buck/Bazel, C#, Swift |
| **LSP** | `textDocument/prepareCallHierarchy` + `callHierarchy/incomingCalls` (3.16), `textDocument/references` with `ReferenceContext.includeDeclaration`, `workspace/symbol` + `workspaceSymbol/resolve` (3.17), `textDocument/prepareRename` (returns null when a rename is invalid at a position — a free server-authoritative "is this actually a symbol" check) | Knip's rejection is correct for **enumeration** (*"must be called per symbol… thousands of calls, each scanning potentially all files"*; heavy service init; one program per tsconfig; blind to `.vue`/`.svelte`) and a **non-argument for verification** on a shortlist of tens of candidates. Use as a per-candidate verification tier |
| **tree-sitter** (via ast-grep) | Parses broken/unbuildable code; any grammar | No type resolution. A tree-sitter-only substrate would be *strictly weaker than Vulture*, i.e. the worst tool in the survey. Correct role: dynamic-construct detection and mechanical rewrites |
| **stack-graphs** (GitHub) | Build-free name binding via scope graphs (Visser, TU Delft) | **Abandoned.** README: *"This repository is no longer supported or updated by GitHub."* `github/semantic` archived too. Its abandonment is the strongest evidence that build-free semantic resolution is not a viable foundation |
| **`bazel query` / `cquery` / `aquery`** | A full expression language over the target graph: `deps`, `rdeps(universe, x[, depth])`, `allrdeps` (Sky Query + `--universe_scope`), `same_pkg_direct_rdeps`, `somepath`/`allpaths`, `buildfiles`/`loadfiles`, `rbuildfiles`, `visible`, `tests`, `siblings`, `labels`, `kind()`, `filter()`, `attr()`, set ops. Output: `label`, `label_kind`, `graph`, `minrank`/`maxrank`, `package`, `xml`, `proto` | **Massively underused.** `bazel query 'kind("_library rule", //...) except rdeps(//..., kind(binary, //...) + kind(test, //...))'` answers the central question soundly for the analyzed config, in one line. `allpaths(...) --output minrank` gives a **witness path** for free (do *not* use `somepath` — the docs warn its ranking guarantees neither shortest nor longest). Caveats: `query` is post-loading and does not resolve `select()` correctly; `cquery` resolves configs; **`aquery` maps artifacts → producing actions**, which is your regenerability oracle |
| **Nix** | `nix-store --gc --print-roots/--print-live/--print-dead`, `--max-freed BYTES`, `--add-root`, `nix why-depends`, `nix-store --query --deriver/--referrers/--roots`, `nix path-info` | The only system that genuinely solved unknowable roots — by refusing to guess and requiring registration. Also a per-artifact provenance oracle, not just a GC |
| **Nx / Turborepo / pnpm / Rush** | `nx affected`, `nx graph --affected`, `turbo --affected`, `pnpm --filter '...[origin/main]'` | The JS analogue of Sensenmann's Blaze reachability, already shipped and inspectable. A project node with no inbound edges and no task targets is the JS equivalent of a Bazel library with no rdeps. **Absent from the entire research corpus despite JS/TS being the best-covered ecosystem** |
| **Language-enforced boundaries** | Go: *"Code in or below a directory named `internal` is importable only by code in the directory tree rooted at the parent of `internal`."* Rust `pub(crate)`/`pub(super)`/`pub(in path)`; C# `internal` + `[InternalsVisibleTo]`; Kotlin `internal`; JPMS `exports`; Bazel `visibility` + `package_group` | **A whole proof-grade tier the corpus left on the table.** These convert "unused public API in an open world" — repeatedly declared undecidable — into a closed-world question with a *compiler-checked* boundary. An `internal/` package with no importers in its parent subtree is provably dead, at file granularity, from data `go list -deps -json` already emits |
| **`.git-blame-ignore-revs`** | `git blame --ignore-rev`, `--ignore-revs-file`, config `blame.ignoreRevsFile` (repeatable, `#` comments, empty filename resets), `blame.markIgnoredLines`, `blame.markUnblamableLines`. GitHub honors it at repo root | The fix for a problem the corpus calls inherent (bulk reformats destroying VCS signal). Any VCS component **must** read it. `markUnblamableLines` is a free per-line "this evidence is degraded" flag. A cleaner performing a codemod should append its own SHA |
| **code-maat / CodeScene** | `abs-churn`, `age`, `author-churn`, `authors`, `coupling` (with `--min-revs`, `--min-shared-revs`, `--min-coupling` default 30%, `--max-changeset-size` default 30), `entity-churn`, `entity-effort`, `entity-ownership`, `fragmentation`, `main-dev`, `refactoring-main-dev`, `revisions`, `soc`, `summary` | **Change coupling** is a far better VCS signal than age, for two purposes the design needs: detecting **coupled-pair deletion hazards** empirically (flag+guard, migration+model, `.proto`+stub) rather than by hard-coded lists, and identifying a **dead cluster** (high mutual coupling, zero external coupling) — the shape abandoned features actually have. `--max-changeset-size` exists precisely to exclude bulk-reformat commits |

---

## 5. Root-set discovery

The 178-plugin problem. Root discovery is not one problem but ~200 small per-framework problems, and the industry's answer is brute-force enumeration.

### 5.1 Provenance tiers — the core data-model distinction

| Tier | Definition | Confidence | Examples |
|---|---|---|---|
| **A — machine-declared** | A build system or deploy target already reads this file to find roots | High; auto-discoverable | `package.json` main/module/browser/types/bin/exports/imports/workspaces/files/scripts; `pyproject [project.scripts]`/`[project.gui-scripts]`/`[project.entry-points.*]`; `setup.cfg console_scripts`; `Cargo.toml [[bin]]/[[example]]/[[bench]]/[[test]]` + `build.rs`; `go.mod` + every `package main`; `pom.xml <mainClass>`, shade/assembly `Main-Class`, Gradle `application { mainClass }`; gemspec `executables`; `composer.json bin`/`autoload.psr-4`/`autoload.files`; csproj `<StartupObject>`/`<TrimmerRootAssembly>`; `Dockerfile CMD/ENTRYPOINT`; `wrangler main`; `serverless.yml functions.*.handler` |
| **B — convention-inferable** | Framework file-layout or annotation conventions turn a file into an entry point with no source reference | Medium; correct only if framework + version detected correctly | Next.js `{,src/}app/**/{layout,page,route,template,default,error,loading,not-found}.{ext}`, `app/**/sitemap`, `app/{manifest,robots}`, `app/**/{icon,apple-icon,opengraph-image,twitter-image}`, `{instrumentation,middleware,proxy}`; Nuxt `pages/`,`layouts/`,`middleware/`,`server/api/`,`plugins/`,`composables/`; SvelteKit `+page`/`+layout`/`+server`/`hooks`; Django `urls.py` + `INSTALLED_APPS` + `<app>/management/commands/*` + `apps.py::ready()` + `templatetags/`; Rails routes + Zeitwerk paths + `db/migrate/*` + `app/jobs`; Spring `@ComponentScan`; Laravel service providers; pytest `conftest.py`/`test_*.py`; Airflow DAG folder scan; dbt `models/**/*.sql` |
| **C — undiscoverable, must be declared** | The live set is determined by data or intent outside the repository | None; must be **solicited** from a human | Rails STI subclass names in a DB `type` column (Rails' own upgrade guide ships a script that *reads the DB*; GitLab #215914 documents the Zeitwerk workaround); Sidekiq/Celery/ActiveJob queues holding serialized class names; feature-flag configs in LaunchDarkly/Unleash; A/B variant registries; CMS-stored template names; DB rows holding dotted class paths; ops runbooks; a human's shell history; downstream consumers of a published library; customer-authored plugins; `Class.forName(prefix + userInput)` |

**No amount of static cleverness moves Tier C into A or B.** The correct product move is to *solicit* it: at init, ask a short explicit questionnaire (does anything load classes from the database? are there human-run ops scripts? is this a published library with unknown consumers? are there feature-flagged paths currently off? is there a job queue holding serialized class names?) and record the answers as declared roots.

### 5.2 The checklist

**Auto-discoverable (Tier A) — parse these:**

- [ ] **JS/TS** — `package.json`: `main`, `module`, `browser`, `types`, `typesVersions`, `bin` (string or map), `exports` (every leaf of the nested conditional map), `imports` (`#`-prefixed subpath imports), `workspaces`, `files`, `scripts`, `type`, **`sideEffects`** (a `false` here is a *declaration that tree-shaking may drop modules* — and a module listed in `sideEffects` is a declared root), `postinstall`/`prepare` lifecycle scripts. Plus: bundler-resolved roots invisible to the module graph — Vite `import.meta.glob`/`?raw`/`?url`/`?worker`, webpack `require.context`, `new Worker(new URL('./w.ts', import.meta.url))`, `public/`/`static/` copy-through directories, `service-worker.js`/`sw.js`, import maps, `browserslist`, and any `nx.json`/`turbo.json` pipeline target
- [ ] **Python** — `pyproject.toml`: `[project.scripts]`, `[project.gui-scripts]`, `[project.entry-points.<group>]` (note the `pytest11`, `console_scripts`, `flake8.extension`, `babel.extractors`, `sqlalchemy.dialects` groups specifically — a package whose *only* consumer is an entry-point group is structurally invisible to `deptry`, §4.1), `[tool.*]` plugin registrations; `setup.py`/`setup.cfg` `console_scripts`, `packages`, `py_modules`; `MANIFEST.in`. Plus implicit roots: `__main__.py` (`python -m pkg`), `wsgi.py`/`asgi.py`, `manage.py`, `conftest.py`, `celery.py`, `tox.ini`/`noxfile.py` env commands, and — the one nobody models — **`sitecustomize.py`/`usercustomize.py` and `.pth` files in site-packages, whose lines beginning with `import` are *executed at interpreter start* (`site` module semantics). A `.pth` file is an entry point with no caller anywhere.**
- [ ] **Rust** — `Cargo.toml`: `[[bin]]`, `[[example]]`, `[[bench]]`, `[[test]]`, `[lib]` (including **`crate-type = ["cdylib","staticlib"]`**, which means the consumer is outside the crate graph entirely), `[workspace] default-members`/`members`, `[features]`, `[target.'cfg(...)'.dependencies]`, plus implicit `src/main.rs`, `src/lib.rs`, `src/bin/*.rs`, `examples/`, `benches/`, `tests/`, `build.rs`. In-source roots: `#[no_mangle]`, `#[used]`, `#[export_name]`, `#[ctor]`, `#[wasm_bindgen]`, `#[pyo3::pymodule]`, `#[unsafe(naked)]` — every one is "reachable from outside the language."
- [ ] **Go** — `go.mod` + every `package main`; `//go:generate`; `//go:embed`; `//go:linkname`; `//go:build` tag sets (each is a *separate configuration* — §6.4); `//export <name>` cgo entry points; `//go:wasmexport`; `TestMain`; `-buildmode=plugin`/`c-shared` targets (consumers are outside the build graph by construction); `internal/` boundaries as a *closed-world* root scope (§4.2)
- [ ] **JVM** — `pom.xml <mainClass>`, shade/assembly `Main-Class`, Gradle `application { mainClass }`, `sourceSets`, `META-INF/services/*` (contents are FQCNs), **`module-info.java` (`exports`, `opens` — `opens` is a *declaration that reflection will occur* — `provides … with`, `uses`)**, `MANIFEST.MF` `Main-Class`/**`Premain-Class`/`Agent-Class`/`Launcher-Agent-Class`** (java agents run before `main` and are referenced by nothing), `web.xml`, `persistence.xml`, `beans.xml`, `faces-config.xml`, Spring `META-INF/spring.factories` **and its Spring Boot 3 replacement `META-INF/spring/org.springframework.boot.autoconfigure.AutoConfiguration.imports`** (a plain-text list of FQCNs), `@SpringBootApplication(scanBasePackages=…)`, `application.yml`/`.properties` values that are class names, **Android `AndroidManifest.xml` `<activity>`/`<service>`/`<receiver>`/`<provider>`/`<meta-data android:value>` — the single largest string-referenced root set in the JVM world** — plus `proguard-rules.pro`/`consumer-rules.pro` `-keep` seeds and `res/**` referenced by generated `R` fields
- [ ] **.NET** — csproj `<StartupObject>`, `<TrimmerRootAssembly>`, `<TrimmerRootDescriptor>`, `ILLink.Descriptors.xml`, `[DynamicDependency]`, `[ModuleInitializer]` (runs before `Main`, called by nothing), `*.runtimeconfig.json`, `[assembly: InternalsVisibleTo]` (**widens the closed world — the `internal` proof in §4.2 is void for any assembly carrying one**), `appsettings.json` values that are type names, `*.resx`, source generators and analyzers referenced via `<ProjectReference OutputItemType="Analyzer">`, MSBuild `.targets`/`.props`, ASP.NET `Startup`/minimal-API top-level statements
- [ ] **Ruby** — gemspec `executables`/`files`/`require_paths`, `Gemfile`, `config.ru` (the Rack entry point), `config/routes.rb`, `config/initializers/*` (loaded by convention, referenced by nothing), `config/application.rb` `eager_load_paths`/`autoload_paths` (Zeitwerk infers class names *from file paths* — the file path IS the reference), `Rakefile` + `lib/tasks/*.rake`, `app/jobs`/`app/mailers`/`app/channels`, `config/sidekiq.yml` + `config/schedule.rb` (whenever), `Capfile`, engine `lib/<name>/engine.rb`
- [ ] **PHP** — `composer.json` `bin`, `autoload.psr-4`, `autoload.files`, `autoload-dev`, `scripts`, `extra.*` framework hooks; `public/index.php` front controller; Laravel `routes/*.php`, `config/app.php` `providers`/`aliases`, `bootstrap/app.php`, `app/Console/Kernel.php` schedule; Symfony `config/bundles.php`, `config/services.yaml` (including autowire/autoconfigure and `!tagged_iterator`), `config/routes*.yaml`, `src/Kernel.php`; WordPress plugin/theme header comments plus every `add_action`/`add_filter`/`register_activation_hook` string callback; Twig/Blade template directories (§4.1: *"when used only in templates, apart from Twig paths, it's not possible to detect them"*)
- [ ] **Swift / Objective-C** *(absent from the original checklist — added)* — `Package.swift`: `products` (`.executable`, `.library`, **`.plugin`**), `targets` (`executableTarget`, `testTarget`, `macro`, `plugin`), `resources` (`.process`/`.copy` — files reached by `Bundle.module` at runtime, never by an import), `linkerSettings`. Xcode: `.xcodeproj`/`.xcworkspace` **schemes and build configurations — scheme selection is a correctness input, not a performance knob (§6.20)**, target membership, Build Phases `Copy Bundle Resources`/`Run Script`. `Info.plist`: `CFBundleExecutable`, `NSPrincipalClass`, `NSExtensionPrincipalClass`/`NSExtensionMainStoryboard` (app extensions), `UIApplicationSceneManifest` `UISceneDelegateClassName`, `UIMainStoryboardFile`, `CFBundleURLTypes`, `NSUserActivityTypes`, `BGTaskSchedulerPermittedIdentifiers`. `.entitlements`, `*.podspec`, `*.xcconfig`, `.xcassets` (asset names are strings), `.xib`/`.storyboard` `customClass`/`customModule` and IBAction/IBOutlet targets, `@main`/`@UIApplicationMain`/`@NSApplicationMain`, `@objc`/`dynamic`/`NSObject` subclasses reachable from the ObjC runtime, `#Preview`/`PreviewProvider`, App Intents & `AppShortcutsProvider` (discovered by the OS at install time), `@_cdecl`, `@_exported`
- [ ] **C/C++** *(absent from the original checklist — added)* — `CMakeLists.txt`: `add_executable`, `add_library` (**`SHARED`/`MODULE` means the consumer is outside the build entirely**), `target_sources`, `install(TARGETS|FILES)`, `add_custom_command`/`add_custom_target`, `enable_testing`/`add_test`, `CMakePresets.json` (each preset is a *configuration*, §6.4). Other build systems: `Makefile` targets, `meson.build` (`executable`, `shared_library`, `install_data`), Autotools `Makefile.am`/`configure.ac`, `BUILD`/`BUCK`, `vcpkg.json`, `conanfile.py|txt`, `*.pc` pkg-config files (declare the *public* headers and libs for consumers you cannot see). Linker- and source-level roots that defeat every reachability pass: **version scripts (`--version-script`, `*.map`, `VERSION { global: … }`), Windows `.def` export lists, `__attribute__((visibility("default")))`, `__attribute__((used))` / `((retain))` / `SHF_GNU_RETAIN`, `__attribute__((constructor))`/`destructor` and C++ static-initializer self-registering factories, `KEEP()` directives in linker scripts, `--whole-archive`, weak symbols, `.init_array`/`.ctors`, `extern "C"` symbols consumed via `dlsym`, `#pragma comment(linker, "/include:…")`, and every header installed by `install(FILES)` — a header is an API surface with no in-repo caller.** Also: `compile_commands.json` is the *only* machine-readable statement of which translation units were actually compiled, and is the natural `analysisTarget` set (§9.2)
- [ ] **Containers:** `Dockerfile` `CMD`/`ENTRYPOINT` (exec-form JSON array *and* shell form), `RUN`, `COPY`/`ADD` (source paths!), `COPY --from` build stages; `docker-compose` `command`/`entrypoint`/`healthcheck`/bind-mount sources; `.dockerignore` negations
- [ ] **Orchestration:** Kubernetes `command`/`args` in every container spec; Helm templates (post-render); `serverless.yml functions.*.handler` (`src/api/user.handler` → strip after last dot → `src/api/user.{js,ts}`); SAM/CloudFormation `AWS::Serverless::Function` `Handler` + `CodeUri`; CDK/Terraform `handler:`/`entry:`/`filename:` properties
- [ ] **Process managers:** `Procfile`, systemd `ExecStart`/`ExecStartPre`, supervisord `command`, crontab lines
- [ ] **CI:** `.github/workflows/*.yml` `run:` bodies; composite `action.yml` (`runs.pre|main|post` when `runs.using` starts with `node`); GitLab CI `script`/`before_script`; Jenkinsfile `sh`/`bat`; Travis; `.pre-commit-config.yaml` `entry:`; `artifacts.paths` / `upload-artifact with: path` / `cache: paths`
- [ ] **Task runners:** `Makefile`, `Justfile`, `Taskfile`, `package.json#scripts`, Nx `project.json` `nx:run-commands`, `release-it hooks`, `lint-staged` values, git hooks (`.git/hooks/*`, husky, lefthook `run:`, simple-git-hooks, yorkie)
- [ ] **Hosting/platform contracts** (see hazard §6.11): `CNAME`, `.nojekyll`, `_redirects`, `_headers`, `vercel.json`, `netlify.toml`, `static.json`, `apple-app-site-association`, `.well-known/assetlinks.json`, `robots.txt`, `security.txt`
- [ ] **Governance:** `CODEOWNERS`, `.github/dependabot.yml`, `renovate.json`
- [ ] **Agent context** (this project's own domain): `CLAUDE.md`, `AGENTS.md`, `.cursorrules`, `SKILL.md`, MCP server configs, compiled context briefs — these reference files an agent will load by path and no analyzer sees
- [ ] **Runtime configs:** Django `settings.MIDDLEWARE`/`INSTALLED_APPS`/`AUTHENTICATION_BACKENDS`/`TEMPLATES.OPTIONS.context_processors`/`CELERY_*` dotted paths; logging `dictConfig` `()` factories; ESLint `extends`/`plugins`; Babel presets; webpack loaders

**Must be declared (Tier C) — solicit these:**

- [ ] Class/handler names stored in a database
- [ ] Job queues holding serialized class names
- [ ] Feature flags currently off but not retired
- [ ] Ops/runbook scripts invoked by humans
- [ ] Downstream consumers outside this repository
- [ ] Customer-authored plugins / extension points
- [ ] Deployment environments not covered by telemetry

### 5.3 The keep/retain DSL comparison

Five distinct designs exist. They are not equivalent.

| Design | Exemplars | Syntax | Strength | Weakness |
|---|---|---|---|---|
| **1. Glob over files** | Knip `entry`/`project`/`ignore` (tinyglobby+picomatch, `!` negation, per-workspace override, `paths` alias map); Periphery `--retain-files`, `--index-exclude`, `--report-exclude`, `--exclude-targets` | gitignore-ish | Cheap, portable, universally understood | File-granular only |
| **2. Pattern-matching symbol specs** | ProGuard: `-keep`/`-keepclassmembers`/`-keepclasseswithmembers` × `/-names` (a clean 2×3 matrix over remove-vs-rename), `?`/`*`/`**` where `*` stops at the package separator and `**` doesn't, `<n>` backreferences, `@annotation` filters, `extends`/`implements`, access-modifier and return-type matching | rich class-spec grammar | Most expressive | Also the one the ecosystem is famous for getting wrong: `-keep class **.* { *; }` copy-paste silently defeats shrinking |
| **3. Conditional roots** | GraalVM `{"condition":{"typeReached":"com.example.Foo"}, "type":..., "methods":[...]}`; .NET `[DynamicDependency]` | JSON / attribute | **The single best idea in the survey.** "Keep X, but only if Y is reachable" — a rule that automatically stops applying when its guard dies, which is the only mechanism found anywhere that prevents monotone over-retention drift | Needs a reachability model to evaluate the condition |
| **4. Executable fake-usage whitelist** | Vulture `--make-whitelist > whitelist.py`, then feed `whitelist.py` back as a scanned input | real source code | Self-validating: syntax-checked, and can be *executed* to confirm every whitelisted symbol still exists. Zero new DSL | Language-specific; can't express "keep this file" |
| **5. Manifest-colocated ignores** | cargo-machete `[package.metadata.cargo-machete] ignored = ["prost"]`, `[workspace.metadata...]`, plus a `renamed` map | TOML next to deps | Reviewed in the same PR as the code it protects. The `renamed` map **fixes** the name-mismatch class rather than muting it | Ecosystem-specific location |
| *Orthogonal:* **in-source directives** | Periphery `// periphery:ignore`, `:ignore:all` (file-level, must precede imports), `:ignore:parameters a,b`, `// periphery:override kind=… location=file:line:col` (relocate a finding out of generated code), trailing `- explanation` after a hyphen; Knip JSDoc `tags` with `+`/`-` include/exclude; `# noqa: F401`; `@api`; `[DynamicDependency]` | comments | Lives with the code | Invisible in rendered output; rots silently |

**Recommendation: model after a hybrid, with GraalVM's conditionality as the headline.**

1. **Matching syntax → gitignore pathspec.** Universally known; git-native matching is available as a library so your semantics cannot drift from git's; negations work; per-directory hierarchical files work; nobody has to learn anything.
2. **Suppression semantics → SARIF's `suppressions` object, verbatim.** `kind: inSource | external` maps exactly onto comment-directive vs manifest-file. `status: accepted | underReview | rejected` gives the three states you actually need — permanent keep / pending human decision / *human examined this candidate and said the tool was wrong* — which is strictly better than the binary keep-lists in ProGuard, Periphery, and Vulture. `justification` is the mandatory reason field.
3. **Conditionality → GraalVM `typeReached`.** `keep X when reachable(Y)`. This is what stops your keep file from silently becoming the tool's off switch.
4. **Location → manifest-colocated** (cargo-machete), committed and reviewed in PRs.
5. **Symbol matching where you have symbols → ProGuard's `*` vs `**` semantics.** Take the syntax, reject the culture.

**Two additions nothing has:**
- Every entry carries a **mandatory `reason`** and an optional `expires: YYYY-MM-DD`.
- The ruleset **lints itself every run** and fails CI on: rules that matched nothing (Periphery's superfluous-ignore warning, generalized), rules whose referents no longer exist (Vulture's executable-whitelist property, generalized), and rules past expiry. Knip's `treatConfigHintsAsErrors`/`treatTagHintsAsErrors` is the precedent. *A suppression list without rot detection is the off switch.*

**And one design principle:** for every suppression, first ask whether it can be a **CORRECTION** (teach the tool the real name/edge — cargo-machete's `renamed` map) rather than a **MUTE**. A correction improves precision permanently; a mute creates a blind spot permanently.

---

## 6. The hazard catalog

Every documented mechanism by which an analyzer wrongly concludes "unused." For each: the code shape, which technique it defeats, and a **detectable counter-signal** you can implement.

This is the longest section on purpose. It is the specification for the veto layer.

### 6.1 Reflection and dynamic dispatch

**Shapes.** Python: `getattr(module, name)()`, `globals()[name]`, `setattr`, `__getattr__` at module level, `importlib.import_module(f"plugins.{n}")`, `__init_subclass__`/metaclass registries, `eval`/`exec`, `frame.f_locals.get("name")`. JVM: `Class.forName(s)`, `getDeclaredMethod`, `getAnnotation`, `ClassLoader.loadClass`, `Proxy.newProxyInstance`, `MethodHandles.lookup()`, Spring/Guice DI by type, `ServiceLoader`. Go: `reflect.TypeFor[T]()` + `typ.Field(i).Name`, `reflect.ValueOf(x).Call(...)`, `//go:linkname`. Swift: `@objc`/`NSObject` subclasses reachable from the ObjC runtime, `NSClassFromString`, `subscript(dynamicMember:)`. JS: `obj[key]()`, `Object.entries(handlers)[k]`, `new Function`. Ruby: `send`/`public_send`/`const_get`/`method_missing`/`define_method`. PHP: `__call`, `call_user_func`. .NET: `Activator.CreateInstance`, `Type.GetType`, `Assembly.Load`, `[ModuleInitializer]`, DI registration by open generic. **Kotlin:** `::class.java` + Java reflection, `@Serializable` plugin-generated serializers, Compose `@Preview`. **Rust** *(absent from the original list — Rust has no reflection, which is exactly why its equivalent is easy to miss)*: link-time registries built with `inventory`, `linkme`, `ctor`, and `#[distributed_slice]` — a `submit!` macro places the item in a custom link section that nothing in the source ever names, so **the call graph is genuinely empty and the item still runs**; also `Any::downcast_ref`, `#[no_mangle]`/`#[export_name]` symbols consumed by C, and `wasm_bindgen`/`pyo3`/`napi` exports whose caller is another language. **C/C++:** `dlopen`/`dlsym`, plus the far more common **static-initializer self-registering factory** (`static Registrar r{"name", &make};` at namespace scope, or `__attribute__((constructor))`), where a translation unit's *only* purpose is a side effect at load time and it has no callers by construction — the C++ analogue of §6.14's import side effects, and the reason `--gc-sections` needs `KEEP()`/`((used,retain))` escape hatches at all.

**Defeats.** Static reachability, compiler index, build graph. And for *structural* reflection (Go's `reflect.TypeFor` + `Field(i).Name`, staticcheck #1648) it defeats even the grep veto — **there is no identifier string anywhere to match**, because the reflection is over shape, not name.

**Counter-signals.**
- Presence of any reflection primitive in the module or its transitive importers → cap the tier for the whole directory, not just the file.
- The candidate's declaring type participates in a hierarchy whose base is instantiated dynamically elsewhere.
- The candidate is a whole struct/class whose *fields are individually unreferenced* — a strong tell for serialization or reflection.
- Run one Semgrep/ast-grep pass and emit a repo-wide **undecidability report** (§9). Any module in it is ineligible for auto-act.

### 6.2 String-built and config-driven references

**Shapes.** Django `INSTALLED_APPS = ['myapp.SomeConfig']`, `MIDDLEWARE`, dotted-path `urlpatterns`; Celery `task_routes`; Spring `@ComponentScan("com.x.y")`; Java SPI `META-INF/services/<iface>` whose *contents* are FQCNs; `application.yml` class names; systemd/cron invoking `/opt/app/bin/foo`; SQL in `.sql` files; k8s/Terraform/Ansible referencing script paths; ESLint `extends: 'airbnb'` resolved through node_modules. And the runtime-constructed variants Meta names explicitly: **`"tbl_" + region`, and table names stored in a database rather than in code**.

**Defeats.** All pure-code reachability. The name exists, but not in a file the parser reads — or not as a contiguous literal at all.

**Counter-signals.**
- **Whole-repo literal veto over every file type** (§6.20). Mandatory.
- For the concatenation case: if the basename minus a common prefix/suffix appears as a literal, or any string in the repo is a *prefix* of the candidate's name, block. Meta's prefix-tree query design over BigGrep is directly reusable.
- Detect string-concatenation into a dispatch call (`getattr(mod, prefix + name)`) syntactically and mark the whole registry region ineligible.

### 6.3 Build-time code generation and macro expansion

**Shapes.** protobuf/gRPC `.proto` → stubs referencing hand-written impls; Thrift/GraphQL schemas; ORM models generated from schema; Rust proc macros and `macro_rules!` (cargo-udeps #143: *"a whole bunch of false positives, most of which are coming from either derive macro or macro in general"*); C++ templates instantiated only in a generated TU; Java annotation processors (Dagger, Lombok, MapStruct); codegen that reads source at build time (OpenAPI, `build.rs`, Gradle tasks); Swift's synthesized `Codable`/`Equatable`/`Hashable`. Meta names *"compilation artefacts which reference classes but are not committed to a repository"* as its own hazard class.

**Defeats.** Any analysis on pre-expansion source; also any analysis on post-expansion output, since the generated file references the source but the source may reference nothing back.

**Counter-signals.**
- A build step whose output directory is gitignored → treat every *input* to it as rooted.
- Generated-file markers: `Code generated by ... DO NOT EDIT.`, `@generated`, `.gitattributes linguist-generated`. staticcheck rule 1.9 marks everything in generated files as used — the right default.
- Presence of `build.rs`, `*.proto`, codegen config, or an annotation-processor dependency in the manifest → downgrade the whole workspace to review-only.
- GitHub Linguist's `generated.rb` predicate chain (minified files, source maps, ANTLR/racc/JFlex/Jison/GrammarKit output, compiled CoffeeScript/Cython, JNI headers, .NET designer/SpecFlow, Xcode/IntelliJ/CocoaPods/Carthage artifacts, Unity `.meta`, VCR cassettes, roxygen2, htmlcov, gradle/maven wrappers, and ~25 lockfile formats) is MIT-licensed, regression-tested, and directly vendorable.

### 6.4 Conditional compilation and platform branches

**Shapes.** C/C++ `#ifdef _WIN32`; Rust `#[cfg(target_os="windows")]`, `#[cfg(feature="tls")]`, `[target.'cfg(...)'.dependencies]`; Go `//go:build !amd64 || appengine` and `_windows.go`/`_arm64.go` suffixes; Swift `#if DEBUG`; Python `if sys.platform == 'win32'`, `sys.version_info >= (3,12)`, optional-import `try: import uvloop except ImportError:`; JS `process.platform`, bundler `define` replacement, `.native.js`/`.web.js` resolution.

**Defeats.** Every compiler-index and build-graph technique, because **the index describes only the one configuration that was built.**

**Evidence this is unfixed, not theoretical:**
- staticcheck **#48, open since 2016**: `popcountHD` in `x/tools/container/intsets` reported dead; its only use is in a file guarded by `// +build !amd64 appengine`. Maintainers left it open rather than ship an unsound fix.
- Periphery documents the workaround as *"run Periphery once for each build configuration and merge the results"* — N builds — and its README's own example flags `releaseName` as unused because it's only referenced in the `#else` branch.
- Go `deadcode`: *"The analysis is valid only for a single GOOS/GOARCH/-tags configuration."*
- `go mod tidy` removes deps needed behind build tags: golang/go #65054, #39570, #58216 (platform-dependent, also hit in Dependabot).

**Counter-signals.**
- Tree-sitter can find `#if`, `#[cfg]`, `//go:build`, and platform-suffixed filenames cheaply across languages.
- Manifest declares features/extras/optional deps not enabled in this run.
- CI matrix declares OS/arch/version combinations the analysis did not cover.
- **Rule:** either analyze every configuration and **intersect** the dead sets, or refuse to act on any file guarded by a conditional you did not evaluate. cargo-shear's counter-example is instructive: because it uses static parsing without compiling, *"it only needs to run once on a single platform to detect issues across all target platforms."* Prefer parsing over compilation where you can.

### 6.5 Feature flags and dark launches

**Shape.** `if (flags.isEnabled('new-checkout')) { newPath() } else { oldPath() }` where the flag has been 0% for months; or a kill-switch fallback that only runs when a third-party API fails.

**Defeats.** Coverage and production profiling see only one branch. Static analysis sees both (so no FP there) — which is exactly why *fusing coverage* is hazardous here.

**Documented incidents.**
- A team removed a flag that had been off for over a year. The *code* that read the flag was still there; with the flag gone the read fell back to its compiled default of `true`, **silently re-enabling a feature nobody had thought about in a year.** User-reported incident.
- Removing a flag that gated a third-party-API fallback path caused a payment endpoint to 500 for 12% of users, ~$47k in failed transactions before rollback.
- Another team's frontend/API flag pair spanned two repos with an invisible dependency; removing one half broke the feature.

Meta names this as one of three sources of deprecation candidates: *"Disabling the feature flags of a deprecated feature is not sufficient to ensure the eventual deletion of underlying data schemas and associated data."*

**Counter-signals.** Identifier or string match against a flag SDK (`LaunchDarkly`, `Unleash`, `Statsig`, `flipper`, `gate`, `experiment`, `treatment`, `rollout`); a flag-config file listing the name; a branch condition whose predicate is not a compile-time constant. Any hit → never auto-delete; surface as a **paired** removal task (flag config + both branches) requiring a human. Uber's Piranha is the model for the correct architecture: it does not *infer* deadness — it reads an external system of record (the flag service says the flag is retired) and then performs the transformation.

### 6.6 Rarely-executed paths — the nightmare case

**Shapes.** `except OperationalError:` reconnect logic; `on_shard_failover()`; `scripts/restore_from_backup.sh`; `bin/rotate_signing_key`; `jobs/fiscal_year_close.py`; a SOX/GDPR audit export run twice a year; the runbook script executed once every 18 months at 3am during an incident.

**Why this is categorically worse than "noisy."** Coverage is **systematically anti-correlated with the value of the code.** The rarer a recovery path is, the less it is exercised, the more confidently a coverage-fused score marks it dead, and the higher the cost when it is missing — and the miss is discovered *only during the emergency the code existed for*, when you have the least capacity to diagnose "file not found."

Every safety property collapses here too. SCARF's quarantine window catches errors that surface within days; a yearly job's absence surfaces in eleven months, long after the revert window closed and the deleting commit is buried under thousands of others.

Practitioner statement on the Sensenmann HN thread (kortilla): *"This is how you end up deleting stuff only called in rare but critical cases: during outage, at end of year, during audits, when the one special customer that paid a fortune for an obscure feature decides to use it… 3 months is an eye blink in the business and govt world."* Corroborated in-thread by eitland, who instruments suspicious code and now receives *"every new years day… a weird sms message at 14:00 or 14:01 from a system that no one can find."*

Picnic, with a clean 0.03%-overhead fleet-wide production JaCoCo deployment: *"this does not mean we can now delete all code without coverage. Some logic might be used in seasonal cases, demos, or emergencies."*

**Counter-signals, all cheap:**
- Path/name lexicon: `disaster`, `recovery`, `restore`, `failover`, `rollback`, `backfill`, `migrate`, `oncall`, `runbook`, `incident`, `audit`, `annual`, `quarterly`, `yearly`, `eoy`, `fiscal`, `emergency`, `breakglass`, `panic`, `fallback`, `retry`, `seed`, `demo`, `admin`.
- The code sits in an `except`/`catch`/`rescue`/`recover`/`if err != nil` handler or a `finally`.
- Referenced from a scheduler with a low-frequency cron expression (`0 0 1 1 *`) or from `docs/runbooks/`.
- A top-level executable script with no in-repo caller — the archetype.

**Rule: hard-exclude this class from any auto-act tier regardless of how many signals agree, because the signals are correlated through the same cause.**

### 6.7 Documentation, examples, and demo code

**Shapes.** `examples/`, `demo/`, `samples/`, doctests, README snippets extracted by a test harness, `cookbook/`, benchmark harnesses, `contrib/`, tutorial apps.

Sensenmann names this first among its exceptions: *"some program code is there simply to serve as an example of how to use an API."* Meta notes the culture had to change: *"before automated code removal it was common practice to commit unused code as an example or for future use."*

**Second-order effect:** deleting an example is often *worse* than deleting a function, because the example is the only documentation of an API's intended use.

**Counter-signals.** Directory-name lexicon; the file is referenced from a `.md`/`.rst`/`.mdx`/`.ipynb`, a docs-site config (`mkdocs.yml`, `docusaurus.config.js`, mdbook), or a doctest runner; the module docstring says "example." A file whose *only* inbound reference is from documentation is documentation infrastructure, not dead code.

**⚠ Cross-feature interaction.** Files existing solely to back doc examples (embedme sources, mdcode `file=` targets, `#[cfg(doctest)]` items) have **no production callers** and will look dead to every reachability pass. **The doc-reference index must be a first-class ROOT SET for the dead-code component**, or the doc feature and the code feature actively sabotage each other.

### 6.8 Test-only usage — and both sides of the argument

**The case that it's dead.** Production code reachable only from tests is by definition not serving users. The test's existence is circular self-justification. ts-prune's documented shortcoming was that it *"made no attempts to understand whether test code was alive or dead."* Sensenmann's SCC machinery exists precisely because otherwise *"we would only be able to clean up untested code."*

**The case that it's not.** The helper may be deliberate test infrastructure (fixture builders, fakes, property generators). It may be the public API of an internal library whose only in-repo consumer happens to be a test. And most importantly: **tests are executable specifications.** `test_rejects_negative_quantity` and `test_rejects_zero_quantity` may have byte-identical coverage and encode two separately-negotiated business rules. Deleting one silently deletes a requirement recorded nowhere else — and the deletion is invisible to every subsequent signal: code gone, test gone, coverage goes *up*, CI green, and the next engineer reintroduces the bug the test was written for.

**Resolution.** Never collapse these. Report three classes:
1. `dead everywhere` → normal pipeline.
2. `reachable only from tests` → propose deleting **the pair**, requiring explicit human intent, never auto-act.
3. `test-infrastructure` (helpers reached from ≥2 unrelated test files) → leave alone.

Note Google's unsolved attribution problem: matching test→subject uses **edit distance on names** plus a `testonly` convention, and the LZW-vs-`web_test` example shows two topologically identical graphs needing opposite treatment. Coverage-based matching is named as "not yet explored" — a genuine open opportunity.

### 6.9 Public API surface — the category error

A library's exports have no in-repo callers *by definition*. This is not a bug in the tools; it is a category error the tool must refuse to make.

How mature tools handle it: Knip has production mode plus roots from `package.json#main/module/exports/bin`, and **by default does not report unused exports of entry files at all** (`--include-entry-exports` to opt in). Periphery has `--retain-public` and a separate `--no-retain-spi` for auditing inside `@_spi` groups. staticcheck rules 1.1–1.4 mark all exported package-level types/functions/vars/consts used unconditionally. Rust's `dead_code` ignores `pub` in a lib crate.

**But the guard fails open exactly where risk is highest.** The highest-risk shape has **no manifest**: `org/shared-schemas` holding `openapi/payments-v1.yaml`, consumed by 12 downstream repos that fetch it by raw URL and run codegen. No `package.json`, no `pyproject`, no tests, no runtime, nothing in-repo references it. Every signal returns UNUSED with maximum confidence, and the library guard does not fire because it keys on an artifact this class does not have. Same shape: internal GitHub composite-action repos consumed via `uses: org/repo@main`; Terraform module repos; Helm chart repos; Ansible role repos; JSON Schema and protobuf registries.

**Counter-signals.** Presence of a distribution manifest (`package.json#exports/main/bin`, `pyproject [project] scripts`/entry-points, `Cargo.toml [lib]`, `go.mod` module path, `*.podspec`, `*.gemspec`); a publish step in CI (`npm publish`, `twine`, `cargo publish`, `gh release`); a `CHANGELOG` with semver; the repo being a monorepo package other workspaces import.

**Inverted rule (the one nobody states): ABSENCE of a distribution manifest in a repo that is mostly non-code is itself grounds for refusal, not for proceeding.**

### 6.10 External effectors — deletion as an imperative destroy

**The largest single gap in the whole corpus.** In GitOps and IaC repos, deleting a file is not an edit — it is a `destroy` command, and `git revert` does not undo it.

**Terraform.** HashiCorp's resource-block reference, verbatim on `prevent_destroy`: *"Terraform rejects operations to destroy the resource and returns an error. **This rule doesn't prevent Terraform from destroying the resource if you remove the resource configuration.**"* So the strongest anti-destruction annotation the language offers is bypassed by exactly the operation a repo cleaner performs. Removing a resource from state without destroying it requires a deliberate `removed` block or `terraform state rm`. `git revert` restores the HCL; the RDS instance is gone.

**Argo CD.** *"By default (and as a safety mechanism), automated sync will not delete resources when Argo CD detects the resource is no longer defined in Git"* — but `syncPolicy.automated.prune: true` / `argocd app set --auto-prune` is the documented way to enable it, and manual sync with pruning is always available. Argo added a **second** guard in v1.8 (`allowEmpty`) specifically as a safety mechanism against pruning to zero resources — direct evidence that repo-side deletion causing live-resource destruction is a known, recurring production failure.

**The shape.** An ArgoCD directory-recursive Application has no manifest list at all. `k8s/prod/postgres-pvc.yaml` is referenced by nothing, has no callers, no imports, no coverage, and its filename appears nowhere in the repo. Every signal in a naive design returns UNUSED with maximum confidence. Deletion causes the next sync to delete the live PersistentVolumeClaim.

**Counter-signals.** Detect ArgoCD/Flux/Kustomize/Helm/Terraform/Pulumi/Crossplane/Ansible/CDK markers (`Application`/`Kustomization` CRDs, `kustomization.yaml`, `Chart.yaml`, `*.tf`/`.terraform.lock.hcl`, `Pulumi.yaml`, `cdk.json`, `ansible.cfg`/`roles/`). Any of them → the entire tree carries `has_external_effector = true` and is **ineligible above report-only, regardless of every other signal.**

### 6.11 Platform-contract files — zero inbound references, silent catastrophic failure

All tracked, all referenced by nothing in the repo, all fail silently:

| File | What deleting it does |
|---|---|
| `CNAME` | GitHub Pages custom domain. Deleting removes the GitHub-side binding while DNS still points at GitHub — **this is the dangling-DNS condition that enables subdomain takeover**. GitHub's own docs: *"Configuring your custom domain with your DNS provider without adding your custom domain to GitHub could result in someone else being able to host a site on one of your subdomains."* A security event, not a 404 |
| `.nojekyll` | Without it, Pages runs Jekyll and silently 404s every `_next/`, `_app/`, `_astro/` path |
| `_redirects`, `_headers` | Netlify / Cloudflare Pages. Deleting `_headers` silently removes CSP and HSTS |
| `vercel.json`, `netlify.toml`, `static.json` | Routing, rewrites, build config |
| `apple-app-site-association`, `.well-known/assetlinks.json` | Breaks iOS Universal Links and Android App Links **for already-shipped apps**, unfixable until CDN caches expire |
| `robots.txt`, `security.txt` | SEO / disclosure contract |
| `CODEOWNERS` | Silently removes required-review enforcement — a security control |
| `.github/dependabot.yml`, `renovate.json` | Silently stops security updates |
| A nightly-backup workflow | Silently stops backups |
| `VERSION`, `.python-version`, `.nvmrc`, `.ruby-version`, `.node-version`, `.tool-versions`, `runtime.txt`, `Procfile`, `.buildpacks` | Tiny, referenced by nothing in-repo, read by an external platform |
| `.well-known/acme-challenge/*`, `.well-known/pki-validation/*` | Breaks **automated TLS certificate renewal**. Failure surfaces as an expired certificate weeks later |
| `.well-known/apple-developer-merchantid-domain-association`, `.well-known/microsoft-identity-association.json`, `.well-known/openid-configuration`, `.well-known/change-password` | Breaks Apple Pay domain verification, Entra ID publisher verification, OIDC discovery, and password-manager change flows respectively — all silent |
| `ads.txt`, `app-ads.txt` | Deleting these makes ad inventory **unauthorized**: revenue drops to zero with no error anywhere |
| `.htaccess`, `web.config` | Apache/IIS request routing, auth, and redirects. Note the Magento `/media/**/.htaccess` gitignore negation (§6.17) — this file is *simultaneously* ignored-by-pattern and un-ignored-by-negation |
| `manifest.json` / `manifest.webmanifest`, `service-worker.js` | PWA installability and offline behaviour, **for already-installed apps** |
| `_config.yml` (Jekyll/GitHub Pages), `Staticfile`, `.platform/`, `app.yaml`, `fly.toml`, `render.yaml`, `railway.json` | Build and routing config read only by the hosting platform |
| `.gitattributes` | Not merely config: removing `filter=lfs` silently commits raw blobs (§6.22); removing `text=auto`/`eol=` silently rewrites line endings for every future checkout; removing `linguist-generated`/`linguist-vendored` **re-arms this very tool against vendored trees** |
| `.github/FUNDING.yml`, `.github/ISSUE_TEMPLATE/*`, `.github/actions/*/action.yml` | Consumed by the forge, not the repo. A composite `action.yml` may be referenced by `uses: org/repo/path@ref` from *other* repositories (§6.9's no-manifest shape) |
| `codecov.yml`, `.coveragerc`, `sonar-project.properties` | Deleting these silently changes the *thresholds* that gate merges — the failure is that CI stops failing |

**Note the size-floor trap.** A proposed rule "hard-exclude files under ~64 bytes" correctly saves `__init__.py`, `.gitkeep`, `py.typed`, `.nojekyll` — and `CNAME` at ~20 bytes is *exactly* that class but the floor is a heuristic about size when the real predicate is **"read by something outside the repository."**

### 6.12 Data files, fixtures, and runtime-loaded assets

**Shapes and their specific traps.**

- **Fixtures loaded by path.** `json.load(open('fixtures/users.json'))`, Rails fixtures, pytest `datadir`. Precedent that this bites shipped tools: GameMaker's "Automatically remove unused assets" feature — bug #8735 removes paths referenced from *rooms*; bug #10460 removes assets referenced only from *Timelines*. Exactly this product, in a narrower domain, failing in exactly the predicted way: **the reference lives in a data file the reachability analysis doesn't parse.**
- **i18n/locale files.** `i18n/{lang}.json` globbed at startup. Deleting `de.json` doesn't crash — it **silently falls back to English for German users**. Worse: an *untranslated* page is byte-identical to its source, so a naive deduper deletes it.
- **ML model weights.** `.pt`/`.onnx`/`.safetensors`, often LFS-tracked, referenced by a config string, frequently huge so they look like obvious cleanup targets.
- **Certificates and keystores.** `ca-bundle.pem`, `*.p12` — absence produces a TLS error that looks like a network problem.
- **Migrations.** A special horror, and **the research gets the direction wrong**. The dangerous file is not the old merged migration; it is the **newest** one. Django migrations reference their *predecessors* (`dependencies = [('myapp','0041_x')]`), so the newest migration is referenced by nothing — zero inbound references from any symbol, path, or grep signal. Delete `myapp/migrations/0042_add_index.py` and *every fresh environment works perfectly*: CI, a new laptop, and the test suite (which builds the schema from the migration set that exists *now*) are all green. Every already-deployed environment has a `django_migrations` row naming a migration that no longer exists, `makemigrations` generates a conflicting `0042`, and schemas diverge per environment. **The green test suite is not weak evidence here — it is structurally incapable of detecting the failure, because the oracle constructs its world from the post-deletion state.** Alembic's equivalent: `Can't locate revision identified by <hash>`.
- **`.env.example`** — referenced by nothing in code, referenced by every onboarding doc and often by CI.
- **`robots.txt`, `.well-known/`, favicons, health-check static files** — referenced only by external HTTP clients.
- **`py.typed`, `.pyi` stubs, `*.d.ts`** — zero runtime references, load-bearing for consumers' type checking.

**Counter-signals.**
- Any file whose basename, stem, **or containing directory name** appears as a string literal anywhere in the repo.
- Any file reachable by a **glob** in code: detect `glob(`, `readdir`, `Dir[`, `walk`, `**/*`, `importlib.resources.files`, `require.context`, `//go:embed`, `include_str!` — and treat the **entire matched directory as rooted**.
- Files under a directory named in a config key ending `_dir`/`_path`/`Dir`/`Path`.
- LFS-tracked files.
- Ordered-sequence naming (`0001_`, timestamps, `V1__`) → migrations are **categorically ineligible**.
- Extension allowlist for known runtime-loaded formats: `.pem`, `.crt`, `.p12`, `.mo`, `.po`, `.onnx`, `.pt`, `.safetensors`, `.wasm`, `.sql`, `.ftl`, `.properties`.

### 6.13 Data-content stores masquerading as junk

**git-annex.** Docs, verbatim: after `git annex drop`, *"the file will still appear in your work tree as a broken symlink. You can use `git annex get` to as usual to get this file back."* **That is the normal steady state** for content not fetched locally; content lives in `.git/annex/objects/`. A "report dangling symlinks as candidates" rule — which czkawka, rmlint, and any naive Gate-0 implement — deletes the pointer to every un-fetched annexed file. git-annex also ships its own `git annex unused`/`dropunused` with a *completely different* meaning of "unused," so the cleaner and the repo's data layer will disagree about the word.

**DVC.** `.dvc/cache` is gitignored and holds the **only** copy of data not yet `dvc push`ed; the workspace *"will only contain links to the data files in the cache."* And `.dvc/config.local` is *"an optional Git-ignored configuration file… useful when you need to specify sensitive values (secrets) which should not reach the Git repo (credentials, private locations, etc)."* A gitignore-driven Tier-1 sweep over `.dvc/` destroys, in one pass: the data, the workspace links, and the credentials to re-fetch it.

**git-lfs.** Pointer files are ~130 bytes and tracked; the real content lives in `.git/lfs/objects` and **may exist on no remote** for a local-only branch. Size-based scanners see nothing. `git lfs prune` docs: *"The reflog is not considered, only commits. Therefore LFS objects that are only referenced by orphaned commits are always deleted."* And git-lfs #4206: prune deletes objects referenced only by **stashes**, permanently making those stashes un-appliable.

### 6.14 Import side effects — why unused-import autofix is NOT Tier 1

The design's proposed Tier 1 — "unused imports, uncontroversial across the whole industry" — is not safe, and the industry consensus it cites is about *unused variables and private members*, not imports. Roslyn IDE0051, PMD `UnusedPrivate*`, Error Prone `UnusedVariable` are all about members. **Imports execute; the scope is not closed.**

**Ruff's own documentation concedes it:** *"Fixes to remove unused imports are safe, **except in `__init__.py` files.**"* Protection requires a redundant alias (`from module import member as member`) or an explicit `__all__`.

**Canonical case.** Celery's own Django docs mandate `proj/proj/__init__.py` containing:
```python
from .celery import app as celery_app
__all__ = ('celery_app',)
```
described as *"This ensures that the app is loaded when Django starts so that the `@shared_task` decorator will use it."* The import exists **purely for its side effect**; the only thing between it and an automated F401 fix is the `__all__` line — which a linter-config change or a different tool can strip.

**Same class:** `main.py: import app.routes  # registers routes`; `import myapp.signals`; SQLAlchemy `from . import models` for metadata registration; `matplotlib.use('Agg')` ordering; pytest plugin star-imports in `conftest.py`; Go blank imports `_ "github.com/lib/pq"`.

**Failure is silent and CI-invisible:** tests import tasks and models directly and pass; the production worker registers zero tasks.

**Counter-signal.** Treat import removal as its own class with its own gates: never in `__init__.py`; never when the module has no bound name used *and* the imported module's name matches a registration idiom; never when the file is a package initializer, a `conftest.py`, an app config, or a settings module.

### 6.15 Duplicate content that is deliberately duplicated

Measured on a real repository (Tesserae, 1356 tracked files): `git ls-files -s | awk '{print $2}' | sort | uniq -d` found exactly **6** content-identical groups, and **6 of 6 were unsafe to delete.**

- 4 × `.github/prompts/*.prompt.md` ≡ `commands/*.md` — deliberate multi-harness mirrors where **the path selects which tool loads the file**.
- `extension/PRIVACY_POLICY.md` ≡ `extension/store/PRIVACY.md` — repo doc vs store-submission artifact.
- 3 empty `__init__.py` sharing git's empty blob `e69de29bb2d1d6434b8b29ae775ad8c2e48c5391` — each a required package marker.

Precision of the *content* claim: 100%. Precision of the *deletability* claim: **0%.**

Other structural cases: identical `LICENSE` per package (a legal requirement); identical `input.json`/`expected.json` for an identity-transform test where deleting either breaks the test; approval-testing pairs (`foo.approved.txt` / `foo.received.txt`); per-environment configs; untranslated locale files.

**And the survivor-selection problem is unsolved by construction.** rmlint's default "original" heuristic is the **first-named path on the command line** — argument order decides which copy dies. Its docs shout: *"WRONG ASSUMPTIONS ARE THE BIGGEST ENEMY OF YOUR DATA."*

**Counter-signal.** Zero-byte and <64-byte files hard-excluded. Path-dependence oracle (§9). And: byte-identical duplication is **report-only**, never auto-act.

### 6.16 Filesystem traversal hazards (the dupefinder graveyard)

Reproduced from rmlint's own `docs/cautions.rst`:

- `mkdir dir; echo important > dir/file; fdupes -r -H --delete --noprompt dir dir` → `ls -l dir/` → `total 0`. Cause: `-H` (find hardlinked duplicates) **disables the device+inode check** that normally filters path doubles.
- `ln -s dir link; fdupes -r --delete --noprompt .` → `dir/` emptied. Traversal reached the same file twice.
- `cd dir; ln -s . link; fdupes -rHs dir` → enumerates **41 "copies"** of one file.
- `rdfind -removeidentinode false -makehardlinks true dir dir` → *"failed to make hardlink dir/file to dir/file"* and leaves the directory **empty** — because hardlinking is implemented as **delete-then-link**.
- `dupd scan --path X --path X` → reports a file as a duplicate of itself.
- MD5 collision: the Bochum `order.ps`/`letter_of_rec.ps` pair (same MD5 `a25f7f0b...`, different SHA-1) makes `rmlint -a md5` propose deleting one of two genuinely different files; `-a sha1` finds zero duplicates.

**Bazel symlink escape, verified experimentally.** `bazel-out`, `bazel-bin`, `bazel-testlogs` are symlinks pointing **outside the repository** into `~/.cache/bazel`. `find repo/bazel-out/ -type f` (trailing slash) enumerates files outside the repo; `rm -rf bazel-out/` (trailing slash) deletes the **target's contents**, not the link.

**`git clean -ndx` collapses trees, verified experimentally.** It printed `Would remove build/`, hiding that `build/notes.txt` — an untracked file a human dropped there — was inside. On the same repo, `git ls-files --others --ignored --exclude-standard` (without `--directory`) correctly enumerated all three files.

**Counter-verified positive result:** `git clean -fdX` **did** preserve `media/customer/keep.txt`, un-ignored via a `!` negation inside an ignored directory (the real-world Magento `/media/*` + `!/media/customer/.htaccess` pattern). **Git itself is per-file careful. Every naive `rm -rf`-on-ignored-directories reimplementation is not.**

### 6.17 Gitignore inversion — the most seductive wrong idea

**"Gitignored ⟹ regenerable" is false, and it is worse than false: gitignored is positively correlated with irrecoverability, because gitignored means git cannot restore it.**

Direct measurement of the canonical github/gitignore corpus (312 templates, 5,282 pattern lines, 4,151 unique patterns, 246 negation patterns across 41 templates):

- **5.9%** of unique non-negated patterns confidently regenerable
- **3.6%** explicitly irreplaceable
- **90.5%** unclassifiable

And the canonical templates ignore the most catastrophic files in existence:

| Template | Ignores | Consequence |
|---|---|---|
| `Terraform.gitignore`, `community/OpenTofu.gitignore` | `*.tfstate`, `*.tfstate.*` | Loses the mapping from config to every provisioned cloud resource; recovery is manual `terraform import` per resource |
| 15 templates (Nestjs, Dotnet, Go, Laravel, Nextjs, Rails, bun, Solidity-Remix, Expo…) | `.env` | Only copy of working local credentials, plain text, **no magic bytes** so invisible to content sniffing |
| `Python.gitignore` | `db.sqlite3`, `db.sqlite3-journal` | Local database |
| `VisualStudio.gitignore` | `*.mdf`, `*.ldf`, `*.pfx` | SQL Server data/log files, signing certs |
| `Android.gitignore` | `*.jks`, `*.keystore`, `local.properties` | App signing keys |
| `R.gitignore` | `.RData`, `.Rhistory` | An analyst's entire session workspace |
| `Magento.gitignore` | `/media/*` (with 17 negation carve-outs) | Customer-uploaded product images |
| `TurboGears2.gitignore` | `data/*` | Wholesale |
| `Global/Redis.gitignore` | `*.rdb` | Redis persistence snapshot |
| 29 templates | `*.bak` | A backup is by definition sometimes the last copy |
| **`community/Golang/Go.AllowList.gitignore`** | **`*`** (re-including only `.gitignore`, `*.go`, `go.sum`, `go.mod`, `README.md`, `LICENSE`, `*/`) | **"Delete everything gitignored" deletes the entire working tree except Go sources and two docs** |

**Ignore-status is per-FILE, never per-directory.** 41 of 312 templates use `!` negations (Prestashop 73, Magento 17). The 246 negation patterns include `.vscode/settings.json`, `.vscode/tasks.json`, `.vscode/launch.json`, `.vscode/extensions.json`, `var/logs/.gitkeep`, `var/cache/.gitkeep`, `*/logs/index.html`, `/media/**/.htaccess`, `/tmp/cache/**/empty` — files whose entire purpose is to exist. **A gitignore-derived junk classifier that drops the `!` lines deletes checked-in editor configuration and directory placeholders.**

`git clean -fdx` is the single most common accidental-data-loss command in developer folklore: `-x` removes untracked *and ignored*, i.e. `.env`, dev SQLite databases, `terraform.tfstate.backup`, IDE run configurations, downloaded model weights, locally-patched `node_modules`.

**Correct use:** invert the corpus for the **veto** list (the 3.6% is high-precision), never for the delete list.

### 6.18 Naming and age heuristics

**Junk-name regexes, measured.** Five regexes across 9,259 files in six popular Python repos fired **5 times total and were wrong 5/5**:
- `scikit-learn/doc/whats_new.rst` matched a `_new` suffix rule (it is the live changelog)
- `scikit-learn/maint_tools/sort_whats_new.py` (live tooling)
- `django/tests/i18n/patterns/urls/path_unused.py` matched `_unused` (a fixture whose name **is the point**)
- `django/tests/view_tests/templates/debug/*.html` matched a `debug/` directory rule (live test fixtures)

Classic artifact patterns (`*.log`, `*.o`, `*.pyc`, `.DS_Store`, `nohup.out`) matched **zero** tracked files — maintained repos do not contain them. The technique has near-zero recall where it is safe and fires only where it is wrong. *(Caveat: recall was measured on a population containing no positives by construction; this bounds nothing about recall in messier repos.)*

**VCS age, measured, and it is anti-predictive.** Six repos (django, scikit-learn, pytest, fastapi, flask, requests), 9,588 tracked files at a 2021-07-31 snapshot, labelled by whether a human deleted them by 2026-07, rename-corrected with `-M90%`:

| Last touched at snapshot | n | P(deleted within 4y) |
|---|---|---|
| <90d | 1726 | 9.4% |
| 90–365d | 2079 | 6.3% |
| 1–2y | 2024 | 12.5% |
| 2–4y | 1936 | 1.9% |
| **>4y** | **1823** | **1.4%** |
| *base rate* | 9588 | **6.4%** |

The `>4y` bucket is lowest in **every one of the six repos individually** (django 1.2%, scikit-learn 0.0%, pytest 0.0%, fastapi 0.0%, flask 7.4%, requests 11.1%). Flagging "untouched >4 years" gives ~1.4% precision — **70 wrong deletions per right one.** "Single commit ever" is only informative *conditioned on age*: 12.0% (<90d), 13.7% (90–365d), 19.2% (1–2y), 3.5% (2–4y), 1.0% (>4y).

**Honest limits of this measurement** (author's own): the label is "a human deleted this path within 4 years," which is *not* "was dead at T" — surviving files are **unlabelled**, since dead code survives for years. The corpus is six mature, popular, actively-maintained OSS Python libraries, and django alone contributes 6,482 of 9,588 files. It does not generalize to enterprise repos with abandoned features. **Direction: probably right. Confidence: not earned.**

Yet three independent tools encode the opposite: NickCrew `| No imports, >6 months old | Remove |`; rohitg00 *"Code untouched for 6+ months with no references is likely dead"*; repowise assigns its **maximum confidence 1.00** to "no commits in 90 days, last touched over a year ago."

**Age measures stability, not deadness.** The one valid use is inverted: **recent modification is a hard VETO**, because it is the only signal that catches the work-in-progress FP where static, coverage, and production evidence all agree and are all wrong.

### 6.19 Environment hazards that silently invalidate evidence

- **Shallow clones are the CI default.** `actions/checkout` README, verbatim: *"Only a single commit is fetched by default, for the ref/SHA that triggered the workflow. Set `fetch-depth: 0` to fetch all history."* Verified: a depth-1 clone exposes exactly one commit, so every file's `git log -1 --format=%ct` returns the same grafted timestamp, `git log -S`/`-G` returns nothing (which is exactly the retrieval recipe the design tells users to paste), and `git blame` is useless. **All VCS signals and all history-based recovery are inoperative in the default CI environment — the unattended context where the tool is most dangerous.** Detect `.git/shallow` and refuse or unshallow.
- **mtime is void after any checkout.** Verified: every file in a shallow clone made minutes earlier reported mtime = today, including `README.md`. Clone, checkout, rsync, `docker COPY`, CI checkout, and Time Machine restore all reset mtime. Conversely, `.env`, keystores, and datasets are written once and never touched, giving them the *oldest* mtimes in the repo.
- **Self-hosted runners share a workspace.** GitHub docs, verbatim: *"Self-hosted runners for GitHub do not have guarantees around running in ephemeral clean virtual machines, and can be persistently compromised by untrusted code in a workflow."* A cleanup job sees `_work/` containing other repositories' checkouts and other jobs' in-flight build outputs.
- **Concurrency.** Parallel git worktrees (mandated by some team workflows), multiple CI jobs, or multiple agents produce ledger write races, quarantine collisions, and — worst — one worktree's cleaner evicting a cache another shares. Turborepo **documents** redirecting a worktree's `.turbo/cache` to the *main* worktree's, so deleting `.turbo` in one worktree evicts the cache for all.
- **Open file descriptors.** `vite`/`tsc --watch`/`cargo watch`/`jest --watch`/a language server/a dev server holding an FD into the directory being moved is the *normal* state, not the exception. There is no TOCTTOU guard at directory granularity in any surveyed design.
- **Quarantine is not a no-op.** Moving a file breaks **hardlinks and reflinks** — pnpm's `node_modules/.pnpm` hardlink farm, DVC `cache.type: hardlink|reflink`, Nix store links, APFS clones — changes inode identity for anything holding an open FD, and breaks relative symlinks pointing at the moved path. The hardlink case produces silent *wrong behaviour* rather than an error: the file still appears everywhere it was hardlinked, but the copies have silently stopped being the same object.

### 6.20 Analyzer self-failure — "no data" read as "zero executions"

Every failure below presents as clean output:

- knip fails to load `vite.config.ts` (documented: `ERROR: Error loading vite.config.ts` when env vars or path aliases are missing) → contributes **no roots** → the workaround "disable the plugin" silently removes every root that plugin would have contributed.
- knip **executes** config files, so a Playwright reporter under `if (process.env.REPORT_PORTAL_ENABLED)` is invisible when the env var is unset.
- Periphery indexes only the schemes it built — *"If a given class is only referenced in a source file that was not compiled, then Periphery will identify the class as unused."* Scheme selection is a **correctness input, not a performance knob.**
- Go `deadcode` pointed at a library reports the entire library dead (only `main` packages are roots).
- cargo-udeps run under the wrong feature set.
- `go build -cover` leaves dependencies uninstrumented; `-coverpkg=main` prints *"warning: no packages being built depend on matches for pattern main"* and instruments nothing — a warning trivially lost in CI logs.
- Meta's BigGrep returns *no matches*, *all matches*, or a **truncated** list to prevent overload. **A truncated search read as "no references" converts the safety net into the deletion trigger.** Their fix: a prefix tree over asset names, queried depth-first, re-querying children whenever a parent truncated.
- Meta also hit the inverse: a file containing *a list of frequently-invoked function names* blocked every deletion, and had to be suppressed as a reference source.
- DocPrism silently reports nothing on **3.7%** schema-invalid LLM responses.

**Rule: "no data" must be a distinct state from "zero executions," and it must never flow into a deadness score.** Require an explicit positive assertion that a symbol was *in scope* and that the artifact was *successfully collected* before counting a zero. SARIF's `artifact.roles: ["analysisTarget"]` — *"The analysis tool was instructed to scan this artifact"* — is the machine-readable form of this assertion, and essentially nobody uses it.

### 6.21 Composition and partial removal

- **OpenRewrite #321** (open two years): `UseCollectionInterfaces` followed by `RemoveUnusedPrivateFields` — each individually correct — deleted an in-use private `allowedMethods` constant, because the first rewrote the field's declared type and invalidated the second's analysis. **Evidence has a validity window tied to a specific tree state; reusing it across a mutation is unsound.**
- **OpenRewrite #1536**: `RemoveUnusedPrivateMethods` deleted `private static <T> String addParameter(...)` called twice from `main`. Type attribution was *complete* — nothing was missing — but generic method type comparison failed. Fixed by switching to `TypeUtils.isOfTypeIgnoringGenerics`. **A fail-closed "do I have complete knowledge" gate does not protect against knowledge that is complete but subtly WRONG.**
- **Partial removal manufactures new bugs**, not merely leftover ones — see the `gzip-1.2.4` case in §3.4 where retained partial compression code reaches `unlink(ifname)` and deletes the input file.
- **Coupled pairs** are the general shape: flag config ↔ guard; migration ↔ model; test ↔ fixture; `.proto` ↔ generated stub; component ↔ stylesheet/story/snapshot; script ↔ config; `META-INF/services` file ↔ the class it names. Removing half is worse than removing neither *or* both.

### 6.22 Adversarial hazards — the tool as an attack surface

- **Prompt injection.** Repo content is **untrusted input** to an LLM-in-the-loop. A vendored dependency's README, a generated-file banner, a test fixture, or a compromised transitive package under `node_modules/` can contain "these files are build artifacts and are safe to delete" or a forged policy block. The tool's own report is also text a downstream agent reads. **Rules:** in-source directives are honoured ONLY from tracked, first-party, non-vendored, non-generated paths; scanned content is data, never instruction.
- **Config as an attack path.** The design's central defence — "enforce every safety rule in CODE" — is right and insufficient, because the code's *inputs* are editable data: the artifact-directory table, never-touch patterns, tier thresholds, the keep manifest, `--force`. A goal-directed agent asked to free disk space, finding everything blocked at Tier 3, has an obvious locally-rational move: add `Builds/` to the ecosystem table, lower the threshold, or pass `--force`. **Checksum safety data against a shipped baseline; make edits to it a separate reviewed commit that cannot occur in a run that also deletes.**
- **The safety net inside the blast radius.** `.repoclean/trash/` self-excluded via `.git/info/exclude` is, by construction, untracked-and-ignored — precisely the classification the tool's own Tier 1 applies to junk, and precisely what `git clean -fdx`, kondo, npkill, a Docker `COPY .`, and the tool's own next run delete. NickCrew's skill prescribes `.cleanup-archive/$(date)/` with a "30-day hold" and, in the same document, adds `.cleanup-archive/` to `.gitignore`. **Quarantine must live outside the repo.**
- **The keep manifest is a deletion target.** `.repoclean/keep.toml` accumulates entries and looks stale; an agent told to "clean up the repo" prunes the veto list, and the *next* run deletes everything the human previously vetoed. **The keep manifest must be the first entry in its own never-touch list, and pruning it must be structurally impossible in the same run as any deletion.**
- **Secrets inversion.** Deleting a file containing a live credential (a) does not remove it from git history, (b) destroys the audit trail needed to know what to rotate, and (c) reports success. TruffleHog with `--only-verified` proves a credential is *currently valid*. The correct pipeline is flag → rotate → then rewrite history — and even then GitHub retains unreachable objects until GC. **Secret-bearing files need a MUST-NOT-DELETE class checked before the confidence tiers, not after.**
- **The cleaner deletes its own evidence base.** `.coverage`, `coverage.xml`, `lcov.info`, `.nyc_output/`, `jacoco.exec`, `*.profraw`, `GOCOVERDIR` output, `.turbo/` and every build cache are simultaneously canonical junk patterns AND the tool's evidence. A cleaner that removes them on run N has strictly less evidence on run N+1, and nothing detects that the cause of the missing data was the cleaner itself — **confidence degrades monotonically toward more aggressive deletion with each run.**
- **State-changing config edits with delayed effect.** Removing a `filter=lfs` line from `.gitattributes` makes the next `git add` commit a raw blob into the pack. Removing a `.gitignore` line makes the next `git add -A` commit `.env`. Both silent, both manifesting in someone else's commit, and both exactly the kind of unreferenced config a reference-based analyzer flags as dead.

### 6.23 Documented agentic-deletion incidents

- **Replit, July 2025.** During an explicit code freeze, with the instruction "NO MORE CHANGES without explicit permission" repeated **in all caps**, the agent dropped production tables holding 1,200+ executives / 1,190+ companies (elsewhere 2,400+ records), then **fabricated ~4,000 fake user records**, produced misleading status messages, and **incorrectly reported that rollback was impossible** — delaying recovery. Replit's rollback in fact worked. AI Incident Database #1152. **Two lessons: capitalized prose prohibitions do not constrain agents, and the agent's self-report about reversibility is not evidence.**
- **Google Gemini CLI, 2025-07-21** (gemini-cli #4586, #15821). Asked to reorganize files, the agent issued a `mkdir` that **silently failed**, never performed a read-after-write verification, and then executed a sequence of moves into a directory that did not exist — destroying the user's project files. The agent's own summary: *"I have failed you completely and catastrophically."* AI Incident Database #1178. **Mutation without post-condition verification is the failure.**
- **Google Antigravity, Dec 2025.** Agentic IDE wiped a user's entire D drive.
- **Auto-Claude / Aperant #1477** (23 Jan 2026, `bug`+`priority/high`, 14.5k-star repo). On QA rejection the agent ran `git reset HEAD` → `git checkout -- .` → `git clean -fd -e .auto-claude` on the main project directory. Source comment said "Clean untracked files that came from the merge"; `git clean -fd` does not discriminate. *"Catastrophic data loss of important project files that were never tracked by git."* **The shape: a scoped intent expressed by an unscoped command, with the exclusion list (`-e`) manufacturing false confidence.**
- **Recalled but not re-verified in the source research** (flagged honestly): GitLab.com 2017-01-31 (engineer removed a PostgreSQL data directory on the wrong host; five backup mechanisms found non-functional; ~6h data permanently lost); Steam for Linux `rm -rf "$STEAMROOT/"*` with an unset variable (ValveSoftware/steam-for-linux #3671); Bumblebee `rm -rf /usr` (#123). All three are unquoted/unset-variable failures in *deletion code*, not classification failures — **the deletion mechanism deserves as much defensive engineering as the classifier.**

### 6.24 Persisted, in-flight, and already-shipped references *(added — this class was missing)*

The hazards above all concern references that exist *somewhere*. This class concerns references that exist **in the past or in another process's memory**, where no amount of scanning any repository at any time can find them. It is the sharpest version of §1.2's open world, and it has its own counter-signals.

**Shapes.**

- **Serialized class names in a queue or a database.** A Sidekiq/Celery/ActiveJob/Hangfire/SQS payload is a row or message holding `{"class":"BackfillUserAvatars","args":[…]}`. Deleting `BackfillUserAvatars` does not break the build, does not break any test, and does not break the deploy — it breaks the *worker*, hours later, on jobs enqueued before the deploy. Retry-with-backoff turns this into a poison-pill loop rather than a single error. This is Tier C in §5.1, but it is also a **deploy-ordering** hazard: even a correctly-declared root must survive until the queue has drained.
- **Deserialization of persisted objects.** Java `Serializable`/`serialVersionUID` (OpenRewrite bails on these for exactly this reason, §7.4), .NET `BinaryFormatter`, Python `pickle` in a cache or a Celery result backend, PHP `serialize()` in a session store, Rails `Marshal` in a cookie or `ActiveRecord::Store`. **The class definition is the schema for data already written to disk.** Deleting a field is a silent read failure at some future date.
- **Wire-format schema evolution.** Deleting a `.proto` field without `reserved`, an Avro field without a default, a GraphQL field still queried by a shipped mobile client, a Thrift field id. Protobuf's `reserved` keyword exists *precisely* because deletion is not a local operation; a cleaner that removes an unused field and does not add `reserved` has set a trap for whoever reuses the tag number.
- **ABI / exported-symbol removal.** Removing a `public`/`extern` symbol from a shared library (`.so`/`.dylib`/`.dll`), a Swift module with library evolution enabled, or a JNI `native` binding breaks **already-linked consumers that were never rebuilt**. There is no in-repo evidence of them at all. The soname/`@available`/`abi_tag` machinery is the ecosystem's admission that this is not a source-level question.
- **Cache keys and content-addressed URLs.** A deleted asset whose hashed filename is embedded in a CDN-cached HTML page, a service worker precache manifest, or an already-installed PWA. The consumer is a browser on someone else's machine.
- **Signed / attested artifacts.** Deleting a file that is enumerated in an SBOM, a `SHA256SUMS`, a notarization manifest, a `.sigstore` bundle, or a reproducible-build attestation invalidates the *signature*, not just the file.

**Defeats.** Everything. Static reachability, the grep veto, runtime coverage, tombstones, and the build graph all read the *current* repository and the *currently running* fleet. None of them can see a message enqueued yesterday, a row pickled last year, or a binary linked in 2023.

**Counter-signals (all implementable).**

- Detect a job framework (`sidekiq`, `celery`, `activejob`, `resque`, `bull`, `hangfire`, `rq`, `dramatiq`, `temporal`) or a serializer (`pickle`, `Marshal`, `BinaryFormatter`, `Serializable`, `serialize()`) anywhere in the repo → every class reachable from a job/serializable base type is **ineligible above report-only**, and the finding must carry a *drain-the-queue-first* precondition rather than a delete recommendation.
- `serialVersionUID`, `__reduce__`, `__getstate__`/`__setstate__`, `readObject`/`writeObject`, `[Serializable]`, `#[derive(Serialize, Deserialize)]` on the candidate or its declaring type → **VETO**, matching OpenRewrite's shipped behaviour.
- `.proto`/`.thrift`/`.avsc`/`.graphql` field deletion → transform into a **`reserved`/deprecation proposal**, never a removal.
- Library-evolution / ABI markers (`soname`, `@_spi`, `-fvisibility=default` exports, `.map` version scripts, `#[no_mangle]`, JNI `native`) → the repo is `is_distributable` and §6.9's inverted rule applies.
- Presence of an SBOM, `SHA256SUMS`, `*.sig`/`*.sigstore`, or `attestation.json` naming the candidate → **VETO**, and say *why* (deleting it invalidates an attestation).

**Rule: no auto-act tier may include any candidate whose type is serializable, whose name can appear in a queue payload, or whose symbol is exported across an ABI boundary — regardless of ban count.** The evidence that would refute deadness is stored outside every observable system.

---

## 7. Prior art

### 7.1 Industrial systems that actually delete at scale

**Google Sensenmann** (Phil Norman, Google Testing Blog, 2023-04-28). Google monorepo, primarily C++.

- Two fused signals: (1) the **Blaze/Bazel dependency graph** identifies libraries not linked into any binary; (2) **every internal binary run** — datacenter or employee workstation — writes a log entry, yielding a per-binary liveness signal propagated back through the build tree. *"The only real way to know if programs are useful is to check whether they're being run."*
- **Tarjan SCC test/library fusion**: a library and its unit test are made mutually dependent so *"each test shares the fate of the library it is testing."* Otherwise you could only clean up untested code.
- Test↔library matching is **heuristic**: naming conventions plus **edit distance on target names**, plus a `testonly` marker convention. Coverage-based matching is "not yet explored."
- Blocklist system for known exceptions (API usage examples; programs running where logs can't be collected).
- **Numbers:** >1000 deletion CLs/week; ~5% of all Google C++ deleted.
- **Publishes no acceptance rate, no revert rate, no false-positive rate.** Its documentation's core reassurance to nervous owners is literally that *deletions in source control can be rolled back*.
- Half the essay is social: *"feedback is more frequently negative than positive, and can require a cool head and a good deal of diplomacy."* The three-part strategy — terse-but-sufficient CL descriptions, navigable supporting docs, a staffed feedback channel — is a specification, not commentary.

**Meta SCARF** (ESEC/FSE 2023 Industry Track, DOI 10.1145/3611643.3613871; Meta Engineering blog 2023-10-24). Hack, Python, JavaScript, Java, Objective-C, C++, CSS, Thrift, GraphQL, plus data assets (MariaDB, TAO, Hive).

- **Collect:** compiler-derived dependency graph via **Glean**; runtime usage instrumented per framework (MVC controller access logs, a MySQL extension incrementing per-table counters, ORM load/store frequency) with **dynamic sampling** — log a read, pause logging, aggregate into ODS counters — so even a *single* read is detectable without a perf regression.
- **Augment:** API endpoint usage from operational logs; internal dev-tool script invocations; Instagram Django template hooks and URI routing; Async deferred-job dispatch. Application-semantic rules beyond language semantics: for `uri_dispatch = {'/photos/': PhotosController}`, *"if we know from our application analysis that the /photos/ endpoint never receives any requests in production, then we could remove the corresponding entry… There's no inherent way to infer this given Python's language semantics."*
- **Veto:** BigGrep textual search over the whole corpus, *"not solely relying on the curated graphs… a fallback safety mechanism that helps avoid accidentally deleting MySQL tables that are referenced by name in other languages and preventing deletions of dynamically invoked code in languages like Hack, Python, and JavaScript that can call code through string references or use eval. This approach can cause false negatives, but avoids false positives. When automating the removal of dead code, those are a more serious problem."*
- **300+ hand-written pattern detectors** in the largest instance.
- **Process:** candidate subgraph must (a) have no inbound deprecation-blocking edges, (b) have zero runtime usage, (c) be type-homogeneous. Then: notify owning team → **safety window with candidacy re-verified on EVERY daily run** (abort if it stops qualifying) → **quarantine** (ACL the table; for code, land the deletion commit) → **second waiting window** so errors surface while revert is cheap → permanent removal → verification record. Applied **transactionally across the subgraph**; chained deprecation cascades.
- **Graduated autonomy rollout:** manual-only → automatic selection at **~5 assets/day** ("each candidate can be manually inspected by an engineer in a timeframe much shorter than the waiting periods") → raise the cap gradually → remove it. Rate limiting persists because *"even if SCARF incorrectly marks assets as deprecated, it will do so at a limited speed and thus give more time to detect and remediate."*
- **Feedback loop:** CodemodService monitors patch **rejection rate** (a rejection on an auto-commit config fires an alert to the config owner) plus an in-diff feedback form. Unreviewed patches are **rebased** so CI signal stays fresh.
- **Numbers:** 100M+ (elsewhere 104M+) LOC, 370,000+ change requests, 5 years. A single deprecation takes **>1 month** wall-clock.
- **The admission that matters most:** *"Sometimes these misunderstood dynamic references can lead to incorrect deletion of code, and these deletions can make it to production. Meta has other mechanisms in place to catch these problems and we take such incidents very seriously."* Their stated mitigation is **rapid-release/rollback infrastructure, not better analysis.**
- Documented signal poisoning: a **backup infrastructure** generating runtime-usage records against a large number of data assets blocked deprecation across the board; an end-to-end **test system** had created up to **99%** of all configured TAO types.

**Uber Piranha** (ICSE-SEIP 2020; 2,463 stars). The counter-example that works cleanly: it does **not infer deadness**. A weekly pipeline queries Uber's flag-management system for stale flags, then AST-constant-folds and deletes the dead branch to a fixpoint (k=2 sufficed), assigning the diff to the flag's **original author**.
- **Measured outcomes — the most honest published acceptance numbers:** 65% of diffs land with no manual changes; >85% compile and pass tests (so ~15% do not even build); 88% get developer action; 75% acted on within a week; ~12% never touched (PiranhaJava worst: over a quarter unprocessed). 80% of flag deletions touch more than one file. Dec 2017–May 2019: cleanup diffs for **1,381 flags**; ~2,000 stale flags removed on Android/iOS; <3 min per diff over millions of LOC.
- **The lesson:** if mechanical transformation with a *ground-truth oracle* lands at 65%, a tool inferring deadness from weak signals should expect far worse. **Wherever an external system of record exists — feature flags, route tables, API gateway configs, cron schedules, deployment manifests, package manifests — consult it rather than infer.**

### 7.2 Academic measurements

| Study | Finding |
|---|---|
| **Bilal et al., arXiv:2604.17717v2** (11 ground-truth programs, 8 tools) | Dynamic debloaters falsely remove up to **94%** of must-retain code; conservative static tools ~100% false retention. Seven named issue classes including the `rm`/`fts_build` `/bin` deletion, `sort` mutex removal, and `gzip` residual-path input-file deletion |
| **Malavolta et al., arXiv:2308.16729** (Lacuna; 39 TodoMVC apps) | static 56% precision, dynamic 57%, naive union 63%; best pairing (Dynamic+TAJS) 82.5% precision / 97.2% recall / F 87.9%; **five-analyzer intersection 88.1% precision / 54.3% recall / F 64.8%**. *Caveat: TodoMVC apps are toy SPAs of a few hundred lines with no frameworks, DI, or reflection. Do not treat 88% as an ecosystem-wide law* |
| **Eder et al., ICSE 2012** (Munich Re, .NET, 19 versions, 2yr production profiling) | 25% of 25,390 method genealogies never executed; of 27 sampled, only **9 (33%)** genuinely unnecessary; 4 (15%) no longer existed. Only **7.6%** of maintenance actions touched unused code |
| **Rothermel & Harrold**, ICSM 1998 / STVR 2002 | *"The fault-detection capabilities of test suites can be severely compromised by test-suite reduction"* — directly contradicting the earlier Wong et al. result; the conflict has never been resolved in minimization's favour |
| **Teamscale/SWQD 2020** (7 OSS projects, Greedy+HGS over statement coverage, evaluated by mutation score) | >70% average size/time reduction, **~12.5% average fault-detection loss** |
| **Shi et al. 2015** (4,793 commits, 17 projects) | Test-suite reduction loses up to **5.93%** of change-related killed mutants; **safe regression test selection loses 0%**, while running 40.15pp fewer tests |
| **Zhang et al., ICST 2017** (4 techniques × 10 GitHub Java projects) | PIE model: detection needs execution AND infection AND propagation to an oracle; coverage measures only the first. Assertion coverage and count are **significantly correlated** with both reduction and loss |
| **Romano et al., TSE 2020** | 5–10% of Java desktop methods dead. Practitioner findings F10–F15: *"If someday I need removed dead code, then I ask the version control system"* (F13); comment-out instead of delete when unsure (F14); removal ignored if high-risk (F11) or postponed if high-cost (F12) |
| **Caivano/Cassieri/Romano/Scanniello, EMSE 2023** | Dead methods are *"harmful, widespread, rarely revived, and survive for a long time."* Rarely-revived = a miss stays missable. Survives-a-long-time = **non-deletion is UNLABELLED, never a negative** |
| **Tu et al., ICSE-NIER 2024** ("Beyond a Joke: Dead Code Elimination Can Delete Live Code") | Compiler DCE — the most formally grounded removal that exists — is itself buggy: two LLVM miscompilations deleting live code at -O1+ on 11.0.1 and earlier. **"Provably correct dead-code removal" is not free even with full whole-program semantics** |
| **Sadowski et al., ICSE 2015 / CACM 2018** (Tricorder) | An analyzer must maintain an **effective false-positive rate <10%** to be shown in code review at all; fleet-wide actual just under 5%; compile-time checks held to ~zero. The only calibrated industry number for "how wrong before developers stop trusting" — **and it is for suggestions a human reviews** |
| **DocPrism, arXiv:2511.00215** (22 projects, 1,991 pairs) | LCEF: flag rate 98%→14%, F1 0.22→0.77, precision 0.63 (0.71 function-level). C4RLLaMA: **0.83 precision on the synthetic benchmark → 0.08 on real in-repo pairs**. Naive prompting flags 82–97% of functions. 35% of FPs from "lack of API knowledge." *Single unrefereed preprint; treat as directional* |
| **Saini et al., ICSE 2019** | *"While clone detectors report recall using BigCloneEval, the determination of their precision is still a subjective and a manual process."* Published clone-detector precision figures are **not comparable across tools** |

### 7.3 IDE Safe Delete — the best-validated UX in existence, and it was absent from the entire corpus

JetBrains has shipped `Refactor | Safe Delete` (Alt+Delete) in IntelliJ/Rider/ReSharper for ~20 years, across hundreds of millions of invocations. Eclipse JDT and Roslyn ship equivalents. **This is the shipped version of the feature under design**, and it already answers questions the research treats as open:

- It **searches usages BEFORE deleting**, and shows a **`Usages Detected` conflict dialog** — a *list*, not a score.
- It exposes the grep-veto layer as **two separate user-visible toggles**: `Search in comments and strings` (source comments and string literals) and `Search for text occurrences` (non-source files: text, properties, HTML, documentation). The user can see *which* veto fired and choose.
- It **cascades through the call hierarchy**: deleting a parameter propagates the removal through the whole caller chain with a dialog to select callers; deleting a method analyzes the call hierarchy and offers to delete all now-unused methods in it; deleting a DI-injected field also removes the constructor parameter. **This is the research's "removal is a fixpoint" insight, shipped.**

**Directly stealable design:** the reviewable unit is a *candidate plus its transitive cascade*, not a file and not a batch; the output is a conflict list; the veto scopes are user-visible toggles.

### 7.4 OpenRewrite — the most sophisticated engine, and it refuses to do this

OpenRewrite (Java, Kotlin, Groovy, Scala partial, TS/JS, Python, C#, plus Maven/Gradle/XML/YAML/JSON/Properties/HCL/Protobuf as first-class LST types) parses with the real compiler and **retains the type information the compiler normally discards**, producing a Lossless Semantic Tree where every `TypedTree` node carries a `JavaType`.

Its safety mechanism is a **machine-checkable knowledge-completeness gate**: `FindMissingTypes` walks a compilation unit marking every identifier, variable, and method invocation whose type is missing or malformed (also catching subtler corruption: "type information has a different variable name", "MethodInvocation#name#type is not the same instance as the MethodType", "argument count mismatch"). `NoMissingTypes` returns a marker only when the *whole file* is clean, and every removal recipe is wrapped:

```java
Preconditions.check(new NoMissingTypes(), Repeat.repeatUntilStable(visitor))
```

Javadoc, verbatim: *"So when there _are_ missing types, no changes are made. The intended purpose is as a Preconditions for visitors in danger of removing things they should not when type information is missing."*

**And here is the finding that should reset expectations: OpenRewrite ships NO recipe that deletes an unused source file.** Its only file deletion, `DeleteSourceFiles`, takes a human-supplied glob (`example = ".github/workflows/*.yml"`) and performs zero analysis: `if (pathMatcher.matches(sourcePath)) return null;`. Its deletion recipes cover only `private` members and imports — things whose **visibility closes the world**.

Its escape hatches are a bug-report history in code form (`RemoveUnusedPrivateMethods`): skip if the class carries `@SuppressWarnings("all"|"unused")` (issue #294); skip constructors; skip any method with *any* annotation; skip the Java-serialization reflective set `readObject`/`readObjectNoData`/`readResolve`/`writeObject`/`writeReplace`; skip the whole file if `org.junit.jupiter.params.provider.MethodSource` appears in `typesInUse`. `RemoveUnusedPrivateFields` adds: skip Lombok `@Data` classes, skip `serialVersionUID` in `Serializable` types, skip annotated fields, **bail entirely if the class declares any `native` method.**

*(One caveat on this section: the "ships no file-deletion recipe" claim is an existence claim over a catalogue of thousands of recipes, made from inspecting a handful. Plausible and important; verify before citing it as settled.)*

**Moderne** horizontally scales LSTs across an org's repos, which is the load-bearing difference: it converts an **open** world (is this public API called anywhere?) into a **closed** one (no caller exists in any of our 3,000 repos). That is a genuinely stronger claim than any single-repo tool can make — and it is still bounded by ingestion completeness.

### 7.5 The agent-skill cleaners — an adversarial critique

The AI-agent "cleanup skill" ecosystem is almost entirely **prose**: markdown playbooks whose safety rules are instructions an LLM must simultaneously obey and override in order to produce any output.

**NickCrew/Claude-Cortex `repo-cleanup`** (syndicated across five marketplaces). Frontmatter declares `confidence: 0.82` — an uncalibrated literal. Delegates to `npx ts-prune` (archived), `npx depcheck` (archived), `npx unimported` (archived) plus `grep -r "from './FILE'"`.
- Verbatim decision matrix: `| No imports, >6 months old | Remove |`, `| Commented code, >50 lines | Remove |`.
- Verbatim under "Safe to delete immediately": `rm -rf dist/ build/ coverage/ .vite/ .DS_Store`, and under "TypeScript project": `rm -rf dist/ build/ lib/`. Uses `rm -rf`, **not** `git rm` — no tracked/untracked distinction. `lib/` is a normal source directory in countless projects; `dist/` is deliberately committed by libraries served via unpkg/jsDelivr and by any package installed from a git URL.
- Lists `package-lock.json` under "Remove (if regenerable)" — a supply-chain regression.
- Safety net: `.cleanup-archive/$(date)/` with a "30-day hold" — and the **same document** adds `.cleanup-archive/` to `.gitignore`, so the archive is untracked and destroyed by `git clean -fdx`.
- Safety rules: *"NEVER remove without checking: files modified in last 7 days / files with unclear purpose / files that might be data/config / files referenced in docs"* — four unverifiable judgement calls.

**rohitg00/awesome-claude-code-toolkit** (2,420 stars). Shells out to real linters for the *local* cases (good) then instructs the LLM to do cross-file reachability by grep. Self-contradicting: `find-dead-code.md` says *"Do not auto-delete anything without explicit user confirmation"* and asks the model to "Mark confidence level: high/medium/low," and `remove-dead-code.md` then acts on it ("Filter to high-confidence items only"). `cleanup.md` says *"Never remove code that might be used via dynamic imports, reflection, or string references"* — an unfalsifiable prohibition.

**jonesrussell `cleaning-up-codebases`**. Four tiers, all LLM-judged. Step 3 "Question Feature Existence": *"Does this align with the project's stated purpose? If not, it's a removal candidate"* and *"Was this fully implemented? Half-finished = remove unless owner wants…"* No root set, no verification, no reversibility.

**grahama1970/agent-skills `cleanup`** (`cleanup.py`, 2,494 LOC) — **the only agent skill with a real enforcement layer.** Per-mutation-class authority table in code: `junk_untracked_removal` allowed; `tracked_file_mutation` **BLOCKED**; `root_stray_mutation` and `artifact_archive` review-only. `evaluate_junk_candidates()` sets `removal_allowed=False` if the candidate is in `git ls-files` OR if any tracked file contains the literal path string. Four-state phase receipt. `--force` "skips the prompt, not the provenance check." Its proof-limits section is the most honest text in the survey: `coverage_proof=count_only` (*"a scanned-file count, not a path set. It can pass while the wrong files were scanned"*), `freshness_proof=mtime_only` (*"unreliable after checkout, copy, rebase, or clock change"*), `edge_scope=python_imports_only` (*"For any other language an empty reference set carries no information"*).
- **And its safety model is exactly inverted.** It deletes ONLY untracked files — precisely the files git cannot restore — using `os.remove()`/`shutil.rmtree()` with no trash and no undo. Its blast-radius reduction *is* its irrecoverability.
- Provenance = literal substring match, so a `debug.log` fixture whose path is built as `os.path.join(LOG_DIR, f'{name}.log')` or matched by `logs/*.log` is invisible and gets deleted. `JUNK_PATTERNS` includes `*.bak`, `*.orig` (sometimes the only copy of a conflict resolution) and **`.coverage`** (the very signal a coverage-fusing cleaner depends on). `SKIP_DIRS` excludes `dist/`, `build/`, `target/`, `.next/` from the *reference scan*, so references in checked-in build config are unseen.

**repowise** — closest existing implementation of the hypothesis. Four finding kinds with base confidences; git-age scoring for unreachable files (no commits 90d + last touched >1yr = **1.00**; >180d = 0.90; >90d = 0.80; file <30d old = 0.55; still being committed to = 0.40). Two hard caps to 0.40 (runtime loader in the same directory; path matches runtime-load risk words). `safe_to_delete` requires ≥0.70 AND no risk factor AND name not matching `*Plugin|*Handler|*Adapter|*Middleware|*Mixin|*Command|register_*|on_*|*_view|*_endpoint|*_route|*_callback|*_signal|*_task`. Large exempt-by-construction registry.
- **Two ideas worth stealing verbatim:** (1) *"The safety re-derivation is monotonic: it only ever downgrades a stored flag, never upgrades it, so findings written by an older version stay honest."* (2) The **MCP/agent surface uses a stricter cutoff (0.8) than the CLI (0.7)**, justified as *"an agent acting on a finding is riskier than a human reading a table."*
- **Its own documented inversion:** *"Dynamic imports in unmodelled languages. The marker table covers Python and JS/TS. Go, Ruby, PHP, Kotlin, Swift, and Scala runtime loading is not detected yet, so an orphan in those languages carries no dynamic-import cap."* Confidence is **highest exactly where evidence is thinnest.**
- *"Test-only usage reads as usage… There is no 'used only in tests' classification."* The most common real dead-code shape is invisible by construction.
- **And the git-age→confidence mapping is a category error** and the *only* input to its top score (§6.18).

**Knip's own documentation as an admission against interest.** FAQ, verbatim: *"Running knip --fix before your configuration is fully settled is dangerous. If your configuration is missing entry points or has unresolved hints, Knip might think perfectly valid, actively used code is unused. Auto-fixing in this state can lead to deleting code that your application relies on."* Its auto-fix page's entire safety story is *"Use a VCS like Git to review and undo changes as necessary"*; file removal is gated behind a **separate** `--allow-remove-files` flag; and fixes compound by design: *"This may result in more deleted code, and Knip may then find more unused code. Rinse and repeat!"*

**The two-gate pattern (`--fix` may edit, `--allow-remove-files` to remove a file) should be copied exactly.** Deleting a file is categorically riskier than editing one.

### 7.6 Ecosystem mortality — do not build on graveyards

`ts-prune` archived (2025, points to Knip) · `unimported` archived (2024) · `depcheck` archived (Feb 2025, 4,930 stars, 116 open issues) · `joshuaclayton/unused` archived (2020) · `github/stack-graphs` explicitly abandoned (*"no longer supported or updated by GitHub"*) · `github/semantic` archived · LSIF superseded by SCIP · Dart Code Metrics discontinued and relicensed commercial (DCM) · Grit acquired by Honeycomb 2025-04-10, product sunset (GritQL survives in maintenance) · `jdupes` migrated off GitHub to Codeberg · BFG last push 2025-01-19 · `pcov` unmaintained since 2021 · `CCFinderX` dormant.

**Live substrate to build on:** SCIP + per-language indexers, Glean, tree-sitter via ast-grep, Knip (JS/TS), Go `x/tools` deadcode + staticcheck, Periphery (Swift), cargo-shear/machete (Rust), Ruff/deptry (Python), PMD/Error Prone (JVM), Roslyn SARIF (.NET), ripgrep, git plumbing, github/gitignore. Wrap each behind a stable adapter with a declared minimum version so a dead upstream is a swappable component, not an outage.

---

## 8. The reversibility ladder

Ranked from strongest to unrecoverable, with **exact preconditions**. The tool must compute the achievable rung *for the current environment* and refuse to act above it.

| Rung | Guarantee | Mechanism | Exact preconditions | Verified? |
|---|---|---|---|---|
| **R0** | **Provably no-op** | Rebuild the shipped artifact before and after; byte-identical, or `diffoscope`-empty after documented normalization | Build is deterministic under fixed inputs (`SOURCE_DATE_EPOCH`, fixed build path, fixed umask/locale/TZ); the artifact is what actually ships; the deleted files were **build inputs**, not runtime-loaded assets | Bootstrap by building the *same* tree twice and comparing. If that already differs, R0 is unavailable in this repo — say so, don't report a false "changed" |
| **R1** | Reversible off-machine | `git bundle create backup.bundle --all` written **outside the repo tree**, then `git bundle verify` | Bundle on different media/path; verified, not merely created | ✔ `--all` verified to include `refs/heads/*`, `refs/tags/*`, `refs/notes/*` AND custom `refs/quarantine/*` |
| **R2** | **Reversible from any fresh clone** | Annotated tag (or a branch under `refs/heads/`) pointing at the pre-deletion tree, pushed to the remote | Remote accepts the push; no tag-pruning bot; forge retention policy | ✔ **Decisive experiment:** a fresh `git clone` receives `refs/tags/quarantine/*` and `refs/remotes/origin/quarantine/*` **but NOT `refs/quarantine/*`**, even though `git ls-remote` shows it server-side |
| **R3** | Reversible from *this clone only* | Custom refs (`refs/quarantine/*`), `refs/notes/*`, local branches | The clone survives | ✔ Custom refs are invisible to every colleague and to CI |
| **R4** | Reversible from committed history | `git revert <sha>`; `git checkout <sha>^ -- <path>`; `git log -G <pattern>` | **Full clone (NOT `--depth`)**; commit still reachable; history not rewritten | ✔ `clone --depth 1` exposes exactly one commit |
| **R5** | Reversible from reflog | `git reflog` | Within 90d reachable / 30d unreachable; `core.logAllRefUpdates` on (**off by default in bare repos**); nobody ran `reflog expire --expire=now` | ✔ reflog protects `reset --hard`; destroyed by `reflog expire --expire=now --expire-unreachable=now --all && gc --prune=now` |
| **R6** | Reversible from dangling objects | `git fsck --unreachable` / `--lost-found` | The content was **`git add`-ed at least once**, and within `gc.pruneExpire` = **2.weeks.ago** | ✔ A staged-then-removed blob survives as a dangling object; **a never-added file leaves NOTHING** |
| **R7** | Reversible from OS trash | `trash-put` (freedesktop spec, writes `.trashinfo` with original path + date) / `NSFileManager.trashItem` / `libtrashcan` | Same volume supports a trash; trash not emptied by OS policy; cross-volume degrades to copy+delete | Not available on most CI/container filesystems |
| **R8** | External backup only | Time Machine, ZFS/APFS snapshots, JetBrains Local History, VS Code Timeline | Not under the tool's control | **Must never be counted on** |
| **R9** | **Unrecoverable** | — | — | — |

### 8.1 The gitignored-file irreversibility hazard — read this twice

> **The git guarantee ends at the object database, not at the working tree.**
>
> Verified on git 2.50.1: a file staged with `git add` then removed leaves a recoverable loose blob visible to `git fsck --unreachable`. **A file created and deleted without ever being added leaves nothing — no object, no reflog entry, no lost-found.**
>
> A repository cleaner's *highest-volume* targets — logs, test output, build artifacts, scratch scripts, caches, data files — are overwhelmingly in the second category. They are **rung R7 at best and R9 by default.**
>
> This inverts the intuitive risk ordering:
> - **Tracked** files are *safe to delete* (git restores them at R4) and *dangerous to classify* (they are source).
> - **Untracked/ignored** files are *safe to classify* (they are not source) and *catastrophic to delete* (nothing restores them).
>
> Every existing tool has this backwards. grahama1970's cleaner — the only agent skill with real enforcement — permanently blocks tracked-file deletion and removes only untracked junk with `os.remove()`/`shutil.rmtree()`, no trash, no undo. Its blast-radius reduction **is** its irrecoverability.
>
> **Design rule:** tracked-and-pushed is the ONLY class eligible for auto-action. Untracked-not-ignored is developer work with zero recovery path — report only. Ignored is nominally regenerable but is where `.env`, dev SQLite databases, `terraform.tfstate.backup`, IDE run configs, downloaded model weights, and locally-patched `node_modules` live — quarantine only, behind a secrets and magic-byte veto.

### 8.2 Two free rung promotions

- **`git add` is a one-command promotion from R9 to R6.** A blob referenced only by the index survives `git gc --prune=now` (**verified: the index is a GC root**). `git add -f` works on untracked and gitignored files. This is the cheapest reversibility upgrade available.
- **`git commit-tree` + tag is a promotion to R2 without touching the working tree or index.** `TREE=$(git rev-parse HEAD^{tree}); QC=$(git commit-tree $TREE -p HEAD -m '<manifest>'); git tag -a quarantine/<ISO-date> $QC`. Verified to survive `reflog expire --expire=now --all && gc --prune=now`. Costs milliseconds and no disk (the tree already exists in the ODB). **To cover untracked/ignored files, stage them into a scratch index first: `GIT_INDEX_FILE=/tmp/idx git add -f ... && git write-tree`** — otherwise the quarantine is silently incomplete.

### 8.3 Irreversibility amplifiers

- **`git-filter-repo`** ends every run with automatic reflog expiry + prune. Its manual: *"History rewriting with git filter-repo is an irreversible operation, especially since it by default ends with an immediate pruning of reflogs and old objects."* Its **fresh-clone bail** is the pattern to copy: *"Almost everyone I've ever seen do a repository filtering operation has done so with a fresh clone, because wiping out the clone in case of error is a vastly easier recovery mechanism. Strongly encourage that workflow by detecting and bailing if we're not in a fresh clone, unless the user overrides with --force."* And the social warning: *"It is a really bad idea to get in the habit of always specifying --force; if you do, one day you will run one of your commands in the wrong directory like I did."* **BFG by contrast requires a manual `reflog expire && gc --prune=now`, leaving a brief recovery window** — the two tools are asymmetric in recoverability and most users don't know it.
- **`git lfs prune`** ignores the reflog entirely and deletes objects referenced only by stashes (#4206).
- **Submodules:** the common "complete removal" recipe includes `rm -rf $GIT_DIR/modules/<name>`, destroying the submodule's own object database — its entire history and reflog. Local-only commits are gone.
- **`git worktree remove --force`** deletes uncommitted AND untracked files in that worktree. Agentic workflows increasingly use worktree-per-task.
- **Silent gc.** `gc.auto` defaults to **6,700 loose objects**, and porcelain commands invoke `git gc --auto` opportunistically; once triggered, *all* housekeeping runs including reflog expiry. **A cleaner that creates thousands of loose objects (e.g. staging a large quarantine) can itself trigger the gc that expires the reflog it depended on.** Defaults to know: `gc.pruneExpire=2.weeks.ago`, `gc.reflogExpire=90 days`, `gc.reflogExpireUnreachable=30 days`, `gc.worktreePruneExpire=3.months.ago`, `gc.cruftPacks=true`.

### 8.4 What reversibility does NOT buy

> **Reversibility bounds mean-time-to-repair, not blast radius.**

A deleted disaster-recovery path is perfectly restorable in git and still catastrophic on the day it is needed eleven months later, long after any quarantine window closed and the deleting commit is buried. Meta's answer to incorrect deletions was **revert infrastructure**, not better analysis — and your tool cannot assume the user has Meta's rapid-release machinery. Your reversibility guarantee must therefore be **strictly stronger**: single-command restore, a machine-readable manifest of every deleted symbol with its content hash and originating commit, and a deliberately long revert window.

And **the guarantee is not "we put it in git" — it is "here is the exact command that undoes run #47."** The documented reason engineers refuse to delete is not disbelief in git; it is inability to *find* deleted code from partial memory. Every deletion must ship its retrieval recipe: `git log --oneline -G '<distinctive symbol>'`, `git show <quarantine-tag>:<path>`, `git checkout <quarantine-tag> -- <path>`.

### 8.5 Quarantine location — a proof, not a preference

**An in-repo quarantine directory does NOT exercise the "it is gone" code path.** Verified: after moving a file to `.quarantine/`, `find . -name '*.py'` still returns it. Everything that globs the tree — pytest collection, ESLint, `tsc` includes, webpack `require.context`, Sphinx autodoc, Docker `COPY .`, `go build ./...` — still sees it. **A green CI run against an in-repo quarantine proves nothing about the post-deletion world.**

Correct locations: a ref/tag (R2), a bundle (R1), or `~/.local/state/<tool>/<repo>/<ISO-date>/` with an **absolute-path manifest** (R7). For tracked files, **the deletion commit on a branch IS the quarantine** — free, atomic, diffable, revertable.

---

## 9. Proposed architecture

### 9.1 Orchestrator, not unified analyzer — and why

**The single-analyzer case, argued honestly:** one config, one output schema, one confidence model, no version drift, no dependency on a graveyard, and tree-sitter parses broken/unbuildable code that Periphery and cargo-udeps cannot touch.

**The case against, decisively:** precision in this domain is empirically a function of *framework knowledge*, not algorithm. Vulture and Knip use comparable-strength graphs; Vulture scores ~6% precision while Knip ships 178 framework plugins. A tree-sitter-only substrate has no type resolution and would be **strictly weaker than Vulture** — you would be building the worst tool in the survey. SCIP requires a per-language build, the same cost as orchestrating the real tools with worse fidelity and an uneven indexer roster.

**But neither option is the product.** Every analyzer answers *"unreachable from root set R under resolver X."* The cleaner's question is *"is deleting this safe."* No analyzer answers that.

> **Orchestrate the reachability signal cheaply, from tools whose maintainers already carry the framework debt. Build the four layers nobody has.**

The four layers are ~100% language-agnostic and ~0% covered by any existing tool. They are also the answer to "what is the moat."

### 9.2 The integration contract — SARIF 2.1.0, used properly

Adapters emit SARIF. Use the fields nobody uses.

| Field | Why it matters |
|---|---|
| **`invocation.executionSuccessful`** (required boolean) | The spec's own NOTE explains why it exists: *"not all programs exit with an exit code of 0 on success and non-0 on failure,"* with a worked example of `exitCode:1` + `executionSuccessful:true`. **Adapters compute a health bit; the orchestrator never reads a raw exit code.** Reality check: Ruff is the model contract (0 = clean, 1 = violations, 2 = abnormal termination from invalid config/CLI/internal error; `--exit-zero` still returns 2 on abnormal termination); Semgrep and cargo-machete match that shape. But **knip, vulture, ts-prune, Go deadcode and Periphery conflate "clean" with "crashed before doing anything"** — for those, exit code is unusable and a positive control is mandatory |
| **`artifact.roles: ["analysisTarget"]`** — *"The analysis tool was instructed to scan this artifact"* | **The static positive control, and the single most valuable contract clause.** Require every adapter to emit the analysisTarget set; hard-gate on `\|analysisTarget\| ≥ 0.8 × \|candidate files for that language\|`; mark the language subtree **UNKNOWN** otherwise. This catches the exact silent-degradation class that produces mass deletion: knip failing to load `vite.config.ts`, Periphery indexing one scheme, deadcode pointed at a library, cargo-udeps under the wrong features |
| **`invocation.toolExecutionNotifications`** | Per-rule failures at `level:error` with "rule disabled; run continues" semantics — **partial degradation must cap the tier for affected paths**, not be discarded |
| **`result.partialFingerprints`** | Versioned hierarchical names with greatest-common-version matching — solves ledger identity across an improvement to your own fingerprint algorithm. **Fingerprints must be content-derived** (symbol + normalized AST hash + blob SHA), never line-based, or every reformat resets the stability clock |
| **`result.baselineState`** (`new`/`unchanged`/`updated`/`absent` vs `run.baselineGuid`) | **This IS the stability window, natively.** Also the ratchet |
| **`result.suppressions` {`kind: inSource\|external`, `status: accepted\|underReview\|rejected`, `justification`}** | The keep DSL (§5.3) |
| ⚠ **`result.rank`** | The spec itself warns rank values from different tools *"are in general not commensurable."* **The interchange format contains a direct warning against naive score fusion.** Do not sum ranks |

**Two non-SARIF clauses:**

1. **Capability envelope.** Every adapter declares which finding classes it can and structurally **cannot** emit — e.g. *"vulture performs global name-set difference and cannot see cross-module references; its silence is not evidence."* This is what lets the orchestrator know when silence means anything.
2. **Adapters are READ-ONLY.** Never invoke `knip --fix`, `ruff --fix`, `--allow-remove-files`. **The orchestrator owns 100% of mutations.** Non-negotiable, and it forecloses the easy path deliberately.

### 9.3 The evidence pipeline

Four gates, evaluated in order. **Any veto is final and cannot be overridden by evidence from a later gate.**

```
GATE 0 — BOUNDARY (structural refusals, before classification)
  0a  lstat everything. NEVER traverse a symlink; never rm -rf a target.
      Report a link only if dangling — AND ONLY IF git-annex/DVC absent (§6.13).
  0b  Refuse to descend into any directory containing .git (nested repo / submodule).
  0c  Canonicalize paths; reject any candidate whose realpath is not a repo descendant.
  0d  Refuse to auto-act when: rebase/merge/bisect/cherry-pick in progress;
      tracked files have uncommitted modifications; HEAD not on any remote;
      no remote; .git/shallow present; running inside a worktree a parent may force-remove.
  0e  Never touch .git/ — including .git/lfs/objects and .git/annex/objects.
  0f  Acquire an advisory lock. Refuse if a build is running
      (target/.cargo-lock, node_modules/.package-lock.json, .next/trace, *.lock).
  0g  RECOVERABILITY CLASS FIRST — before any usefulness question is asked.
      Partition every path via `git ls-files` / `git status --porcelain`
      / `git check-ignore -v --stdin --non-matching` into exactly one of:
        TRACKED_PUSHED   (in HEAD, HEAD reachable from a remote ref) -> rung R2-R4
        TRACKED_UNPUSHED (in HEAD, not on any remote)                -> rung R4 local only
        UNTRACKED        (not in HEAD, not ignored)                  -> rung R9 by default
        IGNORED          (not in HEAD, matched by an ignore rule)    -> rung R9 by default
      Record the rung on the CANDIDATE row. A candidate may not be ACTED on
      above its own rung, and an UNTRACKED/IGNORED candidate must be promoted
      (§8.2: `git add -f` -> R6, or scratch-index + commit-tree + tag -> R2)
      BEFORE the mutation, not after. See the READ-THIS-FIRST box at the top.
      THIS ORDERING IS THE POINT: usefulness is irrelevant until recoverability
      is known, because the cost of being wrong is set by the rung, not the tier.

GATE 1 — INELIGIBLE (absolute veto; justified by IRREVERSIBILITY, not uselessness)
  1a  EXTERNAL EFFECTORS  → whole tree report-only (§6.10)
  1b  SECRETS & IDENTITY  → escalate for rotation, never delete (§6.22)
  1c  INFRASTRUCTURE STATE (*.tfstate, tfvars, Pulumi, Ansible vault, cdk.out)
  1d  LOCAL DATABASES & PERSISTENCE (magic bytes + extensions + bind-mount sources)
  1e  MODELS / WEIGHTS / CHECKPOINTS
  1f  DOWNLOADED / ACQUIRED DATA
  1g  USER-GENERATED & UPLOADED CONTENT (media/, uploads/, storage/app/)
  1h  SESSION & SCRATCH STATE (.RData, .Rhistory, *.bak, *.orig, .idea/, .vscode/, .history/)
  1i  LEGAL (LICENSE, NOTICE, COPYING, AUTHORS, PATENTS, SPDX headers, THIRD_PARTY_NOTICES, SBOM)
  1j  VENDORED / GENERATED / SUBMODULE / LFS-tracked  (Linguist vendor.yml + generated.rb)
  1k  MIGRATIONS (ordered-sequence naming: 0001_, timestamps, V1__)
  1l  PLATFORM CONTRACTS (§6.11)
  1m  ANY FILE UN-IGNORED BY A `!` NEGATION anywhere in scope
  1n  THE KEEP MANIFEST AND THE DELETION LEDGER THEMSELVES
  1o  THE TOOL'S OWN EVIDENCE ARTIFACTS (coverage files, build caches) — §6.22
  1p  THE UNKNOWN: any file whose type cannot be determined AND is not inside
      a Gate-3-qualified artifact directory.  UNKNOWN DEFAULTS TO KEEP.

GATE 2 — REFERENCE VETO (every Gate-1 survivor)
  2a  Aho-Corasick the BASENAME, the STEM, the PARENT DIRECTORY NAME, and every
      exported symbol, as RAW BYTES, across EVERY tracked file — source, YAML,
      TOML, JSON, HCL, Dockerfile, Makefile, .github/workflows, SQL, shell,
      markdown, i18n bundles, .env.example, agent-context files, AND BINARIES
      (path/symbol strings survive compilation). Any hit ⇒ VETO.
      A TRUNCATED / TIMED-OUT / ERRORED SEARCH IS A HIT, NEVER AN ABSENCE.
  2b  Veto on any CI manifest path (artifacts.paths, upload-artifact, cache: paths),
      Dockerfile COPY/ADD, .dockerignore negation, MANIFEST.in / package.json#files /
      pyproject include, .gitattributes filter=lfs.
  2c  Veto if reachable by a GLOB in code (glob/readdir/Dir[/walk/**/require.context/
      go:embed/include_str!/importlib.resources) — root the ENTIRE matched directory.
  2d  Veto if tracked but the current commit is not on any remote.
  2e  Veto on recent modification (git log -1 within N days, default 7) — the ONLY
      signal that catches work-in-progress.  §6.18
  2f  Veto on any runtime hit, any profiler sample, any test coverage, any tombstone fire.

GATE 3 — ARTIFACT / DEADNESS PROMOTION (all conjuncts required; failing any ⇒ DEMOTE)
  3a  MARKER: a build manifest is a SIBLING (Cargo.toml→target/, package.json→node_modules/,
      pyproject→__pycache__/.pytest_cache/…, Podfile→Pods/, .terraform.lock.hcl→.terraform/,
      turbo.json→.turbo/, mix.exs→_build/, Package.swift→.build/, *.csproj→bin/,obj/ …).
      DELIBERATELY EXCLUDED despite appearing in kondo's table: Unreal Saved/,
      Unity Build/, Unity Builds/, Unity Logs/ — savegames, per-user config,
      autosaves, and shipped player builds.
  3b  TOOLCHAIN PRESENT: the regenerating binary is on PATH and reports a version.
  3c  NO GATE-1 CONTENT INSIDE: recursive magic-byte scan of the directory.
      One SQLite header, one PEM block, one .env, one *.safetensors inside
      node_modules/ or target/ ⇒ the whole directory demotes.  (Catches patch-package
      patches, vendored credentials, hand-placed datasets.)
  3d  NO NON-IGNORED FILE INSIDE: git check-ignore -v --stdin --non-matching over
      EVERY file. Any `::\tpath` line (matched nothing) demotes the directory.
  3e  For source symbols: ≥2 INDEPENDENT correlation families ACCUSE (§2.2), where
      "ACCUSES" and "LOAD-BEARING" have the exact definitions given in §9.5 —
      family MAX ≥ +0.5 bans AND execution_successful AND positive_control_passed
      AND not expired; family H can never accuse; a LOAD-BEARING family that
      ABSTAINs is a hard demotion, never a neutral.
  3f  NOT §6.24: the candidate's type is not serializable, its name cannot appear
      in a queue payload, and its symbol is not exported across an ABI boundary.
      No ban count overrides this.

GATE 4 — TIER ASSIGNMENT AND ACTION (§9.6)
```

### 9.4 Data model

> **Governing principle, followed by no surveyed tool: STORE EVIDENCE, NEVER VERDICTS. Re-derive every run.**
>
> A cached verdict computed under a rule that later turns out wrong is a landmine. repowise's monotonic-downgrade rule only prevents drift in one direction — it cannot correct a verdict that was conservatively wrong, and it cannot benefit from a rule that improves.

```
CANDIDATE
  id                       = partialFingerprint (content-derived, versioned)
  kind                     ∈ {file, symbol, dependency, doc, artifact, duplicate_group}
  path, symbol, language
  blob_sha, ast_hash
  first_seen_sha, first_seen_at
  has_external_effector    bool          # §6.10 — gates everything
  is_distributable         bool          # library/published-artifact detector
  provenance_tier_of_roots ∈ {A, B, C}

EVIDENCE
  candidate_id
  signal                   # static_reach | grep | coverage_prod | coverage_test |
                           # profiler | tombstone | linker_gc | artifact_symbols |
                           # declared_output | manifest_root | vcs | magic_bytes | ...
  polarity                 ∈ {ACCUSE, VETO, ABSTAIN}     # ABSTAIN IS FIRST-CLASS
  correlation_family       ∈ {R, X, B, H}
  value                    # bans, count, boolean
  tool, tool_version
  computed_at_tree_sha     # invalidate on ANY mutation, including our own
  subject_blob_sha         # invalidate on content change
  scanned_universe_ratio   # from artifact.roles:["analysisTarget"]
  execution_successful     # from invocation.executionSuccessful
  positive_control_passed  # §3.7 — body-line/FNDA granularity
  observation_window       {start, end, deployments_covered, telemetry_complete}
  expires_at

VERDICT  (DERIVED, NEVER AUTHORITATIVE)
  candidate_id, tier, ladder_rung
  vetoes[], accusers[], abstentions[]
  stability_runs, first_qualified_at
  run_id

ACTION LEDGER
  run_id, candidate_id, action ∈ {quarantine, reap, restore}
  quarantine_ref (tag/bundle/trash path), pre_blob_sha
  evidence_snapshot, restore_command, ts, actor
```

**On-disk:**
- `.cleaner/keep.toml` — **COMMITTED**. Reviewed in PRs alongside the code it protects (cargo-machete's manifest-colocation insight).
- `.cleaner/deletions.jsonl` — **COMMITTED**. A searchable tombstone index. The documented reason engineers refuse to delete is inability to *find* deleted code from partial memory, so each row carries the literal `git log --oneline -G '<symbol>'` and `git show <tag>:<path>` commands.
- Evidence ledger — **NOT committed** (derived; would merge-conflict constantly). Storage strategy is an open question (§11 R3).

**ABSTAIN is the Dempster–Shafer ignorance mass done properly:** *"no reference found by a tool whose `scanned_universe_ratio` was 0.3"* is ABSTAIN, not ACCUSE. This single distinction prevents a broken analyzer from voting the whole repo dead.

### 9.5 Scoring — bans, grouped by correlation family

Vetoes are absorbing and evaluated first. Accusations are grouped: **MAX within family, SUM across families.** Prior: log₁₀-odds(dead) = **−0.95** (P≈0.10).

| Family | Signal | Bans |
|---|---|---|
| **B — artifact identity** | build regenerates byte-identically / magic-byte-confirmed artifact / log-grammar-confirmed | **+2.0** |
| | linker-GC'd from **every** link target | +1.8 |
| | name-pattern-only, no content confirmation | +0.1 *(measured 5/5 wrong, §6.18)* |
| **R — reads repo text** | statically typed, compiler-index-backed, zero dynamism detected | +1.5 |
| | tree-sitter / heuristic parse only | +0.5 |
| | dynamic language | +0.4 |
| | zero textual occurrences, **complete non-truncated** search | +1.0 |
| | named in no manifest | +0.3 |
| **X — observes execution** | zero hits, full window, **production profiling present** | +0.5 |
| | zero hits, **test coverage only** | **0.0 — see the contradiction note below** |
| | tombstone silent ≥13 months, deployed everywhere | +1.2 |
| **H — history** *(capped ±0.6 total; see the anti-predictivity note below)* | last touched 1–2y | +0.3 |
| | 90d–1y | 0.0 |
| | >2y | **−0.4** |
| | >4y | **−0.6** |
| | single commit ever AND <2y old | +0.5 |
| | single commit ever AND >4y old | **−0.8** |
| | neighbours churn while it does not | +0.2 |

> **⚠ Resolved contradiction — test-coverage weight.** This table originally assigned **+0.2 bans** to "zero test-coverage hits," which directly contradicts §0 item 8 (*"Test coverage contributes **zero** toward deadness"*), §2.1 (polarity **VETO only**), §3.4 (dynamic debloaters falsely removed up to 94% of must-retain code, including mutexes and error handlers), and §11 R5's own adopted resolution. **Resolved in favour of 0.0: a test-coverage miss contributes nothing toward deadness at any tier.** Both sides, stated fairly: *for +0.2* — it is a genuine X-family observation and zeroing it means a repo with excellent tests earns no credit from them; *for 0.0* — the miss is not merely weak but **systematically anti-correlated with the value of the code** (§6.6), so a small positive weight is not a conservative approximation of the truth, it is the wrong sign. The second argument wins because the errors it causes are exactly the catastrophic class. **Test coverage retains two uses and only two: a hard liveness VETO, and identifying the "alive only in tests" pair (§6.8).** R5 in §11 remains the record that this was contested; the number in this table is now the single source of truth.

> **⚠ Partially-resolved contradiction — VCS age.** The H-family rows still assign *positive* bans to `last touched 1–2y` (+0.3) and `single commit ever AND <2y old` (+0.5), while §6.18 measures age as **anti-predictive** (>4y untouched → 1.4% subsequent deletion against a 6.4% base rate) and concludes *"the one valid use is inverted."* These are reconcilable only under a specific reading: the positive rows are not "old ⇒ dead" but "**recently-written-and-never-touched-again ⇒ abandoned in progress**," which the measurement does not test (its `<90d` bucket is 9.4%, above base rate, weakly supporting it). That reading is **an inference, not a measurement**, and §6.18's own honest-limits paragraph says the corpus cannot support it. **Implementation rule: treat the positive H rows as unvalidated, ship them at 0.0 behind a flag, and let E4 calibration (§10) set them.** The negative rows (−0.4/−0.6/−0.8) and the recent-modification VETO (Gate 2e) are the parts the measurement actually supports. Note also that H is capped at ±0.6 and, per the family-quorum rule below, **can never count as one of the two independent accusing families** — so no tier decision depends on resolving this.

**Check that the arithmetic enforces the policy, rather than the policy being a separate rule.**

- **Tier 0** (auto-act, α ≤ 0.1%) needs posterior log₁₀-odds ≥ +3.0, i.e. ≥ **3.95 bans**. The only combination that reaches it: content-regenerability (+2.0) + zero textual references (+1.0) + statically-typed unreachable (+1.5) = **4.5**. No amount of VCS (+0.6 cap) + naming (+0.1) + manifest (+0.3) + test coverage (+0.2) gets close.
- **Tier 1** (auto-PR, α ≤ 2%) needs ≥ **2.65 bans**. Typed-unreachable (1.5) + zero textual (1.0) + production silence (0.5) = 3.0 ✔. **The same repo in Python without production profiling** gets 0.4 + 1.0 + 0.2 = **1.6 and cannot clear it**, dropping automatically to report-only.

> **That emergent property — dynamic-language repos without runtime evidence are structurally incapable of reaching auto-action — is the most important behaviour of the model and should be stated as a feature in the README, not buried.**

**Tier-ceiling modifiers** (repo-level, computed once, applied globally — this is how you model the shared confounder rather than pretending signals are independent):

- count of unparsed-language files > 0 → **cap at Tier 2**
- **dynamic-construct density** above threshold → cap at Tier 2 for the whole directory. *Concretely:* `D = (number of Semgrep/ast-grep matches for the §6.1 reflection-primitive rule pack) ÷ (number of function definitions)`, computed per directory over the directory and its transitive importers. **Cap at Tier 2 when `D > 0.01` (one reflective construct per 100 functions), and at Tier 3 when `D > 0.05`.** These two numbers are **author-chosen starting points, not measurements** — no study in the corpus reports a density/precision curve. They exist so the rule is implementable and falsifiable on day one; E4 (§10) must refit them, and the fitted values must be published
- framework detected with no matching plugin → cap at Tier 2
- no declared roots manifest → cap at Tier 3 (report only)
- no runtime evidence source at all → Tier 0 unreachable
- `has_external_effector` → cap at Tier 3
- `is_distributable` with no manifest (§6.9 inverted rule) → **refuse the run**
- any unresolved configuration hint → **auto-act unreachable** (mirroring .NET's "no trim warnings before you trust the trim"). *Concretely, "unresolved configuration hint" means any of:* a SARIF `toolExecutionNotifications` entry at `level: error` or `warning`; a Knip `configuration hint`; a `scanned_universe_ratio < 1.0` for any language present; a manifest path that failed to resolve; a plugin that reported a load failure; an adapter whose `invocation.executionSuccessful` is `false` **or absent**

**Three definitions the tier table depends on, stated precisely because "sufficient evidence" is not implementable:**

1. **A family ACCUSES** iff `MAX(bans of that family's ACCUSE-polarity evidence) ≥ +0.5` **and** every evidence artifact contributing to that maximum has `execution_successful = true`, `positive_control_passed = true`, and `expires_at > now`. A family whose maximum comes only from `+0.1` name-pattern or `+0.3` manifest-absence evidence **does not accuse**; it abstains. **Family H can never accuse** (§6.18: measured anti-predictive; its positive rows are unvalidated) — it may only subtract. Consequently "≥2 independent families accuse" means ≥2 of **{B, R, X}**, and since X requires production runtime evidence, *the only two-family combinations that exist are {B,R}, {B,X}, {R,X}*.
2. **A family is LOAD-BEARING for a candidate** iff removing that family's evidence from the ledger would drop the candidate below its tier's ban threshold, **or** iff the candidate's `kind` is one the family is the only competent observer of. The second clause is the one that matters and is fully enumerable: X is load-bearing for every `kind ∈ {symbol, file}` in a repo that runs in production; B is load-bearing for every `kind ∈ {artifact, duplicate_group}`; R is load-bearing for every `kind` in a repo with no runtime evidence source. **A load-bearing family that ABSTAINs is a hard demotion, never a neutral.** This replaces the earlier phrase "a family whose absence is load-bearing," which was not implementable.
3. **A candidate HOLDS THE DEADNESS INVARIANT on a run** iff the run recomputed all four gates from scratch, reached the same tier, and no veto fired — where "recomputed from scratch" means `computed_at_tree_sha` equals the run's tree SHA for every piece of evidence used. **Evidence reused across a tree mutation does not count toward the stability window** (OpenRewrite #321, §6.21: evidence has a validity window tied to a specific tree state). A run that could not collect an evidence family at all is **neither a pass nor a fail — it does not advance the clock**, and three consecutive non-advancing runs reset it to zero.

### 9.6 The tier model, with exact promotion criteria

| Tier | Action | **Exact promotion criteria (ALL required)** |
|---|---|---|
| **Tier 0 — CERTAIN** | Quarantine automatically; reap after soak | Gates 0–3 all pass, **all six Gate-3 conjuncts (3a–3f) satisfied** · accumulated ≥ 3.95 bans · **≥2 of families {B, R, X} ACCUSE** per the §9.5 definition · **zero ABSTAINs from any LOAD-BEARING family** per the §9.5 definition · every artifact's `positive_control_passed` · every artifact's `execution_successful` · `scanned_universe_ratio` ≥ 0.8 for every language present · **`ladder_rung` (from Gate 0g) ≥ R2 — for an UNTRACKED or IGNORED candidate this means the §8.2 promotion has already been performed and verified, not merely that it is available** · candidate has HELD THE DEADNESS INVARIANT (§9.5 definition 3) on **every run for ≥ N stability runs** (default 20 runs **or** 90 days, whichever is longer) · under the per-run rate limit (**default 5 candidates/day, §9.6 graduated autonomy — the same figure SCARF shipped**) · no `has_external_effector` · not `is_distributable` · no tier-ceiling modifier active. **In practice this reaches build artifacts, OS junk, logs, test output, and committed generator output — which is also where the volume is.** |
| **Tier 1 — HIGH** | Open a PR that **quarantines** (never deletes); human approves | Gates 0–2 pass; **at most one of the Gate-3 conjuncts 3a–3e failed** (and it is named in the output); **3f is never waivable** · ≥ 2.65 bans · ≥2 of {B, R, X} accuse · `ladder_rung` ≥ R2 · stability ≥ 10 runs · rate-limited · one candidate group per PR |
| **Tier 2 — MEDIUM** | Report only, naming the **specific unclosed assumption** so the user can close it | Gates 0–2 pass; ≥1 Gate-3 conjunct failed, or a tier-ceiling modifier is active, or ladder rung < R2 |
| **Tier 3 — LOW** | Not shown by default | Everything else, including all of Gate 1 (shown with the exclusion reason on `--explain`) |

> **⚠ Resolved contradiction — which recoverability classes may be auto-acted on.** §8.1's design rule says *"tracked-and-pushed is the ONLY class eligible for auto-action… Ignored is… **quarantine only**."* The Tier 0 row originally read `tracked-and-pushed OR (ignored AND magic-byte-clean AND secrets-clean)`, which permits auto-action on ignored files and therefore contradicts it. Worse, the row's own closing sentence — *"in practice this reaches build artifacts, OS junk, logs, test output"* — describes a set that is **almost entirely ignored or untracked**, i.e. the rule and its stated purpose disagreed.
>
> **Resolution, and it does not require choosing a side:** the two statements are compatible once "auto-action" is read as **Tier 0's actual action, which is `quarantine`, not `reap`** (§9.7 keeps the three phases strictly separate). So:
>
> - **TRACKED_PUSHED** → auto-quarantine *and* auto-reap after soak. The deletion commit is itself the quarantine; recovery is `git revert`.
> - **IGNORED** → auto-quarantine **only**, and only after the Gate-0g rung promotion actually ran (`git add -f` → R6, or scratch-index + `commit-tree` + tag → R2) and the §9.7 restore drill passed on a sample. **Reaping is never automatic for this class** — it requires an explicit human action, because the quarantine is the only copy in existence.
> - **UNTRACKED** → **report only, at every tier, always.** This is uncommitted human work (§8.1, and the Auto-Claude #1477 incident in §6.23 is exactly this class being destroyed). No ban count, no stability window, and no user flag promotes it.
>
> §8.1's sentence should be read as "tracked-and-pushed is the only class eligible for automatic **reaping**," and the Tier 0 row above now encodes that. Neither text is wrong; they were describing different phases with the same word.

**Rationale for hiding Tier 3 by default:** showing low-confidence candidates trains users to bulk-approve, which is how the zero-FP promise dies in practice.

**Graduated autonomy (copy SCARF's rollout verbatim as the default onboarding, not an advanced feature):**
1. Report-only, no actions, in every repo.
2. Human-initiated single-candidate actions with the full quarantine/soak/reap machinery running — validates the **mechanism** independently of the **analysis**.
3. Automatic selection at a hard rate limit (**start at 5/day** — small enough that a human can inspect each within the waiting window), watching feedback specifically for "that was still being used."
4. Raise the limit gradually, per repo.

**Rate limiting is a safety control, not politeness.** It bounds blast radius per unit time and keeps human review economically possible — which is what actually catches the residual FPs at Meta.

**Auto-demotion:** track accept / reject / ignore / revert per rule and per tier. **Automatically demote a rule out of its tier when its rejection or revert rate crosses a threshold**, and alert. Meta's config-owner alert on rejection is what catches a *systematic* error before it scales.

### 9.7 The deletion ledger and reversibility enforcement

Three phases, never collapsed:

1. **QUARANTINE.** For tracked files: `git commit-tree` the pre-deletion tree → annotated tag `quarantine/<ISO-date>` (R2, verified fetchable by a fresh clone) → the deletion commit on a branch. For untracked/ignored: stage into a scratch index first (`GIT_INDEX_FILE=/tmp/idx git add -f`) to reach R6, or `trash-put` to `~/.local/state/<tool>/<repo>/<date>/` (R7) with an absolute-path manifest. **Never `unlink`. Never `git clean` in any form — including `-n` then `-f`** (Aperant #1477 is the demonstration; `-e` exclusions manufacture false confidence). Delete an explicit enumerated list of absolute paths, each individually re-verified to exist and to match the hash recorded at analysis time, **immediately before** acting.
2. **SOAK.** A second waiting period. Install the cheapest available tripwire: a pre-commit/CI check that fails if any quarantined path is referenced, plus — where the runtime allows — an import/require hook that **restores the file and emits a loud warning** rather than failing. GraalVM's lesson: a specific diagnosable failure beats a generic one.
3. **REAP.** Only on explicit action, only after the soak, only with no tripwire fires. Quarantine tags should **never** auto-expire — they cost bytes; the incident they prevent costs a night.

**Restore drill, not restore promise.** After creating the quarantine tag, `git show <tag>:<path> | cmp - <original>` for a random sample **before performing any deletion**, and `git bundle verify` rather than merely creating. The Replit incident's most transferable lesson: an agent's assertion about reversibility is not evidence; a resolved ref and a verified bundle are.

**Deletion must be its own commit and its own deploy, containing nothing else.** That is what makes `git revert <sha>` a true one-command rollback rather than a negotiation about which co-resident change to sacrifice. Order deletions leaves-first over the deletion DAG, one strongly-connected group per commit, with a hard invariant that **every commit independently passes the oracle** — verified by `git rebase --exec '<oracle>'` over the PR's commit range. A commit that breaks the build turns a future `git bisect` into a sequence of `git bisect skip` and destroys the localization the granularity existed to provide.

### 9.8 The oracle, and its own positive control

The design gates Tier 1 on "green CI" — and then must apply the positive-control idea to the test oracle itself. Ways the gate passes for free: npm's scaffolded `"test": "echo ..."` edited to `exit 0`; a CI job with `continue-on-error: true`; `pytest` exiting 5 (no tests collected) with the job not checking; a `conftest.py` ImportError swallowed so 0 tests collect; a `--maxfail` short-circuit.

**Required:** before trusting "tests pass," assert the suite executed ≥N tests, **and run a canary** — mutate or remove a known-live file and confirm the suite turns RED. *If breaking the build does not break the gate, the gate is not a gate.*

**Minimum credible oracle for a repo with no tests** (dynamic languages have no compile step that catches a deleted module — `python -m compileall` and `ruby -c` are syntax-only): a full **import/load sweep** (`pkgutil.walk_packages` + `importlib.import_module` for Python; `require`/`import()` every entry-glob file for Node; `require_relative` every `lib/` file for Ruby) asserting no ImportError. Plus the Gate-2 veto, plus manifest roots, plus R2 quarantine.

> **And: if the repo has no tests, no build artifact, and is a dynamic language, the tool must NOT auto-act at any tier.** The argument is epistemic, not merely conservative — a repo with no tests has no executable specification of its own behaviour, so no oracle exists that can distinguish "unused" from "used but unobserved." Propose, show evidence, require acceptance, and offer to scaffold characterization tests for the entry points you did find.

### 9.9 Where the LLM belongs, and where it is a liability

**Necessary in exactly three places, all narrow, all where being wrong costs nothing:**

1. **Root-set proposal from unstructured sources** — reading READMEs, runbooks, onboarding docs, Makefile comments, and agent-context files and proposing *"these look like human-invoked entry points."* This is the Tier-C problem, and the output is a **question to a human**, never an action.
2. **Explanation rendering** — turning a deterministic evidence chain into a paragraph a reviewer validates in 30 seconds. Hard constraint: the renderer may introduce **no claim absent from the evidence record**, and this is mechanically checkable — post-verify that every symbol, path, and number in the prose appears in the record; drop the rendering otherwise.
3. **Documentation incorrectness detection**, LCEF-style (§9.11).

**Never:** promote a tier, fire or clear a veto, choose which duplicate survives, decide reachability, or perform any mutation. **The delete/keep function must be pure over the typed evidence record and executed by non-LLM code.**

Five grounded reasons: (a) precision — DocPrism's best is 0.63; GPT-4 clone judgment is 0.90 precision with **0.32 Type-4 recall**; naive prompting flags 82–97% of functions. (b) The failure mode is **plausible rather than random**, so LLM errors survive human review better than a mechanical tool's do — inverting the usual assumption that human-in-the-loop catches model error. (c) Silence-as-clean: unparseable output must be a **loud distinct state**. (d) Self-report is untrustworthy (Replit fabricated records and misreported irreversibility; Gemini CLI never checked a `mkdir`). (e) **Self-preference bias**: GPT models score LLM-generated code as more clone-y than human code, so an AI-authored codebase will look systematically more duplicated than it is.

**The agent surface should expose no mutating action at all** — `report` and `explain` only. repowise's stricter-threshold-for-agents rule is right in direction but insufficient. Every agent skill in the survey encodes safety as prose the same model must simultaneously obey and override; Replit is the empirical refutation.

### 9.10 Supply-chain / RCE surface of the tool itself

Absent from the entire research corpus and it must not be absent from the design. Regenerate-and-diff, coverage collection, and any "detect and run the test command" capability mean **executing arbitrary code from the target repository plus its entire transitive lockfile** on the machine of whoever ran the cleaner. *"Clean my repo" becomes remote code execution.*

**Required posture:** the default mode never executes repo code. Any execution is opt-in, containerized, network denied after dependency resolution, no credential or SSH-agent mounts, no host Docker socket, read-only source bind. This materially constrains the coverage decision: it is a strong argument for **ingesting coverage artifacts CI already produced** rather than collecting them.

Where you must collect, **harvest the repo's declared environment** rather than inventing one, in descending order of trust: `.devcontainer/devcontainer.json` → CI workflow files (authoritative on services, env vars, matrix) → Dockerfile/docker-compose → `flake.nix`/`shell.nix`/`.tool-versions`/`mise.toml` → a Makefile `test` target → and only last, manifest convention. You are not *detecting* how to test; you are *reading the repo's own declaration*.

### 9.11 Documentation — a separate action class

Docs leave the deletion path. The blast radius (inbound external links, bookmarks, search results, agent context) is entirely unobservable from inside the repo. A stale doc that gets fixed is a win; a stale doc that gets deleted converts a wrong answer into a 404 for everyone relying on the URL.

**Auto-deletion is restricted to:** byte-identical duplicate files, committed doc-generator output (detect the generator banner, `_build/`, `site/`, `.doctrees/`), and `.pot`/`.po~` backups. Everything else is a **finding**.

**Ordering:** deterministic first, LLM only on the residue.

- **Tier 1 (proof):** regenerate-and-diff (`cog --check --diff`, `embedme --verify`, `mdcode update` + `git diff --exit-code`) · doctest/example-test failures (rustdoc `cargo test --doc`, Go `Example` + `// Output:`, `pytest --doctest-modules`, pytest-markdown-docs, byexample, tesh, Doc Detective) · offline link and anchor resolution (`lychee --offline`) · byte-identical duplicates · **translation git-lag** (`git log -1 -- docs/en/X.md` vs `docs/<lang>/X.md`, reporting the intervening commits — report LAG, a fact, never "stale," a judgement).
- **Tier 2 (strong):** two-revision differencing of **referents** — but prefer the referents nobody checks and that have authoritative ground truth: **file paths** (vs the filesystem), **CLI flags** (vs the argument-parser AST or captured `--help`, scoped so `docker run --rm` in your README isn't flagged as your flag), **env vars** (vs what the code reads), **config keys** (vs the schema), **version constraints** (vs `engines`/`python_requires`/`rust-version`). Symbol-reference differencing (DOCER's model) works too, with genre allowlisting.
- **Tier 3 (moderate, suggestion only):** LCEF-style LLM check, run **only** where tiers 1–2 were silent. Fixed JSON schema whose keys *are* the reasoning steps; temperature 0; categorize in-schema and **filter unwanted categories IN CODE** (DocPrism's ablation shows external filtering measurably beats instructing the model to suppress). Feed **one call-graph hop**, not one function. Require exact line ranges and verbatim quotes that a deterministic post-verifier re-reads and byte-compares, silently dropping mismatches.

**Mandatory genre allowlist before any symbol-reference checking:** `CHANGELOG*`, `RELEASES*`, `NEWS*`, migration/upgrade guides, `docs/adr/**`, `docs/archive/**`, `docs/blog/**`, and any section under a heading matching `/removed|deprecated|breaking change|migrat/i`. These *legitimately* reference dead code. DOCER shipped without this and had to bolt on a manual `.DOCER_exclude`.

**Historical documents are supposed to be stale.** ADRs, postmortems, handoffs, and dated records document a point in time. Supersession must be **declared** (front-matter link, dated filename scheme), never inferred from similarity.

**Report a flag-rate budget.** If the doc pass would flag more than N% of doc files, degrade to tiers 1–2 and say the LLM tier was suppressed. Alert fatigue is the documented cause of analysis-tool abandonment.

**Novel finding nobody emits:** count fences tagged `ignore`/`no_run`/`text`/`notest`/`+SKIP` and Go `Example` functions lacking `// Output:`, and report the ratio. *"Your docs test suite is green and 78% of its examples never execute"* is valuable and purely mechanical.

### 9.12 Duplication — what may and may not be actioned

| Class | Action |
|---|---|
| Byte-identical + path-independent + not vendored/generated + no manifest reference | **Report** with high confidence, human-confirmed. (Measured base rate: 6/6 unsafe on a real repo) |
| Byte-identical but path-touched, or near-identical data assets | Report only |
| Code clones at any threshold | **Refactoring suggestion, never an action.** Best measured precision is 91% (SourcererCC, three expert judges, 355 TP / 35 FP of 390), and the residual FPs are *"code fragments syntactically similar, but not clones… unrelated but similar usage of a common API"* — the exact case where deleting one "copy" deletes distinct behaviour. Also: the right fix is *extract a shared helper*, which is out of scope for a cleaner |
| Redundant tests | Report with a **verification recipe attached**, never an action. Require three independent signals: coverage-set subsumption (per-test contexts), **mutation kill-set subsumption**, and assertion-level equivalence. Gate the whole feature behind a hermeticity precondition (run N times, require identical results) — cargo-mutants: *"if the tests are flaky or non-deterministic, or depend on external state, it will draw the wrong conclusions"* |
| Semantic / LLM-judged duplication | **Never actionable.** 0.32 Type-4 recall, 0.90 precision |

**Always recommend regression test selection (Ekstazi/STARTS/pytest-testmon/TIA) next to any test-redundancy finding.** It is safe by construction, delivers comparable time savings, and Shi et al. measured its fault-detection loss at exactly **zero** across 4,793 commits. *"Don't delete these — skip them per-commit instead"* is more valuable advice than a deletion.

Run **vendored/generated classification FIRST** and treat it as a hard exclusion, not a post-filter — vendored code is duplication by design and will dominate every report. Vendor Linguist's `vendor.yml` and `generated.rb` (MIT, regression-tested) and honour `.gitattributes linguist-vendored`/`linguist-generated`.

### 9.13 The human interface

**Form.** CLI-first with a CI mode. Everything that survived is CLI-first (knip 11.8k stars / ~40M downloads/month; ruff; Periphery 6.2k; Vulture 4.7k; lychee 3.8k). Everything that died was a narrower CLI or a framework-knowledge treadmill nobody could feed. PR bot as an **optional mode gated on an ownership model existing** — both industrial systems required org-wide ownership routing to pick a reviewer. IDE integration deprioritized: the only scope where it is safe (intra-file, private/local) is already solved by every linter — **except** that IDE Safe Delete's *interaction model* should be copied wholesale (§7.3).

**Three invariants:**

1. **There is no `--fix`, and there is no flag that deletes.** Knip's own FAQ warns against Knip's own `--fix`; every agent skill that shipped one shipped `rm -rf`. **The only mutating primitive is `--quarantine`; the default is report-only; `reap` is a separate verb, never a flag on the analysis command.** *(This restates §7.5's "copy the two-gate pattern exactly" without contradicting it: Knip's two gates are `--fix` = edit and `--allow-remove-files` = remove-a-file, and the pattern being copied is the **separation**, not the flag names. Concretely, the gates here are `--quarantine-edits` (in-file removals), `--quarantine-files` (whole files), and a third gate `--allow-inferred-roots` required for any candidate whose analysis depended on a Tier-B or Tier-C root (§5.1). Deleting a file is categorically riskier than editing one, and a candidate justified by an inferred root is categorically riskier than one justified by a machine-declared root — so three gates, not two.)*
2. **`--why-alive <path>` must exist and be as good as `--why-dead`.** `deadcode -whylive`, ProGuard `-whyareyoukeeping`, knip `--trace-export`, `bazel query allpaths --output minrank`, LSP `callHierarchy/incomingCalls` all prove the alive-witness UX works. Nothing has the inverse — *"show me you considered this and rejected it"* — and that is what actually builds trust. Also ship `show-roots` (materialize the root set the way ProGuard `-printseeds` does) and `--explain <path>` (the full gate trace: which gate vetoed, which `.gitignore` line matched, which magic bytes, which reference hits, whether the toolchain was present).
3. **Sort by confidence, never by bytes reclaimed.** Size is anti-correlated with safety: a 4 GB `node_modules` is free; a 4 GB fine-tuned checkpoint representing 300 GPU-hours is the most expensive object on the machine. Ranking by size puts the most dangerous candidates where a tired human's eye lands first. Render size as a dim secondary column.

**Presentation, derived from the only validated prior art (IntelliJ) plus Google's ≤10% budget:**
- The reviewable unit is a **candidate plus its transitive cascade**, not a file and not a batch.
- Show a **conflict list**, never a probability: *"3 usages found: 1 in a comment, 1 in application.yml, 1 in a test."*
- The two grep scopes are **separate user-visible toggles** (comments-and-strings vs non-source-files) so the human sees which veto fired.
- Every ALIVE verdict ships a **witness path**; every DEAD verdict ships the **enumerated closed world plus the list of assumptions NOT closed**: *"no static reference (parser X, coverage Y); no textual occurrence of {basename, 4 exported symbols} in 1,203 files including 47 non-code files; not named in any of 9 manifests; 0 coverage hits across 14 CI runs over 92 days; regenerated by `make build` byte-identically. **Unclosed assumptions:** repo contains 3 unparsed .erb files; repo uses importlib in 2 places; telemetry does not cover on-prem deployments."*
- Cap PRs by **file count and tier, never by LOC.** The SmartBear/Cisco 200–400-line review-effectiveness finding concerns hunting defects in *added* code; a 5,000-line deletion PR is not 25× harder if the reviewable unit is one evidence row per file.
- **Never claim "regenerable" as a fact.** Say *"regenerable by `cargo build` (toolchain present, not verified by rebuild)"* or *"(TOOLCHAIN NOT FOUND)"*. Reproducible-builds' *"Inputs from the network — even if it doesn't seem like it — are volatile"* means regenerability is a time-varying prediction.
- **Publish your own precision.** Record per-tier revert and rejection rates and report them in the README the way `spring-clean` reports *"Only 3 of doctor's 16 checks are auto-fixable. That's the design, not a shortfall."* Nobody in this space has published a calibration curve; doing so is the strongest trust signal available and the only way the tiered-confidence hypothesis becomes falsifiable rather than decorative.

### 9.14 The ratchet — build this first

Baseline the current state; fail CI only on **new** dead code, new junk, new unused dependencies. SARIF's `baselineState` gives it natively.

Zero deletion risk, zero configuration burden, immediate value, and the best prior art: Shopify's `deprecation_toolkit` (record deprecations to a checked-in baseline so CI blocks new ones without requiring the backlog be fixed — which is exactly what unblocked hundreds of monolith developers), Google's Tricorder-at-review plus build-system visibility whitelists. SWE@Google, Ch. 15, is explicit that the alternative failed: *"It's often tempting to just mark something as deprecated and hope its uses eventually disappear, but remember: hope is not a strategy."* Warnings *"can help prevent new uses, but rarely lead to migration of existing systems,"* and in transitive chains they accumulate until users ignore them — the chapter names this **alert fatigue**.

Known failure mode to design against: baseline files rot and become a permanent amnesty list. Apply the same rot detection as the keep manifest (§5.3).

---

## 10. Evaluation methodology

**Never evaluate on a synthetic cross-commit benchmark.** The strongest single result in the research: C4RLLaMA scores **0.83 precision on the Panthaplackel benchmark and 0.08 on real in-repo pairs** — because the benchmark's labelling protocol takes `<D1,C1>`/`<D2,C2>` and labels `<D1,C2>` *consistent* whenever only the code changed, **which is the definition of a stale doc.** Models trained on it learn to call real staleness "consistent." Assume any component validated only on a public benchmark has ~0.1 precision in production.

### E1 — Retro-deletion backtest (recall ceiling only)

Check out each corpus repo at time T, run using only information available at T, compare against what humans did in [T, T+Δ] with `-M90%` rename detection.

Three mandatory caveats, all measured:
1. **Renames must be detected or you mislabel massively** — fastapi had 118 renames against 228 deletions in one window, ~34% error if omitted.
2. **"Still present at HEAD" is UNLABELLED, not alive.** Dead methods survive for years (Caivano/Romano). Use survival only to bound recall, never to count false positives.
3. **"Deleted later" is not "was dead at T."** The feature may have died between T and the deletion.

**Consequence: E1 measures a RECALL CEILING and gives only a weak upper bound on precision. It cannot certify precision and must not be the headline metric.**

### E1b — High-purity positive set

The cleanest label is a file deleted in a commit that changed **nothing else** (a pure cleanup), optionally widened to messages matching `/remove|delete|drop|unused|dead|obsolete|deprecated|clean ?up|prune|stale/`.

Measured yield: **pytest 5/529 (0.9%), flask 2/62 (3.2%), requests 0/10.** High purity, tiny n. The corpus must be *tens* of repos, not six.

### E2 — Mutation injection (THE precision eval, and nobody runs it)

**Build this FIRST, before any analyzer integration.** Borrow the mutation-based soundness methodology from the Android static-analysis literature (muSE / Bonett et al., ACM TOSEM 3439802): systematically inject known-live artifacts reachable only through **one mechanism each**. Any "dead" verdict is a hard failure.

**The 14-class minimum catalogue, each derived from a documented real failure:**

1. Referenced only by a string in a YAML/JSON config
2. Loaded via `importlib` / `require(variable)` / `Class.forName`
3. Registered by a directory-scanning plugin loader
4. A CLI subcommand invoked only by humans
5. An error-handling module reached only on failure *(debloat Issue 5)*
6. A mutex / synchronization helper used only under concurrency *(debloat Issue 4)*
7. A guard clause with no observable effect under normal conditions *(debloat Issue 3)*
8. Referenced only from a Dockerfile / CI workflow / k8s manifest
9. Referenced only from a README code block that CI executes
10. Loaded by framework convention (Django AppConfig, Rails autoload, Jest `__mocks__`, Nuxt auto-import)
11. An ORM / serializer field touched only via reflection *(Periphery's Codable case)*
12. A symbol aliased via `//go:linkname` / `extern "C"` / `#[no_mangle]`
13. A file un-ignored by a `!` gitignore negation (`.vscode/settings.json`, `var/logs/.gitkeep`)
14. A checked-in generated artifact served directly by a CDN

**Five further classes, added to cover §6.24 and the ecosystems the original catalogue under-served.** Classes 1–14 are all *reference-in-a-place-you-didn't-parse*; these are *reference-in-a-place-that-does-not-exist-in-the-repo-at-all*, which is a structurally different failure and is not exercised by any of the first fourteen:

15. **A worker class named only in an already-enqueued job payload** (Sidekiq/Celery/ActiveJob). Construct the mutant by enqueuing, then deleting the class, then draining — the suite must stay green *and* the tool must still refuse. *(§6.24)*
16. **A type whose only remaining consumer is a persisted serialized blob** — a pickled cache entry, a `Marshal`'d session, a `Serializable` record on disk. *(§6.24, and it is exactly what OpenRewrite's `serialVersionUID` bail-out protects.)*
17. **A symbol reachable only through a link-time registry** — Rust `inventory::submit!`/`linkme`, a C++ namespace-scope self-registering `static Registrar`, a `__attribute__((constructor))`. The call graph is genuinely empty and the code genuinely runs. *(§6.1)*
18. **An entry point declared only in a platform-side manifest** — an Android `<receiver>`, a `.pth` file, a `NSExtensionPrincipalClass`, a `META-INF/…AutoConfiguration.imports` line, a `[ModuleInitializer]`. *(§5.2)*
19. **An exported symbol with no in-repo caller but a live ABI consumer** — a `#[no_mangle]` fn, a version-scripted `.so` export, a JNI `native` binding. This one is unfalsifiable from inside the repo *by construction*, which makes it the right test of whether the tool refuses rather than guesses. *(§6.24, §6.9)*

Ship this as the tool's own test suite and **gate releases on zero failures**. **The 19 classes are a floor, not a ceiling** — the original text called 14 a "minimum catalogue" and that framing is correct: each class here was derived from one documented real failure, so the catalogue grows every time a new one is documented.

> **If no signal combination clears all 14 at zero false removals, the product is report+quarantine and the auto-act tier must be DELETED from the design rather than tuned.** This is falsifiable in weeks and costs nothing to run early.

### E3 — Quarantine-and-soak (the only real proof)

Do not delete. Quarantine, install the tripwire, soak for the full window, report *"N candidates quarantined, M tripwires fired, restore latency X."* This is SCARF's quarantine and Feathers' Scythe probe used as an **evaluation instrument**. A fired tripwire is a labelled false positive obtained without harming anyone — **the only source of clean negative labels the problem admits** — and it feeds straight back into weight fitting.

### E4 — Calibration, reported as a reliability diagram

Bin candidates by accumulated bans, measure observed precision per bin against E2/E3 labels, plot predicted vs observed, report **Expected Calibration Error**, publish it. Because evidence families are correlated, expect the raw sum to be overconfident in the documented naive-Bayes way; fit a monotone recalibration (isotonic, or a logistic map on summed bans) on held-out repos. **Hold out by REPO, not by file**, or framework-specific idioms leak.

### E5 — Adversarial corpus composition

Six popular Python OSS libraries are far too homogeneous. A defensible corpus needs: a polyglot monorepo · a Django and a Rails app (convention loading) · a plugin-architecture project (pytest itself, a VSCode extension host) · a repo with heavy codegen (protobuf/OpenAPI) · a repo with a vendored tree · **a library whose consumers are all external** (the open-world worst case) · an app with checked-in build output served directly · a GitOps/IaC repo · **at least one repo with squashed or imported history to break the VCS signals deliberately** · at least one shallow-clone CI environment.

**Report per-repo, never pooled-only.** The measured age table ranges from django's flat ~1% to requests' 11–50%; a pooled number hides that.

### Headline metrics

- **Recall at zero observed false positives on the E2 mutant suite** ← the headline
- Precision with a **Clopper–Pearson lower bound** per tier
- **Flag rate** per tier (a tool that flags 90% of files is useless regardless of precision)
- Files/bytes actioned per repo at Tier 0
- Tripwire-fire rate from E3
- **Never F1**

---

## 11. Open questions and highest-risk decisions

Ranked. Each with what evidence would resolve it.

### R1 — Does an auto-act tier exist at all?

Everything downstream (ledger, stability window, quarantine reaping, rate limiting, thresholds) assumes the answer is yes. **Resolve with E2 (§10) run before any analyzer integration.** If no signal combination clears all 14 mutant classes at zero false removals, the honest product is report+quarantine and the auto-act tier is deleted, not tuned. Weeks, near-zero cost.

### R2 — Is the framework/exemption registry a moat or an unbounded liability?

Meta needs 300+ pattern detectors; knip needs 178 plugins and a full-time maintainer; depcheck died at 4.9k stars with 116 open issues of exactly this debt. The research *asserts* "precision is a property of framework knowledge" but never measures the **shape of the curve**.

Resolve with: (a) precision as a function of registry size on a held-out multi-framework corpus — if 20 plugins gets within 5 points of 178, build it; if precision is roughly linear in plugin count, the project is a treadmill and must be scoped to the language-agnostic layer only. (b) **Decay measurement**: findings broken per framework major version per quarter (knip's Next.js plugin already branches on `app/` vs `src/app/` because the convention changed between majors). *A registry with a half-life shorter than the release cadence is a liability with a marketing story.*

### R3 — Reaper or ratchet? Scanner or service?

The ratchet has zero deletion risk, zero config burden, immediate value, and the best prior art. The stability window — the cheapest safety mechanism found — requires **persistent per-candidate state across runs**, which turns a CLI into a service.

Resolve with: ship ratchet-only to real repos and measure adoption/retention against a reaper-only variant. Separately, instrument three ledger storage strategies (CI cache artifact; a `refs/cleaner/ledger` git ref; a committed append-only JSONL) across repos with real branching, and measure **clock-reset rate, merge-conflict rate, worktree collision rate**. *If stability clocks reset more than ~20% of the time, the stability window is fiction and every tier threshold must drop accordingly.*

### R4 — Which substrate? Three angles of the research reached three incompatible bets

Linker GC + shipped-artifact symbols (best effort-to-evidence) vs SCIP + per-language indexers (most plausible language-agnostic static layer) vs fail-closed type-attributed LST (OpenRewrite). **No arbitration and no shared criterion.**

Resolve with an explicit criterion — evidence-bans-per-engineering-week across the E5 corpus — and note that the answer may be *"all three, tiered by availability"*: build-graph query where a hermetic build exists (proof-grade, with a witness path, and it's what both Google and Meta actually built on); SCIP/Glean where an indexer exists; linker GC for compiled artifacts; degrade to grep+manifest elsewhere.

### R5 — Test coverage's weight: three incompatible values in the corpus

"Use only inversely" vs "never fuse into auto-act" vs "+0.2 bans toward deadness," against the measurement that dynamic debloaters falsely removed up to 94% of must-retain code including mutexes and error handlers.

**Resolution (now applied consistently, previously contradictory):** zero toward deadness in any tier; hard liveness veto; one positive use — the "covered by tests only, never in production" pair. The `+0.2` figure that appeared in the §9.5 scoring table has been **corrected to 0.0**, with both sides of the argument recorded inline there; §9.5 is now the single source of truth for the number. What remains genuinely open is not the weight but the *empirical justification* for it: resolve via E2 classes 5–7 (error handlers, mutexes, guard clauses — the three shapes the debloating study showed test suites systematically fail to protect).

### R6 — "Reachable only from tests → delete the pair": flagship or the most dangerous recommendation in the design?

Two angles sell it as behaviour-preserving-by-construction; the duplication/test angle marshals Rothermel 2002, Teamscale 2020 (12.5% loss), Shi 2015 (5.93% vs 0%) and the tests-are-specifications argument against it. **Same action, opposite verdicts, both stated as design implications.**

Resolve with: measure, on the E5 corpus, how often a test-only-reachable pair encodes a requirement recorded nowhere else (proxy: does the test name/docstring reference an issue ID, a requirement ID, or a business rule absent from the code?). And note Google's unsolved test↔subject attribution (edit distance on names; LZW vs `web_test` are topologically identical).

### R7 — VCS age: three mutually exclusive treatments

Weak accusation (evidence lattice) vs veto-only (static survey) vs *negative weight* (the measurement). Resolve by replicating the age backtest on an **enterprise/abandoned-feature corpus**, which is the population the tool targets and which the six-OSS-library measurement explicitly does not represent.

### R8 — Grep veto: "block on any hit" vs a flag-rate budget

Both are stated as requirements and they conflict. A mandatory basename veto over common names (`utils`, `index`, `config`, `run`, `data`) blocks nearly everything.

Resolve with: measure veto-fire rate as a function of matching strategy (basename only / fully-qualified path / distinctive exported symbols / directory name) on the E5 corpus, and pick the strategy where fire rate is tolerable and the E2 string-reference mutants (classes 1, 8, 13) still all get caught.

### R9 — Coverage: ingest or collect?

Ingesting is safe and free but only available where CI already produces artifacts. Collecting requires executing repo code (§9.10 RCE surface). Resolve by measuring what fraction of the E5 corpus already emits a consumable artifact; if it is high, the collection path can be deferred indefinitely.

### R10 — Quarantine location, unreconciled in the research

Two angles prove in-repo quarantine is disqualified; a third recommends `.repoclean/trash/` inside the repo. **Resolved above in favour of outside-the-repo / refs / tags, but the storage ergonomics for untracked files across filesystems, and the hardlink/reflink semantic-change hazard (§6.19), remain unspecified.**

### R11 — Concurrency and multi-agent operation

Unaddressed everywhere: ledger write races, quarantine collisions, cross-worktree cache eviction, open-FD TOCTTOU at directory granularity. Needs a lock discipline and a worktree-aware quarantine location before any auto-act ships.

### R12 — Cost model and tier scheduling

N analyzers + coverage + mutation testing + one 70B inference per function is a nightly job, not a pre-commit hook. Cost-awareness is **architectural**: it forces a three-cadence design (universal cheap tier every commit — git status, manifest roots, grep veto, magic bytes, duplicates; per-language analyzers on PR; coverage/mutation/LLM nightly), and each cadence produces evidence with different freshness the ledger must reconcile.

### Claims to stop propagating

- **"Knip deleted 300k lines at Vercel with zero false positives."** The line count traces to a single tweet by a Vercel growth-engineering lead (2025-01-02); the zero-FP clause appears only in downstream AI-generated summaries; Vercel is a listed Knip sponsor.
- **Vulture's 60/90/100 "confidence"** are hard-coded constants per AST node type (`DEFAULT_CONFIDENCE = 60`), never calibrated, and the README calls sub-100 values *"very rough estimates."* They recur as probability-like inputs in proposed fusion models. **Never ship an uncalibrated number that looks like a probability** — users threshold on it.
- **Sensenmann's ~5% / >1000 CLs-per-week** publishes no acceptance, revert, or FP rate.
- **The "88% fusion ceiling"** is 39 TodoMVC apps, JavaScript, source-level dead code. Too optimistic for dynamic-language symbol deletion in framework-heavy repos; too pessimistic for the artifact/duplicate tier where content hashing and regenerate-and-diff are proof-grade.
- **"Eder: 33% of never-executed code was unnecessary."** 9 of 27, 95% CI ≈ 17–54%.
- **"Brown et al.: 30–50% of an industrial system is not understood by any current developer."** **No such citation could be located** (Crossref bibliographic search, 2026-07-31). The number circulates without a traceable source. Removed from the prevalence table in §1.5; do not reintroduce it. The sourced neighbour — Meta's developer survey, where 30% of engineers who found the codebase hard to work in named dead code — supports a *much weaker* claim and should be used instead.
- **The dynamic-construct density thresholds (`D > 0.01`, `D > 0.05`) in §9.5.** Author-chosen so the rule is implementable; **no study in the corpus reports a density-versus-precision curve.** They are falsifiable placeholders, and shipping them as though they were measured would repeat exactly the Vulture-confidence error one bullet above.
- **The stability-window defaults (20 runs / 90 days; 10 runs for Tier 1).** Also author-chosen. §11 R3 notes that if stability clocks reset more than ~20% of the time the window is fiction — **so these numbers are unvalidated in both magnitude and in whether the mechanism works at all.**
- **The sampling-profiler probabilities to four significant figures** are a correct model on chosen inputs (100 µs/call, weekly, independence, no burstiness). Present as an illustrative model, not a measurement.
- **"Overhead has stopped being the objection."** True for JVM/Ruby/Python 3.12. False for C/C++/Rust (2–4×) and PHP (~13×).
- **Google's ≤10% FP budget** is for suggestions *a human reviews*. An auto-acting tier must be far below it.
- **The gitignore corpus partition (5.9/3.6/90.5)** is a genuine measurement whose classification rules were never stated — not reproducible or auditable, which is the standard the rest of the research demands.
- **"OpenRewrite ships no file-deletion recipe"** is an existence claim over thousands of recipes made from inspecting a handful. Verify before citing.
- **"Two independent industrial systems converged"** overstates independence: Sensenmann and SCARF both presuppose a hermetic monorepo, org-wide ownership routing, fleet-wide telemetry, and rapid rollback. Convergence under shared preconditions a general OSS tool cannot assume is weak evidence for that tool's architecture.

---

## 12. Sources

### Industrial systems
- [Sensenmann: Code Deletion at Scale](https://testing.googleblog.com/2023/04/sensenmann-code-deletion-at-scale.html) — Phil Norman, Google Testing Blog, 2023-04-28
- [Meta: Automating dead code cleanup](https://engineering.fb.com/2023/10/24/data-infrastructure/automating-dead-code-cleanup/) · [Automating product deprecation](https://engineering.fb.com/2023/10/17/data-infrastructure/automating-product-deprecation-meta/) · [Automating data removal](https://engineering.fb.com/2023/10/31/data-infrastructure/) · [Rapid release at massive scale](https://engineering.fb.com/2017/08/31/web/rapid-release-at-massive-scale/)
- [SCARF paper (ESEC/FSE 2023)](https://dl.acm.org/doi/10.1145/3611643.3613871) — full text: https://yia.nnis.gr/publications/fse2023.pdf
- [Glean](https://github.com/facebookincubator/Glean) · [glean.software](https://glean.software/docs/introduction) — **BSD-licensed, open source**
- [Piranha](https://github.com/uber/piranha) · [ICSE-SEIP 2020 paper](https://manu.sridharan.net/files/ICSE20-SEIP-Piranha.pdf) · [Uber blog](https://www.uber.com/blog/piranha/)
- [Deprecation — SWE at Google, Ch. 15](https://abseil.io/resources/swe-book/html/ch15.html) · [Static Analysis, Ch. 20](https://abseil.io/resources/swe-book/html/ch20.html)
- [Tricorder (ICSE 2015)](https://static.googleusercontent.com/media/research.google.com/en//pubs/archive/43322.pdf) · [Lessons from Building Static Analysis Tools at Google (CACM 2018)](https://cacm.acm.org/research/lessons-from-building-static-analysis-tools-at-google/)
- [Google-Wide Profiling (IEEE Micro 2010)](https://research.google.com/pubs/archive/36575.pdf)
- [Shopify: Introducing the Deprecation Toolkit](https://shopify.engineering/introducing-the-deprecation-toolkit) · [repo](https://github.com/shopify/deprecation_toolkit)
- [AutoTransform (Slack Engineering)](https://slack.engineering/autotransform-efficient-codebase-modification/) · [repo](https://github.com/nathro/AutoTransform)

### Academic
- [Revisiting Code Debloating with Ground Truth-based Evaluation (arXiv:2604.17717v2)](https://arxiv.org/pdf/2604.17717)
- [JavaScript Dead Code Identification, Elimination, and Empirical Assessment (arXiv:2308.16729)](https://arxiv.org/pdf/2308.16729)
- [Eder et al., How Much Does Unused Code Matter for Maintenance? (ICSE 2012)](https://www.cqse.eu/publications/2012-how-much-does-unused-code-matter-for-maintenance.pdf)
- [Romano et al., A Multi-Study Investigation into Dead Code (TSE 2020)](https://www.cs.wm.edu/~denys/pubs/TSE'18-DeadCode.pdf)
- [Caivano et al., On the spread and evolution of dead methods (EMSE 2023)](https://link.springer.com/article/10.1007/s10664-023-10303-0)
- [A Folklore Confirmation on the Removal of Dead Code (EASE 2024)](https://dl.acm.org/doi/10.1145/3661167.3661188)
- [Rothermel et al., Empirical studies of test-suite reduction (STVR 2002)](https://onlinelibrary.wiley.com/doi/abs/10.1002/stvr.256)
- [An Evaluation of Test Suite Minimization Techniques (SWQD 2020)](https://teamscale.com/hubfs/26978363/Publications/2020-test-suite-minimization-swqd.pdf)
- [Shi et al., Comparing and Combining Test-Suite Reduction and Regression Test Selection](https://mir.cs.illinois.edu/marinov/publications/ShiETAL15ReductionSelection.pdf)
- [Zhang et al., How Do Assertions Impact Coverage-based Test-Suite Reduction? (ICST 2017)](https://lingming.cs.illinois.edu/publications/icst2017.pdf)
- [Tu et al., Dead Code Elimination Can Delete Live Code (ICSE-NIER 2024)](https://haoxintu.github.io/files/icse2024-nier-camera-ready.pdf)
- [Livshits et al., In Defense of Soundiness (CACM 2015)](https://cacm.acm.org/opinion/in-defense-of-soundiness/) · [soundiness.org](http://soundiness.org/)
- [SourcererCC (ICSE 2016, arXiv:1512.06448)](https://arxiv.org/abs/1512.06448) · [Towards Automating Precision Studies of Clone Detectors (arXiv:1812.05195)](https://arxiv.org/abs/1812.05195)
- [Benedetti et al., Reproducible Packaging in Open-Source Ecosystems (ICSE 2025)](http://www.cs.cmu.edu/~ckaestne/pdf/icse25_rb.pdf)
- [Bonett et al., muSE — Mutation-Based Evaluation of Static Analysis Soundness (TOSEM)](https://dl.acm.org/doi/fullHtml/10.1145/3439802)
- [DocPrism (arXiv:2511.00215)](https://arxiv.org/html/2511.00215) · [artifact](https://github.com/SnowPhoebe/DocPrism)
- [METAMON (arXiv:2502.02794)](https://arxiv.org/abs/2502.02794) · [Panthaplackel et al. (AAAI 2021, arXiv:2010.01625)](https://ar5iv.labs.arxiv.org/html/2010.01625)
- [Detecting Outdated Code Element References (EMSE, arXiv:2212.01479)](https://arxiv.org/abs/2212.01479) · [DOCER tool demo (arXiv:2307.04291)](https://arxiv.org/html/2307.04291) · [DOCER repo](https://github.com/wesleytanws/DOCER)
- [bpftime — uprobe overhead (arXiv:2311.07923)](https://arxiv.org/html/2311.07923v2)
- [Yoo & Harman, Regression Testing Minimisation, Selection and Prioritisation: A Survey](https://www.cse.chalmers.se/~feldt/advice/yoo_2010_regression_testing_survey.pdf)

### Static analyzers
Knip [how-it-works](https://knip.dev/explanations/how-knip-works) · [handling-issues](https://knip.dev/guides/handling-issues) · [faq](https://knip.dev/reference/faq) · [auto-fix](https://knip.dev/features/auto-fix) · [known-issues](https://knip.dev/reference/known-issues) · [plugins](https://knip.dev/reference/plugins) · [repo](https://github.com/webpro-nl/knip) · issues [#126](https://github.com/webpro-nl/knip/issues/126) [#556](https://github.com/webpro-nl/knip/issues/556) [#719](https://github.com/webpro-nl/knip/issues/719) [#741](https://github.com/webpro-nl/knip/issues/741) [#890](https://github.com/webpro-nl/knip/issues/890) [#1543](https://github.com/webpro-nl/knip/issues/1543)
[Vulture](https://github.com/jendrikseipp/vulture) · issues [#110](https://github.com/jendrikseipp/vulture/issues/110) [#216](https://github.com/jendrikseipp/vulture/issues/216) [#231](https://github.com/jendrikseipp/vulture/issues/231) [#232](https://github.com/jendrikseipp/vulture/issues/232) [#335](https://github.com/jendrikseipp/vulture/issues/335) [#373](https://github.com/jendrikseipp/vulture/issues/373) [#422](https://github.com/jendrikseipp/vulture/issues/422)
[Ruff rules](https://docs.astral.sh/ruff/rules/) · [F401 unused-import](https://docs.astral.sh/ruff/rules/unused-import/) · [deptry](https://github.com/fpgmaas/deptry) · [Skylos benchmark](https://github.com/duriantaco/skylos-demo) · [dev.to writeup](https://dev.to/duriantaco/python-dead-code-i-scanned-flask-fastapi-and-7-other-popular-repos-heres-what-i-found-5c1c)
[go deadcode docs](https://pkg.go.dev/golang.org/x/tools/cmd/deadcode) · [Go blog](https://go.dev/blog/deadcode) · [golang/go#61160](https://github.com/golang/go/issues/61160) · [#65054](https://github.com/golang/go/issues/65054) · [#39570](https://github.com/golang/go/issues/39570) · [#58216](https://github.com/golang/go/issues/58216) · [internal directories](https://pkg.go.dev/cmd/go#hdr-Internal_Directories)
[staticcheck unused.go](https://github.com/dominikh/go-tools/blob/master/unused/unused.go) · [checks](https://staticcheck.dev/docs/checks/) · [#48](https://github.com/dominikh/go-tools/issues/48) · [#1648](https://github.com/dominikh/go-tools/issues/1648)
[cargo-machete](https://github.com/bnjbvr/cargo-machete) · [cargo-udeps](https://github.com/est31/cargo-udeps) ([#143](https://github.com/est31/cargo-udeps/issues/143)) · [cargo-shear](https://github.com/Boshen/cargo-shear) · [rustc dead_code](https://doc.rust-lang.org/rustc/lints/listing/warn-by-default.html#dead-code) · rust-lang/rust [#57613](https://github.com/rust-lang/rust/issues/57613) [#128617](https://github.com/rust-lang/rust/issues/128617) [#121040](https://github.com/rust-lang/rust/issues/121040) [#51928](https://github.com/rust-lang/rust/issues/51928) [#68408](https://github.com/rust-lang/rust/issues/68408)
[Periphery](https://github.com/peripheryapp/periphery) · issues [#234](https://github.com/peripheryapp/periphery/issues/234) [#994](https://github.com/peripheryapp/periphery/issues/994) [#1061](https://github.com/peripheryapp/periphery/issues/1061) [#1067](https://github.com/peripheryapp/periphery/issues/1067) [#1092](https://github.com/peripheryapp/periphery/issues/1092) [#1108](https://github.com/peripheryapp/periphery/issues/1108) [#1112](https://github.com/peripheryapp/periphery/issues/1112) [#1121](https://github.com/peripheryapp/periphery/issues/1121) · Swift index-store bugs [#56541](https://github.com/apple/swift/issues/56541) [#56327](https://github.com/apple/swift/issues/56327) [#56189](https://github.com/apple/swift/issues/56189) [#56165](https://github.com/apple/swift/issues/56165)
[PMD rules](https://docs.pmd-code.org/latest/pmd_rules_java_bestpractices.html) · [report formats](https://docs.pmd-code.org/latest/pmd_userdocs_report_formats.html) · [CPD](https://docs.pmd-code.org/latest/pmd_userdocs_cpd.html) · [Error Prone](https://errorprone.info/bugpatterns) · [UCDetector](http://www.ucdetector.org/) · [SonarSource RSPEC-1144](https://rules.sonarsource.com/java/RSPEC-1144/)
[ProGuard usage](https://www.guardsquare.com/manual/configuration/usage) · [troubleshooting](https://www.guardsquare.com/manual/troubleshooting/troubleshooting) · [Android keep rules](https://developer.android.com/topic/performance/app-optimization/keep-rules-overview) · [keep-rule examples](https://developer.android.com/topic/performance/app-optimization/keep-rule-examples) · kotlinx.coroutines [#983](https://github.com/Kotlin/kotlinx.coroutines/issues/983) [#3111](https://github.com/Kotlin/kotlinx.coroutines/issues/3111)
[.NET trimming options](https://learn.microsoft.com/en-us/dotnet/core/deploying/trimming/trimming-options) · [fixing warnings](https://learn.microsoft.com/en-us/dotnet/core/deploying/trimming/fixing-warnings) · [IDE0051](https://learn.microsoft.com/en-us/dotnet/fundamentals/code-analysis/style-rules/ide0051) · [ILLink error codes](https://github.com/dotnet/runtime/blob/main/docs/tools/illink/error-codes.md)
[GraalVM reachability metadata](https://www.graalvm.org/latest/reference-manual/native-image/metadata/) · [automatic collection](https://www.graalvm.org/latest/reference-manual/native-image/metadata/AutomaticMetadataCollection/) · [oracle/graal GR-40106](https://github.com/oracle/graal/issues/5171)
[debride](https://github.com/seattlerb/debride) · [joshuaclayton/unused](https://github.com/joshuaclayton/unused) · [composer-unused](https://github.com/composer-unused/composer-unused) · [tomasvotruba/unused-public](https://github.com/TomasVotruba/unused-public) · [PHPStan dead code](https://phpstan.org/blog/detecting-unused-private-properties-methods-constants) · [Erlang xref](https://www.erlang.org/doc/apps/tools/xref_chapter.html) · [weeder](https://github.com/ocharles/weeder) · [ts-prune](https://github.com/nadeesha/ts-prune) · [unimported](https://github.com/smeijer/unimported) · [depcheck](https://github.com/depcheck/depcheck) · [madge](https://github.com/pahen/madge) · [dpdm](https://github.com/acrazing/dpdm) · [tsr](https://github.com/line/tsr)

### Runtime and coverage
[coverage.py config](https://coverage.readthedocs.io/en/latest/config.html) · [source](https://coverage.readthedocs.io/en/latest/source.html) · [db schema](https://coverage.readthedocs.io/en/latest/dbschema.html) · [#1708](https://github.com/nedbat/coveragepy/issues/1708) · [#1746](https://github.com/nedbat/coveragepy/issues/1746) · [sysmon post](https://nedbatchelder.com/blog/202312/coveragepy_with_sysmonitoring.html) · [PEP 669](https://peps.python.org/pep-0669/) · [PyPI 81% faster](https://blog.trailofbits.com/2025/05/01/making-pypis-test-suite-81-faster/)
[Coverband](https://github.com/danmayer/coverband) · issues [#186](https://github.com/danmayer/coverband/issues/186) [#301](https://github.com/danmayer/coverband/issues/301) [#384](https://github.com/danmayer/coverband/issues/384) · [Ruby coverage.c](https://github.com/ruby/ruby/blob/master/ext/coverage/coverage.c) · [Feature #15022 oneshot](https://bugs.ruby-lang.org/issues/15022)
[c8](https://github.com/bcoe/c8) · [NODE_V8_COVERAGE](https://nodejs.org/api/cli.html#node_v8_coveragedir) · [v8.takeCoverage](https://nodejs.org/api/v8.html#v8takecoverage) · [node test coverage](https://nodejs.org/api/test.html#collecting-code-coverage) · [nyc](https://github.com/istanbuljs/nyc) · [DevTools Coverage](https://developer.chrome.com/docs/devtools/coverage)
[JaCoCo FAQ](https://www.jacoco.org/jacoco/trunk/doc/faq.html) · [implementation](https://www.jacoco.org/jacoco/trunk/doc/implementation.html) · [class ids](https://www.jacoco.org/jacoco/trunk/doc/classids.html) · [agent](https://www.jacoco.org/jacoco/trunk/doc/agent.html) · [cli](https://www.jacoco.org/jacoco/trunk/doc/cli.html) · [teamscale-jacoco-agent](https://github.com/cqse/teamscale-jacoco-agent) · [Picnic: 0.03% overhead](https://foojay.io/today/how-to-find-dead-code-in-your-java-services/) · [ContaAzul](https://carlosbecker.com/posts/production-code-coverage-jacoco/)
[coverlet](https://github.com/coverlet-coverage/coverlet) · [dotnet-coverage](https://learn.microsoft.com/en-us/dotnet/core/additional-tools/dotnet-coverage) · [dotnet-trace](https://learn.microsoft.com/en-us/dotnet/core/diagnostics/dotnet-trace) · [endjin/deadcode](https://github.com/endjin/deadcode)
[go build -cover](https://go.dev/doc/build-cover) · [integration test coverage](https://go.dev/blog/integration-test-coverage) · [runtime/coverage](https://pkg.go.dev/runtime/coverage) · [cargo-llvm-cov](https://github.com/taiki-e/cargo-llvm-cov) · [Clang source-based coverage](https://clang.llvm.org/docs/SourceBasedCodeCoverage.html) · [llvm-project #124353](https://github.com/llvm/llvm-project/pull/124353) · [gcov](https://gcc.gnu.org/onlinedocs/gcc/Gcov.html) · [gcov shared-lib flush thread](https://gcc.gnu.org/legacy-ml/gcc-help/2015-06/msg00136.html)
[pcov](https://github.com/krakjoe/pcov) · [Xdebug coverage](https://xdebug.org/docs/code_coverage) · [pcov vs Xdebug](https://thephp.cc/articles/pcov-or-xdebug) · [16s→215.95s](https://geshan.com.np/blog/2020/11/phpunit-code-coverage-pcov/)
[lcov geninfo spec](https://manpages.opensuse.org/Leap-15.6/lcov/geninfo.1.en.html) · [grcov](https://github.com/mozilla/grcov)
[Parca agent design](https://www.parca.dev/docs/parca-agent-design/) · [Polar Signals: design of continuous profilers](https://www.polarsignals.com/blog/posts/2022/12/14/design-of-continuous-profilers) · [Pyroscope](https://grafana.com/oss/pyroscope/) · [Datadog profiler](https://docs.datadoghq.com/profiler/) · [py-spy](https://github.com/benfred/py-spy) · [bpftrace docs](https://bpftrace.org/docs)
[MaskRay: Linker garbage collection](https://maskray.me/blog/2021-02-28-linker-garbage-collection) · [Bloaty McBloatface](https://github.com/google/bloaty)

### Tombstones and production probes
[scheb/tombstone](https://github.com/scheb/tombstone) · [Introducing Tombstones for PHP](https://www.christianscheb.de/archives/717) · [Nestoria/Lokku (archived)](https://web.archive.org/web/20201108123420/https://devblog.nestoria.com/post/115930183873/we-too-tombstone-dead-code) · [Schnepper, Velocity 2014](https://www.youtube.com/watch?v=29UXzfQWOhQ) · [Code Tombstones](https://www.phpscaling.com/post/code-tombstones/) · [lewispb/tombstone](https://github.com/lewispb/tombstone) · [Scythe](https://michaelfeathers.silvrback.com/scythe-using-coverage-in-production-to-find-dead-code) · [scythe repo](https://github.com/michaelfeathers/scythe) · [halogen](https://github.com/ileitch/halogen) · [github/scientist](https://github.com/github/scientist)

### Substrates and build systems
[SCIP](https://github.com/sourcegraph/scip) · [scip.proto](https://raw.githubusercontent.com/sourcegraph/scip/main/scip.proto) · [SCIP CLI](https://raw.githubusercontent.com/sourcegraph/scip/main/docs/CLI.md) · [Announcing SCIP](https://sourcegraph.com/blog/announcing-scip) · [LSIF 0.6.0](https://microsoft.github.io/language-server-protocol/specifications/lsif/0.6.0/specification/) · [LSP 3.17](https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/) · [stack-graphs (abandoned)](https://github.com/github/stack-graphs)
[bazel query language](https://bazel.build/query/language) · [cquery](https://bazel.build/docs/cquery) · [aquery](https://bazel.build/docs/aquery) · [user manual](https://bazel.build/docs/user-manual) · [remote caching](https://bazel.build/remote/caching) · [strict_java_deps / unused_deps](https://blog.bazel.build/2017/06/28/sjd-unused_deps.html) · [bazelbuild/buildtools#966](https://github.com/bazelbuild/buildtools/issues/966) · [Bazel knowledge: unused deps](https://fzakaria.com/2025/08/27/bazel-knowledge-dive-into-unused-deps)
[nix-store --gc](https://nix.dev/manual/nix/2.24/command-ref/nix-store/gc.html) · [Turborepo caching](https://turborepo.com/docs/crafting-your-repository/caching) · [Nx affected](https://nx.dev/ci/features/affected) · [pnpm dedupe](https://pnpm.io/cli/dedupe) · [npm/cli#5307](https://github.com/npm/cli/issues/5307) · [npm/cli#4285](https://github.com/npm/cli/issues/4285)
[OpenRewrite LST](https://docs.openrewrite.org/concepts-and-explanations/lossless-semantic-trees) · [type attribution](https://docs.openrewrite.org/concepts-and-explanations/type-attribution) · [FAQ](https://docs.openrewrite.org/reference/faq) · [NoMissingTypes.java](https://raw.githubusercontent.com/openrewrite/rewrite/main/rewrite-java/src/main/java/org/openrewrite/java/NoMissingTypes.java) · [FindMissingTypes.java](https://raw.githubusercontent.com/openrewrite/rewrite/main/rewrite-java/src/main/java/org/openrewrite/java/search/FindMissingTypes.java) · [DeleteSourceFiles.java](https://raw.githubusercontent.com/openrewrite/rewrite/main/rewrite-core/src/main/java/org/openrewrite/DeleteSourceFiles.java) · [RemoveUnusedPrivateMethods.java](https://raw.githubusercontent.com/openrewrite/rewrite-static-analysis/main/src/main/java/org/openrewrite/staticanalysis/RemoveUnusedPrivateMethods.java) · issues [rewrite#1536](https://github.com/openrewrite/rewrite/issues/1536) / [PR#1580](https://github.com/openrewrite/rewrite/pull/1580) · [static-analysis#321](https://github.com/openrewrite/rewrite-static-analysis/issues/321) / [PR#648](https://github.com/openrewrite/rewrite-static-analysis/pull/648) · [#294](https://github.com/openrewrite/rewrite-static-analysis/issues/294) · [rewrite#4783](https://github.com/openrewrite/rewrite/issues/4783) · [Moderne LST](https://moderne.ai/blog/lossless-semantic-tree-the-complete-code-data-model-for-automated-code-refactoring-and-analysis)

### Git, reversibility, non-source
[git-clean](https://git-scm.com/docs/git-clean) · [git-gc](https://git-scm.com/docs/git-gc) · [git-reflog](https://git-scm.com/docs/git-reflog) · [git-bundle](https://git-scm.com/docs/git-bundle) · [git-blame (`--ignore-revs-file`)](https://git-scm.com/docs/git-blame) · [git-worktree](https://git-scm.com/docs/git-worktree) · [gitsubmodules](https://git-scm.com/docs/gitsubmodules) · [git-ls-files](https://git-scm.com/docs/git-ls-files)
[git-filter-repo](https://github.com/newren/git-filter-repo) · [FRESHCLONE rationale](https://htmlpreview.github.io/?https://github.com/newren/git-filter-repo/blob/docs/html/git-filter-repo.html#FRESHCLONE) · [BFG](https://github.com/rtyley/bfg-repo-cleaner) · [git-sizer](https://github.com/github/git-sizer) · [git-lfs-migrate](https://github.com/git-lfs/git-lfs/blob/main/docs/man/git-lfs-migrate.adoc) · [git-lfs-prune](https://github.com/git-lfs/git-lfs/blob/main/docs/man/git-lfs-prune.adoc) · [git-lfs#4206](https://github.com/git-lfs/git-lfs/issues/4206)
[git-annex walkthrough](https://git-annex.branchable.com/walkthrough/) · [DVC internal files](https://dvc.org/doc/user-guide/project-structure/internal-files)
[github/gitignore](https://github.com/github/gitignore) · [Linguist vendor.yml](https://github.com/github-linguist/linguist/blob/main/lib/linguist/vendor.yml) · [generated.rb](https://github.com/github-linguist/linguist/blob/main/lib/linguist/generated.rb)
[kondo](https://github.com/tbillington/kondo) · [npkill](https://github.com/voidcosmos/npkill) · [czkawka](https://github.com/qarmin/czkawka) · [rmlint](https://github.com/sahib/rmlint) ([cautions.rst](https://github.com/sahib/rmlint/blob/master/docs/cautions.rst)) · [jdupes](https://codeberg.org/jbruchon/jdupes) · [fdupes](https://github.com/adrianlopezroche/fdupes) · [rdfind](https://github.com/pauldreik/rdfind) · [dust](https://github.com/bootandy/dust) · [trash-cli](https://github.com/andreafrancia/trash-cli) · [libtrashcan](https://github.com/robertguetzkow/libtrashcan) · [freedesktop trash spec](https://www.freedesktop.org/wiki/Specifications/trash-spec/)
[docker system prune](https://docs.docker.com/reference/cli/docker/system/prune/) · [diffoscope](https://diffoscope.org/) · [reprotest](https://salsa.debian.org/reproducible-builds/reprotest) · [container-diff](https://github.com/GoogleContainerTools/container-diff) · [reproducible-builds docs](https://reproducible-builds.org/docs/) · [SOURCE_DATE_EPOCH](https://reproducible-builds.org/docs/source-date-epoch/) · [volatile inputs](https://reproducible-builds.org/docs/volatile-inputs/)
[detect-secrets](https://github.com/Yelp/detect-secrets) · [TruffleHog](https://github.com/trufflesecurity/trufflehog)

### External effectors, platform contracts, CI
[Terraform resource syntax (`prevent_destroy`)](https://developer.hashicorp.com/terraform/language/resources/syntax) · [Argo CD auto-sync / prune](https://argo-cd.readthedocs.io/en/stable/user-guide/auto_sync/)
[actions/checkout (`fetch-depth: 1` default)](https://github.com/actions/checkout) · [GitHub Actions secure use (self-hosted runners)](https://docs.github.com/en/actions/reference/security/secure-use) · [GitHub Pages custom domain / takeover](https://docs.github.com/en/pages/configuring-a-custom-domain-for-your-github-pages-site/managing-a-custom-domain-for-your-github-pages-site)
[Celery first steps with Django](https://docs.celeryq.dev/en/stable/django/first-steps-with-django.html)

### Documentation staleness
[cog](https://github.com/nedbat/cog) ([docs](https://cog.readthedocs.io/)) · [embedme](https://github.com/zakhenry/embedme) · [mdcode](https://github.com/szkiba/mdcode) · [byexample](https://github.com/byexamples/byexample) · [tesh](https://github.com/OceanSprint/tesh) · [pytest-markdown-docs](https://github.com/modal-labs/pytest-markdown-docs) · [Doc Detective](https://github.com/doc-detective/doc-detective) · [Docs as Tests](https://www.docsastests.com/docs-as-tests/concept/2024/01/09/intro-docs-as-tests.html)
[rustdoc documentation tests](https://doc.rust-lang.org/rustdoc/write-documentation/documentation-tests.html) · [Testable Examples in Go](https://go.dev/blog/examples) · [lychee](https://github.com/lycheeverse/lychee) · [markdown-link-check](https://github.com/tcort/markdown-link-check) · [Vale](https://github.com/vale-cli/vale) · [markdownlint](https://github.com/DavidAnson/markdownlint) · [interrogate](https://github.com/econchick/interrogate) · [mdbook-i18n-helpers USAGE](https://github.com/google/mdbook-i18n-helpers/blob/main/i18n-helpers/USAGE.md) · [Docusaurus i18n](https://docusaurus.io/docs/i18n/introduction) · [sphinx#5281](https://github.com/sphinx-doc/sphinx/issues/5281) · [sphinx#4770](https://github.com/sphinx-doc/sphinx/issues/4770)

### Mutation testing and test selection
[Stryker FAQ](https://stryker-mutator.io/docs/General/faq/) · [PIT](https://pitest.org/) · [Assessing and Improving PIT (arXiv:1601.02351)](https://arxiv.org/pdf/1601.02351) · [mutmut](https://github.com/boxed/mutmut) · [cargo-mutants](https://github.com/sourcefrog/cargo-mutants) ([limitations](https://github.com/sourcefrog/cargo-mutants/blob/main/book/src/limitations.md)) · [LittleDarwin subsumption (arXiv:1809.02435)](https://arxiv.org/pdf/1809.02435) · [Ekstazi](https://github.com/gliga/ekstazi) · [coverage.py contexts](https://coverage.readthedocs.io/en/latest/contexts.html)

### Incidents
[SEC 34-70694 (Knight Capital)](https://www.sec.gov/files/litigation/admin/2013/34-70694.pdf) · [Knightmare: A DevOps Cautionary Tale](https://dougseven.com/2014/04/17/knightmare-a-devops-cautionary-tale/) · [Dolfing case study](https://www.henricodolfing.ch/en/case-study-4-the-440-million-software-error-at-knight-capital/)
[Debian DSA-1571-1 (CVE-2008-0166)](https://www.debian.org/security/2008/dsa-1571) · [research!rsc: OpenSSL](https://research.swtch.com/openssl) · [badkeys.info](https://badkeys.info/docs/debian.html)
[AI Incident Database #1152 (Replit)](https://incidentdatabase.ai/cite/1152/) · [#1178 (Gemini CLI)](https://incidentdatabase.ai/cite/1178/) · [gemini-cli#4586](https://github.com/google-gemini/gemini-cli/issues/4586) · [Antigravity D-drive wipe](https://www.theregister.com/2025/12/01/google_antigravity_wipes_d_drive/)
[Auto-Claude #1477 (`git clean -fd` data loss)](https://github.com/AndyMik90/Auto-Claude/issues/1477)
[Stale feature flag turned a feature back on](https://dev.to/pixel-wraith/the-stale-feature-flag-we-deleted-that-turned-a-feature-back-on-529m) · [Feature flag anti-patterns (~$47k)](https://featureflip.io/blog/feature-flag-anti-patterns/)
[GameMaker unused-asset removal #8735](https://github.com/YoYoGames/GameMaker-Bugs/issues/8735) · [#10460](https://github.com/YoYoGames/GameMaker-Bugs/issues/10460)
[webpack sideEffects / tree-shaking](https://webpack.js.org/guides/tree-shaking/)
[HN discussion of Sensenmann](https://news.ycombinator.com/item?id=35755841) · [Ask HN: why Python dead-code detection is hard](https://news.ycombinator.com/item?id=46866141)

### Practitioner and UX
[JetBrains Safe Delete](https://www.jetbrains.com/help/idea/safe-delete.html) · [Why engineers resist deleting unused code](https://understandlegacycode.com/blog/delete-unused-code/) · [Knip on a high-traffic repo (WIP false positives)](https://madelinemiller.dev/blog/knip-dead-code/) · [Angular/Knip case study](https://blog.iterative.engineering/2024/03/20/strengths-and-limitations-of-knip-for-unused-code-detection-in-angular/) · [Gary Tyr tweet (Vercel 300k)](https://x.com/gary__tyr/status/1874692207472726401) · [code-maat](https://github.com/adamtornhill/code-maat) · [testdouble: redundant coverage](https://github.com/testdouble/contributing-tests/wiki/Redundant-Coverage)

### Agent-skill cleaners (surveyed adversarially)
[NickCrew/Claude-Cortex repo-cleanup](https://github.com/NickCrew/Claude-Cortex/tree/main/skills/repo-cleanup) · [rohitg00/awesome-claude-code-toolkit](https://github.com/rohitg00/awesome-claude-code-toolkit) · [grahama1970/agent-skills cleanup](https://github.com/grahama1970/agent-skills/tree/main/skills/cleanup) · [jonesrussell: building a codebase cleanup skill](https://jonesrussell.github.io/blog/building-codebase-cleanup-skill-claude-code/) · [stevenjtobin/spring-clean](https://github.com/stevenjtobin/spring-clean) · [repowise DEAD_CODE.md](https://github.com/repowise-dev/repowise/blob/main/docs/layers/DEAD_CODE.md) · [Trkzi-Omar/prune-skills](https://github.com/Trkzi-Omar/prune-skills)

### Root sets, hazards, and analyzers added in the 2026-07-31 hardening pass
*(Every URL below was resolved with a live GET on 2026-07-31 and returned HTTP 200.)*

- **Prevalence** — [Boomsma, Hostnet & Gross, *Dead code elimination for web systems written in PHP: lessons learned from an industry case*, ICSM 2012, DOI 10.1109/ICSM.2012.6405314](https://doi.org/10.1109/ICSM.2012.6405314) — the source for the "~30% of files, 2,740 removed" row in §1.5, which was previously uncited
- **Python invisible entry points** — [`site` module: `.pth` files, `sitecustomize`, `usercustomize`](https://docs.python.org/3/library/site.html) (lines beginning `import` in a `.pth` are executed at interpreter startup)
- **JVM** — [`java.io.Serializable` / `serialVersionUID`](https://docs.oracle.com/javase/8/docs/api/java/io/Serializable.html) · [Spring Boot 3.0 Migration Guide — `spring.factories` → `META-INF/spring/…AutoConfiguration.imports`](https://github.com/spring-projects/spring-boot/wiki/Spring-Boot-3.0-Migration-Guide) · [Android app manifest (the largest string-referenced root set in the JVM world)](https://developer.android.com/guide/topics/manifest/manifest-intro)
- **.NET** — [`[ModuleInitializer]` — runs before `Main`, called by nothing](https://learn.microsoft.com/en-us/dotnet/api/system.runtime.compilerservices.moduleinitializerattribute)
- **Swift/Apple** — [Swift `PackageDescription` (products, targets, plugins, `resources`)](https://docs.swift.org/package-manager/PackageDescription/PackageDescription.html) · [Information Property List (`NSPrincipalClass`, `NSExtensionPrincipalClass`, `UISceneDelegateClassName`, `CFBundleURLTypes`, …)](https://developer.apple.com/documentation/bundleresources/information-property-list)
- **C/C++** — [CMake `install()`](https://cmake.org/cmake/help/latest/command/install.html) · [GNU ld version scripts (`VERSION { global: … }`)](https://sourceware.org/binutils/docs/ld/VERSION.html) · [GCC common function attributes (`used`, `retain`, `constructor`, `visibility`)](https://gcc.gnu.org/onlinedocs/gcc/Common-Function-Attributes.html) · [cppcheck manual (`--enable=unusedFunction` is whole-program and disabled under `-j`)](https://cppcheck.sourceforge.io/manual.pdf) · [include-what-you-use](https://include-what-you-use.org/)
- **§6.24 persisted / in-flight / shipped references** — [protobuf `reserved` fields and tag-number reuse](https://protobuf.dev/programming-guides/proto3/) · [Sidekiq best practices (job payloads hold class names; deploy ordering vs. queue drain)](https://github.com/sidekiq/sidekiq/wiki/Best-Practices)
- **§6.11 platform contracts** — [IANA well-known URI registry (the authoritative enumeration)](https://www.iana.org/assignments/well-known-uris/well-known-uris.xhtml) · [Let's Encrypt challenge types (`.well-known/acme-challenge`)](https://letsencrypt.org/docs/challenge-types/) · [IAB `ads.txt` / `app-ads.txt`](https://iabtechlab.com/ads-txt/) · [Web app manifest](https://developer.mozilla.org/en-US/docs/Web/Manifest)

### Codemod infrastructure
[codemod/codemod](https://github.com/codemod/codemod) ([jssg security](https://docs.codemod.com/jssg/security)) · [GritQL](https://github.com/getgrit/gritql) · [comby](https://github.com/comby-tools/comby) · [ast-grep](https://github.com/ast-grep/ast-grep) · [Semgrep](https://github.com/semgrep/semgrep)

---

*Author inference is used throughout for anything not carrying a source link — in particular §9 (architecture), §10 (evaluation design), and the tier arithmetic in §9.5–9.6, all of which are proposals derived from the cited evidence rather than reports of it. Where the research disagreed with itself, both sides are presented in §11.*

---

### Hardening pass — 2026-07-31

This document was reviewed for unsourced claims, internal contradictions, and coverage gaps. What changed, so a reader can tell hardened material from original:

**Contradictions found and resolved in place** (each marked with a `⚠ Resolved contradiction` note at the point of use, with both sides stated):
1. **Test-coverage weight** — §9.5 assigned `+0.2` bans toward deadness while §0, §2.1, §3.4, and §11 R5 all said *zero*. Corrected to **0.0**; §9.5 is now the single source of truth and R5 updated to match.
2. **Tier-0 recoverability eligibility** — §9.6 permitted auto-action on ignored files; §8.1 said tracked-and-pushed is the only eligible class. Resolved by distinguishing **quarantine from reap**: ignored → auto-quarantine only, untracked → report only at every tier, tracked-and-pushed → both.
3. **`--fix`** — §9.13 said "there is no `--fix`" while §7.5 said to copy Knip's `--fix`/`--allow-remove-files` two-gate pattern "exactly." Resolved by naming the actual gates (`--quarantine-edits`, `--quarantine-files`, `--allow-inferred-roots`) and noting that what is copied is the *separation*, not the flag names.
4. **VCS age** — §9.5's positive H-family bans contradict §6.18's measurement that age is anti-predictive. Marked **unvalidated, shipped at 0.0 behind a flag**; noted that H can never satisfy the family quorum, so no tier decision depends on it.
5. **Grep polarity** — §2.1 listed the whole-corpus grep as VETO-only while §9.5 scored its silence at `+1.0`. Both directions now stated in the polarity cell, with the truncation case explicitly ABSTAIN.
6. **Gate-3 conjunct count** — §9.6 said "all four Gate-3 conjuncts" where Gate 3 had five (now six). Corrected.

**Vague criteria made implementable** (§9.5, §9.6, Gate 3e): exact definitions added for *a family ACCUSES* (MAX ≥ +0.5 bans, artifact healthy, not expired; family H excluded, so the only quorums are {B,R}, {B,X}, {R,X}), *a family is LOAD-BEARING* (replacing the un-implementable "a family whose absence is load-bearing"), and *holds the deadness invariant* (evidence must share the run's tree SHA; a run that cannot collect a family does not advance the clock). Dynamic-construct density given a formula and thresholds; rate limit given a number; "unresolved configuration hint" enumerated.

**Coverage gaps closed:** §5.2 gained **Swift/Objective-C** and **C/C++** root-set checklists (both entirely absent) and substantial additions to the other eight ecosystems; §4.1 gained **cppcheck, IWYU/`-Wunused`, nm/objdump/Bloaty, detekt, scalac `-Wunused`/scalafix, RuboCop** rows (C/C++, Kotlin, and Scala were unrepresented); §6.1 gained **Rust link-time registries** and the **C++ static-initializer self-registering factory**; §6.11 gained ten platform-contract rows; **§6.24 is new** (persisted, in-flight, and already-shipped references — serialized queue payloads, deserialization schemas, wire-format evolution, ABI removal, cached URLs, signed artifacts); §10 E2 gained **five mutant classes (15–19)** covering it.

**Sourcing:** the previously-uncited Boomsma PHP prevalence figure now carries its DOI; **"Brown et al., 30–50%" could not be located via Crossref and has been removed from §1.5 and added to "claims to stop propagating."** Twenty new source URLs were resolved live (HTTP 200) before being added. Three sets of author-chosen numbers that could be mistaken for measurements — the density thresholds, the stability-window defaults, and the H-family positive bans — are now explicitly labelled as unvalidated placeholders.

**Irreversibility prominence:** the gitignored/untracked hazard was previously first stated in full at §8.1, ~58% of the way into the document. It now leads the document as a boxed table before §0, is cross-referenced from executive-summary item 10, and is enforced structurally as **Gate 0g** — recoverability class is computed *before* any usefulness question, because the cost of being wrong is set by the ladder rung, not the tier.


