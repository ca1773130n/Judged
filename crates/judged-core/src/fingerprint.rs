//! Content-derived finding identity.
//!
//! §9.2: `partialFingerprints` uses "versioned hierarchical names with greatest
//! -common-version matching", which is what lets a ledger survive an
//! improvement to our own fingerprint algorithm — old entries keep matching on
//! the old key while new entries are written under the new one. And:
//! **fingerprints must be content-derived (symbol + normalized AST hash + blob
//! SHA), never line-based, or every reformat resets the stability clock.**
//!
//! Nothing in this module may read a line number. [`crate::sarif::Location`]
//! carries one for display; it is not an input here.
//!
//! # Resolving the blob-SHA tension
//!
//! §9.2 asks for two things that pull against each other. *Content-derived*
//! argues for mixing the git blob SHA into the id. *Never resets the stability
//! clock on a reformat* argues against it, because the blob SHA changes on any
//! edit anywhere in the file — a formatter run, an unrelated function two
//! hundred lines away, a trailing-newline fix.
//!
//! v1 resolves it mechanically rather than by convention: **`blob_sha`
//! participates in the hash if and only if `symbol` is `None`.**
//!
//! - **Symbol-scoped findings** ("`pkg.models.Widget` is never referenced")
//!   already have a content-derived anchor that survives reformatting: the
//!   fully-qualified symbol name. Adding the blob would make the id strictly
//!   less stable while adding nothing that distinguishes one finding from
//!   another, so it is excluded — and excluded by the algorithm, not by asking
//!   callers to remember to pass `None`. A caller that happens to have the blob
//!   on hand cannot destabilize the id by supplying it.
//! - **File-scoped findings** ("this whole file is unreferenced") have no
//!   symbol. Without the blob their id would be derived from a rule name and a
//!   path, which is not content-derived at all — precisely what §9.2 forbids.
//!   So the blob is the anchor, and the fact that it moves on every edit is the
//!   correct behaviour rather than a cost: the accusation is *about the whole
//!   file's content*, so different content is a different accusation and has to
//!   re-earn its stability window. This is the same instinct as §9.4's
//!   `subject_blob_sha` ("invalidate on content change") and its governing
//!   principle, "store evidence, never verdicts; re-derive every run".
//!
//! The third leg of the resolution is [`normalize_message`], which erases the
//! positions and timings tools bake into their own diagnostics. All three have
//! to hold for a reformat to be a no-op: the location is not an input, the blob
//! is not an input for symbol-scoped findings, and the embedded line numbers are
//! normalized out of the message.
//!
//! When a future version gains the normalized AST hash §9.2 actually asks for,
//! it becomes the symbol-scoped anchor and is added under `judged/v2` — old
//! entries keep matching on the v1 key, which is the entire point of versioning
//! the name.

use sha2::{Digest, Sha256};

/// Current algorithm version. Bump this and add a new key rather than changing
/// what an existing key means — that is the whole point of the versioning.
///
/// The numeral in [`FINGERPRINT_ALGORITHM`] is this value; they move together.
pub const FINGERPRINT_VERSION: u32 = 1;

/// The key under which a v1 fingerprint is stored in
/// [`crate::sarif::SarifResult::partial_fingerprints`].
pub const FINGERPRINT_ALGORITHM: &str = "judged/v1";

/// Substituted for a checkout-absolute path by [`normalize_message`].
const PATH_PLACEHOLDER: &str = "<path>";
/// Substituted for a line or column number by [`normalize_message`].
const NUMBER_PLACEHOLDER: &str = "<n>";
/// Substituted for a wall-clock duration by [`normalize_message`].
const DURATION_PLACEHOLDER: &str = "<dur>";

/// Punctuation stripped from a token's edges before it is inspected, so that
/// `(src/a.py:10:5)` and `src/a.py:10:5` are recognised as the same shape.
/// Interior punctuation is untouched — it is part of paths and symbol names.
const EDGE_PUNCTUATION: &[char] = &[
    ',', ';', '.', ':', '!', '?', '(', ')', '[', ']', '{', '}', '\'', '"', '`',
];

/// Words after which a bare integer is a position, not data.
const POSITION_KEYWORDS: &[&str] = &["line", "lines", "col", "cols", "column", "columns"];

/// Unit words that turn a preceding bare number into a duration (`45 ms`).
/// Deliberately excludes bare `m` and `h`, which are far more often something
/// else than they are minutes and hours.
const DURATION_UNIT_WORDS: &[&str] = &[
    "ns", "us", "µs", "ms", "s", "sec", "secs", "second", "seconds", "minute", "minutes", "hour",
    "hours",
];

/// Characters a unit suffix may be spelled with in an attached duration
/// (`1.234s`, `450ms`, `3m12s`).
const DURATION_UNIT_CHARS: &[char] = &['n', 'u', 'µ', 'm', 's', 'h'];

/// The inputs a fingerprint is derived from.
///
/// `symbol` and `blob_sha` are optional because not every accusation has them:
/// a repo-level finding has no symbol, and an untracked file has no blob. Their
/// absence weakens identity stability, which is a property of the finding, not
/// an error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FingerprintInput {
    /// Rule that produced the finding, scoped to the emitting tool.
    pub rule_id: String,
    /// Repo-relative URI of the artifact the finding is about.
    pub artifact_uri: String,
    /// Fully-qualified symbol name, when the finding is about a symbol.
    pub symbol: Option<String>,
    /// Git blob SHA of the artifact's content, when it is tracked.
    ///
    /// Hashed only when `symbol` is `None`; see the module documentation for
    /// why. Supplying it alongside a symbol is harmless and ignored.
    pub blob_sha: Option<String>,
    /// Finding text. Normalized through [`normalize_message`] before hashing so
    /// that a tool rewording its own diagnostic does not reset the clock.
    pub message: String,
}

/// Derive the stable identity of a finding.
///
/// Returns `judged/v1:<64 lowercase hex>` — the algorithm name is inside the
/// value as well as being the map key, so a fingerprint pasted into an issue or
/// a keep manifest is self-describing.
///
/// The digest is SHA-256 over a length-prefixed encoding of the participating
/// fields. Length prefixes are not decoration: plain concatenation would give
/// `rule_id = "ab", uri = "c"` and `rule_id = "a", uri = "bc"` the same id, and
/// two merged accusations in the ratchet ledger means one of them is
/// permanently baselined by the other's approval.
pub fn fingerprint(input: &FingerprintInput) -> String {
    let mut hasher = Sha256::new();

    // Domain separation: a v2 encoding that happens to serialize to the same
    // bytes must still not produce the same digest.
    absorb(&mut hasher, FINGERPRINT_ALGORITHM.as_bytes());
    absorb(&mut hasher, input.rule_id.as_bytes());
    absorb(&mut hasher, input.artifact_uri.as_bytes());
    absorb_optional(&mut hasher, input.symbol.as_deref());
    absorb_optional(&mut hasher, anchoring_blob_sha(input));
    absorb(&mut hasher, normalize_message(&input.message).as_bytes());

    format!("{FINGERPRINT_ALGORITHM}:{}", hex::encode(hasher.finalize()))
}

/// The blob SHA as far as identity is concerned: present only for file-scoped
/// findings. See the module documentation.
fn anchoring_blob_sha(input: &FingerprintInput) -> Option<&str> {
    if input.symbol.is_some() {
        None
    } else {
        input.blob_sha.as_deref()
    }
}

/// Absorb one field, length-prefixed so field boundaries are unambiguous.
fn absorb(hasher: &mut Sha256, field: &[u8]) {
    hasher.update((field.len() as u64).to_le_bytes());
    hasher.update(field);
}

/// Absorb an optional field. The presence tag keeps `None` distinct from
/// `Some("")`: "this finding has no symbol" and "this finding's symbol is the
/// empty string" are different claims.
fn absorb_optional(hasher: &mut Sha256, field: Option<&str>) {
    match field {
        None => hasher.update([0u8]),
        Some(value) => {
            hasher.update([1u8]);
            absorb(hasher, value.as_bytes());
        }
    }
}

/// Strip the parts of a diagnostic that describe the run rather than the
/// finding: line and column numbers, checkout-absolute paths, wall-clock
/// durations, and wrapping whitespace.
///
/// Hashing raw message text would reset the stability clock (§9.2) on every
/// reformat, on every CI runner whose workspace root differs from a developer's,
/// and on every run whose timings differ. What survives is what the finding is
/// *about*: the prose, the symbol names, and repo-relative paths — which are
/// stable across checkouts and are genuinely part of the accusation.
///
/// Erased positions become `<n>`, absolute paths `<path>`, durations `<dur>`.
/// The result is idempotent, so a normalized message can be stored and
/// re-normalized without drifting.
///
/// Counts are deliberately *not* erased. Blanket digit removal would merge
/// genuinely distinct findings, and a run-scoped total has no business in a
/// per-result message in the first place — that is what
/// [`crate::sarif::Invocation::tool_execution_notifications`] is for.
pub fn normalize_message(msg: &str) -> String {
    // Pass one: normalize each whitespace-separated token in isolation, with a
    // one-token lookback for the `line 12` / `column 5` prose form.
    let mut tokens: Vec<Token> = Vec::new();
    for raw in msg.split_whitespace() {
        let (leading, core, trailing) = split_edge_punctuation(raw);
        let previous_core = tokens.last().map(|t| t.core.as_str()).unwrap_or_default();
        tokens.push(Token {
            leading: leading.to_string(),
            core: normalize_core(core, previous_core),
            trailing: trailing.to_string(),
        });
    }

    // Pass two: fold `<number> <unit word>` into a single duration. It needs two
    // adjacent tokens, so it cannot be done in pass one.
    fold_spaced_durations(&mut tokens);

    tokens
        .iter()
        .map(Token::render)
        .collect::<Vec<_>>()
        .join(" ")
}

/// One whitespace-separated token, split so that edge punctuation survives
/// normalization of the interesting part.
struct Token {
    leading: String,
    core: String,
    trailing: String,
}

impl Token {
    fn render(&self) -> String {
        format!("{}{}{}", self.leading, self.core, self.trailing)
    }
}

fn normalize_core(core: &str, previous_core: &str) -> String {
    let without_position = strip_position_suffix(core);

    if is_absolute_path(without_position) {
        return PATH_PLACEHOLDER.to_string();
    }
    if is_attached_duration(without_position) {
        return DURATION_PLACEHOLDER.to_string();
    }
    if is_bare_integer(without_position) && is_position_keyword(previous_core) {
        return NUMBER_PLACEHOLDER.to_string();
    }
    without_position.to_string()
}

/// Remove a trailing `:<line>` or `:<line>:<column>` from a token.
///
/// This is the single most important rule in the normalizer: it is what makes a
/// reformat invisible. Bounded at two groups because a third would be data, and
/// refuses to strip when what remains is itself a bare number so that `12:30`
/// stays a clock rather than becoming `12`.
fn strip_position_suffix(core: &str) -> &str {
    let mut result = core;
    for _ in 0..2 {
        match strip_one_position(result) {
            Some(shorter) => result = shorter,
            None => break,
        }
    }
    result
}

fn strip_one_position(core: &str) -> Option<&str> {
    let (prefix, digits) = core.rsplit_once(':')?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if prefix.is_empty() || prefix.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(prefix)
}

/// A path rooted outside the repository. Erased because it encodes the checkout
/// location, which differs between a developer's machine and a CI runner while
/// the finding does not. Repo-relative paths are kept.
fn is_absolute_path(core: &str) -> bool {
    if core.len() > 1 && core.starts_with('/') {
        return true;
    }
    // Windows drive letter: `C:\...` or `C:/...`. SARIF is a cross-platform
    // interchange format, so adapters do produce these.
    let bytes = core.as_bytes();
    bytes.len() > 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/')
}

/// `1.234s`, `450ms`, `3m12s`: a digit-led token spelled only with digits, dots
/// and unit letters, ending in a unit. The digit-led requirement is what keeps
/// identifiers and rule codes (`F401`, `deadbeef`) out.
fn is_attached_duration(core: &str) -> bool {
    let Some(first) = core.chars().next() else {
        return false;
    };
    let Some(last) = core.chars().next_back() else {
        return false;
    };
    first.is_ascii_digit()
        && matches!(last, 's' | 'm' | 'h')
        && core
            .chars()
            .all(|c| c.is_ascii_digit() || c == '.' || DURATION_UNIT_CHARS.contains(&c))
}

fn is_bare_integer(core: &str) -> bool {
    !core.is_empty() && core.bytes().all(|b| b.is_ascii_digit())
}

/// `45`, `2.5` — the number half of a spaced duration.
fn is_decimal_number(core: &str) -> bool {
    let bytes = core.as_bytes();
    match (bytes.first(), bytes.last()) {
        (Some(first), Some(last)) => {
            first.is_ascii_digit()
                && last.is_ascii_digit()
                && bytes.iter().all(|b| b.is_ascii_digit() || *b == b'.')
        }
        _ => false,
    }
}

fn is_position_keyword(core: &str) -> bool {
    POSITION_KEYWORDS
        .iter()
        .any(|keyword| core.eq_ignore_ascii_case(keyword))
}

fn is_duration_unit_word(core: &str) -> bool {
    DURATION_UNIT_WORDS
        .iter()
        .any(|word| core.eq_ignore_ascii_case(word))
}

/// Collapse `45 ms` / `2.5 seconds` into a single `<dur>` token. Only fires when
/// the two tokens are adjacent with no intervening punctuation, so `45, ms` is
/// left alone.
fn fold_spaced_durations(tokens: &mut Vec<Token>) {
    let mut index = 1;
    while index < tokens.len() {
        let foldable = tokens[index].leading.is_empty()
            && is_duration_unit_word(&tokens[index].core)
            && tokens[index - 1].trailing.is_empty()
            && is_decimal_number(&tokens[index - 1].core);

        if foldable {
            let unit = tokens.remove(index);
            let number = &mut tokens[index - 1];
            number.core = DURATION_PLACEHOLDER.to_string();
            number.trailing = unit.trailing;
        } else {
            index += 1;
        }
    }
}

/// Split a token into (leading punctuation, core, trailing punctuation).
fn split_edge_punctuation(token: &str) -> (&str, &str, &str) {
    let after_leading = token.trim_start_matches(EDGE_PUNCTUATION);
    let leading = &token[..token.len() - after_leading.len()];
    let core = after_leading.trim_end_matches(EDGE_PUNCTUATION);
    let trailing = &after_leading[core.len()..];
    (leading, core, trailing)
}
