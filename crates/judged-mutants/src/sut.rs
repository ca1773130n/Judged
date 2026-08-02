//! The system under test, and the two controls the suite needs to be meaningful.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use judged_core::git::Repo;
use judged_core::ledger::{Evidence, Family};
use judged_core::veto::{literal, reachability, recency};
use judged_core::{Error, Result};

use crate::mutant::Ecosystem;

/// What a cleaner claims is dead after looking at a repository.
///
/// There is no field for "confidence" and no field for "score". §9.2 records
/// that the SARIF spec itself warns rank values from different tools "are in
/// general not commensurable"; the suite grades on claims, not on how sure the
/// tool felt.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SutVerdict {
    /// Repo-relative paths the tool says can be removed.
    pub claimed_dead_paths: Vec<PathBuf>,
    /// Symbols the tool says can be removed.
    pub claimed_dead_symbols: Vec<SymbolClaim>,
}

/// One symbol a tool says can be removed, and the file it attributed it to.
///
/// # Why the file is part of the claim
///
/// Gate 2a asks whether an artifact's name occurs anywhere in the repository,
/// and excludes the artifact's own file from the corpus first — a declaration is
/// not a reference to itself. For a path claim the file to exclude is the claim.
/// For a symbol claim it is the file that declares the symbol, and a bare name
/// cannot say which file that is.
///
/// A bare name therefore excludes nothing, every symbol is found in its own
/// declaration, and every symbol claim is rescued. That is safe and it is
/// useless: a veto that fires on every input is a constant function, and a
/// constant function measures nothing. §3.7 makes the point about positive
/// controls — a control that always passes is theatre — and it holds for a gate
/// the same way.
///
/// The information was never missing. Vulture prints `path:line: unused ...`,
/// deadcode carries a `Position`, knip carries an artifact `uri`. Only the type
/// lost it.
///
/// # Why the file is optional
///
/// Because a tool genuinely may not say, and that has to stay distinguishable
/// from *said, and it is this file*. [`SymbolClaim::unattributed`] is the case
/// with no location; see [`UNKNOWN_DEFINING_FILE`] for what Gate 2a does with
/// it and why that direction is the right one **for that case**.
///
/// # What it is not
///
/// Not a claim that the file is dead. `claimed_dead_paths` is where a tool says
/// that, and putting a declaration site there would invent a claim the tool
/// never made — the same error every adapter's `files_touched` exists to avoid.
/// Nor is it graded: [`crate::runner`] matches claims against ground truth by
/// **name**, exactly as before. Provenance is for the veto.
///
/// Fields are private and the two cases have their own constructors, so
/// "attributed to nothing" can only ever be written on purpose.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct SymbolClaim {
    // Ordered before the path, so a set of claims collates by symbol name and
    // the report keeps the order it had when a claim was a bare string.
    name: String,
    declared_in: Option<PathBuf>,
}

impl SymbolClaim {
    /// A symbol the analyzer attributed to a file, repo-relative.
    pub fn declared_in(name: impl Into<String>, file: impl Into<PathBuf>) -> SymbolClaim {
        SymbolClaim {
            name: name.into(),
            declared_in: Some(file.into()),
        }
    }

    /// A symbol the analyzer named without saying where it lives.
    ///
    /// Spelled out rather than reached by passing `None`, because this is the
    /// case that costs a decoy and it should be legible at the call site.
    pub fn unattributed(name: impl Into<String>) -> SymbolClaim {
        SymbolClaim {
            name: name.into(),
            declared_in: None,
        }
    }

    /// The symbol, spelled exactly as the analyzer spelled it.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The file the analyzer said declares it, or `None` when it did not say.
    pub fn declaration_site(&self) -> Option<&Path> {
        self.declared_in.as_deref()
    }

    /// The same claim with its declaration site replaced — `None` to drop one
    /// that cannot be used.
    ///
    /// The name is carried through untouched on purpose: a site that could not
    /// be resolved must never be able to change what was claimed, only how well
    /// the gate can be told where to look.
    pub(crate) fn with_declaration_site(self, file: Option<PathBuf>) -> SymbolClaim {
        SymbolClaim {
            name: self.name,
            declared_in: file,
        }
    }

    /// One claim per distinct name, sorted, keeping a declaration site only
    /// where every claim of that name agreed on one.
    ///
    /// Every adapter needs this and needs it to mean the same thing, so it lives
    /// here rather than three times over.
    ///
    /// # Why one claim per name
    ///
    /// Because that is the contract each adapter already documents and the
    /// reason is unchanged: a tool reports the same name once per file it occurs
    /// in, and a claim list whose length depends on how many copies of a module
    /// a repository happens to hold cannot be diffed between runs. Collapsing
    /// cannot hide a false removal either — grading asks whether a live name was
    /// claimed at all, not how often.
    ///
    /// # Why disagreement drops the site
    ///
    /// Gate 2a excludes the declaring file before searching for references. When
    /// two files both declare `Whatever`, there is no single file to exclude,
    /// and picking one — whichever sorted first, say — would exclude one
    /// declaration and then find the symbol in the other, rescuing on evidence
    /// the harness manufactured by choosing. There is nothing to exclude, which
    /// is what [`SymbolClaim::unattributed`] means, and the conservative
    /// treatment at [`UNKNOWN_DEFINING_FILE`] is right for it in the way it was
    /// never right for the ordinary case.
    ///
    /// A `Some` beside a `None` is disagreement too: one of the tool's findings
    /// located the symbol and another did not, so the harness cannot say the
    /// located file is the only one.
    pub fn dedup_by_name(claims: impl IntoIterator<Item = SymbolClaim>) -> Vec<SymbolClaim> {
        // `Option<Option<PathBuf>>`: the outer layer is "have we seen this name
        // before", the inner one is the site itself, which is legitimately
        // absent. Flattening the two would make the first unattributed claim
        // look like a name never seen.
        let mut by_name: BTreeMap<String, Option<Option<PathBuf>>> = BTreeMap::new();
        for claim in claims {
            let site = claim.declared_in;
            by_name
                .entry(claim.name)
                .and_modify(|agreed| {
                    if *agreed != Some(site.clone()) {
                        *agreed = Some(None);
                    }
                })
                .or_insert(Some(site));
        }
        by_name
            .into_iter()
            .map(|(name, agreed)| SymbolClaim {
                name,
                declared_in: agreed.flatten(),
            })
            .collect()
    }
}

impl fmt::Display for SymbolClaim {
    /// The name alone. A claim is rendered wherever the old bare string was
    /// rendered, and the declaration site is reported as its own field rather
    /// than smuggled into the middle of a sentence.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.name)
    }
}

/// A cleaner the suite can grade.
pub trait Sut {
    /// Name used in [`crate::runner::SuiteReport`].
    fn name(&self) -> &str;

    /// Analyze `repo` and return what it would remove. Implementations must not
    /// mutate `repo` — §9.2: adapters are read-only, the orchestrator owns 100%
    /// of mutations.
    fn run(&self, repo: &Path) -> Result<SutVerdict>;

    /// Finding classes this SUT **structurally cannot** emit, one short phrase
    /// each. §9.2's first non-SARIF clause, the capability envelope: *"every
    /// adapter declares which finding classes it can and structurally cannot
    /// emit — e.g. 'vulture performs global name-set difference and cannot see
    /// cross-module references; its silence is not evidence.' This is what lets
    /// the orchestrator know when silence means anything."*
    ///
    /// The distinction being drawn is between *scanned it and found nothing*
    /// and *never looked*. Both come out of a tool as silence, and silence is
    /// also what a broken tool produces (§6.20), so an undeclared blind spot is
    /// indistinguishable from a clean bill of health.
    ///
    /// The default is empty — "I claim no structural blind spots" — because the
    /// envelope is an assertion the SUT's author makes and nothing else can
    /// make it for them. It is deliberately a list of prose strings and not a
    /// taxonomy: §9.2 asks for a declaration a human can read next to a report,
    /// and an enum would have to be right about every tool in advance.
    fn cannot_emit(&self) -> Vec<String> {
        Vec::new()
    }

    /// The language ecosystems this SUT can load a repository from, or `None`
    /// for a tool that is language-agnostic.
    ///
    /// The coarsest and most consequential entry in the capability envelope
    /// above: an entire language the tool structurally cannot open.
    /// [`crate::runner::run_suite`] intersects this with each mutant's
    /// [`crate::mutant::Mutant::languages`] and **skips** a class with no
    /// overlap — it is not materialized, this SUT is never spawned on it, and
    /// it is recorded as [`crate::runner::Grade::NotRead`].
    ///
    /// # Why the declaration has to exist at all
    ///
    /// A language-specific analyzer handed a repository in another language
    /// does one of two things, and neither can be graded. It refuses — knip
    /// exits 2 with `Unable to find package.json`, cargo-shear 2 with
    /// `could not find Cargo.toml`, deadcode 1 with `packages contain errors` —
    /// and since every one of those codes is shared with a genuine analysis
    /// failure whose stdout is equally empty, none may be declared healthy
    /// (§6.20), so the run aborts. Or it tolerates the directory, finds none of
    /// its own file types, prints nothing and exits 0 — which grades as zero
    /// false removals, a perfect result for a tool that opened no file.
    ///
    /// # The direction each mistake runs in
    ///
    /// Declaring too *little* is not the safe side. A class dropped from the
    /// measurement is a false removal that never gets counted, and the report
    /// loses discriminating power silently. Declaring too *much* re-creates the
    /// abort. Neither is a default anything can pick, which is why this is
    /// `None` unless a SUT says otherwise: unknown competence is not a claim in
    /// either direction, and a tool that declares nothing is measured on
    /// everything.
    ///
    /// `Some(&[])` — "I read no language at all" — is legal and is the abuse
    /// case the runner is built against: it grades nothing, and a report over
    /// zero graded classes is not a clean run (see
    /// [`crate::runner::SuiteReport::graded_count`]).
    ///
    /// [`crate::mutant::Ecosystem::Polyglot`] must never appear here. It names
    /// a class's liveness mechanism, not a toolchain, and no analyzer can be
    /// pointed at it.
    fn reads(&self) -> Option<&[Ecosystem]> {
        None
    }
}

/// A shared SUT is a SUT.
///
/// Rescue layers compose by **ownership**: [`VetoedSut`] takes a
/// `Box<dyn Sut>`, and so does [`crate::roots::RootedSut`]. Stacking two of them
/// therefore hands the inner layer away, and the inner layer is exactly what a
/// report has to interrogate afterwards — each layer records which claims *it*
/// dropped and why, and a report that cannot attribute a rescue to the layer
/// that made it publishes a combined number §11 R1 cannot use.
///
/// So a caller keeps an [`std::rc::Rc`] and hands a clone on. `Rc` rather than
/// `Arc` because the whole path is single-threaded by construction:
/// [`crate::runner::run_suite`] drives one mutant at a time, which is the same
/// reason the layers record their runs in a `RefCell`.
impl<T: Sut + ?Sized> Sut for std::rc::Rc<T> {
    fn name(&self) -> &str {
        (**self).name()
    }

    fn run(&self, repo: &Path) -> Result<SutVerdict> {
        (**self).run(repo)
    }

    fn cannot_emit(&self) -> Vec<String> {
        (**self).cannot_emit()
    }

    fn reads(&self) -> Option<&[Ecosystem]> {
        (**self).reads()
    }
}

/// Turns a tool's stdout into claims, or fails loudly.
///
/// A plain `fn` pointer rather than a boxed closure, because this is the seam
/// that keeps [`CommandSut`] tool-agnostic: everything specific to a given
/// analyzer — its output format, its schema version, what it calls a symbol —
/// lives in an adapter behind this signature, and none of it lives here.
///
/// Returning `Result` is load-bearing. A parser that cannot read what it was
/// given must say so; degrading to an empty [`SutVerdict`] would report the
/// tool as having found nothing.
pub type VerdictParser = fn(&str) -> Result<SutVerdict>;

/// An arbitrary external command, graded as a SUT.
///
/// This is what lets any tool be graded without Judged knowing anything about
/// it: the command is spawned with the materialized fixture repo as its working
/// directory and handed that repo's path as its final argument, and its stdout
/// goes to a caller-supplied [`VerdictParser`].
///
/// # Failure is never silence
///
/// Every way this can go wrong produces an error, never an empty verdict. The
/// reason is arithmetic rather than tidiness: an empty [`SutVerdict`] contains
/// no claims, so [`crate::runner::grade`] finds no false removals in it and
/// scores it zero — a perfect result. A tool that segfaulted on startup would
/// therefore outscore one that did the work. §6.20's rule is that *"no data"
/// must be a distinct state from "zero executions"*, and §9.2 records that
/// vulture, knip, ts-prune, Go deadcode and Periphery all *"conflate 'clean'
/// with 'crashed before doing anything'"* — so the harness cannot inherit the
/// distinction from the tool and has to impose it.
///
/// Concretely, these are errors: the binary cannot be spawned; the process is
/// killed by a signal; it exits with a status not in
/// [`CommandSut::with_success_exit_codes`]; its stdout is not UTF-8; the parser
/// rejects its stdout. Exactly one thing is an empty verdict — a healthy exit
/// whose output the parser read as no claims.
///
/// Note in particular that stdout written *before* a bad exit is discarded
/// rather than parsed. Partial output from a run that died halfway is a short,
/// plausible, wrong answer, which is worse than no answer.
#[derive(Debug)]
pub struct CommandSut {
    name: String,
    program: OsString,
    args: Vec<OsString>,
    parse: VerdictParser,
    success_exit_codes: Vec<i32>,
    cannot_emit: Vec<String>,
    reads: Option<Vec<Ecosystem>>,
}

impl CommandSut {
    /// A SUT that runs `program` and reads its stdout with `parse`.
    ///
    /// `name` is what the report calls it. Only exit code 0 counts as a healthy
    /// run until [`CommandSut::with_success_exit_codes`] says otherwise.
    pub fn new(
        name: impl Into<String>,
        program: impl Into<OsString>,
        parse: VerdictParser,
    ) -> Self {
        CommandSut {
            name: name.into(),
            program: program.into(),
            args: Vec::new(),
            parse,
            success_exit_codes: vec![0],
            cannot_emit: Vec::new(),
            reads: None,
        }
    }

    /// Arguments to pass before the repo path.
    pub fn with_args(mut self, args: impl IntoIterator<Item = impl Into<OsString>>) -> Self {
        self.args = args.into_iter().map(Into::into).collect();
        self
    }

    /// Exit codes that mean the tool ran to completion, replacing the default
    /// `[0]`.
    ///
    /// §9.2: *"adapters compute a health bit; the orchestrator never reads a raw
    /// exit code."* This is that health bit in its cheapest useful form. Ruff is
    /// the model contract there — 0 clean, 1 violations found, 2 abnormal
    /// termination — and a tool of that shape reports findings *by* exiting
    /// non-zero, so a harness that treated every non-zero exit as a crash could
    /// never grade one.
    ///
    /// It is opt-in per SUT and never inferred, because the failure it guards
    /// runs the other way: widening this set is how a genuinely crashed run
    /// gets read as healthy. Declare only codes the tool documents, and note
    /// that a run killed by a signal has no exit code at all and stays an error
    /// whatever is declared here.
    pub fn with_success_exit_codes(mut self, codes: impl IntoIterator<Item = i32>) -> Self {
        self.success_exit_codes = codes.into_iter().collect();
        self
    }

    /// Declare this tool's capability envelope; see [`Sut::cannot_emit`].
    ///
    /// Judged knows nothing about the command it was handed, so it cannot infer
    /// the envelope. Whoever writes the adapter knows, and this is where they
    /// say it.
    pub fn with_cannot_emit(
        mut self,
        classes: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        self.cannot_emit = classes.into_iter().map(Into::into).collect();
        self
    }

    /// Declare the ecosystems this tool can load a repository from; see
    /// [`Sut::reads`].
    ///
    /// Opt-in and never inferred, for the same reason as
    /// [`CommandSut::with_success_exit_codes`]: Judged knows nothing about the
    /// command it was handed, and guessing a language from an argv would be the
    /// harness asserting something the adapter never said. Left unset, the SUT
    /// is measured on the whole catalogue.
    pub fn with_reads(mut self, ecosystems: impl IntoIterator<Item = Ecosystem>) -> Self {
        self.reads = Some(ecosystems.into_iter().collect());
        self
    }

    /// Every failure from this SUT, spelled the same way: attributed to it by
    /// name, so a report never contains an anonymous crash.
    fn fail(&self, message: String) -> Error {
        Error::Sut {
            sut: self.name.clone(),
            message,
        }
    }

    fn program_name(&self) -> String {
        self.program.to_string_lossy().into_owned()
    }

    /// Restate one claimed path in the repo-relative form [`SutVerdict`]
    /// promises, or refuse it.
    ///
    /// A real analyzer echoes back the path it was invoked on, so handing it an
    /// absolute repo path means absolute findings. Left alone, those reach
    /// [`crate::runner::grade`], strip against nothing, intersect no live path,
    /// and the run scores clean — a false removal turned into a pass by a
    /// spelling difference. That is the silent-under-report failure this whole
    /// type exists to prevent, arriving by a different door.
    ///
    /// A path that leaves the repository is refused outright rather than
    /// dropped (§9.3 gate 0c: *"reject any candidate whose realpath is not a
    /// repo descendant"*). Dropping it would erase a tool's claim on something
    /// outside the tree from the report entirely, which is the one place that
    /// claim most needs to appear.
    fn repo_relative(&self, path: PathBuf, root: &Path) -> Result<PathBuf> {
        // Checked before the absolute test, so `/repo/../etc/passwd` is caught
        // too. Purely lexical on purpose: the target need not exist for the
        // claim to be wrong.
        if path
            .components()
            .any(|part| matches!(part, Component::ParentDir))
        {
            return Err(self.fail(format!(
                "`{}` claimed {} is dead, which climbs out of the repository",
                self.program_name(),
                path.display()
            )));
        }

        if path.is_relative() {
            return Ok(path);
        }

        let relative = path.strip_prefix(root).map_err(|_| {
            self.fail(format!(
                "`{}` claimed {} is dead, which is not inside the repository {}",
                self.program_name(),
                path.display(),
                root.display()
            ))
        })?;

        if relative.as_os_str().is_empty() {
            return Err(self.fail(format!(
                "`{}` claimed the repository root {} is dead",
                self.program_name(),
                root.display()
            )));
        }

        Ok(relative.to_path_buf())
    }
}

impl Sut for CommandSut {
    fn name(&self) -> &str {
        &self.name
    }

    fn cannot_emit(&self) -> Vec<String> {
        self.cannot_emit.clone()
    }

    fn reads(&self) -> Option<&[Ecosystem]> {
        self.reads.as_deref()
    }

    fn run(&self, repo: &Path) -> Result<SutVerdict> {
        // Canonicalize once, and use the *same* string for the working
        // directory and the argument. On macOS a temp repo is handed out as
        // `/var/folders/...`, a symlink to `/private/var/folders/...`; a tool
        // whose cwd resolves to one and whose argument is the other emits paths
        // relative to whichever it happened to use. Those do not strip against
        // the root [`crate::runner::grade`] normalizes with, so they match no
        // live path and the run scores clean. Also §9.3 gate 0c: canonicalize
        // paths. Failing here rather than letting the command fail is the
        // difference between "the fixture is not there" and a mute exit code.
        let root = repo.canonicalize().map_err(|source| {
            self.fail(format!(
                "repository {} could not be resolved: {source}",
                repo.display()
            ))
        })?;

        let output = Command::new(&self.program)
            .args(&self.args)
            .arg(&root)
            .current_dir(&root)
            .output()
            .map_err(|source| {
                // Not installed, not executable, not on PATH. The most likely
                // failure of the lot, and the one whose empty-verdict spelling
                // would be most convincing: a tool nobody ran finds nothing.
                self.fail(format!(
                    "could not spawn `{}`: {source}",
                    self.program_name()
                ))
            })?;

        match output.status.code() {
            Some(code) if self.success_exit_codes.contains(&code) => {}
            Some(code) => {
                return Err(self.fail(format!(
                    "`{}` exited with status {code}; declared healthy: {:?}. \
                     Discarding {} bytes of stdout — a run that ended badly did \
                     not finish the analysis, so its partial output is not a \
                     verdict.{}",
                    self.program_name(),
                    self.success_exit_codes,
                    output.stdout.len(),
                    stderr_tail(&output.stderr),
                )));
            }
            None => {
                // No exit code exists at all. This is why the match is on
                // `code()` and not on `success()`: any `unwrap_or` default here
                // silently picks a side for a process that never got to choose.
                return Err(self.fail(format!(
                    "`{}` was killed by a signal before it could exit ({}). \
                     Discarding {} bytes of stdout.{}",
                    self.program_name(),
                    output.status,
                    output.stdout.len(),
                    stderr_tail(&output.stderr),
                )));
            }
        }

        // Not `from_utf8_lossy`. Lossy decoding turns undecodable bytes into
        // replacement characters and hands the parser something that still
        // looks like output, which comes back as a confident, corrupt verdict.
        let stdout = String::from_utf8(output.stdout).map_err(|source| {
            self.fail(format!(
                "stdout of `{}` is not valid UTF-8: {source}",
                self.program_name()
            ))
        })?;

        // Re-attributed to this SUT. The parser is a free function that does not
        // know which SUT it was called for, and an unattributed parse failure in
        // a nineteen-mutant run is not actionable.
        let mut verdict = (self.parse)(&stdout).map_err(|source| {
            self.fail(format!(
                "could not read the output of `{}`: {source}",
                self.program_name()
            ))
        })?;

        // The parser cannot do this itself: it is handed stdout and nothing
        // else, so it has no way to know where the repository is. `CommandSut`
        // is the only place that holds both the root and the claims.
        let claimed = std::mem::take(&mut verdict.claimed_dead_paths);
        verdict.claimed_dead_paths = claimed
            .into_iter()
            .map(|path| self.repo_relative(path, &root))
            .collect::<Result<Vec<_>>>()?;

        // The same re-rooting for the file a symbol was attributed to, and for
        // the same reason: deadcode's `Position.File` is absolute because the go
        // tool resolves it against an absolute directory, and an absolute path
        // strips against nothing.
        //
        // `.ok()` here where the loop above propagates, and the difference is
        // deliberate. A *claimed path* outside the repository is a claim, and
        // §9.3 gate 0c says refuse it — dropping it would erase the tool's
        // riskiest assertion from the report. A *declaration site* outside the
        // repository is not a claim about anything; it is a hint about where to
        // look, and there is nothing there to look at. Degrading it to
        // "unattributed" says exactly what is true — no in-tree file was named —
        // and lands on the conservative branch documented at
        // [`UNKNOWN_DEFINING_FILE`], which can only rescue more. The symbol
        // itself is carried through untouched, so nothing the tool claimed is
        // lost and grading is not affected either way.
        let symbols = std::mem::take(&mut verdict.claimed_dead_symbols);
        verdict.claimed_dead_symbols = symbols
            .into_iter()
            .map(|claim| {
                let site = claim
                    .declaration_site()
                    .and_then(|path| self.repo_relative(path.to_path_buf(), &root).ok());
                claim.with_declaration_site(site)
            })
            .collect();
        Ok(verdict)
    }
}

/// The tail of `stderr`, for the error message, or an empty string.
///
/// Analyzers put the traceback here. One line is enough to tell a missing
/// plugin apart from a syntax error in the fixture, and it is the difference
/// between an actionable failure and "it exited 1".
fn stderr_tail(stderr: &[u8]) -> String {
    /// Bytes of the line to keep: enough for a message and a path, short
    /// enough that a traceback cannot become the error.
    const LIMIT: usize = 300;

    let text = String::from_utf8_lossy(stderr);
    let Some(last) = text.lines().map(str::trim).rfind(|line| !line.is_empty()) else {
        return String::new();
    };

    // Cut on a character boundary. `String::truncate` panics otherwise, and a
    // panic *here* would abort the run while it was reporting someone else's
    // crash — turning an actionable error into a harness failure and losing the
    // reason. Analyzer output is long and is not required to be ASCII, so this
    // is a live path, not a theoretical one.
    let tail = if last.len() <= LIMIT {
        last.to_string()
    } else {
        let cut = (0..=LIMIT)
            .rev()
            .find(|&i| last.is_char_boundary(i))
            .unwrap_or(0);
        format!("{}…", &last[..cut])
    };
    format!(" Last stderr line: {tail}")
}

/// A deliberately bad cleaner: reachability from obvious entry points, nothing
/// else. No grep veto, no config parsing, no framework conventions.
///
/// **This is the suite's own positive control.** §3.7 and §9.8 establish the
/// principle for evidence artifacts — if known-live symbols do not appear,
/// discard the artifact loudly — and the suite needs the same guarantee about
/// itself. `NaiveSut` must FAIL, loudly and on many mutants. **If a naive
/// cleaner ever passes the suite, the suite is theatre** and its green results
/// on a real tool mean nothing.
pub struct NaiveSut;

impl Sut for NaiveSut {
    fn name(&self) -> &str {
        "naive"
    }

    /// The two limits below are structural, not incidental: they follow from
    /// [`PARSED_EXTENSIONS`] and [`ENTRY_STEMS`] and no input can move them.
    /// Both are demonstrated by tests in `tests/runner_suts.rs`; this is the
    /// same facts in the form a report can carry.
    ///
    /// The control's *other* failures — missing a YAML reference, a CI step, a
    /// Dockerfile `COPY` — are deliberately absent from this list. Those are
    /// wrong answers, not silence, and a capability envelope that also listed
    /// them would be excusing them.
    fn cannot_emit(&self) -> Vec<String> {
        vec![
            "symbols declared outside its parsed extensions (py, pyi, ts, tsx, js, jsx, \
             mjs, cjs, rs, go): those files are never scanned for declarations"
                .to_string(),
            "any artifact named main, index, lib, mod, __init__ or __main__: treated as \
             an entry point unconditionally, so it is never claimed"
                .to_string(),
        ]
    }

    fn run(&self, repo: &Path) -> Result<SutVerdict> {
        let candidates = walk(repo)?;

        // The reference corpus, and the whole flaw. §7.5: grahama1970's
        // `SKIP_DIRS` excludes build config from the reference scan, and
        // NickCrew's entire cross-file check is `grep -r "from './FILE'"`. Every
        // surveyed tool decides what counts as a reference by looking only at
        // files it recognizes as code, so a reference from a YAML task list, a
        // CI step, a Dockerfile `COPY`, or an executed README block does not
        // exist as far as the tool is concerned.
        let mut corpus: Vec<(String, String)> = Vec::new();
        for rel in &candidates {
            if !is_parsed_source(rel) {
                continue;
            }
            let path = repo.join(rel);
            let bytes = fs::read(&path).map_err(|source| Error::Io {
                path: path.clone(),
                source,
            })?;
            corpus.push((rel.clone(), String::from_utf8_lossy(&bytes).into_owned()));
        }

        let mut claimed_dead_paths = BTreeSet::new();
        for rel in &candidates {
            if is_entry_point(rel) {
                continue;
            }
            let basename = basename(rel);
            let stem = stem(rel);
            let referenced = corpus.iter().any(|(owner, text)| {
                owner != rel && (text.contains(&basename) || text.contains(&stem))
            });
            if !referenced {
                claimed_dead_paths.insert(PathBuf::from(rel));
            }
        }

        // The same heuristic one level down, which is how `ts-prune` and friends
        // report unused exports: a declared name whose only textual occurrence
        // in the corpus is its own declaration is called dead. This is what
        // makes the control fail the classes whose live artifact is a symbol
        // rather than a file — reflection, link-time registries, ABI exports.
        //
        // The declaring file is carried with each name. Not extra work for its
        // own sake: it is the same fact the real adapters already parse and used
        // to throw away, and the control has to be able to make the claim a real
        // tool makes or it stops being a control for the gate behind it. The
        // first declaration wins, which for anything that survives the filter
        // below is also the only one — a name declared in two files occurs at
        // least twice and is not claimed at all.
        let mut declared: BTreeMap<String, &str> = BTreeMap::new();
        for (owner, text) in &corpus {
            for name in declarations(text) {
                declared.entry(name).or_insert(owner.as_str());
            }
        }
        let claimed_dead_symbols = declared
            .into_iter()
            .filter(|(name, _)| {
                corpus
                    .iter()
                    .map(|(_, text)| text.matches(name.as_str()).count())
                    .sum::<usize>()
                    <= 1
            })
            .map(|(name, owner)| SymbolClaim::declared_in(name, owner))
            .collect();

        Ok(SutVerdict {
            claimed_dead_paths: claimed_dead_paths.into_iter().collect(),
            claimed_dead_symbols,
        })
    }
}

/// Extensions the naive tool recognizes as code. Everything else — YAML, JSON,
/// TOML, Dockerfile, CI workflows, markdown — is invisible to it, both as a
/// reference and, deliberately, not at all as a candidate: §7.5's `rm -rf lib/`
/// and "`package-lock.json` (if regenerable)" show these tools happily removing
/// files they never parse.
const PARSED_EXTENSIONS: &[&str] = &[
    "py", "pyi", "ts", "tsx", "js", "jsx", "mjs", "cjs", "rs", "go",
];

/// Stems every shipped cleaner treats as a root. Without these the control
/// would be a strawman rather than a faithful reproduction of §7.5 — Knip's
/// documented failure mode is *missing* entry points, not having none.
const ENTRY_STEMS: &[&str] = &["main", "index", "lib", "mod", "__init__", "__main__"];

/// Declaration keywords, in the order they are tried. Line-oriented and
/// language-agnostic on purpose: this is the level of rigour the surveyed tools
/// apply, not an accident of implementation.
const DECLARATION_KEYWORDS: &[&str] = &[
    "def ",
    "class ",
    "fn ",
    "func ",
    "function ",
    "struct ",
    "trait ",
    "interface ",
    "enum ",
];

/// Repo-relative, forward-slashed paths of every file under `root`, sorted.
///
/// `.git` is skipped. Packed objects and loose refs contain the names of files
/// that really are dead, and counting history as a live reference would make the
/// control accidentally safe — which would destroy its value as a control.
fn walk(root: &Path) -> Result<Vec<String>> {
    let mut out = Vec::new();
    walk_into(root, "", &mut out)?;
    out.sort();
    Ok(out)
}

fn walk_into(dir: &Path, prefix: &str, out: &mut Vec<String>) -> Result<()> {
    let entries = fs::read_dir(dir).map_err(|source| Error::Io {
        path: dir.to_path_buf(),
        source,
    })?;
    for entry in entries {
        let entry = entry.map_err(|source| Error::Io {
            path: dir.to_path_buf(),
            source,
        })?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == ".git" {
            continue;
        }
        let rel = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        let file_type = entry.file_type().map_err(|source| Error::Io {
            path: entry.path(),
            source,
        })?;
        if file_type.is_dir() {
            walk_into(&entry.path(), &rel, out)?;
        } else {
            out.push(rel);
        }
    }
    Ok(())
}

fn basename(rel: &str) -> String {
    rel.rsplit('/').next().unwrap_or(rel).to_string()
}

fn stem(rel: &str) -> String {
    let base = basename(rel);
    match base.rsplit_once('.') {
        // A leading dot is the whole name, not an extension: `.gitignore` has no
        // stem to strip.
        Some((head, _)) if !head.is_empty() => head.to_string(),
        _ => base,
    }
}

fn is_parsed_source(rel: &str) -> bool {
    match basename(rel).rsplit_once('.') {
        Some((head, ext)) if !head.is_empty() => PARSED_EXTENSIONS.contains(&ext),
        _ => false,
    }
}

fn is_entry_point(rel: &str) -> bool {
    ENTRY_STEMS.contains(&stem(rel).as_str())
}

/// Names declared in `text`, by a line scan that takes the first identifier
/// following the first declaration keyword on each line.
fn declarations(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        for keyword in DECLARATION_KEYWORDS {
            let Some(rest) = after_keyword(line, keyword) else {
                continue;
            };
            if let Some(name) = leading_identifier(rest) {
                out.push(name);
            }
            break;
        }
    }
    out
}

/// The text after the first occurrence of `keyword` that starts on an
/// identifier boundary, so `pub extern "C" fn f` is found but `defer` is not
/// mistaken for `def`.
fn after_keyword<'a>(line: &'a str, keyword: &str) -> Option<&'a str> {
    let mut from = 0;
    while let Some(offset) = line[from..].find(keyword) {
        let at = from + offset;
        let preceded_by_identifier = line[..at]
            .chars()
            .next_back()
            .is_some_and(|c| c.is_alphanumeric() || c == '_');
        if !preceded_by_identifier {
            return Some(&line[at + keyword.len()..]);
        }
        from = at + keyword.len();
    }
    None
}

fn leading_identifier(text: &str) -> Option<String> {
    let mut chars = text.chars();
    let first = chars.next()?;
    if !(first.is_alphabetic() || first == '_') {
        return None;
    }
    let mut name = String::from(first);
    for c in chars {
        if c.is_alphanumeric() || c == '_' {
            name.push(c);
        } else {
            break;
        }
    }
    Some(name)
}

/// A cleaner that claims nothing is ever dead.
///
/// The negative control, and the reason [`crate::mutant::GroundTruth`] carries
/// decoys. This SUT has a perfect false-removal record and is completely
/// useless; a suite that cannot tell it apart from a working tool is measuring
/// nothing. It must fail on decoy recall while passing on false removals.
pub struct RefusingSut;

impl Sut for RefusingSut {
    fn name(&self) -> &str {
        "refusing"
    }

    /// The envelope that makes this control's perfect score readable.
    ///
    /// Zero false removals is the best number the suite can report, and this
    /// SUT earns it by never looking at anything. Declaring total blindness is
    /// what stops that number from being read as competence — which is exactly
    /// the confusion §9.2 introduces the envelope to prevent.
    fn cannot_emit(&self) -> Vec<String> {
        vec![
            "every finding class: this control claims nothing under any circumstances, \
             so its silence is never evidence about any artifact"
                .to_string(),
        ]
    }

    fn run(&self, _repo: &Path) -> Result<SutVerdict> {
        Ok(SutVerdict::default())
    }
}

// ---------------------------------------------------------------------------
// Gate 2, wrapped around an accuser
// ---------------------------------------------------------------------------

/// One of the §9.3 Gate 2 sub-gates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Gate {
    /// 2a — the whole-repo literal search. Meta's BigGrep.
    Literal,
    /// 2b/2c — a manifest names the path, or something enumerates its directory
    /// at runtime.
    Reachability,
    /// 2e — the path was modified recently enough to be work in progress.
    Recency,
}

impl Gate {
    /// Stable lower-case label, for reports.
    pub fn as_str(self) -> &'static str {
        match self {
            Gate::Literal => "literal",
            Gate::Reachability => "reachability",
            Gate::Recency => "recency",
        }
    }
}

impl fmt::Display for Gate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Which sub-gates a [`VetoedSut`] runs.
///
/// [`Gate::Literal`] is **structurally mandatory**: it has no field here, so
/// [`GateSet::includes`] always answers `true` for it and no constructor can
/// remove it. Exactly the shape, and exactly the reason, of
/// `NeedleStrategy`'s treatment of the basename needle: §9.3 makes the
/// whole-repo literal search the floor of Gate 2, and a caller must not be able
/// to disable the gate while appearing to run it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GateSet {
    reachability: bool,
    recency: bool,
}

impl GateSet {
    /// The floor: the whole-repo literal search alone.
    pub const LITERAL_ONLY: GateSet = GateSet {
        reachability: false,
        recency: false,
    };

    /// The two gates whose evidence is the repository's **content**: 2a plus
    /// 2b/2c. The default.
    pub const CONTENT: GateSet = GateSet {
        reachability: true,
        recency: false,
    };

    /// Everything §9.3 Gate 2 lists that this build implements, including 2e.
    ///
    /// Not the default, and the reason is a measurement rather than a
    /// preference. A fixture repository is created, written and committed
    /// seconds before the analyzer is spawned, so the newest commit touching
    /// any path in it is always inside any window — Gate 2e rescues every claim
    /// in every class. That is a true answer about the scratch directory and no
    /// answer at all about the tool, and a suite run through it would report a
    /// veto that prevented everything at the cost of everything. Pinned by
    /// `tests/veto_gate.rs` rather than asserted here.
    pub const ALL: GateSet = GateSet {
        reachability: true,
        recency: true,
    };

    /// Whether `gate` is in this set. Always `true` for [`Gate::Literal`].
    pub fn includes(self, gate: Gate) -> bool {
        match gate {
            Gate::Literal => true,
            Gate::Reachability => self.reachability,
            Gate::Recency => self.recency,
        }
    }

    /// The gates in this set, in §9.3 order, for a report that has to say which
    /// grading it is.
    pub fn gates(self) -> Vec<Gate> {
        [Gate::Literal, Gate::Reachability, Gate::Recency]
            .into_iter()
            .filter(|gate| self.includes(*gate))
            .collect()
    }
}

impl Default for GateSet {
    fn default() -> GateSet {
        GateSet::CONTENT
    }
}

/// Whether a blocked claim was about a file or about a symbol.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaimKind {
    Path,
    Symbol,
}

impl ClaimKind {
    /// Stable lower-case label, for reports.
    pub fn as_str(self) -> &'static str {
        match self {
            ClaimKind::Path => "path",
            ClaimKind::Symbol => "symbol",
        }
    }
}

/// One claim Gate 2 dropped, with the evidence that dropped it.
///
/// §9.13 asks for a conflict list rather than a score, and §7.3 records that the
/// best-validated prior art in the whole document — IntelliJ's Safe Delete —
/// shows the *usage list*, not a probability. A blocked claim that cannot say
/// what fired and where is a score wearing a longer name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockedClaim {
    /// The claim, spelled exactly as the analyzer spelled it.
    pub claim: String,
    /// Whether that was a path or a symbol.
    pub kind: ClaimKind,
    /// Which sub-gate rescued it.
    pub gate: Gate,
    /// The literal that fired, when one did. `None` when the veto came from a
    /// search that did not finish — the §6.20 case, where nothing fired
    /// *because nothing looked*.
    pub needle: Option<String>,
    /// Which derivation that literal came from: `basename`, `stem`,
    /// `parent-dir` or `symbol`.
    pub needle_kind: Option<String>,
    /// The file the evidence was found in, repo-relative.
    pub found_in: Option<PathBuf>,
    /// For a symbol claim, the file the analyzer said declares it — the file
    /// Gate 2a excluded from the corpus before searching. `None` for a path
    /// claim, and for a symbol the analyzer did not attribute to any file.
    ///
    /// Reported beside [`Self::found_in`] because without it the one thing a
    /// reader most needs to check is invisible: whether the rescue is a genuine
    /// cross-file reference or the symbol's own declaration read back at it.
    /// Both cases print a plausible file name, and the difference between them
    /// is the difference between a gate and a constant function. That is what
    /// let a veto that rescued *every* symbol claim survive review.
    pub declared_in: Option<PathBuf>,
    /// The whole reason, in a sentence somebody can act on.
    pub detail: String,
}

/// What Gate 2 did during one call to the inner SUT.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VetoRun {
    /// The repository the gate ran over.
    pub repo: PathBuf,
    /// Claims the analyzer made.
    pub claimed: usize,
    /// Claims that survived Gate 2.
    pub survived: usize,
    /// Every claim that did not, with its evidence. In the order the analyzer
    /// made them, never sorted by anything that would flatter the gate (§9.13
    /// invariant 3).
    pub blocked: Vec<BlockedClaim>,
    /// Claims that survived a Gate 2a search which **completed over the whole
    /// corpus and found nothing** — §9.5's R row *"zero textual occurrences,
    /// complete non-truncated search"*, worth **+1.0 bans**.
    ///
    /// # The type system already encodes the qualifier
    ///
    /// §9.5's row is only earned by a *complete* search, and
    /// [`literal::Verdict::Clear`] is documented as exactly that. Its
    /// counterpart cannot leak in: an incomplete search is a **hit** (§6.20), so
    /// a claim whose scan truncated, errored or timed out was vetoed and is in
    /// [`Self::blocked`] rather than here. There is no path by which a search
    /// that did not finish produces evidence of absence.
    ///
    /// Populated only for claims that survived **every** sub-gate, because a
    /// claim rescued by 2b/2c/2e is rescued and has no accusation left to weigh.
    pub complete_search_survivors: Vec<SurvivingClaim>,
}

/// One claim that survived Gate 2, kept in the shape it was made in.
///
/// **Typed, and that is the fix rather than a tidy-up.** This was a
/// `Vec<String>` with a `evidence_for(&str)` lookup beside it, and paths and
/// symbols were chained into the same list — so a surviving *path* named `foo`
/// answered an evidence query about an unrelated *symbol* `foo`, and attached a
/// +1.0 accusation to a claim Gate 2 had rescued.
///
/// That is the only direction of error this file has ever been able to make
/// that MANUFACTURES a ban rather than withholding one, which is why the lookup
/// is gone entirely rather than given a `ClaimKind` argument: an API that takes
/// a bare string will be called with one.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum SurvivingClaim {
    Path(PathBuf),
    Symbol(SymbolClaim),
}

impl SurvivingClaim {
    /// Whether this was a path or a symbol.
    pub fn kind(&self) -> ClaimKind {
        match self {
            SurvivingClaim::Path(_) => ClaimKind::Path,
            SurvivingClaim::Symbol(_) => ClaimKind::Symbol,
        }
    }

    /// The claim as the analyzer spelled it, for a report.
    pub fn claim(&self) -> String {
        match self {
            SurvivingClaim::Path(path) => path.display().to_string(),
            SurvivingClaim::Symbol(symbol) => symbol.name().to_string(),
        }
    }
}

/// Any [`Sut`], with §9.3's Gate 2 run over every claim it makes.
///
/// This is the shape §9.1 describes and the shape §11 R1 actually asks about:
/// an analyzer orchestrated as a **bounded accuser**, never as an oracle, with
/// a veto layer behind it. Everything the suite measured before this type
/// existed was a bare accuser, and a bare accuser's false-removal count is not
/// an answer to "does any signal combination clear the catalogue".
///
/// # A pure filter, and nothing else
///
/// The ordering is §9.3's: the accuser runs first, and Gate 2 runs on every
/// survivor. Nothing here can add a claim, promote one, or turn a rescue into
/// an accusation — the only operation is dropping, and `tests/veto_gate.rs`
/// asserts the subset relation on the claim sets rather than on their sizes.
///
/// # Failure is a veto or an error, never a quiet absence
///
/// If the repository cannot be opened, this returns `Err` rather than passing
/// the claims through ungated. A gate that silently does not run is worse than
/// no gate: the report still says the run was gated. Inside a gate that did
/// run, every incomplete search — a file that would not read, an enumeration
/// that failed, a budget that expired — is a **hit**, because a search that did
/// not finish found nothing precisely *because it did not look* (§6.20, and the
/// truncated-BigGrep incident in §6.20 that turned Meta's safety net into its
/// deletion trigger).
pub struct VetoedSut {
    inner: Box<dyn Sut>,
    name: String,
    gates: GateSet,
    needles: literal::NeedleStrategy,
    /// One entry per call to [`Sut::run`], in call order.
    ///
    /// `RefCell` because [`Sut::run`] takes `&self`: the trait is shaped for an
    /// analyzer that computes an answer, and this wrapper additionally has to
    /// record what it did. Single-threaded by construction —
    /// [`crate::runner::run_suite`] drives one mutant at a time — so there is
    /// nothing here for a lock to protect.
    runs: RefCell<Vec<VetoRun>>,
}

impl VetoedSut {
    /// `inner`, gated by [`GateSet::CONTENT`].
    pub fn new(inner: Box<dyn Sut>) -> VetoedSut {
        VetoedSut::with_gates(inner, GateSet::default())
    }

    /// `inner`, gated by an explicit set.
    pub fn with_gates(inner: Box<dyn Sut>, gates: GateSet) -> VetoedSut {
        let name = format!("{}+veto", inner.name());
        VetoedSut {
            inner,
            name,
            gates,
            needles: DEFAULT_NEEDLES,
            runs: RefCell::new(Vec::new()),
        }
    }

    /// Which needles Gate 2a derives from a claimed path. See
    /// [`DEFAULT_NEEDLES`].
    pub fn with_needles(mut self, needles: literal::NeedleStrategy) -> VetoedSut {
        self.needles = needles;
        self
    }

    /// The gates in force.
    pub fn gates(&self) -> GateSet {
        self.gates
    }

    /// The needle strategy in force.
    pub fn needles(&self) -> literal::NeedleStrategy {
        self.needles
    }

    /// What Gate 2 did, one entry per call to the inner SUT, in call order.
    pub fn runs(&self) -> Vec<VetoRun> {
        self.runs.borrow().clone()
    }
}

/// Which needles Gate 2a derives from a claimed **path**, by default.
///
/// Basename and stem: the file's own name, with and without its extension.
/// This is the shape Meta's BigGrep has and the shape §0 ranks the
/// second-cheapest high-value safety mechanism in the research.
///
/// The parent-directory needle is left out, and §11 R8 is why. It records two
/// requirements that genuinely conflict — §9.3 says block on any hit, while a
/// usable tool needs a tolerable flag rate — and notes that a parent-directory
/// needle over names like `src`, `app` or `config` blocks nearly everything.
/// A gate that rescues every candidate has the same output as a gate that is
/// not wired in, so leaving it on by default would make the suite unable to
/// measure the trade it exists to measure. It is one call away
/// ([`VetoedSut::with_needles`]) and the report states which strategy produced
/// its numbers.
pub const DEFAULT_NEEDLES: literal::NeedleStrategy = literal::NeedleStrategy::WITH_STEM;

/// The defining file of a claimed symbol, when the analyzer did not say.
///
/// Gate 2a excludes a symbol's defining file from the corpus so that a
/// declaration is not read as a reference to itself. With no location to
/// exclude, this excludes nothing: the empty path matches no tracked file, so
/// the search covers the whole repository including whatever declares the
/// symbol.
///
/// That is the conservative direction and it is the only one available **here**.
/// Excluding nothing can only produce more vetoes than excluding the true
/// declaration, never fewer, and a gate that may only rescue is allowed to be
/// wrong in exactly that direction. What it costs is stated rather than hidden:
/// a symbol that really is dead still occurs once, in its own declaration, so
/// Gate 2a rescues it too and the decoy is lost. That number is a column in the
/// report, not a footnote.
///
/// # This is the fallback, not the rule
///
/// It used to be the rule, and that was a measurement defect rather than a
/// conservative choice. Every symbol claim reached Gate 2a with the empty path,
/// so every symbol was found in its own declaration and every symbol claim was
/// rescued — vulture went from 11 of 16 decoys to 0 of 16, deadcode from 2 of 2
/// to 0 of 2, and both reached "zero false removals, GATE PASSED" by claiming
/// nothing at all. The reasoning above is sound and was applied to a case it
/// does not describe: safety is not the question when the alternative is a
/// constant function, because a veto that fires on every input measures nothing
/// (§3.7 makes the same point about a positive control that always passes).
///
/// The information was there the whole time. Vulture prints `path:line:`,
/// deadcode carries a `Position`, knip carries an artifact `uri`; only
/// [`SutVerdict`] lost it. It does not any more — see [`SymbolClaim`] — and this
/// constant is reached only by [`SymbolClaim::unattributed`], where the analyzer
/// genuinely named no file and there is genuinely nothing to exclude.
const UNKNOWN_DEFINING_FILE: &str = "";

/// Separators an analyzer may use to qualify a symbol name, longest first so
/// that `::` is not split as two `:`.
///
/// The same set [`crate::runner`] matches ground-truth symbols with, and for the
/// same reason: ground truth spells a symbol bare, and a tool spells it however
/// its ecosystem does.
const SYMBOL_SEPARATORS: [&str; 4] = ["::", ".", "/", "#"];

impl VetoedSut {
    /// The needles Gate 2a derives for a claimed symbol: the symbol, plus the
    /// basename of the file that declares it when one is known.
    ///
    /// Built here rather than as a `const` because
    /// [`literal::NeedleStrategy::with`] is not a const fn.
    ///
    /// The basename needle is structurally present —
    /// [`literal::NeedleStrategy`] cannot be asked to leave it out, deliberately
    /// — and it derives to nothing from [`UNKNOWN_DEFINING_FILE`], which is why
    /// this used to be describable as "the symbol alone". With a real
    /// declaration site it derives a real literal, so a symbol is also rescued
    /// when another tracked file names the file it lives in. That is a wider
    /// gate than the name alone and it is the right side to be wide on: the
    /// declaring file is excluded first, so a hit means a different file spells
    /// it, which is evidence about the same artifact and can only add rescues.
    /// The stem is still left out — see [`DEFAULT_NEEDLES`] on why a needle that
    /// blocks nearly everything makes the suite unable to measure anything.
    fn symbol_needles() -> literal::NeedleStrategy {
        literal::NeedleStrategy::BASENAME_ONLY.with(literal::NeedleKind::Symbol)
    }

    /// Gate 2 over one claimed path: `Some` when it was rescued, `None` when it
    /// survived.
    ///
    /// The gates are asked in §9.3's order and the first veto wins, because a
    /// veto is absorbing — no later evidence overrides it, so there is nothing
    /// for a second gate to add once one has fired.
    fn judge_path(
        &self,
        claim: &Path,
        literal_veto: &literal::LiteralVeto<'_>,
        reach: Option<&reachability::Reachability>,
        recency_veto: Option<&recency::RecencyVeto>,
        repo: &Repo,
    ) -> Option<BlockedClaim> {
        let verdict = literal_veto.query(&literal::Candidate::file(claim), self.needles);
        if let Some(record) = from_literal(claim.display().to_string(), ClaimKind::Path, &verdict) {
            return Some(record);
        }

        if let Some(reach) = reach {
            if let reachability::Verdict::Vetoed { reason } = reach.verdict(claim) {
                return Some(from_reachability(
                    claim.display().to_string(),
                    ClaimKind::Path,
                    &reason,
                ));
            }
        }

        if let Some(recency_veto) = recency_veto {
            if let recency::RecencyVerdict::Vetoed(reason) = recency_veto.judge(repo, claim) {
                return Some(BlockedClaim {
                    claim: claim.display().to_string(),
                    kind: ClaimKind::Path,
                    gate: Gate::Recency,
                    needle: None,
                    needle_kind: None,
                    found_in: Some(claim.to_path_buf()),
                    declared_in: None,
                    detail: reason.to_string(),
                });
            }
        }

        None
    }

    /// Gate 2 over one claimed symbol.
    ///
    /// Only Gate 2a runs. 2b, 2c and 2e are path-scoped — a manifest names a
    /// file, a directory is enumerated, a path was committed recently — and a
    /// declaration site is not a claim that the file is dead, so it is not a
    /// path those gates may be asked about. Feeding it to them would convert a
    /// symbol claim into a file claim the analyzer never made, which is the same
    /// invention every adapter's `files_touched` exists to refuse.
    ///
    /// What the declaration site *is* for is the exclusion Gate 2a performs
    /// before it searches: drop the file that declares the symbol, then ask
    /// whether anything else names it. Without that exclusion the declaration
    /// answers the question about itself and the gate rescues unconditionally
    /// — see [`UNKNOWN_DEFINING_FILE`], which is now only the fallback for a
    /// claim the analyzer did not attribute to any file.
    ///
    /// Both spellings are searched: the claim as the tool wrote it, and its
    /// trailing identifier. A tool that qualifies a name has not told us which
    /// spelling occurs in the source — `deadcode` reports `Ledger.Add` for
    /// source that reads `func (l *Ledger) Add()` — so searching only the
    /// qualified form would let a rescue be missed on a spelling difference.
    /// Searching both can only add rescues.
    fn judge_symbol(
        &self,
        claim: &SymbolClaim,
        literal_veto: &literal::LiteralVeto<'_>,
    ) -> Option<BlockedClaim> {
        let name = claim.name();
        let mut spellings = vec![name.to_string()];
        if let Some(tail) = trailing_identifier(name) {
            if tail != name {
                spellings.push(tail);
            }
        }

        // The declaration site the analyzer gave, so Gate 2a can exclude it and
        // ask the question it means to ask: is this symbol named ANYWHERE ELSE.
        // [`UNKNOWN_DEFINING_FILE`] only when the analyzer did not say.
        let declaring = claim
            .declaration_site()
            .map_or_else(|| PathBuf::from(UNKNOWN_DEFINING_FILE), Path::to_path_buf);

        for spelling in spellings {
            let candidate = literal::Candidate::symbol(declaring.clone(), spelling);
            let verdict = literal_veto.query(&candidate, VetoedSut::symbol_needles());
            if let Some(mut record) = from_literal(name.to_string(), ClaimKind::Symbol, &verdict) {
                record.declared_in = claim.declaration_site().map(Path::to_path_buf);
                return Some(record);
            }
        }
        None
    }
}

/// The trailing identifier of a qualified symbol name, or `None` when there is
/// no separator to strip.
fn trailing_identifier(symbol: &str) -> Option<String> {
    let mut best: Option<&str> = None;
    for separator in SYMBOL_SEPARATORS {
        if let Some((_, tail)) = symbol.rsplit_once(separator) {
            // The shortest tail is the one produced by the last separator in the
            // string, whichever separator that was.
            if best.is_none_or(|current| tail.len() < current.len()) {
                best = Some(tail);
            }
        }
    }
    best.filter(|tail| !tail.is_empty()).map(str::to_string)
}

/// A [`BlockedClaim`] from Gate 2a's answer, or `None` when it did not veto.
fn from_literal(
    claim: String,
    kind: ClaimKind,
    verdict: &literal::Verdict,
) -> Option<BlockedClaim> {
    let literal::Verdict::Vetoed { reason, .. } = verdict else {
        return None;
    };
    Some(match reason {
        literal::VetoReason::Reference { first } => {
            let needle = first.needle();
            BlockedClaim {
                claim,
                kind,
                gate: Gate::Literal,
                needle: Some(needle.text().to_string()),
                needle_kind: Some(needle.kind().as_str().to_string()),
                found_in: Some(first.file().to_path_buf()),
                // Filled in by `judge_symbol` for a symbol claim; there is no
                // declaration site to name for a path claim, and none to
                // invent here.
                declared_in: None,
                detail: format!(
                    "{} names it: the {} needle {:?} occurs at byte {}",
                    first.file().display(),
                    needle.kind().as_str(),
                    needle.text(),
                    first.offset()
                ),
            }
        }
        // The §6.20 case, and the one rule that outranks everything else in this
        // layer. Nothing fired, and that is not an absence of references — it is
        // an absence of looking.
        literal::VetoReason::IncompleteSearch { state } => {
            let (file, what) = describe_scan_state(state);
            BlockedClaim {
                claim,
                kind,
                gate: Gate::Literal,
                needle: None,
                needle_kind: None,
                found_in: file,
                declared_in: None,
                detail: format!(
                    "the whole-repo search did not complete ({what}); an incomplete \
                     search is a hit, never an absence (§6.20)"
                ),
            }
        }
    })
}

/// How far a scan got, as a file to blame and a sentence.
fn describe_scan_state(state: &literal::ScanState) -> (Option<PathBuf>, String) {
    match state {
        // Unreachable: `Verdict::Vetoed { IncompleteSearch }` is never built from
        // a completed scan. Spelled out rather than `unreachable!` so that an
        // impossible state is a message instead of a panic (AGENTS.md rule 12).
        literal::ScanState::Completed => (
            None,
            "the scanner reported an incomplete search over a completed scan, \
             which is a bug in Gate 2a"
                .to_string(),
        ),
        literal::ScanState::Truncated {
            file,
            limit_bytes,
            actual_bytes,
        } => (
            Some(file.clone()),
            format!(
                "{} is {actual_bytes} bytes, over the {limit_bytes}-byte per-file \
                 limit, so its contents were never searched",
                file.display()
            ),
        ),
        literal::ScanState::Errored { file, message } => (
            file.clone(),
            match file {
                Some(file) => format!("{} could not be read: {message}", file.display()),
                None => message.clone(),
            },
        ),
        literal::ScanState::TimedOut {
            budget,
            elapsed,
            files_searched,
            files_total,
        } => (
            None,
            format!(
                "the {}ms budget expired after {}ms with {files_searched} of \
                 {files_total} files searched",
                budget.as_millis(),
                elapsed.as_millis()
            ),
        ),
    }
}

/// A [`BlockedClaim`] from Gate 2b/2c's answer.
fn from_reachability(
    claim: String,
    kind: ClaimKind,
    reason: &reachability::VetoReason,
) -> BlockedClaim {
    let (needle, needle_kind, found_in) = match reason {
        reachability::VetoReason::EnumeratedDirectory {
            construct,
            found_in,
            ..
        } => (
            Some(construct.clone()),
            Some("construct".to_string()),
            Some(found_in.clone()),
        ),
        reachability::VetoReason::ManifestPath { manifest, rooted } => (
            Some(rooted.display().to_string()),
            Some("manifest-path".to_string()),
            Some(manifest.clone()),
        ),
        reachability::VetoReason::IncompleteRead { path, .. } => (None, None, Some(path.clone())),
    };
    BlockedClaim {
        claim,
        kind,
        gate: Gate::Reachability,
        needle,
        needle_kind,
        found_in,
        // 2b/2c are path-scoped; `judge_symbol` never reaches them.
        declared_in: None,
        detail: reason.to_string(),
    }
}

impl VetoRun {
    /// §9.5 R-family evidence, paired with the claim that earned it.
    ///
    /// Returns pairs rather than answering a lookup, so evidence is never
    /// separable from the typed claim it belongs to. The previous shape — a
    /// `Vec<String>` of survivors and an `evidence_for(&str)` beside it —
    /// answered a query about a *symbol* with a *path*'s survival whenever the
    /// two shared a spelling, which attached an accusation to a claim Gate 2
    /// had rescued. See [`SurvivingClaim`].
    ///
    /// The +1.0 row is the only one Gate 2a can license: the +1.5 row
    /// additionally requires *"zero dynamism detected"*, which needs the
    /// per-repo dynamism density §2.2 describes and nothing here measures, and
    /// the +0.4/+0.5 rows describe the **analyzer's** depth rather than the
    /// search's completeness.
    ///
    /// So this is the R family's whole contribution in this build, and it is
    /// worth being plain that it is a floor rather than a score: an analyzer
    /// whose own analysis is compiler-index-backed earns nothing extra here,
    /// because the row that would recognise it carries a qualifier nobody
    /// computes.
    pub fn evidence(&self) -> Vec<(SurvivingClaim, Evidence)> {
        self.complete_search_survivors
            .iter()
            .cloned()
            .map(|claim| {
                (
                    claim,
                    Evidence::new(
                        Family::R,
                        "zero textual occurrences, complete non-truncated search",
                        1.0,
                    ),
                )
            })
            .collect()
    }
}

impl Sut for VetoedSut {
    fn name(&self) -> &str {
        &self.name
    }

    fn cannot_emit(&self) -> Vec<String> {
        self.inner.cannot_emit()
    }

    fn reads(&self) -> Option<&[Ecosystem]> {
        self.inner.reads()
    }

    /// §9.3's ordering: the accuser runs first, and Gate 2 runs on **every**
    /// survivor.
    fn run(&self, repo: &Path) -> Result<SutVerdict> {
        let claims = self.inner.run(repo)?;

        // Before any claim is judged. A Gate 2 that cannot open the repository
        // must not hand the claims back untouched: the report would still say
        // the run was gated, and a gate that silently did not run is worse than
        // one that was never added — it is the disarming failure of §6.20 with a
        // reassuring label on it.
        let handle = Repo::discover(repo).map_err(|source| Error::Sut {
            sut: self.name.clone(),
            message: format!(
                "Gate 2 could not open the repository at {}: {source}. Refusing to \
                 report an ungated run as a gated one.",
                repo.display()
            ),
        })?;

        let literal_veto = literal::LiteralVeto::new(&handle);
        // Scanned once per repository, not once per claim: 2b/2c is a single
        // pass whose result is queried per candidate.
        let reach = self
            .gates
            .includes(Gate::Reachability)
            .then(|| reachability::Reachability::scan(handle.root()));
        let recency_veto = self
            .gates
            .includes(Gate::Recency)
            .then(recency::RecencyVeto::default);

        let claimed = claims.claimed_dead_paths.len() + claims.claimed_dead_symbols.len();
        let mut blocked: Vec<BlockedClaim> = Vec::new();
        let mut claimed_dead_paths: Vec<PathBuf> = Vec::new();
        let mut claimed_dead_symbols: Vec<SymbolClaim> = Vec::new();

        for path in &claims.claimed_dead_paths {
            match self.judge_path(
                path,
                &literal_veto,
                reach.as_ref(),
                recency_veto.as_ref(),
                &handle,
            ) {
                Some(record) => blocked.push(record),
                None => claimed_dead_paths.push(path.clone()),
            }
        }
        for symbol in &claims.claimed_dead_symbols {
            match self.judge_symbol(symbol, &literal_veto) {
                Some(record) => blocked.push(record),
                None => claimed_dead_symbols.push(symbol.clone()),
            }
        }

        let survived = claimed_dead_paths.len() + claimed_dead_symbols.len();
        // Every survivor reached here by a Gate 2a search that returned
        // `Verdict::Clear`, which is the complete-corpus, zero-hit case and
        // nothing else — see `VetoRun::complete_search_survivors`.
        let complete_search_survivors: Vec<SurvivingClaim> = claimed_dead_paths
            .iter()
            .cloned()
            .map(SurvivingClaim::Path)
            .chain(
                claimed_dead_symbols
                    .iter()
                    .cloned()
                    .map(SurvivingClaim::Symbol),
            )
            .collect();
        self.runs.borrow_mut().push(VetoRun {
            repo: handle.root().to_path_buf(),
            claimed,
            survived,
            blocked,
            complete_search_survivors,
        });

        Ok(SutVerdict {
            claimed_dead_paths,
            claimed_dead_symbols,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The collapse rule, stated as the three cases that matter.
    ///
    /// The middle one is the whole reason this is not a one-liner: two files
    /// declaring the same name give Gate 2a no single file to exclude, and
    /// keeping either would let the gate find the symbol in the other and
    /// "rescue" it on evidence the harness manufactured by choosing.
    #[test]
    fn a_declaration_site_survives_dedup_only_when_every_claim_agreed() {
        let claims = vec![
            SymbolClaim::declared_in("agreed", "a.py"),
            SymbolClaim::declared_in("agreed", "a.py"),
            SymbolClaim::declared_in("split", "a.py"),
            SymbolClaim::declared_in("split", "b.py"),
            SymbolClaim::declared_in("partly", "a.py"),
            SymbolClaim::unattributed("partly"),
            SymbolClaim::unattributed("never"),
        ];

        let deduped = SymbolClaim::dedup_by_name(claims);

        assert_eq!(
            deduped,
            vec![
                SymbolClaim::declared_in("agreed", "a.py"),
                SymbolClaim::unattributed("never"),
                SymbolClaim::unattributed("partly"),
                SymbolClaim::unattributed("split"),
            ],
            "one claim per name, sorted by name, and a site only where the \
             claims agreed on one"
        );
    }

    /// Order must not decide the answer. `Some` seen before `None` and `None`
    /// seen before `Some` are the same disagreement, and a rule that kept the
    /// first-seen site would make the verdict depend on how the tool happened to
    /// order its output.
    #[test]
    fn disagreement_is_symmetric_in_the_order_the_claims_arrived() {
        let forwards = SymbolClaim::dedup_by_name(vec![
            SymbolClaim::declared_in("x", "a.py"),
            SymbolClaim::unattributed("x"),
        ]);
        let backwards = SymbolClaim::dedup_by_name(vec![
            SymbolClaim::unattributed("x"),
            SymbolClaim::declared_in("x", "a.py"),
        ]);
        assert_eq!(forwards, backwards);
        assert_eq!(forwards, vec![SymbolClaim::unattributed("x")]);
    }
}
