//! Gate 1 — the never-touch inventory — assembled, and measured (§9.3).
//!
//! [`judged_core::gate1`] holds the sixteen classes as three modules with three
//! vocabularies: 1a–1f are [`state`], 1g–1k are [`content`], 1l–1p are
//! [`contracts`]. Each answers about its own classes and abstains about the
//! rest, which is right for a module and useless to a caller: a candidate is
//! ineligible if **any** class refuses, so somebody has to ask all sixteen and
//! put the answers in §9.3's order. This is that assembler, and [`Gate1Sut`] is
//! what makes it a layer the E2 suite can grade.
//!
//! # Why it runs first
//!
//! Every other layer reasons about **usefulness**: is this referenced (Gate 2),
//! was it declared an entry point (the root set). Gate 1 is the only one that
//! reasons about the **cost of being wrong**, and §9.3 puts it first for that
//! reason — its refusals are *"justified by IRREVERSIBILITY, not uselessness"*.
//! A file can be provably unreferenced and still be the last copy of something.
//!
//! The ordering is structural rather than conventional. [`Gate1Sut`] wraps the
//! analyzer directly and every later layer wraps *it*, so a claim Gate 1 refused
//! is never handed to Gate 2 at all. That is also what makes the refusal
//! absorbing without anything having to enforce absorption: there is no later
//! evidence, because there is no later question.
//!
//! # Why it lives in this crate
//!
//! For the same reason [`crate::roots`] does: because this is where it is
//! *measured*. Nothing here depends on the E2 catalogue, so moving the assembler
//! into `judged_core::gate1` is a re-export away once that module has one.

use std::cell::RefCell;
use std::fmt;
use std::path::{Path, PathBuf};

use judged_core::gate1::content::{ContentGate, ContentVerdict};
use judged_core::gate1::contracts::{ContractGate, Disposition, Refusal, TypeSignal};
use judged_core::gate1::state::{sniff, Magic, StateGate, StateVerdict, HEAD_BYTES};
use judged_core::git::{RecoverabilityClass, Repo};
use judged_core::{Error, Result};

use crate::sut::{ClaimKind, Sut, SutVerdict, SymbolClaim};
use crate::Ecosystem;

// ---------------------------------------------------------------------------
// One class's objection
// ---------------------------------------------------------------------------

/// One §9.3 class refusing one candidate, with the evidence behind it.
///
/// §9.13 asks for a conflict list rather than a score, and §7.3 records that the
/// best-validated prior art in the research — IntelliJ's Safe Delete — shows the
/// *usage list*, not a probability. A refusal that cannot say which class fired
/// and on what is a score wearing a longer name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gate1Finding {
    /// The §9.3 class code: `1a` through `1p`.
    pub class: &'static str,
    /// The class name as §9.3 writes it.
    pub title: &'static str,
    /// What was observed, quoted closely enough to re-check by hand.
    pub evidence: String,
}

impl fmt::Display for Gate1Finding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}: {}", self.class, self.title, self.evidence)
    }
}

/// What all sixteen classes had to say about one candidate.
///
/// Every class that fired is carried, not just the first. §9.3's refusals stack:
/// `media/customer/.htaccess` is an upload tree *and* an Apache routing contract
/// *and* a file re-included by a `!` negation, and reporting only whichever the
/// evaluation order reached first would make the verdict an artifact of the
/// code's shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gate1Verdict {
    path: PathBuf,
    findings: Vec<Gate1Finding>,
    type_signal: Option<TypeSignal>,
}

impl Gate1Verdict {
    /// The candidate, relative to the working tree root.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Every class that refused, in §9.3's order. Empty when none did.
    pub fn findings(&self) -> &[Gate1Finding] {
        &self.findings
    }

    /// How the file's type was determined, or `None` when it was not — which is
    /// itself class 1p, and appears in [`Gate1Verdict::findings`] as one.
    pub fn type_signal(&self) -> Option<TypeSignal> {
        self.type_signal
    }

    /// Whether any class refused.
    ///
    /// **The complement is not a safety claim.** No class objecting means the
    /// never-touch inventory has nothing to say, not that the candidate is dead:
    /// Gate 2, the root set and Gate 3 have not run, and Gate 1 refuses rather
    /// than accuses (§9.1).
    pub fn is_ineligible(&self) -> bool {
        !self.findings.is_empty()
    }
}

// ---------------------------------------------------------------------------
// The assembler
// ---------------------------------------------------------------------------

/// All sixteen Gate 1 classes over one repository.
///
/// Built once per repository and queried per candidate: [`StateGate::survey`]
/// walks the tree for effector markers, [`ContentGate::build`] walks it for
/// `.gitattributes`, and both are whole-tree facts that must not be recomputed
/// per file.
pub struct Gate1 {
    repo: Repo,
    state: StateGate,
    content: ContentGate,
}

impl Gate1 {
    /// Build every class over the working tree containing `root`.
    ///
    /// # Errors
    ///
    /// [`Error::Git`] when `root` is not inside a working tree, and
    /// [`Error::Io`] when the tree cannot be listed at all. Both are errors
    /// rather than an empty gate on purpose: a Gate 1 that silently did not run
    /// reports a candidate as having cleared the never-touch inventory when
    /// nothing checked it, which is §6.20's disarming failure with a reassuring
    /// label on it.
    pub fn build(root: &Path) -> Result<Gate1> {
        let repo = Repo::discover(root)?;
        // Everything is built on the working tree root rather than on `root` as
        // given, so that the three modules agree about what a repo-relative path
        // means. On macOS the two differ by `/private`, and a candidate keyed
        // one way and judged the other silently matches nothing.
        let state = StateGate::survey_in(repo.root(), Some(&repo));
        let content = ContentGate::build(repo.root())?;
        Ok(Gate1 {
            repo,
            state,
            content,
        })
    }

    /// The working tree this gate was built for.
    pub fn root(&self) -> &Path {
        self.repo.root()
    }

    /// The repository handle, for a caller that also needs Gate 0g.
    pub fn repo(&self) -> &Repo {
        &self.repo
    }

    /// Everything the survey could not read.
    ///
    /// Carried out rather than folded into a count, because a gate that surveyed
    /// half a tree and found no effectors looks exactly like one that surveyed
    /// all of it. §6.20 is that sentence.
    pub fn gaps(&self) -> &[String] {
        self.state.scan_gaps()
    }

    /// Ask all sixteen classes about one candidate, in §9.3's order.
    ///
    /// `path` may be absolute or relative to the working tree root.
    ///
    /// # Errors
    ///
    /// Propagates the content and contract gates' own errors — an unreadable
    /// candidate, a path outside the tree, a `git check-ignore` that failed.
    /// None of them degrades to "no objection": a class that could not be
    /// evaluated has not cleared the candidate.
    pub fn judge(&self, path: &Path) -> Result<Gate1Verdict> {
        let mut findings = Vec::new();

        // 1a–1f.
        if let StateVerdict::Ineligible(state) = self.state.judge(path) {
            for finding in state {
                findings.push(Gate1Finding {
                    class: finding.class.code(),
                    title: finding.class.title(),
                    evidence: finding.evidence.describe(),
                });
            }
        }

        // 1g–1k.
        if let ContentVerdict::Ineligible { class, evidence } = self.content.judge(path)? {
            findings.push(Gate1Finding {
                class: class.tag(),
                title: class.label(),
                evidence: evidence.to_string(),
            });
        }

        // 1l–1p.
        let contract = ContractGate::new(&self.repo).classify(path)?;
        if contract.disposition() == Disposition::NeverTouch {
            for reason in contract.reasons() {
                findings.push(Gate1Finding {
                    class: reason.class().code(),
                    title: contract_title(reason),
                    evidence: contract_evidence(reason),
                });
            }
        }

        Ok(Gate1Verdict {
            path: contract.path().to_path_buf(),
            findings,
            type_signal: contract.type_signal(),
        })
    }
}

/// The class name for a 1l–1p refusal, in §9.3's words.
///
/// `ContractClass` carries the code but not the title, so it is written here
/// rather than reached for. One place, so the report and the trace cannot drift.
fn contract_title(reason: &Refusal) -> &'static str {
    match reason {
        Refusal::PlatformContract(_) => "platform contracts",
        Refusal::NegationUnIgnored(_) => "un-ignored by a `!` negation",
        Refusal::ToolArtifact(artifact) => match artifact.kind() {
            judged_core::gate1::contracts::ToolArtifactKind::Ledger => {
                "the keep manifest and the deletion ledger themselves"
            }
            judged_core::gate1::contracts::ToolArtifactKind::Evidence => {
                "the tool's own evidence artifacts"
            }
        },
        Refusal::UnknownType => "the unknown",
    }
}

/// What fired, for a 1l–1p refusal, quoted so it can be re-checked by hand.
fn contract_evidence(reason: &Refusal) -> String {
    match reason {
        Refusal::PlatformContract(contract) => format!(
            "matched `{}`; read by {} — deleting it {}",
            contract.pattern(),
            contract.consumer(),
            contract.effect()
        ),
        Refusal::NegationUnIgnored(negation) => format!(
            "re-included by `{}` at {}:{}",
            negation.pattern(),
            negation.source().display(),
            negation.line()
        ),
        Refusal::ToolArtifact(artifact) => {
            let junk = if artifact.also_canonical_junk() {
                "; it is also on every canonical junk list, which is why the class exists"
            } else {
                ""
            };
            format!(
                "matched `{}` — {}{}",
                artifact.pattern(),
                artifact.what(),
                junk
            )
        }
        Refusal::UnknownType => "no extension, magic signature or path convention determined \
             what this file is, and §9.3's 1p rule is that the unknown defaults to keep"
            .to_string(),
    }
}

// ---------------------------------------------------------------------------
// The trace — what `judged explain` prints
// ---------------------------------------------------------------------------

/// The ignore rule that decided a path, whatever kind of rule it is.
///
/// §9.13 asks `--explain` for *"which `.gitignore` line matched"*, and that
/// question is not the same one [`contracts::ContractGate`] asks. The gate asks
/// only *is this a `!` negation*, because 1m is the class that turns a negation
/// into a refusal, and it answers `None` for the ordinary rule that ignores a
/// build directory. A reader looking at an `IGNORED` path needs the ordinary
/// rule too — it is the line they would edit — so this reports the deciding rule
/// whatever it turns out to be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IgnoreRule {
    /// The ignore file holding the rule, as git names it.
    pub source: PathBuf,
    /// The 1-based line number within [`IgnoreRule::source`].
    pub line: u32,
    /// The rule verbatim, including any leading `!`.
    pub pattern: String,
}

impl IgnoreRule {
    /// Whether this rule *re-includes* the path — §9.3 class 1m.
    pub fn is_negation(&self) -> bool {
        self.pattern.starts_with('!')
    }
}

/// Everything Gate 0g and Gate 1 know about one path.
///
/// Assembled here rather than in the CLI so that the command is a renderer. The
/// ordering of the fields is §9.3's ordering, and that is deliberate: a reader
/// who takes the trace as a model of the pipeline should take away the right
/// one, which begins with recoverability and not with usefulness.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gate1Trace {
    /// The candidate, relative to the working tree root.
    pub path: PathBuf,
    /// Whether the path exists on disk at all.
    pub exists: bool,
    /// **Gate 0g.** What git could give back after we delete it (§8.1).
    pub recoverability: RecoverabilityClass,
    /// The §8.1 ladder rung that class implies.
    pub rung: &'static str,
    /// What the rung means for somebody about to act.
    pub rung_meaning: &'static str,
    /// The ignore rule that decided the path, when one did.
    pub ignore_rule: Option<IgnoreRule>,
    /// The magic-byte signature at the head of the file, when one matched.
    /// §2.1's perfect-portability veto: it reads the file rather than believing
    /// its name.
    pub magic: Option<Magic>,
    /// How the file's type was determined, or `None` — which is class 1p.
    pub type_signal: Option<TypeSignal>,
    /// **Gate 1.** Every class that refused, in §9.3's order. Empty when none
    /// did, which is not permission to delete (§9.1).
    pub findings: Vec<Gate1Finding>,
    /// Everything the survey could not read (§6.20).
    pub gaps: Vec<String>,
}

impl Gate1Trace {
    /// Whether any Gate 1 class refused.
    pub fn is_ineligible(&self) -> bool {
        !self.findings.is_empty()
    }
}

impl Gate1 {
    /// The full Gate 0g + Gate 1 trace for one path.
    ///
    /// # Errors
    ///
    /// As [`Gate1::judge`], plus [`Error::Git`] when the path is outside the
    /// working tree or when git could not answer about its ignore status. A
    /// trace that could not be computed is an error rather than a shorter trace:
    /// §6.20's rule is that a search which did not finish found nothing
    /// *because it did not look*, and the whole value of this command is that a
    /// reader can trust what it does not say.
    pub fn trace(&self, path: &Path) -> Result<Gate1Trace> {
        let verdict = self.judge(path)?;
        let relative = verdict.path().to_path_buf();
        let absolute = self.repo.root().join(&relative);

        let recoverability = self.repo.recoverability(&absolute)?;
        let ignore_rule = deciding_ignore_rule(self.repo.root(), &relative)?;
        let magic = head_bytes(&absolute).and_then(|head| sniff(&head));

        Ok(Gate1Trace {
            exists: absolute.exists(),
            path: relative,
            recoverability,
            rung: rung(recoverability),
            rung_meaning: rung_meaning(recoverability),
            ignore_rule,
            magic,
            type_signal: verdict.type_signal(),
            findings: verdict.findings().to_vec(),
            gaps: self.gaps().to_vec(),
        })
    }
}

/// The head of a file, or `None` when it cannot be read.
///
/// `None` rather than an error because a magic-byte sniff that found nothing and
/// one that could not look are separated by the caller's own report, which says
/// which happened: [`Gate1Trace::exists`] distinguishes them for the case that
/// matters, and the sixteen classes have already refused an unreadable candidate
/// under 1p.
fn head_bytes(path: &Path) -> Option<Vec<u8>> {
    use std::io::Read;
    let mut file = std::fs::File::open(path).ok()?;
    let mut head = vec![0u8; HEAD_BYTES];
    let read = file.read(&mut head).ok()?;
    head.truncate(read);
    Some(head)
}

/// The ignore rule that decided `relative`, by asking git.
///
/// `git check-ignore -vz --no-index --stdin --non-matching`, and every part of
/// that command line is load-bearing for the same reasons
/// [`contracts::ContractGate`] documents on its own call:
///
/// - `--no-index`, because without it git answers from the index for tracked
///   paths and names no pattern at all — and the question would then be
///   unanswerable for exactly the population §6.17 measured.
/// - `--non-matching`, so a path matched by nothing produces a record with an
///   empty pattern rather than no record. "No rule matched" has to stay
///   distinguishable from "git printed nothing because something went wrong"
///   (§6.20).
/// - `-z`, so no filename can be misread whatever it contains.
///
/// # Errors
///
/// [`Error::Git`] when git could not be run or answered with a status other than
/// 0 or 1. Never `Ok(None)` on a failure: that would report "no ignore rule"
/// about a path nothing asked about.
fn deciding_ignore_rule(root: &Path, relative: &Path) -> Result<Option<IgnoreRule>> {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let path = relative.to_str().ok_or_else(|| {
        Error::Git(format!(
            "path {} is not valid UTF-8; refusing to guess its ignore status",
            relative.display()
        ))
    })?;

    let mut child = Command::new("git")
        .current_dir(root)
        .args([
            "check-ignore",
            "-vz",
            "--no-index",
            "--stdin",
            "--non-matching",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|source| Error::Git(format!("could not run `git check-ignore`: {source}")))?;
    {
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| Error::Git("git check-ignore refused a stdin pipe".to_string()))?;
        stdin
            .write_all(path.as_bytes())
            .and_then(|()| stdin.write_all(b"\0"))
            .map_err(|source| {
                Error::Git(format!("could not write to `git check-ignore`: {source}"))
            })?;
    }
    let output = child
        .wait_with_output()
        .map_err(|source| Error::Git(format!("`git check-ignore` did not finish: {source}")))?;

    // Exit 1 is "none of the given paths are ignored" — an answer.
    match output.status.code() {
        Some(0) | Some(1) => {}
        other => {
            return Err(Error::Git(format!(
                "`git check-ignore` on {path} exited with {} : {}",
                other.map_or_else(|| "a signal".to_string(), |code| code.to_string()),
                String::from_utf8_lossy(&output.stderr).trim()
            )))
        }
    }

    // `<source>\0<line>\0<pattern>\0<pathname>\0`, and a non-matching path
    // yields an empty source, a zero line and an empty pattern.
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut fields = stdout.split('\0');
    let (Some(source), Some(line), Some(pattern)) = (fields.next(), fields.next(), fields.next())
    else {
        return Ok(None);
    };
    if source.is_empty() || pattern.is_empty() {
        return Ok(None);
    }
    Ok(Some(IgnoreRule {
        source: PathBuf::from(source),
        line: line.parse().unwrap_or(0),
        pattern: pattern.to_string(),
    }))
}

/// The §8.1 reversibility rung a recoverability class implies.
///
/// Gate 0g computes the class; this is what the class *costs*, and §9.3 is
/// explicit that the ordering is the point — usefulness is irrelevant until
/// recoverability is known, because the cost of being wrong is set by the rung,
/// not the tier. A report that prints the class without the rung has published
/// the cheap half.
pub fn rung(class: RecoverabilityClass) -> &'static str {
    match class {
        RecoverabilityClass::TrackedPushed => "R2–R4",
        RecoverabilityClass::TrackedUnpushed => "R4, local only",
        RecoverabilityClass::Untracked | RecoverabilityClass::Ignored => {
            "R7 at best, R9 by default"
        }
    }
}

/// What the rung means for a reader who is about to act.
pub fn rung_meaning(class: RecoverabilityClass) -> &'static str {
    match class {
        RecoverabilityClass::TrackedPushed => {
            "in the index and on a remote branch: `git checkout <sha>^ -- <path>` restores it, \
             and the restore survives losing this clone"
        }
        RecoverabilityClass::TrackedUnpushed => {
            "committed locally and on no remote: restorable from this clone and from nothing \
             else, so the recovery path dies with the disk"
        }
        RecoverabilityClass::Untracked => {
            "never `git add`-ed: deleting it leaves no blob, no reflog entry and no lost-found. \
             §8.2's promotion (`git add -f`, R9→R6) has to happen BEFORE the mutation, not after"
        }
        RecoverabilityClass::Ignored => {
            "matched by an ignore rule and never `git add`-ed: zero recovery path, and the class \
             §6.17 measured as most likely to hold the only copy of something — .env, a dev \
             SQLite database, terraform.tfstate.backup"
        }
    }
}

// ---------------------------------------------------------------------------
// The layer
// ---------------------------------------------------------------------------

/// One claim Gate 1 refused, with every class that objected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefusedClaim {
    /// The claim, spelled exactly as the analyzer spelled it.
    pub claim: String,
    /// Whether that was a path or a symbol.
    pub kind: ClaimKind,
    /// The first §9.3 class that refused — the one a one-line report quotes.
    pub class: &'static str,
    /// Every class that refused, in §9.3's order, this one included.
    pub findings: Vec<Gate1Finding>,
    /// For a symbol claim, the file the analyzer said declares it — the file
    /// that was actually judged, because Gate 1's classes are properties of
    /// files. `None` for a path claim, and for a symbol the analyzer attributed
    /// to nothing.
    pub declared_in: Option<PathBuf>,
    /// The whole reason, in a sentence somebody can act on.
    pub detail: String,
}

/// What Gate 1 did during one call to the inner SUT.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gate1Run {
    /// The repository the gate ran over.
    pub repo: PathBuf,
    /// Claims the analyzer made.
    pub claimed: usize,
    /// Claims Gate 1 did not refuse.
    pub survived: usize,
    /// Every claim it did refuse, with its conflict list. In the order the
    /// analyzer made them, never sorted by anything that would flatter the layer
    /// (§9.13 invariant 3).
    pub refused: Vec<RefusedClaim>,
    /// How many of [`Gate1Run::refused`] were refused **only** because the
    /// analyzer named no file for them.
    ///
    /// Its own column, and it has to be, because it is the one number that can
    /// turn this layer into a constant function. A tool that attributes nothing
    /// would have every symbol claim refused under 1p and score a perfect
    /// false-removal record by saying nothing at all — the shape §3.7 names for
    /// a positive control that always passes. Every shipped adapter attributes
    /// (vulture prints `path:line:`, deadcode carries a `Position`, knip an
    /// artifact `uri`), so in practice this is small; a report that does not
    /// print it cannot show that.
    pub unattributed: usize,
    /// Everything the survey could not read. A gate that scanned half a tree and
    /// found no effectors looks exactly like one that scanned all of it (§6.20).
    pub gaps: Vec<String>,
}

/// Any [`Sut`], with §9.3's Gate 1 run over every claim it makes — **first**.
///
/// # A pure filter, and nothing else
///
/// The accuser runs, and every claim is checked against the never-touch
/// inventory. Nothing here can add a claim, promote one, or turn a refusal into
/// an accusation — the only operation is dropping, and `tests/gate1_gate.rs`
/// asserts the subset relation on the claim sets rather than on their sizes.
///
/// # Why this is a different layer from the veto and from the root set
///
/// Because it answers a different question, and the difference is the whole of
/// §1.3. Gate 2 asks whether anything *names* the candidate; the root set asks
/// whether it was *declared an entry point*. Both are about usefulness, and a
/// `.env`, a `terraform.tfstate` and an analyst's `.RData` are all genuinely
/// useless to the build — and all irreplaceable. §9.3's word for that is
/// irreversibility, and no amount of usefulness evidence moves it.
///
/// # Failure is a refusal or an error, never a quiet absence
///
/// If the repository cannot be opened, or a class cannot be evaluated, this
/// returns `Err` rather than passing the claims through ungated — the same rule
/// [`crate::sut::VetoedSut`] follows, for the same reason: the report would
/// still say the run was gated.
pub struct Gate1Sut {
    inner: Box<dyn Sut>,
    name: String,
    /// One entry per call to [`Sut::run`], in call order. `RefCell` because
    /// [`Sut::run`] takes `&self` and this wrapper additionally has to record
    /// what it did; single-threaded by construction, since
    /// [`crate::runner::run_suite`] drives one mutant at a time.
    runs: RefCell<Vec<Gate1Run>>,
}

impl Gate1Sut {
    /// `inner`, with every claim checked against the never-touch inventory.
    pub fn new(inner: Box<dyn Sut>) -> Gate1Sut {
        let name = format!("{}+gate1", inner.name());
        Gate1Sut {
            inner,
            name,
            runs: RefCell::new(Vec::new()),
        }
    }

    /// What Gate 1 did, one entry per call to the inner SUT, in call order.
    pub fn runs(&self) -> Vec<Gate1Run> {
        self.runs.borrow().clone()
    }
}

impl Sut for Gate1Sut {
    fn name(&self) -> &str {
        &self.name
    }

    fn cannot_emit(&self) -> Vec<String> {
        self.inner.cannot_emit()
    }

    fn reads(&self) -> Option<&[Ecosystem]> {
        self.inner.reads()
    }

    /// §9.3's ordering, made structural: the accuser runs, Gate 1 judges every
    /// claim, and whatever survives is what any later layer is even shown.
    fn run(&self, repo: &Path) -> Result<SutVerdict> {
        let claims = self.inner.run(repo)?;

        let gate = Gate1::build(repo).map_err(|source| Error::Sut {
            sut: self.name.clone(),
            message: format!(
                "Gate 1 could not be built over the repository at {}: {source}. Refusing to \
                 report an ungated run as a gated one.",
                repo.display()
            ),
        })?;

        let claimed = claims.claimed_dead_paths.len() + claims.claimed_dead_symbols.len();
        let mut refused: Vec<RefusedClaim> = Vec::new();
        let mut unattributed = 0usize;
        let mut claimed_dead_paths: Vec<PathBuf> = Vec::new();
        let mut claimed_dead_symbols: Vec<SymbolClaim> = Vec::new();

        for path in &claims.claimed_dead_paths {
            let verdict = gate.judge(path)?;
            match record(path.display().to_string(), ClaimKind::Path, None, &verdict) {
                Some(entry) => refused.push(entry),
                None => claimed_dead_paths.push(path.clone()),
            }
        }

        for symbol in &claims.claimed_dead_symbols {
            // Gate 1's classes are properties of FILES. A symbol is judged by
            // the file that declares it, which is the right reading rather than
            // a convenience: removing a symbol from a migration edits the
            // migration, and §9.3 refuses the migration either way.
            match symbol.declaration_site() {
                Some(site) => {
                    let verdict = gate.judge(site)?;
                    match record(
                        symbol.name().to_string(),
                        ClaimKind::Symbol,
                        Some(site.to_path_buf()),
                        &verdict,
                    ) {
                        Some(entry) => refused.push(entry),
                        None => claimed_dead_symbols.push(symbol.clone()),
                    }
                }
                // The analyzer named no file, so there is nothing to evaluate
                // sixteen file classes against. §9.3's 1p rule is the answer —
                // the unknown defaults to keep — and this is counted in its own
                // column so a report can show how much of the refusal rate is
                // this one case rather than a class actually firing.
                None => {
                    unattributed += 1;
                    refused.push(RefusedClaim {
                        claim: symbol.name().to_string(),
                        kind: ClaimKind::Symbol,
                        class: "1p",
                        findings: vec![Gate1Finding {
                            class: "1p",
                            title: "the unknown",
                            evidence: "the analyzer named no file for this symbol, so none of \
                                       the sixteen file classes could be evaluated against it"
                                .to_string(),
                        }],
                        declared_in: None,
                        detail: format!(
                            "1p the unknown: `{}` was claimed without a declaration site, so \
                             Gate 1 had nothing to judge. §9.3: the unknown defaults to keep.",
                            symbol.name()
                        ),
                    });
                }
            }
        }

        let survived = claimed_dead_paths.len() + claimed_dead_symbols.len();
        self.runs.borrow_mut().push(Gate1Run {
            repo: gate.root().to_path_buf(),
            claimed,
            survived,
            refused,
            unattributed,
            gaps: gate.gaps().to_vec(),
        });

        Ok(SutVerdict {
            claimed_dead_paths,
            claimed_dead_symbols,
        })
    }
}

/// One refusal, recorded with everything a reader needs to check it — or `None`
/// when no class objected.
fn record(
    claim: String,
    kind: ClaimKind,
    declared_in: Option<PathBuf>,
    verdict: &Gate1Verdict,
) -> Option<RefusedClaim> {
    let first = verdict.findings().first()?;
    // Every class, not just the first. §9.3's refusals stack, and a report that
    // names one of three leaves a reader who resolves it believing the candidate
    // is now eligible.
    let all: Vec<String> = verdict
        .findings()
        .iter()
        .map(|finding| finding.to_string())
        .collect();
    Some(RefusedClaim {
        claim,
        kind,
        class: first.class,
        findings: verdict.findings().to_vec(),
        declared_in,
        detail: all.join("; "),
    })
}
