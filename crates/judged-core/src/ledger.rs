//! The ban ledger and §9.6's tier model.
//!
//! Everything else in this workspace only ever *removes* claims. This module is
//! the first thing that scores one, and therefore the first thing whose output
//! could ever authorize a deletion. The decisions behind it are written down in
//! `docs/decisions/2026-08-02-ban-ledger-and-tier-model.md`; three of them are
//! load-bearing enough to repeat here.
//!
//! # Store evidence, never verdicts
//!
//! §9.4's governing principle, which it notes **no surveyed tool follows**:
//! *"STORE EVIDENCE, NEVER VERDICTS. Re-derive every run."* So [`Ledger`] holds
//! [`Evidence`] and a [`Tier`] is a function of it, recomputed on every call.
//! There is no cached tier and no setter. A stored verdict is a claim about a
//! tree state that has since changed — §6.21's OpenRewrite #321 failure.
//!
//! # MAX within family, SUM across families
//!
//! §9.5's arithmetic, and not an implementation convenience. Static reachability
//! and test coverage share the repo-dynamism confounder: a module reached only
//! through `getattr` is missed by the static graph *and* by the test suite, so
//! their agreement is close to one observation reported twice. Summing within a
//! family would produce the documented overconfidence pathology.
//!
//! # A criterion nobody could evaluate demotes, exactly like one that failed
//!
//! §9.6's Tier 0 has fourteen conjuncts and this build can evaluate a few. The
//! tempting reading — nobody said they failed, so they pass — is the §6.20
//! inversion applied to our own scoreboard: the tier would climb on the strength
//! of what was never measured. So [`Criterion`] has three states, and
//! [`Assignment`] carries every one that did not hold, which makes the distance
//! between this build and §9.6 countable rather than arguable.
//!
//! **No candidate can reach Tier 0 or Tier 1 in this build.** The stability
//! window alone guarantees it. A Tier 2 result here means *capped*, not *scored*.

use std::collections::BTreeMap;
use std::fmt;

/// §2.2's correlation families.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Family {
    /// Build / artifact identity: linker GC, shipped-symbol presence, declared
    /// outputs, regenerate-and-diff.
    B,
    /// Reads repository text: static reachability, the grep veto, manifest
    /// roots, name heuristics. Everything this project shipped before coverage.
    R,
    /// Observes execution: production coverage, tombstones, profiler samples.
    X,
    /// History: VCS age, churn, co-change.
    H,
}

impl Family {
    /// Stable label, for reports.
    pub fn as_str(self) -> &'static str {
        match self {
            Family::B => "B",
            Family::R => "R",
            Family::X => "X",
            Family::H => "H",
        }
    }

    /// Whether this family is even *allowed* to accuse.
    ///
    /// §9.5 definition 1: **"Family H can never accuse"** — §6.18 measured age
    /// as anti-predictive (>4y untouched → 1.4% subsequent deletion against a
    /// 6.4% base rate) and its positive rows are marked unvalidated. It may only
    /// subtract.
    pub fn may_accuse(self) -> bool {
        !matches!(self, Family::H)
    }

    /// §9.5 caps family H at ±0.6 total. `None` for every other family.
    pub fn cap(self) -> Option<f64> {
        matches!(self, Family::H).then_some(0.6)
    }
}

impl fmt::Display for Family {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The prior, from §9.5: log₁₀-odds(dead) = −0.95, i.e. P ≈ 0.10.
///
/// Exposed rather than folded into the totals — see [`Assignment::prior`] and
/// §4 of the decision record on why the thresholds are compared against the sum
/// of bans alone.
pub const PRIOR_LOG10_ODDS: f64 = -0.95;

/// §9.6's Tier 0 ban threshold.
pub const TIER0_BANS: f64 = 3.95;

/// §9.6's Tier 1 ban threshold.
pub const TIER1_BANS: f64 = 2.65;

/// §9.5 definition 1's accusation floor.
pub const ACCUSE_FLOOR: f64 = 0.5;

/// One piece of evidence, with everything §9.5 definition 1 requires to decide
/// whether it may count toward an accusation.
///
/// The health flags are not optional metadata. §9.5: a family accuses only when
/// *"every evidence artifact contributing to that maximum has
/// `execution_successful = true`, `positive_control_passed = true`, and
/// `expires_at > now`"*. An artifact that failed its positive control is the
/// §3.7 case — it looks exactly like a clean measurement — so evidence that
/// cannot vouch for itself is carried and then excluded, never dropped silently.
#[derive(Debug, Clone, PartialEq)]
pub struct Evidence {
    family: Family,
    signal: String,
    bans: f64,
    execution_successful: bool,
    positive_control_passed: bool,
    expired: bool,
}

impl Evidence {
    /// Evidence that ran cleanly and passed its control.
    pub fn new(family: Family, signal: impl Into<String>, bans: f64) -> Evidence {
        Evidence {
            family,
            signal: signal.into(),
            bans,
            execution_successful: true,
            positive_control_passed: true,
            expired: false,
        }
    }

    /// Mark the producing run as failed — it did not complete.
    pub fn execution_failed(mut self) -> Evidence {
        self.execution_successful = false;
        self
    }

    /// Mark the artifact as having failed its positive control (§3.7).
    pub fn control_failed(mut self) -> Evidence {
        self.positive_control_passed = false;
        self
    }

    /// Mark the evidence as past its `expires_at`.
    pub fn expired(mut self) -> Evidence {
        self.expired = true;
        self
    }

    /// Which family it belongs to.
    pub fn family(&self) -> Family {
        self.family
    }

    /// The §9.5 row that produced it, spelled as the table spells it.
    pub fn signal(&self) -> &str {
        &self.signal
    }

    /// Its ban weight. Negative for the exculpating H rows.
    pub fn bans(&self) -> f64 {
        self.bans
    }

    /// Whether this artifact may contribute to an accusation.
    pub fn healthy(&self) -> bool {
        self.execution_successful && self.positive_control_passed && !self.expired
    }

    /// Why it may not, when it may not.
    pub fn unhealthy_reason(&self) -> Option<&'static str> {
        if !self.execution_successful {
            Some("the run that produced it did not complete")
        } else if !self.positive_control_passed {
            Some("it failed its positive control (§3.7)")
        } else if self.expired {
            Some("it is past its expiry")
        } else {
            None
        }
    }
}

/// One candidate's evidence, and the arithmetic §9.5 defines over it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Ledger {
    evidence: Vec<Evidence>,
}

impl Ledger {
    /// An empty ledger.
    pub fn new() -> Ledger {
        Ledger::default()
    }

    /// Record a piece of evidence.
    pub fn record(&mut self, evidence: Evidence) -> &mut Ledger {
        self.evidence.push(evidence);
        self
    }

    /// Everything recorded, in the order it arrived.
    pub fn evidence(&self) -> &[Evidence] {
        &self.evidence
    }

    /// The family's maximum **accuse-polarity** ban, counting only healthy
    /// artifacts.
    ///
    /// `None` when the family contributed nothing that could accuse — which is
    /// distinct from contributing zero, and the distinction is §6.20's.
    pub fn family_max(&self, family: Family) -> Option<f64> {
        self.evidence
            .iter()
            .filter(|e| e.family == family && e.healthy() && e.bans > 0.0)
            .map(|e| e.bans)
            .fold(None, |best: Option<f64>, bans| {
                Some(best.map_or(bans, |b| b.max(bans)))
            })
    }

    /// Whether `family` ACCUSES, by §9.5 definition 1 exactly.
    ///
    /// MAX ≥ [`ACCUSE_FLOOR`], every contributing artifact healthy, and the
    /// family allowed to accuse at all. §9.5 is explicit that a family whose
    /// maximum comes only from `+0.1` name-pattern or `+0.3` manifest-absence
    /// evidence **abstains** rather than accusing — which the floor produces
    /// arithmetically rather than by a special case.
    pub fn accuses(&self, family: Family) -> bool {
        family.may_accuse()
            && self
                .family_max(family)
                .is_some_and(|max| max >= ACCUSE_FLOOR)
    }

    /// Every family that accuses, in family order.
    pub fn accusing(&self) -> Vec<Family> {
        [Family::B, Family::R, Family::X, Family::H]
            .into_iter()
            .filter(|family| self.accuses(*family))
            .collect()
    }

    /// The accumulated bans: **MAX within family, SUM across families** (§9.5).
    ///
    /// The exculpating H rows are negative and are summed with their own sign
    /// after the family cap, because a family that may only subtract still has
    /// to be able to.
    pub fn total_bans(&self) -> f64 {
        let mut per_family: BTreeMap<Family, f64> = BTreeMap::new();
        for evidence in self.evidence.iter().filter(|e| e.healthy()) {
            let slot = per_family.entry(evidence.family).or_insert(0.0);
            // MAX by magnitude in the accusing direction, and the most
            // exculpating value in the other — a family contributes its single
            // strongest statement, not the sum of its correlated ones.
            if evidence.bans > 0.0 {
                *slot = slot.max(evidence.bans);
            } else {
                *slot = slot.min(evidence.bans);
            }
        }
        let total: f64 = per_family
            .into_iter()
            .map(|(family, bans)| match family.cap() {
                Some(cap) => bans.clamp(-cap, cap),
                None => bans,
            })
            .sum();
        // `clamp` can hand back a negative zero, and `{:.2}` renders that as
        // `-0.00` — a minus sign on the one number a reader is looking at to
        // decide whether anything accused. Normalized here rather than at each
        // call site.
        if total == 0.0 {
            0.0
        } else {
            total
        }
    }

    /// The posterior log₁₀-odds under §9.5's prior, for a reader who wants the
    /// probability rather than the ban count.
    ///
    /// Reported beside the ban total and never compared against a threshold —
    /// see §4 of the decision record.
    pub fn posterior_log10_odds(&self) -> f64 {
        PRIOR_LOG10_ODDS + self.total_bans()
    }
}

// ---------------------------------------------------------------------------
// The tier model (§9.6)
// ---------------------------------------------------------------------------

/// §9.6's tiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tier {
    /// Quarantine automatically; reap after soak.
    Zero,
    /// Open a PR that quarantines; a human approves.
    One,
    /// Report only, naming the specific unclosed assumption.
    Two,
    /// Not shown by default.
    Three,
}

impl Tier {
    /// Stable label, for reports.
    pub fn as_str(self) -> &'static str {
        match self {
            Tier::Zero => "0",
            Tier::One => "1",
            Tier::Two => "2",
            Tier::Three => "3",
        }
    }

    /// What §9.6 says happens at this tier.
    pub fn action(self) -> &'static str {
        match self {
            Tier::Zero => "quarantine automatically; reap after soak",
            Tier::One => "open a PR that quarantines, never deletes; a human approves",
            Tier::Two => "report only, naming the specific unclosed assumption",
            Tier::Three => "not shown by default",
        }
    }
}

impl fmt::Display for Tier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// How a §9.6 criterion came out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Checked, and it holds.
    Satisfied,
    /// Checked, and it does not.
    Failed,
    /// **Not checkable in this build.** Demotes exactly as `Failed` does.
    ///
    /// The whole point of the distinction is that it is visible: a report can
    /// say how much of §9.6 was evaluated at all, which is the number that says
    /// what a tier here is worth.
    NotEvaluable,
}

impl Outcome {
    /// Whether this outcome permits promotion.
    pub fn holds(self) -> bool {
        matches!(self, Outcome::Satisfied)
    }
}

/// One §9.6 criterion and how it came out.
#[derive(Debug, Clone, PartialEq)]
pub struct Criterion {
    /// The criterion as §9.6 names it.
    pub name: &'static str,
    /// Satisfied, failed, or not evaluable here.
    pub outcome: Outcome,
    /// The reason, in a sentence somebody can act on.
    pub detail: String,
}

/// A tier, and every reason it is not higher.
#[derive(Debug, Clone, PartialEq)]
pub struct Assignment {
    tier: Tier,
    criteria: Vec<Criterion>,
    total_bans: f64,
    prior: f64,
    accusing: Vec<Family>,
}

impl Assignment {
    /// The assigned tier.
    pub fn tier(&self) -> Tier {
        self.tier
    }

    /// Every criterion considered, in §9.6's order.
    pub fn criteria(&self) -> &[Criterion] {
        &self.criteria
    }

    /// The criteria that did not hold — failed or not evaluable — which is the
    /// list §9.6 Tier 2 requires be *named* rather than summarized.
    pub fn blockers(&self) -> Vec<&Criterion> {
        self.criteria
            .iter()
            .filter(|c| !c.outcome.holds())
            .collect()
    }

    /// How many criteria this build could not evaluate at all.
    ///
    /// The honesty number. A tier assigned with most of §9.6 unevaluated is a
    /// cap, not a score, and a report that does not print this cannot say which.
    pub fn not_evaluable(&self) -> usize {
        self.criteria
            .iter()
            .filter(|c| c.outcome == Outcome::NotEvaluable)
            .count()
    }

    /// Accumulated bans (§9.5's MAX-within, SUM-across).
    pub fn total_bans(&self) -> f64 {
        self.total_bans
    }

    /// §9.5's prior, reported separately and never folded into the comparison.
    pub fn prior(&self) -> f64 {
        self.prior
    }

    /// Families that accuse, by §9.5 definition 1.
    pub fn accusing(&self) -> &[Family] {
        &self.accusing
    }
}

/// What the caller already knows about the candidate from the gate layers.
///
/// Everything here is something this build genuinely computes. Anything §9.6
/// requires and this struct does not carry is a [`Outcome::NotEvaluable`]
/// criterion — deliberately, so that adding a field is the only way to promote a
/// criterion out of that state.
#[derive(Debug, Clone, Copy, Default)]
pub struct GateState {
    /// Gate 0–2 all passed for this candidate.
    pub gates_0_to_2_pass: bool,
    /// Gate 3f found nothing (§9.3: *"No ban count overrides this"*).
    pub gate_3f_clear: bool,
}

/// Assign a tier from a ledger and what the gates found.
///
/// Every §9.6 criterion appears in the result, including the ones this build
/// cannot evaluate. A criterion that does not hold demotes, and the tier is the
/// best one whose criteria all hold.
pub fn assign(ledger: &Ledger, gates: GateState) -> Assignment {
    let accusing = ledger.accusing();
    let total = ledger.total_bans();
    let quorum = accusing.len() >= 2;

    let mut criteria = vec![
        Criterion {
            name: "gates 0-2 pass",
            outcome: if gates.gates_0_to_2_pass {
                Outcome::Satisfied
            } else {
                Outcome::Failed
            },
            detail: "Gate 0 (recoverability), Gate 1 (never-touch) and Gate 2 (reference veto)"
                .to_string(),
        },
        Criterion {
            name: "3f never waivable",
            outcome: if gates.gate_3f_clear {
                Outcome::Satisfied
            } else {
                Outcome::Failed
            },
            detail: "§9.3: the candidate's type is not serializable, its name cannot appear in \
                     a queue payload, and its symbol is not exported across an ABI boundary"
                .to_string(),
        },
        Criterion {
            name: ">=2 of {B,R,X} accuse",
            outcome: if quorum {
                Outcome::Satisfied
            } else {
                Outcome::Failed
            },
            detail: format!(
                "§9.5 definition 1. Accusing: {}. A family accuses at MAX >= +{ACCUSE_FLOOR} \
                 bans with every contributing artifact healthy; family H never accuses.",
                if accusing.is_empty() {
                    "none".to_string()
                } else {
                    accusing
                        .iter()
                        .map(|family| family.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                }
            ),
        },
    ];

    // Everything §9.6 requires that nothing in this build computes. Listed
    // individually rather than as one "not implemented" line, because the count
    // is the measure of how far from §9.6 an assignment here is.
    for (name, detail) in NOT_EVALUABLE {
        criteria.push(Criterion {
            name,
            outcome: Outcome::NotEvaluable,
            detail: (*detail).to_string(),
        });
    }

    let base = Assignment {
        tier: Tier::Three,
        criteria,
        total_bans: total,
        prior: PRIOR_LOG10_ODDS,
        accusing,
    };

    // §9.6, read top down. Tier 3 is "everything else", so the walk starts at
    // the top and stops at the first tier whose criteria all hold.
    let tier = if base.criteria.iter().all(|c| c.outcome.holds()) && total >= TIER0_BANS {
        Tier::Zero
    } else if base.criteria.iter().all(|c| c.outcome.holds()) && total >= TIER1_BANS {
        Tier::One
    } else if gates.gates_0_to_2_pass {
        // §9.6 Tier 2: gates 0-2 pass, and at least one Gate-3 conjunct failed,
        // or a ceiling is active, or the ladder rung is below R2. In this build
        // that is always true, because the rung cannot be computed at all.
        Tier::Two
    } else {
        Tier::Three
    };

    Assignment { tier, ..base }
}

/// The §9.6 criteria this build cannot compute, each with what it would take.
///
/// Kept as data rather than prose so the report can print them and a reader can
/// count them. Every entry is a promise about what is *not* known, and moving one
/// out of this list is the only way a candidate here ever exceeds Tier 2.
const NOT_EVALUABLE: &[(&str, &str)] = &[
    (
        "all six Gate-3 conjuncts",
        "3a-3e do not exist: 3a-3d are directory conjuncts for build artifacts and 3e is the \
         family quorum. Only 3f is implemented.",
    ),
    (
        "zero ABSTAINs from a LOAD-BEARING family",
        "§9.5 definition 2 makes X load-bearing for every symbol and file in a repo that runs \
         in production, and B load-bearing for every artifact. Neither can be evaluated \
         without those families existing.",
    ),
    (
        "scanned_universe_ratio >= 0.8",
        "no adapter reports the fraction of each language's files it actually scanned.",
    ),
    (
        "ladder_rung >= R2",
        "Gate 0g classifies recoverability, but §9.6 requires the §8.2 promotion to have been \
         performed and verified for an untracked or ignored candidate, which nothing does.",
    ),
    (
        "held the deadness invariant for N runs",
        "§9.5 definition 3 needs a store of prior runs keyed by tree SHA, and a default window \
         of 20 runs or 90 days. There is no such store.",
    ),
    (
        "under the per-run rate limit",
        "§9.6's graduated autonomy caps acted-on candidates per day. Nothing acts, so nothing \
         counts.",
    ),
    (
        "no has_external_effector",
        "§0 item 11: in a GitOps or IaC repository the file IS the desired state of a live \
         system. Nothing detects that.",
    ),
    (
        "not is_distributable",
        "§6.9's inverted rule for published artifacts. Nothing detects it.",
    ),
];
