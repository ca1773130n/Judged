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

use std::collections::BTreeSet;
use std::path::Path;

use judged_core::fingerprint::{fingerprint, FingerprintInput, FINGERPRINT_ALGORITHM};
use judged_core::sarif::SarifResult;
use judged_core::{Error, Result};
use serde::{Deserialize, Serialize};

/// Repo-relative location of the committed baseline.
///
/// §9.4 puts the committed, PR-reviewed file at the centre of the design
/// (cargo-machete's manifest-colocation insight). Naming the path here rather
/// than in the CLI keeps "where the baseline lives" a property of the format.
pub const BASELINE_PATH: &str = ".judged/baseline.jsonl";

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
    pub fn load(path: &Path) -> Result<Baseline> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Baseline::default())
            }
            Err(source) => {
                return Err(Error::Io {
                    path: path.to_path_buf(),
                    source,
                })
            }
        };

        let mut entries = Vec::new();
        for (index, line) in text.lines().enumerate() {
            // Blank lines are tolerated because a human-merged file collects
            // them; anything else that fails to parse is reported with its line
            // number, which is the only thing that makes a merge-mangled
            // baseline fixable without bisecting it by hand.
            if line.trim().is_empty() {
                continue;
            }
            let entry: BaselineEntry =
                serde_json::from_str(line).map_err(|source| Error::Json {
                    context: format!("{} line {}", path.display(), index + 1),
                    source,
                })?;
            if entry.fingerprint.is_empty() {
                return Err(Error::Baseline(format!(
                    "{} line {}: empty fingerprint; the fingerprint is the join key, so an \
                     entry without one can never match a finding and would be indistinguishable \
                     from rot forever",
                    path.display(),
                    index + 1
                )));
            }
            entries.push(entry);
        }
        Ok(Baseline::new(entries))
    }

    /// Write the baseline as JSONL, one entry per line.
    ///
    /// Creates the parent directory, so the first `judged ratchet --write` on a
    /// repository does not require `mkdir .judged` first.
    ///
    /// Output is byte-stable for a given set of entries: `serde_json` emits
    /// derived struct fields in declaration order, and nothing here iterates a
    /// hash map. That matters because the file is committed (§9.4) — churn in a
    /// reviewed file is how a baseline stops being read.
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|source| Error::Io {
                    path: parent.to_path_buf(),
                    source,
                })?;
            }
        }

        let mut text = String::new();
        for entry in &self.entries {
            let line = serde_json::to_string(entry).map_err(|source| Error::Json {
                context: format!("serializing baseline entry {}", entry.fingerprint),
                source,
            })?;
            text.push_str(&line);
            text.push('\n');
        }

        std::fs::write(path, text).map_err(|source| Error::Io {
            path: path.to_path_buf(),
            source,
        })
    }

    /// Accept every finding in `results` as of `first_seen` (RFC 3339).
    ///
    /// Duplicate fingerprints collapse to their first occurrence. Two adapters
    /// covering overlapping targets legitimately report the same finding twice,
    /// and duplicate lines in a committed file are review noise that also makes
    /// the rot report double-count.
    pub fn from_results(results: &[SarifResult], first_seen: &str) -> Baseline {
        let mut seen = BTreeSet::new();
        let mut entries = Vec::new();
        for result in results {
            let fingerprint = result_fingerprint(result);
            if !seen.insert(fingerprint.clone()) {
                continue;
            }
            entries.push(BaselineEntry {
                fingerprint,
                rule_id: result.rule_id.clone(),
                uri: primary_uri(result).to_string(),
                // SARIF carries no symbol field of its own, so the denormalized
                // symbol can only come from a caller building entries directly.
                symbol: None,
                first_seen: first_seen.to_string(),
                // Bulk-baselining an existing backlog is the entire point of
                // §9.14, so neither an expiry nor a justification is demanded
                // here; both are things a human adds when triaging a line.
                expires: None,
                justification: None,
            });
        }
        Baseline::new(entries)
    }
}

/// The join key for a finding, in the canonical `judged/v1:<hex>` form.
///
/// Prefers the adapter's own `partialFingerprints` entry: only the adapter
/// knows the symbol and blob SHA that §9.2 wants mixed in, and neither is
/// recoverable from a [`SarifResult`]. When the adapter emitted none, one is
/// derived here from the inputs a result does carry — rule, artifact URI and
/// normalized message.
///
/// The derived form is deliberately weaker than a symbol-anchored fingerprint,
/// but it satisfies the property §9.2 actually insists on: it is **never
/// line-based**. `Location::start_line` is not an input, and
/// `judged_core::fingerprint::normalize_message` erases the positions tools
/// bake into their own text. A reformat therefore cannot manufacture a new
/// finding.
///
/// Both sides of the diff run through this function, so baseline-time and
/// check-time identity cannot drift apart.
pub(crate) fn result_fingerprint(result: &SarifResult) -> String {
    if let Some(digest) = result.partial_fingerprints.get(FINGERPRINT_ALGORITHM) {
        return format!("{FINGERPRINT_ALGORITHM}:{digest}");
    }
    fingerprint(&FingerprintInput {
        rule_id: result.rule_id.clone(),
        artifact_uri: primary_uri(result).to_string(),
        symbol: None,
        blob_sha: None,
        message: result.message.clone(),
    })
}

/// The artifact a result is about: its first location, or `""` when the tool
/// reported none. An empty URI is a real state — some findings are about a
/// project rather than a file — and rot detection has to know not to treat it
/// as a missing file.
pub(crate) fn primary_uri(result: &SarifResult) -> &str {
    result
        .locations
        .first()
        .map(|location| location.uri.as_str())
        .unwrap_or_default()
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
