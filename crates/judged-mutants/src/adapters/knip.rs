//! Knip (JS/TS) — SARIF stdout to [`SutVerdict`], and the health bit its SARIF
//! refuses to carry.
//!
//! Knip is the strongest tool in the §4.1 survey — ~178 framework plugins, a
//! module graph rather than a name-set difference, and a real file-level claim —
//! so it is the fairest test the E2 catalogue can offer. It is also the one tool
//! whose own documentation is an admission against interest (§7.5, verbatim):
//! *"Running knip --fix before your configuration is fully settled is dangerous.
//! If your configuration is missing entry points or has unresolved hints, Knip
//! might think perfectly valid, actively used code is unused. Auto-fixing in
//! this state can lead to deleting code that your application relies on."*
//!
//! # Which invocation this parses
//!
//! [`RECOMMENDED_ARGS`] — `--reporter sarif --no-progress`. SARIF is preferred
//! over `--reporter json` because [`judged_core::sarif`] already models it and
//! §9.2 makes it the integration contract. Both were captured before this
//! parser was written; the tests below quote the bytes.
//!
//! A run looks like this — captured from knip 6.31.0 against the m14 fixture on
//! 2026-08-01, reflowed here only for the margin:
//!
//! ```text
//! {"$schema":"…sarif-schema-2.1.0.json","version":"2.1.0","runs":[{
//!   "tool":{"driver":{"name":"knip","version":"6.31.0",…}},
//!   "results":[
//!     {"ruleId":"knip/files","ruleIndex":0,"level":"error",
//!      "message":{"text":"Unused file: dist/widget.7f3a91c.js"},
//!      "locations":[{"physicalLocation":{"artifactLocation":
//!        {"uri":"dist/widget.7f3a91c.js"}}}]},
//!     …]}]}
//! ```
//!
//! # The thing that matters most, and it is not the parser
//!
//! §6.20 catalogues analyzer self-failure — *"every failure below presents as
//! clean output"* — and names knip's `vite.config.ts` case: a plugin that fails
//! to load contributes **no roots**, and every file that plugin would have
//! rooted is then reported unused. Three facts were measured here, not recalled,
//! and together they are worse than §6.20 records:
//!
//! 1. **A knip whose entry pattern resolves to nothing reports every file in the
//!    project as unused, and says so in clean, schema-valid SARIF with exit code
//!    1 — the same exit code as a healthy run that found issues.** Captured as
//!    `CAPTURED_GHOST_ENTRY` below: one typo in `knip.json#entry` turns a
//!    two-file project into two "unused file" claims.
//! 2. **The `sarif` reporter discards knip's configuration hints entirely.** The
//!    hint that would have revealed case 1 — `[src/entrypoint.ts]  knip.json
//!    Refine entry pattern (no matches)` — is printed only by the default
//!    `symbols` reporter, and only to stderr. Under `--reporter sarif` it
//!    appears on neither stream. `--treat-config-hints-as-errors` does not
//!    restore it: it only moves the exit code to 1, which the run was already
//!    using. So the machine-readable reporter §9.2 asks adapters to use is
//!    precisely the one that deletes the degradation signal.
//! 3. **Knip's SARIF contains no `invocations` and no `artifacts` array**, so it
//!    asserts neither `executionSuccessful` nor a single
//!    `roles: ["analysisTarget"]`. It satisfies none of the three gates in
//!    [`judged_core::sarif::assess_run_health`]. §6.20's rule — *"'no data' must
//!    be a distinct state from 'zero executions'"* — cannot be met from knip's
//!    stdout at all.
//!
//! Consequently this module draws a hard line. [`parse`] translates claims and
//! **never asserts health**; [`degradation`] reads what knip says about itself
//! on stderr; [`invocation`] computes §9.2's health bit from the exit code and
//! stderr, which is the only place that information exists; and [`sarif_run`]
//! assembles the three into the [`judged_core::sarif::Run`] the orchestrator is
//! entitled to. A caller that runs knip and reads only [`parse`] has measured
//! nothing about whether the run was configured, and [`CAPABILITY_ENVELOPE`]
//! says so in the text a report prints.
//!
//! # Why there is a JSON parser in here
//!
//! `judged-mutants` depends on `judged-core` and `tempfile` and nothing else,
//! and this adapter may not change that. The alternative — scanning the SARIF
//! for substrings — is the failure mode the round was warned about, so the
//! bottom of this file contains a small, strict, total JSON reader instead. It
//! rejects trailing bytes, unterminated strings, bad escapes and unbounded
//! nesting, because every one of those is a way for a truncated stream to become
//! a short, plausible, wrong verdict.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use judged_core::sarif::{
    Artifact, Invocation, Level, Location, Notification, Run, SarifResult, Tool,
};
use judged_core::{Error, Result};

use crate::mutant::Ecosystem;
use crate::sut::{SutVerdict, SymbolClaim};

/// The tool name every error and every notification from this module carries.
const TOOL: &str = "knip";

/// The invocation this parser was written against, and the one a `CommandSut`
/// should use.
///
/// `--no-progress` is not cosmetic. Knip writes a live progress line when it
/// believes it is attached to a terminal; suppressing it explicitly means the
/// captured stream is the same whether or not the harness allocated a pty.
///
/// Note what is absent: `--fix`, and therefore also `--allow-remove-files`.
/// §9.2's first rule is that adapters are read-only, and knip's two-gate design
/// (§7.5: *"the two-gate pattern should be copied exactly"*) exists because
/// removing a file is categorically riskier than editing one. This adapter
/// grades what knip would remove; it never lets knip remove it.
pub const RECOMMENDED_ARGS: &[&str] = &["--reporter", "sarif", "--no-progress"];

/// Exit codes that mean knip ran to completion, for
/// [`crate::sut::CommandSut::with_success_exit_codes`].
///
/// Measured, not assumed: `0` on a clean repository, `1` when issues were found,
/// `2` on `Unable to find package.json`, on an unparseable `knip.json`, and on
/// an invalid `--include` value. `1` therefore has to be healthy or knip could
/// never be graded at all — which is exactly the hazard [`parse`] compensates
/// for, since knip also exits **1** when it refuses to run and prints its help
/// text instead.
pub const SUCCESS_EXIT_CODES: &[i32] = &[0, 1];

/// The ecosystems knip can load a repository from, for
/// [`crate::sut::CommandSut::with_reads`].
///
/// Knip builds a JavaScript/TypeScript module graph rooted in `package.json`
/// and `tsconfig.json`. Without a `package.json` it does not analyze badly — it
/// does not start: measured 2026-08-01 against knip 6.31.0 on the materialized
/// catalogue, `ERROR: Unable to find package.json` and exit 2 on m08, m13 and
/// m18, against a SARIF log and exit 1 on m02, m10 and m14.
///
/// `Ecosystem::TypeScript` is this enum's name for the whole JS/TS ecosystem,
/// so a fixture whose JavaScript half is untyped (m10) declares it too.
///
/// Note what is **not** here, because it was here and it was wrong. An earlier
/// build claimed knip reads `Polyglot`, on the reasoning that every polyglot
/// fixture contains a JS or TS half. Three of the five do not — m08 is Python
/// with a CI workflow, m13 is PHP, m18 is Python with Kotlin — and the claim
/// was what made `--sut knip` abort on the first of them. `Polyglot` is a
/// property of a liveness mechanism, not a toolchain; the two polyglot fixtures
/// knip really does read declare `TypeScript` in
/// [`crate::mutant::Mutant::languages`] and are graded through that.
pub const READS: &[Ecosystem] = &[Ecosystem::TypeScript];

/// What Knip can and cannot say, in the form §9.2 requires of every adapter.
///
/// An envelope declares what a tool structurally **cannot say**. It is not a
/// list of the tool's known mistakes: §4.1's measured false-positive modes —
/// template-string `import()`, CJS member access, HTML `<script src>`,
/// auto-mocks, a framework with no plugin — are knip saying something *wrong*,
/// loudly, and they are the entire thing E2 exists to count. Listing them as
/// declared blind spots would excuse the measurement. They are named in the last
/// paragraph as context for reading a report, and deliberately not as classes.
///
/// The one item that is genuinely structural and is easy to mistake for a
/// mistake is the third: a knip run whose configuration did not resolve emits no
/// signal at all under the SARIF reporter. That is not knip being wrong about a
/// file, it is knip's chosen output channel being unable to carry the fact that
/// it was misconfigured — a property of the interface, not of the analysis.
pub const CAPABILITY_ENVELOPE: &str = "\
knip resolves a module graph from a configured entry set and reports what that \
graph did not reach; its silence is not evidence.

Structurally cannot emit:

(1) Any finding about a non-JS/TS artifact. Knip scans the files its `project` \
patterns match in a JavaScript or TypeScript workspace and requires a \
package.json to run at all — without one it exits 2 having analyzed nothing. A \
dead Python module, Go file, Rust crate, YAML task, CI step or Dockerfile stage \
is invisible to it, and so is every artifact in a repository that has no \
package.json.

(2) Any finding about a file reachable from a configured entry point through a \
resolvable specifier. Reachability is the whole method, so an edge knip can \
follow suppresses the finding, and its silence about a file is not evidence the \
file is live.

(3) Any statement about its own health, when run with `--reporter sarif`. Its \
SARIF log carries no `invocations` array, so `executionSuccessful` is never \
asserted, and no `artifacts` array, so no file is ever marked \
roles: [\"analysisTarget\"] -- it satisfies none of the three gates in \
judged_core::sarif::assess_run_health. Worse, the `sarif` reporter DISCARDS the \
configuration hints that the default reporter prints to stderr, including \
\"Refine entry pattern (no matches)\", which is the one line that reveals an \
entry set that resolved to nothing. Measured: a knip whose entry pattern has a \
typo reports every file in the project as unused, in clean schema-valid SARIF, \
at exit code 1, with an empty stderr. A run whose configuration did not resolve \
is not evidence about the repository, and knip's SARIF cannot tell you that you \
are looking at one. The exit code and stderr can, which is why `invocation` \
takes both and `parse` promises nothing.

(4) A claim about a symbol below the export boundary. Knip reports unused \
exports, exported types, exported enum members and exported namespace members. \
A module-private function with no callers is not an issue type it has.

Not listed above, on purpose: the false-positive modes §4.1 measures -- \
template-string import(), CJS member access (`m.fn()` untraced while \
destructuring is traced), HTML <script src>, unknown CLI arguments, auto-mocks \
and auto-imports, cross-workspace relative paths, conditional dependencies in \
executed config files, and any framework it has no plugin for. Those are wrong \
answers, not silence. They are what E2 grades, and an envelope that declared \
them would be excusing the number this suite exists to produce.";

/// [`CAPABILITY_ENVELOPE`] in the shape [`crate::sut::Sut::cannot_emit`] wants:
/// one prose class per entry, so a report can list them and a `Sut` impl can
/// return them without restating anything.
pub fn cannot_emit() -> Vec<String> {
    [
        "any finding about a non-JS/TS artifact: knip requires a package.json and analyzes \
         only the files its project patterns match, so a dead Python module, Go file, YAML \
         task, CI step or Dockerfile stage is invisible to it",
        "any finding about a file reachable from a configured entry point through a \
         resolvable specifier: reachability is the whole method, so its silence about a file \
         is not evidence the file is live",
        "any statement about its own health under --reporter sarif: the log carries no \
         invocations and no artifacts, and the sarif reporter discards the configuration \
         hints the default reporter prints to stderr, so a run whose entry set resolved to \
         nothing is byte-indistinguishable from a healthy one and reports every file unused",
        "any claim about a symbol below the export boundary: it reports unused exports, \
         exported types, exported enum members and exported namespace members, and has no \
         issue type for a module-private declaration",
    ]
    .iter()
    .map(|class| (*class).to_string())
    .collect()
}

/// Which half of [`SutVerdict`] each knip issue type is allowed to fill, and why.
///
/// Knip is the first adapter here that can fill `claimed_dead_paths`, and the
/// difference from Vulture is not a matter of degree. Vulture reports unused
/// *names* and never names a file, so "this file is dead" had to be inferred and
/// was therefore refused. Knip computes `unused files = project files − (entry +
/// resolved)` and prints the path, and `--allow-remove-files` exists for no
/// other purpose than to act on that list. A `knip/files` result is a deletion
/// claim knip made itself; recording it as one invents nothing.
pub const MAPPING_DECISION: &str = "\
Knip issue types map onto SutVerdict by ruleId, as follows.

CLAIMED_DEAD_PATHS -- knip/files, and nothing else. `Unused file: <path>` is a \
genuine, first-party deletion claim: knip computes unused files as project \
files minus everything reachable from the entry set, prints the repo-relative \
path, and ships `--allow-remove-files` (a second gate behind `--fix`) precisely \
so a user can act on it. The claim is taken from the result's \
locations[].physicalLocation.artifactLocation.uri, not from the message text, \
because the uri is the field SARIF defines for it.

CLAIMED_DEAD_SYMBOLS -- knip/exports, knip/nsExports, knip/types, knip/nsTypes, \
knip/enumMembers, knip/namespaceMembers. Each names one exported symbol that \
nothing imports. Two of these six are inexact and the inexactness is stated \
rather than closed:

  (a) The qualifier is dropped. Knip writes `Unused export: unusedApi (api)`, \
  `Unused exported enum member: Blue (Colour)` and `Unused exported namespace \
  member: Square (Shapes)`, where the parenthetical is the namespace or enum \
  the symbol lives in. SutVerdict has no field for a qualified symbol, so the \
  claim recorded is the bare name and the qualifier is kept on the finding for \
  the report. Keeping the parenthetical instead would mean no ground-truth \
  symbol could ever match, which would under-report every enum and namespace \
  finding to zero.

  (b) knip/nsExports and knip/nsTypes are classified from knip's own documented \
  issue-type list; knip 6.31.0 folds both into knip/exports and knip/types with \
  the qualifier suffix, so no result carrying those rule ids was captured and \
  their message shape is inferred from the uniform `<label>: <payload>` shape \
  that all nine captured message types share.

CARRIED BUT NOT CLAIMED -- knip/dependencies, knip/devDependencies, \
knip/optionalPeerDependencies, knip/catalog, knip/catalogReferences. These name \
a package.json manifest entry, which is neither a repo path nor a source \
symbol. Putting `esbuild` into claimed_dead_symbols would invent a claim about \
a source symbol knip never made, and putting package.json into \
claimed_dead_paths would invent a far worse one. They are exposed by \
`manifest_claims` so a report can print them, and they are ungraded: E2 ground \
truth has no dependency-name field, so this grading is silent about the entire \
false-removal class where the live artifact is a declared dependency.

NEVER A CLAIM -- knip/unlisted and knip/binaries are the INVERSE of a deadness \
claim (\"you use this and did not declare it\"). knip/duplicates says two names \
export the same value without saying which is redundant. knip/cycles is emitted \
at level warning and is not a deadness claim at all.

DEGRADATION, NOT A CLAIM -- knip/unresolved. `Unresolved import: <specifier>` \
is an edge knip could not follow, which is a hole in the very graph the \
knip/files claims were computed from. It is surfaced as a warning notification \
by `sarif_run`, and it never removes or suppresses a claim: surfacing knip's \
own degradation is the adapter's job, filtering knip's answers is not.

UNKNOWN ruleId -- an error, and the whole verdict is discarded. Knip's issue \
types are a closed, documented list and each one needs a mapping decision \
before its findings can be counted; a new one is not a symbol claim by default. \
This is the opposite of the vulture adapter's choice to accept an unrecognised \
node kind, and the difference is that a vulture line's shape already fixed it \
as a symbol claim, whereas a knip ruleId is the only thing that says whether a \
finding is about a file, a symbol, a manifest entry or nothing at all. \
Guessing would silently mis-key the verdict; dropping it would silently shrink \
it.

HEALTH IS NOT IN HERE. `parse` translates claims and asserts nothing about \
whether the run was configured. See CAPABILITY_ENVELOPE item (3): knip's SARIF \
carries no invocations and no artifacts, and the sarif reporter drops the \
configuration hints that would reveal an entry set that resolved to nothing. \
Use `invocation` and `sarif_run`, which read the exit code and stderr, and \
treat a verdict obtained without them as a verdict of unknown provenance.";

/// What one knip issue type means for [`SutVerdict`]. See [`MAPPING_DECISION`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Claim {
    /// `knip/files` — a repo-relative path knip says can be removed.
    Path,
    /// An exported symbol knip says nothing imports.
    Symbol,
    /// A `package.json` manifest entry. Carried, never graded.
    Manifest,
    /// `knip/unresolved` — an edge knip could not follow. Degradation.
    ReachabilityGap,
    /// `knip/unlisted`, `knip/binaries` — the inverse of a deadness claim.
    Inverse,
    /// `knip/duplicates`, `knip/cycles` — not a claim that anything is dead.
    None,
}

/// One `result` from knip's SARIF log, classified.
///
/// Not ordered: [`judged_core::sarif::Level`] is deliberately not `Ord`, and a
/// finding type that could be sorted by severity is one step from a report that
/// ranks by it. §9.2 records the SARIF spec's own warning that rank values from
/// different tools *"are in general not commensurable"*.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnipFinding {
    /// `knip/files`, `knip/exports`, … exactly as knip spelled it.
    pub rule_id: String,
    /// What this issue type means here, decided once at parse time.
    pub claim: Claim,
    /// SARIF severity as knip reported it. `warning` for `knip/cycles`, `error`
    /// for every other type observed. **Nothing branches on this**: knip's
    /// levels come from a static per-rule `defaultConfiguration`, not from any
    /// measurement, and a threshold on them would mean the same thing here as a
    /// threshold on vulture's confidence.
    pub level: Level,
    /// `message.text`, whole and unedited, for the report.
    pub message: String,
    /// The message after its `<label>: ` prefix — a path, a symbol, a package
    /// name or a specifier, depending on [`Self::claim`].
    pub payload: String,
    /// The `(…)` suffix knip appends to a symbol payload: the namespace or enum
    /// the symbol belongs to. Dropped from the claim, kept for the report.
    pub qualifier: Option<String>,
    /// `locations[0].physicalLocation.artifactLocation.uri`, exactly as knip
    /// printed it: repo-relative, forward-slashed. Not re-rooted here — the
    /// adapter does not know the repository root, and `CommandSut` does.
    pub uri: Option<String>,
    /// `region.startLine`, 1-based, for display only. §9.2: fingerprints are
    /// content-derived and never line-based.
    pub start_line: Option<u32>,
}

impl KnipFinding {
    /// The path this finding claims is dead, or `None` if it claims no path.
    pub fn path(&self) -> Option<&Path> {
        match self.claim {
            Claim::Path => self.uri.as_deref().map(Path::new),
            _ => None,
        }
    }

    /// The symbol this finding claims is dead, without its qualifier, or `None`.
    pub fn symbol(&self) -> Option<&str> {
        match self.claim {
            Claim::Symbol => Some(self.payload.as_str()),
            _ => None,
        }
    }

    /// The same symbol, with the module knip located it in.
    ///
    /// [`Self::uri`] is `locations[0]` — for a `knip/exports` result, the module
    /// that exports the symbol, which is the file Gate 2a must exclude from the
    /// corpus before deciding whether anything references it.
    ///
    /// `None` there yields [`SymbolClaim::unattributed`] rather than an error.
    /// SARIF's `locations` is optional and knip is free to omit it; a finding
    /// with no location is knip declining to say where the symbol lives, which
    /// is precisely the case that constructor exists for. Refusing the run would
    /// discard a claim knip did make over a field it never promised.
    pub fn symbol_claim(&self) -> Option<SymbolClaim> {
        let name = self.symbol()?;
        Some(match self.uri.as_deref() {
            Some(uri) => SymbolClaim::declared_in(name, uri),
            None => SymbolClaim::unattributed(name),
        })
    }
}

/// A whole knip SARIF log, parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnipReport {
    /// `tool.driver.name`. Checked against `knip` so that pointing the adapter
    /// at another SARIF-emitting analyzer is an error rather than a silent
    /// mis-mapping of somebody else's rule ids.
    pub tool_name: String,
    /// `tool.driver.version`, recorded because §9.4 invalidates evidence when it
    /// changes.
    pub tool_version: Option<String>,
    /// Every result, in the order knip emitted them.
    pub findings: Vec<KnipFinding>,
    /// Whether the log carried an `invocations` array. Measured on knip 6.31.0:
    /// it does not. Kept as a field rather than assumed, so that a knip release
    /// which starts asserting `executionSuccessful` is visible immediately.
    pub declared_invocations: bool,
    /// Whether the log carried an `artifacts` array — §9.2's positive control.
    /// Measured on knip 6.31.0: it does not.
    pub declared_artifacts: bool,
}

/// Knip's SARIF stdout, as the verdict the suite grades. See
/// [`MAPPING_DECISION`], and read [`CAPABILITY_ENVELOPE`] item (3) before
/// treating the result as evidence.
///
/// This is the [`crate::sut::VerdictParser`] entry point.
///
/// # Errors
///
/// Anything that is not one knip SARIF log. In particular the two shapes knip
/// itself produces when it declines to run, both captured:
///
/// * Its **help text**, printed to stdout with exit code **1** — the same code a
///   healthy run uses — when it is handed a positional argument. `CommandSut`
///   appends the repository path to every command, so this is the default
///   outcome of wiring knip up naively, and a tolerant parser would read it as a
///   clean repository.
/// * A one-line pointer at its own help page — `Run ... for help`, or
///   `Configuration file load error? ...` — printed with exit code 2 when there
///   is no `package.json`, or when `knip.json` cannot be parsed. Both are
///   quoted byte-for-byte in this module's tests.
///
/// An empty `results` array is **not** an error: knip exits 0 and emits an empty
/// log when it finds nothing. Per [`CAPABILITY_ENVELOPE`] that silence is not
/// evidence, and establishing that the run was configured is the caller's job.
pub fn parse(stdout: &str) -> Result<SutVerdict> {
    Ok(verdict_from_findings(&parse_report(stdout)?.findings))
}

/// The same mapping, applied to already-parsed findings, for a caller that also
/// wants to print them.
pub fn verdict_from_findings(findings: &[KnipFinding]) -> SutVerdict {
    // Sorted and deduplicated. Knip reports each unused file once, but an
    // exported symbol of the same name can be reported from several files, and
    // a claim list whose length depends on how many modules happen to export
    // `default` cannot be diffed between runs. Collapsing duplicates cannot
    // hide a false removal: grading asks whether a live artifact was claimed at
    // all, not how often.
    //
    // Each symbol claim now carries the module knip located it in — the `uri`
    // this adapter has always parsed and never passed on. Gate 2a excludes that
    // module before asking whether anything imports the symbol; without it every
    // export is found in its own module and rescued. See
    // [`crate::sut::SymbolClaim`].
    //
    // `SymbolClaim::dedup_by_name` collapses the duplicates and drops the module
    // when they disagree: `default` exported from two modules has no single file
    // to exclude, and excluding one would leave the gate finding the symbol in
    // the other.
    let paths: BTreeSet<&Path> = findings.iter().filter_map(KnipFinding::path).collect();
    let symbols = SymbolClaim::dedup_by_name(findings.iter().filter_map(KnipFinding::symbol_claim));
    SutVerdict {
        claimed_dead_paths: paths.into_iter().map(Path::to_path_buf).collect(),
        claimed_dead_symbols: symbols,
    }
}

/// The manifest entries knip called unused, sorted and deduplicated.
///
/// This is the blast radius [`MAPPING_DECISION`] declines to grade: the
/// `package.json` lines a human acting on this run would have deleted. It is
/// deliberately **not** part of [`SutVerdict`] — E2 ground truth has no
/// dependency-name field, so a claim here can be neither confirmed nor
/// falsified, and reporting it as a symbol claim would put a package name where
/// a source symbol belongs.
pub fn manifest_claims(findings: &[KnipFinding]) -> Vec<String> {
    let entries: BTreeSet<&str> = findings
        .iter()
        .filter(|finding| finding.claim == Claim::Manifest)
        .map(|finding| finding.payload.as_str())
        .collect();
    entries.into_iter().map(str::to_string).collect()
}

/// Parse a whole knip SARIF log.
///
/// # Errors
///
/// The stream is not JSON, is not a SARIF log, is not knip's, or contains a
/// result this adapter has no mapping for. Nothing is skipped: §6.20's rule is
/// that *"no data" must be a distinct state from "zero executions"*, and a
/// parser that ignored the results it did not understand would turn a knip
/// upgrade into a silently shorter verdict — the shape a cleaner reads as
/// "safe to proceed".
pub fn parse_report(stdout: &str) -> Result<KnipReport> {
    let value = json::parse(stdout).map_err(|reason| malformed(&reason))?;
    let root = value
        .object()
        .ok_or_else(|| malformed("the log is not a JSON object"))?;

    let runs = json::get(root, "runs")
        .and_then(json::Value::array)
        .ok_or_else(|| malformed("no `runs` array; this is not a SARIF 2.1.0 log"))?;
    let [run] = runs else {
        return Err(malformed(&format!(
            "expected exactly one run, found {}; knip emits one run per invocation and \
             merging several would attribute one run's findings to another's health",
            runs.len()
        )));
    };
    let run = run
        .object()
        .ok_or_else(|| malformed("`runs[0]` is not an object"))?;

    let driver = json::get(run, "tool")
        .and_then(json::Value::object)
        .and_then(|tool| json::get(tool, "driver"))
        .and_then(json::Value::object)
        .ok_or_else(|| malformed("no `tool.driver` object"))?;
    let tool_name = json::get(driver, "name")
        .and_then(json::Value::string)
        .ok_or_else(|| malformed("no `tool.driver.name`"))?
        .to_string();
    if tool_name != TOOL {
        return Err(malformed(&format!(
            "this log was produced by `{tool_name}`, not by knip; its rule ids mean \
             something else and mapping them here would mis-key the verdict"
        )));
    }
    let tool_version = json::get(driver, "version")
        .and_then(json::Value::string)
        .map(str::to_string);

    let results = match json::get(run, "results") {
        None => &[][..],
        Some(value) => value
            .array()
            .ok_or_else(|| malformed("`results` is present but is not an array"))?,
    };
    let findings = results
        .iter()
        .enumerate()
        .map(|(index, result)| parse_result(index, result))
        .collect::<Result<Vec<_>>>()?;

    Ok(KnipReport {
        tool_name,
        tool_version,
        findings,
        declared_invocations: json::get(run, "invocations").is_some(),
        declared_artifacts: json::get(run, "artifacts").is_some(),
    })
}

fn malformed(reason: &str) -> Error {
    Error::Sut {
        sut: TOOL.to_string(),
        message: format!("stdout is not one knip SARIF log: {reason}"),
    }
}

fn bad_result(index: usize, reason: &str) -> Error {
    Error::Sut {
        sut: TOOL.to_string(),
        message: format!("results[{index}]: {reason}"),
    }
}

fn parse_result(index: usize, result: &json::Value) -> Result<KnipFinding> {
    let result = result
        .object()
        .ok_or_else(|| bad_result(index, "not an object"))?;

    let rule_id = json::get(result, "ruleId")
        .and_then(json::Value::string)
        .ok_or_else(|| bad_result(index, "no `ruleId`"))?
        .to_string();
    let claim = classify(&rule_id).ok_or_else(|| bad_result(index, &unmapped_rule(&rule_id)))?;

    let level_text = json::get(result, "level")
        .and_then(json::Value::string)
        .ok_or_else(|| bad_result(index, "no `level`"))?;
    let level = parse_level(level_text).ok_or_else(|| {
        bad_result(
            index,
            &format!("`level` is {level_text:?}, not one of none/note/warning/error"),
        )
    })?;

    let message = json::get(result, "message")
        .and_then(json::Value::object)
        .and_then(|message| json::get(message, "text"))
        .and_then(json::Value::string)
        .ok_or_else(|| bad_result(index, "no `message.text`"))?
        .to_string();
    let payload = payload_of(&message)
        .ok_or_else(|| {
            bad_result(
                index,
                &format!(
                    "message {message:?} has no `<label>: <payload>` shape, which every knip \
                     message observed does"
                ),
            )
        })?
        .to_string();

    let (payload, qualifier) = match claim {
        Claim::Symbol => split_qualifier(&payload),
        _ => (payload, None),
    };

    let (uri, start_line) = location_of(result, index)?;

    if claim == Claim::Path && uri.is_none() {
        return Err(bad_result(
            index,
            "a `knip/files` result with no artifact uri: the path claim has no path, and \
             recovering it from the message text would be reading a field SARIF does not \
             define for it",
        ));
    }
    if claim == Claim::Symbol && payload.is_empty() {
        return Err(bad_result(index, "a symbol claim with an empty name"));
    }

    Ok(KnipFinding {
        rule_id,
        claim,
        level,
        message,
        payload,
        qualifier,
        uri,
        start_line,
    })
}

/// Knip's documented issue types, mapped once. See [`MAPPING_DECISION`].
///
/// The list is knip's own, taken from `knip --help`:
/// *"(1) Issue types: files, dependencies, unlisted, unresolved, exports,
/// nsExports, types, nsTypes, enumMembers, namespaceMembers, duplicates,
/// catalog, catalogReferences, cycles"*, plus the three rule ids the reporters
/// emit that the summary line folds together — `devDependencies`,
/// `optionalPeerDependencies` and `binaries`.
fn classify(rule_id: &str) -> Option<Claim> {
    let issue_type = rule_id.strip_prefix("knip/")?;
    Some(match issue_type {
        "files" => Claim::Path,
        "exports" | "nsExports" | "types" | "nsTypes" | "enumMembers" | "namespaceMembers" => {
            Claim::Symbol
        }
        "dependencies"
        | "devDependencies"
        | "optionalPeerDependencies"
        | "catalog"
        | "catalogReferences" => Claim::Manifest,
        "unresolved" => Claim::ReachabilityGap,
        "unlisted" | "binaries" => Claim::Inverse,
        "duplicates" | "cycles" => Claim::None,
        _ => return None,
    })
}

fn unmapped_rule(rule_id: &str) -> String {
    format!(
        "ruleId {rule_id:?} has no mapping in this adapter. Knip's issue types are a closed, \
         documented list and each needs a decision before its findings can be counted: a new \
         one may be a file claim, a symbol claim, a manifest entry or no claim at all. \
         Refusing the whole log is deliberate — dropping the result would shrink the verdict \
         silently, and guessing would mis-key it. Add the mapping, and say so in \
         MAPPING_DECISION"
    )
}

fn parse_level(text: &str) -> Option<Level> {
    Some(match text {
        "none" => Level::None,
        "note" => Level::Note,
        "warning" => Level::Warning,
        "error" => Level::Error,
        _ => return None,
    })
}

/// `Unused file: dist/widget.js` → `dist/widget.js`.
///
/// Every knip message captured has the shape `<label>: <payload>`, and the label
/// is prose containing no colon. Splitting on the **first** `": "` is therefore
/// right even when the payload contains one — which it does for
/// `knip/cycles`, whose payload is a comma-separated path list.
fn payload_of(message: &str) -> Option<&str> {
    let (_, payload) = message.split_once(": ")?;
    if payload.is_empty() {
        return None;
    }
    Some(payload)
}

/// `unusedApi (api)` → `("unusedApi", Some("api"))`.
///
/// The parenthetical is the namespace or enum the symbol belongs to. A
/// JavaScript identifier contains neither a space nor a parenthesis, so the
/// split is unambiguous; a payload that does not end in `)` is returned whole,
/// which is the `Unused export: default` case.
fn split_qualifier(payload: &str) -> (String, Option<String>) {
    let Some(inner) = payload.strip_suffix(')') else {
        return (payload.to_string(), None);
    };
    let Some((name, qualifier)) = inner.rsplit_once(" (") else {
        return (payload.to_string(), None);
    };
    if name.is_empty() || qualifier.is_empty() {
        return (payload.to_string(), None);
    }
    (name.to_string(), Some(qualifier.to_string()))
}

/// `locations[0].physicalLocation.{artifactLocation.uri, region.startLine}`.
///
/// SARIF allows several locations; knip emits at most one, and taking the first
/// is stated here rather than left to be discovered. An empty `locations` array
/// is legal SARIF for a repo-level finding and yields `None`.
fn location_of(
    result: &[(String, json::Value)],
    index: usize,
) -> Result<(Option<String>, Option<u32>)> {
    let Some(locations) = json::get(result, "locations") else {
        return Ok((None, None));
    };
    let locations = locations
        .array()
        .ok_or_else(|| bad_result(index, "`locations` is present but is not an array"))?;
    let Some(first) = locations.first() else {
        return Ok((None, None));
    };
    let physical = first
        .object()
        .and_then(|location| json::get(location, "physicalLocation"))
        .and_then(json::Value::object)
        .ok_or_else(|| bad_result(index, "`locations[0]` has no `physicalLocation` object"))?;

    let uri = json::get(physical, "artifactLocation")
        .and_then(json::Value::object)
        .and_then(|artifact| json::get(artifact, "uri"))
        .and_then(json::Value::string)
        .map(str::to_string);
    if uri.as_deref() == Some("") {
        return Err(bad_result(index, "an empty `artifactLocation.uri`"));
    }

    let start_line = match json::get(physical, "region")
        .and_then(json::Value::object)
        .and_then(|region| json::get(region, "startLine"))
    {
        None => None,
        Some(value) => Some(
            value
                .integer()
                .and_then(|n| u32::try_from(n).ok())
                .ok_or_else(|| {
                    bad_result(
                        index,
                        "`region.startLine` is not a non-negative whole number",
                    )
                })?,
        ),
    };

    Ok((uri, start_line))
}

/// What knip said about itself on **stderr**, as SARIF notifications.
///
/// This is the only channel on which knip reports its own degradation, and under
/// `--reporter sarif` it is frequently empty even when the run was misconfigured
/// — see [`CAPABILITY_ENVELOPE`] item (3). Every line captured from knip 6.31.0
/// has one of three shapes:
///
/// * `ERROR: <what failed>`, optionally followed by `Reason: <detail>`. §6.20's
///   named case is exactly this: `ERROR: Error loading vite.config.ts (Cannot
///   find module 'vite')`. Recorded at [`Level::Error`].
/// * `Unexpected argument '<arg>'. This command does not take positional
///   arguments`, which knip prints before dumping its help text to stdout and
///   exiting **1**. Recorded at [`Level::Error`].
/// * A `Configuration hints (<n>)` block, printed only by the default `symbols`
///   reporter. Recorded at [`Level::Warning`], one notification per hint,
///   verbatim — `Refine entry pattern (no matches)` is the line that means the
///   entry set resolved to nothing, and no run whose entry set resolved to
///   nothing is evidence about a repository.
///
/// Any other non-blank line is also recorded, at [`Level::Warning`], rather than
/// dropped. An adapter that only forwarded the messages it recognized would make
/// a new failure message invisible, which is the failure this whole function
/// exists to prevent.
pub fn degradation(stderr: &str) -> Vec<Notification> {
    stderr
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| Notification {
            // `Reason: …` continues the `ERROR:` above it and a hint line
            // continues the `Configuration hints (n)` header; both are carried
            // at warning level, which is enough to degrade the run without
            // claiming the continuation line is itself a failure.
            level: if line.starts_with("ERROR: ") || line.starts_with("Unexpected argument ") {
                Level::Error
            } else {
                Level::Warning
            },
            message: format!("knip stderr: {line}"),
        })
        .collect()
}

/// §9.2's health bit for one knip run: *"adapters compute a health bit; the
/// orchestrator never reads a raw exit code."*
///
/// `execution_successful` is true only for the exit codes knip uses for a
/// completed analysis ([`SUCCESS_EXIT_CODES`]). `None` — a process killed by a
/// signal, which has no exit code at all — is false, and so is `2`, which is
/// what knip returns when it never got as far as analyzing anything.
///
/// The exit code is deliberately **not** enough on its own, and this function
/// does not pretend otherwise: knip exits 1 both when it found issues and when
/// it refused a positional argument. The notifications from [`degradation`] and
/// [`parse_report`]'s refusal to read help text are the other two gates.
///
/// What this function will not do is synthesize a healthy invocation out of
/// nothing. §6.20: absence is not success.
pub fn invocation(exit_code: Option<i32>, stderr: &str, findings: &[KnipFinding]) -> Invocation {
    let mut notifications = degradation(stderr);

    // An unresolved import is a missing edge in the graph every knip/files claim
    // was computed from, so it degrades the run that produced them. It never
    // suppresses a claim — surfacing knip's degradation is this adapter's job,
    // second-guessing knip's answers is not.
    for finding in findings {
        if finding.claim == Claim::ReachabilityGap {
            notifications.push(Notification {
                level: Level::Warning,
                message: format!(
                    "knip could not resolve an import, so the module graph the unused-file \
                     claims were computed from has a hole in it: {}",
                    finding.message
                ),
            });
        }
    }

    match exit_code {
        Some(code) if SUCCESS_EXIT_CODES.contains(&code) => Invocation {
            execution_successful: true,
            tool_execution_notifications: notifications,
        },
        other => {
            notifications.push(Notification {
                level: Level::Error,
                message: match other {
                    Some(code) => format!(
                        "knip exited with status {code}; it completes with {SUCCESS_EXIT_CODES:?} \
                         and uses 2 for a run that never analyzed anything"
                    ),
                    None => "knip was killed by a signal and never chose an exit code".to_string(),
                },
            });
            Invocation {
                execution_successful: false,
                tool_execution_notifications: notifications,
            }
        }
    }
}

/// A whole knip run in the §9.2 contract shape, ready for
/// [`judged_core::sarif::assess_run_health`].
///
/// Note what is not synthesized. `artifacts` is left **empty**, because knip
/// declares no `analysisTarget` and inventing one would forge the single clause
/// §9.2 calls the most valuable in the whole contract — the positive control
/// that catches a tool which ran to completion over almost nothing.
///
/// The consequence, measured rather than predicted: **a knip run can never be
/// [`judged_core::sarif::RunHealth::Healthy`]**, at any
/// `expected_analysis_targets` including zero, because the coverage gate is not
/// satisfiable by a log that declares no scanned universe. The best available
/// state is `Degraded`, which is the correct reading of a tool that did the work
/// and then declined to say what it had looked at.
pub fn sarif_run(report: &KnipReport, exit_code: Option<i32>, stderr: &str) -> Run {
    Run {
        tool: Tool {
            name: report.tool_name.clone(),
            version: report.tool_version.clone(),
        },
        invocations: vec![invocation(exit_code, stderr, &report.findings)],
        artifacts: Vec::<Artifact>::new(),
        results: report
            .findings
            .iter()
            .map(|finding| SarifResult {
                rule_id: finding.rule_id.clone(),
                level: finding.level,
                message: finding.message.clone(),
                locations: finding
                    .uri
                    .iter()
                    .map(|uri| Location {
                        uri: uri.clone(),
                        start_line: finding.start_line,
                    })
                    .collect(),
                partial_fingerprints: std::collections::BTreeMap::new(),
                baseline_state: None,
                suppressions: Vec::new(),
            })
            .collect(),
        baseline_guid: None,
    }
}

/// Every file some finding lands in, sorted and deduplicated.
///
/// The blast radius, including the files a symbol or manifest finding merely
/// points *into*. Deliberately not [`SutVerdict::claimed_dead_paths`]: knip
/// naming `package.json` as the location of an unused dependency is not knip
/// claiming `package.json` is dead.
pub fn files_touched(findings: &[KnipFinding]) -> Vec<PathBuf> {
    let files: BTreeSet<&str> = findings
        .iter()
        .filter_map(|finding| finding.uri.as_deref())
        .collect();
    files.into_iter().map(PathBuf::from).collect()
}

/// A small, strict JSON reader.
///
/// `judged-mutants` depends on `judged-core` and `tempfile`, and an adapter may
/// not change that, so knip's SARIF is read here rather than with `serde_json`.
/// Every rejection below exists because the alternative is a truncated stream
/// becoming a short, plausible, wrong verdict: unterminated strings, trailing
/// bytes after the document, malformed escapes and unbounded nesting are all
/// errors, never partial successes.
mod json {
    /// The subset of JSON a SARIF log uses. Objects keep insertion order in a
    /// `Vec` because they are small and because a duplicate key must stay
    /// visible rather than being silently overwritten by a map insert.
    #[derive(Debug, Clone, PartialEq)]
    pub enum Value {
        Null,
        Bool(bool),
        Number(f64),
        String(String),
        Array(Vec<Value>),
        Object(Vec<(String, Value)>),
    }

    impl Value {
        pub fn object(&self) -> Option<&[(String, Value)]> {
            match self {
                Value::Object(fields) => Some(fields),
                _ => None,
            }
        }

        pub fn array(&self) -> Option<&[Value]> {
            match self {
                Value::Array(items) => Some(items),
                _ => None,
            }
        }

        pub fn string(&self) -> Option<&str> {
            match self {
                Value::String(text) => Some(text),
                _ => None,
            }
        }

        /// The value as a whole number, or `None` if it is not one. A SARIF
        /// `startLine` of `12.5` is not a line number and must not round to one.
        pub fn integer(&self) -> Option<i64> {
            match self {
                Value::Number(n) if n.fract() == 0.0 && n.is_finite() => Some(*n as i64),
                _ => None,
            }
        }
    }

    /// First value for `key`, or `None`. Linear because SARIF objects have a
    /// handful of keys and a map would cost more than it saved.
    pub fn get<'a>(fields: &'a [(String, Value)], key: &str) -> Option<&'a Value> {
        fields
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value)
    }

    /// Nesting depth beyond which the input is refused. SARIF from knip nests
    /// six deep; a stream that nests 64 deep is not knip output, and recursing
    /// on it would abort the process with a stack overflow rather than an error.
    const MAX_DEPTH: usize = 64;

    /// Parse exactly one JSON document. Trailing non-whitespace is an error.
    pub fn parse(text: &str) -> Result<Value, String> {
        let bytes = text.as_bytes();
        let mut at = 0;
        let value = parse_value(bytes, &mut at, 0)?;
        skip_whitespace(bytes, &mut at);
        if at != bytes.len() {
            return Err(format!(
                "{} trailing bytes after the JSON document, starting {:?}",
                bytes.len() - at,
                snippet(text, at)
            ));
        }
        Ok(value)
    }

    /// Up to 40 characters from `at`, on a character boundary, for an error.
    fn snippet(text: &str, at: usize) -> String {
        let tail = &text[at..];
        let end = tail
            .char_indices()
            .map(|(index, c)| index + c.len_utf8())
            .take(40)
            .last()
            .unwrap_or(0);
        tail[..end].to_string()
    }

    fn skip_whitespace(bytes: &[u8], at: &mut usize) {
        while matches!(bytes.get(*at), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            *at += 1;
        }
    }

    fn parse_value(bytes: &[u8], at: &mut usize, depth: usize) -> Result<Value, String> {
        if depth > MAX_DEPTH {
            return Err(format!("nested deeper than {MAX_DEPTH} levels"));
        }
        skip_whitespace(bytes, at);
        match bytes.get(*at) {
            None => Err("input ended where a value was expected".to_string()),
            Some(b'{') => parse_object(bytes, at, depth),
            Some(b'[') => parse_array(bytes, at, depth),
            Some(b'"') => Ok(Value::String(parse_string(bytes, at)?)),
            Some(b't') => literal(bytes, at, "true", Value::Bool(true)),
            Some(b'f') => literal(bytes, at, "false", Value::Bool(false)),
            Some(b'n') => literal(bytes, at, "null", Value::Null),
            Some(_) => parse_number(bytes, at),
        }
    }

    fn literal(bytes: &[u8], at: &mut usize, word: &str, value: Value) -> Result<Value, String> {
        if bytes[*at..].starts_with(word.as_bytes()) {
            *at += word.len();
            return Ok(value);
        }
        Err(format!("expected `{word}` at byte {at}"))
    }

    fn parse_object(bytes: &[u8], at: &mut usize, depth: usize) -> Result<Value, String> {
        *at += 1; // `{`
        let mut fields = Vec::new();
        skip_whitespace(bytes, at);
        if bytes.get(*at) == Some(&b'}') {
            *at += 1;
            return Ok(Value::Object(fields));
        }
        loop {
            skip_whitespace(bytes, at);
            if bytes.get(*at) != Some(&b'"') {
                return Err(format!("expected a quoted key at byte {at}"));
            }
            let key = parse_string(bytes, at)?;
            skip_whitespace(bytes, at);
            if bytes.get(*at) != Some(&b':') {
                return Err(format!("expected `:` after key {key:?} at byte {at}"));
            }
            *at += 1;
            let value = parse_value(bytes, at, depth + 1)?;
            fields.push((key, value));
            skip_whitespace(bytes, at);
            match bytes.get(*at) {
                Some(b',') => *at += 1,
                Some(b'}') => {
                    *at += 1;
                    return Ok(Value::Object(fields));
                }
                _ => return Err(format!("expected `,` or `}}` at byte {at}")),
            }
        }
    }

    fn parse_array(bytes: &[u8], at: &mut usize, depth: usize) -> Result<Value, String> {
        *at += 1; // `[`
        let mut items = Vec::new();
        skip_whitespace(bytes, at);
        if bytes.get(*at) == Some(&b']') {
            *at += 1;
            return Ok(Value::Array(items));
        }
        loop {
            items.push(parse_value(bytes, at, depth + 1)?);
            skip_whitespace(bytes, at);
            match bytes.get(*at) {
                Some(b',') => *at += 1,
                Some(b']') => {
                    *at += 1;
                    return Ok(Value::Array(items));
                }
                _ => return Err(format!("expected `,` or `]` at byte {at}")),
            }
        }
    }

    fn parse_string(bytes: &[u8], at: &mut usize) -> Result<String, String> {
        *at += 1; // opening quote
        let mut out = String::new();
        loop {
            let byte = *bytes
                .get(*at)
                .ok_or_else(|| "a string was never closed".to_string())?;
            match byte {
                b'"' => {
                    *at += 1;
                    return Ok(out);
                }
                b'\\' => {
                    *at += 1;
                    push_escape(bytes, at, &mut out)?;
                }
                0x00..=0x1f => {
                    return Err(format!("a raw control byte {byte:#04x} inside a string"))
                }
                _ => {
                    // The input is `&str`, so a multi-byte sequence is valid
                    // UTF-8 by construction; copy it whole.
                    let start = *at;
                    let len = utf8_len(byte);
                    let end = start + len;
                    let slice = bytes
                        .get(start..end)
                        .ok_or_else(|| "a string was never closed".to_string())?;
                    out.push_str(std::str::from_utf8(slice).map_err(|e| e.to_string())?);
                    *at = end;
                }
            }
        }
    }

    fn utf8_len(lead: u8) -> usize {
        match lead {
            0x00..=0x7f => 1,
            0xc0..=0xdf => 2,
            0xe0..=0xef => 3,
            _ => 4,
        }
    }

    fn push_escape(bytes: &[u8], at: &mut usize, out: &mut String) -> Result<(), String> {
        let byte = *bytes
            .get(*at)
            .ok_or_else(|| "a string ended inside an escape".to_string())?;
        *at += 1;
        let c = match byte {
            b'"' => '"',
            b'\\' => '\\',
            b'/' => '/',
            b'b' => '\u{8}',
            b'f' => '\u{c}',
            b'n' => '\n',
            b'r' => '\r',
            b't' => '\t',
            b'u' => return push_unicode_escape(bytes, at, out),
            other => return Err(format!("unknown escape `\\{}`", other as char)),
        };
        out.push(c);
        Ok(())
    }

    fn push_unicode_escape(bytes: &[u8], at: &mut usize, out: &mut String) -> Result<(), String> {
        let first = hex4(bytes, at)?;
        // A lone surrogate is not a character. Paired with its low half it is,
        // and knip prints file paths, which may legitimately contain astral
        // characters written this way.
        if (0xd800..0xdc00).contains(&first) {
            if bytes.get(*at) != Some(&b'\\') || bytes.get(*at + 1) != Some(&b'u') {
                return Err("a high surrogate escape with no low surrogate after it".to_string());
            }
            *at += 2;
            let second = hex4(bytes, at)?;
            if !(0xdc00..0xe000).contains(&second) {
                return Err("a high surrogate escape followed by a non-low surrogate".to_string());
            }
            let combined = 0x1_0000 + ((first - 0xd800) << 10) + (second - 0xdc00);
            out.push(
                char::from_u32(combined)
                    .ok_or_else(|| "an escape that is not a character".to_string())?,
            );
            return Ok(());
        }
        out.push(
            char::from_u32(first).ok_or_else(|| "an escape that is not a character".to_string())?,
        );
        Ok(())
    }

    fn hex4(bytes: &[u8], at: &mut usize) -> Result<u32, String> {
        let digits = bytes
            .get(*at..*at + 4)
            .ok_or_else(|| "a `\\u` escape with fewer than four digits".to_string())?;
        let mut value = 0u32;
        for digit in digits {
            let nibble = match digit {
                b'0'..=b'9' => u32::from(digit - b'0'),
                b'a'..=b'f' => u32::from(digit - b'a') + 10,
                b'A'..=b'F' => u32::from(digit - b'A') + 10,
                _ => return Err("a `\\u` escape with a non-hex digit".to_string()),
            };
            value = value * 16 + nibble;
        }
        *at += 4;
        Ok(value)
    }

    fn parse_number(bytes: &[u8], at: &mut usize) -> Result<Value, String> {
        let start = *at;
        if bytes.get(*at) == Some(&b'-') {
            *at += 1;
        }
        let integer_start = *at;
        let digits = take_digits(bytes, at);
        if digits == 0 {
            return Err(format!("expected a value at byte {start}"));
        }
        // JSON forbids a leading zero. Accepting `01` would mean this reader
        // describes a format knip does not emit, which is the whole reason it is
        // hand-written rather than substring-scanned.
        if digits > 1 && bytes[integer_start] == b'0' {
            return Err(format!(
                "a leading zero in a number at byte {integer_start}"
            ));
        }
        if bytes.get(*at) == Some(&b'.') {
            *at += 1;
            if take_digits(bytes, at) == 0 {
                return Err(format!("a `.` with no digits after it at byte {at}"));
            }
        }
        if matches!(bytes.get(*at), Some(b'e' | b'E')) {
            *at += 1;
            if matches!(bytes.get(*at), Some(b'+' | b'-')) {
                *at += 1;
            }
            if take_digits(bytes, at) == 0 {
                return Err(format!("an exponent with no digits at byte {at}"));
            }
        }
        let text = std::str::from_utf8(&bytes[start..*at]).map_err(|e| e.to_string())?;
        text.parse::<f64>()
            .map(Value::Number)
            .map_err(|e| format!("{text:?} is not a number: {e}"))
    }

    fn take_digits(bytes: &[u8], at: &mut usize) -> usize {
        let start = *at;
        while matches!(bytes.get(*at), Some(b'0'..=b'9')) {
            *at += 1;
        }
        *at - start
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The claimed symbol *names*. A claim also carries the module knip located
    /// it in; the tests that are about that say so.
    fn symbol_names(verdict: &SutVerdict) -> Vec<&str> {
        verdict
            .claimed_dead_symbols
            .iter()
            .map(SymbolClaim::name)
            .collect()
    }
    use judged_core::sarif::{assess_run_health, RunHealth};

    /// Captured verbatim: `npx knip@6 --reporter sarif` in the materialized m14
    /// fixture (§10 E2 class 14, checked-in generated asset), knip 6.31.0,
    /// 2026-08-01, exit code 1.
    ///
    /// `dist/widget.7f3a91c.js` is m14's LIVE artifact — the committed bundle
    /// the CDN serves, named by exactly one `<script src>` attribute in
    /// `public/index.html`. Knip claims it.
    const CAPTURED_M14: &str = r#"{"$schema":"https://docs.oasis-open.org/sarif/sarif/v2.1.0/errata01/os/schemas/sarif-schema-2.1.0.json","version":"2.1.0","runs":[{"tool":{"driver":{"name":"knip","version":"6.31.0","semanticVersion":"6.31.0","informationUri":"https://knip.dev","rules":[{"id":"knip/files","name":"files","shortDescription":{"text":"Unused files"},"helpUri":"https://knip.dev/reference/issue-types","defaultConfiguration":{"level":"error"},"properties":{"problem.severity":"error"}},{"id":"knip/devDependencies","name":"devDependencies","shortDescription":{"text":"Unused devDependencies"},"helpUri":"https://knip.dev/reference/issue-types","defaultConfiguration":{"level":"error"},"properties":{"problem.severity":"error"}},{"id":"knip/binaries","name":"binaries","shortDescription":{"text":"Unlisted binaries"},"helpUri":"https://knip.dev/reference/issue-types","defaultConfiguration":{"level":"error"},"properties":{"problem.severity":"error"}}]}},"results":[{"ruleId":"knip/files","ruleIndex":0,"level":"error","message":{"text":"Unused file: dist/widget.0c9e142.js"},"locations":[{"physicalLocation":{"artifactLocation":{"uri":"dist/widget.0c9e142.js"}}}]},{"ruleId":"knip/files","ruleIndex":0,"level":"error","message":{"text":"Unused file: dist/widget.7f3a91c.js"},"locations":[{"physicalLocation":{"artifactLocation":{"uri":"dist/widget.7f3a91c.js"}}}]},{"ruleId":"knip/files","ruleIndex":0,"level":"error","message":{"text":"Unused file: src/unusedFeatureFlags.ts"},"locations":[{"physicalLocation":{"artifactLocation":{"uri":"src/unusedFeatureFlags.ts"}}}]},{"ruleId":"knip/devDependencies","ruleIndex":1,"level":"error","message":{"text":"Unused devDependency: esbuild"},"locations":[{"physicalLocation":{"artifactLocation":{"uri":"package.json"},"region":{"startLine":10,"startColumn":6,"endColumn":13}}}]},{"ruleId":"knip/binaries","ruleIndex":2,"level":"error","message":{"text":"Unlisted binary: esbuild"},"locations":[{"physicalLocation":{"artifactLocation":{"uri":"package.json"}}}]},{"ruleId":"knip/binaries","ruleIndex":2,"level":"error","message":{"text":"Unlisted binary: tsc"},"locations":[{"physicalLocation":{"artifactLocation":{"uri":"package.json"}}}]}]}]}
"#;

    /// Captured verbatim: the same invocation in the m02 fixture (§10 E2 class
    /// 2, dynamic import), exit code 1.
    ///
    /// `src/transports/websocketTransport.ts` is m02's LIVE TypeScript artifact,
    /// reached only by a template-literal `import()`. §4.1 lists template-string
    /// `import()` first among knip's measured false-positive modes; this is that
    /// prediction, tested.
    const CAPTURED_M02: &str = r#"{"$schema":"https://docs.oasis-open.org/sarif/sarif/v2.1.0/errata01/os/schemas/sarif-schema-2.1.0.json","version":"2.1.0","runs":[{"tool":{"driver":{"name":"knip","version":"6.31.0","semanticVersion":"6.31.0","informationUri":"https://knip.dev","rules":[{"id":"knip/files","name":"files","shortDescription":{"text":"Unused files"},"helpUri":"https://knip.dev/reference/issue-types","defaultConfiguration":{"level":"error"},"properties":{"problem.severity":"error"}},{"id":"knip/binaries","name":"binaries","shortDescription":{"text":"Unlisted binaries"},"helpUri":"https://knip.dev/reference/issue-types","defaultConfiguration":{"level":"error"},"properties":{"problem.severity":"error"}}]}},"results":[{"ruleId":"knip/files","ruleIndex":0,"level":"error","message":{"text":"Unused file: src/transports/websocketTransport.ts"},"locations":[{"physicalLocation":{"artifactLocation":{"uri":"src/transports/websocketTransport.ts"}}}]},{"ruleId":"knip/files","ruleIndex":0,"level":"error","message":{"text":"Unused file: src/unusedAnalytics.ts"},"locations":[{"physicalLocation":{"artifactLocation":{"uri":"src/unusedAnalytics.ts"}}}]},{"ruleId":"knip/binaries","ruleIndex":1,"level":"error","message":{"text":"Unlisted binary: tsc"},"locations":[{"physicalLocation":{"artifactLocation":{"uri":"package.json"}}}]}]}]}
"#;

    /// Captured verbatim: the same invocation in the m10 fixture (§10 E2 class
    /// 10, framework convention: Django AppConfig + Jest `__mocks__`), exit 1.
    ///
    /// Knip claims only the JavaScript decoy. It does not claim
    /// `__mocks__/redis.js`, and it says nothing at all about the Python half —
    /// the second being envelope, not competence.
    const CAPTURED_M10: &str = r#"{"$schema":"https://docs.oasis-open.org/sarif/sarif/v2.1.0/errata01/os/schemas/sarif-schema-2.1.0.json","version":"2.1.0","runs":[{"tool":{"driver":{"name":"knip","version":"6.31.0","semanticVersion":"6.31.0","informationUri":"https://knip.dev","rules":[{"id":"knip/files","name":"files","shortDescription":{"text":"Unused files"},"helpUri":"https://knip.dev/reference/issue-types","defaultConfiguration":{"level":"error"},"properties":{"problem.severity":"error"}},{"id":"knip/devDependencies","name":"devDependencies","shortDescription":{"text":"Unused devDependencies"},"helpUri":"https://knip.dev/reference/issue-types","defaultConfiguration":{"level":"error"},"properties":{"problem.severity":"error"}},{"id":"knip/binaries","name":"binaries","shortDescription":{"text":"Unlisted binaries"},"helpUri":"https://knip.dev/reference/issue-types","defaultConfiguration":{"level":"error"},"properties":{"problem.severity":"error"}}]}},"results":[{"ruleId":"knip/files","ruleIndex":0,"level":"error","message":{"text":"Unused file: src/color_utils.js"},"locations":[{"physicalLocation":{"artifactLocation":{"uri":"src/color_utils.js"}}}]},{"ruleId":"knip/devDependencies","ruleIndex":1,"level":"error","message":{"text":"Unused devDependency: jest"},"locations":[{"physicalLocation":{"artifactLocation":{"uri":"package.json"}}}]},{"ruleId":"knip/binaries","ruleIndex":2,"level":"error","message":{"text":"Unlisted binary: jest"},"locations":[{"physicalLocation":{"artifactLocation":{"uri":"package.json"}}}]}]}]}
"#;

    /// Captured verbatim, and the most important artifact in this file.
    ///
    /// A two-file project whose `knip.json` says `"entry": ["src/entrypoint.ts"]`
    /// while the real entry is `src/main.ts`. One typo. Knip declares **both**
    /// files unused, in clean schema-valid SARIF, at exit code 1, with an empty
    /// stderr. The default `symbols` reporter prints
    /// `[src/entrypoint.ts]  knip.json  Refine entry pattern (no matches)` to
    /// stderr; `--reporter sarif` prints it nowhere, and
    /// `--treat-config-hints-as-errors` does not bring it back.
    const CAPTURED_GHOST_ENTRY: &str = r#"{"$schema":"https://docs.oasis-open.org/sarif/sarif/v2.1.0/errata01/os/schemas/sarif-schema-2.1.0.json","version":"2.1.0","runs":[{"tool":{"driver":{"name":"knip","version":"6.31.0","semanticVersion":"6.31.0","informationUri":"https://knip.dev","rules":[{"id":"knip/files","name":"files","shortDescription":{"text":"Unused files"},"helpUri":"https://knip.dev/reference/issue-types","defaultConfiguration":{"level":"error"},"properties":{"problem.severity":"error"}}]}},"results":[{"ruleId":"knip/files","ruleIndex":0,"level":"error","message":{"text":"Unused file: src/helper.ts"},"locations":[{"physicalLocation":{"artifactLocation":{"uri":"src/helper.ts"}}}]},{"ruleId":"knip/files","ruleIndex":0,"level":"error","message":{"text":"Unused file: src/main.ts"},"locations":[{"physicalLocation":{"artifactLocation":{"uri":"src/main.ts"}}}]}]}]}
"#;

    /// Captured verbatim: a healthy repository with nothing to report. Exit 0,
    /// empty `results`, empty `rules`, empty stderr.
    const CAPTURED_CLEAN: &str = r#"{"$schema":"https://docs.oasis-open.org/sarif/sarif/v2.1.0/errata01/os/schemas/sarif-schema-2.1.0.json","version":"2.1.0","runs":[{"tool":{"driver":{"name":"knip","version":"6.31.0","semanticVersion":"6.31.0","informationUri":"https://knip.dev","rules":[]}},"results":[]}]}
"#;

    /// Captured verbatim: §6.20's named case reproduced. A `vite.config.ts` that
    /// imports a module knip cannot load. Exit 1, stdout below, stderr
    /// [`CAPTURED_VITE_STDERR`].
    const CAPTURED_VITE_STDOUT: &str = r#"{"$schema":"https://docs.oasis-open.org/sarif/sarif/v2.1.0/errata01/os/schemas/sarif-schema-2.1.0.json","version":"2.1.0","runs":[{"tool":{"driver":{"name":"knip","version":"6.31.0","semanticVersion":"6.31.0","informationUri":"https://knip.dev","rules":[{"id":"knip/files","name":"files","shortDescription":{"text":"Unused files"},"helpUri":"https://knip.dev/reference/issue-types","defaultConfiguration":{"level":"error"},"properties":{"problem.severity":"error"}},{"id":"knip/binaries","name":"binaries","shortDescription":{"text":"Unlisted binaries"},"helpUri":"https://knip.dev/reference/issue-types","defaultConfiguration":{"level":"error"},"properties":{"problem.severity":"error"}},{"id":"knip/unresolved","name":"unresolved","shortDescription":{"text":"Unresolved imports"},"helpUri":"https://knip.dev/reference/issue-types","defaultConfiguration":{"level":"error"},"properties":{"problem.severity":"error"}}]}},"results":[{"ruleId":"knip/files","ruleIndex":0,"level":"error","message":{"text":"Unused file: src/orphan.ts"},"locations":[{"physicalLocation":{"artifactLocation":{"uri":"src/orphan.ts"}}}]},{"ruleId":"knip/binaries","ruleIndex":1,"level":"error","message":{"text":"Unlisted binary: vite"},"locations":[{"physicalLocation":{"artifactLocation":{"uri":"package.json"}}}]},{"ruleId":"knip/unresolved","ruleIndex":2,"level":"error","message":{"text":"Unresolved import: ./config/aliases"},"locations":[{"physicalLocation":{"artifactLocation":{"uri":"vite.config.ts"},"region":{"startLine":2,"startColumn":31,"endColumn":47}}}]}]}]}
"#;

    /// Captured verbatim, stderr of the same run. §6.20 quotes the first line.
    const CAPTURED_VITE_STDERR: &str = "\
ERROR: Error loading vite.config.ts (Cannot find module 'vite')
ERROR: Please fix or visit https://knip.dev/reference/known-issues
";

    /// Captured verbatim: `--include exports,types,enumMembers` over a file with
    /// one unused export, one unused type, one unused enum member and an unused
    /// exported class. Exit 1.
    const CAPTURED_SYMBOLS: &str = r#"{"$schema":"https://docs.oasis-open.org/sarif/sarif/v2.1.0/errata01/os/schemas/sarif-schema-2.1.0.json","version":"2.1.0","runs":[{"tool":{"driver":{"name":"knip","version":"6.31.0","semanticVersion":"6.31.0","informationUri":"https://knip.dev","rules":[{"id":"knip/exports","name":"exports","shortDescription":{"text":"Unused exports"},"helpUri":"https://knip.dev/reference/issue-types","defaultConfiguration":{"level":"error"},"properties":{"problem.severity":"error"}},{"id":"knip/types","name":"types","shortDescription":{"text":"Unused exported types"},"helpUri":"https://knip.dev/reference/issue-types","defaultConfiguration":{"level":"error"},"properties":{"problem.severity":"error"}},{"id":"knip/enumMembers","name":"enumMembers","shortDescription":{"text":"Unused exported enum members"},"helpUri":"https://knip.dev/reference/issue-types","defaultConfiguration":{"level":"error"},"properties":{"problem.severity":"error"}}]}},"results":[{"ruleId":"knip/exports","ruleIndex":0,"level":"error","message":{"text":"Unused export: unusedExport"},"locations":[{"physicalLocation":{"artifactLocation":{"uri":"src/lib.ts"},"region":{"startLine":3,"startColumn":17,"endColumn":29}}}]},{"ruleId":"knip/exports","ruleIndex":0,"level":"error","message":{"text":"Unused export: Widget"},"locations":[{"physicalLocation":{"artifactLocation":{"uri":"src/lib.ts"},"region":{"startLine":12,"startColumn":14,"endColumn":20}}}]},{"ruleId":"knip/types","ruleIndex":1,"level":"error","message":{"text":"Unused exported type: UnusedType"},"locations":[{"physicalLocation":{"artifactLocation":{"uri":"src/lib.ts"},"region":{"startLine":5,"startColumn":13,"endColumn":23}}}]},{"ruleId":"knip/enumMembers","ruleIndex":2,"level":"error","message":{"text":"Unused exported enum member: Blue (Colour)"},"locations":[{"physicalLocation":{"artifactLocation":{"uri":"src/lib.ts"},"region":{"startLine":9,"startColumn":3,"endColumn":7}}}]}]}]}
"#;

    /// Captured verbatim: namespace-qualified exports, a bare `default` export,
    /// and two unused namespace members. Exit 1.
    const CAPTURED_NAMESPACES: &str = r#"{"$schema":"https://docs.oasis-open.org/sarif/sarif/v2.1.0/errata01/os/schemas/sarif-schema-2.1.0.json","version":"2.1.0","runs":[{"tool":{"driver":{"name":"knip","version":"6.31.0","semanticVersion":"6.31.0","informationUri":"https://knip.dev","rules":[{"id":"knip/exports","name":"exports","shortDescription":{"text":"Unused exports"},"helpUri":"https://knip.dev/reference/issue-types","defaultConfiguration":{"level":"error"},"properties":{"problem.severity":"error"}},{"id":"knip/types","name":"types","shortDescription":{"text":"Unused exported types"},"helpUri":"https://knip.dev/reference/issue-types","defaultConfiguration":{"level":"error"},"properties":{"problem.severity":"error"}},{"id":"knip/namespaceMembers","name":"namespaceMembers","shortDescription":{"text":"Unused exported namespace members"},"helpUri":"https://knip.dev/reference/issue-types","defaultConfiguration":{"level":"error"},"properties":{"problem.severity":"error"}}]}},"results":[{"ruleId":"knip/exports","ruleIndex":0,"level":"error","message":{"text":"Unused export: unusedApi (api)"},"locations":[{"physicalLocation":{"artifactLocation":{"uri":"src/api.ts"},"region":{"startLine":2,"startColumn":17,"endColumn":26}}}]},{"ruleId":"knip/exports","ruleIndex":0,"level":"error","message":{"text":"Unused export: default"},"locations":[{"physicalLocation":{"artifactLocation":{"uri":"src/dupes.ts"},"region":{"startLine":3,"startColumn":16,"endColumn":23}}}]},{"ruleId":"knip/types","ruleIndex":1,"level":"error","message":{"text":"Unused exported type: ApiType (api)"},"locations":[{"physicalLocation":{"artifactLocation":{"uri":"src/api.ts"},"region":{"startLine":3,"startColumn":13,"endColumn":20}}}]},{"ruleId":"knip/namespaceMembers","ruleIndex":2,"level":"error","message":{"text":"Unused exported namespace member: Square (Shapes)"},"locations":[{"physicalLocation":{"artifactLocation":{"uri":"src/shapes.ts"},"region":{"startLine":3,"startColumn":16,"endColumn":22}}}]},{"ruleId":"knip/namespaceMembers","ruleIndex":2,"level":"error","message":{"text":"Unused exported namespace member: Never (Shapes)"},"locations":[{"physicalLocation":{"artifactLocation":{"uri":"src/shapes.ts"},"region":{"startLine":4,"startColumn":15,"endColumn":20}}}]}]}]}
"#;

    /// Captured verbatim: a repository exercising dependency, unlisted and
    /// cycle findings. `knip/cycles` is the one type knip emits at level
    /// `warning`, and its payload is a comma-separated path list. Exit 1.
    const CAPTURED_MIXED: &str = r#"{"$schema":"https://docs.oasis-open.org/sarif/sarif/v2.1.0/errata01/os/schemas/sarif-schema-2.1.0.json","version":"2.1.0","runs":[{"tool":{"driver":{"name":"knip","version":"6.31.0","semanticVersion":"6.31.0","informationUri":"https://knip.dev","rules":[]}},"results":[{"ruleId":"knip/files","ruleIndex":0,"level":"error","message":{"text":"Unused file: src/dupes.ts"},"locations":[{"physicalLocation":{"artifactLocation":{"uri":"src/dupes.ts"}}}]},{"ruleId":"knip/dependencies","ruleIndex":1,"level":"error","message":{"text":"Unused dependency: left-pad"},"locations":[{"physicalLocation":{"artifactLocation":{"uri":"package.json"}}}]},{"ruleId":"knip/unlisted","ruleIndex":2,"level":"error","message":{"text":"Unlisted dependency: some-unlisted-package"},"locations":[{"physicalLocation":{"artifactLocation":{"uri":"src/index.ts"},"region":{"startLine":3,"startColumn":31,"endColumn":52}}}]},{"ruleId":"knip/exports","ruleIndex":3,"level":"error","message":{"text":"Unused export: unusedNs (ns)"},"locations":[{"physicalLocation":{"artifactLocation":{"uri":"src/ns.ts"},"region":{"startLine":2,"startColumn":17,"endColumn":25}}}]},{"ruleId":"knip/types","ruleIndex":4,"level":"error","message":{"text":"Unused exported type: UnusedNsType (ns)"},"locations":[{"physicalLocation":{"artifactLocation":{"uri":"src/ns.ts"},"region":{"startLine":3,"startColumn":13,"endColumn":25}}}]},{"ruleId":"knip/cycles","ruleIndex":5,"level":"warning","message":{"text":"Circular dependency: src/cycleA.ts, src/cycleB.ts"},"locations":[{"physicalLocation":{"artifactLocation":{"uri":"src/cycleA.ts"}}}]}]}]}
"#;

    /// Captured verbatim: the head of knip's stdout when handed a positional
    /// path argument — which is what `CommandSut` appends to every command.
    /// Exit code **1**, the same code a healthy run uses.
    const CAPTURED_POSITIONAL_HELP: &str = "\n✂️  Find unused dependencies, exports and files in your JavaScript and TypeScript projects\n\nUsage: knip [options]\n\nOptions:\n  -h, --help                   Print this help text\n  -V, --version                Print version\n";

    /// Captured verbatim, stderr of the same run.
    const CAPTURED_POSITIONAL_STDERR: &str = "Unexpected argument '/Users/neo/.blackhole/Judged/2026-08-01/knip/repos/m14'. This command does not take positional arguments\n";

    /// Captured verbatim: stdout and stderr of a run in a directory with no
    /// `package.json`. Exit code 2.
    const CAPTURED_NO_MANIFEST_STDOUT: &str =
        "\nRun `knip --help` or visit https://knip.dev for help\n";
    const CAPTURED_NO_MANIFEST_STDERR: &str = "ERROR: Unable to find package.json\n";

    /// Captured verbatim: stdout and stderr with an unparseable `knip.json`.
    /// Exit code 2. Note the trailing space on the `Reason:` line.
    const CAPTURED_BAD_CONFIG_STDOUT: &str =
        "Configuration file load error? Visit https://knip.dev/reference/known-issues\n";
    const CAPTURED_BAD_CONFIG_STDERR: &str = "\
ERROR: Error loading /Users/neo/.blackhole/Judged/2026-08-01/knip/degrade/badcfg/knip.json
Reason: Error parsing /Users/neo/.blackhole/Judged/2026-08-01/knip/degrade/badcfg/knip.json
";

    /// Captured verbatim: the configuration hints the **default** reporter
    /// prints to stderr. Under `--reporter sarif` this block appears on neither
    /// stream, which is [`CAPABILITY_ENVELOPE`] item (3).
    const CAPTURED_CONFIG_HINTS_STDERR: &str = "\
Configuration hints (3)
src/nothing-here/**      knip.json  Remove from ignore
left-pad                 knip.json  Remove from ignoreDependencies
src/does-not-exist.ts    knip.json  Refine entry pattern (no matches)
";

    fn paths(verdict: &SutVerdict) -> Vec<String> {
        verdict
            .claimed_dead_paths
            .iter()
            .map(|path| path.display().to_string())
            .collect()
    }

    #[test]
    fn parses_the_captured_m14_run_field_by_field() {
        let report = parse_report(CAPTURED_M14).unwrap();
        assert_eq!(report.tool_name, "knip");
        assert_eq!(report.tool_version.as_deref(), Some("6.31.0"));
        assert_eq!(report.findings.len(), 6);

        let bundle = &report.findings[1];
        assert_eq!(bundle.rule_id, "knip/files");
        assert_eq!(bundle.claim, Claim::Path);
        assert_eq!(bundle.level, Level::Error);
        assert_eq!(bundle.message, "Unused file: dist/widget.7f3a91c.js");
        assert_eq!(bundle.payload, "dist/widget.7f3a91c.js");
        assert_eq!(bundle.qualifier, None);
        assert_eq!(bundle.uri.as_deref(), Some("dist/widget.7f3a91c.js"));
        assert_eq!(bundle.start_line, None);

        let dependency = &report.findings[3];
        assert_eq!(dependency.rule_id, "knip/devDependencies");
        assert_eq!(dependency.claim, Claim::Manifest);
        assert_eq!(dependency.payload, "esbuild");
        assert_eq!(dependency.uri.as_deref(), Some("package.json"));
        assert_eq!(dependency.start_line, Some(10));
    }

    #[test]
    fn knip_calls_the_live_cdn_bundle_of_m14_dead() {
        // §10 E2 class 14. `dist/widget.7f3a91c.js` is the committed bundle the
        // CDN serves; the only thing naming it is a <script src> attribute.
        // This is a FALSE REMOVAL and the assertion exists to record it, not to
        // be relaxed if a later knip stops making it — if that happens, this
        // test failing is the finding.
        let verdict = parse(CAPTURED_M14).unwrap();
        assert_eq!(
            paths(&verdict),
            vec![
                "dist/widget.0c9e142.js",
                "dist/widget.7f3a91c.js",
                "src/unusedFeatureFlags.ts",
            ],
            "knip claims the live bundle alongside both decoys"
        );
        assert!(
            verdict.claimed_dead_symbols.is_empty(),
            "no symbol-level issue type was enabled in this run"
        );
    }

    #[test]
    fn knip_calls_the_template_literal_import_target_of_m02_dead() {
        // §4.1's first listed knip false-positive mode — template-string
        // import() — measured against §10 E2 class 2.
        let verdict = parse(CAPTURED_M02).unwrap();
        assert_eq!(
            paths(&verdict),
            vec![
                "src/transports/websocketTransport.ts",
                "src/unusedAnalytics.ts",
            ]
        );
    }

    #[test]
    fn knip_survives_m10_by_claiming_only_the_decoy() {
        // The Jest `__mocks__` convention holds: `__mocks__/redis.js` is not
        // claimed. Two qualifications, both measured rather than assumed, so
        // that this pass is not read as more than it is.
        //
        // First, knip roots the DIRECTORY, not the mock. Dropping an obviously
        // dead `__mocks__/definitelyDead.js` into the same fixture produced no
        // finding either, so the rule is "everything under __mocks__ is an
        // entry point" — zero false removals here, and equally no ability to
        // find a genuinely dead mock. §10 E2 class 14's own notes name that
        // trade: "a tool that roots all of dist/ is safe and scores zero decoy
        // recall". This is that, in knip's favour.
        //
        // Second, its silence about the Django half is envelope, not
        // competence — see cannot_emit item 1. Note that knip called `jest`
        // itself an unused devDependency in the same run.
        let verdict = parse(CAPTURED_M10).unwrap();
        assert_eq!(paths(&verdict), vec!["src/color_utils.js"]);
        for live in ["__mocks__/redis.js", "reporting/apps.py"] {
            assert!(
                !paths(&verdict).iter().any(|claimed| claimed == live),
                "knip claimed {live}"
            );
        }
    }

    #[test]
    fn a_misconfigured_entry_set_claims_every_file_and_says_nothing_about_it() {
        // §6.20 and §7.5, made executable. One typo in knip.json#entry and knip
        // reports the entire project unused -- including the real entry point --
        // in clean SARIF at exit 1 with an EMPTY stderr.
        let verdict = parse(CAPTURED_GHOST_ENTRY).unwrap();
        assert_eq!(paths(&verdict), vec!["src/helper.ts", "src/main.ts"]);

        let report = parse_report(CAPTURED_GHOST_ENTRY).unwrap();
        assert!(
            degradation("").is_empty(),
            "there was nothing on stderr to find"
        );
        assert!(
            !report.declared_invocations && !report.declared_artifacts,
            "and nothing in the log either: no invocations, no analysisTarget"
        );

        // So the run is indistinguishable from a healthy one on stdout, and the
        // exit code says nothing either -- it is the same 1 a good run uses.
        let healthy = parse_report(CAPTURED_M14).unwrap();
        assert_eq!(
            invocation(Some(1), "", &report.findings),
            invocation(Some(1), "", &healthy.findings),
            "if these ever differ, knip has gained a way to report this and the \
             envelope's item (3) must be revisited"
        );
    }

    #[test]
    fn parse_never_asserts_health_so_assess_run_health_fails_a_knip_run() {
        // Knip's SARIF carries no `invocations`, so §9.2's health bit is never
        // asserted and judged-core correctly refuses the raw log. This is a
        // statement about knip's output, not a defect in the translation, and
        // `sarif_run` is where the missing bit is computed from the exit code.
        let report = parse_report(CAPTURED_M14).unwrap();
        assert!(!report.declared_invocations);
        assert!(!report.declared_artifacts);

        let bare = Run {
            tool: Tool {
                name: report.tool_name.clone(),
                version: report.tool_version.clone(),
            },
            invocations: Vec::new(),
            artifacts: Vec::new(),
            results: Vec::new(),
            baseline_guid: None,
        };
        assert!(matches!(
            assess_run_health(&bare, 0),
            RunHealth::Failed { .. }
        ));
    }

    #[test]
    fn a_knip_run_can_never_be_healthy_under_the_ss9_2_contract() {
        // Measured, and it contradicts what this adapter's author first wrote
        // down. Knip's SARIF declares no `artifacts`, so §9.2's positive control
        // -- the clause the research calls the most valuable in the contract --
        // has nothing to check. `assess_run_health` refuses at zero expected
        // targets as well as at ten: "coverage cannot be validated". The best a
        // knip run can reach is Degraded.
        let report = parse_report(CAPTURED_M14).unwrap();
        let run = sarif_run(&report, Some(1), "");
        assert_eq!(run.tool.name, "knip");
        assert_eq!(run.tool.version.as_deref(), Some("6.31.0"));
        assert_eq!(run.results.len(), 6);
        assert!(
            run.invocations[0].execution_successful,
            "the health bit the adapter computes from the exit code IS assertable"
        );
        assert!(
            run.artifacts.is_empty(),
            "an artifact invented here would forge §9.2's positive control"
        );

        for expected in [0usize, 10] {
            let RunHealth::Degraded { reasons } = assess_run_health(&run, expected) else {
                panic!("a log with no analysisTarget must not read as healthy or failed");
            };
            assert!(
                reasons.iter().any(|r| r.contains("analysisTarget")),
                "the coverage gate must say what is missing: {reasons:?}"
            );
        }
    }

    #[test]
    fn the_vite_config_failure_degrades_the_run_without_losing_its_claims() {
        // §6.20's named knip case. stderr carries the ERROR lines and stdout
        // carries an unresolved import -- a hole in the graph the file claim was
        // computed from. Both must reach the report; neither may delete a claim.
        let report = parse_report(CAPTURED_VITE_STDOUT).unwrap();
        let verdict = verdict_from_findings(&report.findings);
        assert_eq!(
            paths(&verdict),
            vec!["src/orphan.ts"],
            "degradation must not suppress what knip claimed"
        );

        let run = sarif_run(&report, Some(1), CAPTURED_VITE_STDERR);
        let RunHealth::Degraded { reasons } = assess_run_health(&run, 0) else {
            panic!("a run that could not load vite.config.ts is not healthy");
        };
        let joined = reasons.join("\n");
        assert!(
            joined.contains("Error loading vite.config.ts (Cannot find module 'vite')"),
            "the operator's only clue must be carried verbatim: {joined}"
        );
        assert!(
            joined.contains("Unresolved import: ./config/aliases"),
            "an edge knip could not follow is a hole in the graph: {joined}"
        );
    }

    #[test]
    fn symbol_findings_claim_the_bare_name_and_keep_the_qualifier() {
        let report = parse_report(CAPTURED_SYMBOLS).unwrap();
        let enum_member = report
            .findings
            .iter()
            .find(|finding| finding.rule_id == "knip/enumMembers")
            .unwrap();
        assert_eq!(
            enum_member.message,
            "Unused exported enum member: Blue (Colour)"
        );
        assert_eq!(enum_member.payload, "Blue");
        assert_eq!(enum_member.qualifier.as_deref(), Some("Colour"));

        let verdict = verdict_from_findings(&report.findings);
        assert_eq!(
            symbol_names(&verdict),
            vec!["Blue", "UnusedType", "Widget", "unusedExport"]
        );
        assert!(
            verdict.claimed_dead_paths.is_empty(),
            "an export finding names a file it lives in, which is not a claim that the \
             file is dead"
        );
    }

    #[test]
    fn a_default_export_has_no_qualifier_to_strip() {
        let report = parse_report(CAPTURED_NAMESPACES).unwrap();
        let verdict = verdict_from_findings(&report.findings);
        assert_eq!(
            symbol_names(&verdict),
            vec!["ApiType", "Never", "Square", "default", "unusedApi"]
        );
        let default = report
            .findings
            .iter()
            .find(|finding| finding.payload == "default")
            .unwrap();
        assert_eq!(default.qualifier, None);
    }

    #[test]
    fn manifest_and_inverse_and_cycle_findings_produce_no_claim() {
        // MAPPING_DECISION, made executable: a package name is not a symbol, an
        // unlisted dependency is the inverse of a deadness claim, and a cycle is
        // not a claim at all.
        let report = parse_report(CAPTURED_MIXED).unwrap();
        let verdict = verdict_from_findings(&report.findings);

        assert_eq!(paths(&verdict), vec!["src/dupes.ts"]);
        assert_eq!(symbol_names(&verdict), vec!["UnusedNsType", "unusedNs"]);
        for never in ["left-pad", "some-unlisted-package", "src/cycleA.ts"] {
            assert!(
                !verdict
                    .claimed_dead_symbols
                    .iter()
                    .any(|s| s.name() == never),
                "{never} reached claimed_dead_symbols"
            );
            assert!(
                !paths(&verdict).iter().any(|p| p == never),
                "{never} reached claimed_dead_paths"
            );
        }

        assert_eq!(manifest_claims(&report.findings), vec!["left-pad"]);

        let cycle = report
            .findings
            .iter()
            .find(|finding| finding.rule_id == "knip/cycles")
            .unwrap();
        assert_eq!(cycle.level, Level::Warning);
        assert_eq!(cycle.claim, Claim::None);
        assert_eq!(cycle.payload, "src/cycleA.ts, src/cycleB.ts");
    }

    #[test]
    fn help_text_on_stdout_is_never_a_verdict() {
        // The trap `CommandSut` walks into by default: it appends the repo path,
        // knip refuses positional arguments, prints help to STDOUT, and exits 1
        // -- the same code a healthy run uses. A tolerant parser reports a clean
        // repository for a tool that never ran.
        let error = parse(CAPTURED_POSITIONAL_HELP).unwrap_err().to_string();
        assert!(
            error.contains("not one knip SARIF log"),
            "unhelpful error: {error}"
        );

        let notifications = degradation(CAPTURED_POSITIONAL_STDERR);
        assert_eq!(notifications.len(), 1);
        assert_eq!(notifications[0].level, Level::Error);
        assert!(notifications[0].message.contains("Unexpected argument"));
    }

    #[test]
    fn the_two_abnormal_exits_are_errors_on_both_channels() {
        for (stdout, stderr, clue) in [
            (
                CAPTURED_NO_MANIFEST_STDOUT,
                CAPTURED_NO_MANIFEST_STDERR,
                "Unable to find package.json",
            ),
            (
                CAPTURED_BAD_CONFIG_STDOUT,
                CAPTURED_BAD_CONFIG_STDERR,
                "Error parsing",
            ),
        ] {
            assert!(
                parse(stdout).is_err(),
                "non-JSON stdout produced a verdict: {stdout:?}"
            );
            let invocation = invocation(Some(2), stderr, &[]);
            assert!(
                !invocation.execution_successful,
                "exit 2 read as a completed analysis"
            );
            let joined: Vec<&str> = invocation
                .tool_execution_notifications
                .iter()
                .map(|n| n.message.as_str())
                .collect();
            assert!(
                joined.iter().any(|m| m.contains(clue)),
                "the reason was dropped: {joined:?}"
            );
        }
    }

    #[test]
    fn a_signal_death_is_never_a_completed_analysis() {
        let invocation = invocation(None, "", &[]);
        assert!(!invocation.execution_successful);
        assert!(invocation.tool_execution_notifications[0]
            .message
            .contains("killed by a signal"));
    }

    #[test]
    fn configuration_hints_from_the_default_reporter_degrade_the_run() {
        // The hints exist -- on stderr, from the DEFAULT reporter only. A caller
        // that runs knip that way gets them; a caller using --reporter sarif
        // gets nothing, which is why this is an envelope item and not a feature.
        let notifications = degradation(CAPTURED_CONFIG_HINTS_STDERR);
        assert_eq!(notifications.len(), 4, "header plus three hints");
        assert!(notifications
            .iter()
            .all(|n| n.level == Level::Warning || n.level == Level::Error));
        let joined = notifications
            .iter()
            .map(|n| n.message.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            joined.contains("Refine entry pattern (no matches)"),
            "the one line that means the entry set resolved to nothing was dropped: {joined}"
        );

        let invocation = invocation(Some(1), CAPTURED_CONFIG_HINTS_STDERR, &[]);
        assert!(
            invocation.execution_successful,
            "hints degrade a run, they do not fail it"
        );
        let run = Run {
            tool: Tool {
                name: TOOL.to_string(),
                version: None,
            },
            invocations: vec![invocation],
            artifacts: Vec::new(),
            results: Vec::new(),
            baseline_guid: None,
        };
        assert!(matches!(
            assess_run_health(&run, 0),
            RunHealth::Degraded { .. }
        ));
    }

    #[test]
    fn an_empty_log_is_a_clean_run_not_an_error() {
        let verdict = parse(CAPTURED_CLEAN).unwrap();
        assert_eq!(verdict, SutVerdict::default());
        let report = parse_report(CAPTURED_CLEAN).unwrap();
        assert!(report.findings.is_empty());
        assert!(sarif_run(&report, Some(0), "").invocations[0].execution_successful);
    }

    #[test]
    fn an_unmapped_rule_id_is_an_error_never_a_shorter_verdict() {
        // Deliberately the opposite of the vulture adapter's choice. A vulture
        // line's shape already fixes it as a symbol claim; a knip ruleId is the
        // only thing that says whether a finding is about a file, a symbol, a
        // manifest entry or nothing, so a new one has no safe default.
        let forged = CAPTURED_M14.replace("knip/binaries", "knip/futureIssueType");
        let error = parse(&forged).unwrap_err().to_string();
        assert!(error.contains("knip/futureIssueType"), "unhelpful: {error}");
        assert!(error.contains("MAPPING_DECISION"), "no next step: {error}");
    }

    #[test]
    fn another_tools_sarif_log_is_refused() {
        let forged = CAPTURED_M14.replace(r#""name":"knip""#, r#""name":"eslint""#);
        let error = parse(&forged).unwrap_err().to_string();
        assert!(error.contains("eslint"), "unhelpful: {error}");
    }

    #[test]
    fn malformed_logs_are_errors_never_an_empty_verdict() {
        let malformed: Vec<(&str, String)> = vec![
            ("empty stream", String::new()),
            ("blank stream", "   \n\n".to_string()),
            (
                "truncated mid-document, e.g. a closed pipe",
                CAPTURED_M14[..CAPTURED_M14.len() / 2].to_string(),
            ),
            (
                "truncated inside a string",
                CAPTURED_M14
                    .split_once("Unused file: dist/widget.0c")
                    .map(|(head, _)| format!("{head}Unused file: dist/widget.0c"))
                    .unwrap(),
            ),
            ("not JSON at all", "Unused file: src/a.ts\n".to_string()),
            ("a JSON array, not a log", "[]".to_string()),
            ("an object with no runs", r#"{"version":"2.1.0"}"#.to_string()),
            ("runs is not an array", r#"{"runs":{}}"#.to_string()),
            ("no runs at all", r#"{"runs":[]}"#.to_string()),
            (
                "two runs, whose health cannot be attributed",
                format!(
                    r#"{{"runs":[{},{}]}}"#,
                    r#"{"tool":{"driver":{"name":"knip"}},"results":[]}"#,
                    r#"{"tool":{"driver":{"name":"knip"}},"results":[]}"#
                ),
            ),
            (
                "no tool.driver.name",
                r#"{"runs":[{"tool":{"driver":{}},"results":[]}]}"#.to_string(),
            ),
            (
                "results is not an array",
                r#"{"runs":[{"tool":{"driver":{"name":"knip"}},"results":{}}]}"#.to_string(),
            ),
            (
                "a result with no ruleId",
                r#"{"runs":[{"tool":{"driver":{"name":"knip"}},"results":[{"level":"error","message":{"text":"Unused file: a.ts"}}]}]}"#.to_string(),
            ),
            (
                "a result with no message.text",
                r#"{"runs":[{"tool":{"driver":{"name":"knip"}},"results":[{"ruleId":"knip/files","level":"error"}]}]}"#.to_string(),
            ),
            (
                "a level knip cannot emit",
                r#"{"runs":[{"tool":{"driver":{"name":"knip"}},"results":[{"ruleId":"knip/files","level":"critical","message":{"text":"Unused file: a.ts"},"locations":[{"physicalLocation":{"artifactLocation":{"uri":"a.ts"}}}]}]}]}"#.to_string(),
            ),
            (
                "a message with no `<label>: <payload>` shape",
                r#"{"runs":[{"tool":{"driver":{"name":"knip"}},"results":[{"ruleId":"knip/files","level":"error","message":{"text":"something happened"},"locations":[{"physicalLocation":{"artifactLocation":{"uri":"a.ts"}}}]}]}]}"#.to_string(),
            ),
            (
                "a file claim with no path to claim",
                r#"{"runs":[{"tool":{"driver":{"name":"knip"}},"results":[{"ruleId":"knip/files","level":"error","message":{"text":"Unused file: a.ts"}}]}]}"#.to_string(),
            ),
            (
                "a file claim with an empty uri",
                r#"{"runs":[{"tool":{"driver":{"name":"knip"}},"results":[{"ruleId":"knip/files","level":"error","message":{"text":"Unused file: a.ts"},"locations":[{"physicalLocation":{"artifactLocation":{"uri":""}}}]}]}]}"#.to_string(),
            ),
            (
                "a startLine that is not a whole number",
                r#"{"runs":[{"tool":{"driver":{"name":"knip"}},"results":[{"ruleId":"knip/files","level":"error","message":{"text":"Unused file: a.ts"},"locations":[{"physicalLocation":{"artifactLocation":{"uri":"a.ts"},"region":{"startLine":1.5}}}]}]}]}"#.to_string(),
            ),
            (
                "two JSON documents concatenated",
                format!("{CAPTURED_CLEAN}{CAPTURED_CLEAN}"),
            ),
        ];
        for (why, stream) in malformed {
            let parsed = parse(&stream);
            assert!(
                parsed.is_err(),
                "silently accepted {why}: {:?}",
                &stream[..stream.len().min(120)]
            );
        }
    }

    #[test]
    fn a_claim_reported_twice_is_claimed_once() {
        let doubled = CAPTURED_SYMBOLS.replace(
            r#"{"ruleId":"knip/exports","ruleIndex":0,"level":"error","message":{"text":"Unused export: Widget"},"locations":[{"physicalLocation":{"artifactLocation":{"uri":"src/lib.ts"},"region":{"startLine":12,"startColumn":14,"endColumn":20}}}]}"#,
            r#"{"ruleId":"knip/exports","ruleIndex":0,"level":"error","message":{"text":"Unused export: Widget"},"locations":[{"physicalLocation":{"artifactLocation":{"uri":"src/lib.ts"},"region":{"startLine":12,"startColumn":14,"endColumn":20}}}]},{"ruleId":"knip/exports","ruleIndex":0,"level":"error","message":{"text":"Unused export: Widget"},"locations":[{"physicalLocation":{"artifactLocation":{"uri":"src/other.ts"},"region":{"startLine":1,"startColumn":1,"endColumn":7}}}]}"#,
        );
        let report = parse_report(&doubled).unwrap();
        assert_eq!(report.findings.len(), 5, "the forged log has five results");
        let verdict = verdict_from_findings(&report.findings);
        assert_eq!(
            symbol_names(&verdict),
            vec!["Blue", "UnusedType", "Widget", "unusedExport"]
        );
        let widget = verdict
            .claimed_dead_symbols
            .iter()
            .find(|claim| claim.name() == "Widget")
            .expect("Widget is claimed");
        assert_eq!(
            widget.declaration_site(),
            None,
            "the forged log reports Widget from src/lib.ts AND src/other.ts, so \
             there is no single module for Gate 2a to exclude; excluding one \
             would let it find the symbol in the other"
        );
    }

    #[test]
    fn files_touched_reports_the_blast_radius_it_does_not_claim() {
        let report = parse_report(CAPTURED_M14).unwrap();
        assert_eq!(
            files_touched(&report.findings),
            vec![
                PathBuf::from("dist/widget.0c9e142.js"),
                PathBuf::from("dist/widget.7f3a91c.js"),
                PathBuf::from("package.json"),
                PathBuf::from("src/unusedFeatureFlags.ts"),
            ]
        );
        assert!(
            !paths(&verdict_from_findings(&report.findings))
                .iter()
                .any(|path| path == "package.json"),
            "naming package.json as a finding's location is not claiming it is dead"
        );
    }

    #[test]
    fn the_level_never_changes_what_is_claimed() {
        // The same refusal the vulture adapter makes about confidence. Knip's
        // levels come from a static per-rule defaultConfiguration, so rewriting
        // every one of them must not move a single claim.
        let baseline = parse(CAPTURED_MIXED).unwrap();
        for level in ["none", "note", "warning", "error"] {
            let rewritten = CAPTURED_MIXED
                .replace(r#""level":"error""#, &format!(r#""level":"{level}""#))
                .replace(r#""level":"warning""#, &format!(r#""level":"{level}""#));
            assert_eq!(
                parse(&rewritten).unwrap(),
                baseline,
                "a claim moved when every level became {level:?}"
            );
        }
    }

    #[test]
    fn the_read_only_rule_is_visible_in_the_recommended_invocation() {
        // §9.2 rule 1, and §7.5's two-gate pattern. Neither gate may appear.
        for forbidden in ["--fix", "-f", "--allow-remove-files", "--format", "-F"] {
            assert!(
                !RECOMMENDED_ARGS.contains(&forbidden),
                "the recommended invocation would let knip mutate the repository: {forbidden}"
            );
        }
        assert!(RECOMMENDED_ARGS.contains(&"sarif"));
    }

    #[test]
    fn the_envelope_and_the_mapping_decision_are_reportable() {
        assert!(CAPABILITY_ENVELOPE.contains("silence is not evidence"));
        assert!(CAPABILITY_ENVELOPE.contains("entry set"));
        assert!(CAPABILITY_ENVELOPE.contains("Refine entry pattern (no matches)"));
        assert!(MAPPING_DECISION.contains("--allow-remove-files"));
        assert!(MAPPING_DECISION.contains("CLAIMED_DEAD_PATHS"));
        assert!(MAPPING_DECISION.contains("CARRIED BUT NOT CLAIMED"));
    }

    #[test]
    fn the_envelope_comes_in_the_shape_the_sut_trait_asks_for() {
        let classes = cannot_emit();
        assert!(
            classes.len() >= 3,
            "an envelope of {} classes",
            classes.len()
        );
        let joined = classes.join("\n");
        assert!(joined.contains("non-JS/TS"));
        assert!(joined.contains("not evidence"));
        assert!(joined.contains("sarif"));
    }

    #[test]
    fn the_envelope_declares_silence_and_never_excuses_a_false_positive() {
        // §4.1's measured false-positive modes are knip saying something WRONG,
        // loudly. They are what E2 counts; declaring them as blind spots would
        // excuse the number.
        let joined = cannot_emit().join("\n");
        for excuse in [
            "template",
            "CJS",
            "script src",
            "auto-mock",
            "auto-import",
            "plugin",
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

    #[test]
    fn the_json_reader_rejects_what_a_substring_scan_would_swallow() {
        // Each of these is a way a truncated or hostile stream could otherwise
        // become a short, plausible verdict.
        for bad in [
            r#"{"a":1"#,
            r#"{"a":}"#,
            r#"{"a" 1}"#,
            r#"{a:1}"#,
            "[1,]",
            "[,1]",
            r#""unterminated"#,
            r#""bad \q escape""#,
            r#""\ud83d""#,
            r#""\ud83dnot-low""#,
            r#""\u00""#,
            "01",
            "1.",
            "1e",
            "+1",
            "tru",
            "{} {}",
            "",
        ] {
            assert!(
                json::parse(bad).is_err(),
                "the JSON reader accepted {bad:?}"
            );
        }
        // And accepts what SARIF actually contains, including an astral escape
        // in a path and a negative exponent in a number.
        let value = json::parse(r#"{"uri":"src/🚀.ts","n":-1.5e-3,"ok":[true,null]}"#).unwrap();
        let fields = value.object().unwrap();
        assert_eq!(
            json::get(fields, "uri").unwrap().string(),
            Some("src/🚀.ts")
        );
        assert_eq!(json::get(fields, "n").unwrap().integer(), None);
    }

    #[test]
    fn a_path_containing_a_json_escape_survives_the_round_trip() {
        // Not captured: it needs a filename with a quote in it, which macOS
        // permits. It pins that the claim comes from the decoded uri and not
        // from a substring of the raw stream.
        let forged = r#"{"runs":[{"tool":{"driver":{"name":"knip","version":"6.31.0"}},"results":[{"ruleId":"knip/files","level":"error","message":{"text":"Unused file: src/od\"d.ts"},"locations":[{"physicalLocation":{"artifactLocation":{"uri":"src/od\"d.ts"}}}]}]}]}"#;
        let verdict = parse(forged).unwrap();
        assert_eq!(paths(&verdict), vec![r#"src/od"d.ts"#]);
    }
}
