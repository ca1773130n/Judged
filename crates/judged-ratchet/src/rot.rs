//! Rot detection: keeping the baseline from becoming a permanent amnesty list.
//!
//! §9.14 names this as the known failure mode of every ratchet, and points at
//! the same treatment the keep manifest gets in §5.3. The underlying reason is
//! SWE@Google Ch. 15, quoted in §9.14: "it's often tempting to just mark
//! something as deprecated and hope its uses eventually disappear, but
//! remember: hope is not a strategy." A baseline nobody prunes is exactly that
//! hope, written down.

use judged_core::git::Repo;
use judged_core::sarif::Run;

use crate::baseline::Baseline;

/// Why a baseline entry should no longer be there.
///
/// Every variant carries what a human needs to act on it without re-running
/// anything, because the cost of pruning is what decides whether pruning
/// happens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RotReason {
    /// No finding in the current run carries this fingerprint. Either the
    /// finding was fixed, or our own analysis stopped producing it — both mean
    /// the amnesty is now protecting nothing.
    NeverMatched { fingerprint: String },
    /// The file the entry points at no longer exists, so the entry can never
    /// match again regardless of what the analyzers do.
    ReferentGone { uri: String },
    /// The entry carried an explicit expiry and it has passed.
    Expired {
        fingerprint: String,
        expires: String,
    },
}

/// Find every baseline entry that has stopped earning its place.
///
/// `now` is an RFC 3339 timestamp supplied by the caller rather than read from
/// the clock, so that a CI run and a local run over the same inputs agree.
pub fn detect_rot(_baseline: &Baseline, _run: &Run, _repo: &Repo, _now: &str) -> Vec<RotReason> {
    todo!("detect_rot: unmatched fingerprints, missing referents, passed expiries")
}
