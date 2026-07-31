//! Rot detection: keeping the baseline from becoming a permanent amnesty list.
//!
//! §9.14 names this as the known failure mode of every ratchet, and points at
//! the same treatment the keep manifest gets in §5.3. The underlying reason is
//! SWE@Google Ch. 15, quoted in §9.14: "it's often tempting to just mark
//! something as deprecated and hope its uses eventually disappear, but
//! remember: hope is not a strategy." A baseline nobody prunes is exactly that
//! hope, written down.

use std::collections::BTreeSet;

use judged_core::git::Repo;
use judged_core::sarif::Run;

use crate::baseline::{result_fingerprint, Baseline};

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
    /// The entry carried an explicit expiry and it has passed — or the expiry
    /// is not a date this crate can evaluate, which is treated the same way for
    /// the reason given on `has_expired`.
    Expired {
        fingerprint: String,
        expires: String,
    },
}

/// Find every baseline entry that has stopped earning its place.
///
/// `now` is an RFC 3339 timestamp supplied by the caller rather than read from
/// the clock, so that a CI run and a local run over the same inputs agree.
///
/// At most **one** reason is reported per entry, because the remediation is the
/// same for all of them — delete the line — and three reasons for one line is
/// how a rot report becomes something people skim past. Precedence is
/// most-specific first:
///
/// 1. [`RotReason::ReferentGone`] — the strongest statement available. The file
///    is gone, so the entry can never match again no matter what the analyzers
///    do, and it necessarily drags its fingerprint out of the run as well.
///    Reporting the implied `NeverMatched` alongside it would double every
///    deleted file in the report.
/// 2. [`RotReason::Expired`] — a deadline a human set has passed. Independent of
///    whether the finding still matches, which is the point of setting one.
/// 3. [`RotReason::NeverMatched`] — nothing in this run carries the fingerprint.
pub fn detect_rot(baseline: &Baseline, run: &Run, repo: &Repo, now: &str) -> Vec<RotReason> {
    let seen: BTreeSet<String> = run.results.iter().map(result_fingerprint).collect();

    let mut reasons = Vec::new();
    for entry in baseline.entries() {
        if referent_is_gone(repo, &entry.uri) {
            reasons.push(RotReason::ReferentGone {
                uri: entry.uri.clone(),
            });
        } else if let Some(expires) = entry.expires.as_deref().filter(|e| has_expired(e, now)) {
            reasons.push(RotReason::Expired {
                fingerprint: entry.fingerprint.clone(),
                expires: expires.to_string(),
            });
        } else if !seen.contains(&entry.fingerprint) {
            reasons.push(RotReason::NeverMatched {
                fingerprint: entry.fingerprint.clone(),
            });
        }
    }
    reasons
}

/// Whether the artifact a baseline entry points at has disappeared.
///
/// An empty URI is not a missing file: project-scoped findings (an unused
/// dependency, say) legitimately have no artifact, and joining the repo root
/// with `""` would test the root directory instead — a check that passes by
/// accident rather than by meaning anything.
///
/// URIs are documented repo-relative. An absolute one would make `join`
/// discard the root and test a path outside the working tree; that is wrong but
/// harmless here, because the worst this function can do is put a line in a
/// report. Nothing in this crate deletes (§9.14).
fn referent_is_gone(repo: &Repo, uri: &str) -> bool {
    !uri.is_empty() && !repo.root().join(uri).exists()
}

/// Whether an entry's `expires` has been reached, given `now`.
///
/// Comparison is lexicographic over the two shapes the format accepts —
/// `YYYY-MM-DD` (§5.3's `expires`) and RFC 3339 UTC (`first_seen`, and `now`) —
/// which sort correctly against each other because both begin with the same
/// fixed-width date and `""` sorts before `"T"`. That is why this crate needs
/// no date library: it never does arithmetic on dates, only ordering.
///
/// A value that is not in either shape counts as expired. The baseline is
/// hand-edited, so `2026/08/01` and `next quarter` will be typed into it, and
/// both would compare as far-future under a lexicographic rule — silently
/// granting a longer amnesty than the author asked for. §12: fail loudly. The
/// raw text is carried into the report so the author can see what they wrote.
fn has_expired(expires: &str, now: &str) -> bool {
    !is_iso_dated(expires) || expires <= now
}

/// Whether `value` starts with a `YYYY-MM-DD` date, which is the prefix both
/// accepted shapes share.
fn is_iso_dated(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() >= 10
        && bytes[..4].iter().all(u8::is_ascii_digit)
        && bytes[4] == b'-'
        && bytes[5..7].iter().all(u8::is_ascii_digit)
        && bytes[7] == b'-'
        && bytes[8..10].iter().all(u8::is_ascii_digit)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expiry_ordering_holds_across_the_two_accepted_shapes() {
        // A date-only expiry compared against an RFC 3339 instant is the common
        // case: §5.3 specifies `expires: YYYY-MM-DD` while `now` is a full
        // timestamp. Pinned because the whole no-date-library decision rests on
        // these comparisons being exact rather than approximately right.
        assert!(has_expired("2026-07-30", "2026-07-31T12:00:00Z"));
        assert!(!has_expired("2026-08-01", "2026-07-31T12:00:00Z"));
        // The expiry day has arrived: the amnesty was granted "until" that
        // date, not "through" it.
        assert!(has_expired("2026-07-31", "2026-07-31T12:00:00Z"));
        // Same shape on both sides, to the instant.
        assert!(has_expired("2026-07-31T12:00:00Z", "2026-07-31T12:00:00Z"));
        assert!(!has_expired("2026-07-31T12:00:01Z", "2026-07-31T12:00:00Z"));
    }

    #[test]
    fn a_date_that_is_not_iso_shaped_is_treated_as_expired() {
        assert!(!is_iso_dated("2026/07/30"));
        assert!(!is_iso_dated("31-07-2026"));
        assert!(!is_iso_dated("soon"));
        assert!(is_iso_dated("2026-07-30"));
        assert!(is_iso_dated("2026-07-30T00:00:00Z"));
    }
}
