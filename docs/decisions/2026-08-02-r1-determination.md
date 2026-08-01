# R1 — does an auto-act tier exist at all? Determination

> ## STANDING DETERMINATION. NOT A FINAL CALL — and the reason is named.
>
> **Status: NOT YET TESTABLE. The pre-commitment is not triggered, and the auto-act tier is
> presumed absent until it is.**
>
> This supersedes the INTERIM version of this document, which was written before Gate 1 existed
> and which said it should be re-run verbatim once Gate 1 landed. Gate 1 has landed. It was
> re-run. **It moved nothing**, and §2.6 is the part of this document that explains why that is
> the expected result rather than a disappointment.
>
> What still blocks a final call is unchanged: Gate 3 does not exist, and no X-family (runtime)
> signal exists anywhere in the project. Every number below was produced by a stack of four
> static analyzers plus three rescue layers that all read repository text — and the section that
> names the pre-commitment asks about *signal combinations*, of which that is one.
>
> **Do not cite this document as resolving R1.** It records what four analyzers plus the three
> rescue layers built so far actually do, states what that evidence can and cannot carry, and
> fixes in advance the exact observations that would overturn it.

**Date:** 2026-08-02 · **Repository:** `Judged` · **Author:** second measurement round, post-Gate 1

---

## 1. The pre-commitment

Three statements of the same commitment, quoted verbatim from
`docs/research/2026-07-31-universal-safe-repo-cleaner-research.md`, with line numbers.

**§10 E2, line 1418** — the pre-commitment itself:

> **If no signal combination clears all 14 at zero false removals, the product is
> report+quarantine and the auto-act tier must be DELETED from the design rather than tuned.**
> This is falsifiable in weeks and costs nothing to run early.

**§11 R1, lines 1449–1451** — the open question it answers:

> ### R1 — Does an auto-act tier exist at all?
>
> Everything downstream (ledger, stability window, quarantine reaping, rate limiting,
> thresholds) assumes the answer is yes. **Resolve with E2 (§10) run before any analyzer
> integration.** If no signal combination clears all 14 mutant classes at zero false removals,
> the honest product is report+quarantine and the auto-act tier is deleted, not tuned. Weeks,
> near-zero cost.

**§0, line 34 and line 43** — the same commitment in the executive summary:

> 6. The measured precision ceiling for multi-signal fusion on real code is ~88% with recall
> collapsing to ~54%. Fusion gets you a good *ranked queue for a human*, not an auto-delete
> tier. **This is in direct tension with §9.6 shipping a Tier 0 at all** — deliberately so:
> §11 R1 makes the existence of an auto-act tier the single highest-risk open question,
> resolvable in weeks by E2 (§10), with the pre-committed answer that if E2 does not come back
> clean the tier is **deleted from the design rather than tuned**. […]

> 15. Recommended first six months: ratchet + root-set materializer + never-touch inventory +
> quarantine ladder + a 14-class mutation-injection safety suite. Gate any auto-act tier on
> that suite showing zero false removals. If it can't, ship with no auto-act tier and say so.

And **§10 E2, line 1416**, which governs the scope of "all 14" and is argued in §5.4 below:

> Ship this as the tool's own test suite and **gate releases on zero failures**. **The 19
> classes are a floor, not a ceiling** — the original text called 14 a "minimum catalogue" and
> that framing is correct: each class here was derived from one documented real failure, so the
> catalogue grows every time a new one is documented.

---

## 2. What was measured

### 2.1 Provenance

| | |
|---|---|
| Repository | `/Users/neo/Developer/Projects/Judged` |
| Commit | `b64ca87a0c6ab6a9b907f826bb6310b1212ac4fe` (`main`), from `git rev-parse HEAD` |
| Working tree | **not clean, and this matters.** `git status --porcelain` reports 10 modified and 8 untracked paths — the Gate 1 work (`judged-core/src/gate1/`, `judged-mutants/src/gate1.rs`, `judged-cli/src/explain_cmd.rs`, their tests and CLI wiring) plus concurrent edits to `judged-core/src/roots/manifest.rs` and `judged-core/src/veto/reachability.rs`. §2.3 is the control that says what those non-Gate-1 edits did to the numbers. |
| Binary | `/private/tmp/claude-501/judged-r1det2/release/judged`, sha256 `69ca8445ebcd6f57745fd7590f83287ff420d75c65737c6f74a79e00641a4974` — built from that working tree, not from `b64ca87` |
| Toolchain | `rustc 1.94.1 (e408947bf 2026-03-25)`, `cargo 1.94.1 (29ea6fb6a 2026-03-24)` |
| Host | `Darwin 25.3.0 arm64` |

Analyzer versions are unchanged from the interim round and were recovered by the same commands:
vulture 2.16 (`vulture --version`), knip 6.31.0 (`npx --yes knip@6 --version`), `x/tools`
deadcode v0.48.0 (`go version -m ~/go/bin/deadcode`; the binary has no `--version` flag),
cargo-shear 1.13.3 (`--version` prints `Version: dev`; the version comes from the build-path
string in the binary), go1.26.2, node v24.14.0, npx 11.9.0.

### 2.2 Environment and invocation

```sh
export PATH="/Users/neo/.blackhole/Judged/2026-08-02b/.venv/bin:$HOME/go/bin:/Users/neo/.blackhole/Judged/2026-08-01-tools/bin:$PATH"
export CARGO_TARGET_DIR=/private/tmp/claude-501/judged-r1det2
cargo build --release -p judged-cli
cd /Users/neo/Developer/Projects/Judged
```

Every cell in §2.4 was produced by one of these thirty-six commands, run from the repository
root, with exit status read from `$?` directly after the command and never through a pipe:

```sh
judged mutants --sut <sut>                          # bare
judged mutants --sut <sut> --gate1                  # + Gate 1 (§9.3 1a–1p), the never-touch inventory
judged mutants --sut <sut> --veto                   # + Gate 2 (§9.3), needles basename+stem
judged mutants --sut <sut> --roots                  # + the root set (§5), tiers A+B+C
judged mutants --sut <sut> --veto --roots           # the previous round's full stack
judged mutants --sut <sut> --gate1 --veto --roots   # the full stack
#   <sut> ∈ { naive, refusing, vulture, knip, deadcode, shear }
```

The 36 `--json` runs took 2 minutes 0 seconds of wall clock, 06:34:35 to 06:36:35, most of it
`npx` resolving knip. The five full-stack runs were repeated in human-readable form (06:38:47 to
06:39:12) and agree with the JSON line for line; the human output is what §2.6 quotes.

### 2.3 The control: what the non-Gate-1 working-tree edits did

The binary is not built from a clean `b64ca87`, so before reading anything else, the twenty-four
configurations the interim document measured were re-run and compared cell by cell.

**Exactly one cell moved, and it is a control row: `naive --roots`, 17 → 16 false removals.** The
class list for that row is identical (m01, m02, m03, m08, m09, m13, m14, m16, m18, m19), so the
root set now rescues one further artifact inside a class it was already failing. `naive --veto
--roots` is unchanged at 4, so it is an artifact Gate 2 also catches.

**No cell for any of the four real analyzers moved.** vulture, knip, deadcode and cargo-shear
reproduce the interim document's bare, `--veto`, `--roots` and `--veto --roots` rows exactly —
same false-removal counts, same failing classes, same decoy recalls. So the concurrent edits to
`roots/manifest.rs` and `veto/reachability.rs` are E2-invisible on the population this
determination rests on, and every comparison below between "before Gate 1" and "after Gate 1" is
a comparison of Gate 1 and nothing else.

### 2.4 The full stack — every analyzer against every rescue combination

`false removals` is the gated number: §10 E2 gates on it and, per the output's own words, *on
nothing else*. `prevented` is that configuration's reduction against its own bare row. `decoys`
is genuinely-dead files correctly found, over the number planted in the fixtures that SUT's
languages caused to be built.

| SUT | config | classes graded | not read | **false removals** | prevented | decoys | classes with false removals | gate | exit |
|---|---|---|---|---|---|---|---|---|---|
| `naive` | bare | 19/19 | 0 | **20** | — | 31/31 | m01,m02,m03,m08,m09,m10,m12,m13,m14,m16,m18,m19 | FAIL | 1 |
| `naive` | `--gate1` | 19/19 | 0 | **15** | 5 | 30/31 | m01,m02,m03,m08,m09,m10,m12,m16,m18,m19 | FAIL | 1 |
| `naive` | `--veto` | 19/19 | 0 | **7** | 13 | 26/31 | m02,m10,m12,m18,m19 | FAIL | 1 |
| `naive` | `--roots` | 19/19 | 0 | **16** | 4 | 31/31 | m01,m02,m03,m08,m09,m13,m14,m16,m18,m19 | FAIL | 1 |
| `naive` | `--veto --roots` | 19/19 | 0 | **4** | 16 | 26/31 | m02,m18,m19 | FAIL | 1 |
| `naive` | **full stack** | 19/19 | 0 | **3** | 17 | 26/31 | m02,m19 | FAIL | 1 |
| `refusing` | bare | 19/19 | 0 | **0** | — | 0/31 | — | PASS | 0 |
| `refusing` | `--gate1` | 19/19 | 0 | **0** | 0 | 0/31 | — | PASS | 0 |
| `refusing` | `--veto` | 19/19 | 0 | **0** | 0 | 0/31 | — | PASS | 0 |
| `refusing` | `--roots` | 19/19 | 0 | **0** | 0 | 0/31 | — | PASS | 0 |
| `refusing` | `--veto --roots` | 19/19 | 0 | **0** | 0 | 0/31 | — | PASS | 0 |
| `refusing` | **full stack** | 19/19 | 0 | **0** | 0 | 0/31 | — | PASS | 0 |
| `vulture` | bare | 10/19 | 9 | **6** | — | 11/16 | m01,m10,m11,m16 | FAIL | 1 |
| `vulture` | `--gate1` | 10/19 | 9 | **6** | 0 | 11/16 | m01,m10,m11,m16 | FAIL | 1 |
| `vulture` | `--veto` | 10/19 | 9 | **4** | 2 | 10/16 | m10,m11 | FAIL | 1 |
| `vulture` | `--roots` | 10/19 | 9 | **5** | 1 | 11/16 | m01,m11,m16 | FAIL | 1 |
| `vulture` | `--veto --roots` | 10/19 | 9 | **3** | 3 | 10/16 | m11 | FAIL | 1 |
| `vulture` | **full stack** | 10/19 | 9 | **3** | 3 | 10/16 | m11 | FAIL | 1 |
| `knip` | bare | 3/19 | 16 | **2** | — | 4/6 | m02,m14 | FAIL | 1 |
| `knip` | `--gate1` | 3/19 | 16 | **1** | 1 | 3/6 | m02 | FAIL | 1 |
| `knip` | `--veto` | 3/19 | 16 | **1** | 1 | 2/6 | m02 | FAIL | 1 |
| `knip` | `--roots` | 3/19 | 16 | **2** | 0 | 4/6 | m02,m14 | FAIL | 1 |
| `knip` | `--veto --roots` | 3/19 | 16 | **1** | 1 | 2/6 | m02 | FAIL | 1 |
| `knip` | **full stack** | 3/19 | 16 | **1** | 1 | 2/6 | m02 | FAIL | 1 |
| `deadcode` | bare | 1/19 | 18 | **2** | — | 2/2 | m12 | FAIL | 1 |
| `deadcode` | `--gate1` | 1/19 | 18 | **2** | 0 | 2/2 | m12 | FAIL | 1 |
| `deadcode` | `--veto` | 1/19 | 18 | **1** | 1 | 2/2 | m12 | FAIL | 1 |
| `deadcode` | `--roots` | 1/19 | 18 | **2** | 0 | 2/2 | m12 | FAIL | 1 |
| `deadcode` | `--veto --roots` | 1/19 | 18 | **1** | 1 | 2/2 | m12 | FAIL | 1 |
| `deadcode` | **full stack** | 1/19 | 18 | **1** | 1 | 2/2 | m12 | FAIL | 1 |
| `shear` | bare | 6/19 | 13 | **0** | — | 9/9 | — | PASS | 0 |
| `shear` | `--gate1` | 6/19 | 13 | **0** | 0 | 9/9 | — | PASS | 0 |
| `shear` | `--veto` | 6/19 | 13 | **0** | 0 | 5/9 | — | PASS | 0 |
| `shear` | `--roots` | 6/19 | 13 | **0** | 0 | 9/9 | — | PASS | 0 |
| `shear` | `--veto --roots` | 6/19 | 13 | **0** | 0 | 5/9 | — | PASS | 0 |
| `shear` | **full stack** | 6/19 | 13 | **0** | 0 | 5/9 | — | PASS | 0 |

### 2.5 The four real analyzers, aggregated

Sums over `vulture + knip + deadcode + shear`. `naive` and `refusing` are controls and are
excluded — the first is a deliberately bad cleaner whose only job is to fail, the second removes
nothing.

| Configuration | false removals | distinct classes false-removed | decoys |
|---|---|---|---|
| bare | **10** | 7 — m01, m02, m10, m11, m12, m14, m16 | 26/33 |
| `--gate1` (Gate 1 only) | **9** | 6 — m01, m02, m10, m11, m12, m16 | 25/33 |
| `--veto` (Gate 2 only) | **6** | 4 — m02, m10, m11, m12 | 19/33 |
| `--roots` (root set only) | **9** | 6 — m01, m02, m11, m12, m14, m16 | 26/33 |
| `--veto --roots` (the previous full stack) | **5** | 3 — m02, m11, m12 | 19/33 |
| **`--gate1 --veto --roots` — the full stack** | **5** | **3 — m02, m11, m12** | **19/33** |

*Denominator note.* 33 is the sum of the four per-tool denominators and double-counts four
decoys: m02 and m10 are each graded by both vulture and knip, and each plants 2 decoys. The
distinct file count is 29, which is the denominator `README.md` uses. Whether the *found* counts
double-count as well is not determinable from the JSON, which carries per-class decoy counts and
not the file lists, so the two documents' decoy denominators are not currently reconciled. This
is flagged rather than resolved because §10 E2 gates on false removals and on nothing else; no
number in §5 depends on it.

Per-layer attribution under the full stack, read from the JSON `rescue.layers` array. `claims
judged` is **that layer's** denominator, not the accuser's — each layer is handed only what the
previous one passed through:

| SUT | layer | claims judged | claims rescued | false removals prevented |
|---|---|---|---|---|
| `vulture` | gate1 | 37 | 1 | **0** |
| | roots | 36 | 1 | 1 |
| | veto | 35 | 7 | 2 |
| `knip` | gate1 | 6 | 2 | **1** |
| | roots | 4 | 0 | 0 |
| | veto | 4 | 1 | 0 |
| `deadcode` | gate1 | 6 | 0 | **0** |
| | roots | 6 | 0 | 0 |
| | veto | 6 | 1 | 1 |
| `shear` | gate1 | 9 | 0 | **0** |
| | roots | 9 | 0 | 0 |
| | veto | 9 | 4 | 0 |
| `naive` | gate1 | 130 | 14 | **5** |
| | roots | 116 | 8 | 4 |
| | veto | 108 | 20 | 8 |

`false_removals_prevented_unattributed` is 0 for every SUT: every prevention is credited to a
named layer.

### 2.6 What Gate 1 actually did — and this is the section not to skim

**The headline: Gate 1's marginal contribution to the four real analyzers is exactly zero.** The
last two rows of §2.5 are identical in every column. Five false removals over three classes,
19 of 33 decoys, with Gate 1 and without it. Per analyzer the counterfactual is 3→3, 1→1, 1→1,
0→0.

That is the number. Everything below is about the two places where Gate 1 *did* move something —
because what decides whether a movement counts is not its size but *what it was based on*.

#### 2.6.1 Gate 1 refuses on irreversibility, so it cannot be evidence about R1

§9.3's Gate 1 is the never-touch inventory. Its refusal is justified by the *cost of being
wrong*, not by any belief about whether the artifact is used. The `judged explain` output states
this to the operator in as many words:

```
GATE 1 — the never-touch inventory (§9.3)
  INELIGIBLE — 2 of the sixteen classes refuse this path.
  A Gate 1 refusal is absorbing: it is justified by IRREVERSIBILITY, not by
  uselessness, so no later evidence of uselessness moves it.
```

The pre-commitment asks whether a **signal combination** clears the catalogue. §2.2 of the
research defines the families in terms of what *accuses*. Gate 1 never accuses; it can only
refuse. So it adds no family, and no result it produces — including a lower number — is by itself
evidence that a signal combination got better. A Gate 1 that lowered the count would be making a
wrong answer cheaper, not making the answer more correct.

The two rescues it did produce are exactly that. Both are read below off the justification string
the gate printed for itself.

#### 2.6.2 The one rescue on a real analyzer: knip / m14, and the rule cannot tell live from dead

Run alone, `--gate1` takes knip from 2 false removals to 1. It clears m14 — committed build
output whose only consumer is a CDN path. Verbatim from the run:

```
  m14  FAIL  typescript  0 false  1/2 decoys  committed build output whose only consumer is a CDN path
       gate1 rescued live: dist/widget.7f3a91c.js   [§10 E2 class 14]
       the stack also rescued 1 genuinely-dead decoy file(s) — the price
       [gate1/1j] rescued path dist/widget.0c9e142.js — 1j vendored, generated, submodule or LFS-tracked: matches GitHub Linguist vendor.yml `(^|/)dist/`, so it is not this repository's code
       [gate1/1j] rescued path dist/widget.7f3a91c.js — 1j vendored, generated, submodule or LFS-tracked: matches GitHub Linguist vendor.yml `(^|/)dist/`, so it is not this repository's code
```

Read the two `[gate1/1j]` lines together. `dist/widget.7f3a91c.js` is the **live** asset;
`dist/widget.0c9e142.js` is the **planted decoy**, a stale previous build that is genuinely dead.
The justification is byte-for-byte identical for both: *matches GitHub Linguist vendor.yml
`(^|/)dist/`*.

The fixture's own header says why that is decisive
(`crates/judged-mutants/src/fixtures/m14_checked_in_generated_asset.rs`, lines 23–29):

> **The trap the decoy sets.** `dist/widget.0c9e142.js` is the previous release's bundle, left
> behind, and it really is dead. Both files are minified, hashed, and sitting in the same
> "obviously regenerable" directory. Only the HTML tells them apart. A tool that roots all of
> `dist/` is safe and scores zero decoy recall; a tool that treats `dist/` as junk deletes a live
> production asset.

Gate 1 is the first of those two tools. The fixture anticipated it by name.

What makes the live file live is one line in `public/index.html`:
`<script src="/dist/widget.7f3a91c.js" defer></script>`. Gate 1 never read it. It read the path,
matched a vendored-directory pattern, and refused both files on a property neither of them
uniquely has.

**So Gate 1 rescued m14 because the live artifact happened to look like protected content, not
for any reason connected to why it was live.** The proof is that the identical reason destroyed
the decoy in the same breath: knip's decoy recall goes 4/6 → 3/6. The exchange rate is 1:1 — one
false removal prevented, one genuinely-dead file made permanently invisible — and the layer
cannot tell you which of the two it just did, because it printed the same sentence for both.

This is the outcome the design intends for a cost gate and it is a perfectly good thing for a
cost gate to do. It is not a fact about whether a signal combination can identify dead code, and
reporting it as one would be the most misleading thing this document could say.

Gate 2 clears m14 on its own as well, by finding the filename in the HTML — a rescue that *is*
connected to the liveness. In the full stack Gate 1 runs first and takes the credit (§2.5:
knip's gate1 row shows 1 prevented, its veto row 0, where under `--veto --roots` the veto showed
1). The counterfactual in §2.5's last two rows is what corrects for that ordering artifact.

#### 2.6.3 Gate 1's other measured effects on real analyzers: one no-op and two zeroes

- **vulture:** one claim refused (`__ledger_telemetry_installed__`, under 1j for
  `(^|/)vendors?/`), zero false removals prevented, zero decoys lost. The refused claim is
  neither a live artifact nor a planted decoy, so it changes neither half of the gate.
- **deadcode, cargo-shear:** zero claims refused. Gate 1 is inert on both.

#### 2.6.4 The one unique rescue in the whole sweep is against the `naive` control, and its reason is a homograph

On `naive`, Gate 1 is the only layer that clears m18 — an entry point declared only in a
platform-side manifest. `--veto --roots` leaves it failing; the full stack passes it. The reason,
verbatim from the JSON `refused_claims[]`:

```
"claim":  "vendor/site-packages/zzz_ledger_bootstrap.pth",
"class":  "1e",
"detail": "1e models, weights and checkpoints: the path matches the `.pth` extension;
           1j vendored, generated, submodule or LFS-tracked: matches GitHub Linguist
           vendor.yml `(^|/)vendors?/`, so it is not this repository's code"
```

Class 1e is *models, weights and checkpoints*. `.pth` is PyTorch's checkpoint extension. The file
is a **Python path-configuration file**: CPython's `site` module globs `*.pth` out of every site
directory at interpreter start and executes any line beginning with `import`. That is the entire
reason it is live, and it is the reason the fixture states — *"a `.pth` file is an entry point
with no caller anywhere"*. Gate 1 refused it because its extension collides with an unrelated
format from a different ecosystem.

Right outcome, wrong reason, and the wrongness is not incidental: an operator handed this report
would go and look for a model registry. `docs/evals/2026-08-02-gate1-corpus.md` §2.3 records the
same collision independently.

**Both of Gate 1's rescues in this sweep are therefore of the second kind — the kind that is not
evidence about R1.** One rescued a live file for a reason that also condemned a dead one; the
other rescued a live file for a reason that misidentifies what the file is. Neither read anything
about liveness, and neither could have.

#### 2.6.5 An independent reason not to read a Gate-1 drop as evidence, had there been one

`docs/evals/2026-08-02-gate1-corpus.md` §3 measures Gate 1 on 3,751 tracked files across nine
real repositories: it protects **28.4%** of them, against §6.17's **3.6%** of canonical gitignore
patterns being explicitly irreplaceable, and a 47-row hand check found **17 of 47 protections
wrong**, all from one sub-rule. A layer that refuses roughly a quarter of a repository will
incidentally cover a good fraction of whatever you point it at. Even a large drop attributable to
Gate 1 would need this base rate quoted beside it before it meant anything. Here the drop is
zero, so the point is precautionary — but it is the point that would matter first if the number
ever moves.

### 2.7 The five that survive the whole stack

Verbatim from the `removed live:` lines of the four full-stack runs:

| Class | Mechanism | Analyzer | Artifact still called dead |
|---|---|---|---|
| m02 | module name computed at runtime and passed to `importlib` / `require` | `knip` | `src/transports/websocketTransport.ts` |
| m11 | model field enumerated reflectively by a serializer, never named | `vulture` | `legal_hold_until` |
| m11 | " | `vulture` | `retention_days` |
| m11 | " | `vulture` | `tenant_slug` |
| m12 | symbol bound through a `//go:linkname` alias rather than an import | `deadcode` | `TelemetryFlush` |

Unchanged from the interim round, artifact for artifact. **All three surviving classes are inside
the original fourteen**, which is what makes the catalogue-scope argument in §5.4 non-load-bearing
today.

### 2.8 Coverage of the catalogue

Union of the classes each analyzer actually graded, from the `--json` reports — unchanged from the
interim round:

```
vulture:  m01 m02 m03 m05 m08 m10 m11 m15 m16 m18   (10)
knip:     m02 m10 m14                               (3)
deadcode: m12                                       (1)
shear:    m04 m06 m07 m09 m17 m19                   (6)
union:    18 of 19
read by NO analyzer: m13
```

Twenty gradings over eighteen distinct classes — `vulture` and `knip` both grade m02 and m10.
m13 (a file rescued from a broad ignore rule by an explicit `!` negation, in a PHP/composer
fixture) is read by none of the four. The fixture says so itself, in
`crates/judged-mutants/src/fixtures/m13_gitignore_negation.rs` lines 194–195: *"None of the four
analyzers Judged adapts reads PHP, so every one of them skips this class […]"*

---

## 3. The reading

Stated plainly, and only as far as the evidence goes.

1. **The full rescue stack is worth about half, and Gate 1 is not part of that half.** Ten false
   removals bare, five with all three layers on. The same five with only Gate 2 and the root set
   on. The reduction is Gate 2's and the root set's entirely.

2. **Gate 1's zero is the expected result, not a defect in Gate 1.** §10 E2's nineteen classes
   are built to exercise *reference mechanisms* — a name written down somewhere a reader does not
   look. That is Gate 2's domain by construction. The catalogue contains no `.env`, no
   `terraform.tfstate` and no analyst's `.RData`, so it cannot measure the layer that exists for
   them. `docs/evals/2026-08-02-gate1-corpus.md` §3.1 measures the consequence directly: six of
   Gate 1's sixteen classes never fire even on a nine-repository corpus, and they are the six
   about untracked and ignored state. **Reading §2.5 as "Gate 1 earns nothing" is the same
   category error as reading a tool's silence as a clean result** (§6.20).

3. **The three layers are close to disjoint, and unequal.** Under the full stack the five
   preventions on real analyzers are attributed Gate 2 → 3, root set → 1, Gate 1 → 1. Under
   `--veto --roots`, with Gate 1 removed, the same five are attributed Gate 2 → 4, root set → 1.
   The one that moved is knip's m14: Gate 2 catches it either way, and Gate 1 is credited only
   because it runs first. **Corrected for the ordering, the unique contributions are Gate 2 → 4,
   root set → 1, Gate 1 → 0.** The root set's one is m10, the Django `AppConfig`, rescued via a
   manifest dependency — a rescue that *is* connected to why the class is live, which is the
   contrast §2.6 turns on.

4. **Gate 2 is the layer with a price; Gate 1 has one too, and it is uninformative.** Gate 2 costs
   7 of 33 decoys across the four analyzers. The root set costs none. Gate 1 alone costs one — and
   it is the m14 decoy it destroyed with the same sentence it used to rescue the m14 live file
   (§2.6.2). In the full stack Gate 1's decoy cost is 0, because Gate 2 had already taken that
   decoy.

5. **Zero has been reached only where nothing was risked.** Two SUTs score zero false removals.
   `refusing` scores zero having found 0 of 31 decoys — the suite prints its own warning for this:
   *"this SUT removed nothing at all, so it cleared the gate without demonstrating it can find
   anything."* `cargo-shear` scores zero with 9/9 decoys bare, which is a real result, but it
   grades 6 of 19 classes and makes 9 file-level claims in total; its scope excludes every claim
   that would have been wrong.

6. **The residue is concentrated and it is mechanism-shaped, not tuning-shaped.** Three classes
   survive: a runtime-computed module specifier, a reflectively-enumerated serializer field, and a
   `//go:linkname` alias. Each is a reference that exists in a place the measured layers do not
   read — not a threshold set slightly wrong. Adding a third layer that reads repository text did
   not touch any of them, which is the sharpest available demonstration of that claim.

7. **One of the three is an implementation gap, not a design gap, and it is identifiable now.**
   §5.2 line 373 names `//go:linkname` explicitly among Go's root sources. The root set as built
   does not implement it: `grep -rl linkname crates/judged-core/src/roots` matches no file. m12 is
   therefore a class the design says the root set should rescue and the current root set cannot.
   The same grep shows `.pth`, `#[no_mangle]`, `AndroidManifest.xml`, `//go:embed` and
   `composer.json` are also absent, while `package main`, `cdylib` and `Dockerfile` are present.
   *(That §5.2 lists these is a measurement of the document; that their absence explains m12 and
   m18 is inference.)* Note the sting in m18: Gate 1 rescued it on the naive control by accident
   (§2.6.4) while the layer the design says should rescue it deliberately still cannot.

---

## 4. What changed since the interim version

For a reader who has the interim document open.

| | Interim | Now |
|---|---|---|
| Gate 1 in the binary | no | **yes**, `--gate1` |
| Four analyzers, best configuration | 5 false removals, 3 classes | **5 false removals, 3 classes** |
| Which classes | m02, m11, m12 | **m02, m11, m12** |
| Which artifacts | 5, listed | **the same 5** |
| Decoys under the best configuration | 19/33 | **19/33** |
| `naive`, best configuration | 4 | 3 |
| Status | NOT YET TESTABLE, not triggered | **NOT YET TESTABLE, not triggered** |
| Blocking item | an X-family signal | **an X-family signal** |

The interim document's §6 item 2 was *"Gate 1 — in flight this session; absent from every number
above. Re-run §2.4 verbatim once it lands."* That item is now **discharged, and it came back
null.** It is struck from §7.

Nothing else in the interim document's argument is weakened by the new evidence, and one part of
it is strengthened: §5.3's claim that the measured stack is one-family now has a third layer
supporting it.

---

## 5. What this does not support

This section is the reason to trust the rest of the document. Each item is a reason the numbers
in §2 are weaker than they look.

### 5.1 The catalogue is in-sample, so every rescue number is an upper bound

The 19 mutant classes and the vocabulary of all three rescue layers come out of the same research
document. §10 E2 specifies the classes; §9.3, §6.12, §6.2, §5.2 and §6.11 specify the never-touch
classes, the reference shapes Gate 2 hunts for and the manifest keys the root set parses. The
fixtures plant the constructs the layers were written to find. Nothing in §2 is evidence about a
reference shape nobody wrote down.

The direction of the bias is one-way: an in-sample catalogue can only make rescue look *better*
than it is. A held-out class can fail in a way no fixture models; it cannot be rescued by a rule
that does not exist. So **5 is a floor on the true residual false-removal count for these four
analyzers, not an estimate of it.**

Two out-of-sample measurements now exist — `docs/evals/2026-08-02-out-of-sample-corpus.md` (nine
real repositories: flag rate and root-set yield) and `docs/evals/2026-08-02-gate1-corpus.md` (the
same nine: Gate 1's protection rate). Neither measures false removals, because neither corpus has
ground truth about what is live. They do not close this gap. Nothing does, short of a catalogue
whose classes were documented somewhere other than this research file.

### 5.2 Coverage is 18 of 19

No analyzer reads m13. That is one nineteenth of the catalogue for which §2's analyzer rows
contain no information at all — not a pass, not a fail. It is reported as `[NOT READ by this
SUT]` and subtracted from the denominator, which is the correct handling and is also why the
denominators in §2.4 differ per row.

Two further things must be said about m13 rather than left implied:

- The `naive` control does exercise it, and Gate 2 rescues both its artifacts — m13 is in
  `naive`'s bare and `--roots` failure lists and absent from `--veto`, `--veto --roots` and the
  full stack. So *something* in the stack has an opinion on m13; no measured **analyzer**
  configuration does.
- m13's ecosystem is the one the root set also does not parse (`composer.json`, per §5.2's PHP
  bullet). The single class no analyzer reads is the same class the root set cannot produce roots
  for. "18 of 19" therefore understates the hole slightly.

### 5.3 Four analyzers plus three text-reading layers is still not "no signal combination"

This is the most important limit in this document, and it is the one that decides the status in
§6.

**§2.2, lines 153–158, names the correlation families:**

> **Correlation families** (take MAX within family, SUM across families):
>
> - **Family R — reads repository text:** static reachability, grep veto, manifest roots, name
>   heuristics. All fail on dynamic dispatch.
> - **Family X — observes execution:** production coverage, tombstones, profiler samples,
>   class-load logs. Independent of R.
> - **Family B — build/artifact identity:** linker GC, shipped-symbol presence, declared outputs,
>   regenerate-and-diff.
> - **Family H — history:** VCS age, churn, co-change.

Every accusing signal measured in §2 is Family R. All four analyzers read repository text. Gate 2
is the grep veto, named in R. The root set is manifest roots, named in R. Gate 1 reads paths,
magic bytes and ignore rules — repository text again — and in any case accuses nothing, so it
cannot supply a family (§2.6.1). There is no B signal, no X signal, and no H signal anywhere in
the measured stack. §2.2's own warning applies directly: signals inside one family that share the
dynamism confounder are *"close to one observation reported twice"* — and m02 and m11, two of the
three surviving classes, are exactly dynamic dispatch, the failure mode the section says the whole
R family shares.

**§9.5's family-quorum rule, definition 1 at line 1224, makes this decisive:**

> **Family H can never accuse** […] Consequently "≥2 independent families accuse" means ≥2 of
> **{B, R, X}**, and since X requires production runtime evidence, *the only two-family
> combinations that exist are {B,R}, {B,X}, {R,X}*.

A one-family stack cannot satisfy a two-family quorum. And §9.5's tier-ceiling modifiers, line
1217, close it from the other side:

> - no runtime evidence source at all → Tier 0 unreachable

**Consequence: not one of the thirty-six configurations in §2.4 could have produced a Tier-0
action even if it had scored zero false removals.** The suite grades *claims*. Tier 0 acts on
claims that have additionally cleared Gate 1, all six Gate-3 conjuncts, a two-family quorum, a
ladder-rung check, a stability window of 20 runs or 90 days, and a rate limit. §2 measures the
error rate of the accusation, which bounds nothing about the error rate of the action except from
above.

**A fair test of the pre-commitment would have to include, at minimum:**

- at least one **X-family** signal — ingested production coverage, a tombstone, a class-load log
  or profiler samples (§3.2) — because §9.5 makes Tier 0 unreachable without one, and because X is
  the only family independent of the confounder that produced m02 and m11;
- at least one **B-family** signal — regenerate-and-diff, linker GC, or shipped-symbol presence —
  since §9.6 says Tier 0 in practice reaches *"build artifacts, OS junk, logs, test output, and
  committed generator output,"* none of which any of the four analyzers measured here even looks
  at;
- **Gate 3**, in particular 3f, which §9.3 says *"No ban count overrides"*, and which §6.24 line
  815 states independently: *"no auto-act tier may include any candidate whose type is
  serializable, whose name can appear in a queue payload, or whose symbol is exported across an
  ABI boundary — regardless of ban count."* m11 is a serializer field. On a reading of the design,
  3f vetoes it outright — **inference, not measurement; Gate 3 does not exist.**
- §3.7's positive control on every evidence artifact, and §9.5's `execution_successful` /
  `positive_control_passed` / expiry preconditions, without which a family does not accuse at all.

Gate 1 now exists and is enforced, so one line of the previous version of this list is struck.
Its arrival changed nothing about this section's conclusion, for the reason §2.6.1 gives: Gate 1
is a cost gate, not a signal, and adding it to a one-family stack leaves a one-family stack.

### 5.4 The catalogue is 19 classes; the pre-commitment names 14. It binds against 19.

The pre-commitment says "all 14" in all three places it appears (lines 1418, 1451, 43). The
catalogue it is supposed to gate has 19 classes. That has to be settled explicitly, because the
two readings are not equivalent and one of them is convenient.

**The argument for 14.** A pre-commitment that can be widened after the fact is not a
pre-commitment. Broadening the bar from 14 to 19 after results are in is structurally the same
move as narrowing it would be, and the document's own instruction is that the tier is deleted
*rather than tuned* — tuning the gate's scope is still tuning. Classes 15–19 are also, by their
own description at line 1408, a *"structurally different failure"*; one could argue they belong to
a different gate.

**The argument for 19, which wins.**

1. **The widening is not post-hoc.** It is at line 1416, two lines *above* the pre-commitment, in
   the same section, written before any measurement existed: *"Ship this as the tool's own test
   suite and **gate releases on zero failures**. **The 19 classes are a floor, not a ceiling** —
   the original text called 14 a 'minimum catalogue' and that framing is correct."* The release
   gate is stated over the whole suite. A release gate over 19 and an auto-act gate over 14 in
   adjacent sentences, with 14 explicitly labelled a *minimum*, reads as a numeral left behind by
   an edit, not as a scoped exemption.
2. **Reading it as 14 makes the design contradict itself.** §6.24 line 815 forbids any auto-act
   tier from including a serializable type, a queue-payload name, or an ABI export — *regardless
   of ban count*. Classes 15, 16, 18 and 19 are precisely those shapes. Under the 14-reading, a
   tier could clear its gate on a catalogue that omits every case §6.24 independently forbids it
   from touching. The gate would certify something the hazard catalogue prohibits.
3. **Line 1408 argues for 19, not against it.** It says classes 15–19 are *"not exercised by any
   of the first fourteen."* If they were exercised by the first fourteen, scoping the gate to 14
   would be harmless. Because they are not, scoping to 14 excludes exactly the failures nothing
   else in the catalogue catches — the opposite of what a minimum catalogue is for.
4. **It is the inconvenient reading, and that is a reason to prefer it here.** The 14-reading is
   the one that keeps the tier alive more cheaply. §11's own "claims to stop propagating" list is a
   record of what happens when the convenient reading gets adopted and repeated.

**And it changes nothing today.** All five false removals surviving the full stack are in classes
m02, m11 and m12 — inside the original fourteen (§2.7). The determination in §6 is identical under
both readings. This is settled now so that it cannot be litigated later, when it will matter.

### 5.5 Five more things §2 does not say

- **"5 false removals" is not a precision figure and must never be printed as one.** It is 5 out
  of 58 claims judged across four analyzers on a 19-class synthetic catalogue. §10's headline
  metrics ask for *"recall at zero observed false positives"* and *"precision with a
  Clopper–Pearson lower bound per tier"*. No tier was assigned to anything here, so no per-tier
  interval is computable, and an interval over n = 58 in-sample synthetic claims would be a number
  that looks like a measurement and is not one. §11's "claims to stop propagating" (line 1512) is
  explicit about the failure mode: *"Never ship an uncalibrated number that looks like a
  probability."*
- **`shear`'s zero is not "cargo-shear has no false positives."** Stating it that way would
  reproduce, in this repository, the exact shape of the claim §11 line 1511 exists to kill
  (*"Knip deleted 300k lines at Vercel with zero false positives"*). cargo-shear grades 6 of 19
  classes, makes 9 claims, and the classes it cannot read include every one it fails.
- **`refusing`'s zero is the null result, and the gate does not distinguish it.** The suite gates
  on false removals *and on nothing else*, and a tool that answers nothing scores perfectly. Its
  0/31 decoy line is the only thing separating it from a clean pass. Any future "zero false
  removals" claim about any configuration must be quoted together with its decoy recall or it is
  not a result.
- **Gate 1's zero is likewise not a measurement of Gate 1.** §3 item 2 and §2.6 both say this and
  it is repeated here because it is the sentence most likely to be quoted out of context. E2 does
  not test the classes Gate 1 exists for; six of its sixteen classes have never fired in any
  measurement this project has run.
- **A single measurement round on one host is not a stability claim.** Each configuration was run
  twice for the five full-stack rows (text and `--json`) and agreed, which is a consistency check,
  not a repeatability measurement across hosts, toolchain versions, or npx-resolved knip builds.
  The binary was also built from a working tree carrying other agents' in-flight edits; §2.3 is
  the control for that, and it is a comparison against one prior round, not a bound.

---

## 6. The determination

### Status: **NOT YET TESTABLE.** The pre-commitment is **not triggered** — and the auto-act tier is **presumed absent** until it is.

Both halves are load-bearing; neither may be quoted without the other. This is the same status the
interim version reached, now reached with Gate 1 built, enforced, and measured.

**Why not triggered.** The pre-commitment's antecedent, verbatim, is *"no signal combination clears
all 14 at zero false removals"* — read as the whole catalogue, per §5.4. Establishing it requires
evidence about signal combinations. What §2 contains is four analyzers and three rescue layers,
every one of the seven inside Family R or accusing nothing at all (§5.3, §2.6.1). §9.5's quorum
rule says a combination means ≥2 of {B, R, X}; not one such combination exists in this project yet,
and §9.5 line 1217 makes Tier 0 formally unreachable without an X signal, so no configuration
measured here was ever a candidate. Four points inside one family do not establish a claim about
all combinations across three.

**Why "presumed absent" rather than "still open".** The burden of proof sits on the tier, not on
its critics. §0 item 6 records that the design ships Tier 0 *in direct tension with* the measured
fusion ceiling; §0 item 15 says *"if it can't, ship with no auto-act tier and say so."* The correct
posture while the question is untestable is the one that costs nothing if the tier eventually
clears and costs everything if it does not. Concretely: no work should depend on Tier 0 existing,
nothing should ship that auto-acts, and the ledger, stability window, quarantine reaping and rate
limiter — which §11 R1 names as the downstream that *"assumes the answer is yes"* — are building
on an unresolved premise and should be written to survive its deletion.

**Why Gate 1's arrival did not move the status in either direction.** It could have moved it two
ways and did neither. It did not lower the count, so there is no cheaper-looking stack to argue
from. And it could not have counted even if it had, because Gate 1 refuses on irreversibility
rather than on usefulness: it makes a wrong answer cheaper, not more correct, and the two rescues
it did produce are both of the kind that reads nothing about liveness (§2.6.2, §2.6.4). The
determination rests today on exactly the evidence it rested on before, plus one discharged item.

**Why the fact that five false removals remain is not itself a trigger.** They fall in three
classes, and all three are mechanisms the design claims to handle at layers that do not exist yet:
m11 by Gate 3f and §6.24's serializer veto, m02 by §9.5's dynamic-construct tier ceiling, m12 by a
§5.2 root source the implementation has not written. Calling the pre-commitment triggered on
evidence gathered before those layers exist would be reading a partial implementation as a verdict
on a design.

**And the tripwire that keeps this honest.** The move this determination must not become cover for
is the one the pre-commitment forbids by name. Adding Gate 3 because §9.3 specifies it is
*implementation* — as adding Gate 1 was. Widening a needle set, adding a root rule, adjusting a
threshold, adding a Gate 1 class, or editing a fixture **because m02, m11 or m12 failed** is
*tuning*, and the pre-commitment already answers it: the tier is deleted rather than tuned. The
line is whether a change alters the specification or implements more of it. Every change made
between this document and the final call should be classifiable on that line, and a change that is
not classifiable is tuning.

### What would reverse this determination

**To TRIGGERED — delete the auto-act tier from the design.** Any one of:

1. **The direct test.** A configuration with at least one X-family and one B-family signal, with
   Gates 0–3 all enforced and the §9.5 quorum applied, run over the whole catalogue, still produces
   ≥1 false removal on any class. That satisfies the antecedent for the strongest combination the
   design contains, and no weaker one can rescue it.
2. **The enumeration shortcut, which is cheap and available now, and which Gate 1's arrival has
   made cheaper.** Take m02, m11 and m12 and walk every specified mechanism — Gate 1a–1p (now
   implemented and measurable rather than hypothetical), Gate 2a–2f, Gate 3a–3f, every §5.2 root
   source, every §9.5 tier-ceiling modifier — and show that for at least one of them, none applies.
   If a class in the catalogue can be blocked by no mechanism the design contains, then no signal
   combination clears the catalogue, and the antecedent is satisfied without waiting for any
   further implementation. §2.5 establishes only a part of that walk: Gate 1's sixteen classes were
   asked about m02, m11 and m12 and none fired. That is one layer of the design, not the design
   — Gate 3's conjuncts, §6.24's serializer veto and §9.5's dynamic-construct ceiling are all
   unwritten, and each is specified to bear on one of these three classes. The walk is not
   complete until they exist and also fail.
3. **A false removal that survives with an X signal present.** Any class false-removed by a
   configuration carrying real runtime evidence is the sharpest possible form of (1), because X is
   the family the design leans on to break the R-family confounder.

**To CLEARED — the tier survives, provisionally.** All of:

1. Zero false removals across all 19 classes, by a configuration satisfying §9.6's Tier-0 criteria
   in full, including the two-family quorum, every Gate-3 conjunct, `ladder_rung ≥ R2`, and the
   positive controls;
2. with m13 actually read, so coverage is 19 of 19 rather than 18;
3. **and** the same at zero on a catalogue whose classes were documented outside this research
   file — because §5.1 makes every in-sample rescue number an upper bound, and a gate cleared only
   in-sample is a gate cleared against its own author.

**What would reverse it and must not count.** A drop in the false-removal count produced by editing
a fixture, weakening or deleting an assertion, narrowing the catalogue, changing a needle strategy
in response to a specific failure, adding a Gate 1 class or a root rule aimed at a specific mutant.
Each of those changes the measuring instrument. If the number moves and the instrument moved with
it, the number is not evidence.

**And one that would count for less than it appears to.** A drop produced by a *cost* gate — Gate
1, or anything else refusing on irreversibility rather than on usefulness — is not evidence about
signal combinations at all, whatever its size. If such a drop is ever reported, §2.6's test is the
one to apply: read the justification the gate printed, and ask whether it would have printed the
same sentence about a genuinely dead file. If it would, the rescue is a coincidence of shape and
the number means nothing about R1. That test is not hypothetical; it is how the two rescues in
this round were classified, and both failed it.

---

## 7. What remains before a final call

Ordered by how much each would change the picture. Item 2 of the previous version — *"Gate 1
(§9.3 1a–1p), in flight this session"* — is struck: it landed, it was measured, it came back null,
and §2.6 records the result.

1. **An X-family signal, at all.** Nothing in the project observes execution. Until one exists,
   §9.5 line 1217 makes Tier 0 unreachable by construction and R1 is not answerable in either
   direction. **This is the single blocking item, and it is now the only one that was ever
   blocking.**
2. **Gate 3, and 3f specifically.** 3f is the conjunct §9.3 says no ban count overrides, and it is
   the mechanism that would speak to m11 and to classes 15–19 as a group. It is now the only
   specified gate that does not exist.
3. **A B-family signal — regenerate-and-diff.** §9.6 puts Tier 0's actual volume in build artifacts
   and generated output, which no analyzer measured here reads.
4. **Root-set coverage for the sources §5.2 names and the implementation lacks:** `//go:linkname`
   and `//export` (Go), `.pth` and `sitecustomize.py` (Python), `#[no_mangle]` / `#[used]` /
   `#[ctor]` (Rust), `AndroidManifest.xml` (JVM), `composer.json` (PHP). m12 is a direct
   consequence; m18 and m13 are probable ones. §2.6.4 sharpens this: m18's `.pth` is currently
   "rescued" only by an extension collision inside a cost gate, on one control SUT.
5. **A reader for m13's ecosystem**, so the catalogue is measured at 19 of 19.
6. **Gate 1 measured on the population it exists for.** Six of its sixteen classes have never
   fired in any measurement this project has run, and they are the six about untracked and ignored
   state (`docs/evals/2026-08-02-gate1-corpus.md` §3.1, §7). This does not block R1 — Gate 1 is not
   a signal — but it blocks any claim about whether the *cost* half of the design works.
7. **An out-of-sample catalogue** — classes derived from failures documented outside this research
   file. Without it §5.1 caps what any clean run can mean.
8. **§10's headline metrics as specified:** recall at zero observed false positives, per-tier
   precision with a Clopper–Pearson lower bound, and flag rate. None is computable from §2, and
   until a tier is assigned to anything, none is computable at all.

---

## Reproducing this document

```sh
export PATH="/Users/neo/.blackhole/Judged/2026-08-02b/.venv/bin:$HOME/go/bin:/Users/neo/.blackhole/Judged/2026-08-01-tools/bin:$PATH"
export CARGO_TARGET_DIR=/private/tmp/claude-501/judged-r1det2
cargo build --release -p judged-cli
cd /Users/neo/Developer/Projects/Judged
J="$CARGO_TARGET_DIR/release/judged"          # not on PATH; invoke by path
for sut in naive refusing vulture knip deadcode shear; do
  for flags in "" "--gate1" "--veto" "--roots" "--veto --roots" "--gate1 --veto --roots"; do
    "$J" mutants --sut "$sut" ${=flags}       # zsh word-splitting
    echo "exit=$?"                            # read from $?, never through a pipe
  done
done
```

Gate 1's per-claim evidence — the material §2.6 rests on — is in the JSON at
`mutants[].gate1.refused_claims[]`, whose `class` names the §9.3 class that fired and whose
`detail` quotes the rule verbatim. The counterfactual in §2.5's last two rows is the difference
between `--veto --roots` and `--gate1 --veto --roots`, and it is the only honest way to read Gate
1's contribution, because the stack credits a rescue to whichever layer ran first and Gate 1 runs
first.

Related, and all still current: `docs/evals/2026-08-01-four-analyzers-e2.md` (the bare rows),
`docs/evals/2026-08-02-gate2-veto.md` (Gate 2 alone),
`docs/evals/2026-08-02-gate1-corpus.md` (Gate 1 alone, and on nine real repositories),
`docs/evals/2026-08-02-out-of-sample-corpus.md` (nine real repositories).
`docs/evals/2026-08-01-vulture-e2-baseline.md` is superseded and must not be quoted.
