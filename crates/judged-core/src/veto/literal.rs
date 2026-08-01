//! Gate 2a — the whole-repo literal veto (§9.3).
//!
//! Aho-Corasick the basename, the stem, the parent directory name and the
//! symbol name, **as raw bytes**, across **every tracked file**: source, YAML,
//! TOML, JSON, HCL, Dockerfile, Makefile, `.github/workflows`, SQL, shell,
//! markdown, i18n bundles, `.env.example`, agent-context files, and binaries.
//! Any hit is a VETO.
//!
//! Binaries are in the corpus on purpose. Path and symbol strings survive
//! compilation, pickling and serialization, so a `.pkl`, a `.so` or a compiled
//! template is frequently the *only* place a live class name still appears
//! (E2 class m16 is exactly this shape). A text-only scanner reads those files
//! as empty and reports the class dead.
//!
//! # The two directions are not symmetric
//!
//! §2.1 puts it plainly: a hit is a **mandatory veto**; a *complete,
//! non-truncated, zero-hit* search is at most a weak accusation; and a search
//! that was truncated, timed out, errored or ran over an incomplete file set
//! must **abstain, never accuse**. Those are separate code paths here, and
//! [`Verdict`] has no third value: either the search provably covered the whole
//! corpus and found nothing ([`Verdict::Clear`]), or the candidate is vetoed.
//! There is deliberately no `bool` anywhere on this boundary, because "searched
//! and found nothing" and "did not finish" are the two states §6.20 records
//! Meta conflating — a truncated BigGrep read as "no references" turns the
//! safety net into the deletion trigger.
//!
//! # Self-reference
//!
//! A file always contains its own name, and a symbol always appears at its own
//! definition site. Counting either would make the veto a constant function and
//! nothing would ever be removable. So the candidate's own file is **excluded
//! from the corpus** — for a path candidate that is the file itself, for a
//! symbol candidate its defining file. The consequence is deliberate and worth
//! stating: a use of the symbol from elsewhere *inside its own defining file*
//! is invisible to this gate. That is a reachability question (Gate 2c), not a
//! literal one, and this layer does not parse.
//!
//! # Known miss: names that are never contiguous (§6.2)
//!
//! `await import(`./transports/${kind}Transport.js`)` assembles a specifier at
//! runtime, so the stem `websocketTransport` may appear nowhere in the
//! repository as a contiguous literal (E2 class m02). No amount of literal
//! searching fixes that, and this module does **not** add prefix or suffix
//! matching to paper over it: §6.2 sketches a prefix-tree remedy modelled on
//! Meta's BigGrep query design, and that is a later decision. What this module
//! does instead is *measure* the gap — the parent-directory needle happens to
//! catch that specific mutant, because `transports/` is the static prefix of the
//! computed specifier, and `tests/veto_literal.rs` records exactly that.

use std::collections::HashSet;
use std::fs::File;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use aho_corasick::{AhoCorasick, MatchKind};

use crate::git::Repo;

/// Environment variables that would redirect the corpus enumeration at a
/// different repository. Same list, and same reason, as `git::INHERITED_GIT_ENV`
/// — a veto computed over the wrong repository is a confident wrong answer.
const INHERITED_GIT_ENV: [&str; 6] = [
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_COMMON_DIR",
    "GIT_NAMESPACE",
];

/// Which derivation a needle came from.
///
/// Carried on every [`Hit`] so a report can say *which* needle fired and *where*
/// (§9.13 asks for a conflict list a human can read, not a score) and so the
/// fire rate of each derivation can be measured rather than guessed (§11 R8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum NeedleKind {
    /// The file name with its extension: `dunning.py`.
    Basename,
    /// The file name without its final extension: `dunning`.
    Stem,
    /// The name of the directory the file sits in: `ledger`.
    ParentDir,
    /// The candidate symbol's own name.
    Symbol,
}

impl NeedleKind {
    /// Stable lower-case label, for reports and baselines.
    pub fn as_str(self) -> &'static str {
        match self {
            NeedleKind::Basename => "basename",
            NeedleKind::Stem => "stem",
            NeedleKind::ParentDir => "parent-dir",
            NeedleKind::Symbol => "symbol",
        }
    }
}

/// Which needles a query derives.
///
/// §11 R8 records two requirements that conflict: §9.3 says "block on any hit",
/// while a usable tool needs a tolerable flag rate — and a parent-directory
/// needle over names like `src`, `app` or `config` blocks nearly everything.
/// That is not resolvable by argument, only by measurement, so the strategy is
/// an explicit, selectable set and every query reports which needles it used and
/// which of them fired.
///
/// [`NeedleKind::Basename`] is **structurally mandatory**: it has no field here,
/// so [`NeedleStrategy::includes`] always answers `true` for it and
/// [`NeedleStrategy::without`] cannot remove it. §9.3 makes the basename the
/// floor of Gate 2a, and a caller must not be able to disable the gate while
/// appearing to run it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NeedleStrategy {
    stem: bool,
    parent_dir: bool,
    symbol: bool,
}

impl NeedleStrategy {
    /// The narrowest legal strategy: the basename alone.
    pub const BASENAME_ONLY: NeedleStrategy = NeedleStrategy {
        stem: false,
        parent_dir: false,
        symbol: false,
    };
    /// Basename plus the extension-stripped stem.
    pub const WITH_STEM: NeedleStrategy = NeedleStrategy {
        stem: true,
        parent_dir: false,
        symbol: false,
    };
    /// Basename, stem and the parent directory name. The widest *path-derived*
    /// strategy, and the one §11 R8 expects to dominate the flag rate.
    pub const WITH_PARENT_DIR: NeedleStrategy = NeedleStrategy {
        stem: true,
        parent_dir: true,
        symbol: false,
    };
    /// Everything §9.3 lists. The default, because Gate 2 may only rescue.
    pub const MAXIMAL: NeedleStrategy = NeedleStrategy {
        stem: true,
        parent_dir: true,
        symbol: true,
    };

    /// Whether `kind` is part of this set. Always `true` for
    /// [`NeedleKind::Basename`].
    pub fn includes(self, kind: NeedleKind) -> bool {
        match kind {
            NeedleKind::Basename => true,
            NeedleKind::Stem => self.stem,
            NeedleKind::ParentDir => self.parent_dir,
            NeedleKind::Symbol => self.symbol,
        }
    }

    /// This set plus `kind`.
    pub fn with(self, kind: NeedleKind) -> NeedleStrategy {
        self.set(kind, true)
    }

    /// This set minus `kind`. A no-op for [`NeedleKind::Basename`].
    pub fn without(self, kind: NeedleKind) -> NeedleStrategy {
        self.set(kind, false)
    }

    fn set(mut self, kind: NeedleKind, on: bool) -> NeedleStrategy {
        match kind {
            // Not a silent no-op by accident: see the type-level note above.
            NeedleKind::Basename => {}
            NeedleKind::Stem => self.stem = on,
            NeedleKind::ParentDir => self.parent_dir = on,
            NeedleKind::Symbol => self.symbol = on,
        }
        self
    }
}

impl Default for NeedleStrategy {
    fn default() -> NeedleStrategy {
        NeedleStrategy::MAXIMAL
    }
}

/// What is being asked about: a repo-relative path, optionally with the name of
/// a symbol defined in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    path: PathBuf,
    symbol: Option<String>,
}

impl Candidate {
    /// A whole file, named by its path **relative to the working tree root**.
    /// An absolute path inside the tree is accepted and made relative.
    pub fn file(path: impl Into<PathBuf>) -> Candidate {
        Candidate {
            path: path.into(),
            symbol: None,
        }
    }

    /// A symbol, together with the file that defines it. The defining file is
    /// what gets excluded from the corpus (see the module docs on
    /// self-reference); it is not itself the thing being judged.
    pub fn symbol(defining_file: impl Into<PathBuf>, name: impl Into<String>) -> Candidate {
        Candidate {
            path: defining_file.into(),
            symbol: Some(name.into()),
        }
    }

    /// The candidate path, or for a symbol candidate its defining file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The symbol name, when this is a symbol candidate.
    pub fn symbol_name(&self) -> Option<&str> {
        self.symbol.as_deref()
    }
}

/// One literal that was searched for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Needle {
    kind: NeedleKind,
    text: String,
}

impl Needle {
    /// Which derivation produced this needle.
    pub fn kind(&self) -> NeedleKind {
        self.kind
    }

    /// The literal itself.
    pub fn text(&self) -> &str {
        &self.text
    }
}

/// One occurrence of one needle in one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    file: PathBuf,
    needle: Needle,
    offset: usize,
}

impl Hit {
    /// The tracked file, relative to the working tree root.
    pub fn file(&self) -> &Path {
        &self.file
    }

    /// Which needle fired.
    pub fn needle(&self) -> &Needle {
        &self.needle
    }

    /// Byte offset of the match within that file's bytes. A byte offset rather
    /// than a line number because half the corpus is binary and has no lines.
    pub fn offset(&self) -> usize {
        self.offset
    }
}

/// How far the search actually got.
///
/// **Only [`ScanState::Completed`] licenses a "no veto" answer.** Every other
/// state is a hit (§9.3, §6.20). A search that did not finish found nothing
/// *because it did not look*.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScanState {
    /// Every tracked file was enumerated and read in full.
    Completed,
    /// A tracked file was larger than the per-file byte limit, so its contents
    /// were not searched. Meta's BigGrep truncates for the same reason and this
    /// is the state that must never be read as an absence.
    Truncated {
        file: PathBuf,
        limit_bytes: u64,
        actual_bytes: u64,
    },
    /// A tracked file could not be read, or the corpus could not be enumerated
    /// at all. `file` is `None` when the failure was not attributable to one
    /// file (enumeration failed, or no needle could be derived).
    Errored {
        file: Option<PathBuf>,
        message: String,
    },
    /// The time budget ran out before the corpus was covered.
    TimedOut {
        budget: Duration,
        elapsed: Duration,
        files_searched: usize,
        files_total: usize,
    },
}

impl ScanState {
    /// Whether the search provably covered the whole corpus.
    pub fn is_complete(&self) -> bool {
        matches!(self, ScanState::Completed)
    }
}

/// Everything the query looked at and everything it found.
///
/// Its fields are private and it has no public constructor, so a [`Verdict`]
/// cannot be fabricated without a report that the scanner actually produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanReport {
    needles: Vec<Needle>,
    hits: Vec<Hit>,
    hits_capped: bool,
    files_searched: usize,
    files_total: usize,
    bytes_searched: u64,
    elapsed: Duration,
    state: ScanState,
}

impl ScanReport {
    /// Every literal that was searched for, in derivation order.
    pub fn needles(&self) -> &[Needle] {
        &self.needles
    }

    /// Every recorded occurrence: at most one per (file, needle) pair, capped at
    /// [`ScanLimits::max_hits`]. This is the conflict list §9.13 asks for.
    pub fn hits(&self) -> &[Hit] {
        &self.hits
    }

    /// Whether more hits existed than were recorded. Affects the *report* only,
    /// never the verdict — the scan itself still covered the whole corpus.
    pub fn hits_capped(&self) -> bool {
        self.hits_capped
    }

    /// Tracked files whose bytes were actually searched.
    pub fn files_searched(&self) -> usize {
        self.files_searched
    }

    /// Tracked files in the corpus, after excluding the candidate itself.
    pub fn files_total(&self) -> usize {
        self.files_total
    }

    /// Bytes fed to the automaton. The denominator for a fire-rate measurement.
    pub fn bytes_searched(&self) -> u64 {
        self.bytes_searched
    }

    /// Wall-clock time the scan took.
    pub fn elapsed(&self) -> Duration {
        self.elapsed
    }

    /// How far the search got. See [`ScanState`].
    pub fn state(&self) -> &ScanState {
        &self.state
    }

    /// The distinct needle kinds that fired, sorted. For §11 R8 measurement.
    pub fn kinds_fired(&self) -> Vec<NeedleKind> {
        let mut kinds: Vec<NeedleKind> = self.hits.iter().map(|hit| hit.needle.kind).collect();
        kinds.sort_unstable();
        kinds.dedup();
        kinds
    }
}

/// Why a candidate was vetoed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VetoReason {
    /// A needle occurred somewhere that is not the candidate itself. `first` is
    /// the headline occurrence; the full list is on the report.
    Reference { first: Hit },
    /// The search did not provably cover the corpus, so its silence proves
    /// nothing (§6.20).
    IncompleteSearch { state: ScanState },
}

/// The answer. There is no third value and no `bool`.
///
/// A veto can only ever **rescue**. Nothing in this module can produce evidence
/// that a candidate is dead; the strongest thing [`Verdict::Clear`] says is
/// "this search covered every tracked file and none of them names it", which
/// §2.1 rates a *weak* accusation and which some other layer is free to
/// discard.
///
/// Both variants are `#[non_exhaustive]`, so [`decide`] is the only thing
/// anywhere that can produce a `Verdict`. Outside this module a `Clear` cannot
/// be assembled from a vetoed candidate's report — the mapping from evidence to
/// answer exists in exactly one place and cannot be routed around.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use]
pub enum Verdict {
    /// Rescued. Absorbing: no later evidence overrides it.
    #[non_exhaustive]
    Vetoed {
        reason: VetoReason,
        report: ScanReport,
    },
    /// The search **completed over the whole corpus** and found nothing.
    #[non_exhaustive]
    Clear { report: ScanReport },
}

impl Verdict {
    /// Whether the candidate was rescued.
    pub fn is_veto(&self) -> bool {
        matches!(self, Verdict::Vetoed { .. })
    }

    /// Why, when it was.
    pub fn reason(&self) -> Option<&VetoReason> {
        match self {
            Verdict::Vetoed { reason, .. } => Some(reason),
            Verdict::Clear { .. } => None,
        }
    }

    /// What the search looked at, either way.
    pub fn report(&self) -> &ScanReport {
        match self {
            Verdict::Vetoed { report, .. } | Verdict::Clear { report } => report,
        }
    }
}

/// Bounds on the search. Every bound, when reached, produces a **veto** rather
/// than a shortened answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanLimits {
    /// Refuse to read a tracked file larger than this, and record
    /// [`ScanState::Truncated`]. Bounds memory without ever letting a large file
    /// be silently skipped. `None` means read anything.
    pub max_file_bytes: Option<u64>,
    /// Wall-clock budget for the whole scan. `None` means no deadline.
    pub budget: Option<Duration>,
    /// How many hits to record. A report-size bound only; the scan still covers
    /// the whole corpus and the verdict is unaffected.
    pub max_hits: usize,
}

impl Default for ScanLimits {
    fn default() -> ScanLimits {
        ScanLimits {
            // 64 MiB is well above any hand-written file and above most build
            // artifacts, so in practice this bounds memory rather than the
            // answer — but when it does bite, it vetoes.
            max_file_bytes: Some(64 * 1024 * 1024),
            budget: None,
            max_hits: 32,
        }
    }
}

/// A tracked path, with the index mode that says how to read it.
struct TrackedEntry {
    path: PathBuf,
    kind: EntryKind,
}

#[derive(PartialEq, Eq)]
enum EntryKind {
    /// A regular file: read its bytes.
    Blob,
    /// A symlink: its tracked content is the target *string*, which is what a
    /// reference would be written into. Following it instead would search some
    /// other file's bytes twice and would fail outright on a dangling link.
    Symlink,
    /// A submodule gitlink. §9.3 Gate 0b refuses to descend into nested
    /// repositories, so it is not part of this repository's corpus.
    Gitlink,
}

/// Gate 2a over one repository.
pub struct LiteralVeto<'a> {
    repo: &'a Repo,
    limits: ScanLimits,
}

impl<'a> LiteralVeto<'a> {
    /// Gate 2a with [`ScanLimits::default`].
    pub fn new(repo: &'a Repo) -> LiteralVeto<'a> {
        LiteralVeto::with_limits(repo, ScanLimits::default())
    }

    /// Gate 2a with explicit bounds.
    pub fn with_limits(repo: &'a Repo, limits: ScanLimits) -> LiteralVeto<'a> {
        LiteralVeto { repo, limits }
    }

    /// The bounds in force.
    pub fn limits(&self) -> &ScanLimits {
        &self.limits
    }

    /// Ask whether anything in the repository names `candidate`.
    ///
    /// Infallible **by design**: there is no `Result` here because every way
    /// this can fail — git will not enumerate the index, a file will not open,
    /// the budget expires, no needle can be derived — is itself a verdict, and
    /// that verdict is always VETO. A `Result` would hand the caller an error
    /// they could `unwrap_or(false)`, and that single line is the §6.20
    /// inversion in miniature.
    ///
    /// Costs one `git ls-files` and one full read of the corpus **per call**, so
    /// asking about N candidates re-reads the repository N times. That is
    /// correct and, for a repository of a few thousand files, fast enough; it is
    /// not what a monorepo needs. The batched shape is one automaton built over
    /// every candidate's needles and a single pass over the corpus, attributing
    /// each match back to its candidate — a different entry point, not a
    /// different answer, so it can be added without disturbing this one.
    pub fn query(&self, candidate: &Candidate, strategy: NeedleStrategy) -> Verdict {
        let started = Instant::now();

        let rel = match self.candidate_rel(candidate) {
            Ok(rel) => rel,
            Err(message) => return self.abort(Vec::new(), started, None, message),
        };
        let needles = derive_needles(&rel, candidate.symbol_name(), strategy);
        if needles.is_empty() {
            // Searching for nothing finds nothing, and that is not an absence.
            return self.abort(
                needles,
                started,
                None,
                format!(
                    "no needle could be derived from {} under {strategy:?}",
                    rel.display()
                ),
            );
        }
        let automaton = match AhoCorasick::builder()
            // Leftmost-longest so that a file spelling the whole basename is
            // reported as a basename hit rather than as the stem nested inside
            // it. The verdict is identical either way; the report is not.
            .match_kind(MatchKind::LeftmostLongest)
            .build(needles.iter().map(|needle| needle.text.as_bytes()))
        {
            Ok(automaton) => automaton,
            Err(source) => {
                return self.abort(
                    needles,
                    started,
                    None,
                    format!("could not build the search automaton: {source}"),
                )
            }
        };

        let corpus = match tracked_files(self.repo.root()) {
            Ok(corpus) => corpus,
            Err(message) => return self.abort(needles, started, None, message),
        };

        self.scan(&rel, needles, &automaton, corpus, started)
    }

    /// Walk the corpus. The only place a [`ScanReport`] is built.
    fn scan(
        &self,
        candidate_rel: &Path,
        needles: Vec<Needle>,
        automaton: &AhoCorasick,
        corpus: Vec<TrackedEntry>,
        started: Instant,
    ) -> Verdict {
        let corpus: Vec<TrackedEntry> = corpus
            .into_iter()
            // Self-reference: the candidate's own file is not evidence about
            // the candidate. Gitlinks are a different repository (Gate 0b).
            .filter(|entry| entry.kind != EntryKind::Gitlink && entry.path != candidate_rel)
            .collect();
        let files_total = corpus.len();

        let mut hits: Vec<Hit> = Vec::new();
        let mut hits_capped = false;
        let mut files_searched = 0usize;
        let mut bytes_searched = 0u64;
        // The first way in which the search fell short of covering the corpus.
        // Kept even when a hit is also found, so that `Completed` is never
        // claimed for a scan that was not.
        let mut shortfall: Option<ScanState> = None;

        for entry in &corpus {
            if let Some(budget) = self.limits.budget {
                let elapsed = started.elapsed();
                if elapsed >= budget {
                    // A deadline is the one shortfall worth stopping for: every
                    // further file would only miss it again.
                    shortfall.get_or_insert(ScanState::TimedOut {
                        budget,
                        elapsed,
                        files_searched,
                        files_total,
                    });
                    break;
                }
            }

            let bytes = match self.read_entry(entry) {
                Ok(bytes) => bytes,
                Err(state) => {
                    shortfall.get_or_insert(state);
                    continue;
                }
            };
            files_searched += 1;
            bytes_searched += bytes.len() as u64;

            // One hit per distinct needle per file: enough to say what fired and
            // where, without walking a 60 MB binary once per occurrence.
            let mut seen = vec![false; needles.len()];
            let mut seen_count = 0usize;
            for m in automaton.find_iter(&bytes) {
                let index = m.pattern().as_usize();
                if seen[index] {
                    continue;
                }
                seen[index] = true;
                seen_count += 1;
                if hits.len() < self.limits.max_hits {
                    hits.push(Hit {
                        file: entry.path.clone(),
                        needle: needles[index].clone(),
                        offset: m.start(),
                    });
                } else {
                    hits_capped = true;
                }
                if seen_count == needles.len() {
                    break;
                }
            }
        }

        decide(ScanReport {
            needles,
            hits,
            hits_capped,
            files_searched,
            files_total,
            bytes_searched,
            elapsed: started.elapsed(),
            state: shortfall.unwrap_or(ScanState::Completed),
        })
    }

    /// The bytes of one tracked entry, or the state that reading it produced.
    fn read_entry(&self, entry: &TrackedEntry) -> Result<Vec<u8>, ScanState> {
        let absolute = self.repo.root().join(&entry.path);
        let errored = |message: String| ScanState::Errored {
            file: Some(entry.path.clone()),
            message,
        };

        if entry.kind == EntryKind::Symlink {
            let target = std::fs::read_link(&absolute)
                .map_err(|source| errored(format!("could not read symlink: {source}")))?;
            return Ok(target.as_os_str().as_encoded_bytes().to_vec());
        }

        let metadata = std::fs::metadata(&absolute)
            .map_err(|source| errored(format!("could not stat tracked file: {source}")))?;
        if metadata.is_dir() {
            return Err(errored(
                "tracked as a file but present as a directory".to_string(),
            ));
        }
        if let Some(limit) = self.limits.max_file_bytes {
            if metadata.len() > limit {
                return Err(ScanState::Truncated {
                    file: entry.path.clone(),
                    limit_bytes: limit,
                    actual_bytes: metadata.len(),
                });
            }
        }
        // Sizing the buffer from the stat saves a growth loop, but the stat is
        // attacker- and accident-controlled, so clamp it: with a limit in force
        // the file is already known to fit, and without one this is a hint, not
        // a promise. `read_to_end` grows past it if the file really is bigger.
        let hint = metadata
            .len()
            .min(self.limits.max_file_bytes.unwrap_or(16 * 1024 * 1024));
        let mut bytes = Vec::with_capacity(hint as usize);
        File::open(&absolute)
            .and_then(|mut file| file.read_to_end(&mut bytes))
            .map_err(|source| errored(format!("could not read tracked file: {source}")))?;
        Ok(bytes)
    }

    /// `candidate.path()` as a path relative to the working tree root.
    fn candidate_rel(&self, candidate: &Candidate) -> Result<PathBuf, String> {
        let path = candidate.path();
        let rel = if path.is_absolute() {
            // The root is canonicalized, so canonicalize the candidate too where
            // we can; a candidate that is already deleted still has to be
            // answerable.
            let resolved = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
            resolved
                .strip_prefix(self.repo.root())
                .map(normalize_rel)
                .map_err(|_| {
                    format!(
                        "{} is outside the working tree {}",
                        path.display(),
                        self.repo.root().display()
                    )
                })?
        } else {
            normalize_rel(path)
        };
        // A lossy needle is a *weakened* needle: `to_string_lossy` would replace
        // the undecodable bytes with U+FFFD and then search for a literal that
        // cannot occur, turning a silent encoding problem into a silent "no
        // references". Refuse instead, which vetoes.
        if rel.to_str().is_none() {
            return Err(format!(
                "candidate path {} is not valid UTF-8; refusing to search for a lossy needle",
                rel.display()
            ));
        }
        Ok(rel)
    }

    /// A veto that no search reached: the corpus, the needles or the candidate
    /// itself was unusable.
    fn abort(
        &self,
        needles: Vec<Needle>,
        started: Instant,
        file: Option<PathBuf>,
        message: String,
    ) -> Verdict {
        decide(ScanReport {
            needles,
            hits: Vec::new(),
            hits_capped: false,
            files_searched: 0,
            files_total: 0,
            bytes_searched: 0,
            elapsed: started.elapsed(),
            state: ScanState::Errored { file, message },
        })
    }
}

/// The single mapping from evidence to answer.
///
/// One function, so the rule is stated once and can be read in one place: a hit
/// vetoes; failing that, anything short of a completed scan vetoes; only a
/// completed scan with no hits is clear.
fn decide(report: ScanReport) -> Verdict {
    if let Some(first) = report.hits.first().cloned() {
        return Verdict::Vetoed {
            reason: VetoReason::Reference { first },
            report,
        };
    }
    if !report.state.is_complete() {
        let state = report.state.clone();
        return Verdict::Vetoed {
            reason: VetoReason::IncompleteSearch { state },
            report,
        };
    }
    Verdict::Clear { report }
}

/// Basename, stem, parent directory name and symbol, filtered by `strategy`.
///
/// Deduplicated by literal text, keeping the earliest (most specific) kind, so
/// that a file like `Makefile` — whose basename and stem are the same string —
/// contributes one needle rather than two and cannot be double-counted in a
/// fire-rate measurement.
fn derive_needles(rel: &Path, symbol: Option<&str>, strategy: NeedleStrategy) -> Vec<Needle> {
    let mut candidates: Vec<(NeedleKind, Option<String>)> = vec![(
        NeedleKind::Basename,
        rel.file_name().map(|s| s.to_string_lossy().into_owned()),
    )];
    if strategy.includes(NeedleKind::Stem) {
        candidates.push((
            NeedleKind::Stem,
            rel.file_stem().map(|s| s.to_string_lossy().into_owned()),
        ));
    }
    if strategy.includes(NeedleKind::ParentDir) {
        candidates.push((
            NeedleKind::ParentDir,
            rel.parent()
                .and_then(Path::file_name)
                .map(|s| s.to_string_lossy().into_owned()),
        ));
    }
    if strategy.includes(NeedleKind::Symbol) {
        candidates.push((NeedleKind::Symbol, symbol.map(str::to_string)));
    }

    let mut needles: Vec<Needle> = Vec::with_capacity(candidates.len());
    for (kind, text) in candidates {
        let Some(text) = text else { continue };
        // An empty needle matches at every position in every file, which would
        // veto the entire repository and hide every real signal.
        if text.is_empty() || needles.iter().any(|needle| needle.text == text) {
            continue;
        }
        needles.push(Needle { kind, text });
    }
    needles
}

/// Drop `./` components so a caller-supplied `./a/b` compares equal to git's
/// `a/b`. Anything else is left alone: `..` in a candidate path is a caller
/// error we would rather surface than silently resolve.
fn normalize_rel(path: &Path) -> PathBuf {
    path.components()
        .filter(|component| !matches!(component, Component::CurDir))
        .collect()
}

/// Every path in the index, with the mode that says how to read it.
///
/// Shells out to `git ls-files -sz` for the same reason [`crate::git`] shells
/// out for everything else: git's own index is the only authority on what
/// "tracked" means, and re-deriving it from a directory walk plus `.gitignore`
/// parsing is precisely the kind of second, subtly-different implementation that
/// makes a veto unsound. This belongs next to [`Repo`] and should move there as
/// soon as one change may touch both files.
///
/// Returns `Err(message)` on any failure. Callers turn that into a veto — an
/// enumeration that did not finish tells us nothing about the corpus.
fn tracked_files(root: &Path) -> Result<Vec<TrackedEntry>, String> {
    let mut command = Command::new("git");
    command
        .args(["ls-files", "-s", "-z"])
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for var in INHERITED_GIT_ENV {
        command.env_remove(var);
    }
    command.env("GIT_TERMINAL_PROMPT", "0");

    let output = command.output().map_err(|source| {
        format!(
            "could not run `git ls-files` in {}: {source}",
            root.display()
        )
    })?;
    if !output.status.success() {
        return Err(format!(
            "`git ls-files -s -z` in {} exited with {}: {}",
            root.display(),
            match output.status.code() {
                Some(code) => code.to_string(),
                None => "a signal".to_string(),
            },
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    let mut entries: Vec<TrackedEntry> = Vec::new();
    let mut seen: HashSet<PathBuf> = HashSet::new();
    for record in output.stdout.split(|byte| *byte == 0) {
        if record.is_empty() {
            continue;
        }
        // `<mode> SP <object> SP <stage> TAB <path>`.
        let Some(tab) = record.iter().position(|byte| *byte == b'\t') else {
            return Err(format!(
                "unparseable `git ls-files -s` record: {:?}",
                String::from_utf8_lossy(record)
            ));
        };
        let mode = record[..tab]
            .split(|byte| *byte == b' ')
            .next()
            .unwrap_or_default();
        let path = std::str::from_utf8(&record[tab + 1..]).map_err(|_| {
            // git.rs refuses to guess about non-UTF-8 paths and so do we. The
            // result is a veto, which is the safe direction.
            format!(
                "tracked path is not valid UTF-8: {:?}",
                String::from_utf8_lossy(&record[tab + 1..])
            )
        })?;
        let path = PathBuf::from(path);
        // Unmerged paths appear once per stage; one scan of the working tree
        // file answers for all of them. Through a set, not a linear scan: a
        // monorepo has six figures of tracked files and this runs per query.
        if !seen.insert(path.clone()) {
            continue;
        }
        entries.push(TrackedEntry {
            path,
            kind: match mode {
                b"120000" => EntryKind::Symlink,
                b"160000" => EntryKind::Gitlink,
                _ => EntryKind::Blob,
            },
        });
    }
    Ok(entries)
}
