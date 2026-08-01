> **SUPERSEDED, same day, by
> [`2026-08-01-four-analyzers-e2.md`](2026-08-01-four-analyzers-e2.md) — read that one.**
> It grades vulture, knip, deadcode and cargo-shear, and it is the only current source for
> any number about any of them, vulture included.
>
> **This file is a frozen record, not a current result. Do not quote a figure from it.**
> Everything below was produced by an earlier runner that handed every analyzer all nineteen
> classes, including the ones it cannot read. The runner now declares each analyzer's
> ecosystems and skips the rest, so the denominators used here no longer exist — which is why
> lines like "0 passed, 19 failed" and "0 of 31" do not reproduce today. The decoys have also
> since been given symbols, which vulture, a symbol-level tool, does find.
>
> One number survives unchanged, counted the same way in both runs: **6 false removals, on
> m01, m10, m11 and m16.** The reasoning behind those six, and the per-class notes on what
> vulture named inside live files, is why the file is kept — it is the record of the first run
> against a shipped analyzer and the only place the vulture rows are walked one at a time.

# Vulture against the E2 catalogue — first real measurement

**Date:** 2026-08-01 · **Tool:** vulture 2.16 · **Result: GATE FAILED — 6 false removals across 4 of the 12 classes vulture can actually read** (7 of the 19 are Rust/Go/TypeScript, which it never opened).

Until now E2 had only graded two system-under-tests we wrote ourselves, so it bounded the
harness and nothing else. This is the first run against a shipped analyzer.

## What was run

```
$ vulture --version
vulture 2.16

$ judged mutants --sut vulture
```

Vulture was installed into a throwaway `uv` virtualenv outside the repository and put on
`PATH`; `--sut vulture` invokes it at its own defaults — no `--min-confidence`, no
whitelist, no exclusions — because §4.1's 6% precision figure is a measurement of vulture
as shipped. The venv was deleted after the run. Nothing about vulture is vendored here.

Verbatim final lines of the report:

```
19 classes: 0 passed, 19 failed
decoy recall: 0 of 31 genuinely-dead files found
false removals: 6 — GATE FAILED (§11 R1: if this is not zero, the auto-act tier is deleted from the design rather than tuned)
classes with false removals: m01, m10, m11, m16
```

Exit code 1.

## Read this before the table: vulture is Python-only

**Ten of the nineteen classes produced no vulture output at all, and on nine of them vulture
read not one line of source.** Those classes score zero false removals because their live
artifact is Rust, Go, TypeScript, PHP, a shell script or an ini file — none of which vulture
parses. A green cell for m17 (Rust link-time registry) or m12 (Go `//go:linkname`) records
that vulture could not read the file, not that it reasoned about the mechanism.

This is §9.2's capability envelope and §6.20's rule that no data is not the same as zero
findings, applied to the grade rather than to the tool. The score below is **not** an
19-class score. It is a 7-class score with twelve abstentions, and the honest denominator is
in the summary at the bottom.

## Per-class result

`.py` counts files vulture actually parsed. "Graded FR" is false removals the suite recorded.

| Class | Ecosystem | `.py` | Graded FR | What happened |
|---|---|---|---|---|
| m01 yaml string ref | python | 6 | **1** | Claimed `DunningConfig`, the Django `AppConfig` named only in `apps.yaml`. Also claimed `name`, `verbose_name` and `ready` inside the same live file. §4.1's Django mode. |
| m02 dynamic import | polyglot | 5 | 0 | **Silent by accident.** `RedisBackend` escaped only because `build()` on line 17 of the same file instantiates it. Vulture did claim `ping`, a method of the live class. The TypeScript half was never read. |
| m03 plugin dir scan | python | 6 | 0 | **Silent by grading, not by judgement.** Vulture claimed `EXTENSION` and `emit` — every symbol the live plugin file contains. Ground truth names the *module* `pluginhost.plugins.tsvwriter`, which vulture cannot emit, so nothing matched. Acting on this run would have emptied the live plugin. |
| m04 human CLI subcommand | rust | 0 | 0 | Never looked. No Python in the repository. |
| m05 error path only | python | 6 | 0 | **Genuine pass, wrong mechanism.** `from ledger.recovery import quarantine_partial_write` sits inside the `except` branch and is a static AST reference, so vulture sees the name as used. The class defeats coverage- and execution-derived signals; it does not probe an AST tool. |
| m06 concurrency helper | rust | 0 | 0 | Never looked. |
| m07 guard clause | rust | 0 | 0 | Never looked. |
| m08 CI manifest ref | polyglot | 2 | 0 | Parsed two Python files and emitted nothing. Both live artifacts (`verify_release.sh`, `uwsgi.ini`) and both decoys are non-Python; nothing gradeable was in a language it reads. |
| m09 README executed block | rust | 0 | 0 | Never looked. |
| m10 framework convention | polyglot | 4 | **1** | Claimed `ReportingConfig`, plus `name`, `verbose_name` and `ready` in the same live file. §4.1's Django mode again. The Jest `__mocks__/redis.js` half was never read. |
| m11 reflective field | python | 5 | **3** | Claimed all three Pydantic `BaseModel` fields — `tenant_slug`, `retention_days`, `legal_hold_until` — enumerated at runtime by `type(model).model_fields`. This is precisely §4.1's 102 FastAPI false positives and Django model-field issue #110. Worst class in the run. |
| m12 linkname alias | go | 0 | 0 | Never looked. |
| m13 gitignore negation | polyglot | 0 | 0 | Never looked. |
| m14 checked-in generated asset | typescript | 0 | 0 | Never looked. |
| m15 enqueued job payload | python | 5 | 0 | **Silent by accident.** `RebuildInvoiceIndex` escaped only because `app.register_task(RebuildInvoiceIndex())` appears on line 11 of the same file. Vulture claimed `run` — the task body — and `name`, the routing key the queued payload matches on. Deleting either breaks the worker. |
| m16 persisted serialized blob | python | 5 | **1** | Claimed `RateSnapshot`, whose only remaining consumer is a pickle on disk. Exactly what OpenRewrite's `serialVersionUID` bail-out protects against. |
| m17 link-time registry | rust | 0 | 0 | Never looked. |
| m18 platform-side manifest | polyglot | 4 | 0 | The graded live symbol `OtaUpdateReceiver` is Kotlin and was never read. Vulture did claim `__ledger_telemetry_installed__` inside `ledger_startup_hook.py`, a live file executed at interpreter startup by a `.pth`. |
| m19 ABI consumer export | rust | 0 | 0 | Never looked. |

## The honest denominator

Seven classes put a Python name in front of vulture that it was capable of emitting —
m01, m02, m05, m10, m11, m15, m16. (m03's live artifact is a module path, which vulture
structurally cannot name; m18's is Kotlin.)

**On those seven, vulture false-removed on four: m01, m10, m11, m16.** Of the three it
survived, two (m02, m15) were saved by an incidental self-reference in the same file, not by
any reasoning about the liveness mechanism. Remove the one line that happens to mention the
symbol and both become false removals. Only m05 is a pass on the merits, and it is a pass
against a mechanism aimed at coverage tools rather than at AST tools.

## The grade under-reports vulture

The adapter maps vulture's findings to symbol claims only and never to file claims, because
vulture reports names and never names a file; `MAPPING_DECISION` states that this makes the
count a lower bound. That is not theoretical here.

**A vulture finding lands inside a file the ground truth declares LIVE in seven classes:
m01, m02, m03, m10, m15, m16, m18.** The grade catches three of them (m01, m10, m16). In
m02, m03, m15 and m18 the suite records zero false removals while vulture named something
inside the live artifact — a class method, an entire plugin module's contents, a Celery
task's `run` body, a startup hook's attribute. A human deleting what vulture named would
have broken all four.

So: 6 graded false removals, and four further classes where the harm is real and invisible
to the grade.

## Decoy recall: 0 of 31 is a mapping artifact, and this cuts in vulture's favour

The suite reports zero decoys found, and read naively that says vulture found nothing true.
It did not. Decoys are graded as **files**, and vulture never claims a file. Counting
instead the decoy files vulture named at least one dead symbol in: **11 of 31 — which is
all 11 Python decoys and none of the 20 non-Python ones.** On decoys inside its language
vulture was perfect. The zero is the adapter's mapping, not vulture's blindness, and a
reader must not take it as evidence of incompetence.

Note the consequence for the pass/fail column: passing a class requires zero false removals
*and* full decoy recall, so a tool that structurally cannot claim a file can never pass any
class in this suite. "0 passed, 19 failed" is partly that. The number that matters is the
false-removal count.

## What this does and does not tell us about §11 R1

§11 R1 makes the existence of an auto-act tier the highest-risk open question and
pre-commits the consequence: *"If no signal combination clears all 14 mutant classes at zero
false removals, the honest product is report+quarantine and the auto-act tier is deleted,
not tuned."* (The catalogue has since grown to 19; classes 15–19 cover §6.24 and the
under-served ecosystems.)

**What this run establishes.** Vulture alone is not a signal an auto-act tier can be built
on. It is not close. Four of the seven classes it could read produced a false removal, and
two of the three survivors were luck. This is consistent with §4.1's measurement of 44 true
positives against 644 false positives across nine popular repositories (~6% precision),
including 59 on httpx, which contains zero dead items. E2 reproduces that failure shape on
purpose-built fixtures in seconds, which is the cheap-and-early property §10 wanted from it.

**What this run does not establish.** It does not resolve R1. R1 asks about *signal
combinations*, and one tool is not a combination. Twelve classes were never graded against
vulture in any meaningful sense — nine because vulture read no source at all, plus m08, m03
and m18 for the reasons in the table — so this run says nothing about whether those
mechanisms are catchable. Nothing here bounds knip, ts-prune, Go `deadcode` or Periphery,
and a multi-tool intersection could behave differently in either direction.

**What it does establish about the method.** E2 discriminates. It failed a real tool for
real reasons traceable to documented upstream issues, having already failed the naive
control (20 false removals across 12 classes) and passed the refusing control. A suite that
green-lit vulture would have been theatre.

**The next measurement that would move R1** is a second analyzer in a different ecosystem —
knip or ts-prune against the TypeScript classes, Go `deadcode` against m12 — because the
twelve abstentions above are where an auto-act tier would actually have to earn its keep,
and no data has been collected on them yet.

## Reproducing this

```bash
uv venv && uv pip install vulture          # outside the repository
PATH="$PWD/.venv/bin:$PATH" judged mutants --sut vulture
```

The `--sut vulture` path — the adapter, the command SUT and the CLI option — was built in
the same session as this run, so it would be wrong to claim the repository was untouched.
What matters is narrower and is true: the nineteen fixtures predate it unchanged, and no
fixture, adapter or grading rule was adjusted **after** seeing the score. The adapter was
written against Vulture's documented output format and unit-tested on captured text before
the tool was ever installed.
