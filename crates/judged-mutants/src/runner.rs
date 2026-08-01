//! Running the suite and grading the result.

use std::collections::BTreeSet;
use std::path::Path;

use crate::mutant::{Ecosystem, GroundTruth, Mutant};
use crate::sut::{Sut, SutVerdict, SymbolClaim};
use judged_core::{Error, Result};

/// What happened to one class under one SUT.
///
/// Three states, not two, and the third is the whole point. §6.20: *"'no data'
/// must be a distinct state from 'zero executions,' and it must never flow into
/// a deadness score."* A class the SUT cannot read produces exactly what a
/// class it read and correctly kept produces — no claims — so without a
/// separate state the two are the same row, and declaring a narrower language
/// set becomes a way to raise a green.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Grade {
    /// Analyzed: zero false removals **and** every decoy found.
    Passed,
    /// Analyzed: a false removal, a missed decoy, or both.
    Failed,
    /// Not analyzed. The SUT declared it cannot read this class's languages
    /// ([`crate::sut::Sut::reads`]), so the repository was never built and the
    /// tool was never spawned. **Not a pass**: nothing was attempted, so there
    /// is nothing to have got right.
    NotRead,
}

/// How one SUT did on one mutant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MutantReport {
    /// Which mutant, e.g. `m07`.
    pub mutant_id: String,
    /// Its ecosystem, carried so a report can be read without the catalogue.
    pub ecosystem: Ecosystem,
    /// Passed, failed, or never attempted. Held as one field rather than as a
    /// `passed` flag beside a `not_read` flag so that "passed and not read"
    /// cannot be spelled at all — that combination is the bug this state exists
    /// to prevent, and a shape that can express it will eventually express it.
    pub grade: Grade,
    /// Live paths and symbols the SUT claimed were dead. Any entry here is a
    /// hard failure — §10 E2: "any 'dead' verdict is a hard failure."
    pub false_removals: Vec<String>,
    /// Genuinely-dead decoy **files** the SUT correctly identified, by either
    /// route: claiming the decoy's path, or claiming a symbol the decoy
    /// defines. A decoy found both ways counts once.
    pub decoys_found: usize,
    /// Decoy **files** planted by this mutant — the denominator of decoy
    /// recall. Not paths plus symbols: the grading rule in `grade` records why
    /// the unit is the file.
    ///
    /// Zero for a [`Grade::NotRead`] class, because the fixture was never
    /// materialized and therefore never declared any ground truth. A skipped
    /// class is in neither the numerator nor the denominator of decoy recall:
    /// scoring a Go tool's silence about a PHP repository's decoys is the same
    /// category error as scoring its silence about their liveness.
    pub decoys_total: usize,
}

impl MutantReport {
    /// Passed means zero false removals **and** at least the decoys the suite
    /// requires. Both halves are necessary; see [`crate::sut::RefusingSut`].
    ///
    /// False for [`Grade::NotRead`] — it was not passed, it was not attempted —
    /// and derived from [`MutantReport::grade`] rather than stored beside it,
    /// so the two cannot drift.
    pub fn passed(&self) -> bool {
        self.grade == Grade::Passed
    }
}

/// How one SUT did on the whole catalogue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuiteReport {
    /// The SUT that was graded.
    pub sut_name: String,
    /// One report per mutant, in catalogue order. Every mutant appears,
    /// including the skipped ones: a class that vanished from the report could
    /// not be told apart from one that was never in the catalogue.
    pub reports: Vec<MutantReport>,
    /// Total false removals across the **graded** classes.
    ///
    /// Kept at the top level because it is the release gate: §10 E2 requires
    /// releases be gated on zero failures, and §11 R1 pre-commits that if this
    /// number is not zero, the auto-act tier is **deleted from the design
    /// rather than tuned**. A number that decides that deserves to be
    /// impossible to miss.
    ///
    /// Skipped classes cannot change it — by construction, since a class that
    /// was never analyzed carries no claims. Which is exactly why it must never
    /// be read on its own: zero here says "nothing was wrong in what was
    /// measured", and [`SuiteReport::graded_count`] is what says how much that
    /// was. A gate that consults only this number certifies a SUT that read
    /// nothing (§6.20).
    pub false_removal_count: usize,
}

impl SuiteReport {
    /// How many classes were actually analyzed — the denominator of everything
    /// else in this report.
    ///
    /// **A gate must require this to be non-zero.** A run over zero graded
    /// classes has a false-removal count of zero, and those are the same bytes
    /// as a clean run. §6.20: no data is not zero findings.
    pub fn graded_count(&self) -> usize {
        self.reports.len() - self.not_read_count()
    }

    /// Graded classes with zero false removals and full decoy recall.
    pub fn passed_count(&self) -> usize {
        self.count(Grade::Passed)
    }

    /// Graded classes that failed on either half.
    pub fn failed_count(&self) -> usize {
        self.count(Grade::Failed)
    }

    /// Classes the SUT declared it cannot read, which were therefore never
    /// built and never handed over.
    ///
    /// Its own column, never folded into passed or failed. §6.20's rule made
    /// arithmetic: if this were added to the passed count, narrowing an
    /// adapter's [`crate::sut::Sut::reads`] would raise a green.
    pub fn not_read_count(&self) -> usize {
        self.count(Grade::NotRead)
    }

    /// All four counts are computed from the rows rather than stored beside
    /// them. A cached total is a second source of truth for the one piece of
    /// arithmetic this whole build exists to keep honest, and the failure it
    /// would allow — a `not_read_count` that disagrees with the rows — is
    /// exactly the "skipped read as passed" bug in a different spelling.
    fn count(&self, grade: Grade) -> usize {
        self.reports.iter().filter(|row| row.grade == grade).count()
    }
}

/// Materialize every mutant the SUT can read, run the SUT over each, and grade
/// it.
///
/// Each graded mutant gets its own throwaway repository; a mutant that fails to
/// materialize is an error, never a skip, because a silently skipped mutant
/// turns into a pass the SUT did not earn.
///
/// A class the SUT declares it cannot read ([`Sut::reads`]) is the one thing
/// that *is* skipped, and it is skipped loudly: [`Grade::NotRead`], counted in
/// [`SuiteReport::not_read_count`] and in no other column. The distinction
/// between that and a pass is the whole of §6.20 — see [`Grade`].
pub fn run_suite(sut: &dyn Sut, mutants: &[Box<dyn Mutant>]) -> Result<SuiteReport> {
    let mut reports = Vec::with_capacity(mutants.len());

    for mutant in mutants {
        // Before the temp directory, before materialization, before the tool.
        // Building a tree the SUT will never be shown is work whose only
        // product is a directory nobody reads — and it would leave the verdict
        // one line of code away from being collected anyway.
        if !reads_mutant(sut, mutant.as_ref()) {
            reports.push(MutantReport {
                mutant_id: mutant.id().to_string(),
                ecosystem: mutant.ecosystem(),
                grade: Grade::NotRead,
                // Empty and zero throughout, and these are not "no findings":
                // they are the absence of a measurement. `Grade::NotRead` is
                // what carries that, which is why it exists rather than being
                // inferred from a row that looks clean.
                false_removals: Vec::new(),
                decoys_found: 0,
                decoys_total: 0,
            });
            continue;
        }

        // `TempDir` is held for exactly this iteration: §10 E2's methodology
        // depends on each mutant exercising *one* mechanism, and a repository
        // reused across mutants would let one mutant's files satisfy another
        // mutant's reference scan.
        // `Builder::prefix`, not `TempDir::new()`, and the prefix is the whole
        // point. `TempDir::new()` names the directory `.tmpXXXXXX` — a HIDDEN
        // directory — and a hidden repository is not a neutral place to run an
        // analyzer from. `go help packages`: "Directory and file names that
        // begin with "." or "_" are ignored by the go tool". Measured
        // 2026-08-01 against deadcode (x/tools v0.48.0) on the m12 fixture: the
        // same tree under `.tmpABC/` produces `deadcode: no packages` and exit
        // 1, and under `tmpABC/` produces the package array and exit 0.
        //
        // The damage that did is the §6.20 shape exactly. deadcode's exit 1 is
        // shared with "this repository has no Go in it", so the harness could
        // not tell "I scanned nothing because you hid the tree from me" from
        // "there is nothing here to scan" — and m12 is the catalogue's only Go
        // class and the one §4.1 predicts deadcode false-removes on. The suite
        // was structurally unable to grade the prediction it exists to test,
        // and nothing in the output said so.
        //
        // Kept non-hidden for every SUT rather than special-cased for Go: a
        // tool that skips the tree reports nothing, nothing is zero false
        // removals, and zero false removals is what clears the §11 R1 gate.
        let repo = tempfile::Builder::new()
            .prefix("judged-e2-")
            .tempdir()
            .map_err(|source| Error::Io {
                path: std::env::temp_dir(),
                source,
            })?;

        // A mutant that cannot build its repository is an error, never a skip.
        // Skipping would drop a row from the report and read as a pass the SUT
        // never earned. Re-wrapped so the id survives whatever the fixture
        // returned.
        let truth = mutant
            .materialize(repo.path())
            .map_err(|e| Error::Fixture {
                mutant_id: mutant.id().to_string(),
                message: e.to_string(),
            })?;

        // Likewise a crashed SUT. Recording it as "claimed nothing" would score
        // a perfect zero false removals — §3.7's signature of every catastrophic
        // failure in this space is an artifact that reports ~0% for everything
        // and is trusted anyway.
        let verdict = sut.run(repo.path()).map_err(|e| Error::Sut {
            sut: sut.name().to_string(),
            message: format!("failed on mutant {}: {}", mutant.id(), e),
        })?;

        reports.push(grade(mutant.as_ref(), &truth, &verdict, repo.path()));

        // Explicit rather than relying on drop, which discards the error. A
        // leaked tree per mutant is nineteen per run.
        let path = repo.path().to_path_buf();
        repo.close().map_err(|source| Error::Io { path, source })?;
    }

    let false_removal_count = reports.iter().map(|r| r.false_removals.len()).sum();
    Ok(SuiteReport {
        sut_name: sut.name().to_string(),
        reports,
        false_removal_count,
    })
}

/// Whether `sut` can load `mutant`'s repository at all — the predicate
/// [`run_suite`] skips on.
///
/// A SUT that declares no language set reads everything; otherwise the class is
/// read when at least one of the languages actually present in its repository
/// ([`Mutant::languages`]) is one the SUT declared ([`Sut::reads`]).
///
/// **Any overlap, not every language.** A polyglot fixture is graded by a tool
/// that can parse one of its halves, because the tool really did open files and
/// really did have an opinion; requiring it to read the whole tree would drop
/// classes it genuinely analyzes, and a dropped class is a false removal that
/// never gets counted. m02 is the concrete case: knip reads its TypeScript half
/// and false-removes a dynamically imported transport there (measured
/// 2026-08-01), which is a finding, not a language mismatch.
///
/// Public because a caller that wants to say *how many* classes this run will
/// skip — the CLI's refusal message does — must ask the same question the same
/// way. A second copy of this predicate could report a different number from
/// the one the runner acted on.
pub fn reads_mutant(sut: &dyn Sut, mutant: &dyn Mutant) -> bool {
    match sut.reads() {
        None => true,
        Some(declared) => mutant
            .languages()
            .iter()
            .any(|language| declared.contains(language)),
    }
}

/// Grade one SUT verdict against one mutant's ground truth.
///
/// Grading never fails and never returns early: §10 E2 wants the whole
/// catalogue's picture, so a false removal on mutant 3 must not hide the state
/// of mutants 4 through 19.
fn grade(
    mutant: &dyn Mutant,
    truth: &GroundTruth,
    verdict: &SutVerdict,
    repo_root: &Path,
) -> MutantReport {
    let claimed_paths: BTreeSet<String> = verdict
        .claimed_dead_paths
        .iter()
        .map(|p| normalize(p, repo_root))
        .collect();
    // By name, and only by name. A claim now carries the file the analyzer
    // attributed it to, and that provenance exists for Gate 2a — which has to
    // exclude a symbol's own declaration before searching for references to it.
    // Grading is a different question: ground truth names a live symbol and asks
    // whether the tool claimed it, and where the tool thought it lived cannot
    // change the answer. Keying on the pair would make a claim about the right
    // symbol in an unexpected file grade clean, which is the silent
    // under-reporting this whole module is built against.
    let claimed_symbols: BTreeSet<&str> = verdict
        .claimed_dead_symbols
        .iter()
        .map(SymbolClaim::name)
        .collect();

    // Ground truth spells symbols bare (`DunningConfig`); real tools spell them
    // however their ecosystem does (`ledger.dunning.DunningConfig`,
    // `badge::render_badge`, `pkg/sampler.drain`). Exact equality would let a
    // SUT delete a live symbol and be graded clean purely because it qualified
    // the name — the gate silently under-reporting the number it exists to
    // report. Matching the trailing segment can only ever find MORE false
    // removals than equality, never fewer, which is the direction a safety gate
    // is allowed to be wrong in.
    fn names_same_symbol(claimed: &str, live: &str) -> bool {
        claimed == live
            || ["::", ".", "/", "#"]
                .iter()
                .any(|sep| claimed.ends_with(&format!("{sep}{live}")))
    }

    let live_paths: BTreeSet<String> = truth
        .live_paths
        .iter()
        .map(|p| normalize(p, repo_root))
        .collect();

    // A `BTreeSet` throughout, so the output order is the collation order of the
    // artifact names and not the iteration order of whatever the SUT built.
    // A report that differs between runs cannot be diffed in CI.
    let mut false_removals: BTreeSet<String> =
        claimed_paths.intersection(&live_paths).cloned().collect();
    for symbol in truth.live_symbols.iter() {
        if claimed_symbols.iter().any(|c| names_same_symbol(c, symbol)) {
            false_removals.insert(symbol.clone());
        }
    }

    // A `Vec` rather than a set, because the symbol route is index-aligned with
    // it. Two identical decoy paths would now be counted twice where the old
    // set silently deduplicated them; `assert_ground_truth_is_on_disk` rejects
    // that, so a fixture author who declares a decoy twice gets a loud failure
    // instead of a quietly smaller denominator.
    let decoy_paths: Vec<String> = truth
        .decoy_dead_paths
        .iter()
        .map(|p| normalize(p, repo_root))
        .collect();

    // Recall is out of decoy **files**, with either route counting as finding
    // one — not out of (paths + symbols).
    //
    // A decoy is a file. A tool that names the only symbol that file defines
    // has found the file, and there is nothing further for it to say. Counting
    // the path and the symbol as two separate things to find would halve the
    // score of every tool that can structurally take only one of the two
    // routes, which is the same category error this whole field exists to
    // repair (§6.20: "no data" is a distinct state from "zero executions") —
    // moved out of the numerator and into the denominator. It would also make
    // the number incomparable between a Python fixture whose decoys have
    // symbols and a CI fixture whose decoys are a bash script and an nginx
    // config.
    //
    // Counting files also means a tool that reports both a dead file and its
    // dead export — knip does exactly this — scores 1, not 2. A recall rate
    // above 100% is a broken instrument, and it would be read as the tool being
    // better than it is.
    let decoys_found = decoy_paths
        .iter()
        .enumerate()
        .filter(|(index, path)| {
            claimed_paths.contains(*path)
                || truth.decoy_dead_symbols.get(*index).is_some_and(|symbol| {
                    // `""` declares that this decoy has no symbol route at all.
                    // Left unguarded, `names_same_symbol` would match it against
                    // a claim of `""`, or of anything ending in a separator, and
                    // credit a find to a claim about nothing. A short
                    // `decoy_dead_symbols` reads the same way, so a fixture that
                    // under-declares loses recall rather than inventing it.
                    !symbol.is_empty()
                        && claimed_symbols.iter().any(|c| names_same_symbol(c, symbol))
                })
        })
        .count();

    MutantReport {
        mutant_id: mutant.id().to_string(),
        ecosystem: mutant.ecosystem(),
        // Both halves are required. Zero false removals alone is the score of a
        // tool that refuses to answer, which is safe and worthless; full decoy
        // recall alone is the score of a tool that deletes the repository.
        grade: if false_removals.is_empty() && decoys_found == decoy_paths.len() {
            Grade::Passed
        } else {
            Grade::Failed
        },
        false_removals: false_removals.into_iter().collect(),
        decoys_found,
        decoys_total: decoy_paths.len(),
    }
}

/// Render an artifact path as the repo-relative, forward-slashed string both
/// sides of the comparison are keyed on.
///
/// `SutVerdict` documents its paths as repo-relative, but `GroundTruth` is
/// produced by a fixture that naturally has `dir` in hand and may return
/// `dir.join(...)`. Comparing those raw yields an empty intersection, which
/// presents as *every mutant passing* — the gate silently disabled rather than
/// noisily broken. Normalizing both sides is the cheap defence.
fn normalize(path: &Path, repo_root: &Path) -> String {
    let relative = path.strip_prefix(repo_root).unwrap_or(path);
    relative
        .components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}
