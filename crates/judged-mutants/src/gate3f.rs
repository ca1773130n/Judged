//! Gate 3f as a refusal layer over any [`Sut`] (§9.3, §6.24).
//!
//! The fifth layer, and the one with the strongest claim on a claim. §9.3 ends
//! 3f with *"No ban count overrides this"* — the only conjunct in the design
//! that says so — because the evidence that would refute deadness is not in any
//! observable system. A queue payload enqueued yesterday, a row pickled last
//! year, a binary linked in 2023: static reachability, the grep veto, coverage,
//! tombstones and the build graph all read the current tree and the current
//! fleet, and none of them can see any of those.
//!
//! # Same invariant as everything else here
//!
//! It refuses; it never accuses. The claim set it returns is a **subset** of
//! what the inner SUT claimed, asserted on the sets rather than on their sizes
//! in `tests/gate3f_gate.rs`.
//!
//! # Where it sits in the stack
//!
//! Innermost of the evidence layers and immediately after Gate 1, because both
//! are about the *cost of being wrong* rather than about usefulness. A claim 3f
//! refuses is never handed to a later layer, so there is no later evidence for
//! anything to override it with — which is the composition expressing what
//! §9.3's sentence says in prose.

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use judged_core::gate3f::{Condition, Gate3f, Gate3fVerdict};
use judged_core::{Error, Result};

use crate::mutant::Ecosystem;
use crate::sut::{ClaimKind, Sut, SutVerdict, SymbolClaim};

/// One claim 3f refused, with the evidence that refused it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gate3fRefusal {
    /// The claim, spelled exactly as the analyzer spelled it.
    pub claim: String,
    /// Whether that was a path or a symbol.
    pub kind: ClaimKind,
    /// The condition reported as the headline: the first in [`Condition`]
    /// order that fired.
    ///
    /// **One refusal per claim, never one per condition.** A claim refused for
    /// two reasons is still one claim, and emitting a record per condition made
    /// the layer account for more claims than it was handed — which the CLI's
    /// rescue-layer invariant caught and refused the whole run over, correctly.
    /// Everything that fired is in [`Self::conditions`].
    pub condition: Condition,
    /// Every condition that fired, in [`Condition`] order.
    pub conditions: Vec<Condition>,
    /// The marker literal that matched.
    pub marker: String,
    /// The file it matched in, repo-relative, and the 1-based line.
    pub found_in: PathBuf,
    pub line: usize,
    /// For a symbol claim, the file the analyzer said declares it — the file
    /// that was actually read. `None` for a path claim, and for a symbol the
    /// analyzer attributed to nothing.
    ///
    /// Carried for the same reason Gate 1 carries it: 3f's conditions are
    /// properties of a file, so a symbol is judged by the file that declares it,
    /// and a reader who cannot see which file that was cannot check the refusal.
    pub declared_in: Option<PathBuf>,
    /// The whole reason, including what deleting the candidate would break.
    pub detail: String,
}

/// What 3f did during one call to the inner SUT.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gate3fRun {
    /// The repository judged.
    pub repo: PathBuf,
    /// Claims the analyzer made.
    pub claimed: usize,
    /// Claims that survived.
    pub survived: usize,
    /// Every claim that did not, in the order the analyzer made them.
    pub refused: Vec<Gate3fRefusal>,
    /// Job frameworks detected anywhere in the repository, as
    /// `name@file:line`.
    ///
    /// Carried per run rather than summarized, because the queue condition is
    /// unfalsifiable from the outside without it: "refused, queue payload" with
    /// no named framework is a verdict a reader cannot check.
    pub frameworks: Vec<String>,
    /// Symbol claims the analyzer attributed to no file, and which 3f therefore
    /// could not judge at all.
    ///
    /// **Not refusals and not clearances.** §6.20's distinction, kept as its own
    /// number: a gate that silently treated "nothing to read" as "nothing to
    /// say" would report a clean pass over claims it never examined.
    pub unattributed: usize,
}

/// Any [`Sut`], with §9.3's Gate 3f run over every claim it makes.
pub struct Gate3fSut {
    inner: Box<dyn Sut>,
    name: String,
    /// One entry per call to [`Sut::run`], in call order. `RefCell` because
    /// [`Sut::run`] takes `&self`; single-threaded by construction, since
    /// [`crate::runner::run_suite`] drives one mutant at a time.
    runs: RefCell<Vec<Gate3fRun>>,
}

impl Gate3fSut {
    /// `inner`, with every claim judged by 3f.
    pub fn new(inner: Box<dyn Sut>) -> Gate3fSut {
        let name = format!("{}+gate3f", inner.name());
        Gate3fSut {
            inner,
            name,
            runs: RefCell::new(Vec::new()),
        }
    }

    /// What 3f did, one entry per call to the inner SUT, in call order.
    pub fn runs(&self) -> Vec<Gate3fRun> {
        self.runs.borrow().clone()
    }
}

impl Sut for Gate3fSut {
    fn name(&self) -> &str {
        &self.name
    }

    fn cannot_emit(&self) -> Vec<String> {
        self.inner.cannot_emit()
    }

    fn reads(&self) -> Option<&[Ecosystem]> {
        self.inner.reads()
    }

    fn run(&self, repo: &Path) -> Result<SutVerdict> {
        let claims = self.inner.run(repo)?;

        // Before any claim is judged. A 3f that cannot scan the repository must
        // not hand the claims back untouched: the report would still say the run
        // was gated, and a gate that silently did not run is worse than one
        // never added.
        let gate = Gate3f::build(repo).map_err(|source| Error::Sut {
            sut: self.name.clone(),
            message: format!(
                "Gate 3f could not scan {}: {source}. Refusing to report an ungated run \
                 as a gated one.",
                repo.display()
            ),
        })?;

        let claimed = claims.claimed_dead_paths.len() + claims.claimed_dead_symbols.len();
        let mut refused: Vec<Gate3fRefusal> = Vec::new();
        let mut claimed_dead_paths: Vec<PathBuf> = Vec::new();
        let mut claimed_dead_symbols: Vec<SymbolClaim> = Vec::new();
        let mut unattributed = 0usize;

        for path in &claims.claimed_dead_paths {
            let relative = relative_to(path, repo);
            let verdict = gate.judge_path(Path::new(&relative))?;
            if let Some(entry) = record(path.display().to_string(), ClaimKind::Path, None, &verdict)
            {
                refused.push(entry);
            } else {
                claimed_dead_paths.push(path.clone());
            }
        }

        for symbol in &claims.claimed_dead_symbols {
            let declared = symbol.declaration_site().map(Path::to_path_buf);
            if declared.is_none() {
                unattributed += 1;
            }
            let verdict = gate.judge_symbol(symbol.name(), declared.as_deref())?;
            if let Some(entry) = record(
                symbol.name().to_string(),
                ClaimKind::Symbol,
                declared,
                &verdict,
            ) {
                refused.push(entry);
            } else {
                claimed_dead_symbols.push(symbol.clone());
            }
        }

        let survived = claimed_dead_paths.len() + claimed_dead_symbols.len();
        self.runs.borrow_mut().push(Gate3fRun {
            repo: repo.to_path_buf(),
            claimed,
            survived,
            refused,
            frameworks: gate
                .frameworks()
                .iter()
                .map(|f| format!("{}@{}:{}", f.name, f.found_in.display(), f.line))
                .collect(),
            unattributed,
        });

        Ok(SutVerdict {
            claimed_dead_paths,
            claimed_dead_symbols,
        })
    }
}

/// One refusal per claim, carrying every condition that fired.
///
/// The headline is the first finding in [`Condition`] order, and the rest ride
/// in `conditions` and in the detail. Emitting one record per condition instead
/// makes a claim refused twice count twice, which is not a cosmetic difference:
/// the CLI asserts that a rescue layer accounts for exactly the claims it was
/// handed, and it refused an entire run over the discrepancy rather than
/// publishing it.
fn record(
    claim: String,
    kind: ClaimKind,
    declared_in: Option<PathBuf>,
    verdict: &Gate3fVerdict,
) -> Option<Gate3fRefusal> {
    let first = verdict.findings().first()?;
    let conditions: Vec<Condition> = {
        let mut all: Vec<Condition> = verdict.findings().iter().map(|f| f.condition).collect();
        all.dedup();
        all
    };
    let detail = verdict
        .findings()
        .iter()
        .map(|finding| finding.detail.clone())
        .collect::<Vec<_>>()
        .join("; also ");

    Some(Gate3fRefusal {
        claim,
        kind,
        condition: first.condition,
        conditions,
        marker: first.marker.clone(),
        found_in: first.found_in.clone(),
        line: first.line,
        declared_in,
        detail,
    })
}

/// `path` rendered relative to `repo_root`, forward-slashed.
fn relative_to(path: &Path, repo_root: &Path) -> String {
    path.strip_prefix(repo_root)
        .unwrap_or(path)
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}
