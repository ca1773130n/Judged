# The ban ledger and the tier model — building the thing that authorizes deletion

**Date:** 2026-08-02 · **Status:** decided, implemented in the same change · **Supersedes:** nothing

Everything shipped so far only ever *removes* claims. Gate 1 refuses, Gate 2 vetoes, the root set
rescues, coverage rescues, Gate 3f refuses. Nothing in the codebase computes a ban, and nothing
assigns a tier — which is why the previous handoff had to be corrected twice about what blocks a
quorum. §9.5's question *"which two families accuse"* is not one the code can answer for any family.

This is the machinery that answers it. It is also, unavoidably, the machinery whose output
authorizes deletion, so the choices are written down before the code rather than after.

---

## 1. What this is not

**It does not delete, quarantine, or open a PR.** It computes a tier and the reasons for it. Every
action §9.6 attaches to a tier stays unimplemented, and the CLI gains a way to *see* the assignment
and nothing that acts on it.

That is not caution for its own sake. §9.6's Tier 0 criteria include a stability window of *"20 runs
or 90 days, whichever is longer"*, a per-run rate limit, and `ladder_rung ≥ R2` with a performed and
verified §8.2 promotion. None of those exist. A tier computed without them is a number, not a
licence, and the gap between the two is exactly what this document exists to keep visible.

---

## 2. Store evidence, never verdicts

§9.4 states the governing principle and says no surveyed tool follows it:

> **STORE EVIDENCE, NEVER VERDICTS. Re-derive every run.**

So the ledger holds `Evidence` — a family, the §9.5 signal that produced it, its ban weight, and the
health flags §9.5 definition 1 requires — and the tier is a *function* of the ledger, recomputed on
every call. There is no cached tier and no field anybody can set. A stored verdict is a claim about
a tree state that has since changed, which §6.21 records as the OpenRewrite #321 failure.

The arithmetic is §9.5's, not ours: **MAX within family, SUM across families**, because static
reachability and test coverage share the repo-dynamism confounder and multiplying correlated
evidence produces the documented overconfidence pathology.

---

## 3. An unevaluable criterion demotes, and is named

This is the decision that matters, and it is §6.20 applied to our own scoreboard.

§9.6's Tier 0 has fourteen conjuncts. This codebase can evaluate a few of them. The tempting
implementation treats the rest as satisfied — nobody said they failed — and that is precisely the
inversion this project exists to prevent: a criterion nobody checked would be indistinguishable from
one that passed, and the tier would climb on the strength of what was never measured.

So every criterion is one of three things: **satisfied**, **failed**, or **not evaluable here** —
and the last two both demote, identically. The assignment carries the list, so the distance between
this build and §9.6 is countable rather than argued.

A consequence worth stating plainly: **no candidate can reach Tier 0 or Tier 1 in this build, and
that is a property of the code rather than of any repository it is pointed at.** The stability
window alone guarantees it. Anyone reading a Tier 2 result should understand it as "capped", not as
"scored".

---

## 4. Thresholds, and one interpretation flagged as such

§9.5 gives the prior as log₁₀-odds(dead) = **−0.95** (P ≈ 0.10), and §9.6 requires *"accumulated
≥ 3.95 bans"* for Tier 0 and *"≥ 2.65"* for Tier 1. Read as log₁₀-odds increments summed onto that
prior, those land at posterior log-odds 3.0 (P ≈ 99.9%) and 1.7 (P ≈ 98%) — round numbers on the
odds scale, which is what makes the reading credible.

**That is an inference, not a quotation.** §9.6 says "accumulated ≥ 3.95 bans" and does not say the
prior is included. The implementation therefore compares the *sum of bans* against the literal
thresholds and exposes the prior separately, so nothing depends on the interpretation being right.
If a later reading says the prior should be added, one line changes and every number in the report
moves visibly.

---

## 5. The pre-commitment

Written before the first run, so it cannot be chosen afterwards:

- The project has **one** family capable of accusing — R, through its analyzers. B does not exist.
  X exists but shipped as test coverage, pinned at 0.0 by §9.5's resolved contradiction and unable
  to accuse at any weight. H can never accuse by §9.5 definition 1.
- Therefore **the first run must cap every candidate at Tier 2**, on every repository, for two
  independent reasons: the family quorum cannot be met, and R's maximum row is +1.5, below Tier 1's
  2.65 on its own.
- If a candidate reaches Tier 1 or Tier 0, the implementation is wrong — a family is accusing that
  should not, or a criterion that cannot be evaluated is being treated as satisfied. The correct
  response is to fix it, not to publish the run.
- If every candidate caps at Tier 2 **and** the assignment names the quorum failure and the
  unevaluable conjuncts, that is the honest product the determination already describes — report
  plus quarantine — now derived by arithmetic instead of asserted in prose.

---

## 6. What this leaves undone

- **Every action.** Quarantine, the soak, the PR, the reaping. §9.6 attaches them to tiers; none is
  implemented and none should be until a tier can legitimately exceed 2.
- **The stability window and the deadness invariant** (§9.5 definition 3) need a store of prior runs
  keyed by tree SHA. There is none, so the criterion is `NotEvaluable` and demotes.
- **`scanned_universe_ratio`, `ladder_rung`, `has_external_effector`, `is_distributable`** and the
  rate limit are all §9.6 criteria this build cannot compute. Same treatment.
- **Gate 3a–3e.** 3f exists; the rest are directory conjuncts and the family quorum, and 3e's answer
  now comes from this ledger.
- **A second accusing family**, which is the only thing that changes the cap. That is Family B, or an
  X signal sourced from production rather than from a test run.
