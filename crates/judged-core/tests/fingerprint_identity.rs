//! Identity properties of [`judged_core::fingerprint::fingerprint`].
//!
//! §9.2 states the whole requirement in one sentence: *"Fingerprints must be
//! content-derived (symbol + normalized AST hash + blob SHA), never line-based,
//! or every reformat resets the stability clock."* Each test below pins one half
//! of that sentence — the stability half (a finding that did not change keeps its
//! id) and the distinctness half (findings that did change get different ids).

use std::collections::HashMap;

use judged_core::fingerprint::{
    fingerprint, normalize_message, FingerprintInput, FINGERPRINT_ALGORITHM, FINGERPRINT_VERSION,
};

/// A symbol-scoped accusation: "this function is unreachable".
fn symbol_finding(symbol: &str, blob_sha: Option<&str>, message: &str) -> FingerprintInput {
    FingerprintInput {
        rule_id: "ruff/F401".to_string(),
        artifact_uri: "src/pkg/models.py".to_string(),
        symbol: Some(symbol.to_string()),
        blob_sha: blob_sha.map(str::to_string),
        message: message.to_string(),
    }
}

/// A file-scoped accusation: "this whole file is unreferenced". No symbol to
/// anchor identity to, so the content hash is the only anchor available.
fn file_finding(uri: &str, blob_sha: Option<&str>, message: &str) -> FingerprintInput {
    FingerprintInput {
        rule_id: "knip/unused-file".to_string(),
        artifact_uri: uri.to_string(),
        symbol: None,
        blob_sha: blob_sha.map(str::to_string),
        message: message.to_string(),
    }
}

#[test]
fn output_is_the_versioned_algorithm_name_and_64_lowercase_hex() {
    let fp = fingerprint(&symbol_finding(
        "pkg.models.Widget",
        None,
        "`Widget` is never referenced",
    ));

    let (algorithm, digest) = fp
        .split_once(':')
        .unwrap_or_else(|| panic!("fingerprint must be `<algorithm>:<digest>`, got {fp:?}"));

    assert_eq!(algorithm, FINGERPRINT_ALGORITHM, "algorithm name in {fp:?}");
    // §9.2's greatest-common-version matching only works if the version is
    // legible in the name, so the two constants must never drift apart.
    assert_eq!(
        FINGERPRINT_ALGORITHM,
        format!("judged/v{FINGERPRINT_VERSION}"),
        "the version numeral must be the one in the algorithm name"
    );
    assert_eq!(digest.len(), 64, "SHA-256 hex width in {fp:?}");
    assert!(
        digest
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
        "digest must be lowercase hex, got {digest:?}"
    );
}

#[test]
fn fingerprint_is_deterministic_across_calls() {
    let input = symbol_finding("pkg.models.Widget", Some("a1b2c3"), "unused");
    assert_eq!(fingerprint(&input), fingerprint(&input));
}

#[test]
fn v1_encoding_is_frozen() {
    // Baselines are committed (§9.4: `.cleaner/deletions.jsonl` — COMMITTED), so
    // the v1 encoding is an on-disk format. Silently changing it would orphan
    // every entry in every repository using Judged. The expected digest below
    // was computed independently of this implementation, from the documented
    // encoding: SHA-256 over little-endian-u64-length-prefixed fields, in the
    // order algorithm, rule_id, artifact_uri, symbol, blob_sha, normalized
    // message, with each optional field preceded by a 0x00/0x01 presence tag.
    //
    // If this test fails, the encoding changed: publish it as `judged/v2` and
    // leave v1 alone. Do not update the constant.
    let input = symbol_finding(
        "pkg.models.Widget",
        None,
        "`Widget` is never referenced (src/pkg/models.py:10:5)",
    );

    assert_eq!(
        fingerprint(&input),
        "judged/v1:5397a34b731ad4ce655845d8a18205083ad85df42a1b26ef54f8867bfb849f5b"
    );
}

#[test]
fn moving_a_finding_from_line_10_to_line_400_is_the_same_finding() {
    // The only thing that changed between these two runs is where in the file
    // the symbol now sits. §9.2: identity is never line-based.
    let at_line_10 = symbol_finding(
        "pkg.models.Widget",
        None,
        "`Widget` is never referenced (src/pkg/models.py:10:5)",
    );
    let at_line_400 = symbol_finding(
        "pkg.models.Widget",
        None,
        "`Widget` is never referenced (src/pkg/models.py:400:5)",
    );

    assert_eq!(
        fingerprint(&at_line_10),
        fingerprint(&at_line_400),
        "a line number must never reach the hash"
    );
}

#[test]
fn reformatting_a_file_does_not_reset_a_symbol_scoped_fingerprint() {
    // A formatter re-indented the file: every line moved, so the diagnostic's
    // embedded position changed *and* the git blob SHA changed. The symbol and
    // the finding did not. §9.2: "or every reformat resets the stability clock."
    let before_reformat = symbol_finding(
        "pkg.models.Widget",
        Some("0f4b1e2a3c5d6e7f8091a2b3c4d5e6f708192a3b"),
        "`Widget` is never referenced (src/pkg/models.py:10:5)",
    );
    let after_reformat = symbol_finding(
        "pkg.models.Widget",
        Some("9e8d7c6b5a4938271605f4e3d2c1b0a998877665"),
        "`Widget` is never referenced (src/pkg/models.py:412:9)",
    );

    assert_eq!(
        fingerprint(&before_reformat),
        fingerprint(&after_reformat),
        "reformatting must not reset the stability clock for a symbol-scoped finding"
    );
}

#[test]
fn blob_sha_is_ignored_when_a_symbol_anchors_the_finding() {
    // Half one of the documented resolution: when a symbol is present it is the
    // identity anchor, and blob_sha — which changes on every edit anywhere in
    // the file — is deliberately excluded. A caller that happens to have the
    // blob on hand must not destabilize the id by passing it.
    let without_blob = symbol_finding("pkg.models.Widget", None, "unused");
    let with_blob = symbol_finding("pkg.models.Widget", Some("deadbeef"), "unused");
    let with_other_blob = symbol_finding("pkg.models.Widget", Some("cafebabe"), "unused");

    assert_eq!(fingerprint(&without_blob), fingerprint(&with_blob));
    assert_eq!(fingerprint(&with_blob), fingerprint(&with_other_blob));
}

#[test]
fn blob_sha_participates_when_there_is_no_symbol() {
    // Half two: a file-scoped finding has no symbol, so without the blob its id
    // would be derived from a path — not from content at all, which is exactly
    // what §9.2 forbids. Here the blob is the anchor, and its changing is
    // *correct*: "this whole file is dead" is a claim about the file's content,
    // so new content is a new claim that has to re-earn its stability window.
    let original = file_finding(
        "src/legacy/report.ts",
        Some("1111111111111111111111"),
        "Unused file",
    );
    let edited = file_finding(
        "src/legacy/report.ts",
        Some("2222222222222222222222"),
        "Unused file",
    );
    let untracked = file_finding("src/legacy/report.ts", None, "Unused file");

    assert_ne!(
        fingerprint(&original),
        fingerprint(&edited),
        "a file-scoped finding is anchored to the file's content"
    );
    assert_ne!(
        fingerprint(&original),
        fingerprint(&untracked),
        "an absent blob is a distinct identity from any present blob"
    );
}

#[test]
fn every_field_that_participates_changes_the_fingerprint() {
    let base = symbol_finding("pkg.models.Widget", None, "unused");
    let baseline = fingerprint(&base);

    let mut other_rule = base.clone();
    other_rule.rule_id = "ruff/F841".to_string();
    assert_ne!(baseline, fingerprint(&other_rule), "rule_id");

    let mut other_uri = base.clone();
    other_uri.artifact_uri = "src/pkg/views.py".to_string();
    assert_ne!(baseline, fingerprint(&other_uri), "artifact_uri");

    let mut other_symbol = base.clone();
    other_symbol.symbol = Some("pkg.models.Gadget".to_string());
    assert_ne!(baseline, fingerprint(&other_symbol), "symbol");

    let mut other_message = base.clone();
    other_message.message = "never referenced".to_string();
    assert_ne!(baseline, fingerprint(&other_message), "message");
}

#[test]
fn an_absent_symbol_is_not_an_empty_symbol() {
    // A file-scoped finding and a finding about a symbol whose name happens to
    // serialize as "" are different accusations and must not share an id.
    let mut absent = file_finding("src/a.py", None, "unused");
    absent.rule_id = "r".to_string();
    let mut empty = absent.clone();
    empty.symbol = Some(String::new());

    assert_ne!(fingerprint(&absent), fingerprint(&empty));
}

#[test]
fn concatenation_ambiguity_does_not_collide() {
    // The classic canonical-encoding bug: "ab" + "c" hashing the same as
    // "a" + "bc". Field boundaries must be unambiguous in the hashed encoding.
    let left = FingerprintInput {
        rule_id: "ab".to_string(),
        artifact_uri: "c".to_string(),
        symbol: None,
        blob_sha: None,
        message: String::new(),
    };
    let right = FingerprintInput {
        rule_id: "a".to_string(),
        artifact_uri: "bc".to_string(),
        symbol: None,
        blob_sha: None,
        message: String::new(),
    };

    assert_ne!(fingerprint(&left), fingerprint(&right));
}

/// SplitMix64. A deterministic generator so a property failure is reproducible
/// from the seed alone; the workspace has no proptest dependency and this needs
/// no shrinking to be actionable.
struct SplitMix64(u64);

impl SplitMix64 {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn pick<'a, T>(&mut self, choices: &'a [T]) -> &'a T {
        let index = (self.next_u64() % choices.len() as u64) as usize;
        &choices[index]
    }
}

/// The identity a fingerprint is supposed to be a function of, spelled out
/// independently of the implementation: blob_sha only counts when there is no
/// symbol, and the message only counts after normalization.
type EffectiveIdentity = (String, String, Option<String>, Option<String>, String);

fn effective_identity(input: &FingerprintInput) -> EffectiveIdentity {
    let blob = match input.symbol {
        Some(_) => None,
        None => input.blob_sha.clone(),
    };
    (
        input.rule_id.clone(),
        input.artifact_uri.clone(),
        input.symbol.clone(),
        blob,
        normalize_message(&input.message),
    )
}

#[test]
fn fingerprinting_is_injective_over_generated_findings() {
    // Property: two findings share a fingerprint if and only if they share an
    // effective identity. A false collision would silently merge two
    // accusations in the ratchet ledger — one of them would be permanently
    // baselined by the other's approval.
    const SEED: u64 = 0x4A75_6467_6564_0001;
    let mut rng = SplitMix64(SEED);

    let rules = [
        "ruff/F401",
        "ruff/F841",
        "knip/unused-file",
        "knip/unused-export",
    ];
    let uris = [
        "src/a.py",
        "src/b.py",
        "src/a.ts",
        "src/nested/a.py",
        "a.py",
    ];
    let symbols = [None, Some("a"), Some("a.b"), Some("b"), Some("")];
    let blobs = [None, Some("1111"), Some("2222"), Some("")];
    let bodies = [
        "`os` imported but unused",
        "Local variable `x` is assigned to but never used",
        "Unused export `formatDate`",
    ];

    let mut by_fingerprint: HashMap<String, EffectiveIdentity> = HashMap::new();
    let mut by_identity: HashMap<EffectiveIdentity, String> = HashMap::new();

    for _ in 0..20_000 {
        // The trailing position is deliberate: raw messages differ constantly
        // while the finding does not, so this exercises the normalization edge
        // of the property, not just the hashing edge.
        let message = format!(
            "{} ({}:{}:{})",
            rng.pick(&bodies),
            rng.pick(&uris),
            rng.next_u64() % 5000,
            rng.next_u64() % 200,
        );
        let input = FingerprintInput {
            rule_id: rng.pick(&rules).to_string(),
            artifact_uri: rng.pick(&uris).to_string(),
            symbol: rng.pick(&symbols).map(str::to_string),
            blob_sha: rng.pick(&blobs).map(str::to_string),
            message,
        };

        let identity = effective_identity(&input);
        let fp = fingerprint(&input);

        // No two distinct findings may share an id.
        if let Some(previous) = by_fingerprint.get(&fp) {
            assert_eq!(
                *previous, identity,
                "seed {SEED:#x}: two distinct findings collided on {fp}"
            );
        }
        // ...and one finding may not acquire two ids, however it was spelled.
        if let Some(previous) = by_identity.get(&identity) {
            assert_eq!(
                *previous, fp,
                "seed {SEED:#x}: one finding produced two ids for {identity:?}"
            );
        }
        by_fingerprint.insert(fp.clone(), identity.clone());
        by_identity.insert(identity, fp);
    }

    // Guard the guard: if normalization or hashing degenerated to a constant,
    // the loop above would pass vacuously.
    assert!(
        by_fingerprint.len() > 500,
        "expected many distinct identities, got {}",
        by_fingerprint.len()
    );
}
