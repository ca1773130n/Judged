//! The fourth rescue layer: observed execution (§9.5, Family X).
//!
//! Gate 1 asks *what does it cost to be wrong about this*. Gate 2 asks *does
//! anything in this repository name it*. The root set asks *was it declared an
//! entry point*. All three read the repository's text, which makes all three
//! Family R — and §9.5 requires a quorum of at least two of {B, R, X} before any
//! Tier-0 action, so a build made only of them cannot reach Tier 0 no matter how
//! it scores. This layer is the first thing here that observes the program
//! running.
//!
//! # It is a veto, not an accuser, and that is not negotiable
//!
//! A coverage **hit** is proof of use, so it drops a claim. A coverage **miss**
//! contributes **zero** toward deadness at any tier (§9.5), so it does nothing
//! at all. The miss is not merely weak evidence: it is systematically
//! anti-correlated with the value of the code, because error handlers,
//! disaster-recovery paths, platform branches and migration shims are precisely
//! what a test suite never enters and precisely what must survive.
//!
//! Concretely, this type has the same invariant as [`crate::sut::VetoedSut`] and
//! [`crate::roots::RootedSut`]: the claim set it returns is a **subset** of what
//! the inner SUT claimed. `tests/coverage_gate.rs` asserts the subset relation
//! on the sets rather than on their sizes, so no future change can turn a miss
//! into a nomination without failing.
//!
//! # An unverified artifact rescues nothing, and says so
//!
//! Reading coverage is the one place in this workspace where the *input* is a
//! measurement somebody else took, which brings §3.7's failure mode inside the
//! trust boundary: an artifact that was never written, was written by a run that
//! died on boot, or is in a dialect the parser cannot read all look exactly like
//! a repository nothing uses.
//!
//! So there is no path through this layer in which an artifact is believed
//! without its positive control ([`judged_core::coverage::control`]). Missing
//! artifact, missing control, unparseable either, failing control — all four
//! produce the same behaviour: **zero rescues, and a gap in the run that says
//! which it was.** That is the safe direction (a layer that rescues nothing can
//! only leave claims standing, never invent one) and the loud one, because a
//! silent zero here is indistinguishable from a class where coverage genuinely
//! had nothing to say.

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use judged_core::coverage::{Control, ControlOutcome, Coverage};
use judged_core::Result;

use crate::mutant::Ecosystem;
use crate::sut::{ClaimKind, Sut, SutVerdict, SymbolClaim};

/// Where a repository is expected to have put its tracefile.
///
/// The path CI tools converge on: `nyc`/`c8`, `cargo-llvm-cov`, `coverage.py`'s
/// `lcov` report and `simplecov-lcov` all write here or are routinely configured
/// to. Overridable, because a convention that cannot be overridden is a reason
/// to not use the layer at all.
pub const DEFAULT_ARTIFACT: &str = "coverage/lcov.info";

/// One claim that observed execution rescued, with the evidence.
///
/// §9.13 asks for a conflict list rather than a score. For this layer the
/// evidence is the strongest kind in the whole system — not "something names
/// it", but "it ran, this many times" — so the count is carried rather than
/// summarized to a boolean. A rescue that says `called 4,281 times` and a rescue
/// that says `called once` are different facts, and the second is the one worth
/// looking at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoverageRescue {
    /// The claim, spelled exactly as the analyzer spelled it.
    pub claim: String,
    /// Whether that was a path or a symbol.
    pub kind: ClaimKind,
    /// The `SF:` path the evidence came from, as the artifact spelled it —
    /// which is a path on the machine that ran the tests, and is deliberately
    /// not rewritten to look local.
    pub source: PathBuf,
    /// The instrumenter's name for the function, when the rescue came from an
    /// `FNDA` record. `None` for a path rescue.
    pub function: Option<String>,
    /// Times that function was entered. `None` for a path rescue, whose evidence
    /// is line-granular; see [`CoverageRescue::detail`].
    pub calls: Option<u64>,
    /// The whole reason, in a sentence somebody can act on.
    pub detail: String,
}

/// Why this run had no usable coverage. Never a reason to rescue, always a
/// reason to distrust the zero beside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoverageGap {
    /// No tracefile at the expected path. Ordinary — most repositories in a
    /// suite will not have one — and still recorded, because "no artifact" and
    /// "an artifact that rescued nothing" are the §6.20 pair that must never
    /// share a row.
    NoArtifact { expected: PathBuf },
    /// A tracefile, and no control beside it. §3.7: an artifact nobody declared
    /// a check for is an artifact nobody can tell apart from a broken one, so it
    /// is discarded rather than believed.
    NoControl {
        artifact: PathBuf,
        expected: PathBuf,
    },
    /// The tracefile or the control would not parse.
    Unreadable { path: PathBuf, message: String },
    /// The control ran and refused the artifact.
    ControlFailed {
        artifact: PathBuf,
        failures: Vec<String>,
    },
}

impl CoverageGap {
    /// A stable lower-case label, for a report that groups by cause.
    pub fn kind(&self) -> &'static str {
        match self {
            CoverageGap::NoArtifact { .. } => "no-artifact",
            CoverageGap::NoControl { .. } => "no-control",
            CoverageGap::Unreadable { .. } => "unreadable",
            CoverageGap::ControlFailed { .. } => "control-failed",
        }
    }
}

impl std::fmt::Display for CoverageGap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CoverageGap::NoArtifact { expected } => write!(
                f,
                "no coverage artifact at {}; this class contributed no X-family \
                 evidence either way",
                expected.display()
            ),
            CoverageGap::NoControl { artifact, expected } => write!(
                f,
                "{} has no positive control at {}; an artifact with no declared \
                 always-live symbols cannot be told apart from one produced by a \
                 run that died on boot (§3.7), so it was discarded whole",
                artifact.display(),
                expected.display()
            ),
            CoverageGap::Unreadable { path, message } => {
                write!(f, "{} could not be read: {message}", path.display())
            }
            CoverageGap::ControlFailed { artifact, failures } => write!(
                f,
                "{} failed its positive control and was discarded whole: {}",
                artifact.display(),
                failures.join("; ")
            ),
        }
    }
}

/// What the coverage layer did during one call to the inner SUT.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoverageRun {
    /// The repository the artifact was looked for in.
    pub repo: PathBuf,
    /// Claims the analyzer made.
    pub claimed: usize,
    /// Claims that survived.
    pub survived: usize,
    /// Every claim that did not, with its evidence. In the order the analyzer
    /// made them, never sorted by anything that would flatter the layer (§9.13
    /// invariant 3).
    pub rescued: Vec<CoverageRescue>,
    /// The artifact that was believed, repo-relative. `None` whenever
    /// [`CoverageRun::gap`] is set — those two fields are the §6.20 distinction
    /// made structural, and exactly one of them is populated.
    pub artifact: Option<PathBuf>,
    /// What the positive control saw. `Some` only when an artifact and a control
    /// were both read; a failing outcome is carried here *and* summarized in
    /// [`CoverageRun::gap`], so a report can show the numbers that refused it.
    pub control: Option<ControlOutcome>,
    /// Why there was nothing to believe, when there was nothing to believe.
    pub gap: Option<CoverageGap>,
}

impl CoverageRun {
    /// Whether this run had an artifact that passed its control.
    ///
    /// **The denominator.** A suite-level rescue count without a count of the
    /// classes that had usable coverage is the shape §6.20 forbids: zero
    /// rescues over nineteen classes with no artifact, and zero rescues over
    /// nineteen classes fully covered, are the same number and opposite
    /// findings.
    pub fn had_coverage(&self) -> bool {
        self.artifact.is_some()
    }
}

/// Any [`Sut`], with observed execution (§9.5, Family X) run over every claim it
/// makes.
///
/// # A pure filter, and nothing else
///
/// The accuser runs first, and every survivor is checked against the artifact.
/// Nothing here can add a claim, promote one, or turn a miss into an
/// accusation — the only operation is dropping.
pub struct CoveredSut {
    inner: Box<dyn Sut>,
    name: String,
    /// Where to look, relative to each repository's root. Relative rather than
    /// absolute because [`Sut::run`] is handed a different repository per
    /// mutant, and a layer pinned to one absolute artifact would answer every
    /// class with the same file.
    artifact: PathBuf,
    /// One entry per call to [`Sut::run`], in call order. `RefCell` because
    /// [`Sut::run`] takes `&self` and this wrapper additionally has to record
    /// what it did; single-threaded by construction, since
    /// [`crate::runner::run_suite`] drives one mutant at a time.
    runs: RefCell<Vec<CoverageRun>>,
}

impl CoveredSut {
    /// `inner`, with [`DEFAULT_ARTIFACT`] read from each repository.
    pub fn new(inner: Box<dyn Sut>) -> CoveredSut {
        CoveredSut::with_artifact(inner, DEFAULT_ARTIFACT)
    }

    /// `inner`, with a tracefile at an explicit repo-relative path.
    pub fn with_artifact(inner: Box<dyn Sut>, artifact: impl Into<PathBuf>) -> CoveredSut {
        let name = format!("{}+coverage", inner.name());
        CoveredSut {
            inner,
            name,
            artifact: artifact.into(),
            runs: RefCell::new(Vec::new()),
        }
    }

    /// The repo-relative artifact path this layer reads.
    pub fn artifact(&self) -> &Path {
        &self.artifact
    }

    /// What the layer did, one entry per call to the inner SUT, in call order.
    pub fn runs(&self) -> Vec<CoverageRun> {
        self.runs.borrow().clone()
    }

    /// Load the artifact for one repository, or say why there is nothing to
    /// load.
    ///
    /// Every failure returns `Err` and no failure returns coverage: there is
    /// deliberately no branch in which a tracefile reaches the caller without a
    /// control having passed on it.
    fn load(&self, repo: &Path) -> std::result::Result<(Coverage, ControlOutcome), Box<Rejection>> {
        let artifact = repo.join(&self.artifact);
        if !artifact.is_file() {
            return Err(CoverageGap::NoArtifact {
                expected: self.artifact.clone(),
            }
            .into());
        }

        // The control is looked for before the artifact is parsed. An artifact
        // with no control is discarded whichever way it would have parsed, and
        // parsing first would let a malformed-tracefile message stand in for the
        // more important fact that nobody declared what this file should show.
        let control_path = Control::path_for(&artifact);
        if !control_path.is_file() {
            return Err(CoverageGap::NoControl {
                artifact: self.artifact.clone(),
                expected: Control::path_for(&self.artifact),
            }
            .into());
        }

        let control = Control::read(&control_path).map_err(|error| CoverageGap::Unreadable {
            path: Control::path_for(&self.artifact),
            message: error.to_string(),
        })?;
        let coverage = Coverage::read(&artifact).map_err(|error| CoverageGap::Unreadable {
            path: self.artifact.clone(),
            message: error.to_string(),
        })?;

        let outcome = control.check(&coverage);
        if !outcome.passed() {
            return Err(Box::new(Rejection {
                gap: CoverageGap::ControlFailed {
                    artifact: self.artifact.clone(),
                    failures: outcome.failures(),
                },
                // Carried through the refusal, not discarded with it. "The
                // control failed" and "the control failed because 0 of 40
                // functions were ever entered" are different remediations, and
                // an operator who is shown only the first fixes the wrong thing.
                control: Some(outcome),
            }));
        }
        Ok((coverage, outcome))
    }
}

/// A refusal to believe an artifact, with whatever the control managed to
/// measure before refusing.
struct Rejection {
    gap: CoverageGap,
    control: Option<ControlOutcome>,
}

/// Every rejection that happens before the control could run carries no numbers,
/// which is what this conversion says.
///
/// Boxed on the way out because a `Result` whose error is larger than its `Ok`
/// makes every caller pay for the failure path (`clippy::result_large_err`), and
/// this one is only ever built once per repository.
impl From<CoverageGap> for Box<Rejection> {
    fn from(gap: CoverageGap) -> Box<Rejection> {
        Box::new(Rejection { gap, control: None })
    }
}

impl Sut for CoveredSut {
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
        let claimed = claims.claimed_dead_paths.len() + claims.claimed_dead_symbols.len();

        let (coverage, control) = match self.load(repo) {
            Ok(loaded) => loaded,
            // Nothing to believe, so nothing is rescued and every claim stands.
            // Recorded rather than returned as an error: across a suite most
            // repositories legitimately have no tracefile, and aborting on the
            // first would make the layer unusable exactly where it is being
            // measured. The gap is what keeps the zero from reading as a
            // finding.
            Err(rejection) => {
                self.runs.borrow_mut().push(CoverageRun {
                    repo: repo.to_path_buf(),
                    claimed,
                    survived: claimed,
                    rescued: Vec::new(),
                    artifact: None,
                    control: rejection.control,
                    gap: Some(rejection.gap),
                });
                return Ok(claims);
            }
        };

        let mut rescued: Vec<CoverageRescue> = Vec::new();
        let mut claimed_dead_paths: Vec<PathBuf> = Vec::new();
        let mut claimed_dead_symbols: Vec<SymbolClaim> = Vec::new();

        for path in &claims.claimed_dead_paths {
            let relative = relative_to(path, repo);
            match coverage.executed_file(&relative) {
                Some(file) => rescued.push(CoverageRescue {
                    claim: path.display().to_string(),
                    kind: ClaimKind::Path,
                    source: file.source().to_path_buf(),
                    function: None,
                    calls: None,
                    detail: format!(
                        "{} executed {} of {} recorded lines",
                        file.source().display(),
                        file.lines_hit(),
                        file.lines_found()
                    ),
                }),
                None => claimed_dead_paths.push(path.clone()),
            }
        }

        for symbol in &claims.claimed_dead_symbols {
            match coverage.called_function(symbol.name()) {
                Some((file, function)) => rescued.push(CoverageRescue {
                    claim: symbol.name().to_string(),
                    kind: ClaimKind::Symbol,
                    source: file.source().to_path_buf(),
                    function: Some(function.name().to_string()),
                    calls: Some(function.calls()),
                    detail: format!(
                        "{} was entered {} time(s), recorded in {}",
                        function.name(),
                        function.calls(),
                        file.source().display()
                    ),
                }),
                None => claimed_dead_symbols.push(symbol.clone()),
            }
        }

        let survived = claimed_dead_paths.len() + claimed_dead_symbols.len();
        self.runs.borrow_mut().push(CoverageRun {
            repo: repo.to_path_buf(),
            claimed,
            survived,
            rescued,
            artifact: Some(self.artifact.clone()),
            control: Some(control),
            gap: None,
        });

        Ok(SutVerdict {
            claimed_dead_paths,
            claimed_dead_symbols,
        })
    }
}

/// `path` rendered relative to `repo_root`, forward-slashed.
///
/// An analyzer may spell a claim absolutely, and an absolute local path compared
/// against a tracefile's absolute *remote* path matches nothing at all — which
/// presents as a rescue layer that never fires, the silent-disabling shape this
/// codebase normalizes against everywhere it compares a path.
fn relative_to(path: &Path, repo_root: &Path) -> String {
    path.strip_prefix(repo_root)
        .unwrap_or(path)
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}
