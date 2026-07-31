//! The committed baseline: what we already owe, so CI can block only what is new.
//!
//! §9.14 says build this first. Baseline the current state and fail CI only on
//! *new* dead code, new junk, new unused dependencies. Zero deletion risk, zero
//! configuration burden, and the best prior art in the survey: Shopify's
//! `deprecation_toolkit`, which unblocked hundreds of monolith developers
//! precisely because it did not require the backlog to be fixed first.
//!
//! Stored as JSONL, one entry per line, because the file is **committed** (§9.4)
//! and therefore reviewed and merged by humans. One line per finding keeps
//! diffs readable and merge conflicts local to the findings that actually
//! changed.
//!
//! The known failure mode, named in §9.14: baseline files rot and become a
//! permanent amnesty list. That is what [`crate::rot`] exists to detect and why
//! [`BaselineEntry::expires`] exists at all.

use std::path::Path;

use judged_core::sarif::SarifResult;
use judged_core::Result;
use serde::{Deserialize, Serialize};

/// One accepted finding.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BaselineEntry {
    /// Content-derived identity from `judged_core::fingerprint`. The join key.
    pub fingerprint: String,
    /// Rule that produced the finding. Denormalized so a human reading the file
    /// can tell what was accepted without running anything.
    pub rule_id: String,
    /// Repo-relative URI at the time of acceptance. Denormalized for the same
    /// reason, and used by rot detection to notice the referent is gone.
    pub uri: String,
    /// Symbol, when the finding was about one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symbol: Option<String>,
    /// RFC 3339 timestamp of first acceptance. Supplied by the caller rather
    /// than read from the clock here, so baselines are reproducible in tests.
    pub first_seen: String,
    /// Optional RFC 3339 expiry. An entry past its expiry is rot, not amnesty.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires: Option<String>,
    /// Why this was accepted. Optional, because bulk-baselining an existing
    /// backlog is the entire point and demanding a justification per line would
    /// reintroduce the configuration burden the ratchet exists to avoid.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub justification: Option<String>,
}

/// A JSONL-backed set of accepted findings.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Baseline {
    entries: Vec<BaselineEntry>,
}

impl Baseline {
    /// Build a baseline from entries already in hand.
    pub fn new(entries: Vec<BaselineEntry>) -> Self {
        Self { entries }
    }

    /// The accepted findings, in file order.
    pub fn entries(&self) -> &[BaselineEntry] {
        &self.entries
    }

    /// Read a baseline from a JSONL file.
    ///
    /// A missing file is an empty baseline — the first run on a repository must
    /// not need a setup step. A *malformed* file is an error: silently dropping
    /// an unparseable line would quietly un-accept findings and fail CI for
    /// reasons nobody could explain.
    pub fn load(_path: &Path) -> Result<Baseline> {
        todo!("Baseline::load: parse JSONL, missing file means empty, bad line is an error")
    }

    /// Write the baseline as JSONL, one entry per line.
    pub fn save(&self, _path: &Path) -> Result<()> {
        todo!("Baseline::save: one compact JSON object per line, trailing newline")
    }

    /// Accept every finding in `results` as of `first_seen` (RFC 3339).
    pub fn from_results(_results: &[SarifResult], _first_seen: &str) -> Baseline {
        todo!("Baseline::from_results: one entry per result, keyed by judged/v1 fingerprint")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry() -> BaselineEntry {
        BaselineEntry {
            fingerprint: "judged/v1:00ff".to_string(),
            rule_id: "unused-export".to_string(),
            uri: "src/foo.ts".to_string(),
            symbol: Some("foo".to_string()),
            first_seen: "2026-07-31T00:00:00Z".to_string(),
            expires: None,
            justification: None,
        }
    }

    #[test]
    fn absent_optional_fields_are_omitted_not_nulled() {
        // The baseline is committed and reviewed in PRs (§9.4). A wall of
        // `"expires":null` is review noise, and review noise is how a baseline
        // rots into the permanent amnesty list §9.14 warns about.
        let mut e = entry();
        e.symbol = None;
        let json = serde_json::to_string(&e).expect("BaselineEntry must serialize");

        assert!(!json.contains("expires"), "got {json}");
        assert!(!json.contains("justification"), "got {json}");
        assert!(!json.contains("symbol"), "got {json}");
        assert!(
            json.contains(r#""fingerprint":"judged/v1:00ff""#),
            "got {json}"
        );
    }

    #[test]
    fn an_entry_is_exactly_one_jsonl_line() {
        let json = serde_json::to_string(&entry()).expect("BaselineEntry must serialize");

        assert!(!json.contains('\n'), "got {json}");
    }
}
