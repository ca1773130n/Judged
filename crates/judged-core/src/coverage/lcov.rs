//! The lcov `.info` tracefile, read as observed execution.
//!
//! §2.1 calls `FNDA:0,<name>` the single most valuable cross-language primitive
//! in the survey — *"this named function was never called"*, spelled the same
//! way by gcov, llvm-cov, coverage.py's `lcov` report, nyc/c8, simplecov and
//! Xdebug, and mergeable across runs with `lcov -a`. One parser therefore
//! reaches several ecosystems at once, which is why it is the first X-family
//! adapter rather than JaCoCo or `go tool covdata`.
//!
//! # The grammar, and the parts deliberately ignored
//!
//! A tracefile is a flat sequence of `KEY:value` lines. A record opens with
//! `SF:<source file>` and closes with `end_of_record`; inside it:
//!
//! - `FN:<line>,<name>` — a function starts here. lcov 2.x also emits
//!   `FN:<start>,<end>,<name>`; both are read (see [`parse_fn`]).
//! - `FNDA:<count>,<name>` — that function was entered `count` times. **The
//!   load-bearing line.**
//! - `DA:<line>,<count>[,<checksum>]` — that line was executed `count` times.
//!
//! `TN`, `LF`, `LH`, `FNF`, `FNH`, `BRDA`, `BRF`, `BRH` and `VER` are summary or
//! branch data and are skipped. `LF`/`LH` in particular are *recomputed* from
//! the `DA` lines rather than believed: they are a second source of truth for
//! the one number that decides whether a file was executed, and this module
//! exists because a coverage artifact that lies is the documented failure mode
//! (§3.7).
//!
//! Unknown keys are skipped; a **known** key that will not parse is an error.
//! That split is deliberate. A tracefile carrying a key from a newer lcov is
//! ordinary and its absence costs only recall, which the positive control then
//! catches as a floor failure. A malformed `FNDA` is a claim this module cannot
//! read, and silently dropping it converts "called 4000 times" into "no record
//! of a call" — the §6.20 inversion, where the safety layer's own failure reads
//! as evidence for deletion.
//!
//! # The lcov 2.x index form is not read, and that is safe
//!
//! lcov 2.x can emit functions as `FNL:<index>,<start>,<end>` plus
//! `FNA:<index>,<count>,<name>`. Those keys are unknown here, so such an
//! artifact parses to zero functions. That is not a silent hole: zero functions
//! fails the positive control's floor, the artifact is discarded whole, and the
//! layer rescues nothing rather than rescuing wrongly. Widening the parser is a
//! recall improvement; the control is what makes its absence loud.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::{Error, Result};

/// One function in one source file: its name, where it starts, and how many
/// times it was entered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionCoverage {
    name: String,
    start_line: Option<u32>,
    calls: u64,
}

impl FunctionCoverage {
    /// The name exactly as the instrumenter spelled it.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The line its `FN:` record gave, when the artifact carried one. `None`
    /// when only an `FNDA:` was present — legal, and enough to answer the only
    /// question this module asks.
    pub fn start_line(&self) -> Option<u32> {
        self.start_line
    }

    /// Times entered, summed across every record naming it.
    pub fn calls(&self) -> u64 {
        self.calls
    }

    /// Whether this function was entered at all.
    ///
    /// The only predicate a caller may act on. There is deliberately no
    /// `was_never_called`: `calls == 0` is [`super`]'s "miss", which contributes
    /// zero toward deadness, and a named accessor for it would be an invitation
    /// to use it as one.
    pub fn was_called(&self) -> bool {
        self.calls > 0
    }
}

/// Everything one tracefile says about one source file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileCoverage {
    source: PathBuf,
    functions: Vec<FunctionCoverage>,
    lines_found: usize,
    lines_hit: usize,
}

impl FileCoverage {
    /// The path exactly as the `SF:` line spelled it, which is whatever the
    /// machine that ran the tests called it — usually absolute, and usually not
    /// a path that exists here. Matching is [`Coverage::executed_file`]'s job.
    pub fn source(&self) -> &Path {
        &self.source
    }

    /// Every function record, sorted by name so a report is diffable.
    pub fn functions(&self) -> &[FunctionCoverage] {
        &self.functions
    }

    /// Distinct lines the artifact carried a `DA:` record for.
    pub fn lines_found(&self) -> usize {
        self.lines_found
    }

    /// Of those, the ones with a non-zero count.
    pub fn lines_hit(&self) -> usize {
        self.lines_hit
    }

    /// Whether anything in this file ran: a line executed, or a function
    /// entered.
    ///
    /// Line granularity is right *here* and wrong for a symbol, and the
    /// difference is §2.3's whole point. In Python, Ruby and JavaScript the
    /// `def`, `class` and module-level lines execute at **import**, so a module
    /// that is merely imported reports covered lines while every function body
    /// reads dead. For a file claim that is still proof: something loaded this
    /// file, so it is not an orphan and the claim must be dropped. For a symbol
    /// claim it proves nothing, which is why [`Coverage::called_function`] reads
    /// `FNDA` and never touches a line.
    pub fn was_executed(&self) -> bool {
        self.lines_hit > 0 || self.functions.iter().any(FunctionCoverage::was_called)
    }

    /// The function record whose name is the same symbol as `symbol`, by
    /// [`names_same_symbol`]. The first match in name order, so the answer does
    /// not depend on the artifact's line order.
    pub fn function(&self, symbol: &str) -> Option<&FunctionCoverage> {
        self.functions
            .iter()
            .find(|function| names_same_symbol(&function.name, symbol))
    }
}

/// One parsed lcov tracefile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Coverage {
    artifact: PathBuf,
    files: Vec<FileCoverage>,
}

impl Coverage {
    /// Read and parse a tracefile from disk.
    pub fn read(artifact: &Path) -> Result<Coverage> {
        let text = std::fs::read_to_string(artifact).map_err(|source| Error::Io {
            path: artifact.to_path_buf(),
            source,
        })?;
        Coverage::parse(artifact, &text)
    }

    /// Parse a tracefile already in memory. `artifact` is used only to name the
    /// document in errors.
    pub fn parse(artifact: &Path, text: &str) -> Result<Coverage> {
        let mut records: BTreeMap<String, RecordBuilder> = BTreeMap::new();
        let mut open: Option<String> = None;

        for (index, raw) in text.lines().enumerate() {
            let number = index + 1;
            // CRLF: a tracefile produced on a Windows runner and read here would
            // otherwise carry `\r` into every function name, and a name that
            // matches nothing is a rescue that silently never happens.
            let line = raw.trim_end_matches('\r').trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            if line == "end_of_record" {
                // Not merely tidiness: a stray `end_of_record` means the
                // records after it are attributed to no file at all, and the
                // parse would quietly cover a different source tree than the
                // artifact describes.
                if open.take().is_none() {
                    return Err(malformed(
                        artifact,
                        number,
                        "end_of_record outside a record",
                    ));
                }
                continue;
            }

            let Some((key, value)) = line.split_once(':') else {
                // Not an error: tracefiles in the wild carry banners and tool
                // version lines that are not `KEY:value` at all.
                continue;
            };

            match key {
                "SF" => {
                    let source = value.trim();
                    if source.is_empty() {
                        return Err(malformed(artifact, number, "SF: with no path"));
                    }
                    // Merging by source path rather than appending a record per
                    // `SF:` is what makes `lcov -a`-style concatenated
                    // tracefiles read correctly: the same file appears once per
                    // test run, and the counts add. Appending instead would let
                    // `executed_file` find the *first* record — quite possibly
                    // the run that did not touch the file — and answer "not
                    // executed" about a file another run entered.
                    open = Some(normalize(source));
                    records
                        .entry(normalize(source))
                        .or_insert_with(|| RecordBuilder::new(PathBuf::from(source)));
                }
                "FN" | "FNDA" | "DA" => {
                    let Some(key_path) = open.as_ref() else {
                        return Err(malformed(
                            artifact,
                            number,
                            &format!("{key}: before any SF: record"),
                        ));
                    };
                    // Present by construction: `open` is only ever set to a key
                    // that was just inserted.
                    let record = records
                        .get_mut(key_path)
                        .expect("open record is always present");
                    match key {
                        "FN" => {
                            let (start, name) = parse_fn(artifact, number, value)?;
                            record.declare(name, start);
                        }
                        "FNDA" => {
                            let (calls, name) = parse_counted(artifact, number, "FNDA", value)?;
                            record.called(name, calls);
                        }
                        _ => {
                            let (line_number, count) = parse_da(artifact, number, value)?;
                            record.line(line_number, count);
                        }
                    }
                }
                // Summary and branch keys, and anything from a newer lcov. See
                // the module docs on why an unknown key is skipped and a
                // malformed known key is not.
                _ => {}
            }
        }

        if let Some(source) = open {
            return Err(malformed(
                artifact,
                text.lines().count(),
                &format!("record for {source} has no end_of_record"),
            ));
        }

        Ok(Coverage {
            artifact: artifact.to_path_buf(),
            files: records.into_values().map(RecordBuilder::build).collect(),
        })
    }

    /// The artifact this was parsed from.
    pub fn artifact(&self) -> &Path {
        &self.artifact
    }

    /// Every source file record, sorted by the `SF:` path.
    pub fn files(&self) -> &[FileCoverage] {
        &self.files
    }

    /// Function records across the whole artifact that were entered at least
    /// once — the number [`super::control`]'s floor is checked against.
    pub fn functions_called(&self) -> usize {
        self.files
            .iter()
            .flat_map(FileCoverage::functions)
            .filter(|function| function.was_called())
            .count()
    }

    /// Function records across the whole artifact, called or not.
    pub fn functions_total(&self) -> usize {
        self.files.iter().map(|file| file.functions.len()).sum()
    }

    /// The record for `repo_relative` if the artifact says it was executed.
    ///
    /// # Matching a path that was recorded on another machine
    ///
    /// An `SF:` path is whatever the CI runner called the file —
    /// `/home/runner/work/repo/src/queue.py` — and a claim is repo-relative,
    /// `src/queue.py`. Comparing those raw yields no match at all, which
    /// presents as a rescue layer that never fires: the silent-disabling shape
    /// this codebase normalizes against everywhere it compares a path.
    ///
    /// So the rule is suffix matching at a **component boundary**: the record
    /// matches when its path equals the claim or ends with `/` + the claim.
    /// `src/queue.py` therefore matches `/ci/build/src/queue.py`, and does not
    /// match `/ci/build/other_src/queue.py`.
    ///
    /// It is deliberately wide in one direction: a vendored
    /// `vendor/dep/src/queue.py` also ends with `src/queue.py` and would match.
    /// That produces an extra *rescue*, never an extra accusation, which is the
    /// only direction a layer that may only drop claims is allowed to be wrong
    /// in. Anchoring at the repository root instead would need the artifact to
    /// declare where its root was, and nothing in the format carries that.
    ///
    /// The **shortest** matching path wins, so a record for the file itself
    /// beats a vendored namesake whenever both are present. Every match ends
    /// with the same claim, so the shortest is the one with the least in front
    /// of it — `/ci/src/queue.py` over `/ci/vendor/dep/src/queue.py`.
    pub fn executed_file(&self, repo_relative: &str) -> Option<&FileCoverage> {
        let claim = normalize(repo_relative);
        self.files
            .iter()
            .filter(|file| path_ends_with(&normalize_path(&file.source), &claim))
            .filter(|file| file.was_executed())
            .min_by_key(|file| file.source.as_os_str().len())
    }

    /// The file and function record proving `symbol` was **entered**, if the
    /// artifact carries one.
    ///
    /// `FNDA` only. A `DA` hit on the line an `FN:` record points at means the
    /// `def` was executed, which in every interpreted language happens at
    /// import, and reading that as a call would rescue every symbol in every
    /// module the test suite so much as imported (§2.3).
    pub fn called_function(&self, symbol: &str) -> Option<(&FileCoverage, &FunctionCoverage)> {
        self.files.iter().find_map(|file| {
            file.functions
                .iter()
                .find(|function| function.was_called() && names_same_symbol(&function.name, symbol))
                .map(|function| (file, function))
        })
    }
}

// ---------------------------------------------------------------------------
// Parsing one line
// ---------------------------------------------------------------------------

/// `FN:` in either dialect: `<line>,<name>` or lcov 2.x's `<start>,<end>,<name>`.
///
/// The two are told apart by asking whether the second field is an integer, and
/// that heuristic has a stated edge: a function genuinely named `42` in a 1.x
/// tracefile would be read as an end line, losing its start line. It keeps its
/// name and its `FNDA`, so the only thing lost is a line number this module
/// never decides anything with.
///
/// The name is everything after the last leading integer field rather than the
/// last comma-separated field, because C++ and Rust names carry commas of their
/// own — `Map<K, V>::insert`, `impl Foo for (A, B)`.
fn parse_fn(artifact: &Path, number: usize, value: &str) -> Result<(Option<u32>, String)> {
    let (first, rest) = value
        .split_once(',')
        .ok_or_else(|| malformed(artifact, number, "FN: expects <line>,<name>"))?;
    let start: u32 = first.trim().parse().map_err(|_| {
        malformed(
            artifact,
            number,
            &format!("FN: line {first:?} is not a number"),
        )
    })?;

    if let Some((second, tail)) = rest.split_once(',') {
        if second.trim().parse::<u32>().is_ok() && !tail.trim().is_empty() {
            return Ok((Some(start), tail.trim().to_string()));
        }
    }

    let name = rest.trim();
    if name.is_empty() {
        return Err(malformed(artifact, number, "FN: with no function name"));
    }
    Ok((Some(start), name.to_string()))
}

/// `FNDA:<count>,<name>`. The name is everything after the first comma, commas
/// and all.
fn parse_counted(artifact: &Path, number: usize, key: &str, value: &str) -> Result<(u64, String)> {
    let (count, name) = value
        .split_once(',')
        .ok_or_else(|| malformed(artifact, number, &format!("{key}: expects <count>,<name>")))?;
    let calls: u64 = count.trim().parse().map_err(|_| {
        malformed(
            artifact,
            number,
            &format!("{key}: count {count:?} is not a number"),
        )
    })?;
    let name = name.trim();
    if name.is_empty() {
        return Err(malformed(
            artifact,
            number,
            &format!("{key}: with no function name"),
        ));
    }
    Ok((calls, name.to_string()))
}

/// `DA:<line>,<count>[,<checksum>]`. The checksum is ignored: it exists to
/// detect a source file that changed under the artifact, and this module never
/// reads the source.
fn parse_da(artifact: &Path, number: usize, value: &str) -> Result<(u32, u64)> {
    let (line, rest) = value
        .split_once(',')
        .ok_or_else(|| malformed(artifact, number, "DA: expects <line>,<count>"))?;
    let line: u32 = line.trim().parse().map_err(|_| {
        malformed(
            artifact,
            number,
            &format!("DA: line {line:?} is not a number"),
        )
    })?;
    let count = rest.split(',').next().unwrap_or("").trim();
    // gcov writes `-` for a line whose count is unknown rather than zero.
    // Unknown is not "never executed", so it is recorded as a found line with no
    // hit and never as evidence of anything.
    let count: u64 = if count == "-" {
        0
    } else {
        count.parse().map_err(|_| {
            malformed(
                artifact,
                number,
                &format!("DA: count {count:?} is not a number"),
            )
        })?
    };
    Ok((line, count))
}

fn malformed(artifact: &Path, number: usize, message: &str) -> Error {
    Error::Coverage {
        path: artifact.to_path_buf(),
        message: format!("line {number}: {message}"),
    }
}

// ---------------------------------------------------------------------------
// Accumulating one source file's records
// ---------------------------------------------------------------------------

/// One `SF:` record under construction, merging every appearance of the same
/// source path.
struct RecordBuilder {
    source: PathBuf,
    /// name -> times entered, summed. `BTreeMap` so the built list is in name
    /// order and a report of it is diffable between runs.
    calls: BTreeMap<String, u64>,
    /// name -> the `FN:` start line, first declaration winning.
    declared: BTreeMap<String, u32>,
    /// line -> times executed, summed.
    lines: BTreeMap<u32, u64>,
}

impl RecordBuilder {
    fn new(source: PathBuf) -> RecordBuilder {
        RecordBuilder {
            source,
            calls: BTreeMap::new(),
            declared: BTreeMap::new(),
            lines: BTreeMap::new(),
        }
    }

    /// An `FN:` record. A function that is declared and never given an `FNDA`
    /// still exists, at zero calls — which is the honest reading and the whole
    /// value of `FNDA:0` as a primitive.
    fn declare(&mut self, name: String, start: Option<u32>) {
        if let Some(start) = start {
            self.declared.entry(name.clone()).or_insert(start);
        }
        self.calls.entry(name).or_insert(0);
    }

    fn called(&mut self, name: String, count: u64) {
        *self.calls.entry(name).or_insert(0) += count;
    }

    fn line(&mut self, number: u32, count: u64) {
        *self.lines.entry(number).or_insert(0) += count;
    }

    fn build(self) -> FileCoverage {
        let lines_found = self.lines.len();
        let lines_hit = self.lines.values().filter(|count| **count > 0).count();
        let declared = self.declared;
        FileCoverage {
            source: self.source,
            functions: self
                .calls
                .into_iter()
                .map(|(name, calls)| FunctionCoverage {
                    start_line: declared.get(&name).copied(),
                    name,
                    calls,
                })
                .collect(),
            lines_found,
            lines_hit,
        }
    }
}

// ---------------------------------------------------------------------------
// Matching names that two different tools spelled
// ---------------------------------------------------------------------------

/// Separators a tool may use to qualify a name. The same set
/// `judged-mutants` matches ground truth with, longest first so `::` is not
/// split as two `:`.
const SYMBOL_SEPARATORS: [&str; 4] = ["::", ".", "/", "#"];

/// Whether an instrumenter's function name and an analyzer's symbol claim name
/// the same thing.
///
/// **Symmetric**, which is the one way this differs from the trailing-segment
/// rule used against ground truth elsewhere in the workspace. There, one side is
/// bare by construction: a fixture declares `DunningConfig` and only the tool
/// qualifies it. Here neither side is under our control — coverage.py writes
/// `ledger.dunning.compute` while vulture claims `compute`, and llvm-cov writes
/// `compute` while deadcode claims `ledger/dunning.compute` — so a one-way rule
/// would miss the rescue in whichever direction it did not anticipate.
///
/// Symmetry can only ever find MORE rescues than equality, never fewer, which is
/// the direction a layer that may only drop claims is allowed to be wrong in.
///
/// Empty names never match, including each other: an instrumenter that emitted a
/// nameless record must not rescue an analyzer that claimed nothing.
pub fn names_same_symbol(left: &str, right: &str) -> bool {
    if left.is_empty() || right.is_empty() {
        return false;
    }
    left == right || ends_with_segment(left, right) || ends_with_segment(right, left)
}

/// Whether `qualified` ends with `bare` at a separator boundary.
fn ends_with_segment(qualified: &str, bare: &str) -> bool {
    SYMBOL_SEPARATORS
        .iter()
        .any(|separator| qualified.ends_with(&format!("{separator}{bare}")))
}

/// A path as the forward-slashed string both sides of a comparison are keyed on,
/// with a leading `./` dropped — `./src/a.py` and `src/a.py` are the same file
/// and tools disagree about which to write.
fn normalize(path: &str) -> String {
    normalize_path(Path::new(path))
}

fn normalize_path(path: &Path) -> String {
    let text = path
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/");
    text.strip_prefix("./").unwrap_or(&text).to_string()
}

/// Whether `recorded` ends with `claim` at a component boundary.
fn path_ends_with(recorded: &str, claim: &str) -> bool {
    recorded == claim || recorded.ends_with(&format!("/{claim}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The one line the whole family rests on, in both directions.
    #[test]
    fn fnda_zero_and_fnda_nonzero_are_different_answers() {
        let text = "SF:/ci/src/a.py\n\
                    FN:3,handle\n\
                    FNDA:0,handle\n\
                    FN:9,ignored\n\
                    FNDA:4,ignored\n\
                    DA:3,1\n\
                    DA:10,0\n\
                    end_of_record\n";
        let coverage = Coverage::parse(Path::new("lcov.info"), text).expect("parses");

        assert!(
            coverage.called_function("ignored").is_some(),
            "FNDA:4 is proof of a call"
        );
        assert!(
            coverage.called_function("handle").is_none(),
            "FNDA:0 is a miss, and a miss is never a hit"
        );
        assert_eq!(coverage.functions_called(), 1);
        assert_eq!(coverage.functions_total(), 2);
    }

    /// §2.3: `def` lines execute at import. A file with covered lines and no
    /// called functions is an imported module, which rescues the *file* and must
    /// rescue none of its symbols.
    #[test]
    fn boot_only_coverage_rescues_the_file_and_no_symbol() {
        let text = "SF:src/handlers.py\n\
                    FN:4,health\n\
                    FNDA:0,health\n\
                    DA:1,1\n\
                    DA:4,1\n\
                    DA:5,0\n\
                    end_of_record\n";
        let coverage = Coverage::parse(Path::new("lcov.info"), text).expect("parses");

        assert!(coverage.executed_file("src/handlers.py").is_some());
        assert!(
            coverage.called_function("health").is_none(),
            "the def line executing at import is not a call to the function"
        );
    }

    /// The paths in an artifact were recorded on another machine.
    #[test]
    fn an_sf_path_matches_a_repo_relative_claim_at_a_component_boundary() {
        let text = "SF:/home/runner/work/repo/src/queue.py\n\
                    DA:1,1\n\
                    end_of_record\n";
        let coverage = Coverage::parse(Path::new("lcov.info"), text).expect("parses");

        assert!(coverage.executed_file("src/queue.py").is_some());
        assert!(
            coverage.executed_file("other_src/queue.py").is_none(),
            "the boundary is a path component, not a substring"
        );
        assert!(coverage.executed_file("queue.py").is_some());
    }

    /// The exact file beats a vendored namesake when both were recorded.
    #[test]
    fn the_shortest_matching_record_wins() {
        let text = "SF:/ci/vendor/dep/src/queue.py\n\
                    DA:1,1\n\
                    end_of_record\n\
                    SF:/ci/src/queue.py\n\
                    DA:1,1\n\
                    end_of_record\n";
        let coverage = Coverage::parse(Path::new("lcov.info"), text).expect("parses");

        let matched = coverage.executed_file("src/queue.py").expect("matches");
        assert_eq!(matched.source(), Path::new("/ci/src/queue.py"));
    }

    /// `lcov -a` concatenates; the same file appears once per run and the counts
    /// add. Appending records instead of merging them would let a lookup find
    /// the run that did not touch the file.
    #[test]
    fn repeated_records_for_one_file_merge_rather_than_shadow() {
        let text = "TN:first\n\
                    SF:src/a.py\n\
                    FNDA:0,work\n\
                    DA:7,0\n\
                    end_of_record\n\
                    TN:second\n\
                    SF:src/a.py\n\
                    FNDA:3,work\n\
                    DA:7,3\n\
                    end_of_record\n";
        let coverage = Coverage::parse(Path::new("lcov.info"), text).expect("parses");

        assert_eq!(coverage.files().len(), 1, "one file, not two records");
        let (_, function) = coverage.called_function("work").expect("called in run two");
        assert_eq!(function.calls(), 3);
        assert_eq!(coverage.files()[0].lines_hit(), 1);
    }

    /// Both `FN:` dialects, and a name with a comma in it.
    #[test]
    fn fn_reads_the_one_line_and_the_start_end_dialects() {
        let text = "SF:src/lib.rs\n\
                    FN:12,map::Map<K, V>::insert\n\
                    FN:40,58,drain\n\
                    FNDA:1,map::Map<K, V>::insert\n\
                    FNDA:1,drain\n\
                    end_of_record\n";
        let coverage = Coverage::parse(Path::new("lcov.info"), text).expect("parses");
        let file = &coverage.files()[0];

        let insert = file
            .function("map::Map<K, V>::insert")
            .expect("kept its commas");
        assert_eq!(insert.start_line(), Some(12));
        assert_eq!(
            file.function("drain")
                .expect("start,end dialect")
                .start_line(),
            Some(40)
        );
    }

    /// Symmetric matching, because neither spelling is under our control.
    #[test]
    fn a_qualified_name_matches_a_bare_one_in_either_direction() {
        assert!(names_same_symbol("ledger.dunning.compute", "compute"));
        assert!(names_same_symbol("compute", "ledger/dunning.compute"));
        assert!(names_same_symbol("Ledger::Add", "Add"));
        assert!(
            !names_same_symbol("recompute", "compute"),
            "not a substring"
        );
        assert!(
            !names_same_symbol("", ""),
            "nameless never matches nameless"
        );
    }

    /// A malformed known key is an error, and an unknown key is not. The module
    /// docs argue why; this pins it.
    #[test]
    fn a_broken_fnda_is_an_error_and_an_unknown_key_is_skipped() {
        let unknown = "SF:src/a.py\n\
                       FNL:0,4,9\n\
                       FNA:0,3,work\n\
                       BRDA:7,0,0,1\n\
                       end_of_record\n";
        let coverage = Coverage::parse(Path::new("lcov.info"), unknown).expect("unknown keys skip");
        assert_eq!(
            coverage.functions_total(),
            0,
            "the 2.x index form parses to nothing, which the control then catches"
        );

        let broken = "SF:src/a.py\nFNDA:many,work\nend_of_record\n";
        let error =
            Coverage::parse(Path::new("lcov.info"), broken).expect_err("must not be silent");
        assert!(error.to_string().contains("line 2"), "{error}");
    }

    /// A record that never closes, and one that closes twice.
    #[test]
    fn record_framing_errors_are_loud() {
        let unclosed = "SF:src/a.py\nDA:1,1\n";
        Coverage::parse(Path::new("lcov.info"), unclosed).expect_err("no end_of_record");

        let orphan = "SF:src/a.py\nend_of_record\nend_of_record\n";
        Coverage::parse(Path::new("lcov.info"), orphan)
            .expect_err("end_of_record outside a record");

        let stray = "DA:1,1\n";
        Coverage::parse(Path::new("lcov.info"), stray).expect_err("DA before any SF");
    }

    /// gcov's `-` for an unknown count is not zero executions, and is never
    /// evidence either way.
    #[test]
    fn an_unknown_line_count_is_found_and_not_hit() {
        let text = "SF:src/a.py\nDA:1,-\nend_of_record\n";
        let coverage = Coverage::parse(Path::new("lcov.info"), text).expect("parses");
        assert_eq!(coverage.files()[0].lines_found(), 1);
        assert_eq!(coverage.files()[0].lines_hit(), 0);
        assert!(coverage.executed_file("src/a.py").is_none());
    }

    /// A tracefile written on a Windows runner.
    #[test]
    fn crlf_line_endings_do_not_end_up_inside_a_function_name() {
        let text = "SF:src/a.py\r\nFNDA:2,work\r\nend_of_record\r\n";
        let coverage = Coverage::parse(Path::new("lcov.info"), text).expect("parses");
        assert!(coverage.called_function("work").is_some());
    }
}
