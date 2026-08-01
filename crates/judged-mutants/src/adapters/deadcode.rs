//! `golang.org/x/tools/cmd/deadcode` (Go) — `-json` stdout to [`SutVerdict`].
//!
//! §2.1 calls this the proof-grade row of the catalogue, and the description is
//! earned: it loads the program from source, builds a call graph by Rapid Type
//! Analysis from every `main` and `init`, and is sound through func values,
//! interface dispatch and reflection. Nothing else in §4.1 makes that claim.
//!
//! It is also the only tool in the catalogue that documents its own breaking
//! point, in its own `-help` text:
//!
//! ```text
//! The analysis can soundly analyze dynamic calls though func values,
//! interface methods, and reflection. However, it does not currently
//! understand the aliasing created by //go:linkname directives, so it
//! will fail to recognize that calls to a linkname-annotated function
//! with no body in fact dispatch to the function named in the annotation.
//! This may result in the latter function being spuriously reported as dead.
//! ```
//!
//! and, for §6.4:
//!
//! ```text
//! The analysis is valid only for a single GOOS/GOARCH/-tags configuration,
//! so a function reported as dead may be live in a different configuration.
//! ```
//!
//! `crate::fixtures::m12_linkname_alias` is built from exactly the first shape,
//! plus a cgo `//export`. What this adapter grades is therefore a prediction
//! §4.1 makes and the tool itself concedes.
//!
//! # The output protocol, as captured
//!
//! `deadcode -json` writes `json.MarshalIndent(objects, "", "\t")` to stdout: a
//! top-level array of `Package{Name, Path, Funcs}`, each `Function` carrying
//! `Name`, `Position{File, Line, Col}`, `Generated` and `Marker`. Real bytes
//! from a materialized m12, x/tools v0.48.0 / go1.26.2 darwin-arm64, with the
//! tab indentation rendered as spaces here and kept verbatim in the tests:
//!
//! ```text
//! [
//!     {
//!         "Name": "sampler",
//!         "Path": "example.com/m12/telemetry/internal/sampler",
//!         "Funcs": [
//!             {
//!                 "Name": "drain",
//!                 "Position": {
//!                     "File": "internal/sampler/drain.go",
//!                     "Line": 9,
//!                     "Col": 6
//!                 },
//!                 "Generated": false,
//!                 "Marker": false
//!             }
//!         ]
//!     }
//! ]
//! ```
//!
//! There is no trailing newline, and an empty result is the literal four bytes
//! `null` — `packages` is a nil `[]any`, and Go marshals a nil slice as `null`,
//! not `[]`.
//!
//! `Position.File` is made relative to the process's working directory when it
//! lies underneath it and is absolute otherwise (`toJSONPosition`), so running
//! the tool with the repository root as its working directory is what makes
//! [`files_with_dead_functions`] repo-relative. This adapter does not know the
//! root and does not re-root anything.
//!
//! Everything here is a pure function of that text. No process is spawned, no
//! file is read, and the parser is testable with Go not installed.
//!
//! # Exit codes, measured rather than assumed
//!
//! Against x/tools v0.48.0, because [`crate::sut::CommandSut`] discards the
//! stdout of a run that ended on an unlisted code and a wrong list here is a
//! silent scoring error in either direction:
//!
//! | Condition | Exit | stdout |
//! | --- | --- | --- |
//! | Dead functions found | 0 | the `Package` array |
//! | Analysis ran, nothing dead (or `-filter` matched nothing) | 0 | `null` |
//! | No main package among the targets | 1 | empty |
//! | Packages contain errors | 1 | empty |
//! | `-json` combined with `-f=template` | 1 | empty |
//! | Unknown flag, or no package arguments | 2 | usage text |
//!
//! So **only 0 is a completed run**, and the productive and empty cases share
//! it. That is the §6.20 shape this crate exists to refuse: the tool cannot tell
//! a caller apart from stdout whether it analyzed a program or refused to.
//!
//! # Why this parser hand-rolls JSON
//!
//! `judged-mutants` depends on `judged-core` and `tempfile` and on nothing else,
//! and `serde_json` is an internal dependency of `judged-core` rather than part
//! of its public surface. Rather than widen the crate's dependency set for one
//! adapter, [`parse_packages`] uses a strict reader scoped to the grammar
//! `encoding/json` emits — including the `\u0026` / `\u003c` / `\u003e`
//! escapes Go's HTML-safe encoder produces for `&`, `<` and `>`, which a real
//! capture contains whenever a Go file name holds one of those characters.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use judged_core::{Error, Result};

use crate::mutant::Ecosystem;
use crate::sut::{SutVerdict, SymbolClaim};

/// What deadcode can and cannot say, in the form §9.2 requires of every adapter.
///
/// §9.2's first non-SARIF clause requires each adapter to declare the finding
/// classes the tool structurally *cannot* emit, because that is what lets the
/// orchestrator know when the tool's silence means anything.
///
/// One entry below — the `//go:linkname` / assembly / cgo caller — sits on the
/// line the vulture adapter draws between *a declared blind spot* and *a known
/// mistake*, and it is worth being explicit about why it is here. It is
/// declared because the missing edge is structural: to the Go type checker a
/// `//go:linkname` directive is a comment and an assembly or C caller is not in
/// the program at all, so no amount of RTA can recover the call. It is **not**
/// an excuse for the answer that follows. A missing edge in an
/// unreachability analysis does not come out as silence, it comes out as an
/// accusation, and that accusation is exactly what §10 E2 class 12 grades. The
/// envelope explains the mechanism; the score still counts the removal.
/// The ecosystems deadcode can load a repository from, for
/// [`crate::sut::CommandSut::with_reads`].
///
/// deadcode loads a Go program from source with `packages.Load`, which needs a
/// module — a `go.mod` and packages the Go toolchain can type-check. There is no
/// partial reading: a directory with no Go in it produces `no Go files in <dir>`
/// or `cannot find main module`, empty stdout and exit 1 (measured 2026-08-01,
/// x/tools with go1.26.2). That exit code is shared with "your Go does not
/// compile", so it cannot be declared healthy (§6.20) and the class has to be
/// skipped before the tool is spawned instead.
pub const READS: &[Ecosystem] = &[Ecosystem::Go];

pub const CAPABILITY_ENVELOPE: &str = "\
deadcode computes whole-program Rapid Type Analysis from every main and init in \
the packages it was given, and is sound through func values, interface dispatch \
and reflection. Its findings are proof-grade within that scope (§2.1). \
Everything below is what that sentence excludes.

Structurally cannot emit:

(1) Any finding that is not a Go FUNCTION. The output protocol has exactly one \
record type -- Package{Name,Path,Funcs}, where Funcs is a list of dead \
functions -- so a dead type, const, var, struct field, file, package, module \
dependency, build file, CI step, YAML task or checked-in asset is not \
expressible in it at all.

(2) Any finding outside the single GOOS/GOARCH/-tags configuration it was run \
in. Its own documentation: \"The analysis is valid only for a single \
GOOS/GOARCH/-tags configuration, so a function reported as dead may be live in \
a different configuration.\" Measured rather than quoted: under CGO_ENABLED=0 \
the entire cmd/libtelemetry main package disappears from the m12 report -- exit \
0, nothing on stderr, and no mention anywhere of the package it did not \
analyze.

(3) Anything at all when the targets contain no main package. Only main \
packages are roots. Pointed at a library it exits 1 with \"deadcode: no main \
packages\" on stderr and writes nothing to stdout; that silence is a refusal, \
not a clean bill of health, and §6.20 records reading it as \"nothing is dead\" \
as the failure mode. This adapter refuses empty stdout for that reason.

(4) A live call edge from assembly, from cgo, or through a //go:linkname alias. \
The linker resolves those names and no parser does, so the edge is absent from \
the call graph by construction. Its own documentation: it \"does not currently \
understand the aliasing created by //go:linkname directives ... This may result \
in the latter function being spuriously reported as dead.\" Note what a missing \
edge does to an unreachability analysis: it does not produce silence, it \
produces an accusation. This entry explains the mechanism and excuses nothing; \
the accusation is precisely what §10 E2 class 12 counts.

(5) A distinction between \"nothing is dead\" and \"the -filter matched no \
package\". Both print the literal four bytes `null` and exit 0.

(6) A marker interface method, and -- without -generated -- any function \
declared in a generated file. Both are dropped before the record is built, so \
the Funcs list of a package is not the list of its dead functions, and a \
package whose every reported function is dead may still contain functions the \
tool chose not to report.";

/// [`CAPABILITY_ENVELOPE`] in the shape [`crate::sut::Sut::cannot_emit`] wants:
/// one prose class per entry, so a report can list them and a `Sut` impl can
/// return them without restating anything.
pub fn cannot_emit() -> Vec<String> {
    [
        "any finding that is not a Go function: the output protocol carries only \
         Package{Name,Path,Funcs} where Funcs lists dead functions, so a dead type, const, var, \
         struct field, file, package, dependency or non-Go artifact is not expressible in it",
        "any finding outside the one GOOS/GOARCH/-tags configuration it ran in: its own \
         documentation says the analysis is valid for a single configuration only, and under \
         CGO_ENABLED=0 an entire main package leaves the m12 report with exit 0 and no mention",
        "anything at all when the targets contain no main package: main and init are the only \
         roots, so pointed at a library it exits 1 with \"deadcode: no main packages\" and \
         writes nothing, and that silence is a refusal rather than a clean bill of health",
        "a live call edge from assembly, from cgo, or through a //go:linkname alias: the linker \
         resolves those names and the type checker sees a comment, so the edge is absent by \
         construction -- and a missing edge in an unreachability analysis surfaces as an \
         accusation, not as silence",
        "a distinction between \"nothing is dead\" and \"the -filter matched no package\": both \
         print the literal four bytes `null` and exit 0",
        "a marker interface method, and without -generated any function in a generated file: \
         both are dropped before the record is built, so a package's Funcs list is not the list \
         of its dead functions",
    ]
    .iter()
    .map(|class| (*class).to_string())
    .collect()
}

/// Which half of [`SutVerdict`] a deadcode finding is allowed to fill, and why.
///
/// **Chosen: symbols only. `claimed_dead_paths` is always empty.**
///
/// The question the mapping has to answer is whether a package — or a file — in
/// which every reported function is dead justifies a path claim. It does not,
/// and the reason is that the premise is not derivable from this output.
///
/// The cost of that refusal is real, is paid on the decoy half of the grade,
/// and is spelled out in the constant rather than left for a reader to
/// discover.
pub const MAPPING_DECISION: &str = "\
deadcode reports dead FUNCTIONS. This adapter maps each Funcs entry to one \
claimed_dead_symbols entry, spelled exactly as the tool spelled it -- `drain`, \
`Ledger.Add`, `_cgo_cmalloc` -- and NEVER to a claimed_dead_paths entry: \
claimed_dead_paths is always empty.

A package or file whose every reported function is dead does NOT justify a path \
claim. \"Every function here is dead\" is not derivable from this output: the \
JSON lists only the functions found unreachable and never states how many a \
package or a file contains, and v0.48.0 drops marker interface methods and \
(without -generated) every function in a generated file BEFORE building the \
record, so a complete-looking listing can coexist with live functions in the \
same package. Beyond functions the protocol says nothing whatever about types, \
consts, vars or struct fields, each of which keeps a file alive on its own.

Inferring the path claim anyway would not merely be unsupported, it would move \
the number this suite exists to produce, in the flattering direction. In m12 \
every function reported in either live file is reported dead -- drain is the \
whole of internal/sampler/drain.go, and TelemetryFlush plus the two cgo \
pseudo-functions are the whole of cmd/libtelemetry/abi.go -- so an \"all \
reported functions dead => file dead\" rule would manufacture two path-level \
false removals on top of the two symbol-level ones deadcode really made, \
doubling the count with claims written here rather than by the tool. §11 R1's \
pre-committed consequence is \
that the auto-act tier is DELETED rather than tuned; an answer that heavy has \
to be attributable to the analyzer.

Consequence, stated because it biases the score in deadcode's favour: decoy \
recall is credited from whichever claim list runner::grade intersects with the \
suite's ground truth, and this adapter fills only the symbol list. On m12 \
deadcode names both planted decoys correctly and by name -- legacyHistogram and \
unusedPercentile -- and names neither by path, so a grade that credits decoys \
from claimed_dead_paths alone scores this adapter 0 of 2 there and marks the \
mutant failed on the decoy half as well as the false-removal half. The reported \
false removals (drain, TelemetryFlush) are the finding and are real; the decoy \
column is an artifact of this mapping and must be read with this paragraph next \
to it.

Nothing is filtered on the way through. The m12 run reports two cgo \
pseudo-functions -- runtime_throw at cmd/libtelemetry/abi.go:27:0 and \
_cgo_cmalloc at :30:0, in a file that is 22 lines long -- and both are carried \
into the verdict. They are claims deadcode made; dropping them because they \
look synthetic, or because their positions point past the end of their file, \
would measure the adapter rather than the tool.";

/// One entry of a package's `Funcs` array.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct DeadFunction {
    /// `prettyName(fn, false)`: unqualified by package, but qualified by
    /// receiver for a method — `drain`, `Ledger.Add`, `Ledger.deadMethod`, and
    /// `f$1` for an anonymous function. Carried exactly as printed;
    /// [`crate::runner`] does the trailing-segment matching against ground
    /// truth, and reshaping the name here would be editing the tool's claim.
    pub name: String,
    /// `Position.File`. Relative to the working directory the tool ran in when
    /// the file lies underneath it, absolute otherwise. Not re-rooted: this
    /// adapter does not know the repository root, and guessing one would
    /// mis-key every comparison downstream.
    pub file: PathBuf,
    /// `Position.Line`.
    pub line: u32,
    /// `Position.Col`. Documented as 1-based; observed to be `0` for cgo
    /// pseudo-functions, so nothing here assumes otherwise.
    pub column: u32,
    /// `Generated` — the function is declared in a file carrying the
    /// `https://go.dev/s/generatedcode` marker. Only ever `true` when the run
    /// passed `-generated`; without it such functions are omitted entirely.
    pub generated: bool,
    /// `Marker` — the function is a marker interface method.
    ///
    /// Always `false` in v0.48.0: markers are `continue`d out of the loop that
    /// builds these records, so the field can only ever be printed as `false`.
    /// It is still parsed and carried, because a release that started reporting
    /// them would otherwise be silently truncated by this adapter — the §6.20
    /// failure, arriving through a dependency upgrade.
    pub marker: bool,
}

/// One element of deadcode's top-level array.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct DeadPackage {
    /// `Name` — the package's declared name (`main`, `sampler`).
    pub name: String,
    /// `Path` — its full import path.
    pub path: String,
    /// `Funcs` — the dead functions the tool reported *for this configuration*,
    /// which per [`CAPABILITY_ENVELOPE`] is not the same as the package's dead
    /// functions.
    pub funcs: Vec<DeadFunction>,
}

/// The tool name every error from this module carries.
const TOOL: &str = "deadcode";

fn refuse(message: impl Into<String>) -> Error {
    Error::Sut {
        sut: TOOL.to_string(),
        message: message.into(),
    }
}

/// Parse the stdout of `deadcode -json <packages>`.
///
/// # Errors
///
/// Every shape that is not this tool's `Package` protocol, because §6.20's rule
/// is that *"no data" must be a distinct state from "zero executions"* and every
/// failure it catalogues "presents as clean output". A parser that shrugged and
/// returned what it could understand would turn each of the cases below into a
/// short, plausible, wrong verdict — which is the shape a cleaner reads as
/// permission to proceed.
///
/// Three of them get their own message rather than the generic one, because all
/// three are otherwise indistinguishable from a quiet, healthy run:
///
/// * **Empty stdout.** The tool writes its whole report to stdout and exits 0
///   whenever the analysis ran, so nothing on stdout means it did not run. The
///   case that matters is `deadcode: no main packages` — §6.20 records that
///   deadcode "pointed at a library reports the entire library dead"; v0.48.0
///   has closed that hole by refusing outright, but the refusal is on *stderr*
///   with exit 1 and its stdout is empty, so a caller reading only stdout still
///   sees a clean run.
/// * **Not JSON.** The default format is a line-oriented compiler diagnostic and
///   `-f=<template>` is whatever the caller asked for. Both exit 0.
/// * **`-whylive` output.** It reuses the same top-level array for `Edge`
///   records, which parse as JSON perfectly well and contain no `Funcs`. A
///   tolerant parser would report zero dead functions for a run that never
///   looked for any.
pub fn parse_packages(stdout: &str) -> Result<Vec<DeadPackage>> {
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        return Err(refuse(
            "stdout is empty. deadcode writes its whole report to stdout and exits 0 whenever \
             the analysis ran, so empty stdout means it did not run. The case to check first is \
             `deadcode: no main packages` on stderr with exit 1: main and init are the only \
             roots, so a library has none and the tool refuses rather than answering. Its \
             silence is not evidence that nothing is dead (§6.20). Check the exit status and \
             stderr; this run must not be graded",
        ));
    }
    if !trimmed.starts_with('[') && !trimmed.starts_with("null") {
        let first = trimmed.lines().next().unwrap_or_default();
        return Err(refuse(format!(
            "stdout is not the JSON package array. Pass `-json`: without it deadcode prints \
             line-oriented diagnostics (`file.go:9:6: unreachable func: drain`) and with \
             `-f=<template>` it prints whatever the template says, and neither can be told from \
             a finding stream. If `-json` was passed, the run failed before analysis and this is \
             diagnostic text — check the exit status. First line: {first:?}"
        )));
    }

    let value = json::parse(trimmed).map_err(|error| {
        refuse(format!(
            "stdout is not well-formed JSON at byte {}: {}. Near: {:?}",
            error.offset,
            error.reason,
            excerpt(trimmed, error.offset)
        ))
    })?;

    match value {
        // Go marshals the nil `[]any` of an empty result as `null`. Per
        // CAPABILITY_ENVELOPE (5) this is also what a `-filter` matching no
        // package prints, and the two are not distinguishable from stdout.
        json::Json::Null => Ok(Vec::new()),
        json::Json::Array(items) => items
            .iter()
            .enumerate()
            .map(|(index, item)| package(item, &format!("[{index}]")))
            .collect(),
        other => Err(refuse(format!(
            "stdout is JSON but not deadcode's output protocol: the top level is {} where an \
             array of Package objects (or `null`) was expected",
            kind_of(&other)
        ))),
    }
}

/// deadcode's stdout, as the verdict the suite grades. See [`MAPPING_DECISION`].
///
/// This is a [`crate::sut::VerdictParser`].
///
/// # Errors
///
/// Propagates [`parse_packages`]. A `null` stream is *not* an error: it is what
/// a completed run with nothing to report prints. Per [`CAPABILITY_ENVELOPE`]
/// that silence does not distinguish "nothing is dead" from "the `-filter`
/// excluded everything", and establishing that the run was actually in scope is
/// the caller's job — §9.2's health bit and positive control, which no amount of
/// stdout parsing can substitute for.
pub fn verdict_from_stdout(stdout: &str) -> Result<SutVerdict> {
    Ok(verdict_from_packages(&parse_packages(stdout)?))
}

/// The same mapping, applied to already-parsed packages, for a caller that also
/// wants to print them.
pub fn verdict_from_packages(packages: &[DeadPackage]) -> SutVerdict {
    // Sorted and deduplicated: two packages can each hold a method the protocol
    // spells `Ledger.Add`, and a claim list whose order and length depend on how
    // many packages a repository happens to have cannot be diffed between runs.
    // Collapsing duplicates cannot hide a false removal — grading asks whether a
    // live name was claimed at all, not how often.
    //
    // Each claim now carries `Position.File`, which this adapter has always
    // parsed into [`DeadFunction::file`] and never passed on. Gate 2a excludes
    // that file before asking whether anything references the symbol; without it
    // every symbol is found in its own declaration and rescued. See
    // [`crate::sut::SymbolClaim`].
    //
    // `SymbolClaim::dedup_by_name` collapses the duplicates and drops the site
    // when two packages disagree — `Ledger.Add` declared in two files has no one
    // declaration to exclude, and choosing would be the harness inventing a
    // fact.
    //
    // `function.file` is the path the go tool resolved: relative to the
    // process's working directory when the file lies under it, absolute
    // otherwise (see [`DeadFunction::file`]). `CommandSut::run` handles both,
    // being the only thing that knows where the repository is.
    let symbols = SymbolClaim::dedup_by_name(
        packages
            .iter()
            .flat_map(|package| &package.funcs)
            .map(|function| SymbolClaim::declared_in(&function.name, &function.file)),
    );
    SutVerdict {
        claimed_dead_paths: Vec::new(),
        claimed_dead_symbols: symbols,
    }
}

/// Every file some dead function was declared in, sorted and deduplicated.
///
/// This is the blast radius [`MAPPING_DECISION`] declines to grade: the files a
/// human acting on this run would have edited. It is deliberately **not**
/// [`SutVerdict::claimed_dead_paths`] — putting it there would be claiming on
/// deadcode's behalf that these files are dead, which it never said, and two of
/// the four entries it returns for m12 are files the suite knows are alive.
///
/// The positions of cgo pseudo-functions are included on the same terms as
/// everything else, so this list can name a file at a line the file does not
/// have.
pub fn files_with_dead_functions(packages: &[DeadPackage]) -> Vec<PathBuf> {
    let files: BTreeSet<&Path> = packages
        .iter()
        .flat_map(|package| &package.funcs)
        .map(|function| function.file.as_path())
        .collect();
    files.into_iter().map(Path::to_path_buf).collect()
}

/// One element of the top-level array, or the reason it is not a `Package`.
fn package(value: &json::Json, at: &str) -> Result<DeadPackage> {
    let fields = object(value, at)?;
    // `-whylive` prints Edge records into the same array shape. Recognized
    // before the missing-`Funcs` complaint, so the message names the flag to
    // drop instead of describing a field.
    if lookup(fields, "Funcs").is_none()
        && lookup(fields, "Callee").is_some()
        && lookup(fields, "Kind").is_some()
    {
        return Err(refuse(format!(
            "{at} is an Edge record, not a Package: this is `-whylive` output. Run deadcode \
             without `-whylive`: it reuses the top-level array for a path from a root to one \
             named function, so tolerating it would report zero dead functions for a run that \
             never looked for any"
        )));
    }
    let funcs = array(field(fields, "Funcs", at)?, &format!("{at}.Funcs"))?;
    Ok(DeadPackage {
        name: text(field(fields, "Name", at)?, &format!("{at}.Name"))?.to_string(),
        path: text(field(fields, "Path", at)?, &format!("{at}.Path"))?.to_string(),
        funcs: funcs
            .iter()
            .enumerate()
            .map(|(index, item)| function(item, &format!("{at}.Funcs[{index}]")))
            .collect::<Result<Vec<_>>>()?,
    })
}

/// One element of a package's `Funcs`, or the reason it is not a `Function`.
fn function(value: &json::Json, at: &str) -> Result<DeadFunction> {
    let fields = object(value, at)?;
    let position = object(field(fields, "Position", at)?, &format!("{at}.Position"))?;
    Ok(DeadFunction {
        name: text(field(fields, "Name", at)?, &format!("{at}.Name"))?.to_string(),
        file: PathBuf::from(text(
            field(position, "File", at)?,
            &format!("{at}.Position.File"),
        )?),
        line: count(field(position, "Line", at)?, &format!("{at}.Position.Line"))?,
        column: count(field(position, "Col", at)?, &format!("{at}.Position.Col"))?,
        generated: flag(field(fields, "Generated", at)?, &format!("{at}.Generated"))?,
        marker: flag(field(fields, "Marker", at)?, &format!("{at}.Marker"))?,
    })
}

/// An object's fields, in the order the tool printed them.
///
/// Unknown members are accepted rather than rejected. A future release that adds
/// a field to `Function` must still produce a verdict: refusing one would turn a
/// dependency upgrade into a hard failure, and this is the direction of §6.20's
/// rule that only leaves a *loud* result.
fn object<'a>(value: &'a json::Json, at: &str) -> Result<&'a [(String, json::Json)]> {
    match value {
        json::Json::Object(fields) => Ok(fields),
        other => Err(refuse(format!(
            "{at} is {} where an object was expected",
            kind_of(other)
        ))),
    }
}

fn array<'a>(value: &'a json::Json, at: &str) -> Result<&'a [json::Json]> {
    match value {
        json::Json::Array(items) => Ok(items),
        other => Err(refuse(format!(
            "{at} is {} where an array was expected",
            kind_of(other)
        ))),
    }
}

fn lookup<'a>(fields: &'a [(String, json::Json)], key: &str) -> Option<&'a json::Json> {
    fields
        .iter()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value)
}

/// A required member. Missing is an error and never a default: `Generated`
/// defaulting to `false` would silently answer a question the stream did not.
fn field<'a>(fields: &'a [(String, json::Json)], key: &str, at: &str) -> Result<&'a json::Json> {
    lookup(fields, key).ok_or_else(|| {
        refuse(format!(
            "{at} has no `{key}` member, which every {} record in deadcode's output protocol \
             carries. A missing member is never defaulted here: a defaulted field answers a \
             question the stream did not",
            if key == "Funcs" {
                "Package"
            } else {
                "Function"
            }
        ))
    })
}

fn text<'a>(value: &'a json::Json, at: &str) -> Result<&'a str> {
    match value {
        json::Json::String(string) => Ok(string),
        other => Err(refuse(format!(
            "{at} is {} where a string was expected",
            kind_of(other)
        ))),
    }
}

fn flag(value: &json::Json, at: &str) -> Result<bool> {
    match value {
        json::Json::Bool(flag) => Ok(*flag),
        other => Err(refuse(format!(
            "{at} is {} where a boolean was expected",
            kind_of(other)
        ))),
    }
}

/// A `Line` or a `Col`.
///
/// Digits only. `token.Position` holds a Go `int` that the tool never negates
/// and never fractions, so `-18`, `18.5` and `1e3` are not values it can print
/// and a stream containing one is not its output. This is a *format* check, the
/// one place a number is inspected at all, and nothing downstream compares
/// either field against anything.
fn count(value: &json::Json, at: &str) -> Result<u32> {
    let lexeme = match value {
        json::Json::Number(lexeme) => lexeme,
        other => Err(refuse(format!(
            "{at} is {} where a number was expected",
            kind_of(other)
        )))?,
    };
    if !lexeme.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(refuse(format!(
            "{at} is `{lexeme}`, which deadcode cannot print: Position.Line and Position.Col are \
             a Go int rendered by encoding/json, so they are always a plain run of digits"
        )));
    }
    lexeme.parse().map_err(|_| {
        refuse(format!(
            "{at} is `{lexeme}`, which does not fit in a u32 and so is not a source position"
        ))
    })
}

fn kind_of(value: &json::Json) -> &'static str {
    match value {
        json::Json::Null => "null",
        json::Json::Bool(_) => "a boolean",
        json::Json::Number(_) => "a number",
        json::Json::String(_) => "a string",
        json::Json::Array(_) => "an array",
        json::Json::Object(_) => "an object",
    }
}

/// A short, character-aligned window around a byte offset, for an error message.
fn excerpt(text: &str, offset: usize) -> &str {
    let start = (offset.saturating_sub(20)..=offset.min(text.len()))
        .find(|index| text.is_char_boundary(*index))
        .unwrap_or(0);
    let end = (start..=(start + 60).min(text.len()))
        .rev()
        .find(|index| text.is_char_boundary(*index))
        .unwrap_or(text.len());
    &text[start..end]
}

/// A strict reader for the JSON grammar `encoding/json` emits.
///
/// Deliberately minimal and deliberately total: it accepts RFC 8259 values,
/// rejects everything else with a byte offset, and has no configuration. The
/// only liberty it takes with the specification is a nesting limit, which exists
/// because this parses the output of an external process inside the harness and
/// a stack overflow aborts the process rather than returning an error.
mod json {
    /// Deeper than any deadcode document (`Package` → `Funcs` → `Function` →
    /// `Position` → scalar is four), shallow enough that the recursive descent
    /// below cannot exhaust the stack.
    const MAX_DEPTH: usize = 32;

    #[derive(Debug, Clone, PartialEq)]
    pub enum Json {
        Null,
        Bool(bool),
        /// The number's lexeme, unconverted. deadcode's only numbers are source
        /// positions; converting through `f64` here would silently round one.
        Number(String),
        String(String),
        Array(Vec<Json>),
        Object(Vec<(String, Json)>),
    }

    #[derive(Debug)]
    pub struct ParseError {
        pub offset: usize,
        pub reason: String,
    }

    /// Parse one complete JSON document. Trailing non-whitespace is an error.
    pub fn parse(text: &str) -> Result<Json, ParseError> {
        let mut parser = Parser {
            bytes: text.as_bytes(),
            pos: 0,
        };
        parser.skip_whitespace();
        let value = parser.value(0)?;
        parser.skip_whitespace();
        if parser.pos != parser.bytes.len() {
            return Err(parser.error("trailing content after the top-level value"));
        }
        Ok(value)
    }

    struct Parser<'a> {
        bytes: &'a [u8],
        pos: usize,
    }

    impl Parser<'_> {
        fn error(&self, reason: impl Into<String>) -> ParseError {
            ParseError {
                offset: self.pos,
                reason: reason.into(),
            }
        }

        fn peek(&self) -> Option<u8> {
            self.bytes.get(self.pos).copied()
        }

        fn skip_whitespace(&mut self) {
            while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
                self.pos += 1;
            }
        }

        fn expect(&mut self, byte: u8) -> Result<(), ParseError> {
            if self.peek() == Some(byte) {
                self.pos += 1;
                Ok(())
            } else {
                Err(self.error(format!("expected `{}`", byte as char)))
            }
        }

        fn literal(&mut self, word: &str, value: Json) -> Result<Json, ParseError> {
            if self.bytes[self.pos..].starts_with(word.as_bytes()) {
                self.pos += word.len();
                Ok(value)
            } else {
                Err(self.error(format!("expected `{word}`")))
            }
        }

        fn value(&mut self, depth: usize) -> Result<Json, ParseError> {
            if depth > MAX_DEPTH {
                return Err(self.error(format!(
                    "nested more than {MAX_DEPTH} deep; deadcode's own documents are four"
                )));
            }
            match self.peek() {
                Some(b'n') => self.literal("null", Json::Null),
                Some(b't') => self.literal("true", Json::Bool(true)),
                Some(b'f') => self.literal("false", Json::Bool(false)),
                Some(b'"') => self.string().map(Json::String),
                Some(b'[') => self.array(depth),
                Some(b'{') => self.object(depth),
                Some(b'-' | b'0'..=b'9') => self.number(),
                Some(other) => Err(self.error(format!("unexpected byte `{}`", other as char))),
                None => Err(self.error("unexpected end of input")),
            }
        }

        fn array(&mut self, depth: usize) -> Result<Json, ParseError> {
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
                    _ => return Err(self.error("expected `,` or `]` in array")),
                }
            }
        }

        fn object(&mut self, depth: usize) -> Result<Json, ParseError> {
            self.expect(b'{')?;
            let mut fields: Vec<(String, Json)> = Vec::new();
            self.skip_whitespace();
            if self.peek() == Some(b'}') {
                self.pos += 1;
                return Ok(Json::Object(fields));
            }
            loop {
                self.skip_whitespace();
                let key = self.string()?;
                if fields.iter().any(|(name, _)| *name == key) {
                    // encoding/json cannot emit one, and last-wins would let a
                    // crafted stream overwrite a field silently.
                    return Err(self.error(format!("duplicate member `{key}`")));
                }
                self.skip_whitespace();
                self.expect(b':')?;
                self.skip_whitespace();
                let value = self.value(depth + 1)?;
                fields.push((key, value));
                self.skip_whitespace();
                match self.peek() {
                    Some(b',') => self.pos += 1,
                    Some(b'}') => {
                        self.pos += 1;
                        return Ok(Json::Object(fields));
                    }
                    _ => return Err(self.error("expected `,` or `}` in object")),
                }
            }
        }

        /// A JSON number's lexeme, validated against the grammar and returned
        /// unconverted.
        fn number(&mut self) -> Result<Json, ParseError> {
            let start = self.pos;
            if self.peek() == Some(b'-') {
                self.pos += 1;
            }
            let integer = self.digits();
            if integer == 0 {
                return Err(self.error("expected a digit"));
            }
            if self.peek() == Some(b'.') {
                self.pos += 1;
                if self.digits() == 0 {
                    return Err(self.error("expected a digit after `.`"));
                }
            }
            if matches!(self.peek(), Some(b'e' | b'E')) {
                self.pos += 1;
                if matches!(self.peek(), Some(b'+' | b'-')) {
                    self.pos += 1;
                }
                if self.digits() == 0 {
                    return Err(self.error("expected a digit in the exponent"));
                }
            }
            // `String::from_utf8` cannot fail: every byte consumed above is ASCII.
            Ok(Json::Number(
                String::from_utf8(self.bytes[start..self.pos].to_vec())
                    .map_err(|_| self.error("number is not ASCII"))?,
            ))
        }

        fn digits(&mut self) -> usize {
            let start = self.pos;
            while matches!(self.peek(), Some(byte) if byte.is_ascii_digit()) {
                self.pos += 1;
            }
            self.pos - start
        }

        /// A JSON string, unescaped.
        ///
        /// The escapes that matter in practice are `\u0026`, `\u003c` and
        /// `\u003e`: Go's encoder is HTML-safe by default and rewrites `&`, `<`
        /// and `>` that way, which a capture really contains whenever a Go file
        /// name holds one. Surrogate pairs are handled although `encoding/json`
        /// emits raw UTF-8 and never produces one, because decoding a pair
        /// wrongly is a wrong answer rather than a loud one.
        fn string(&mut self) -> Result<String, ParseError> {
            self.expect(b'"')?;
            let mut out: Vec<u8> = Vec::new();
            loop {
                let byte = self
                    .peek()
                    .ok_or_else(|| self.error("unterminated string"))?;
                match byte {
                    b'"' => {
                        self.pos += 1;
                        return String::from_utf8(out)
                            .map_err(|_| self.error("string is not valid UTF-8"));
                    }
                    b'\\' => {
                        self.pos += 1;
                        let escape = self
                            .peek()
                            .ok_or_else(|| self.error("unterminated escape"))?;
                        self.pos += 1;
                        match escape {
                            b'"' => out.push(b'"'),
                            b'\\' => out.push(b'\\'),
                            b'/' => out.push(b'/'),
                            b'b' => out.push(0x08),
                            b'f' => out.push(0x0c),
                            b'n' => out.push(b'\n'),
                            b'r' => out.push(b'\r'),
                            b't' => out.push(b'\t'),
                            b'u' => {
                                let code = self.unicode_escape()?;
                                let mut buffer = [0u8; 4];
                                out.extend_from_slice(code.encode_utf8(&mut buffer).as_bytes());
                            }
                            other => {
                                return Err(
                                    self.error(format!("unknown escape `\\{}`", other as char))
                                )
                            }
                        }
                    }
                    control if control < 0x20 => {
                        return Err(self.error("unescaped control character in string"))
                    }
                    _ => {
                        // Raw UTF-8 is copied through byte by byte, so a
                        // multi-byte character is never split.
                        out.push(byte);
                        self.pos += 1;
                    }
                }
            }
        }

        /// The four hex digits after `\u`, plus the low half of a surrogate pair.
        fn unicode_escape(&mut self) -> Result<char, ParseError> {
            let first = self.hex4()?;
            if !(0xD800..0xDC00).contains(&first) {
                return char::from_u32(first)
                    .ok_or_else(|| self.error("escape is an unpaired low surrogate"));
            }
            if self.peek() != Some(b'\\') {
                return Err(self.error("high surrogate is not followed by an escape"));
            }
            self.pos += 1;
            self.expect(b'u')?;
            let second = self.hex4()?;
            if !(0xDC00..0xE000).contains(&second) {
                return Err(self.error("high surrogate is not followed by a low surrogate"));
            }
            let code = 0x1_0000 + ((first - 0xD800) << 10) + (second - 0xDC00);
            char::from_u32(code).ok_or_else(|| self.error("surrogate pair is not a character"))
        }

        fn hex4(&mut self) -> Result<u32, ParseError> {
            let end = self.pos + 4;
            if end > self.bytes.len() {
                return Err(self.error("truncated `\\u` escape"));
            }
            let mut code = 0u32;
            for byte in &self.bytes[self.pos..end] {
                let digit = (*byte as char)
                    .to_digit(16)
                    .ok_or_else(|| self.error("`\\u` escape is not four hex digits"))?;
                code = code * 16 + digit;
            }
            self.pos = end;
            Ok(code)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Captured verbatim from `deadcode -json ./...` run inside a materialized
    /// m12 (`judged_mutants::fixtures::m12_linkname_alias::LinknameAlias`),
    /// x/tools v0.48.0, go1.26.2 darwin/arm64, 2026-08-01. Exit code 0.
    ///
    /// `json.MarshalIndent(objects, "", "\t")`, so the indentation below is
    /// tabs, and there is no trailing newline.
    const CAPTURED_M12: &str = r#"[
	{
		"Name": "main",
		"Path": "example.com/m12/telemetry/cmd/libtelemetry",
		"Funcs": [
			{
				"Name": "TelemetryFlush",
				"Position": {
					"File": "cmd/libtelemetry/abi.go",
					"Line": 18,
					"Col": 6
				},
				"Generated": false,
				"Marker": false
			},
			{
				"Name": "runtime_throw",
				"Position": {
					"File": "cmd/libtelemetry/abi.go",
					"Line": 27,
					"Col": 0
				},
				"Generated": false,
				"Marker": false
			},
			{
				"Name": "_cgo_cmalloc",
				"Position": {
					"File": "cmd/libtelemetry/abi.go",
					"Line": 30,
					"Col": 0
				},
				"Generated": false,
				"Marker": false
			}
		]
	},
	{
		"Name": "collector",
		"Path": "example.com/m12/telemetry/internal/collector",
		"Funcs": [
			{
				"Name": "unusedPercentile",
				"Position": {
					"File": "internal/collector/unused_percentile.go",
					"Line": 5,
					"Col": 6
				},
				"Generated": false,
				"Marker": false
			}
		]
	},
	{
		"Name": "sampler",
		"Path": "example.com/m12/telemetry/internal/sampler",
		"Funcs": [
			{
				"Name": "drain",
				"Position": {
					"File": "internal/sampler/drain.go",
					"Line": 9,
					"Col": 6
				},
				"Generated": false,
				"Marker": false
			},
			{
				"Name": "legacyHistogram",
				"Position": {
					"File": "internal/sampler/legacy_histogram.go",
					"Line": 6,
					"Col": 6
				},
				"Generated": false,
				"Marker": false
			}
		]
	}
]"#;

    /// Captured verbatim: `CGO_ENABLED=0 deadcode -json ./...` in the same m12.
    /// Exit code 0, nothing on stderr — and `cmd/libtelemetry` is simply gone,
    /// with `TelemetryFlush` gone with it. §6.4, measured.
    const CAPTURED_M12_CGO_DISABLED: &str = r#"[
	{
		"Name": "collector",
		"Path": "example.com/m12/telemetry/internal/collector",
		"Funcs": [
			{
				"Name": "unusedPercentile",
				"Position": {
					"File": "internal/collector/unused_percentile.go",
					"Line": 5,
					"Col": 6
				},
				"Generated": false,
				"Marker": false
			}
		]
	},
	{
		"Name": "sampler",
		"Path": "example.com/m12/telemetry/internal/sampler",
		"Funcs": [
			{
				"Name": "drain",
				"Position": {
					"File": "internal/sampler/drain.go",
					"Line": 9,
					"Col": 6
				},
				"Generated": false,
				"Marker": false
			},
			{
				"Name": "legacyHistogram",
				"Position": {
					"File": "internal/sampler/legacy_histogram.go",
					"Line": 6,
					"Col": 6
				},
				"Generated": false,
				"Marker": false
			}
		]
	}
]"#;

    /// Captured verbatim: `deadcode -json -generated ./...` against a probe
    /// module holding a value-receiver method, a pointer-receiver method, a
    /// marker interface method (`Row.tag`) and a generated file.
    const CAPTURED_SHAPES_GENERATED: &str = r#"[
	{
		"Name": "main",
		"Path": "example.com/shapes",
		"Funcs": [
			{
				"Name": "Ledger.Add",
				"Position": {
					"File": "main.go",
					"Line": 8,
					"Col": 18
				},
				"Generated": false,
				"Marker": false
			},
			{
				"Name": "Ledger.deadMethod",
				"Position": {
					"File": "main.go",
					"Line": 9,
					"Col": 17
				},
				"Generated": false,
				"Marker": false
			},
			{
				"Name": "deadWithClosure",
				"Position": {
					"File": "main.go",
					"Line": 17,
					"Col": 6
				},
				"Generated": false,
				"Marker": false
			}
		]
	},
	{
		"Name": "gen",
		"Path": "example.com/shapes/gen",
		"Funcs": [
			{
				"Name": "GeneratedDead",
				"Position": {
					"File": "gen/gen.go",
					"Line": 5,
					"Col": 6
				},
				"Generated": true,
				"Marker": false
			}
		]
	}
]"#;

    /// Captured verbatim: a package whose file name contains `&`, `<` and `>`
    /// and whose functions have non-ASCII names. This is what Go's HTML-safe
    /// encoder does, and the reason the reader has a `\u` escape at all.
    const CAPTURED_ESCAPES: &str = r#"[
	{
		"Name": "pkg",
		"Path": "example.com/esc/pkg",
		"Funcs": [
			{
				"Name": "déjàVu",
				"Position": {
					"File": "pkg/a\u0026b\u003cc\u003e.go",
					"Line": 3,
					"Col": 6
				},
				"Generated": false,
				"Marker": false
			},
			{
				"Name": "Ünused",
				"Position": {
					"File": "pkg/a\u0026b\u003cc\u003e.go",
					"Line": 5,
					"Col": 6
				},
				"Generated": false,
				"Marker": false
			}
		]
	}
]"#;

    /// Captured verbatim: `deadcode -json -filter='zzz-no-such-package' ./...`
    /// in m12. Exit code 0, stdout is four bytes and no newline — the same four
    /// bytes a run that found nothing prints.
    const CAPTURED_NULL: &str = "null";

    /// Captured verbatim: `deadcode ./...` in m12 — the default format, which
    /// is what a caller who forgot `-json` gets. Exit code 0.
    const CAPTURED_DEFAULT_TEXT: &str = "\
cmd/libtelemetry/abi.go:18:6: unreachable func: TelemetryFlush
cmd/libtelemetry/abi.go:27:0: unreachable func: runtime_throw
cmd/libtelemetry/abi.go:30:0: unreachable func: _cgo_cmalloc
internal/collector/unused_percentile.go:5:6: unreachable func: unusedPercentile
internal/sampler/drain.go:9:6: unreachable func: drain
internal/sampler/legacy_histogram.go:6:6: unreachable func: legacyHistogram
";

    /// Captured verbatim:
    /// `deadcode -json -whylive='example.com/m12/telemetry/internal/sampler.Record' ./...`
    /// Exit code 0. A different record type in the same top-level array.
    const CAPTURED_WHYLIVE: &str = r#"[
	{
		"Initial": "example.com/m12/telemetry/cmd/telemetryd.main",
		"Kind": "static",
		"Position": {
			"File": "cmd/telemetryd/main.go",
			"Line": 15,
			"Col": 16
		},
		"Callee": "example.com/m12/telemetry/internal/sampler.Record"
	}
]"#;

    fn dead(name: &str, file: &str, line: u32, column: u32) -> DeadFunction {
        DeadFunction {
            name: name.to_string(),
            file: PathBuf::from(file),
            line,
            column,
            generated: false,
            marker: false,
        }
    }

    fn m12_packages() -> Vec<DeadPackage> {
        vec![
            DeadPackage {
                name: "main".to_string(),
                path: "example.com/m12/telemetry/cmd/libtelemetry".to_string(),
                funcs: vec![
                    dead("TelemetryFlush", "cmd/libtelemetry/abi.go", 18, 6),
                    dead("runtime_throw", "cmd/libtelemetry/abi.go", 27, 0),
                    dead("_cgo_cmalloc", "cmd/libtelemetry/abi.go", 30, 0),
                ],
            },
            DeadPackage {
                name: "collector".to_string(),
                path: "example.com/m12/telemetry/internal/collector".to_string(),
                funcs: vec![dead(
                    "unusedPercentile",
                    "internal/collector/unused_percentile.go",
                    5,
                    6,
                )],
            },
            DeadPackage {
                name: "sampler".to_string(),
                path: "example.com/m12/telemetry/internal/sampler".to_string(),
                funcs: vec![
                    dead("drain", "internal/sampler/drain.go", 9, 6),
                    dead(
                        "legacyHistogram",
                        "internal/sampler/legacy_histogram.go",
                        6,
                        6,
                    ),
                ],
            },
        ]
    }

    #[test]
    fn parses_the_captured_m12_run() {
        assert_eq!(parse_packages(CAPTURED_M12).unwrap(), m12_packages());
    }

    /// §4.1's prediction, as an assertion: `//go:linkname` aliasing makes a live
    /// function "spuriously reported as dead". `drain` is bound only by the
    /// linkname directive; `TelemetryFlush` only by a cgo `//export`. Both are
    /// live. Both are here.
    #[test]
    fn m12_confirms_the_linkname_and_cgo_predictions() {
        let verdict = verdict_from_stdout(CAPTURED_M12).unwrap();
        for live in ["drain", "TelemetryFlush"] {
            assert!(
                verdict
                    .claimed_dead_symbols
                    .iter()
                    .any(|s| s.name() == live),
                "deadcode must be graded as claiming the live symbol {live}; got {:?}",
                verdict.claimed_dead_symbols
            );
        }
    }

    /// The other half of the same run: both genuinely-dead decoys are named, so
    /// the false removals above are not the score of a tool that accuses
    /// everything. Per [`MAPPING_DECISION`] these land in the symbol list only.
    #[test]
    fn m12_also_names_both_genuinely_dead_decoys() {
        let verdict = verdict_from_stdout(CAPTURED_M12).unwrap();
        for decoy in ["legacyHistogram", "unusedPercentile"] {
            assert!(
                verdict
                    .claimed_dead_symbols
                    .iter()
                    .any(|s| s.name() == decoy),
                "the decoy {decoy} must be claimed; got {:?}",
                verdict.claimed_dead_symbols
            );
        }
    }

    /// §6.4, measured: the same repository, the same command, one environment
    /// variable different, and a whole main package leaves the report without a
    /// word. `TelemetryFlush` stops being accused — not because it became live,
    /// but because it stopped being looked at.
    #[test]
    fn one_build_configuration_silently_changes_the_answer() {
        let full = verdict_from_stdout(CAPTURED_M12).unwrap();
        let without_cgo = verdict_from_stdout(CAPTURED_M12_CGO_DISABLED).unwrap();

        assert!(full
            .claimed_dead_symbols
            .iter()
            .any(|s| s.name() == "TelemetryFlush"));
        assert!(
            !without_cgo
                .claimed_dead_symbols
                .iter()
                .any(|s| s.name() == "TelemetryFlush"),
            "under CGO_ENABLED=0 the package holding it is not analyzed at all"
        );
        assert!(
            without_cgo
                .claimed_dead_symbols
                .iter()
                .any(|s| s.name() == "drain"),
            "and the rest of the analysis proceeds exactly as before, which is what makes the \
             difference invisible"
        );
    }

    #[test]
    fn claimed_dead_paths_is_always_empty() {
        for stdout in [
            CAPTURED_M12,
            CAPTURED_M12_CGO_DISABLED,
            CAPTURED_SHAPES_GENERATED,
            CAPTURED_ESCAPES,
            CAPTURED_NULL,
        ] {
            assert!(
                verdict_from_stdout(stdout)
                    .unwrap()
                    .claimed_dead_paths
                    .is_empty(),
                "MAPPING_DECISION forbids a path claim; deadcode never makes one"
            );
        }
    }

    /// Nothing is dropped on the way to the verdict: the two cgo pseudo-functions
    /// are claims deadcode made, however synthetic they look, and an adapter that
    /// tidied them away would be measuring the adapter.
    #[test]
    fn cgo_pseudo_functions_are_carried_not_cleaned() {
        let verdict = verdict_from_stdout(CAPTURED_M12).unwrap();
        for pseudo in ["runtime_throw", "_cgo_cmalloc"] {
            assert!(
                verdict
                    .claimed_dead_symbols
                    .iter()
                    .any(|s| s.name() == pseudo),
                "{pseudo} is in deadcode's output and must survive into the verdict"
            );
        }
    }

    /// deadcode's own schema says `Line, Col int // line and byte index, both
    /// 1-based`. The captured m12 run contains `Col: 0` twice, and `Line: 27`
    /// and `Line: 30` for a file that is 22 lines long.
    #[test]
    fn a_zero_column_parses_because_the_tool_really_prints_one() {
        let packages = parse_packages(CAPTURED_M12).unwrap();
        let cgo = &packages[0].funcs[1];
        assert_eq!(cgo.name, "runtime_throw");
        assert_eq!((cgo.line, cgo.column), (27, 0));
    }

    #[test]
    fn parses_receiver_qualified_method_names_and_the_generated_flag() {
        let packages = parse_packages(CAPTURED_SHAPES_GENERATED).unwrap();
        assert_eq!(
            packages[0]
                .funcs
                .iter()
                .map(|f| f.name.as_str())
                .collect::<Vec<_>>(),
            ["Ledger.Add", "Ledger.deadMethod", "deadWithClosure"]
        );
        let generated = &packages[1].funcs[0];
        assert_eq!(generated.name, "GeneratedDead");
        assert!(generated.generated, "-generated sets Generated: true");
    }

    /// `Row.tag` is a marker interface method, and it is absent from the capture
    /// above: v0.48.0 drops markers before building the record, so `Marker` is
    /// always false in a package listing. Parsed and carried anyway, because a
    /// later release could start reporting them and an adapter that assumed the
    /// field away would silently shrink the verdict.
    #[test]
    fn marker_is_carried_even_though_the_tool_never_sets_it() {
        let packages = parse_packages(CAPTURED_SHAPES_GENERATED).unwrap();
        assert!(packages.iter().flat_map(|p| &p.funcs).all(|f| !f.marker));
        assert!(
            !packages
                .iter()
                .flat_map(|p| &p.funcs)
                .any(|f| f.name == "Row.tag"),
            "marker methods never appear in a package listing"
        );
    }

    #[test]
    fn unescapes_the_html_escapes_go_emits_and_keeps_utf8_intact() {
        let packages = parse_packages(CAPTURED_ESCAPES).unwrap();
        assert_eq!(
            packages[0].funcs,
            vec![
                dead("déjàVu", "pkg/a&b<c>.go", 3, 6),
                dead("Ünused", "pkg/a&b<c>.go", 5, 6),
            ]
        );
    }

    /// The literal four bytes `null` are what an empty result looks like, and
    /// they are also what a `-filter` that matched no package looks like.
    #[test]
    fn the_literal_null_is_an_empty_result_not_an_error() {
        assert_eq!(parse_packages(CAPTURED_NULL).unwrap(), Vec::new());
        assert_eq!(
            verdict_from_stdout(CAPTURED_NULL).unwrap(),
            SutVerdict::default()
        );
    }

    /// v0.48.0 marshals a nil slice, so it cannot print `[]` — but a release
    /// that switched to an empty non-nil slice would mean exactly the same
    /// thing, and refusing it would turn a tool upgrade into a hard failure.
    #[test]
    fn an_empty_array_means_the_same_as_null() {
        assert_eq!(parse_packages("[]").unwrap(), Vec::new());
    }

    /// The §6.20 self-failure this adapter can detect: pointed at a library,
    /// deadcode writes `deadcode: no main packages` to stderr, exits 1, and
    /// leaves stdout empty. Empty stdout must never become an empty verdict.
    #[test]
    fn empty_stdout_is_an_error_that_names_the_no_main_packages_failure() {
        for stdout in ["", "\n", "   \n\t"] {
            let error = parse_packages(stdout).unwrap_err().to_string();
            assert!(
                error.contains("no main packages"),
                "empty stdout must name the library-with-no-roots failure; got {error}"
            );
            assert!(
                error.contains("deadcode"),
                "the error must name the tool; got {error}"
            );
        }
    }

    #[test]
    fn the_default_text_format_is_rejected_with_the_missing_flag_named() {
        let error = parse_packages(CAPTURED_DEFAULT_TEXT)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("-json"),
            "a caller who forgot -json must be told which flag is missing; got {error}"
        );
    }

    /// `-whylive` reuses the top-level array for a different record type. It
    /// parses as JSON and contains no `Funcs`, so a tolerant parser would report
    /// zero dead functions for a run that never looked for any.
    #[test]
    fn whylive_output_is_rejected_by_name() {
        let error = parse_packages(CAPTURED_WHYLIVE).unwrap_err().to_string();
        assert!(
            error.contains("-whylive"),
            "the edge-record shape must be recognised, not read as zero findings; got {error}"
        );
    }

    #[test]
    fn truncated_json_is_an_error() {
        let truncated = &CAPTURED_M12[..CAPTURED_M12.len() / 2];
        assert!(parse_packages(truncated).is_err());
    }

    #[test]
    fn trailing_content_after_the_array_is_an_error() {
        let extra = format!("{CAPTURED_M12}\nbroker: connection reset");
        assert!(parse_packages(&extra).is_err());
    }

    #[test]
    fn a_function_missing_a_field_is_an_error_not_a_default() {
        let missing = CAPTURED_M12.replace(
            "\"Generated\": false,\n\t\t\t\t\"Marker\": false",
            "\"Marker\": false",
        );
        assert_ne!(missing, CAPTURED_M12, "the replacement must actually apply");
        let error = parse_packages(&missing).unwrap_err().to_string();
        assert!(error.contains("Generated"), "got {error}");
    }

    #[test]
    fn a_wrongly_typed_field_is_an_error() {
        let wrong = CAPTURED_M12.replace("\"Line\": 18", "\"Line\": \"18\"");
        assert_ne!(wrong, CAPTURED_M12, "the replacement must actually apply");
        assert!(parse_packages(&wrong).is_err());
    }

    #[test]
    fn a_fractional_or_negative_line_is_not_something_this_tool_can_print() {
        for bad in ["\"Line\": 18.5", "\"Line\": -18", "\"Line\": 1e3"] {
            let text = CAPTURED_M12.replace("\"Line\": 18", bad);
            assert_ne!(text, CAPTURED_M12, "the replacement must actually apply");
            assert!(parse_packages(&text).is_err(), "{bad} must be refused");
        }
    }

    #[test]
    fn deeply_nested_input_is_refused_rather_than_overflowing_the_stack() {
        let bomb = "[".repeat(50_000);
        assert!(parse_packages(&bomb).is_err());
    }

    /// `Position.File`, carried into the claim.
    ///
    /// Without it Gate 2a has no file to exclude from the corpus, so it finds
    /// every symbol in its own declaration and rescues every claim — a veto that
    /// fires unconditionally, which measures nothing. See
    /// [`crate::sut::SymbolClaim`].
    #[test]
    fn a_claim_carries_the_position_deadcode_reported_it_at() {
        let verdict = verdict_from_stdout(CAPTURED_M12).unwrap();
        let drain = verdict
            .claimed_dead_symbols
            .iter()
            .find(|claim| claim.name() == "drain")
            .expect("m12 claims `drain`");
        assert_eq!(
            drain.declaration_site().and_then(Path::to_str),
            Some("internal/sampler/drain.go"),
            "`Position.File` exactly as captured, uncorrected: re-rooting is \
             `CommandSut`'s job, being the only thing that knows where the \
             repository is"
        );
        assert!(
            verdict
                .claimed_dead_symbols
                .iter()
                .all(|claim| claim.declaration_site().is_some()),
            "every deadcode finding has a Position, so no claim from this tool \
             should ever reach Gate 2a unattributed: {:?}",
            verdict.claimed_dead_symbols
        );
    }

    #[test]
    fn symbols_are_sorted_and_deduplicated() {
        // Two packages may each contain a method spelled `Ledger.Add`; grading
        // asks whether a live name was claimed, not how many times.
        let verdict = verdict_from_stdout(CAPTURED_M12).unwrap();
        let mut sorted = verdict.claimed_dead_symbols.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(verdict.claimed_dead_symbols, sorted);
    }

    #[test]
    fn files_with_dead_functions_is_the_blast_radius_and_not_a_claim() {
        assert_eq!(
            files_with_dead_functions(&m12_packages()),
            vec![
                PathBuf::from("cmd/libtelemetry/abi.go"),
                PathBuf::from("internal/collector/unused_percentile.go"),
                PathBuf::from("internal/sampler/drain.go"),
                PathBuf::from("internal/sampler/legacy_histogram.go"),
            ]
        );
    }

    #[test]
    fn the_envelope_names_the_configuration_and_the_rootless_library() {
        for phrase in ["GOOS", "main", "linkname", "cgo"] {
            assert!(
                CAPABILITY_ENVELOPE.contains(phrase),
                "the envelope must mention {phrase}"
            );
        }
        assert!(cannot_emit().len() >= 4);
    }

    #[test]
    fn the_mapping_decision_states_the_path_refusal_and_its_cost() {
        assert!(MAPPING_DECISION.contains("claimed_dead_paths"));
        assert!(MAPPING_DECISION.contains("decoy"));
    }
}
