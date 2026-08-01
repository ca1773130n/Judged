//! The system under test, and the two controls the suite needs to be meaningful.

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

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
    pub claimed_dead_symbols: Vec<String>,
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
        let mut declared: BTreeSet<String> = BTreeSet::new();
        for (_, text) in &corpus {
            declared.extend(declarations(text));
        }
        let claimed_dead_symbols = declared
            .into_iter()
            .filter(|name| {
                corpus
                    .iter()
                    .map(|(_, text)| text.matches(name.as_str()).count())
                    .sum::<usize>()
                    <= 1
            })
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
