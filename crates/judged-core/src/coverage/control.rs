//! The positive control an artifact must pass before anything believes it
//! (§3.7).
//!
//! §0 ranks positive controls the single cheapest high-value safety mechanism in
//! the whole survey, and §3.7 says why: **every** catastrophic failure in the
//! corpus presents identically, as an artifact reporting *"~0% covered
//! everywhere"* that is then trusted. The instrumenter did not attach. The test
//! runner exited before a single test. The artifact is from a different commit,
//! a different subdirectory, a shard that ran nothing. The parser here does not
//! read this lcov dialect. Every one of those produces the same bytes as a
//! genuinely unused codebase, and every one of them, believed, argues for
//! deleting the repository.
//!
//! A control is the declared, checkable claim that closes that gap: *these
//! symbols are always executed; if they are not, something is broken about the
//! measurement rather than about the code.* An artifact that cannot show them is
//! discarded whole, loudly, and rescues nothing.
//!
//! # The granularity is `FNDA`, and line granularity here is theatre
//!
//! §2.3, and it is the difference between a control and a decoration. Under
//! every documented failure mode above — a runner that booted and then died, an
//! instrumenter that attached for one process and no others — you get
//! *boot-only coverage*: the module-level lines of a repository are covered
//! while every function body in it reads dead. In Python this extends to the
//! `def` line itself, which really does execute at import: measured against
//! Coverage.py 7.15.2, a function nothing ever called carries `DA:7,1` on its
//! definition and `DA:8,0` on its body (`tests/coverage_real_artifacts.rs`). A
//! control asserting "line 7 of `handlers.py` was executed" passes on exactly
//! the artifact it exists to reject.
//!
//! So a control names **functions**, and they are checked against `FNDA`, which
//! only a call can raise. The floor ([`Control::min_called_functions`]) is the
//! second half: a handful of named symbols can survive an artifact that lost
//! 99% of its records, and a plausible count of called functions is what
//! notices.
//!
//! # What this does not defend against
//!
//! Forgery. The control is declared in the repository whose coverage is being
//! read, so whoever can write a bogus artifact can write a bogus control beside
//! it. That is the right scope: this is an instrument check, in the sense a
//! calibration standard is — it catches the measurement being broken, which is
//! the failure §3.7 documents and the one that actually happens.
//!
//! # The format
//!
//! One directive per line, `#` for comments, blank lines ignored:
//!
//! ```text
//! # judged coverage positive control
//! symbol handle_request
//! symbol Ledger.Add
//! min-called-functions 40
//! ```
//!
//! At least one `symbol` is required — a control with nothing in it always
//! passes, and §3.7 makes the same point about a positive control that cannot
//! fail as this module makes about coverage that cannot miss. An unknown
//! directive is an error rather than a skip, for the reason a malformed `FNDA`
//! is: a control quietly reduced to fewer assertions than its author wrote is a
//! weaker instrument presenting as the one they asked for.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::coverage::lcov::Coverage;
use crate::{Error, Result};

/// The declared set of always-live symbols, plus the floor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Control {
    path: PathBuf,
    symbols: Vec<String>,
    min_called_functions: usize,
}

impl Control {
    /// The conventional location of the control for `artifact`: the artifact's
    /// own path with `.control` appended.
    ///
    /// Derived rather than configured so the binding is structural. A control
    /// that has to be pointed at separately is a control that can be pointed at
    /// the wrong artifact, or at last week's.
    pub fn path_for(artifact: &Path) -> PathBuf {
        let mut path = artifact.as_os_str().to_os_string();
        path.push(".control");
        PathBuf::from(path)
    }

    /// Read and parse a control from disk.
    pub fn read(path: &Path) -> Result<Control> {
        let text = std::fs::read_to_string(path).map_err(|source| Error::Io {
            path: path.to_path_buf(),
            source,
        })?;
        Control::parse(path, &text)
    }

    /// Parse a control already in memory. `path` is used only to name the
    /// document in errors.
    pub fn parse(path: &Path, text: &str) -> Result<Control> {
        // A `BTreeSet` so a symbol declared twice is one assertion, in a stable
        // order: duplicates would otherwise inflate the default floor, which is
        // derived from this count.
        let mut symbols: BTreeSet<String> = BTreeSet::new();
        let mut floor: Option<usize> = None;

        for (index, raw) in text.lines().enumerate() {
            let number = index + 1;
            let line = raw.trim_end_matches('\r').trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            let (directive, value) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
            let value = value.trim();
            match directive {
                "symbol" => {
                    if value.is_empty() {
                        return Err(malformed(path, number, "symbol: with no name"));
                    }
                    symbols.insert(value.to_string());
                }
                "min-called-functions" => {
                    let parsed = value.parse().map_err(|_| {
                        malformed(
                            path,
                            number,
                            &format!("min-called-functions: {value:?} is not a number"),
                        )
                    })?;
                    floor = Some(parsed);
                }
                other => {
                    return Err(malformed(
                        path,
                        number,
                        &format!(
                            "unknown directive {other:?}; expected `symbol` or \
                             `min-called-functions`"
                        ),
                    ))
                }
            }
        }

        if symbols.is_empty() {
            return Err(Error::Coverage {
                path: path.to_path_buf(),
                message: "a control with no `symbol` line asserts nothing and would \
                          pass on an empty artifact (§3.7)"
                    .to_string(),
            });
        }

        Ok(Control {
            path: path.to_path_buf(),
            min_called_functions: floor.unwrap_or(symbols.len()),
            symbols: symbols.into_iter().collect(),
        })
    }

    /// Where this control was declared.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The declared always-live symbols, in name order.
    pub fn symbols(&self) -> &[String] {
        &self.symbols
    }

    /// How many called functions the whole artifact must carry.
    ///
    /// Defaults to the number of declared symbols, which is the floor those
    /// assertions already imply and therefore adds nothing on its own. A real
    /// repository should raise it: the named symbols can survive an artifact
    /// that lost most of its records, and a plausible total is what notices
    /// that.
    pub fn min_called_functions(&self) -> usize {
        self.min_called_functions
    }

    /// Check `coverage` against this control.
    ///
    /// Never returns an error and never short-circuits: a report has to be able
    /// to say *which* symbols were missing, not merely that one was.
    pub fn check(&self, coverage: &Coverage) -> ControlOutcome {
        let uncalled: Vec<String> = self
            .symbols
            .iter()
            .filter(|symbol| coverage.called_function(symbol).is_none())
            .cloned()
            .collect();

        ControlOutcome {
            control: self.path.clone(),
            artifact: coverage.artifact().to_path_buf(),
            symbols_declared: self.symbols.len(),
            symbols_uncalled: uncalled,
            functions_called: coverage.functions_called(),
            floor: self.min_called_functions,
        }
    }
}

/// What the control said about one artifact.
///
/// Holds its numbers whether it passed or failed. A passing control that cannot
/// say *how much* it saw is the same shape as the artifact it is checking: a
/// green with no denominator (§6.20).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlOutcome {
    control: PathBuf,
    artifact: PathBuf,
    symbols_declared: usize,
    symbols_uncalled: Vec<String>,
    functions_called: usize,
    floor: usize,
}

impl ControlOutcome {
    /// Whether the artifact may be believed.
    pub fn passed(&self) -> bool {
        self.symbols_uncalled.is_empty() && self.functions_called >= self.floor
    }

    /// The control that produced this.
    pub fn control(&self) -> &Path {
        &self.control
    }

    /// The artifact it checked.
    pub fn artifact(&self) -> &Path {
        &self.artifact
    }

    /// How many symbols were asserted.
    pub fn symbols_declared(&self) -> usize {
        self.symbols_declared
    }

    /// The declared symbols the artifact has no call for, in name order.
    pub fn symbols_uncalled(&self) -> &[String] {
        &self.symbols_uncalled
    }

    /// Called functions in the whole artifact.
    pub fn functions_called(&self) -> usize {
        self.functions_called
    }

    /// The floor that was required of it.
    pub fn floor(&self) -> usize {
        self.floor
    }

    /// Why it failed, in sentences somebody can act on. Empty on a pass.
    ///
    /// Both halves are reported, not the first one to fire: an artifact from the
    /// wrong commit and an artifact from a runner that died are different
    /// remediations, and an operator reading one line would fix the wrong thing.
    pub fn failures(&self) -> Vec<String> {
        let mut failures = Vec::new();
        if !self.symbols_uncalled.is_empty() {
            failures.push(format!(
                "{} of {} always-live symbols have no recorded call ({}); the \
                 artifact does not describe a run of this code",
                self.symbols_uncalled.len(),
                self.symbols_declared,
                self.symbols_uncalled.join(", ")
            ));
        }
        if self.functions_called < self.floor {
            failures.push(format!(
                "{} called functions is under the declared floor of {}; the \
                 artifact is missing records rather than the code being unused",
                self.functions_called, self.floor
            ));
        }
        failures
    }
}

fn malformed(path: &Path, number: usize, message: &str) -> Error {
    Error::Coverage {
        path: path.to_path_buf(),
        message: format!("line {number}: {message}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn coverage(text: &str) -> Coverage {
        Coverage::parse(Path::new("lcov.info"), text).expect("fixture parses")
    }

    /// The artifact §3.7 describes: everything present, nothing called. It must
    /// not be believed.
    #[test]
    fn boot_only_coverage_fails_the_control() {
        let control = Control::parse(
            Path::new("lcov.info.control"),
            "symbol handle_request\nsymbol health\n",
        )
        .expect("parses");

        // Every `def` line covered, every body dead — exactly what an
        // instrumenter that attached at import and never saw a request writes.
        let outcome = control.check(&coverage(
            "SF:src/handlers.py\n\
             FN:4,handle_request\n\
             FNDA:0,handle_request\n\
             FN:12,health\n\
             FNDA:0,health\n\
             DA:4,1\n\
             DA:12,1\n\
             end_of_record\n",
        ));

        assert!(!outcome.passed());
        assert_eq!(
            outcome.symbols_uncalled(),
            ["handle_request".to_string(), "health".to_string()]
        );
        assert!(outcome.failures()[0].contains("no recorded call"));
    }

    /// The floor catches what the named symbols cannot: an artifact that kept
    /// the shard those symbols ran in and lost the rest.
    #[test]
    fn the_floor_catches_a_truncated_artifact_the_symbols_survive() {
        let control = Control::parse(
            Path::new("lcov.info.control"),
            "symbol handle_request\nmin-called-functions 40\n",
        )
        .expect("parses");

        let outcome = control.check(&coverage(
            "SF:src/handlers.py\nFNDA:9,handle_request\nend_of_record\n",
        ));

        assert!(
            !outcome.passed(),
            "one call is not a run of this repository"
        );
        assert!(outcome.symbols_uncalled().is_empty());
        assert_eq!(outcome.failures().len(), 1);
        assert!(outcome.failures()[0].contains("under the declared floor"));
    }

    /// Both halves are reported, because they are different remediations.
    #[test]
    fn a_control_that_fails_both_ways_says_both() {
        let control = Control::parse(
            Path::new("c"),
            "symbol missing\nsymbol present\nmin-called-functions 10\n",
        )
        .expect("parses");
        let outcome = control.check(&coverage("SF:a.py\nFNDA:1,present\nend_of_record\n"));

        assert_eq!(outcome.failures().len(), 2);
    }

    #[test]
    fn a_control_that_passes_still_carries_its_numbers() {
        let control =
            Control::parse(Path::new("c"), "symbol present\nmin-called-functions 2\n").expect("ok");
        let outcome = control.check(&coverage(
            "SF:a.py\nFNDA:1,present\nFNDA:3,other\nend_of_record\n",
        ));

        assert!(outcome.passed());
        assert!(outcome.failures().is_empty());
        assert_eq!(outcome.functions_called(), 2);
        assert_eq!(outcome.floor(), 2);
        assert_eq!(outcome.symbols_declared(), 1);
    }

    /// A control names symbols however its author does; the artifact names them
    /// however the instrumenter does.
    #[test]
    fn a_declared_symbol_matches_the_instrumenters_qualified_spelling() {
        let control = Control::parse(Path::new("c"), "symbol handle_request\n").expect("parses");
        let outcome = control.check(&coverage(
            "SF:src/handlers.py\nFNDA:2,app.handlers.handle_request\nend_of_record\n",
        ));
        assert!(outcome.passed());
    }

    /// An empty control passes on an empty artifact, so it is refused at parse
    /// time rather than believed at check time.
    #[test]
    fn a_control_with_no_symbols_is_refused() {
        let error = Control::parse(Path::new("c"), "# nothing here\n").expect_err("refused");
        assert!(error.to_string().contains("asserts nothing"), "{error}");

        Control::parse(Path::new("c"), "min-called-functions 5\n")
            .expect_err("a floor alone is not a control");
    }

    /// A typo must not quietly become a weaker instrument.
    #[test]
    fn an_unknown_directive_is_an_error() {
        let error = Control::parse(Path::new("c"), "symbol a\nsymbols b\n").expect_err("refused");
        assert!(error.to_string().contains("line 2"), "{error}");

        Control::parse(Path::new("c"), "symbol a\nmin-called-functions lots\n")
            .expect_err("a floor that is not a number");
    }

    /// The floor defaults to what the symbols already imply, so it can only be
    /// raised, never silently invented.
    #[test]
    fn the_default_floor_is_the_declared_symbol_count() {
        let control = Control::parse(Path::new("c"), "symbol a\nsymbol b\nsymbol a\n").expect("ok");
        assert_eq!(control.symbols(), ["a".to_string(), "b".to_string()]);
        assert_eq!(control.min_called_functions(), 2, "deduplicated");
    }

    /// The control's location is derived from the artifact's, so the two cannot
    /// be paired wrongly.
    #[test]
    fn the_control_path_is_the_artifact_path_plus_control() {
        assert_eq!(
            Control::path_for(Path::new("coverage/lcov.info")),
            PathBuf::from("coverage/lcov.info.control")
        );
    }
}
