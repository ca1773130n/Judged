//! The ban ledger and the tier model (§9.4, §9.5, §9.6).
//!
//! The tests that matter here are the ones asserting a candidate **cannot** be
//! promoted. This is the only module in the workspace whose output could ever
//! authorize a deletion, so a bug that promotes is categorically worse than one
//! that demotes, and every arithmetic test below is paired with a test that a
//! plausible-looking ledger still fails to clear.

use judged_core::ledger::{
    assign, Evidence, Family, GateState, Ledger, Outcome, Tier, ACCUSE_FLOOR, TIER0_BANS,
    TIER1_BANS,
};

/// Everything the gates can currently say, at its most permissive.
fn clean_gates() -> GateState {
    GateState {
        gates_0_to_2: Outcome::Satisfied,
        gate_3f: Outcome::Satisfied,
    }
}

/// §9.5: **MAX within family, SUM across families.**
///
/// The correlated-evidence pathology in arithmetic form. Three R signals on one
/// candidate are close to one observation reported three times, so the family
/// contributes its strongest statement and not their sum.
#[test]
fn bans_take_the_max_within_a_family_and_sum_across_families() {
    let mut ledger = Ledger::new();
    ledger
        .record(Evidence::new(Family::R, "dynamic language", 0.4))
        .record(Evidence::new(Family::R, "named in no manifest", 0.3))
        .record(Evidence::new(
            Family::R,
            "zero textual occurrences, complete non-truncated search",
            1.0,
        ))
        .record(Evidence::new(
            Family::B,
            "linker-GC'd from every link target",
            1.8,
        ));

    assert_eq!(
        ledger.family_max(Family::R),
        Some(1.0),
        "the strongest R row, not 0.4 + 0.3 + 1.0"
    );
    assert!((ledger.total_bans() - 2.8).abs() < 1e-9, "1.0 + 1.8");
}

/// §9.5 definition 1, both halves: the floor, and the health of every artifact
/// contributing to the maximum.
#[test]
fn a_family_accuses_only_above_the_floor_and_only_on_healthy_evidence() {
    let mut weak = Ledger::new();
    weak.record(Evidence::new(Family::B, "name-pattern-only", 0.1))
        .record(Evidence::new(Family::R, "named in no manifest", 0.3));
    assert!(
        !weak.accuses(Family::B) && !weak.accuses(Family::R),
        "§9.5: a family whose maximum comes only from +0.1 or +0.3 evidence ABSTAINS"
    );

    let mut sick = Ledger::new();
    sick.record(
        Evidence::new(Family::R, "compiler-index-backed, zero dynamism", 1.5).control_failed(),
    );
    assert!(
        !sick.accuses(Family::R),
        "an artifact that failed its positive control looks exactly like a clean one (§3.7)"
    );
    assert_eq!(sick.family_max(Family::R), None);

    let mut healthy = Ledger::new();
    healthy.record(Evidence::new(
        Family::R,
        "tree-sitter / heuristic parse only",
        0.5,
    ));
    assert!(
        healthy.accuses(Family::R),
        "exactly at the floor of +{ACCUSE_FLOOR} is an accusation"
    );
}

/// §9.5 definition 1: **"Family H can never accuse."**
///
/// §6.18 measured age as anti-predictive — >4y untouched gives 1.4% subsequent
/// deletion against a 6.4% base rate — so its positive rows are unvalidated and
/// it may only subtract.
#[test]
fn family_h_never_accuses_however_large_its_bans() {
    let mut ledger = Ledger::new();
    ledger.record(Evidence::new(
        Family::H,
        "single commit ever AND <2y old",
        0.5,
    ));

    assert!(!ledger.accuses(Family::H));
    assert!(ledger.accusing().is_empty());
}

/// H is capped at ±0.6 and must still be able to subtract.
#[test]
fn family_h_is_capped_and_can_still_exculpate() {
    let mut ledger = Ledger::new();
    ledger
        .record(Evidence::new(Family::R, "zero textual occurrences", 1.0))
        .record(Evidence::new(
            Family::H,
            "single commit ever AND >4y old",
            -0.8,
        ));

    assert!(
        (ledger.total_bans() - 0.4).abs() < 1e-9,
        "1.0 + max(-0.8, -0.6) = 0.4, the cap applied to the exculpating direction"
    );
}

/// **The pre-commitment, enforced.**
///
/// `docs/decisions/2026-08-02-ban-ledger-and-tier-model.md` §5, written before
/// the first run: this project has exactly one family able to accuse, so every
/// candidate must cap at Tier 2 — for two independent reasons, either of which
/// alone is sufficient.
///
/// If this test ever fails, the implementation is wrong rather than the
/// repository being interesting.
#[test]
fn one_accusing_family_caps_every_candidate_at_tier_two() {
    let mut ledger = Ledger::new();
    // The strongest row R has, on a perfectly clean candidate, with every gate
    // this build can evaluate satisfied.
    ledger.record(Evidence::new(
        Family::R,
        "statically typed, compiler-index-backed, zero dynamism detected",
        1.5,
    ));

    let assignment = assign(&ledger, clean_gates());

    assert_eq!(assignment.tier(), Tier::Two);
    assert_eq!(assignment.accusing(), [Family::R]);
    assert!(
        assignment.total_bans() < TIER1_BANS,
        "R's maximum row is +1.5, below Tier 1's {TIER1_BANS} on its own"
    );
    assert!(
        assignment
            .blockers()
            .iter()
            .any(|c| c.name == ">=2 of {B,R,X} accuse"),
        "and the quorum is named as a blocker rather than left implicit"
    );
}

/// The other half: even a ledger that clears both ban thresholds is capped,
/// because §9.6's other conjuncts cannot be evaluated here.
///
/// This is the test that would catch the tempting bug — treating an unchecked
/// criterion as satisfied, so the tier climbs on the strength of what was never
/// measured.
#[test]
fn clearing_the_ban_thresholds_does_not_promote_while_criteria_are_unevaluable() {
    let mut ledger = Ledger::new();
    ledger
        .record(Evidence::new(
            Family::B,
            "build regenerates byte-identically",
            2.0,
        ))
        .record(Evidence::new(Family::R, "zero textual occurrences", 1.0))
        .record(Evidence::new(
            Family::X,
            "tombstone silent >=13 months",
            1.2,
        ));

    let assignment = assign(&ledger, clean_gates());

    assert!(
        assignment.total_bans() >= TIER0_BANS,
        "4.2 bans clears even Tier 0's {TIER0_BANS}"
    );
    assert_eq!(assignment.accusing(), [Family::B, Family::R, Family::X]);
    assert_eq!(
        assignment.tier(),
        Tier::Two,
        "and it is still Tier 2, because §9.6's remaining conjuncts are NotEvaluable"
    );
    assert!(assignment.not_evaluable() > 0);
    assert!(
        assignment
            .criteria()
            .iter()
            .any(|c| c.name == "held the deadness invariant for N runs"
                && c.outcome == Outcome::NotEvaluable),
        "the stability window alone guarantees the cap"
    );
}

/// 3f is never waivable (§9.3), and a failing gate set demotes past Tier 2.
#[test]
fn a_failed_gate_demotes_and_gate_3f_is_named() {
    let mut ledger = Ledger::new();
    ledger.record(Evidence::new(Family::R, "zero textual occurrences", 1.0));

    let refused = assign(
        &ledger,
        GateState {
            gates_0_to_2: Outcome::Satisfied,
            gate_3f: Outcome::Failed,
        },
    );
    assert_eq!(
        refused.tier(),
        Tier::Two,
        "still reportable, never promotable"
    );
    assert!(refused
        .blockers()
        .iter()
        .any(|c| c.name == "3f never waivable"));

    let vetoed = assign(
        &ledger,
        GateState {
            gates_0_to_2: Outcome::Failed,
            gate_3f: Outcome::Satisfied,
        },
    );
    assert_eq!(
        vetoed.tier(),
        Tier::Three,
        "a candidate Gate 0-2 refused is not shown by default"
    );
}

/// §9.4: no cached verdict. The tier is a function of the ledger, so adding
/// evidence changes the answer with nothing to invalidate.
#[test]
fn the_tier_is_re_derived_rather_than_stored() {
    let mut ledger = Ledger::new();
    let before = assign(&ledger, clean_gates());
    assert!(before.accusing().is_empty());

    ledger.record(Evidence::new(Family::R, "zero textual occurrences", 1.0));
    let after = assign(&ledger, clean_gates());

    assert_eq!(after.accusing(), [Family::R]);
    assert!(after.total_bans() > before.total_bans());
}

/// The prior is reported and never compared against a threshold — §4 of the
/// decision record flags that reading as an inference rather than a quotation.
#[test]
fn the_prior_is_carried_beside_the_total_and_not_folded_into_it() {
    let mut ledger = Ledger::new();
    ledger.record(Evidence::new(Family::R, "zero textual occurrences", 1.0));
    let assignment = assign(&ledger, clean_gates());

    assert!((assignment.total_bans() - 1.0).abs() < 1e-9, "bans alone");
    assert!(
        (assignment.prior() + 0.95).abs() < 1e-9,
        "§9.5's prior, separately"
    );
    assert!(
        (ledger.posterior_log10_odds() - 0.05).abs() < 1e-9,
        "available to a reader who wants the probability, and used by nothing"
    );
}

/// An empty ledger totals exactly zero, sign included.
///
/// `clamp` can return a negative zero, and `{:.2}` renders that as `-0.00` — a
/// minus sign on the one number a reader checks to see whether anything accused
/// at all. Caught by reading the report rather than by a test, which is why
/// there is now a test.
#[test]
fn an_empty_ledger_totals_positive_zero() {
    let ledger = Ledger::new();
    assert_eq!(ledger.total_bans(), 0.0);
    assert_eq!(format!("{:.2}", ledger.total_bans()), "0.00");

    // And a family whose only evidence cancels to zero after the cap.
    let mut capped = Ledger::new();
    capped.record(Evidence::new(Family::H, "90d-1y", 0.0));
    assert_eq!(format!("{:.2}", capped.total_bans()), "0.00");
}

/// **The answer must not depend on the order evidence arrived.**
///
/// Found by review. An earlier fold used `max` for positive bans and `min` for
/// negative ones against a single running slot, so H rows of `+0.5` then `-0.8`
/// totalled `-0.6`, and the same two in the opposite order totalled `+0.5`. A
/// deadness score that changes with the order a tool happened to emit its
/// findings is the defect this codebase rejects everywhere else.
#[test]
fn the_total_does_not_depend_on_the_order_evidence_was_recorded() {
    let rows = [
        Evidence::new(Family::H, "single commit ever AND <2y old", 0.5),
        Evidence::new(Family::H, "single commit ever AND >4y old", -0.8),
        Evidence::new(Family::R, "dynamic language", 0.4),
        Evidence::new(Family::R, "zero textual occurrences", 1.0),
    ];

    let mut forwards = Ledger::new();
    for row in rows.iter().cloned() {
        forwards.record(row);
    }
    let mut backwards = Ledger::new();
    for row in rows.iter().rev().cloned() {
        backwards.record(row);
    }

    assert_eq!(forwards.total_bans(), backwards.total_bans());
}

/// §9.5 twice over: family H *"may only subtract"*, and its positive rows are
/// unvalidated against §6.18's measurement that age is anti-predictive, so the
/// implementation rule is to **ship them at 0.0**.
///
/// The previous H test only checked `accuses()`, so a positive H row was
/// silently inflating the total while the suite stayed green.
#[test]
fn a_positive_history_row_contributes_nothing_to_the_total() {
    let mut only_positive = Ledger::new();
    only_positive.record(Evidence::new(Family::H, "last touched 1-2y", 0.3));
    assert_eq!(
        only_positive.total_bans(),
        0.0,
        "H may only subtract, so its positive rows ship at 0.0 (§9.5)"
    );

    let mut mixed = Ledger::new();
    mixed
        .record(Evidence::new(Family::R, "zero textual occurrences", 1.0))
        .record(Evidence::new(
            Family::H,
            "single commit ever AND <2y old",
            0.5,
        ))
        .record(Evidence::new(
            Family::H,
            "neighbours churn while it does not",
            0.2,
        ));
    assert!(
        (mixed.total_bans() - 1.0).abs() < 1e-9,
        "R's 1.0 alone; two positive H rows add nothing"
    );
}

/// **A gate nobody ran is not a gate that passed.**
///
/// Found by review, and it is §6.20's inversion committed inside the module
/// written to prevent it: `explain` runs Gate 0g and Gate 1 only, and reported
/// Tier 2 by passing Gate 1's verdict as though it answered for Gates 0-2.
/// `GateState` is tri-state now, and its `Default` is "nothing evaluated" so a
/// caller that forgets a field gets a demotion rather than a promotion.
#[test]
fn an_unrun_gate_demotes_rather_than_being_credited_with_a_pass() {
    let mut ledger = Ledger::new();
    ledger.record(Evidence::new(Family::R, "zero textual occurrences", 1.0));

    let unrun = assign(&ledger, GateState::default());
    assert_eq!(
        unrun.tier(),
        Tier::Three,
        "gates 0-2 were never evaluated, so §9.6's Tier 2 precondition is not met"
    );
    assert!(unrun
        .criteria()
        .iter()
        .any(|c| c.name == "gates 0-2 pass" && c.outcome == Outcome::NotEvaluable));

    // And the contrast, so this is not just "everything is Tier 3".
    let ran = assign(&ledger, clean_gates());
    assert_eq!(ran.tier(), Tier::Two);
}
