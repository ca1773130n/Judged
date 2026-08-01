//! Gate 2 — the reference veto (§9.3).
//!
//! Every analyzer in this repository is a **bounded accuser**, never an oracle
//! (§9.1). Gate 2 is the layer that makes that safe: it can only ever *rescue* a
//! candidate, never nominate one. A veto is absorbing — no later evidence
//! overrides it — and that asymmetry is the whole design, because the two error
//! directions are not comparable (§1.3). Missing dead code costs disk; deleting
//! live code costs an incident.
//!
//! §0 ranks the whole-repo literal veto the second-cheapest high-value safety
//! mechanism in the research, behind only positive controls. Meta ships it as
//! BigGrep and states the trade explicitly: *"This approach can cause false
//! negatives, but avoids false positives. When automating the removal of dead
//! code, those are a more serious problem."*
//!
//! One rule outranks everything else here, and it is the inversion Meta hit in
//! production: **a truncated, timed-out, or errored search is a HIT, never an
//! absence.** A search that did not finish has found nothing *because it did not
//! look*, and reading that as "no references" converts the safety net into the
//! deletion trigger (§6.20).

pub mod literal;
pub mod reachability;
pub mod recency;
