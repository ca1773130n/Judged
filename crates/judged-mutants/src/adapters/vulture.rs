//! Vulture (Python) — stdout to [`SutVerdict`], and nothing else.
//!
//! Vulture prints one finding per line, in exactly one shape. From
//! `vulture/core.py`, `Item.get_report` (v2.16):
//!
//! ```text
//! f"{utils.format_path(self.filename)}:{self.first_lineno:d}: "
//! f"{self.message} ({self.confidence}% confidence{size_report})"
//! ```
//!
//! so a run looks like this — captured, not paraphrased:
//!
//! ```text
//! sample/ledger/dunning.py:1: unused import 'os' (90% confidence)
//! sample/ledger/dunning.py:10: unused property 'grace' (60% confidence)
//! sample/ledger/dunning.py:24: unreachable code after 'return' (100% confidence)
//! sample/ledger/dunning.py:28: unsatisfiable 'while' condition (100% confidence)
//! sample/ledger/jobs.py:5: unused variable 'context' (100% confidence)
//! ```
//!
//! Everything here is a pure function of that text. No process is spawned, no
//! file is read, and the parser is fully testable with Vulture not installed.
//!
//! # Confidence is carried, never consulted
//!
//! The trailing percentage is parsed into [`VultureFinding::confidence`] so a
//! report can print what the tool said, and **nothing in this module branches on
//! it**. That is a deliberate refusal, not an oversight. §4.1 records where the
//! numbers come from — `DEFAULT_CONFIDENCE = 60`, 90 for imports, 100 for
//! arguments and unreachable code — and the "Claims to stop propagating" list in
//! §11 is explicit: they are hard-coded constants per AST node type, never
//! calibrated, and Vulture's own README calls sub-100 values *"very rough
//! estimates."* The failure mode is specific and known: shipped as a
//! probability-shaped number, users threshold on it. §4.1 also records
//! vulture#422, an open **100%-confidence** false positive, so the top of the
//! scale is not a safe threshold either. [`SutVerdict`] has no confidence field
//! for the same reason, and a filter here would be the same mistake one layer
//! earlier.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use judged_core::{Error, Result};

use crate::sut::SutVerdict;

/// What Vulture can and cannot say, in the form §9.2 requires of every adapter.
///
/// §9.2's first non-SARIF clause requires each adapter to declare the finding
/// classes it structurally cannot emit, "e.g. *vulture performs global name-set
/// difference and cannot see cross-module references; its silence is not
/// evidence*" — because that is what lets the orchestrator know when a tool
/// saying nothing means anything at all.
///
/// An envelope declares what a tool structurally **cannot say**. It is not a
/// list of the tool's known mistakes, and the difference decides what E2
/// measures. Vulture's §4.1 false-positive modes — every Django model field,
/// Pydantic fields on FastAPI, Flask's `@app.template_global`, `globals()`,
/// reflection — are vulture saying something *wrong*, loudly. They are the
/// entire thing this suite exists to count, and listing them here as declared
/// blind spots would excuse the measurement. They are named in the last
/// paragraph, as context for reading a report, and deliberately not as classes.
///
/// This is a public constant *and* [`cannot_emit`] because [`crate::sut::Sut`]
/// asks for a `Vec<String>` of classes while a report wants prose.
pub const CAPABILITY_ENVELOPE: &str = "\
vulture performs a global AST name-set difference and cannot see cross-module \
references; its silence is not evidence.

Structurally cannot emit:

(1) Any finding about a FILE. It reports unused names and never names a file, \
so however completely it empties one, it cannot claim the file is dead.

(2) Any finding about a name that occurs anywhere else in the scanned set. \
Vulture unions every used name into one global set and subtracts, so an \
unrelated occurrence of the same spelling in any file suppresses the finding. \
Silence about a name is therefore not evidence the name is live.

(3) Any finding about a non-Python artifact. Only files it parses as Python are \
scanned; a dead YAML task, CI step, Dockerfile stage or fixture is invisible.

(4) A calibrated confidence. DEFAULT_CONFIDENCE = 60, 90 for imports, 100 for \
arguments and unreachable code are hard-coded per AST node type, never \
measured; vulture's README calls sub-100 values \"very rough estimates\", and \
vulture#422 is an open 100%-confidence false positive, so neither end of the \
scale supports a threshold.

Not listed above, on purpose: the false-positive modes §4.1 measures at 44 true \
positives against 644 false positives across 9 popular repositories (~6% \
precision) — 59 on httpx, which contains zero dead items, 260 on Flask via \
@app.template_global, 102 on FastAPI via Pydantic fields, every Django model \
field, plus globals(), dataclasses, TypedDict and Protocol. Those are wrong \
answers, not silence. They are what E2 grades, and an envelope that declared \
them would be excusing the number this suite exists to produce.";

/// [`CAPABILITY_ENVELOPE`] in the shape [`crate::sut::Sut::cannot_emit`] wants:
/// one prose class per entry, so a report can list them and a `Sut` impl can
/// return them without restating anything.
pub fn cannot_emit() -> Vec<String> {
    [
        "any finding about a file: vulture reports unused names and never names a file, so \
         however completely it empties one it cannot claim the file is dead",
        "any finding about a name that occurs anywhere else in the scanned set: it subtracts \
         one global used-name set from one global defined-name set, so an unrelated \
         occurrence of the same spelling in any file suppresses the finding, and its silence \
         about a name is not evidence the name is live",
        "any finding about a non-Python artifact: only files it parses as Python are scanned, \
         so a dead YAML task, CI step, Dockerfile stage or fixture is invisible to it",
        "a calibrated confidence: 60/90/100 are hard-coded per AST node type and never \
         measured, so no threshold on them means anything",
    ]
    .iter()
    .map(|class| (*class).to_string())
    .collect()
}

/// Which half of [`SutVerdict`] a Vulture finding is allowed to fill, and why.
///
/// Vulture reports unused **names**. [`SutVerdict`] carries both
/// `claimed_dead_paths` and `claimed_dead_symbols`, so the adapter has to decide
/// when — if ever — a name-level finding justifies claiming a whole file. The
/// decision is stated here, and printed in the report, so nobody has to read
/// this source to know which grading they are looking at.
///
/// **Chosen: symbols only. `claimed_dead_paths` is always empty.**
///
/// The aggressive reading — infer "this file is dead" when its findings look
/// like they cover it — is closer to what a human running `vulture` and deleting
/// what it names actually does, and it is the only reading that would measure
/// Vulture's true blast radius. It is rejected for two reasons.
///
/// First, it is not derivable from the input. Vulture's stdout names findings;
/// it never states a file's total set of definitions. "All of this file is dead"
/// therefore cannot be computed from stdout — only guessed, from the shape of
/// what happens to have been reported.
///
/// Second, and decisive: E2 exists to answer §11 R1, whose pre-committed
/// consequence is that the auto-act tier is **deleted from the design rather
/// than tuned**. An answer that heavy has to be attributable to the analyzer. A
/// false removal manufactured by an inference layer we wrote would be
/// indistinguishable in the report from one Vulture made, and would corrupt the
/// only number this suite exists to produce.
///
/// The cost is real and is stated rather than hidden.
pub const MAPPING_DECISION: &str = "\
Vulture reports unused NAMES. This adapter maps each `unused <kind> '<name>'` \
finding to one claimed_dead_symbols entry and NEVER to a claimed_dead_paths \
entry: claimed_dead_paths is always empty.

Consequence, stated because it biases the score in Vulture's favour: this \
grading UNDER-reports Vulture's blast radius in two known ways.

(1) A mutant whose live artifact is a whole FILE can be caught here only \
through a symbol claim. A file that a human would delete after vulture emptied \
it is invisible to the grade. `files_touched` computes that set for a caller \
who wants it, but nothing renders it today, so the gap is real and currently \
unquantified rather than merely ungraded.

(2) `unreachable code after 'return'` and `unsatisfiable 'while' condition` \
name a Python keyword, not a symbol, and SutVerdict has no field for a code \
region, so they produce no claim at all. This is exactly the class of \
vulture#422 — an open 100%-confidence false positive where removing the flagged \
`yield` silently converts an async generator into a coroutine.

Both gaps make the reported false-removal count a LOWER BOUND on what deleting \
what vulture names would do.";

/// The kind of thing one Vulture line reports.
///
/// The split that matters is whether the quoted token is a *symbol* or a
/// *keyword*: `unused class 'DunningConfig'` names something a cleaner could
/// delete by name, while `unreachable code after 'return'` names the statement
/// it appears after. Treating the second as a symbol claim would put `return`
/// into [`SutVerdict::claimed_dead_symbols`], which is not a claim Vulture made.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum FindingKind {
    /// `unused <kind> '<name>'`. The payload is Vulture's own word for the node
    /// type — `import`, `class`, `function`, `method`, `property`, `attribute`,
    /// `variable` (which is also how it reports an unused *argument*).
    ///
    /// Held as a `String` rather than a closed enum on purpose: a Vulture
    /// release that adds a node type must still parse and must still produce a
    /// claim. Refusing to parse a new kind would turn a tool upgrade into a
    /// silently smaller verdict, which is the §6.20 failure this crate exists to
    /// prevent.
    Unused(String),
    /// `unreachable code after '<keyword>'`. The quoted token is the keyword the
    /// dead region follows, not a symbol.
    UnreachableCode,
    /// `unsatisfiable '<keyword>' condition`. Likewise a keyword.
    UnsatisfiableCondition,
}

/// One line of Vulture's stdout, parsed.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct VultureFinding {
    /// The path exactly as Vulture printed it — relative to the working
    /// directory it ran in, or absolute when the target lies outside it
    /// (`utils.format_path` falls back to the absolute path when
    /// `Path.relative_to(Path.cwd())` raises). Not re-rooted here: the adapter
    /// does not know the repository root, and guessing one would silently
    /// mis-key every comparison downstream.
    pub path: PathBuf,
    /// 1-based line, Vulture's `first_lineno`.
    pub line: u32,
    /// What was reported.
    pub kind: FindingKind,
    /// The quoted token: a symbol name for [`FindingKind::Unused`], a Python
    /// keyword otherwise.
    pub name: String,
    /// Vulture's percentage, carried for display. **Nothing branches on this.**
    /// See the module documentation.
    pub confidence: u8,
}

impl VultureFinding {
    /// Whether this finding names a symbol a cleaner could delete by name.
    ///
    /// False for unreachable code and unsatisfiable conditions, whose quoted
    /// token is a keyword. Deliberately not a function of [`Self::confidence`].
    pub fn claims_symbol(&self) -> bool {
        matches!(self.kind, FindingKind::Unused(_))
    }
}

/// Parse a whole Vulture stdout stream.
///
/// # Errors
///
/// Any non-blank line that is not a finding is an error, and the error is
/// returned instead of the findings that parsed before it. §6.20: every analyzer
/// self-failure it catalogues "presents as clean output", and its rule is that
/// *"no data" must be a distinct state from "zero executions"*. A parser that
/// skipped the lines it did not understand would turn a broken stream into a
/// short verdict — the shape a cleaner reads as "safe to proceed".
///
/// Two unsupported invocations get their own message rather than the generic
/// one, because both are silent by default:
///
/// * `--verbose` interleaves an AST dump with the findings on stdout, so
///   findings cannot be told from chatter.
/// * `--make-whitelist` prints the same items in a shape no finding parser
///   matches, so a tolerant parser would report *zero findings* for a run that
///   found plenty.
pub fn parse_findings(stdout: &str) -> Result<Vec<VultureFinding>> {
    let mut findings = Vec::new();
    for (index, line) in stdout.lines().enumerate() {
        let number = index + 1;
        if line.trim().is_empty() {
            continue;
        }
        if let Some(reason) = unsupported_invocation(line) {
            return Err(malformed(number, reason, line));
        }
        match parse_finding(line) {
            Ok(finding) => findings.push(finding),
            Err(reason) => return Err(malformed(number, reason, line)),
        }
    }
    Ok(findings)
}

/// The tool name every error from this module carries.
const TOOL: &str = "vulture";

fn malformed(number: usize, reason: &str, line: &str) -> Error {
    Error::Sut {
        sut: TOOL.to_string(),
        message: format!("stdout line {number}: {reason}: {line:?}"),
    }
}

/// Prefixes `Vulture._log` writes to **stdout** under `--verbose`.
const VERBOSE_PREFIXES: &[&str] = &[
    "Scanning: ",
    "Excluded: ",
    "Included whitelist: ",
    "Excluded whitelist: ",
];

/// Recognize the two modes whose stdout is not a finding stream, so the error
/// says which flag to drop instead of "cannot parse". Both are silent failures
/// otherwise, and the whitelist one is worse than silent: it would parse to zero
/// findings and read as a clean repository.
fn unsupported_invocation(line: &str) -> Option<&'static str> {
    if VERBOSE_PREFIXES.iter().any(|p| line.starts_with(p)) {
        return Some(
            "this is `--verbose` progress output, not a finding. Run vulture without \
             `--verbose`: verbose mode interleaves an AST dump with the findings on stdout \
             and the two cannot be told apart",
        );
    }
    let whitelist_item = line.contains("  # unused ");
    let whitelist_comment = line.starts_with("# ") && line.ends_with(')');
    if whitelist_item || whitelist_comment {
        return Some(
            "this is `--make-whitelist` output, not a finding. Run vulture without \
             `--make-whitelist`: it reports the same items in a shape no finding parser \
             matches, so tolerating it would report zero findings for a run that found some",
        );
    }
    None
}

/// One line, or the reason it is not a finding.
fn parse_finding(line: &str) -> std::result::Result<VultureFinding, &'static str> {
    let (path, number, rest) = split_location(line)
        .ok_or("no `<path>:<line>: ` prefix, which every vulture finding starts with")?;
    let (message, tail) = split_confidence_suffix(rest)
        .ok_or("no trailing `(<n>% confidence)`, which every vulture finding ends with")?;
    let confidence = parse_confidence(tail).ok_or(
        "the parenthesized suffix is not `<n>% confidence` with n in 0..=100, optionally \
         followed by `, <n> line` or `, <n> lines`",
    )?;
    let (kind, name) = parse_message(message).ok_or(
        "the message is none of `unused <kind> '<name>'`, `unreachable code after \
         '<keyword>'` or `unsatisfiable '<keyword>' condition`",
    )?;
    Ok(VultureFinding {
        path: PathBuf::from(path),
        line: number,
        kind,
        name,
        confidence,
    })
}

/// Split `path:line: rest` at the **first** `:` followed by digits and `: `.
///
/// Not the first colon, and not the last: a path may contain a colon (macOS
/// permits it, a Windows drive letter guarantees one) and the message never
/// does. Anchoring on the digit run is what makes both cases fall out.
fn split_location(line: &str) -> Option<(&str, u32, &str)> {
    let mut search = 0;
    while let Some(offset) = line[search..].find(':') {
        let colon = search + offset;
        let digits_start = colon + 1;
        let digits_len = line[digits_start..]
            .bytes()
            .take_while(u8::is_ascii_digit)
            .count();
        let digits_end = digits_start + digits_len;
        if digits_len > 0 && colon > 0 && line[digits_end..].starts_with(": ") {
            let number = line[digits_start..digits_end].parse::<u32>().ok()?;
            return Some((&line[..colon], number, &line[digits_end + 2..]));
        }
        search = colon + 1;
    }
    None
}

/// Split `message (suffix)` at the last ` (`, returning the message and the
/// suffix without its parentheses.
fn split_confidence_suffix(rest: &str) -> Option<(&str, &str)> {
    let inner = rest.strip_suffix(')')?;
    let open = inner.rfind(" (")?;
    Some((&inner[..open], &inner[open + 2..]))
}

/// `60% confidence` or `60% confidence, 3 lines`.
///
/// The size suffix is validated and discarded: nothing in E2 grades on it, and a
/// field nothing reads is a field that drifts.
///
/// Rejecting a value above 100 is the one place a number is compared against a
/// constant here, and it is a *format* check, not a threshold: vulture cannot
/// print `120% confidence`, so a line that does is not vulture output. Nothing
/// downstream may ask how large the number is — see the module documentation and
/// `confidence_never_changes_what_is_claimed`.
fn parse_confidence(tail: &str) -> Option<u8> {
    let (digits, remainder) = tail.split_once("% confidence")?;
    let confidence = parse_decimal::<u8>(digits)?;
    if confidence > 100 {
        return None;
    }
    if !remainder.is_empty() {
        let (count, unit) = remainder.strip_prefix(", ")?.split_once(' ')?;
        parse_decimal::<u32>(count)?;
        if unit != "line" && unit != "lines" {
            return None;
        }
    }
    Some(confidence)
}

/// Digits only — no sign, no whitespace, no empty string. `str::parse` accepts a
/// leading `+`, which vulture never prints, and accepting it here would mean the
/// parser is describing a format the tool does not have.
fn parse_decimal<T: std::str::FromStr>(text: &str) -> Option<T> {
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

/// The three message templates in `vulture/core.py`.
fn parse_message(message: &str) -> Option<(FindingKind, String)> {
    if let Some(rest) = message.strip_prefix("unused ") {
        let (kind, quoted) = rest.split_once(' ')?;
        if kind.is_empty() || !kind.bytes().all(|b| b.is_ascii_lowercase() || b == b'_') {
            return None;
        }
        return Some((FindingKind::Unused(kind.to_string()), unquote(quoted)?));
    }
    if let Some(quoted) = message.strip_prefix("unreachable code after ") {
        return Some((FindingKind::UnreachableCode, unquote(quoted)?));
    }
    if let Some(rest) = message.strip_prefix("unsatisfiable ") {
        let quoted = rest.strip_suffix(" condition")?;
        return Some((FindingKind::UnsatisfiableCondition, unquote(quoted)?));
    }
    None
}

/// The `'name'` vulture wraps every reported token in. A Python identifier and a
/// Python keyword both contain no quote, so an inner quote means the line is not
/// what it appears to be.
fn unquote(text: &str) -> Option<String> {
    let inner = text.strip_prefix('\'')?.strip_suffix('\'')?;
    if inner.is_empty() || inner.contains('\'') {
        return None;
    }
    Some(inner.to_string())
}

/// Vulture's stdout, as the verdict the suite grades. See [`MAPPING_DECISION`].
///
/// # Errors
///
/// Propagates [`parse_findings`]. An empty stream is *not* an error: Vulture
/// exits 0 and prints nothing when it finds nothing. Per
/// [`CAPABILITY_ENVELOPE`], that silence is not evidence of anything, and
/// establishing that the run actually happened is the caller's job — §9.2's
/// health bit and positive control, which no amount of stdout parsing can
/// substitute for.
pub fn verdict_from_stdout(stdout: &str) -> Result<SutVerdict> {
    Ok(verdict_from_findings(&parse_findings(stdout)?))
}

/// The same mapping, applied to already-parsed findings, for a caller that also
/// wants to print them.
pub fn verdict_from_findings(findings: &[VultureFinding]) -> SutVerdict {
    // Sorted and deduplicated: vulture reports the same name once per file it
    // occurs in, and a claim list whose order and length depend on how many
    // copies of a module a repository happens to have cannot be diffed between
    // runs. Collapsing duplicates cannot hide a false removal either — grading
    // asks whether a live name was claimed at all, not how often.
    let symbols: BTreeSet<&str> = findings
        .iter()
        .filter(|finding| finding.claims_symbol())
        .map(|finding| finding.name.as_str())
        .collect();
    SutVerdict {
        claimed_dead_paths: Vec::new(),
        claimed_dead_symbols: symbols.into_iter().map(str::to_string).collect(),
    }
}

/// Every file some finding lands in, sorted and deduplicated.
///
/// This is the blast radius [`MAPPING_DECISION`] declines to grade: the files a
/// human acting on this run would have edited. It is deliberately **not**
/// [`SutVerdict::claimed_dead_paths`] — reporting it as such would be claiming
/// on Vulture's behalf that these files are dead, which it never said.
pub fn files_touched(findings: &[VultureFinding]) -> Vec<PathBuf> {
    let files: BTreeSet<&Path> = findings
        .iter()
        .map(|finding| finding.path.as_path())
        .collect();
    files.into_iter().map(Path::to_path_buf).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Captured verbatim: `vulture sample/ledger` against a two-file package,
    /// vulture 2.16, 2026-08-01. Covers every node type vulture has a word for
    /// plus both keyword-shaped messages.
    const CAPTURED_TWO_FILES: &str = "\
sample/ledger/dunning.py:1: unused import 'os' (90% confidence)
sample/ledger/dunning.py:7: unused attribute 'retry_days' (60% confidence)
sample/ledger/dunning.py:8: unused attribute 'unused_attr' (60% confidence)
sample/ledger/dunning.py:10: unused property 'grace' (60% confidence)
sample/ledger/dunning.py:14: unused method 'escalate' (60% confidence)
sample/ledger/dunning.py:18: unused function 'render_badge' (60% confidence)
sample/ledger/dunning.py:22: unused function 'after_return' (60% confidence)
sample/ledger/dunning.py:24: unreachable code after 'return' (100% confidence)
sample/ledger/dunning.py:27: unused function 'whilefalse' (60% confidence)
sample/ledger/dunning.py:28: unsatisfiable 'while' condition (100% confidence)
sample/ledger/jobs.py:1: unused class 'Whatever' (60% confidence)
sample/ledger/jobs.py:5: unused function 'handler' (60% confidence)
sample/ledger/jobs.py:5: unused variable 'context' (100% confidence)
sample/ledger/jobs.py:6: unused variable 'total' (60% confidence)
";

    /// Captured verbatim: `vulture --sort-by-size sample/ledger/dunning.py`.
    /// Same findings, reordered, each carrying the `, N line(s)` suffix.
    const CAPTURED_SORT_BY_SIZE: &str = "\
sample/ledger/dunning.py:1: unused import 'os' (90% confidence, 1 line)
sample/ledger/dunning.py:7: unused attribute 'retry_days' (60% confidence, 1 line)
sample/ledger/dunning.py:8: unused attribute 'unused_attr' (60% confidence, 1 line)
sample/ledger/dunning.py:24: unreachable code after 'return' (100% confidence, 1 line)
sample/ledger/dunning.py:14: unused method 'escalate' (60% confidence, 2 lines)
sample/ledger/dunning.py:18: unused function 'render_badge' (60% confidence, 2 lines)
sample/ledger/dunning.py:28: unsatisfiable 'while' condition (100% confidence, 2 lines)
sample/ledger/dunning.py:10: unused property 'grace' (60% confidence, 3 lines)
sample/ledger/dunning.py:22: unused function 'after_return' (60% confidence, 3 lines)
sample/ledger/dunning.py:27: unused function 'whilefalse' (60% confidence, 3 lines)
";

    /// Captured verbatim: the same file, analyzed from a working directory it is
    /// not under, so `utils.format_path` falls back to the absolute path.
    const CAPTURED_ABSOLUTE_PATH: &str = "\
/Users/neo/.blackhole/Judged/2026-08-01/sample/ledger/jobs.py:1: unused class 'Whatever' (60% confidence)
/Users/neo/.blackhole/Judged/2026-08-01/sample/ledger/jobs.py:5: unused function 'handler' (60% confidence)
/Users/neo/.blackhole/Judged/2026-08-01/sample/ledger/jobs.py:5: unused variable 'context' (100% confidence)
/Users/neo/.blackhole/Judged/2026-08-01/sample/ledger/jobs.py:6: unused variable 'total' (60% confidence)
";

    /// Captured verbatim: two copies of the same module, so the same four names
    /// are reported twice from different files.
    const CAPTURED_DUPLICATE_NAMES: &str = "\
sample/ledger/jobs.py:1: unused class 'Whatever' (60% confidence)
sample/ledger/jobs.py:5: unused function 'handler' (60% confidence)
sample/ledger/jobs.py:5: unused variable 'context' (100% confidence)
sample/ledger/jobs.py:6: unused variable 'total' (60% confidence)
sample/ledger/jobs_copy.py:1: unused class 'Whatever' (60% confidence)
sample/ledger/jobs_copy.py:5: unused function 'handler' (60% confidence)
sample/ledger/jobs_copy.py:5: unused variable 'context' (100% confidence)
sample/ledger/jobs_copy.py:6: unused variable 'total' (60% confidence)
";

    /// Captured verbatim: the head of `vulture --verbose sample`. Vulture's
    /// `_log` sends this to **stdout**, mixed in with the findings.
    const CAPTURED_VERBOSE: &str = "\
Scanning: /Users/neo/.blackhole/Judged/2026-08-01/sample/ledger/dunning.py
1 alias(name='os') import os
1 Import(names=[alias(name='os')]) import os
define import \"os\"
";

    /// Captured verbatim: `vulture --make-whitelist sample/ledger/jobs.py`. The
    /// same four items as `CAPTURED_DUPLICATE_NAMES`' first half, in a shape
    /// that contains no `(N% confidence)` suffix at all.
    const CAPTURED_MAKE_WHITELIST: &str = "\
Whatever  # unused class (sample/ledger/jobs.py:1)
handler  # unused function (sample/ledger/jobs.py:5)
context  # unused variable (sample/ledger/jobs.py:5)
total  # unused variable (sample/ledger/jobs.py:6)
";

    fn unused(path: &str, line: u32, kind: &str, name: &str, confidence: u8) -> VultureFinding {
        VultureFinding {
            path: PathBuf::from(path),
            line,
            kind: FindingKind::Unused(kind.to_string()),
            name: name.to_string(),
            confidence,
        }
    }

    fn dunning_and_jobs() -> Vec<VultureFinding> {
        vec![
            unused("sample/ledger/dunning.py", 1, "import", "os", 90),
            unused("sample/ledger/dunning.py", 7, "attribute", "retry_days", 60),
            unused(
                "sample/ledger/dunning.py",
                8,
                "attribute",
                "unused_attr",
                60,
            ),
            unused("sample/ledger/dunning.py", 10, "property", "grace", 60),
            unused("sample/ledger/dunning.py", 14, "method", "escalate", 60),
            unused(
                "sample/ledger/dunning.py",
                18,
                "function",
                "render_badge",
                60,
            ),
            unused(
                "sample/ledger/dunning.py",
                22,
                "function",
                "after_return",
                60,
            ),
            VultureFinding {
                path: PathBuf::from("sample/ledger/dunning.py"),
                line: 24,
                kind: FindingKind::UnreachableCode,
                name: "return".to_string(),
                confidence: 100,
            },
            unused("sample/ledger/dunning.py", 27, "function", "whilefalse", 60),
            VultureFinding {
                path: PathBuf::from("sample/ledger/dunning.py"),
                line: 28,
                kind: FindingKind::UnsatisfiableCondition,
                name: "while".to_string(),
                confidence: 100,
            },
            unused("sample/ledger/jobs.py", 1, "class", "Whatever", 60),
            unused("sample/ledger/jobs.py", 5, "function", "handler", 60),
            unused("sample/ledger/jobs.py", 5, "variable", "context", 100),
            unused("sample/ledger/jobs.py", 6, "variable", "total", 60),
        ]
    }

    #[test]
    fn parses_every_line_of_a_captured_two_file_run() {
        assert_eq!(
            parse_findings(CAPTURED_TWO_FILES).unwrap(),
            dunning_and_jobs()
        );
    }

    #[test]
    fn parses_the_sort_by_size_suffix() {
        // Same ten findings as the dunning half above, in vulture's size order.
        // The `, N line(s)` suffix is accepted and not retained: nothing in E2
        // grades on it.
        let expected = vec![
            unused("sample/ledger/dunning.py", 1, "import", "os", 90),
            unused("sample/ledger/dunning.py", 7, "attribute", "retry_days", 60),
            unused(
                "sample/ledger/dunning.py",
                8,
                "attribute",
                "unused_attr",
                60,
            ),
            VultureFinding {
                path: PathBuf::from("sample/ledger/dunning.py"),
                line: 24,
                kind: FindingKind::UnreachableCode,
                name: "return".to_string(),
                confidence: 100,
            },
            unused("sample/ledger/dunning.py", 14, "method", "escalate", 60),
            unused(
                "sample/ledger/dunning.py",
                18,
                "function",
                "render_badge",
                60,
            ),
            VultureFinding {
                path: PathBuf::from("sample/ledger/dunning.py"),
                line: 28,
                kind: FindingKind::UnsatisfiableCondition,
                name: "while".to_string(),
                confidence: 100,
            },
            unused("sample/ledger/dunning.py", 10, "property", "grace", 60),
            unused(
                "sample/ledger/dunning.py",
                22,
                "function",
                "after_return",
                60,
            ),
            unused("sample/ledger/dunning.py", 27, "function", "whilefalse", 60),
        ];
        assert_eq!(parse_findings(CAPTURED_SORT_BY_SIZE).unwrap(), expected);
    }

    #[test]
    fn parses_an_absolute_path() {
        let root = "/Users/neo/.blackhole/Judged/2026-08-01/sample/ledger/jobs.py";
        assert_eq!(
            parse_findings(CAPTURED_ABSOLUTE_PATH).unwrap(),
            vec![
                unused(root, 1, "class", "Whatever", 60),
                unused(root, 5, "function", "handler", 60),
                unused(root, 5, "variable", "context", 100),
                unused(root, 6, "variable", "total", 60),
            ]
        );
    }

    #[test]
    fn splits_at_the_line_number_not_at_the_first_colon() {
        // Constructed by hand, not captured: this shape needs a path that itself
        // contains a colon, which macOS allows and a Windows drive letter
        // guarantees. It pins the rule that the split is the first `:<digits>: `
        // and not the first `:`.
        let line = "src/2:3/app.py:12: unused function 'render' (60% confidence)\n";
        assert_eq!(
            parse_findings(line).unwrap(),
            vec![unused("src/2:3/app.py", 12, "function", "render", 60)]
        );
    }

    #[test]
    fn blank_lines_are_not_findings() {
        let padded = format!("\n\n{CAPTURED_TWO_FILES}\n   \n\n");
        assert_eq!(parse_findings(&padded).unwrap(), dunning_and_jobs());
    }

    #[test]
    fn empty_stdout_is_a_clean_run_not_an_error() {
        // Captured: `vulture sample/clean.py` prints nothing and exits 0.
        assert_eq!(parse_findings("").unwrap(), Vec::new());
        assert_eq!(verdict_from_stdout("").unwrap(), SutVerdict::default());
    }

    #[test]
    fn verbose_output_is_rejected_and_names_the_flag() {
        let error = parse_findings(CAPTURED_VERBOSE).unwrap_err().to_string();
        assert!(error.contains("--verbose"), "unhelpful error: {error}");
        assert!(error.contains("Scanning:"), "error drops the line: {error}");
        assert!(
            verdict_from_stdout(CAPTURED_VERBOSE).is_err(),
            "verbose output must never reach a verdict"
        );
    }

    #[test]
    fn make_whitelist_output_is_rejected_and_names_the_flag() {
        // The dangerous one: every line here is a real finding wearing a shape
        // no finding parser matches, so a tolerant parser reports zero findings
        // for a run that found four.
        let error = parse_findings(CAPTURED_MAKE_WHITELIST)
            .unwrap_err()
            .to_string();
        assert!(error.contains("--make-whitelist"), "unhelpful: {error}");
        assert!(
            verdict_from_stdout(CAPTURED_MAKE_WHITELIST).is_err(),
            "whitelist output must never reach a verdict"
        );
    }

    #[test]
    fn malformed_lines_are_errors_never_an_empty_verdict() {
        let malformed = [
            // Truncated mid-write, e.g. a closed pipe.
            "sample/ledger/jobs.py:1: unused cl",
            // No confidence suffix.
            "sample/ledger/jobs.py:1: unused class 'Whatever'",
            // No line number.
            "sample/ledger/jobs.py: unused class 'Whatever' (60% confidence)",
            // Line number is not a number.
            "sample/ledger/jobs.py:one: unused class 'Whatever' (60% confidence)",
            // Confidence is not a number.
            "sample/ledger/jobs.py:1: unused class 'Whatever' (sixty% confidence)",
            // Unquoted name.
            "sample/ledger/jobs.py:1: unused class Whatever (60% confidence)",
            // A message shape vulture does not have.
            "sample/ledger/jobs.py:1: probably fine 'Whatever' (60% confidence)",
            // Suffix that is not the confidence suffix.
            "sample/ledger/jobs.py:1: unused class 'Whatever' (very likely)",
            // Size suffix in a shape vulture never prints.
            "sample/ledger/jobs.py:1: unused class 'Whatever' (60% confidence, huge)",
            // Empty path.
            ":1: unused class 'Whatever' (60% confidence)",
            // Empty name.
            "sample/ledger/jobs.py:1: unused class '' (60% confidence)",
        ];
        for line in malformed {
            let parsed = parse_findings(line);
            assert!(
                parsed.is_err(),
                "silently accepted malformed line: {line:?}"
            );
            let error = parsed.unwrap_err().to_string();
            assert!(
                error.contains(line),
                "error must quote the offending line, got: {error}"
            );
            assert!(
                verdict_from_stdout(line).is_err(),
                "malformed input produced a verdict: {line:?}"
            );
        }
    }

    #[test]
    fn confidence_outside_zero_to_one_hundred_is_malformed() {
        for bad in ["101", "255", "1000"] {
            let line =
                format!("sample/ledger/jobs.py:1: unused class 'Whatever' ({bad}% confidence)");
            assert!(
                parse_findings(&line).is_err(),
                "accepted impossible confidence {bad}"
            );
        }
    }

    #[test]
    fn a_malformed_line_discards_the_findings_before_it() {
        // The §6.20 case: a stream that dies halfway must not read as a shorter
        // clean run. Fourteen good lines then one bad one is an error, not
        // fourteen findings.
        let truncated = format!("{CAPTURED_TWO_FILES}sample/ledger/jobs.py:7: unused vari");
        assert!(parse_findings(&truncated).is_err());
        assert!(verdict_from_stdout(&truncated).is_err());
    }

    #[test]
    fn keyword_findings_name_a_keyword_and_claim_no_symbol() {
        let findings = parse_findings(CAPTURED_TWO_FILES).unwrap();
        let keyword: Vec<&VultureFinding> =
            findings.iter().filter(|f| !f.claims_symbol()).collect();
        assert_eq!(keyword.len(), 2, "expected the two keyword-shaped messages");
        assert_eq!(keyword[0].kind, FindingKind::UnreachableCode);
        assert_eq!(keyword[0].name, "return");
        assert_eq!(keyword[1].kind, FindingKind::UnsatisfiableCondition);
        assert_eq!(keyword[1].name, "while");

        let verdict = verdict_from_findings(&findings);
        for keyword in ["return", "while"] {
            assert!(
                !verdict.claimed_dead_symbols.iter().any(|s| s == keyword),
                "claimed the keyword {keyword:?} as a dead symbol"
            );
        }
    }

    #[test]
    fn the_verdict_claims_symbols_and_never_files() {
        // MAPPING_DECISION, made executable.
        let verdict = verdict_from_stdout(CAPTURED_TWO_FILES).unwrap();
        assert_eq!(
            verdict.claimed_dead_paths,
            Vec::<PathBuf>::new(),
            "a name-level finding must never become a file claim"
        );
        assert_eq!(
            verdict.claimed_dead_symbols,
            vec![
                "Whatever",
                "after_return",
                "context",
                "escalate",
                "grace",
                "handler",
                "os",
                "render_badge",
                "retry_days",
                "total",
                "unused_attr",
                "whilefalse",
            ],
            "one sorted, deduplicated claim per unused name"
        );
    }

    #[test]
    fn a_name_reported_in_two_files_is_claimed_once() {
        let verdict = verdict_from_stdout(CAPTURED_DUPLICATE_NAMES).unwrap();
        assert_eq!(
            verdict.claimed_dead_symbols,
            vec!["Whatever", "context", "handler", "total"]
        );
    }

    #[test]
    fn confidence_never_changes_what_is_claimed() {
        // The refusal in the module docs, as a test. Rewriting every confidence
        // to the bottom of the scale and then to the top must not move a single
        // claim; if a threshold ever appears, this fails.
        let floor = CAPTURED_TWO_FILES
            .replace("(90% confidence)", "(0% confidence)")
            .replace("(100% confidence)", "(0% confidence)")
            .replace("(60% confidence)", "(0% confidence)");
        let ceiling = CAPTURED_TWO_FILES
            .replace("(90% confidence)", "(100% confidence)")
            .replace("(60% confidence)", "(100% confidence)");
        let baseline = verdict_from_stdout(CAPTURED_TWO_FILES).unwrap();
        assert_eq!(verdict_from_stdout(&floor).unwrap(), baseline);
        assert_eq!(verdict_from_stdout(&ceiling).unwrap(), baseline);
    }

    #[test]
    fn files_touched_reports_the_blast_radius_it_does_not_claim() {
        let findings = parse_findings(CAPTURED_TWO_FILES).unwrap();
        assert_eq!(
            files_touched(&findings),
            vec![
                PathBuf::from("sample/ledger/dunning.py"),
                PathBuf::from("sample/ledger/jobs.py"),
            ]
        );
        assert!(
            verdict_from_findings(&findings)
                .claimed_dead_paths
                .is_empty(),
            "files_touched must not leak into the graded verdict"
        );
    }

    #[test]
    fn the_envelope_and_the_mapping_decision_are_reportable() {
        // Both exist to be printed. §9.2 requires the envelope; MAPPING_DECISION
        // exists so a reader of the report knows which grading they have without
        // reading this file.
        assert!(CAPABILITY_ENVELOPE.contains("silence is not evidence"));
        assert!(CAPABILITY_ENVELOPE.contains("cross-module"));
        assert!(MAPPING_DECISION.contains("claimed_dead_paths is always empty"));
        assert!(MAPPING_DECISION.contains("LOWER BOUND"));
    }

    #[test]
    fn the_envelope_comes_in_the_shape_the_sut_trait_asks_for() {
        // `Sut::cannot_emit` wants a list of prose classes, not one block of
        // text, so wiring a vulture SUT is `.with_cannot_emit(cannot_emit())`
        // rather than a reformatting job at the call site.
        let classes = cannot_emit();
        assert!(
            classes.len() >= 3,
            "an envelope of {} classes",
            classes.len()
        );
        let joined = classes.join("\n");
        assert!(joined.contains("never names a file"));
        assert!(joined.contains("global"));
        assert!(joined.contains("not evidence"));
    }

    #[test]
    fn the_envelope_declares_silence_and_never_excuses_a_false_positive() {
        // The distinction `NaiveSut::cannot_emit` draws and this must not blur:
        // an envelope lists what the tool structurally cannot SAY. vulture's
        // §4.1 false-positive modes — Django model fields, Pydantic fields,
        // @app.template_global, globals(), reflection — are it saying something
        // WRONG, loudly, and they are the whole thing E2 exists to count.
        // Listing them as declared blind spots would excuse the measurement.
        let joined = cannot_emit().join("\n");
        for excuse in [
            "Django",
            "Pydantic",
            "template_global",
            "globals()",
            "reflection",
            "decorator",
        ] {
            assert!(
                !joined.contains(excuse),
                "the envelope excuses a false-positive mode: {excuse:?}"
            );
        }
        assert!(
            CAPABILITY_ENVELOPE.contains("wrong answers, not silence"),
            "the prose envelope must say why the FP modes are absent from it"
        );
    }
}
