//! cargo-shear (Rust) — `--format=json` stdout to [`SutVerdict`], and nothing else.
//!
//! §4.1 rates this tool's two capabilities very differently. Unused-dependency
//! detection is ordinary. Detection of **unlinked `.rs` files** — files on disk
//! that no `mod` declaration reaches — is *"near-proof because Rust requires
//! explicit `mod`"*, and §4.1 names it **the strongest file-level signal in any
//! language surveyed**. That capability is the reason this adapter exists, and
//! it is the one that maps onto [`SutVerdict::claimed_dead_paths`].
//!
//! # Captured, not remembered
//!
//! `cargo shear --format=json` writes one pretty-printed JSON object to stdout.
//! Cargo's own chatter (`Updating crates.io index`, `Downloaded …`) goes to
//! **stderr**, so stdout is the document and nothing else. Run inside the
//! materialized m17 fixture:
//!
//! ```text
//! {
//!   "summary": {
//!     "errors": 0,
//!     "warnings": 1,
//!     "fixed": 0
//!   },
//!   "findings": [
//!     {
//!       "code": "shear/unlinked_files",
//!       "severity": "warning",
//!       "message": "1 unlinked file in `schema-migrator`\nsrc/checksum_v1.rs",
//!       "help": "delete this file",
//!       "fixable": false
//!     }
//!   ]
//! }
//! ```
//!
//! Two structural facts fall out of that shape and both drive this module.
//!
//! **The paths live inside `message`, not in a field.** `file` and `location`
//! are `skip_serializing_if = "Option::is_none"` and both are `None` for the
//! file-level classes, because a "delete this file" diagnostic points at no
//! source span. The paths are the second and subsequent **lines of the message
//! string**, behind a JSON `\n` escape. A parser that does not decode string
//! escapes cannot recover a single path.
//!
//! **A failed run still writes to stdout.** In a directory with no
//! `Cargo.toml`, cargo-shear exits 2 and stdout holds 63 bytes of prose where
//! the document belongs:
//!
//! ```text
//! error: Metadata error: `cargo metadata` exited with an error:
//! ```
//!
//! That is §6.20 exactly — a total failure whose stdout a tolerant parser would
//! read as a clean repository. [`parse_output`] rejects it by name.
//!
//! # Running it is not read-only, even without `--fix`
//!
//! See [`READ_ONLY_CAVEAT`]. Everything in this module is a pure function of a
//! string — no process is spawned and no file is read, so the *adapter* obeys
//! §9.2 rule 1 unconditionally and is fully testable with cargo-shear absent.
//! The *invocation* does not: `cargo shear` starts by running `cargo metadata`,
//! which resolves the dependency graph and writes `Cargo.lock`. Observed, not
//! inferred — after one `--format=json` run in each materialized Rust fixture,
//! `git status` in every one reported an untracked `Cargo.lock`.
//!
//! # Provenance of the captures
//!
//! Every `CAPTURED_*` constant in the tests below is real stdout from a real
//! run on 2026-08-01, not a reconstruction. They have two distinct origins, and
//! the difference matters enough to write down.
//!
//! Most came from cargo-shear **rebuilt from its crates.io source** in a
//! scratch directory. cargo-shear cannot be installed by the toolchain that
//! builds this repository — it pulls `ra_ap_syntax` and friends, which require
//! rustc 1.95, while `rust-toolchain.toml` here pins 1.94 — so at the time
//! those captures were taken no released binary was available. That rebuild
//! prints `Version: dev`, because the version string is injected by the release
//! pipeline rather than compiled in.
//!
//! Afterwards the **released 1.13.3 binary** was installed successfully via the
//! nightly toolchain, which closes the gap the paragraph above could not:
//! `CAPTURED_RELEASED_1_13_3` is its output, and the parser handles it
//! unchanged. So the shape is confirmed against a real release, not only
//! against source built locally.
//!
//! An earlier revision of this paragraph stated flatly that cargo-shear "is not
//! installed on this machine". That was true when written and false an hour
//! later. It is corrected rather than deleted because the constraint it
//! describes is real and load-bearing: **an analyzer that needs a newer
//! compiler than the project it analyzes cannot be installed by that project's
//! own toolchain**, which is an operational limit on adapter coverage that
//! §7.6's ecosystem-mortality argument does not name.

use std::collections::BTreeSet;
use std::path::PathBuf;

use judged_core::{Error, Result};

use crate::mutant::Ecosystem;
use crate::sut::SutVerdict;

/// What running cargo-shear does to the repository, which is not nothing.
///
/// §9.2 rule 1 says adapters are read-only and the orchestrator owns 100% of
/// mutations. This module honours that absolutely — it spawns nothing. But a
/// caller wiring [`crate::sut::Sut::run`] around the real binary needs to know
/// that the *invocation* mutates the tree even in its reporting mode, because
/// a harness that fingerprints a fixture after the run, or asserts a clean
/// `git status`, will see a difference the tool caused and attribute it to the
/// mutant.
pub const READ_ONLY_CAVEAT: &str = "\
`cargo shear --format=json` is NOT read-only, despite emitting only a report. \
It runs `cargo metadata` first, which resolves the dependency graph and WRITES \
`Cargo.lock` into the package. Observed 2026-08-01: after one --format=json run \
in each of the six materialized Rust fixtures, `git status` in every one showed \
an untracked Cargo.lock that the fixture's own commit did not contain. It may \
also hit the network to update the crates.io index and download sources.

Consequences for a caller: run it against a COPY of the fixture, or capture the \
tree's fingerprint before invoking, or add Cargo.lock to the comparison's \
ignore set — and never treat a post-run diff as evidence about the mutant. \
Nothing in this module does any of that, because nothing in this module runs \
the tool; this constant exists so the layer that does cannot miss it.";

/// What cargo-shear can and cannot say, in the form §9.2 requires of every
/// adapter.
///
/// An envelope declares what a tool structurally **cannot say**, so the
/// orchestrator knows when its silence means anything. It is not a list of the
/// tool's known mistakes — that distinction is what keeps E2 measuring the tool
/// instead of excusing it, and it is enforced by
/// `the_envelope_declares_silence_and_never_excuses_a_false_positive`.
///
/// The entry that matters most for this round is (2). cargo-shear has no
/// symbol-level finding class *at all*. It cannot say `backfill_missing_avatars`
/// is dead, and it equally cannot say it is live. Its silence about m17 and m19
/// is therefore not a result about link-time registries or ABI exports; it is
/// the tool never having been asked a question it can answer.
/// The ecosystems cargo-shear can load a repository from, for
/// [`crate::sut::CommandSut::with_reads`].
///
/// cargo-shear asks `cargo metadata` for the workspace and reads the crate
/// sources it points at. With no `Cargo.toml` there is nothing for `cargo
/// metadata` to answer with and it never opens a file: measured 2026-08-01,
/// ``error: could not find `Cargo.toml` in <dir> or any parent directory`` and
/// exit 2. Exit 2 is also what a *broken* manifest produces, so it cannot be
/// declared healthy (§6.20).
pub const READS: &[Ecosystem] = &[Ecosystem::Rust];

pub const CAPABILITY_ENVELOPE: &str = "\
cargo-shear answers two questions and no others: is this dependency key used \
anywhere in this workspace's source, and is this .rs file reached by a `mod` \
declaration. Everything else is outside it, and its silence is not evidence.

Structurally cannot emit:

(1) Any finding about a non-Rust artifact, or about a Rust project cargo \
cannot enumerate. It begins by running `cargo metadata`; a repository that is \
not a cargo workspace produces no findings because it produces no run.

(2) Any finding about the liveness of a FUNCTION, TYPE, TRAIT, FIELD, CONST or \
any other symbol. There is no such diagnostic class. A `pub fn` with zero \
callers and a `pub fn` called from everywhere are equally invisible, so \
silence about a symbol carries no information in either direction. This is the \
entry that makes 'shear said nothing about it' NOT a pass.

(3) Any finding about a file that IS named by a `mod` declaration, however \
dead its contents. Reachability here means reached-by-`mod`, not used; the \
only exception is `shear/empty_files`, which fires when such a file declares \
no items at all.

(4) Any import that only exists after macro expansion. It parses with \
rust-analyzer's parser and does not expand macros without `--expand`, which \
requires nightly and `cargo expand`. A dependency reached solely through a \
macro-generated path is therefore outside what it can see — and §6.1's Rust \
link-time registries (inventory, linkme, ctor, #[distributed_slice]) are \
exactly that shape.

(5) Any reason a symbol or file is live that is not spelled in this \
workspace's Rust source or manifests: a `#[no_mangle]` export consumed across \
an ABI boundary by an already-linked binary (§6.24), a `dlopen`ed plugin, a \
`build.rs`-generated reference, or a consumer in another repository. It has no \
class in which such a claim could be expressed.

Not listed above, on purpose: cargo-shear's known WRONG answers. §4.1 records \
that macro-expansion imports are invisible without --expand, which produces \
false 'unused dependency' claims rather than silence. Those are the tool \
saying something wrong, loudly. They are what E2 grades, and an envelope that \
declared them would be excusing the number this suite exists to produce.";

/// [`CAPABILITY_ENVELOPE`] in the shape [`crate::sut::Sut::cannot_emit`] wants:
/// one prose class per entry.
#[must_use]
pub fn cannot_emit() -> Vec<String> {
    [
        "any finding about a non-Rust artifact: it enumerates targets with `cargo metadata`, so \
         anything that is not a cargo workspace produces no findings because it produces no run",
        "any finding about the liveness of a symbol — function, type, trait, field or const: \
         cargo-shear has no symbol-level diagnostic class at all, so an uncalled symbol and a \
         hot one are equally invisible and its silence about either is not evidence",
        "any finding about a file that a `mod` declaration names, however dead its contents: \
         reachability here means reached-by-`mod`, not used, and only a file declaring no items \
         at all is reported (as `shear/empty_files`)",
        "any import that exists only after macro expansion: it parses with rust-analyzer and does \
         not expand macros without `--expand` (nightly, via `cargo expand`), so a dependency used \
         solely through a macro-generated path is outside what it can see",
        "any reason for liveness that is not written in this workspace's Rust source or \
         manifests: an ABI-exported symbol linked into a consumer that was never rebuilt, a \
         dlopen'd plugin, a build.rs-generated reference, or a caller in another repository",
    ]
    .iter()
    .map(|class| (*class).to_string())
    .collect()
}

/// Which half of [`SutVerdict`] a cargo-shear finding is allowed to fill, and
/// why. Printed in the report so nobody has to read this source to know which
/// grading they are looking at.
pub const MAPPING_DECISION: &str = "\
cargo-shear emits two kinds of finding that could bear on a verdict, and this \
adapter treats them differently.

FILE-LEVEL (`shear/unlinked_files`, `shear/empty_files`) -> one \
claimed_dead_paths entry per path. This is the capability §4.1 calls \
near-proof, and it is the only thing graded here.

DEPENDENCY-LEVEL (`shear/unused_dependency` and its five siblings) -> NO \
claim, in either field. A manifest key is not a file and not a symbol, and \
SutVerdict has no field for it; putting `libc` into claimed_dead_symbols would \
assert that cargo-shear called a SYMBOL dead, which it never did. \
`dependency_claims` returns them so a report can print what the tool actually \
said.

claimed_dead_symbols is therefore ALWAYS EMPTY. Per the capability envelope \
that is not this adapter declining to translate — cargo-shear has no \
symbol-level finding class, so there is nothing to translate.

Three consequences, all stated because each biases the score in cargo-shear's \
favour, making the reported false-removal count a LOWER BOUND.

(1) PATHS ARE PACKAGE-RELATIVE, NOT REPO-RELATIVE. `UnlinkedFile::path` is \
rebased onto the package directory before rendering, and the JSON carries the \
package NAME with no `file` field and no package directory. In a single-package \
repository rooted at the repo root the two coincide, which is the case for \
every Rust mutant in this suite. In a WORKSPACE they do not: a finding printed \
as `src/leftover.rs` in package `alpha` is really `crates/alpha/src/leftover.rs`. \
Re-rooting would require this adapter to run `cargo metadata` itself and emit a \
path cargo-shear never printed, so it does not. Every claimed_dead_paths entry \
is therefore package-relative, carried verbatim, with the package name \
available alongside it via `file_claims`; a caller must not read one as \
repo-relative without resolving the package directory first. The cost is real: \
in a workspace, a false removal may fail to match ground truth and go UNCOUNTED.

(2) A false 'unused dependency' claim on a LIVE dependency is ungraded. That \
gap sits directly on top of the highest-risk Rust hazard in the research: §6.1 \
names inventory/linkme/ctor, whose dependency is reachable only through a \
macro, and §4.1 records that macro-expansion imports are invisible without \
--expand. The one place cargo-shear is documented to be wrong about Rust is \
the one place this mapping cannot score it.

(3) `shear/misplaced_dependency` and `shear/unused_feature_dependency` claim \
no removal at all — the first says MOVE to dev-dependencies, the second says \
the key is referenced only from [features]. They are parsed and classified, and \
`dependency_claims` excludes them, because reporting a 'move' as a removal \
would invent a claim.";

/// `summary` — the three aggregate counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Summary {
    /// Error-severity findings.
    pub errors: usize,
    /// Non-error findings (warnings plus advice).
    pub warnings: usize,
    /// Findings rewritten on disk. Non-zero only under `--fix`, which this
    /// adapter refuses — see [`parse_output`].
    pub fixed: usize,
}

/// `severity` — one of miette's three levels, as cargo-shear spells them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// `"error"`. Unused and misplaced non-optional dependencies.
    Error,
    /// `"warning"`. Everything else, including both file-level classes.
    Warning,
    /// `"advice"`. Not emitted by any top-level finding today; accepted because
    /// [`crate::adapters`]'s rule 3 says a tool upgrade must not become a parse
    /// failure.
    Advice,
}

/// `location` — a byte range inside `file`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Location {
    /// Byte offset from the start of the file.
    pub offset: usize,
    /// Length in bytes. Zero points just before `offset`.
    pub length: usize,
}

/// Which of cargo-shear's two file-level classes a finding is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum FileReason {
    /// `shear/unlinked_files`: on disk, reached by no `mod` declaration. §4.1's
    /// near-proof signal.
    Unlinked,
    /// `shear/empty_files`: reached by a `mod`, but declaring no items.
    Empty,
}

impl FileReason {
    /// The word cargo-shear puts in the message for this class, from
    /// `DiagnosticKind::file_list_message`.
    const fn label(self) -> &'static str {
        match self {
            Self::Unlinked => "unlinked",
            Self::Empty => "empty",
        }
    }
}

/// What cargo-shear proposes doing to a dependency key. Distinguished because
/// "remove" and "move" are different claims and conflating them would overstate
/// the tool — see [`MAPPING_DECISION`] consequence (3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DependencyVerb {
    /// The key is unused and cargo-shear says to remove it.
    Remove,
    /// The key is used only by dev targets; cargo-shear says to move it, not to
    /// delete it.
    Relocate,
    /// The key is referenced only from `[features]`. Informational; cargo-shear
    /// offers no fix.
    FeatureOnly,
}

/// What one finding claims, in the terms [`SutVerdict`] is expressed in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShearClaim {
    /// A file-level claim.
    Files {
        /// Unlinked or empty.
        reason: FileReason,
        /// The cargo package name. **Not a directory** — see
        /// [`MAPPING_DECISION`] consequence (1).
        package: String,
        /// **Package-relative** paths, in the order cargo-shear printed them
        /// (which is `BTreeSet` order, so sorted).
        paths: Vec<PathBuf>,
    },
    /// A manifest-level claim naming a dependency key.
    Dependency {
        /// The dependency key as written in the manifest.
        name: String,
        /// What cargo-shear proposes doing to it.
        verb: DependencyVerb,
    },
    /// A diagnostic that claims neither a file nor a dependency: the `ignored`
    /// bookkeeping classes, the `test`/`doctest` target classes, and any class
    /// a future release adds.
    Neither,
}

/// One entry of the `findings` array.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShearFinding {
    /// `code`, verbatim, e.g. `shear/unlinked_files`.
    pub code: String,
    /// `severity`.
    pub severity: Severity,
    /// `message`, with JSON escapes decoded — so a file-level message really
    /// does contain newlines.
    pub message: String,
    /// `file`. Absent on both file-level classes.
    pub file: Option<PathBuf>,
    /// `location`. Absent on both file-level classes.
    pub location: Option<Location>,
    /// `help`. Absent where cargo-shear declines to suggest a fix, which it
    /// does for optional dependencies because removing one can be a breaking
    /// change.
    pub help: Option<String>,
    /// `fixable` — whether `--fix` could repair it. Carried for display.
    /// **Nothing in this module branches on it**: a claim cargo-shear made is
    /// graded whether or not the tool is willing to apply it itself.
    pub fixable: bool,
    /// The claim this finding makes, per [`MAPPING_DECISION`].
    pub claim: ShearClaim,
}

/// A whole `--format=json` document.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ShearOutput {
    /// `summary`.
    pub summary: Summary,
    /// `findings`, in emission order.
    pub findings: Vec<ShearFinding>,
}

/// The tool name every error from this module carries.
const TOOL: &str = "cargo-shear";

fn malformed(reason: impl Into<String>) -> Error {
    Error::Sut {
        sut: TOOL.to_string(),
        message: reason.into(),
    }
}

/// Parse a whole `cargo shear --format=json` stdout stream.
///
/// # Errors
///
/// Everything that is not a well-formed cargo-shear document, and three
/// contract violations that are *well-formed* JSON:
///
/// * **Empty stdout.** Unlike vulture, a successful run always writes an object
///   — a clean repository yields `{"summary":…,"findings":[]}`. Empty stdout
///   means no document was produced, and §6.20 requires that "no data" stay a
///   distinct state from "zero findings".
/// * **`summary.fixed` non-zero.** That only happens under `--fix`, which means
///   the tool rewrote the repository. [`crate::adapters`] rule 1 makes adapters
///   read-only, and a verdict computed from a mutated fixture describes a repo
///   that no longer exists.
/// * **`summary.errors + summary.warnings != findings.len()`.** cargo-shear
///   increments exactly one of those two counters per finding it pushes
///   (`ShearAnalysis::insert`), so the identity is invariant. A document where
///   it fails has lost findings, and a shorter finding list reads as a cleaner
///   repository.
pub fn parse_output(stdout: &str) -> Result<ShearOutput> {
    if stdout.trim().is_empty() {
        return Err(malformed(
            "stdout is empty. `cargo shear --format=json` always writes a JSON object — a clean \
             run writes {\"summary\":{...},\"findings\":[]} — so an empty stream means the \
             document was never produced, not that there is nothing to remove",
        ));
    }
    if let Some(line) = plain_text_error(stdout) {
        return Err(malformed(format!(
            "stdout is a plain-text error, not a JSON document: {line:?}. cargo-shear writes this \
             instead of the document when it cannot run at all and exits 2 (for example when \
             `cargo metadata` fails); check the exit status rather than treating this as a clean \
             repository"
        )));
    }

    let document = Json::parse(stdout).map_err(|reason| {
        malformed(format!(
            "stdout is not valid JSON: {reason}. Expected the object written by \
             `cargo shear --format=json`"
        ))
    })?;
    let root = document
        .object()
        .ok_or_else(|| malformed("the document is not a JSON object"))?;

    let summary = parse_summary(field(root, "summary")?)?;
    let findings_json = field(root, "findings")?
        .array()
        .ok_or_else(|| malformed("`findings` is not an array"))?;

    if summary.fixed != 0 {
        return Err(malformed(format!(
            "`summary.fixed` is {}, which only happens under `--fix`. Adapters are read-only \
             (§9.2): the orchestrator owns 100% of mutations, and a verdict computed after the \
             tool rewrote the repository describes a tree that no longer exists. Re-run \
             cargo-shear without `--fix`",
            summary.fixed
        )));
    }

    let counted = summary.errors + summary.warnings;
    if counted != findings_json.len() {
        return Err(malformed(format!(
            "`summary` counts {counted} findings ({} errors + {} warnings) but the `findings` \
             array holds {}. cargo-shear increments exactly one counter per finding it emits, so \
             the two cannot disagree in a complete document; this one lost findings, and a \
             shorter list reads as a cleaner repository",
            summary.errors,
            summary.warnings,
            findings_json.len()
        )));
    }

    let mut findings = Vec::with_capacity(findings_json.len());
    for (index, value) in findings_json.iter().enumerate() {
        findings.push(
            parse_finding(value)
                .map_err(|reason| malformed(format!("findings[{index}]: {reason}")))?,
        );
    }
    Ok(ShearOutput { summary, findings })
}

/// Recognize the shape cargo-shear writes to stdout when it fails outright, so
/// the error says so instead of "not valid JSON".
fn plain_text_error(stdout: &str) -> Option<&str> {
    let line = stdout.trim_start().lines().next()?;
    line.starts_with("error:").then_some(line)
}

fn field<'a>(object: &'a [(String, Json)], name: &str) -> Result<&'a Json> {
    object
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value)
        .ok_or_else(|| malformed(format!("the document has no `{name}` field")))
}

fn parse_summary(value: &Json) -> Result<Summary> {
    let object = value
        .object()
        .ok_or_else(|| malformed("`summary` is not an object"))?;
    let mut counts = [0usize; 3];
    for (index, name) in ["errors", "warnings", "fixed"].iter().enumerate() {
        counts[index] = field(object, name)?
            .usize()
            .ok_or_else(|| malformed(format!("`summary.{name}` is not a non-negative integer")))?;
    }
    Ok(Summary {
        errors: counts[0],
        warnings: counts[1],
        fixed: counts[2],
    })
}

fn parse_finding(value: &Json) -> std::result::Result<ShearFinding, String> {
    let object = value.object().ok_or("not an object")?;
    let get = |name: &str| object.iter().find(|(key, _)| key == name).map(|(_, v)| v);

    let code = get("code")
        .and_then(Json::string)
        .ok_or("no `code` string")?
        .to_string();
    let severity = match get("severity")
        .and_then(Json::string)
        .ok_or("no `severity` string")?
    {
        "error" => Severity::Error,
        "warning" => Severity::Warning,
        "advice" => Severity::Advice,
        other => return Err(format!("`severity` is {other:?}, not error/warning/advice")),
    };
    let message = get("message")
        .and_then(Json::string)
        .ok_or("no `message` string")?
        .to_string();
    let fixable = get("fixable")
        .and_then(Json::bool)
        .ok_or("no `fixable` boolean")?;

    let file = match get("file") {
        None => None,
        Some(value) => Some(PathBuf::from(
            value.string().ok_or("`file` is not a string")?,
        )),
    };
    let location = match get("location") {
        None => None,
        Some(value) => {
            let inner = value.object().ok_or("`location` is not an object")?;
            let read = |name: &str| -> std::result::Result<usize, String> {
                inner
                    .iter()
                    .find(|(key, _)| key == name)
                    .and_then(|(_, v)| v.usize())
                    .ok_or_else(|| format!("`location.{name}` is not a non-negative integer"))
            };
            Some(Location {
                offset: read("offset")?,
                length: read("length")?,
            })
        }
    };
    let help = match get("help") {
        None => None,
        Some(value) => Some(value.string().ok_or("`help` is not a string")?.to_string()),
    };

    let claim = parse_claim(&code, &message)?;
    Ok(ShearFinding {
        code,
        severity,
        message,
        file,
        location,
        help,
        fixable,
        claim,
    })
}

/// The diagnostic codes whose message names a dependency key, and what
/// cargo-shear proposes doing to it.
const DEPENDENCY_CODES: &[(&str, DependencyVerb)] = &[
    ("shear/unused_dependency", DependencyVerb::Remove),
    ("shear/unused_workspace_dependency", DependencyVerb::Remove),
    ("shear/unused_optional_dependency", DependencyVerb::Remove),
    (
        "shear/unused_feature_dependency",
        DependencyVerb::FeatureOnly,
    ),
    ("shear/misplaced_dependency", DependencyVerb::Relocate),
    (
        "shear/misplaced_optional_dependency",
        DependencyVerb::Relocate,
    ),
];

fn parse_claim(code: &str, message: &str) -> std::result::Result<ShearClaim, String> {
    let file_reason = match code {
        "shear/unlinked_files" => Some(FileReason::Unlinked),
        "shear/empty_files" => Some(FileReason::Empty),
        _ => None,
    };
    if let Some(reason) = file_reason {
        let (package, paths) = parse_file_list(reason, message)?;
        return Ok(ShearClaim::Files {
            reason,
            package,
            paths,
        });
    }
    if let Some((_, verb)) = DEPENDENCY_CODES.iter().find(|(known, _)| *known == code) {
        return Ok(ShearClaim::Dependency {
            name: backticked(message)?,
            verb: *verb,
        });
    }
    // An unrecognized code parses and claims nothing. Refusing it would turn a
    // cargo-shear upgrade into a hard failure; guessing at it would invent a
    // claim. See `an_unknown_diagnostic_code_parses_and_claims_nothing`.
    Ok(ShearClaim::Neither)
}

/// `{count} {label} file{s} in \`{package}\`` followed by one path per line,
/// from `DiagnosticKind::file_list_message`.
///
/// Every element is checked against every other: the count against the number
/// of path lines, the plural against the count, and the label against the
/// diagnostic code. A message that lost lines in transit disagrees with its own
/// count, and silently trusting the surviving lines would under-report the
/// claim — the same §6.20 failure as a truncated `findings` array, one level
/// down.
fn parse_file_list(
    reason: FileReason,
    message: &str,
) -> std::result::Result<(String, Vec<PathBuf>), String> {
    let (head, body) = message
        .split_once('\n')
        .ok_or("a file-list message must have a header line and at least one path line")?;

    let head = head
        .strip_suffix('`')
        .ok_or("the header line does not end with a backticked package name")?;
    let (prefix, package) = head
        .rsplit_once(" in `")
        .ok_or("the header line has no ` in `<package>`` segment")?;
    if package.is_empty() {
        return Err("the package name is empty".to_string());
    }

    let words: Vec<&str> = prefix.split(' ').collect();
    let [count, label, noun] = words[..] else {
        return Err(format!(
            "the header prefix is {prefix:?}, not `<count> <label> file(s)`"
        ));
    };
    if label != reason.label() {
        return Err(format!(
            "the message says {label:?} but the diagnostic code says {:?}",
            reason.label()
        ));
    }
    let count: usize = count
        .parse()
        .map_err(|_| format!("the leading count {count:?} is not a number"))?;
    let expected_noun = if count == 1 { "file" } else { "files" };
    if noun != expected_noun {
        return Err(format!(
            "a count of {count} should be followed by {expected_noun:?}, not {noun:?}"
        ));
    }

    let paths: Vec<&str> = body.split('\n').collect();
    if paths.len() != count {
        return Err(format!(
            "the message claims {count} file(s) and lists {}",
            paths.len()
        ));
    }
    if paths.iter().any(|path| path.is_empty()) {
        return Err("a path line is empty".to_string());
    }
    Ok((
        package.to_string(),
        paths.into_iter().map(PathBuf::from).collect(),
    ))
}

/// The single backticked span every dependency message carries. Requiring
/// exactly one pair keeps a reworded future message from silently yielding the
/// wrong name.
fn backticked(message: &str) -> std::result::Result<String, String> {
    let mut parts = message.split('`');
    let (Some(_), Some(name), Some(_), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(format!(
            "expected exactly one `backticked` dependency name in {message:?}"
        ));
    };
    if name.is_empty() {
        return Err("the dependency name is empty".to_string());
    }
    Ok(name.to_string())
}

/// The verdict the suite grades. See [`MAPPING_DECISION`].
///
/// # Errors
///
/// Propagates [`parse_output`].
pub fn verdict_from_stdout(stdout: &str) -> Result<SutVerdict> {
    Ok(verdict_from_output(&parse_output(stdout)?))
}

/// The same mapping, applied to an already-parsed document, for a caller that
/// also wants to print the findings.
#[must_use]
pub fn verdict_from_output(output: &ShearOutput) -> SutVerdict {
    // Sorted and deduplicated so a claim list can be diffed between runs.
    // Collapsing duplicates cannot hide a false removal: grading asks whether a
    // live path was claimed at all, not how often.
    let paths: BTreeSet<&PathBuf> = output
        .findings
        .iter()
        .filter_map(|finding| match &finding.claim {
            ShearClaim::Files { paths, .. } => Some(paths),
            ShearClaim::Dependency { .. } | ShearClaim::Neither => None,
        })
        .flatten()
        .collect();
    SutVerdict {
        claimed_dead_paths: paths.into_iter().cloned().collect(),
        // Always empty, and not because this adapter declined to translate:
        // cargo-shear has no symbol-level finding class. See CAPABILITY_ENVELOPE (2).
        claimed_dead_symbols: Vec::new(),
    }
}

/// Every dependency key cargo-shear said to **remove**, sorted and deduplicated.
///
/// This is the blast radius [`MAPPING_DECISION`] declines to grade: the manifest
/// lines a human acting on this run would have deleted. It is deliberately not
/// [`SutVerdict::claimed_dead_symbols`] — a manifest key is not a symbol, and
/// putting it there would claim on cargo-shear's behalf something it never said.
///
/// [`DependencyVerb::Relocate`] and [`DependencyVerb::FeatureOnly`] are excluded
/// because neither is a removal.
#[must_use]
pub fn dependency_claims(output: &ShearOutput) -> Vec<String> {
    let names: BTreeSet<&str> = output
        .findings
        .iter()
        .filter_map(|finding| match &finding.claim {
            ShearClaim::Dependency {
                name,
                verb: DependencyVerb::Remove,
            } => Some(name.as_str()),
            _ => None,
        })
        .collect();
    names.into_iter().map(str::to_string).collect()
}

/// Every `(package, package-relative path)` pair cargo-shear claimed, in
/// emission order.
///
/// The package name is carried because the path alone is ambiguous in a
/// workspace — see [`MAPPING_DECISION`] consequence (1). A caller holding a
/// package-name-to-directory map (from `cargo metadata`, which this adapter
/// deliberately does not run) can re-root these; this adapter will not, because
/// emitting a path cargo-shear never printed would be inventing a claim.
#[must_use]
pub fn file_claims(output: &ShearOutput) -> Vec<(String, PathBuf)> {
    output
        .findings
        .iter()
        .filter_map(|finding| match &finding.claim {
            ShearClaim::Files { package, paths, .. } => Some((package, paths)),
            ShearClaim::Dependency { .. } | ShearClaim::Neither => None,
        })
        .flat_map(|(package, paths)| {
            paths
                .iter()
                .map(move |path| (package.clone(), path.clone()))
        })
        .collect()
}

// ---------------------------------------------------------------------------
// A small strict JSON reader.
//
// `judged-mutants` does not depend on `serde_json` — only `judged-core` does,
// and it does not re-export it. Adding the dependency means editing
// `crates/judged-mutants/Cargo.toml`, which this round's file ownership does
// not permit. So the document is read here, strictly: no trailing content, no
// duplicate-key merging, a bounded nesting depth, and complete `\u` handling
// including surrogate pairs (a path is a filename and may hold anything).
// Swapping this for `serde_json` once the manifest can be edited would be a
// pure deletion.
// ---------------------------------------------------------------------------

/// Deeper than any cargo-shear document (which nests three levels) and shallow
/// enough that a hostile stream cannot overflow the stack.
const MAX_DEPTH: usize = 32;

#[derive(Debug, Clone, PartialEq)]
enum Json {
    Null,
    Bool(bool),
    /// Kept as written; only ever read as a `usize`.
    Number(String),
    String(String),
    Array(Vec<Json>),
    Object(Vec<(String, Json)>),
}

impl Json {
    fn parse(text: &str) -> std::result::Result<Self, String> {
        let mut reader = JsonReader {
            bytes: text.as_bytes(),
            pos: 0,
        };
        reader.skip_whitespace();
        let value = reader.value(0)?;
        reader.skip_whitespace();
        if reader.pos != reader.bytes.len() {
            return Err(format!(
                "trailing content after the document at byte {}",
                reader.pos
            ));
        }
        Ok(value)
    }

    fn object(&self) -> Option<&[(String, Self)]> {
        match self {
            Self::Object(entries) => Some(entries),
            _ => None,
        }
    }

    fn array(&self) -> Option<&[Self]> {
        match self {
            Self::Array(items) => Some(items),
            _ => None,
        }
    }

    fn string(&self) -> Option<&str> {
        match self {
            Self::String(text) => Some(text),
            _ => None,
        }
    }

    fn bool(&self) -> Option<bool> {
        match self {
            Self::Bool(value) => Some(*value),
            _ => None,
        }
    }

    /// Digits only. JSON permits `1e2` and `-0`; cargo-shear's counts are
    /// `usize` and serde writes them as bare digits, so anything else is not
    /// the document this parser is for.
    fn usize(&self) -> Option<usize> {
        match self {
            Self::Number(text) if !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()) => {
                text.parse().ok()
            }
            _ => None,
        }
    }
}

struct JsonReader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl JsonReader<'_> {
    fn skip_whitespace(&mut self) {
        while matches!(self.bytes.get(self.pos), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.pos += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn expect(&mut self, byte: u8) -> std::result::Result<(), String> {
        if self.peek() == Some(byte) {
            self.pos += 1;
            Ok(())
        } else {
            Err(format!(
                "expected {:?} at byte {}",
                char::from(byte),
                self.pos
            ))
        }
    }

    fn value(&mut self, depth: usize) -> std::result::Result<Json, String> {
        if depth > MAX_DEPTH {
            return Err(format!("nesting deeper than {MAX_DEPTH} levels"));
        }
        match self.peek() {
            Some(b'{') => self.object(depth),
            Some(b'[') => self.array(depth),
            Some(b'"') => Ok(Json::String(self.string()?)),
            Some(b't') => self.literal("true").map(|()| Json::Bool(true)),
            Some(b'f') => self.literal("false").map(|()| Json::Bool(false)),
            Some(b'n') => self.literal("null").map(|()| Json::Null),
            Some(_) => self.number(),
            None => Err("unexpected end of input".to_string()),
        }
    }

    fn literal(&mut self, word: &str) -> std::result::Result<(), String> {
        if self.bytes[self.pos..].starts_with(word.as_bytes()) {
            self.pos += word.len();
            Ok(())
        } else {
            Err(format!("expected `{word}` at byte {}", self.pos))
        }
    }

    fn number(&mut self) -> std::result::Result<Json, String> {
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        while matches!(
            self.peek(),
            Some(b'0'..=b'9' | b'.' | b'e' | b'E' | b'+' | b'-')
        ) {
            self.pos += 1;
        }
        if self.pos == start {
            return Err(format!("expected a value at byte {start}"));
        }
        let text = std::str::from_utf8(&self.bytes[start..self.pos])
            .map_err(|_| format!("invalid UTF-8 in a number at byte {start}"))?;
        if text.parse::<f64>().is_err() {
            return Err(format!("{text:?} at byte {start} is not a number"));
        }
        Ok(Json::Number(text.to_string()))
    }

    fn object(&mut self, depth: usize) -> std::result::Result<Json, String> {
        self.expect(b'{')?;
        let mut entries: Vec<(String, Json)> = Vec::new();
        self.skip_whitespace();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(Json::Object(entries));
        }
        loop {
            self.skip_whitespace();
            let key = self.string()?;
            if entries.iter().any(|(existing, _)| *existing == key) {
                return Err(format!("duplicate key {key:?}"));
            }
            self.skip_whitespace();
            self.expect(b':')?;
            self.skip_whitespace();
            let value = self.value(depth + 1)?;
            entries.push((key, value));
            self.skip_whitespace();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b'}') => {
                    self.pos += 1;
                    return Ok(Json::Object(entries));
                }
                _ => return Err(format!("expected ',' or '}}' at byte {}", self.pos)),
            }
        }
    }

    fn array(&mut self, depth: usize) -> std::result::Result<Json, String> {
        self.expect(b'[')?;
        let mut items = Vec::new();
        self.skip_whitespace();
        if self.peek() == Some(b']') {
            self.pos += 1;
            return Ok(Json::Array(items));
        }
        loop {
            self.skip_whitespace();
            items.push(self.value(depth + 1)?);
            self.skip_whitespace();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b']') => {
                    self.pos += 1;
                    return Ok(Json::Array(items));
                }
                _ => return Err(format!("expected ',' or ']' at byte {}", self.pos)),
            }
        }
    }

    fn string(&mut self) -> std::result::Result<String, String> {
        self.expect(b'"')?;
        let mut out = String::new();
        loop {
            let byte = self
                .peek()
                .ok_or_else(|| "unterminated string".to_string())?;
            match byte {
                b'"' => {
                    self.pos += 1;
                    return Ok(out);
                }
                b'\\' => {
                    self.pos += 1;
                    self.escape(&mut out)?;
                }
                0x00..=0x1F => {
                    return Err(format!("raw control byte {byte:#04x} inside a string"));
                }
                _ => {
                    let rest = std::str::from_utf8(&self.bytes[self.pos..])
                        .map_err(|_| format!("invalid UTF-8 at byte {}", self.pos))?;
                    let ch = rest
                        .chars()
                        .next()
                        .ok_or_else(|| "unterminated string".to_string())?;
                    out.push(ch);
                    self.pos += ch.len_utf8();
                }
            }
        }
    }

    fn escape(&mut self, out: &mut String) -> std::result::Result<(), String> {
        let byte = self
            .peek()
            .ok_or_else(|| "unterminated escape".to_string())?;
        self.pos += 1;
        let simple = match byte {
            b'"' => Some('"'),
            b'\\' => Some('\\'),
            b'/' => Some('/'),
            b'b' => Some('\u{8}'),
            b'f' => Some('\u{c}'),
            b'n' => Some('\n'),
            b'r' => Some('\r'),
            b't' => Some('\t'),
            b'u' => None,
            other => {
                return Err(format!("unknown escape `\\{}`", char::from(other)));
            }
        };
        if let Some(ch) = simple {
            out.push(ch);
            return Ok(());
        }

        let first = self.hex4()?;
        // A lone high surrogate must be followed by `\uDC00..=\uDFFF`; anything
        // else is not a code point and silently substituting U+FFFD would
        // corrupt a filename.
        let code = if (0xD800..0xDC00).contains(&first) {
            if !self.bytes[self.pos..].starts_with(b"\\u") {
                return Err(format!("high surrogate {first:#06x} with no low surrogate"));
            }
            self.pos += 2;
            let second = self.hex4()?;
            if !(0xDC00..0xE000).contains(&second) {
                return Err(format!(
                    "high surrogate {first:#06x} followed by {second:#06x}, not a low surrogate"
                ));
            }
            0x1_0000 + ((first - 0xD800) << 10) + (second - 0xDC00)
        } else if (0xDC00..0xE000).contains(&first) {
            return Err(format!("lone low surrogate {first:#06x}"));
        } else {
            first
        };
        out.push(char::from_u32(code).ok_or_else(|| format!("{code:#x} is not a code point"))?);
        Ok(())
    }

    fn hex4(&mut self) -> std::result::Result<u32, String> {
        let end = self.pos + 4;
        let digits = self
            .bytes
            .get(self.pos..end)
            .ok_or_else(|| "truncated `\\u` escape".to_string())?;
        let text = std::str::from_utf8(digits)
            .map_err(|_| "invalid UTF-8 in a `\\u` escape".to_string())?;
        let value = u32::from_str_radix(text, 16)
            .map_err(|_| format!("`\\u{text}` is not four hex digits"))?;
        self.pos = end;
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Only the tests name this type: cargo-shear makes no symbol claim, and the
    // assertions below are what hold it to that.
    use crate::sut::SymbolClaim;

    /// Captured by the maintainer on 2026-08-01 from the **released** cargo-shear
    /// 1.13.3 binary (installed via the nightly toolchain), on a two-file probe
    /// crate with one unused dependency and one unlinked file.
    ///
    /// It exists to settle a provenance question the rest of these captures
    /// cannot: the others came from a locally-rebuilt binary, so a release-only
    /// output difference would have been invisible. It is also the shape most
    /// likely to be got wrong — `shear/unlinked_files` carries NO `file` field,
    /// and its paths live inside `message` after a newline.
    const CAPTURED_RELEASED_1_13_3: &str = r#"{"summary":{"errors":1,"warnings":1,"fixed":0},"findings":[{"code":"shear/unused_dependency","severity":"error","message":"unused dependency `hex`","file":"Cargo.toml","location":{"offset":75,"length":3},"help":"remove this dependency","fixable":true},{"code":"shear/unlinked_files","severity":"warning","message":"1 unlinked file in `probe`\nsrc/unlinked_orphan.rs","help":"delete this file","fixable":false}]}"#;

    #[test]
    fn the_released_binary_output_parses_and_the_unlinked_path_survives() {
        let verdict =
            verdict_from_stdout(CAPTURED_RELEASED_1_13_3).expect("released 1.13.3 output parses");

        // The near-proof signal (§4.1: the strongest file-level signal in any
        // language surveyed). An adapter reading the `file` field would drop it
        // silently and make cargo-shear look like it says nothing about files.
        assert!(
            verdict
                .claimed_dead_paths
                .iter()
                .any(|p: &std::path::PathBuf| p.ends_with("src/unlinked_orphan.rs")),
            "the unlinked file must be claimed; got {:?}",
            verdict.claimed_dead_paths
        );
    }

    /// Captured verbatim from `cargo shear --format=json` run in the m17
    /// fixture (`schema-migrator`), cargo-shear 1.13.2 built from the crates.io
    /// source, 2026-08-01. stdout only; cargo's index chatter is on stderr.
    const CAPTURED_M17: &str = r#"{
  "summary": {
    "errors": 0,
    "warnings": 1,
    "fixed": 0
  },
  "findings": [
    {
      "code": "shear/unlinked_files",
      "severity": "warning",
      "message": "1 unlinked file in `schema-migrator`\nsrc/checksum_v1.rs",
      "help": "delete this file",
      "fixable": false
    }
  ]
}
"#;

    /// Captured verbatim from the m19 fixture (`ledger-abi`).
    const CAPTURED_M19: &str = r#"{
  "summary": {
    "errors": 0,
    "warnings": 1,
    "fixed": 0
  },
  "findings": [
    {
      "code": "shear/unlinked_files",
      "severity": "warning",
      "message": "1 unlinked file in `ledger-abi`\nsrc/deprecated_rounding.rs",
      "help": "delete this file",
      "fixable": false
    }
  ]
}
"#;

    /// Captured verbatim from the m06 fixture: two paths in one finding, and
    /// the plural `files` / `delete these files` wording.
    const CAPTURED_M06: &str = r#"{
  "summary": {
    "errors": 0,
    "warnings": 1,
    "fixed": 0
  },
  "findings": [
    {
      "code": "shear/unlinked_files",
      "severity": "warning",
      "message": "2 unlinked files in `m06-workqueue`\nsrc/orphan_backoff.rs\nsrc/unused_priority_lane.rs",
      "help": "delete these files",
      "fixable": false
    }
  ]
}
"#;

    /// Captured verbatim from a two-member workspace built in scratch:
    /// `crates/alpha` (unused `libc`, orphan `src/leftover.rs`) and
    /// `crates/beta` (orphan `src/deep/nested_orphan.rs`), plus an unused
    /// `[workspace.dependencies]` entry. This is the capture that pins the
    /// package-relative path hazard.
    const CAPTURED_WORKSPACE: &str = r#"{
  "summary": {
    "errors": 2,
    "warnings": 2,
    "fixed": 0
  },
  "findings": [
    {
      "code": "shear/unused_dependency",
      "severity": "error",
      "message": "unused dependency `libc`",
      "file": "crates/alpha/Cargo.toml",
      "location": {
        "offset": 76,
        "length": 4
      },
      "help": "remove this dependency",
      "fixable": true
    },
    {
      "code": "shear/unlinked_files",
      "severity": "warning",
      "message": "1 unlinked file in `alpha`\nsrc/leftover.rs",
      "help": "delete this file",
      "fixable": false
    },
    {
      "code": "shear/unlinked_files",
      "severity": "warning",
      "message": "1 unlinked file in `beta`\nsrc/deep/nested_orphan.rs",
      "help": "delete this file",
      "fixable": false
    },
    {
      "code": "shear/unused_workspace_dependency",
      "severity": "error",
      "message": "unused workspace dependency `serde_json`",
      "file": "Cargo.toml",
      "location": {
        "offset": 95,
        "length": 10
      },
      "help": "remove this dependency",
      "fixable": true
    }
  ]
}
"#;

    /// Captured verbatim: an optional dependency (no `help` key at all — it is
    /// `skip_serializing_if = "Option::is_none"`) and `shear/empty_files`.
    const CAPTURED_EXTRA_SHAPES: &str = r#"{
  "summary": {
    "errors": 0,
    "warnings": 2,
    "fixed": 0
  },
  "findings": [
    {
      "code": "shear/unused_optional_dependency",
      "severity": "warning",
      "message": "unused optional dependency `libc`",
      "file": "Cargo.toml",
      "location": {
        "offset": 83,
        "length": 4
      },
      "fixable": false
    },
    {
      "code": "shear/empty_files",
      "severity": "warning",
      "message": "2 empty files in `extra-shapes`\nsrc/alsoblank.rs\nsrc/blank.rs",
      "help": "delete these files",
      "fixable": false
    }
  ]
}
"#;

    /// Captured verbatim from a package with nothing wrong with it. Exit 0.
    const CAPTURED_CLEAN: &str = r#"{
  "summary": {
    "errors": 0,
    "warnings": 0,
    "fixed": 0
  },
  "findings": []
}
"#;

    /// Captured verbatim, all 63 bytes, from `cargo shear --format=json` in a
    /// directory with no `Cargo.toml`. Exit 2, and this goes to **stdout**
    /// where the JSON document would have been.
    const CAPTURED_METADATA_ERROR: &str =
        "error: Metadata error: `cargo metadata` exited with an error: \n";

    fn files(reason: FileReason, package: &str, paths: &[&str]) -> ShearClaim {
        ShearClaim::Files {
            reason,
            package: package.to_string(),
            paths: paths.iter().map(PathBuf::from).collect(),
        }
    }

    #[test]
    fn parses_the_captured_m17_run() {
        let output = parse_output(CAPTURED_M17).unwrap();
        assert_eq!(
            output.summary,
            Summary {
                errors: 0,
                warnings: 1,
                fixed: 0
            }
        );
        assert_eq!(output.findings.len(), 1);
        let finding = &output.findings[0];
        assert_eq!(finding.code, "shear/unlinked_files");
        assert_eq!(finding.severity, Severity::Warning);
        assert_eq!(
            finding.message, "1 unlinked file in `schema-migrator`\nsrc/checksum_v1.rs",
            "the \\n escape must be decoded, or the paths cannot be split off"
        );
        assert_eq!(finding.file, None);
        assert_eq!(finding.location, None);
        assert_eq!(finding.help.as_deref(), Some("delete this file"));
        assert!(!finding.fixable);
        assert_eq!(
            finding.claim,
            files(
                FileReason::Unlinked,
                "schema-migrator",
                &["src/checksum_v1.rs"]
            )
        );
    }

    #[test]
    fn parses_a_multi_path_file_finding() {
        let output = parse_output(CAPTURED_M06).unwrap();
        assert_eq!(
            output.findings[0].claim,
            files(
                FileReason::Unlinked,
                "m06-workqueue",
                &["src/orphan_backoff.rs", "src/unused_priority_lane.rs"]
            )
        );
    }

    #[test]
    fn parses_the_captured_workspace_run_including_locations() {
        let output = parse_output(CAPTURED_WORKSPACE).unwrap();
        assert_eq!(
            output.summary,
            Summary {
                errors: 2,
                warnings: 2,
                fixed: 0
            }
        );
        assert_eq!(output.findings.len(), 4);

        let dep = &output.findings[0];
        assert_eq!(dep.code, "shear/unused_dependency");
        assert_eq!(dep.severity, Severity::Error);
        assert_eq!(dep.file, Some(PathBuf::from("crates/alpha/Cargo.toml")));
        assert_eq!(
            dep.location,
            Some(Location {
                offset: 76,
                length: 4
            })
        );
        assert!(dep.fixable);
        assert_eq!(
            dep.claim,
            ShearClaim::Dependency {
                name: "libc".to_string(),
                verb: DependencyVerb::Remove
            }
        );

        assert_eq!(
            output.findings[3].claim,
            ShearClaim::Dependency {
                name: "serde_json".to_string(),
                verb: DependencyVerb::Remove
            }
        );
    }

    #[test]
    fn parses_empty_files_and_a_finding_with_no_help_key() {
        let output = parse_output(CAPTURED_EXTRA_SHAPES).unwrap();
        assert_eq!(
            output.findings[0].help, None,
            "`help` is skip_serializing_if=Option::is_none and must stay optional"
        );
        assert_eq!(
            output.findings[0].claim,
            ShearClaim::Dependency {
                name: "libc".to_string(),
                verb: DependencyVerb::Remove
            }
        );
        assert_eq!(
            output.findings[1].claim,
            files(
                FileReason::Empty,
                "extra-shapes",
                &["src/alsoblank.rs", "src/blank.rs"]
            )
        );
    }

    #[test]
    fn a_clean_run_is_an_empty_verdict_not_an_error() {
        let output = parse_output(CAPTURED_CLEAN).unwrap();
        assert_eq!(output.findings, Vec::new());
        assert_eq!(
            verdict_from_stdout(CAPTURED_CLEAN).unwrap(),
            SutVerdict::default()
        );
    }

    #[test]
    fn the_verdict_claims_the_unlinked_file_and_never_a_symbol() {
        let verdict = verdict_from_stdout(CAPTURED_M17).unwrap();
        assert_eq!(
            verdict.claimed_dead_paths,
            vec![PathBuf::from("src/checksum_v1.rs")]
        );
        assert_eq!(
            verdict.claimed_dead_symbols,
            Vec::<SymbolClaim>::new(),
            "shear has no symbol-level finding class at all"
        );
    }

    #[test]
    fn says_nothing_at_all_about_the_link_time_registry_symbol() {
        // §6.1's prediction, made executable. m17's live symbol is reached only
        // through `inventory::submit!`, so its call graph is genuinely empty.
        // shear emits nothing about it — and this test exists to record that
        // SILENCE IS NOT A PASS: the reason the verdict is empty of symbols is
        // that shear cannot make a symbol claim of any kind, which is a
        // declared envelope entry and not a judgement about m17.
        let verdict = verdict_from_stdout(CAPTURED_M17).unwrap();
        assert!(
            !verdict
                .claimed_dead_symbols
                .iter()
                .any(|s| s.name() == "backfill_missing_avatars"),
            "a false removal of the registered function"
        );
        assert!(
            !verdict
                .claimed_dead_paths
                .iter()
                .any(|p| p == std::path::Path::new("src/migrations/m0007.rs")),
            "a false removal of the file holding the registered function"
        );
        assert!(
            cannot_emit()
                .iter()
                .any(|class| class.contains("symbol") || class.contains("liveness")),
            "the empty symbol list is only meaningful if the envelope declares \
             that shear structurally cannot make a symbol claim"
        );
    }

    #[test]
    fn says_nothing_at_all_about_the_abi_exported_symbol() {
        // §6.24: m19's `ledger_amortize` is `#[no_mangle] extern "C"` and its
        // only consumer is outside the repository. Same shape of result, same
        // reason, same NOT-A-PASS marker.
        let verdict = verdict_from_stdout(CAPTURED_M19).unwrap();
        assert_eq!(
            verdict.claimed_dead_paths,
            vec![PathBuf::from("src/deprecated_rounding.rs")],
            "only the orphan decoy, never src/ffi.rs"
        );
        assert!(verdict.claimed_dead_symbols.is_empty());
    }

    #[test]
    fn a_dependency_finding_claims_no_path_and_no_symbol_but_is_still_reportable() {
        // MAPPING_DECISION, made executable: a manifest key is neither a file
        // nor a symbol, so it cannot enter SutVerdict without inventing a claim
        // shear never made. It must still be retrievable, or the adapter is
        // being MORE careful than the tool and quietly hiding its blast radius.
        let verdict = verdict_from_stdout(CAPTURED_WORKSPACE).unwrap();
        for name in ["libc", "serde_json"] {
            assert!(
                !verdict
                    .claimed_dead_symbols
                    .iter()
                    .any(|s| s.name() == name),
                "a dependency key leaked into claimed_dead_symbols: {name}"
            );
            assert!(
                !verdict
                    .claimed_dead_paths
                    .iter()
                    .any(|p| p.to_string_lossy().contains(name)),
                "a dependency key leaked into claimed_dead_paths: {name}"
            );
        }
        let output = parse_output(CAPTURED_WORKSPACE).unwrap();
        assert_eq!(dependency_claims(&output), vec!["libc", "serde_json"]);
    }

    #[test]
    fn a_move_is_never_reported_as_a_removal() {
        // MAPPING_DECISION consequence (3). `misplaced_dependency` says move it
        // to dev-dependencies; `unused_feature_dependency` says it is only
        // referenced from [features]. Neither is a claim that anything is dead.
        // Constructed from `DiagnosticKind::message` in cargo-shear 1.13.2, not
        // captured: producing them needs a manifest shape no fixture has.
        let document = r#"{
  "summary": { "errors": 1, "warnings": 1, "fixed": 0 },
  "findings": [
    {
      "code": "shear/misplaced_dependency",
      "severity": "error",
      "message": "misplaced dependency `tempfile`",
      "file": "Cargo.toml",
      "location": { "offset": 10, "length": 8 },
      "help": "move this dependency to `dev-dependencies`",
      "fixable": true
    },
    {
      "code": "shear/unused_feature_dependency",
      "severity": "warning",
      "message": "dependency `serde` only used in features",
      "file": "Cargo.toml",
      "location": { "offset": 40, "length": 5 },
      "fixable": false
    }
  ]
}"#;
        let output = parse_output(document).unwrap();
        assert_eq!(
            output.findings[0].claim,
            ShearClaim::Dependency {
                name: "tempfile".to_string(),
                verb: DependencyVerb::Relocate
            }
        );
        assert_eq!(
            output.findings[1].claim,
            ShearClaim::Dependency {
                name: "serde".to_string(),
                verb: DependencyVerb::FeatureOnly
            }
        );
        assert_eq!(
            dependency_claims(&output),
            Vec::<String>::new(),
            "a move and a feature-only note are not removals"
        );
        assert_eq!(verdict_from_output(&output), SutVerdict::default());
    }

    #[test]
    fn workspace_file_paths_are_package_relative_and_the_package_is_carried() {
        // THE HAZARD. `alpha` lives at `crates/alpha`, so the real
        // repo-relative path is `crates/alpha/src/leftover.rs`. shear's JSON
        // gives a package NAME and a package-RELATIVE path and never the
        // package directory, so the adapter cannot re-root without running
        // `cargo metadata` itself — which would be inventing a path shear never
        // printed. It carries what shear said, and MAPPING_DECISION states the
        // consequence.
        let output = parse_output(CAPTURED_WORKSPACE).unwrap();
        assert_eq!(
            file_claims(&output),
            vec![
                ("alpha".to_string(), PathBuf::from("src/leftover.rs")),
                (
                    "beta".to_string(),
                    PathBuf::from("src/deep/nested_orphan.rs")
                ),
            ]
        );
        let verdict = verdict_from_output(&output);
        assert_eq!(
            verdict.claimed_dead_paths,
            vec![
                PathBuf::from("src/deep/nested_orphan.rs"),
                PathBuf::from("src/leftover.rs"),
            ],
            "package-relative, exactly as shear printed them"
        );
        assert!(
            MAPPING_DECISION.contains("package-relative"),
            "the hazard must be stated where a report will print it"
        );
    }

    #[test]
    fn empty_stdout_is_an_error_and_never_a_clean_run() {
        // §6.20. Unlike vulture, a successful `--format=json` run ALWAYS writes
        // a JSON object — the clean capture above proves it. Empty stdout means
        // the process produced no document, which must not read as "nothing to
        // remove".
        for empty in ["", "   ", "\n\n"] {
            let parsed = parse_output(empty);
            assert!(
                parsed.is_err(),
                "empty stdout parsed as a clean run: {empty:?}"
            );
            assert!(verdict_from_stdout(empty).is_err());
        }
    }

    #[test]
    fn the_captured_metadata_error_is_rejected_and_named() {
        let error = parse_output(CAPTURED_METADATA_ERROR)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("Metadata error"),
            "the error must quote what shear actually printed, got: {error}"
        );
        assert!(
            error.contains("exit"),
            "the error should point at the exit code, got: {error}"
        );
        assert!(verdict_from_stdout(CAPTURED_METADATA_ERROR).is_err());
    }

    #[test]
    fn a_truncated_findings_array_is_an_error() {
        // §6.20's core case. `summary` and `findings` are produced by the same
        // pass in shear (`ShearAnalysis::insert` increments exactly one counter
        // per pushed finding), so errors + warnings == findings.len() always.
        // A document where they disagree is a document that lost findings, and
        // a shorter finding list reads as a cleaner repository.
        let truncated = r#"{
  "summary": { "errors": 2, "warnings": 2, "fixed": 0 },
  "findings": []
}"#;
        let error = parse_output(truncated).unwrap_err().to_string();
        assert!(
            error.contains("summary"),
            "the error must name the mismatch, got: {error}"
        );
        assert!(verdict_from_stdout(truncated).is_err());
    }

    #[test]
    fn a_file_message_whose_count_disagrees_with_its_paths_is_an_error() {
        let lying = r#"{
  "summary": { "errors": 0, "warnings": 1, "fixed": 0 },
  "findings": [
    {
      "code": "shear/unlinked_files",
      "severity": "warning",
      "message": "3 unlinked files in `alpha`\nsrc/one.rs",
      "help": "delete these files",
      "fixable": false
    }
  ]
}"#;
        assert!(
            parse_output(lying).is_err(),
            "a message claiming 3 files and listing 1 must not yield 1 claim"
        );
    }

    #[test]
    fn a_fix_run_is_refused_because_adapters_are_read_only() {
        // Adapter rule 1. `fixed` is non-zero only when `--fix` rewrote the
        // repository, which means the tool mutated the fixture out from under
        // the harness and the verdict describes a repo that no longer exists.
        let fixed = r#"{
  "summary": { "errors": 0, "warnings": 0, "fixed": 3 },
  "findings": []
}"#;
        let error = parse_output(fixed).unwrap_err().to_string();
        assert!(
            error.contains("--fix"),
            "the error must name the flag, got: {error}"
        );
        assert!(verdict_from_stdout(fixed).is_err());
    }

    #[test]
    fn malformed_documents_are_errors_never_an_empty_verdict() {
        let malformed = [
            // Truncated mid-write, e.g. a closed pipe.
            r#"{"summary": {"errors": 0, "warnings": 1, "fix"#,
            // Not an object.
            "[]",
            // No summary.
            r#"{"findings": []}"#,
            // No findings.
            r#"{"summary": {"errors": 0, "warnings": 0, "fixed": 0}}"#,
            // summary is not an object.
            r#"{"summary": 0, "findings": []}"#,
            // A severity shear does not have.
            r#"{"summary":{"errors":0,"warnings":1,"fixed":0},"findings":[{"code":"shear/empty_files","severity":"fatal","message":"1 empty file in `a`\nsrc/x.rs","fixable":false}]}"#,
            // fixable is not a bool.
            r#"{"summary":{"errors":0,"warnings":1,"fixed":0},"findings":[{"code":"shear/empty_files","severity":"warning","message":"1 empty file in `a`\nsrc/x.rs","fixable":"no"}]}"#,
            // A file message with no package backticks.
            r#"{"summary":{"errors":0,"warnings":1,"fixed":0},"findings":[{"code":"shear/unlinked_files","severity":"warning","message":"1 unlinked file\nsrc/x.rs","fixable":false}]}"#,
            // A dependency message with no backticked name.
            r#"{"summary":{"errors":1,"warnings":0,"fixed":0},"findings":[{"code":"shear/unused_dependency","severity":"error","message":"unused dependency","fixable":true}]}"#,
            // Trailing junk after the document.
            r#"{"summary":{"errors":0,"warnings":0,"fixed":0},"findings":[]} oops"#,
            // A file-list message with a header line and no path lines.
            r#"{"summary":{"errors":0,"warnings":1,"fixed":0},"findings":[{"code":"shear/empty_files","severity":"warning","message":"1 empty file in `a`","fixable":false}]}"#,
            // The label contradicts the diagnostic code.
            r#"{"summary":{"errors":0,"warnings":1,"fixed":0},"findings":[{"code":"shear/empty_files","severity":"warning","message":"1 unlinked file in `a`\nsrc/x.rs","fixable":false}]}"#,
            // Plural noun with a count of one.
            r#"{"summary":{"errors":0,"warnings":1,"fixed":0},"findings":[{"code":"shear/empty_files","severity":"warning","message":"1 empty files in `a`\nsrc/x.rs","fixable":false}]}"#,
            // An empty package name.
            r#"{"summary":{"errors":0,"warnings":1,"fixed":0},"findings":[{"code":"shear/empty_files","severity":"warning","message":"1 empty file in ``\nsrc/x.rs","fixable":false}]}"#,
            // Duplicate keys: which one wins is a coin flip, so refuse.
            r#"{"summary":{"errors":0,"warnings":0,"fixed":0},"findings":[],"findings":[]}"#,
        ];
        for document in malformed {
            assert!(
                parse_output(document).is_err(),
                "silently accepted a malformed document: {document}"
            );
            assert!(
                verdict_from_stdout(document).is_err(),
                "malformed input produced a verdict: {document}"
            );
        }
    }

    #[test]
    fn json_string_escapes_are_decoded() {
        // A path can legally contain a quote or a backslash, and shear escapes
        // both. Decoding is what turns the multi-line `message` into paths at
        // all, so an incomplete unescaper silently mis-keys every claim.
        let escaped = r#"{
  "summary": { "errors": 0, "warnings": 1, "fixed": 0 },
  "findings": [
    {
      "code": "shear/unlinked_files",
      "severity": "warning",
      "message": "2 unlinked files in `q\"uote`\nsrc\/sla\\sh.rs\nsrc/unié.rs",
      "help": "delete these files",
      "fixable": false
    }
  ]
}"#;
        let output = parse_output(escaped).unwrap();
        assert_eq!(
            output.findings[0].claim,
            files(
                FileReason::Unlinked,
                "q\"uote",
                &["src/sla\\sh.rs", "src/unié.rs"]
            )
        );
    }

    #[test]
    fn unicode_escapes_including_surrogate_pairs_are_decoded_or_refused() {
        // `\u` is the one escape a hand-rolled reader usually gets wrong, and
        // getting it wrong corrupts a filename rather than failing loudly.
        let document = |message: &str| {
            format!(
                r#"{{"summary":{{"errors":0,"warnings":1,"fixed":0}},"findings":[{{"code":"shear/empty_files","severity":"warning","message":"{message}","fixable":false}}]}}"#
            )
        };

        // A BMP escape and a surrogate pair (U+1F600) in a package name.
        let good = document(r"1 empty file in `café-😀`\nsrc/x.rs");
        let output = parse_output(&good).unwrap();
        assert_eq!(
            output.findings[0].claim,
            files(FileReason::Empty, "café-\u{1f600}", &["src/x.rs"])
        );

        for broken in [
            r"1 empty file in `\ud83d`\nsrc/x.rs",  // lone high surrogate
            r"1 empty file in `\ude00`\nsrc/x.rs",  // lone low surrogate
            r"1 empty file in `\ud83dA`\nsrc/x.rs", // high surrogate, then not a low one
            r"1 empty file in `\u00`\nsrc/x.rs",    // truncated escape
            r"1 empty file in `\uZZZZ`\nsrc/x.rs",  // not hex
        ] {
            assert!(
                parse_output(&document(broken)).is_err(),
                "a broken \\u escape was silently accepted: {broken}"
            );
        }
    }

    #[test]
    fn an_unknown_diagnostic_code_parses_and_claims_nothing() {
        // Forward compatibility, the §6.20 way round: a shear release that adds
        // a diagnostic class must not make the document unparseable (that would
        // turn a tool upgrade into a hard failure), and must not be guessed into
        // a claim either (that would invent one). It parses, and claims nothing.
        let future = r#"{
  "summary": { "errors": 0, "warnings": 1, "fixed": 0 },
  "findings": [
    {
      "code": "shear/some_future_lint",
      "severity": "advice",
      "message": "something new",
      "fixable": false
    }
  ]
}"#;
        let output = parse_output(future).unwrap();
        assert_eq!(output.findings[0].severity, Severity::Advice);
        assert_eq!(output.findings[0].claim, ShearClaim::Neither);
        assert_eq!(verdict_from_output(&output), SutVerdict::default());
    }

    #[test]
    fn fixable_never_changes_what_is_claimed() {
        // The cousin of vulture's confidence refusal. `fixable` is cargo-shear
        // saying whether IT would apply the change, not how sure it is that the
        // change is right — `unused_optional_dependency` is fixable:false purely
        // because removing an optional dependency can be a breaking change.
        // Filtering on it would silently drop real claims.
        let baseline = verdict_from_stdout(CAPTURED_WORKSPACE).unwrap();
        let flipped = CAPTURED_WORKSPACE
            .replace("\"fixable\": true", "\"fixable\": false")
            .replace("\"fixable\": false", "\"fixable\": true");
        assert_eq!(verdict_from_stdout(&flipped).unwrap(), baseline);
    }

    #[test]
    fn severity_never_changes_what_is_claimed() {
        // Same refusal for severity. Both file-level classes are `warning`, and
        // a run that reports only warnings exits 0 — so an adapter that keyed
        // off severity or exit code would discard every unlinked-file claim,
        // which is §4.1's strongest file-level signal in any language.
        let baseline = verdict_from_stdout(CAPTURED_M17).unwrap();
        let promoted = CAPTURED_M17.replace("\"warning\"", "\"error\"");
        let promoted = promoted.replace("\"warnings\": 1", "\"errors\": 1");
        let promoted = promoted.replace("\"errors\": 0", "\"warnings\": 0");
        assert_eq!(verdict_from_stdout(&promoted).unwrap(), baseline);
    }

    #[test]
    fn the_envelope_and_the_mapping_decision_are_reportable() {
        assert!(CAPABILITY_ENVELOPE.contains("silence is not evidence"));
        assert!(CAPABILITY_ENVELOPE.contains("--expand"));
        assert!(MAPPING_DECISION.contains("package-relative"));
        assert!(MAPPING_DECISION.contains("LOWER BOUND"));
    }

    #[test]
    fn the_read_only_caveat_names_the_file_the_tool_writes() {
        // §9.2 rule 1. This module spawns nothing, so it cannot enforce the
        // rule on the layer that does — it can only make the hazard impossible
        // to miss. The caveat has to name the artifact, or a caller cannot add
        // it to an ignore set.
        assert!(READ_ONLY_CAVEAT.contains("Cargo.lock"));
        assert!(READ_ONLY_CAVEAT.contains("NOT read-only"));
        assert!(
            READ_ONLY_CAVEAT.contains("cargo metadata"),
            "the caveat must say which step does the writing"
        );
    }

    #[test]
    fn nothing_in_this_module_touches_the_filesystem() {
        // The other half of rule 1, and the reason every test here runs with
        // cargo-shear uninstalled: the whole adapter is a pure function of a
        // string. This test is a canary — it passes trivially today and starts
        // failing the moment someone gives the parser a repo root to consult,
        // which is how "translate the tool" quietly becomes "re-run the tool".
        let before = std::env::current_dir().unwrap();
        let _ = verdict_from_stdout(CAPTURED_WORKSPACE).unwrap();
        let _ = parse_output(CAPTURED_METADATA_ERROR).unwrap_err();
        assert_eq!(std::env::current_dir().unwrap(), before);
    }

    #[test]
    fn the_envelope_comes_in_the_shape_the_sut_trait_asks_for() {
        let classes = cannot_emit();
        assert!(
            classes.len() >= 4,
            "an envelope of {} classes",
            classes.len()
        );
        let joined = classes.join("\n");
        assert!(joined.contains("Rust"));
        assert!(joined.contains("macro"));
        assert!(joined.contains("mod"));
    }

    #[test]
    fn the_envelope_declares_silence_and_never_excuses_a_false_positive() {
        // The vulture precedent. An envelope lists what the tool structurally
        // cannot SAY. §4.1's known cargo-shear wrong answers — a dependency
        // used only from an unexpanded macro, a build.rs-generated import — are
        // it saying something WRONG, and listing them here would excuse exactly
        // the number E2 exists to produce.
        let joined = cannot_emit().join("\n");
        for excuse in ["false positive", "prost", "may be wrong"] {
            assert!(
                !joined.contains(excuse),
                "the envelope excuses a false-positive mode: {excuse:?}"
            );
        }
        assert!(
            CAPABILITY_ENVELOPE.contains("saying something wrong"),
            "the prose envelope must say why the FP modes are absent from it"
        );
    }
}
