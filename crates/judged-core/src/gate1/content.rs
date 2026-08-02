//! Gate 1 classes 1g–1k — content whose **provenance** forbids removal (§9.3).
//!
//! The other gates ask whether something is *used*. This module asks where it
//! came from, because for five kinds of content the answer to "is it used"
//! is irrelevant to whether deleting it is survivable:
//!
//! | | class | the thing that makes a wrong answer unrecoverable |
//! |---|---|---|
//! | **1g** | user-generated and uploaded content | it was never in the repository's history to begin with; the only copy is the file |
//! | **1h** | session and scratch state | a backup is *by definition* sometimes the last copy, and the templates that ignore it guarantee git is not holding another |
//! | **1i** | legal | the artifact is a compliance obligation, and its duplication across packages is the requirement, not a defect |
//! | **1j** | vendored, generated, submodule, LFS | not ours to reason about, and duplication by design |
//! | **1k** | migrations | the failure is invisible to every oracle we have |
//!
//! # Order of evaluation, and why 1j is first
//!
//! [`ContentGate::judge`] evaluates **1j, then 1k, 1i, 1h, 1g**, and stops at
//! the first class that fires. The first step of that order is not a
//! preference: §9.12 says the vendored/generated classification must run
//! *first, as a hard exclusion rather than a post-filter*, because vendored code
//! is duplication by design and will otherwise dominate every report the tool
//! produces. [`ContentGate::provenance`] exposes exactly that step on its own,
//! so a whole-tree pass can exclude before it does any other work.
//!
//! 1k comes next because it is the class whose failure mode no later gate can
//! see; the remaining three are ordered by nothing more interesting than the
//! cost of the check.
//!
//! # 1k: the newest migration is the dangerous one, and the research says so
//!
//! §6.12 corrects itself on this point and the correction is the whole class.
//! The hazard is *not* the old migration that was long since squashed. Django
//! migrations reference their **predecessors** — `dependencies = [('myapp',
//! '0041_x')]` — so the newest migration in a sequence is referenced by nothing:
//! zero inbound references from any symbol, any path, any grep signal, which is
//! precisely the shape every analyzer in this repository reports as dead.
//!
//! Delete `myapp/migrations/0042_add_index.py` and every *fresh* environment
//! works perfectly. CI is green. A new laptop is green. The test suite is green,
//! because it builds the schema from the migration set that exists *now*. Every
//! already-deployed environment holds a `django_migrations` row naming a file
//! that is gone, `makemigrations` generates a conflicting `0042`, and schemas
//! diverge per environment. Alembic's version of the same sentence is
//! `Can't locate revision identified by <hash>`.
//!
//! **The green test suite is not weak evidence here. It is structurally
//! incapable of detecting the failure, because the oracle constructs its world
//! from the post-deletion state.** That is why this class is *categorical*: no
//! evidence available to this tool can distinguish a safe migration from a fatal
//! one, so the class is refused entire. [`SequenceRank`] records which file was
//! the newest, because the report should name the specific file that would have
//! been deleted, but the rank never changes the verdict.
//!
//! # 1j is vendored, not invented
//!
//! §6.3 notes GitHub Linguist's `vendor.yml` and `generated.rb` are MIT
//! licensed, regression-tested against a real corpus, and directly vendorable —
//! so this module carries the upstream rules verbatim in
//! [`LINGUIST_VENDOR_PATTERNS`] and [`LINGUIST_GENERATED_PREDICATES`] and
//! matches against a mechanical translation of them, rather than a rule set
//! somebody wrote from memory. Every rule that fires quotes the upstream rule
//! that fired, so a surprising veto can be checked against the upstream file.
//!
//! The translation is honest about its residue. This crate has no regex engine,
//! so a pattern using an unbounded quantifier, a wildcard, or a character
//! shorthand cannot be expressed, and is listed by index in
//! [`VENDOR_UNSUPPORTED`] / [`GENERATED_UNSUPPORTED`] instead of being silently
//! approximated — approximating it would be the invention the vendoring exists
//! to avoid. [`vendor_rule_census`] and [`generated_rule_census`] report the
//! split, and the test suite asserts the two halves account for the whole file.
//!
//! **The translation was checked against the thing it translates**, rather than
//! reviewed. 45,915 paths — every tracked file in this repository plus a
//! generated corpus built from the literal fragments of the upstream patterns —
//! were classified twice: once by this module, once by running the 168 upstream
//! regexes through a real regex engine. Upstream called 19,421 of them vendored.
//!
//! - **0 paths where this module claims vendored and upstream does not.**
//! - 401 paths (2.06%) where upstream claims vendored and this module does not,
//!   and **0 of them attributable to a translated rule** — every miss comes from
//!   one of the 41 patterns [`VENDOR_UNSUPPORTED`] already declares.
//!
//! The run earned its keep on the first pass: `^rebar$` is anchored at *both*
//! ends, was translated as a prefix, and vetoed every `rebar.*` in the corpus —
//! 125 wrong hits, which no amount of reading the table would have surfaced.
//! [`Matcher::PathExact`] and a regression test are the fix.
//!
//! **A known cost, stated rather than hidden.** Linguist's list is written for
//! *language statistics*, and it contains `(^|/)dist/`, `(^|/)cache/` and
//! `(^|/)env/`. Vendoring it faithfully therefore makes those directories
//! Gate-1 ineligible, which puts them out of reach of Gate 3's artifact
//! promotion path (§9.3 3a–3d) — a real recall cost, paid deliberately. The
//! alternative is a hand-edited copy of somebody else's regression-tested file,
//! which is worse: it looks vendored and is not.
//!
//! # `.gitattributes` outranks Linguist, and can rescue
//!
//! §9.12 asks for `linguist-vendored` and `linguist-generated` to be honoured,
//! and git's own precedence rules make that a two-way street: a repository that
//! writes `* -linguist-vendored` in `libs/keep/.gitattributes` has *said* that
//! tree is its own code, and this module believes it over its own tables. The
//! same parse supplies `filter=lfs`, which is a statement that the file's real
//! content lives outside the repository (§6.12 lists LFS-tracked files as a
//! counter-signal for exactly that reason).
//!
//! # A read that did not finish is a hit, never an absence
//!
//! [`ContentGate::build`] walks the tree once to find every `.gitattributes`,
//! because a nested one overrides the root one and skipping it would silently
//! answer with the wrong precedence. If that walk hits its entry limit it has
//! *not* established that the tree is unmarked, so the gate reports
//! [`ContentEvidence::AttributesIncomplete`] and vetoes **every** candidate
//! until it can finish. This is §6.20's rule applied to the substrate rather
//! than to an analyzer, and the same shape [`crate::veto::recency`] uses for a
//! shallow clone: a gate that could not run is not a gate that found nothing.

use std::cmp::Ordering;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use crate::{Error, Result};

// ---------------------------------------------------------------------------
// public vocabulary
// ---------------------------------------------------------------------------

/// Which of §9.3's classes 1g–1k refused a candidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ContentClass {
    /// **1g** — user-generated and uploaded content: `media/`, `uploads/`,
    /// `storage/app/`. §6.17 records Magento's `.gitignore` ignoring `/media/*`
    /// with seventeen `!` carve-outs, which is to say customer-uploaded product
    /// images are held in a tree that pattern-based tooling reads as junk.
    UserContent,
    /// **1h** — session and scratch state: `.RData` (an analyst's entire session
    /// workspace), `.Rhistory`, `*.bak`, `*.orig`, editor state directories.
    SessionState,
    /// **1i** — legal: licences, notices, attribution, patent grants, SBOMs and
    /// SPDX headers.
    Legal,
    /// **1j** — vendored, generated, submodule, LFS-tracked. Runs first (§9.12).
    Provenance,
    /// **1k** — migrations. Categorically ineligible; see the module docs.
    Migration,
}

impl ContentClass {
    /// The §9.3 identifier, e.g. `"1k"`. Reports quote this.
    #[must_use]
    pub fn tag(self) -> &'static str {
        match self {
            ContentClass::UserContent => "1g",
            ContentClass::SessionState => "1h",
            ContentClass::Legal => "1i",
            ContentClass::Provenance => "1j",
            ContentClass::Migration => "1k",
        }
    }

    /// The class name as §9.3 writes it.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            ContentClass::UserContent => "user-generated and uploaded content",
            ContentClass::SessionState => "session and scratch state",
            ContentClass::Legal => "legal",
            ContentClass::Provenance => "vendored, generated, submodule or LFS-tracked",
            ContentClass::Migration => "migrations",
        }
    }
}

/// Whether a Linguist `generated.rb` predicate decided on the path or on the
/// bytes.
///
/// Worth recording separately: a path decision is stable and cheap to re-check,
/// a content decision depends on a file that may have changed since.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum GeneratedVia {
    /// The predicate matched the pathname alone.
    Path,
    /// The predicate read the file.
    Content,
}

/// The naming scheme that identified an ordered migration sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SequenceScheme {
    /// `0001_initial.py` — Django, and any tool that counts.
    ZeroPaddedOrdinal,
    /// `20230115120000_create_users.rb` — Rails, Knex, Liquibase, Sequelize.
    Timestamp,
    /// `V2_1__add_column.sql` — Flyway versioned and undo scripts.
    FlywayVersion,
}

impl SequenceScheme {
    /// A short name for reports.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            SequenceScheme::ZeroPaddedOrdinal => "zero-padded ordinal",
            SequenceScheme::Timestamp => "timestamp",
            SequenceScheme::FlywayVersion => "Flyway version",
        }
    }
}

/// Where a migration sits in its own sequence.
///
/// **This never changes the verdict** — 1k is categorical. It exists because the
/// newest migration is the one with no inbound references, hence the one an
/// analyzer will actually propose, and a report that names it is worth more than
/// one that says "a migration".
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SequenceRank {
    /// Nothing in the directory sorts after it. The dangerous one: deleting it
    /// leaves every deployed environment holding a row that names a file which
    /// no longer exists, while every fresh environment is green.
    Newest,
    /// Something in the same directory sorts after it.
    Earlier,
    /// The directory could not be listed, so the rank is unknown. The class is
    /// categorical, so the verdict is unaffected.
    Unknown,
}

impl SequenceRank {
    fn describe(self) -> &'static str {
        match self {
            SequenceRank::Newest => "the newest in its sequence — nothing references it",
            SequenceRank::Earlier => "not the newest in its sequence",
            SequenceRank::Unknown => {
                "position in its sequence unknown: the directory would not list"
            }
        }
    }
}

/// Why a candidate is ineligible. Every variant names the rule that fired and,
/// where the rule is somebody else's, quotes it verbatim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContentEvidence {
    /// 1g — the path is inside a tree that holds content the repository never
    /// authored.
    UploadPath {
        /// The matched rule, written as a path fragment.
        rule: &'static str,
        /// What puts that fragment in this class.
        note: &'static str,
    },
    /// 1h — the path is editor, shell or interpreter state, or a backup.
    SessionArtifact {
        /// The matched rule.
        rule: &'static str,
        /// What that rule protects.
        note: &'static str,
    },
    /// 1i — the basename is a licence, notice, attribution or SBOM file.
    LegalDocument {
        /// The canonical name matched, e.g. `LICENSE`.
        rule: &'static str,
    },
    /// 1i — the file carries an SPDX licence declaration in its header.
    SpdxHeader {
        /// One-based line number of the declaration.
        line: usize,
        /// The identifier as written, e.g. `GPL-2.0-only`.
        declaration: String,
    },
    /// 1j — a `vendor.yml` pattern matched.
    LinguistVendored {
        /// The upstream regex, verbatim.
        pattern: &'static str,
    },
    /// 1j — a `generated.rb` predicate matched.
    LinguistGenerated {
        /// The upstream predicate name, verbatim, e.g. `generated_go?`.
        predicate: &'static str,
        /// Whether it decided on the path or on the bytes.
        via: GeneratedVia,
        /// What matched, in enough detail to check by hand.
        detail: String,
    },
    /// 1j — a `.gitattributes` line said so. Outranks the Linguist tables in
    /// both directions: it can mark a tree, and it can unmark one.
    GitAttribute {
        /// `linguist-vendored`, `linguist-generated` or `filter=lfs`.
        attribute: &'static str,
        /// The pattern as written in the file.
        pattern: String,
        /// Which `.gitattributes` declared it, repo-relative.
        declared_in: PathBuf,
    },
    /// 1j — `.gitmodules` names this path as a submodule. Its content belongs
    /// to another repository and this one holds only a gitlink.
    Submodule {
        /// Repo-relative path of the `.gitmodules` that declared it.
        declared_in: PathBuf,
    },
    /// 1k — the basename is an ordinal in a sequence.
    OrderedSequence {
        /// Which naming scheme matched.
        scheme: SequenceScheme,
        /// The ordinal as written, e.g. `0042`.
        ordinal: String,
        /// Position within the containing directory. Never affects the verdict.
        rank: SequenceRank,
    },
    /// 1k — the path sits under a migrations directory, whatever its basename
    /// looks like. This is what catches Alembic, whose revisions are named by
    /// hash and carry no ordinal at all.
    MigrationDirectory {
        /// The directory component that matched, e.g. `migrations`.
        component: String,
        /// How many entries the directory holds, or `0` if it would not list.
        siblings: usize,
    },
    /// 1j — the `.gitattributes` walk did not finish, so no candidate has been
    /// cleared. A veto, not a verdict about this file (§6.20).
    AttributesIncomplete {
        /// Directory entries visited before the limit was reached.
        visited: usize,
        /// The limit that stopped it.
        limit: usize,
    },
}

impl fmt::Display for ContentEvidence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ContentEvidence::UploadPath { rule, note } => {
                write!(f, "sits under `{rule}` ({note})")
            }
            ContentEvidence::SessionArtifact { rule, note } => {
                write!(f, "matches `{rule}` ({note})")
            }
            ContentEvidence::LegalDocument { rule } => {
                write!(f, "is a `{rule}` file: a compliance artifact, not a duplicate")
            }
            ContentEvidence::SpdxHeader { line, declaration } => write!(
                f,
                "declares `SPDX-License-Identifier: {declaration}` on line {line}"
            ),
            ContentEvidence::LinguistVendored { pattern } => write!(
                f,
                "matches GitHub Linguist vendor.yml `{pattern}`, so it is not this repository's code"
            ),
            ContentEvidence::LinguistGenerated {
                predicate,
                via,
                detail,
            } => {
                let how = match via {
                    GeneratedVia::Path => "by pathname",
                    GeneratedVia::Content => "by content",
                };
                write!(
                    f,
                    "matches GitHub Linguist generated.rb `{predicate}` {how}: {detail}"
                )
            }
            ContentEvidence::GitAttribute {
                attribute,
                pattern,
                declared_in,
            } => write!(
                f,
                "{} sets `{attribute}` on `{pattern}`",
                declared_in.display()
            ),
            ContentEvidence::Submodule { declared_in } => write!(
                f,
                "{} declares it a submodule: the content belongs to another repository",
                declared_in.display()
            ),
            ContentEvidence::OrderedSequence {
                scheme,
                ordinal,
                rank,
            } => write!(
                f,
                "is migration `{ordinal}` ({} naming), {}",
                scheme.label(),
                rank.describe()
            ),
            ContentEvidence::MigrationDirectory {
                component,
                siblings,
            } => write!(
                f,
                "sits in a `{component}` directory of {siblings} entries; \
                 a deployed environment records which of them have run"
            ),
            ContentEvidence::AttributesIncomplete { visited, limit } => write!(
                f,
                "the .gitattributes walk stopped at {visited} of at most {limit} entries, \
                 so nothing in this tree has been cleared"
            ),
        }
    }
}

/// What classes 1g–1k have to say about one candidate.
///
/// There are two states and deliberately no third.
/// [`Abstain`](ContentVerdict::Abstain) means *these five classes have nothing
/// to say*, and is never a claim that the candidate is dead — Gate 1 refuses,
/// it does not accuse (§9.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContentVerdict {
    /// Absolutely ineligible. No later evidence overrides this.
    Ineligible {
        /// Which class refused it.
        class: ContentClass,
        /// Why, in checkable detail.
        evidence: ContentEvidence,
    },
    /// Classes 1g–1k have nothing to say about this candidate.
    Abstain,
}

impl ContentVerdict {
    /// Did a class fire?
    #[must_use]
    pub fn is_ineligible(&self) -> bool {
        matches!(self, ContentVerdict::Ineligible { .. })
    }

    /// The class that fired, if any.
    #[must_use]
    pub fn class(&self) -> Option<ContentClass> {
        match self {
            ContentVerdict::Ineligible { class, .. } => Some(*class),
            ContentVerdict::Abstain => None,
        }
    }

    /// The evidence, if any.
    #[must_use]
    pub fn evidence(&self) -> Option<&ContentEvidence> {
        match self {
            ContentVerdict::Ineligible { evidence, .. } => Some(evidence),
            ContentVerdict::Abstain => None,
        }
    }
}

// ---------------------------------------------------------------------------
// the gate
// ---------------------------------------------------------------------------

/// Default ceiling on directory entries visited while looking for
/// `.gitattributes`.
///
/// High enough that no real repository reaches it — a million entries is
/// several times a `node_modules` tree — and present only so that a pathological
/// or hostile tree produces a loud [`ContentEvidence::AttributesIncomplete`]
/// rather than an unbounded walk.
pub const DEFAULT_ENTRY_LIMIT: usize = 1_000_000;

/// How much of a candidate is read for the content rules.
///
/// Every upstream content predicate decides inside the first forty lines; the
/// one exception is `minified_files?`, which averages line length over the whole
/// file. Truncating that average can only *under*-report a long file's average
/// line length when the truncation point falls mid-line, and a file this size
/// that is still under the threshold is not minified. Reading it all would make
/// the gate quadratic in repository size for no accuracy.
const MAX_CONTENT_BYTES: usize = 1 << 20;

/// Classes 1g–1k, built once per repository.
///
/// Construction walks the tree for `.gitattributes` and reads `.gitmodules`;
/// [`judge`](ContentGate::judge) then needs at most one read of the candidate
/// itself, so a whole-tree pass is one walk plus one read per file.
#[derive(Debug, Clone)]
pub struct ContentGate {
    root: PathBuf,
    /// In git precedence order: shallower first, and within one file, earlier
    /// lines first. The last match wins.
    attributes: Vec<AttributeRule>,
    /// Repo-relative submodule paths from `.gitmodules`, and where they were
    /// declared.
    submodules: Vec<(String, PathBuf)>,
    /// `Some` when the walk stopped early. Vetoes everything while set.
    incomplete: Option<(usize, usize)>,
}

impl ContentGate {
    /// Build the gate for the repository rooted at `root`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Io`] if `root` cannot be listed at all. A directory
    /// *inside* the tree that will not list is not an error: it is a place a
    /// `.gitattributes` might be hiding, so it counts as an incomplete walk and
    /// vetoes every candidate.
    pub fn build(root: &Path) -> Result<ContentGate> {
        ContentGate::build_with_entry_limit(root, DEFAULT_ENTRY_LIMIT)
    }

    /// [`build`](ContentGate::build) with an explicit ceiling on directory
    /// entries visited.
    ///
    /// # Errors
    ///
    /// As [`build`](ContentGate::build).
    pub fn build_with_entry_limit(root: &Path, limit: usize) -> Result<ContentGate> {
        let mut walk = AttributeWalk {
            root: root.to_path_buf(),
            limit,
            visited: 0,
            files: Vec::new(),
            incomplete: None,
        };
        walk.descend(root, "", 0)?;

        let mut attributes = Vec::new();
        for (declared_in, directory) in &walk.files {
            let absolute = root.join(declared_in);
            if let Ok(text) = fs::read_to_string(&absolute) {
                parse_attributes(&text, declared_in, directory, &mut attributes);
            } else {
                // A `.gitattributes` we cannot read may be the one that marks
                // this tree vendored, or the one that unmarks it. Either way we
                // have not read the repository's own statement about itself, so
                // the walk counts as incomplete rather than clean.
                walk.incomplete.get_or_insert((walk.visited, limit));
            }
        }
        attributes.sort_by(|a, b| {
            a.depth
                .cmp(&b.depth)
                .then_with(|| a.directory.cmp(&b.directory))
                .then_with(|| a.line.cmp(&b.line))
        });

        let submodules = read_submodules(root)?;

        Ok(ContentGate {
            root: root.to_path_buf(),
            attributes,
            submodules,
            incomplete: walk.incomplete,
        })
    }

    /// The repository root this gate was built for.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Did the `.gitattributes` walk fail to finish?
    ///
    /// While true, every candidate is refused: the repository may have marked
    /// itself in a file we did not read (§6.20).
    #[must_use]
    pub fn attributes_incomplete(&self) -> bool {
        self.incomplete.is_some()
    }

    /// Class 1j alone — the hard exclusion §9.12 asks to run *first*, exposed on
    /// its own so a whole-tree pass can drop vendored and generated content
    /// before spending anything else on it.
    ///
    /// # Errors
    ///
    /// [`Error::Io`] if the candidate exists but cannot be read.
    pub fn provenance(&self, path: &Path) -> Result<ContentVerdict> {
        let Some(relative) = self.relative(path) else {
            return Ok(ContentVerdict::Abstain);
        };
        let mut content = LazyContent::new(&self.root, &relative);
        self.judge_provenance(&relative, &mut content)
    }

    /// Judge one candidate against classes 1g–1k.
    ///
    /// `path` is repo-relative, or absolute inside [`root`](ContentGate::root).
    ///
    /// # Errors
    ///
    /// [`Error::Io`] if the candidate exists but cannot be read. A candidate
    /// that does not exist is not an error — it simply has no content for the
    /// content rules to look at, and §9.3's 1p owns the question of what a
    /// missing or unidentifiable file means.
    pub fn judge(&self, path: &Path) -> Result<ContentVerdict> {
        let Some(relative) = self.relative(path) else {
            return Ok(ContentVerdict::Abstain);
        };
        let mut content = LazyContent::new(&self.root, &relative);

        // §9.12: the vendored / generated classification is a hard exclusion
        // and runs before anything else.
        let provenance = self.judge_provenance(&relative, &mut content)?;
        if provenance.is_ineligible() {
            return Ok(provenance);
        }
        if let Some(verdict) = self.judge_migration(&relative) {
            return Ok(verdict);
        }
        if let Some(verdict) = judge_legal(&relative, &mut content)? {
            return Ok(verdict);
        }
        if let Some(verdict) = judge_table(
            &relative,
            SESSION_RULES,
            ContentClass::SessionState,
            |rule, note| ContentEvidence::SessionArtifact { rule, note },
        ) {
            return Ok(verdict);
        }
        if let Some(verdict) = judge_table(
            &relative,
            UPLOAD_RULES,
            ContentClass::UserContent,
            |rule, note| ContentEvidence::UploadPath { rule, note },
        ) {
            return Ok(verdict);
        }
        Ok(ContentVerdict::Abstain)
    }

    // -- 1j ------------------------------------------------------------------

    fn judge_provenance(
        &self,
        relative: &RelativePath,
        content: &mut LazyContent,
    ) -> Result<ContentVerdict> {
        if let Some((visited, limit)) = self.incomplete {
            return Ok(ineligible(
                ContentClass::Provenance,
                ContentEvidence::AttributesIncomplete { visited, limit },
            ));
        }
        if let Some(evidence) = self.attribute_evidence(relative) {
            return Ok(ineligible(ContentClass::Provenance, evidence));
        }
        for (declared, declared_in) in &self.submodules {
            if relative.under(declared) {
                return Ok(ineligible(
                    ContentClass::Provenance,
                    ContentEvidence::Submodule {
                        declared_in: declared_in.clone(),
                    },
                ));
            }
        }
        for (index, matcher) in VENDOR_MATCHERS {
            if matcher.matches(relative) {
                return Ok(ineligible(
                    ContentClass::Provenance,
                    ContentEvidence::LinguistVendored {
                        pattern: LINGUIST_VENDOR_PATTERNS[*index],
                    },
                ));
            }
        }
        for (index, matcher) in GENERATED_PATH_MATCHERS {
            if matcher.matches(relative) {
                return Ok(ineligible(
                    ContentClass::Provenance,
                    ContentEvidence::LinguistGenerated {
                        predicate: LINGUIST_GENERATED_PREDICATES[*index],
                        via: GeneratedVia::Path,
                        detail: matcher.describe(),
                    },
                ));
            }
        }
        let Some(lines) = content.lines()? else {
            return Ok(ContentVerdict::Abstain);
        };
        if let Some(evidence) = generated_by_content(relative, lines) {
            return Ok(ineligible(ContentClass::Provenance, evidence));
        }
        Ok(ContentVerdict::Abstain)
    }

    /// The last `.gitattributes` line to match, per attribute, wins — git's own
    /// precedence. A line that *unsets* the attribute therefore rescues a path
    /// an earlier line marked, which is the repository telling us the tree is
    /// its own code.
    fn attribute_evidence(&self, relative: &RelativePath) -> Option<ContentEvidence> {
        // Each attribute is resolved **independently**, which is git's own
        // model: the last line to mention `linguist-vendored` decides
        // `linguist-vendored`, and a later line about `filter` or `text` decides
        // nothing about it. Collapsing them into one stack would let any
        // unrelated later line silently rescue a tree the repository had marked.
        let mut winners: [Option<&AttributeRule>; ATTRIBUTE_NAMES.len()] =
            [None; ATTRIBUTE_NAMES.len()];
        for rule in &self.attributes {
            let Some(slot) = ATTRIBUTE_NAMES
                .iter()
                .position(|name| *name == rule.attribute)
            else {
                continue;
            };
            if rule.matches(relative) {
                winners[slot] = Some(rule);
            }
        }
        let rule = winners.into_iter().flatten().find(|rule| rule.set)?;
        Some(ContentEvidence::GitAttribute {
            attribute: rule.attribute,
            pattern: rule.pattern.clone(),
            declared_in: rule.declared_in.clone(),
        })
    }

    // -- 1k ------------------------------------------------------------------

    fn judge_migration(&self, relative: &RelativePath) -> Option<ContentVerdict> {
        if let Some((scheme, ordinal)) = sequence_ordinal(relative.basename()) {
            let rank = self.rank_in_sequence(relative, scheme, &ordinal);
            return Some(ineligible(
                ContentClass::Migration,
                ContentEvidence::OrderedSequence {
                    scheme,
                    ordinal,
                    rank,
                },
            ));
        }
        let component = relative.directory_components().find(|component| {
            MIGRATION_DIRECTORIES
                .iter()
                .any(|name| component.eq_ignore_ascii_case(name))
        })?;
        let siblings = self
            .list_directory(relative)
            .map_or(0, |entries| entries.len());
        Some(ineligible(
            ContentClass::Migration,
            ContentEvidence::MigrationDirectory {
                component: component.to_string(),
                siblings,
            },
        ))
    }

    /// Is this the newest file in its directory under the same scheme?
    ///
    /// Only files sharing the candidate's scheme are compared: a directory
    /// holding both `0001_initial.py` and `V1__baseline.sql` has two sequences
    /// in it, and ordering across them would be meaningless.
    fn rank_in_sequence(
        &self,
        relative: &RelativePath,
        scheme: SequenceScheme,
        ordinal: &str,
    ) -> SequenceRank {
        let Some(entries) = self.list_directory(relative) else {
            return SequenceRank::Unknown;
        };
        let mut rank = SequenceRank::Newest;
        for entry in entries {
            if entry == relative.basename() {
                continue;
            }
            let Some((other_scheme, other)) = sequence_ordinal(&entry) else {
                continue;
            };
            if other_scheme != scheme {
                continue;
            }
            if compare_ordinals(scheme, &other, ordinal) == Ordering::Greater {
                rank = SequenceRank::Earlier;
                break;
            }
        }
        rank
    }

    fn list_directory(&self, relative: &RelativePath) -> Option<Vec<String>> {
        let directory = self.root.join(relative.directory());
        let reader = fs::read_dir(directory).ok()?;
        let mut names = Vec::new();
        for entry in reader {
            let entry = entry.ok()?;
            names.push(entry.file_name().to_string_lossy().into_owned());
        }
        Some(names)
    }

    // -- paths ---------------------------------------------------------------

    /// Normalize a candidate to a repo-relative, `/`-separated path.
    ///
    /// Returns `None` for a path that is not inside the repository, because a
    /// gate built for one tree has nothing to say about another.
    fn relative(&self, path: &Path) -> Option<RelativePath> {
        let stripped = if path.is_absolute() {
            path.strip_prefix(&self.root).ok()?
        } else {
            path
        };
        RelativePath::new(stripped)
    }
}

fn ineligible(class: ContentClass, evidence: ContentEvidence) -> ContentVerdict {
    ContentVerdict::Ineligible { class, evidence }
}

/// Class 1i. Free-standing because it needs the candidate and nothing about the
/// repository — no `.gitattributes`, no sibling listing, no root.
fn judge_legal(
    relative: &RelativePath,
    content: &mut LazyContent,
) -> Result<Option<ContentVerdict>> {
    if let Some(rule) = legal_basename(relative.basename()) {
        return Ok(Some(ineligible(
            ContentClass::Legal,
            ContentEvidence::LegalDocument { rule },
        )));
    }
    if let Some(verdict) = judge_table(
        relative,
        LEGAL_PATH_RULES,
        ContentClass::Legal,
        |rule, _| ContentEvidence::LegalDocument { rule },
    ) {
        return Ok(Some(verdict));
    }
    let Some(lines) = content.lines()? else {
        return Ok(None);
    };
    for (offset, line) in lines.iter().take(SPDX_HEADER_LINES).enumerate() {
        let Some((_, tail)) = line.split_once(SPDX_MARKER) else {
            continue;
        };
        let declaration = tail.trim().trim_end_matches("*/").trim().to_string();
        if declaration.is_empty() {
            continue;
        }
        return Ok(Some(ineligible(
            ContentClass::Legal,
            ContentEvidence::SpdxHeader {
                line: offset + 1,
                declaration,
            },
        )));
    }
    Ok(None)
}

fn judge_table(
    relative: &RelativePath,
    table: &'static [PathRule],
    class: ContentClass,
    evidence: fn(&'static str, &'static str) -> ContentEvidence,
) -> Option<ContentVerdict> {
    table
        .iter()
        .find(|rule| rule.matcher.matches(relative))
        .map(|rule| ineligible(class, evidence(rule.rule, rule.note)))
}

// ---------------------------------------------------------------------------
// repo-relative paths
// ---------------------------------------------------------------------------

/// A candidate path, normalized once: `/`-separated, no `.` components, and
/// with its basename split out.
///
/// Everything downstream matches against strings rather than [`Path`], because
/// the upstream Linguist rules are regexes over a `/`-joined pathname and
/// re-deriving that per rule would be both slower and a place for the two forms
/// to disagree.
#[derive(Debug, Clone, PartialEq, Eq)]
struct RelativePath {
    /// The whole path, `/`-separated.
    full: String,
    /// Byte offset of the basename within `full`.
    basename_at: usize,
}

impl RelativePath {
    fn new(path: &Path) -> Option<RelativePath> {
        let mut parts: Vec<String> = Vec::new();
        for component in path.components() {
            match component {
                Component::Normal(part) => parts.push(part.to_string_lossy().into_owned()),
                Component::CurDir => {}
                // A candidate outside the tree, or an absolute path that did
                // not strip: neither is this repository's business.
                Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
            }
        }
        if parts.is_empty() {
            return None;
        }
        let full = parts.join("/");
        let basename_at = full.len() - parts[parts.len() - 1].len();
        Some(RelativePath { full, basename_at })
    }

    fn as_str(&self) -> &str {
        &self.full
    }

    fn basename(&self) -> &str {
        &self.full[self.basename_at..]
    }

    /// The containing directory, `""` at the repository root.
    fn directory(&self) -> &str {
        self.full[..self.basename_at].trim_end_matches('/')
    }

    fn directory_components(&self) -> impl Iterator<Item = &str> {
        self.directory().split('/').filter(|part| !part.is_empty())
    }

    /// Is this path `prefix` itself, or inside it?
    fn under(&self, prefix: &str) -> bool {
        let prefix = prefix.trim_end_matches('/');
        if prefix.is_empty() {
            return true;
        }
        self.full == prefix
            || (self.full.len() > prefix.len()
                && self.full.starts_with(prefix)
                && self.full.as_bytes()[prefix.len()] == b'/')
    }
}

// ---------------------------------------------------------------------------
// matchers
// ---------------------------------------------------------------------------

/// The literal shapes the upstream regexes reduce to.
///
/// Each variant names the regex construct it stands for, so a reader can check
/// a translated rule against the upstream file without running anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Matcher {
    /// `(^|/)literal` — at the start of the path, or after a `/`.
    Anchored(&'static str),
    /// `literal` unanchored — anywhere in the path.
    Contains(&'static str),
    /// `^literal` — at the start of the path only.
    Prefix(&'static str),
    /// `^literal$` — the whole path is exactly this. Distinct from
    /// [`Matcher::Prefix`] because upstream's `^rebar$` names one file at the
    /// repository root and nothing else; translating it as a prefix vetoed
    /// every `rebar.*` in the tree, which a differential run against the
    /// upstream regexes caught at 125 wrong hits in a 46,000-path corpus.
    PathExact(&'static str),
    /// `literal$` — the basename ends with it.
    BasenameSuffix(&'static str),
    /// `(^|/)literal$` — the basename is exactly it.
    BasenameExact(&'static str),
    /// `(^|/)literal` where the tail is a wildcard — the basename starts with it.
    BasenamePrefix(&'static str),
    /// `literal$` with `/i`.
    BasenameSuffixCi(&'static str),
    /// `(^|/)literal$` with `/i`.
    BasenameExactCi(&'static str),
}

impl Matcher {
    fn matches(self, path: &RelativePath) -> bool {
        let full = path.as_str();
        let base = path.basename();
        match self {
            // Written out rather than as `contains(&format!("/{literal}"))`
            // because this runs 267 times per candidate over a whole tree, and
            // an allocation per rule per file is the difference between a scan
            // and a wait.
            Matcher::Anchored(literal) => {
                full.starts_with(literal)
                    || full
                        .match_indices(literal)
                        .any(|(at, _)| at > 0 && full.as_bytes()[at - 1] == b'/')
            }
            Matcher::Contains(literal) => full.contains(literal),
            Matcher::Prefix(literal) => full.starts_with(literal),
            Matcher::PathExact(literal) => full == literal,
            Matcher::BasenameSuffix(literal) => base.ends_with(literal),
            Matcher::BasenameExact(literal) => base == literal,
            Matcher::BasenamePrefix(literal) => base.starts_with(literal),
            Matcher::BasenameSuffixCi(literal) => {
                base.len() >= literal.len()
                    && base[base.len() - literal.len()..].eq_ignore_ascii_case(literal)
            }
            Matcher::BasenameExactCi(literal) => base.eq_ignore_ascii_case(literal),
        }
    }

    fn literal(self) -> &'static str {
        match self {
            Matcher::Anchored(literal)
            | Matcher::Contains(literal)
            | Matcher::Prefix(literal)
            | Matcher::PathExact(literal)
            | Matcher::BasenameSuffix(literal)
            | Matcher::BasenameExact(literal)
            | Matcher::BasenamePrefix(literal)
            | Matcher::BasenameSuffixCi(literal)
            | Matcher::BasenameExactCi(literal) => literal,
        }
    }

    fn describe(self) -> String {
        format!("the pathname matches `{}`", self.literal())
    }
}

/// One of this module's own path rules — the 1g, 1h and 1i tables, which have no
/// upstream to vendor and so carry their argument inline.
#[derive(Debug, Clone, Copy)]
struct PathRule {
    rule: &'static str,
    matcher: Matcher,
    note: &'static str,
}

// ---------------------------------------------------------------------------
// 1g — user-generated and uploaded content
// ---------------------------------------------------------------------------

/// Trees that hold content the repository did not author.
///
/// The unifying property is not "binary" or "large": it is that **git never
/// held another copy**, so [`crate::git::RecoverabilityClass`] puts every one of
/// these at rung R9 and a wrong deletion is final. §6.17's Magento example is
/// the shape — `/media/*` ignored with seventeen `!` carve-outs, so the
/// customer-uploaded product images in it are invisible to everything that
/// reasons from tracked state.
const UPLOAD_RULES: &[PathRule] = &[
    PathRule {
        rule: "media/",
        matcher: Matcher::Anchored("media/"),
        note: "Magento's ignored /media/* and Django's MEDIA_ROOT both live here",
    },
    PathRule {
        rule: "uploads/",
        matcher: Matcher::Anchored("uploads/"),
        note: "the near-universal name for a directory written to by users",
    },
    PathRule {
        rule: "upload/",
        matcher: Matcher::Anchored("upload/"),
        note: "the singular spelling of the same thing",
    },
    PathRule {
        rule: "storage/app/",
        matcher: Matcher::Anchored("storage/app/"),
        note: "Laravel's default filesystem disk",
    },
    PathRule {
        rule: "wp-content/uploads/",
        matcher: Matcher::Anchored("wp-content/uploads/"),
        note: "the WordPress media library",
    },
    PathRule {
        rule: "public/system/",
        matcher: Matcher::Anchored("public/system/"),
        note: "Rails Paperclip's default attachment root",
    },
    PathRule {
        rule: "attachments/",
        matcher: Matcher::Anchored("attachments/"),
        note: "files attached by users to something else",
    },
    PathRule {
        rule: "user-content/",
        matcher: Matcher::Anchored("user-content/"),
        note: "the name says whose it is",
    },
    PathRule {
        rule: "usercontent/",
        matcher: Matcher::Anchored("usercontent/"),
        note: "the unhyphenated spelling of the same thing",
    },
];

// ---------------------------------------------------------------------------
// 1h — session and scratch state
// ---------------------------------------------------------------------------

/// Editor, interpreter and shell state, and backups.
///
/// The argument for this class is short and decisive. §6.17 counts **29
/// canonical `.gitignore` templates that ignore `*.bak`** — so a `.bak` in a
/// working tree is, in the overwhelming majority of repositories, *untracked*,
/// which is [`crate::git::RecoverabilityClass`] rung R9: git protects the object
/// database, not the working tree (§8.1). And a backup is by definition
/// sometimes the last copy; that is what it is for. Being ignored is exactly
/// what makes it unrecoverable, not what makes it disposable.
///
/// `.RData` earns its place separately: it is not a file an analyst edits, it is
/// **the entire session workspace** — every fitted model and intermediate frame
/// that was never written down as code.
///
/// Two directories that belong here are missing on purpose: `.idea/` and
/// `.vscode/` are both claimed by class 1j first, because Linguist's own files
/// list them (`intellij_file?` and `vendor.yml` respectively). Listing them here
/// too would be unreachable code that reads as coverage.
const SESSION_RULES: &[PathRule] = &[
    PathRule {
        rule: ".RData",
        matcher: Matcher::BasenameExact(".RData"),
        note: "an entire R session workspace: every object that was never written down as code",
    },
    PathRule {
        rule: ".Rhistory",
        matcher: Matcher::BasenameExact(".Rhistory"),
        note: "the command history that reconstructs how a result was reached",
    },
    PathRule {
        rule: ".Ruserdata",
        matcher: Matcher::BasenameExact(".Ruserdata"),
        note: "per-user RStudio session state",
    },
    PathRule {
        rule: ".Rproj.user/",
        matcher: Matcher::Anchored(".Rproj.user/"),
        note: "RStudio's per-user project state",
    },
    PathRule {
        rule: "*.bak",
        matcher: Matcher::BasenameSuffix(".bak"),
        note: "29 canonical .gitignore templates ignore this, so git is not holding another copy",
    },
    PathRule {
        rule: "*.orig",
        matcher: Matcher::BasenameSuffix(".orig"),
        note: "a merge or patch left this as the pre-conflict content",
    },
    PathRule {
        rule: "*.rej",
        matcher: Matcher::BasenameSuffix(".rej"),
        note: "the hunks a patch could not apply: the only record of what was attempted",
    },
    PathRule {
        rule: "*.swp",
        matcher: Matcher::BasenameSuffix(".swp"),
        note: "a vim swap file holds unsaved edits from a session that did not exit",
    },
    PathRule {
        rule: "*.swo",
        matcher: Matcher::BasenameSuffix(".swo"),
        note: "the second vim swap file, same reason",
    },
    PathRule {
        rule: "*~",
        matcher: Matcher::BasenameSuffix("~"),
        note: "the emacs and vi backup convention",
    },
    PathRule {
        rule: ".history/",
        matcher: Matcher::Anchored(".history/"),
        note: "VS Code Local History: per-edit snapshots that exist only here",
    },
    PathRule {
        rule: ".ipynb_checkpoints/",
        matcher: Matcher::Anchored(".ipynb_checkpoints/"),
        note: "Jupyter's autosave of a notebook, sometimes newer than the notebook",
    },
    PathRule {
        rule: ".vs/",
        matcher: Matcher::Anchored(".vs/"),
        note: "Visual Studio per-user solution state",
    },
];

// ---------------------------------------------------------------------------
// 1i — legal
// ---------------------------------------------------------------------------

/// Canonical legal basenames.
///
/// Matched as a *stem*: `LICENSE`, `LICENSE.md`, `LICENSE-MIT` and
/// `LICENSE_APACHE` all count, because the remainder after the stem begins with
/// `.`, `-` or `_`. `licenses.py` does not, because `S` is none of those.
///
/// §6.15 is why this class exists at all rather than being left to a
/// deduplicator: measured on a real 1356-file repository, **6 of 6**
/// content-identical groups were unsafe to delete, and *"identical `LICENSE` per
/// package (a legal requirement)"* is named as one of the structural cases. The
/// content claim was 100% precise; the deletability claim was **0%**. A tool
/// that keeps one copy of a licence and removes the rest has not removed
/// duplication, it has removed a compliance artifact from every package but one.
const LEGAL_STEMS: &[&str] = &[
    "LICENSE",
    "LICENCE",
    "COPYING",
    "COPYRIGHT",
    "NOTICE",
    "AUTHORS",
    "CONTRIBUTORS",
    "MAINTAINERS",
    "PATENTS",
    "UNLICENSE",
    "LEGAL",
    "THIRD_PARTY_NOTICES",
    "THIRD-PARTY-NOTICES",
    "THIRDPARTYNOTICES",
    "OWNERS",
];

/// Separators that may follow a legal stem.
const LEGAL_STEM_SEPARATORS: [u8; 3] = [b'.', b'-', b'_'];

/// SBOM and licence-metadata shapes that are not a single canonical basename.
const LEGAL_PATH_RULES: &[PathRule] = &[
    PathRule {
        rule: "LICENSES/",
        matcher: Matcher::Anchored("LICENSES/"),
        note: "the REUSE specification's licence directory",
    },
    PathRule {
        rule: ".reuse/",
        matcher: Matcher::Anchored(".reuse/"),
        note: "REUSE metadata: the machine-readable half of the same obligation",
    },
    PathRule {
        rule: "*.spdx",
        matcher: Matcher::BasenameSuffixCi(".spdx"),
        note: "an SPDX software bill of materials",
    },
    PathRule {
        rule: "*.spdx.json",
        matcher: Matcher::BasenameSuffixCi(".spdx.json"),
        note: "an SPDX bill of materials in JSON",
    },
    PathRule {
        rule: "*.spdx.yaml",
        matcher: Matcher::BasenameSuffixCi(".spdx.yaml"),
        note: "an SPDX bill of materials in YAML",
    },
    PathRule {
        rule: "*.spdx.rdf",
        matcher: Matcher::BasenameSuffixCi(".spdx.rdf"),
        note: "an SPDX bill of materials in RDF",
    },
    PathRule {
        rule: "*.cdx.json",
        matcher: Matcher::BasenameSuffixCi(".cdx.json"),
        note: "a CycloneDX bill of materials in JSON",
    },
    PathRule {
        rule: "*.cdx.xml",
        matcher: Matcher::BasenameSuffixCi(".cdx.xml"),
        note: "a CycloneDX bill of materials in XML",
    },
    PathRule {
        rule: "bom.json",
        matcher: Matcher::BasenameExactCi("bom.json"),
        note: "CycloneDX's default output name",
    },
    PathRule {
        rule: "sbom.json",
        matcher: Matcher::BasenameExactCi("sbom.json"),
        note: "the other common default output name",
    },
];

/// How far into a file an SPDX header is looked for.
///
/// Long enough to clear a shebang, an encoding line and a copyright block;
/// short enough that the marker appearing in the body of a document does not
/// count.
const SPDX_HEADER_LINES: usize = 16;

/// The SPDX identifier tag, as the specification spells it.
const SPDX_MARKER: &str = "SPDX-License-Identifier:";

/// Is this basename a legal document?
///
/// Returns the canonical stem it matched, which is what the report quotes.
fn legal_basename(basename: &str) -> Option<&'static str> {
    let upper = basename.to_ascii_uppercase();
    LEGAL_STEMS.iter().copied().find(|stem| {
        let Some(rest) = upper.strip_prefix(*stem) else {
            return false;
        };
        rest.is_empty() || LEGAL_STEM_SEPARATORS.contains(&rest.as_bytes()[0])
    })
}

// ---------------------------------------------------------------------------
// 1k — migrations
// ---------------------------------------------------------------------------

/// Directory names that make everything below them a migration, whatever the
/// basename looks like.
///
/// This is what catches Alembic, whose revisions are named by hash
/// (`8f3a1c92be04_add_users.py`) and carry no ordinal to detect. The failure is
/// the same one: `Can't locate revision identified by <hash>`.
const MIGRATION_DIRECTORIES: &[&str] = &["migrations", "migrate"];

/// Fewest digits that count as a zero-padded ordinal.
///
/// Three is Django's `0001_`, Rails' pre-2008 `001_`, and dbmate's `001_`. A
/// migration numbered below that is still caught by [`MIGRATION_DIRECTORIES`],
/// because migrations live in a directory named for them.
const MIN_ORDINAL_DIGITS: usize = 3;

/// Fewest digits that count as a timestamp rather than an ordinal.
///
/// Ten, and the choice is the difference between a gate and a nuisance. Every
/// timestamped migration scheme in use is at least ten digits — epoch seconds
/// (10), epoch milliseconds (13, node-pg-migrate), `YYYYMMDDHHMMSS` (14, Rails,
/// Knex, dbmate, Sequelize), or that plus milliseconds (17). Eight digits is
/// `YYYYMMDD`, which is not a migration scheme; it is how humans name a dated
/// file. Accepting eight would put every `20260131-notes.md` in a class that is
/// categorically ineligible, and a veto that fires on ordinary documents stops
/// being read.
const MIN_TIMESTAMP_DIGITS: usize = 10;

/// Does this basename name a position in an ordered sequence?
///
/// Returns the scheme and the ordinal as written.
fn sequence_ordinal(basename: &str) -> Option<(SequenceScheme, String)> {
    let bytes = basename.as_bytes();

    // Flyway: `V<version>__`, `U<version>__`. The version may itself contain
    // `_`, so the terminator is the first *doubled* underscore.
    if matches!(bytes.first(), Some(b'V' | b'U')) {
        if let Some(end) = basename.find("__") {
            let version = &basename[1..end];
            let versioned = !version.is_empty()
                && version.bytes().any(|byte| byte.is_ascii_digit())
                && version
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || byte == b'.' || byte == b'_');
            if versioned {
                return Some((SequenceScheme::FlywayVersion, version.to_string()));
            }
        }
    }

    let digits = bytes
        .iter()
        .take_while(|byte| byte.is_ascii_digit())
        .count();
    if digits == 0 {
        return None;
    }
    let separated = matches!(bytes.get(digits), Some(b'_' | b'-' | b'.'));
    if !separated {
        return None;
    }
    let ordinal = basename[..digits].to_string();
    if digits >= MIN_TIMESTAMP_DIGITS {
        return Some((SequenceScheme::Timestamp, ordinal));
    }
    // "Zero-padded" is meant literally. Without it the scheme swallows every
    // `2026-01.pdf` an upload directory holds and every `2024-report.md` a docs
    // directory holds — measured, both, by this module's own test suite before
    // the requirement was added. Django writes `0001_`, and a migration
    // numbered high enough to lose its leading zero has thousands of
    // predecessors to be recognised by.
    if digits >= MIN_ORDINAL_DIGITS && basename.starts_with('0') {
        return Some((SequenceScheme::ZeroPaddedOrdinal, ordinal));
    }
    None
}

/// Order two ordinals of the same scheme.
///
/// Digit strings compare by significant length first, so `0100` beats `0042`
/// without parsing into an integer that a thirteen-digit epoch would overflow at
/// some future width. Flyway versions compare component-wise, so `2.10` beats
/// `2.9`.
fn compare_ordinals(scheme: SequenceScheme, left: &str, right: &str) -> Ordering {
    match scheme {
        SequenceScheme::FlywayVersion => {
            let mut left = left.split(['.', '_']);
            let mut right = right.split(['.', '_']);
            loop {
                match (left.next(), right.next()) {
                    (None, None) => return Ordering::Equal,
                    (None, Some(_)) => return Ordering::Less,
                    (Some(_), None) => return Ordering::Greater,
                    (Some(a), Some(b)) => match compare_digits(a, b) {
                        Ordering::Equal => {}
                        other => return other,
                    },
                }
            }
        }
        SequenceScheme::ZeroPaddedOrdinal | SequenceScheme::Timestamp => {
            compare_digits(left, right)
        }
    }
}

fn compare_digits(left: &str, right: &str) -> Ordering {
    let left = left.trim_start_matches('0');
    let right = right.trim_start_matches('0');
    left.len().cmp(&right.len()).then_with(|| left.cmp(right))
}

// ---------------------------------------------------------------------------
// candidate content, read at most once
// ---------------------------------------------------------------------------

/// The candidate's first [`MAX_CONTENT_BYTES`], split into lines, read lazily
/// and at most once.
///
/// `None` means the file is not there or is not a regular file. That is not an
/// error: §9.3's 1p owns what a missing or unidentifiable path means, and this
/// module's job is to say what the *content* proves. Any other I/O failure is
/// propagated, because "I could not read it" must never be recorded as "I read
/// it and found nothing" (§6.20).
struct LazyContent {
    absolute: PathBuf,
    state: ContentState,
}

/// The three states a candidate's content can be in. Spelled out rather than
/// nested in `Option<Option<_>>` because "not read yet" and "read, and there was
/// nothing there" are exactly the two things §6.20 says must never collapse into
/// each other.
enum ContentState {
    Unread,
    Absent,
    Read(Vec<String>),
}

impl LazyContent {
    fn new(root: &Path, relative: &RelativePath) -> LazyContent {
        LazyContent {
            absolute: root.join(relative.as_str()),
            state: ContentState::Unread,
        }
    }

    fn lines(&mut self) -> Result<Option<&[String]>> {
        if matches!(self.state, ContentState::Unread) {
            self.state = match self.read()? {
                Some(lines) => ContentState::Read(lines),
                None => ContentState::Absent,
            };
        }
        Ok(match &self.state {
            ContentState::Read(lines) => Some(lines.as_slice()),
            ContentState::Unread | ContentState::Absent => None,
        })
    }

    fn read(&self) -> Result<Option<Vec<String>>> {
        let metadata = match fs::symlink_metadata(&self.absolute) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(source) => {
                return Err(Error::Io {
                    path: self.absolute.clone(),
                    source,
                })
            }
        };
        if !metadata.is_file() {
            return Ok(None);
        }
        let bytes = match fs::read(&self.absolute) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(source) => {
                return Err(Error::Io {
                    path: self.absolute.clone(),
                    source,
                })
            }
        };
        let capped = &bytes[..bytes.len().min(MAX_CONTENT_BYTES)];
        let text = String::from_utf8_lossy(capped);
        Ok(Some(text.split('\n').map(str::to_string).collect()))
    }
}

// ---------------------------------------------------------------------------
// Linguist generated.rb — the content half
// ---------------------------------------------------------------------------

/// Which lines a check looks at. Indices are zero-based, matching the upstream
/// Ruby.
#[derive(Debug, Clone, Copy)]
enum LineSelector {
    /// `lines.first(n).any?`
    First(usize),
    /// `lines[i]`
    At(usize),
    /// `lines[-k]`
    FromEnd(usize),
}

/// What a selected line has to look like.
#[derive(Debug, Clone, Copy)]
enum TextMatch {
    /// Every needle appears in the same line.
    ContainsAll(&'static [&'static str]),
    /// At least one needle appears.
    ContainsAny(&'static [&'static str]),
    /// The line starts with at least one needle.
    StartsWithAny(&'static [&'static str]),
}

/// One upstream `generated.rb` content predicate.
///
/// The upstream shape is remarkably uniform — an extension test, a line-count
/// guard, and one or more literal markers at fixed positions — which is what
/// makes vendoring it as a table honest rather than a rewrite.
#[derive(Debug, Clone, Copy)]
struct GeneratedContentRule {
    /// Index into [`LINGUIST_GENERATED_PREDICATES`].
    predicate: usize,
    /// Upstream tests `File.extname`; a basename suffix is the same test for
    /// every extension in these tables, and additionally expresses
    /// `generated_perl_ppport_header?`, whose guard is a filename.
    suffixes: &'static [&'static str],
    /// Whether the extension test carried `.downcase`.
    suffix_case_insensitive: bool,
    /// Upstream `lines.count > min_lines`. `generated_sorbet_rbi?`'s
    /// `lines.count >= 5` is written here as `4`.
    min_lines: usize,
    /// All of these must hold.
    checks: &'static [(LineSelector, TextMatch)],
}

impl GeneratedContentRule {
    fn matches(&self, relative: &RelativePath, lines: &[String]) -> Option<String> {
        let base = relative.basename();
        let suffix_matches = self.suffixes.iter().any(|suffix| {
            if self.suffix_case_insensitive {
                base.len() >= suffix.len()
                    && base[base.len() - suffix.len()..].eq_ignore_ascii_case(suffix)
            } else {
                base.ends_with(suffix)
            }
        });
        if !suffix_matches || lines.len() <= self.min_lines {
            return None;
        }
        let mut detail = None;
        for (selector, matcher) in self.checks {
            let hit = match selector {
                LineSelector::First(count) => lines
                    .iter()
                    .take(*count)
                    .find(|line| matcher.hit(line))
                    .map(String::as_str),
                LineSelector::At(index) => lines
                    .get(*index)
                    .filter(|line| matcher.hit(line))
                    .map(String::as_str),
                LineSelector::FromEnd(back) => lines
                    .len()
                    .checked_sub(*back)
                    .and_then(|index| lines.get(index))
                    .filter(|line| matcher.hit(line))
                    .map(String::as_str),
            };
            let line = hit?;
            detail.get_or_insert_with(|| line.trim().to_string());
        }
        Some(detail.unwrap_or_default())
    }
}

impl TextMatch {
    fn hit(self, line: &str) -> bool {
        match self {
            TextMatch::ContainsAll(needles) => needles.iter().all(|needle| line.contains(needle)),
            TextMatch::ContainsAny(needles) => needles.iter().any(|needle| line.contains(needle)),
            TextMatch::StartsWithAny(needles) => {
                needles.iter().any(|needle| line.starts_with(needle))
            }
        }
    }
}

/// Upstream `maybe_minified?`: the two extensions the two byte-level predicates
/// apply to.
const MAYBE_MINIFIED: [&str; 2] = [".js", ".css"];

/// Upstream `minified_files?`: mean line length over 110.
const MINIFIED_MEAN_LINE_LENGTH: usize = 110;

/// Index of `minified_files?` in [`LINGUIST_GENERATED_PREDICATES`].
const MINIFIED_PREDICATE: usize = 30;

/// Index of `has_source_map?`.
const SOURCE_MAP_PREDICATE: usize = 31;

/// Upstream `has_source_map?`, expanded: `^\/[*\/][\#@] source(?:Mapping)?URL`.
const SOURCE_MAP_PREFIXES: [&str; 8] = [
    "//# sourceMappingURL",
    "//@ sourceMappingURL",
    "/*# sourceMappingURL",
    "/*@ sourceMappingURL",
    "//# sourceURL",
    "//@ sourceURL",
    "/*# sourceURL",
    "/*@ sourceURL",
];

/// The unanchored half of the same alternation.
const SOURCE_URL_NEEDLE: &str = "sourceURL=";

/// Run the `generated.rb` predicates that read bytes.
///
/// `minified_files?` and `has_source_map?` are open-coded because they are the
/// only two upstream predicates that are not "a marker at a known line":
/// one is arithmetic over every line, the other an alternation between an
/// anchored and an unanchored pattern.
fn generated_by_content(relative: &RelativePath, lines: &[String]) -> Option<ContentEvidence> {
    for rule in GENERATED_CONTENT_RULES {
        if let Some(detail) = rule.matches(relative, lines) {
            return Some(ContentEvidence::LinguistGenerated {
                predicate: LINGUIST_GENERATED_PREDICATES[rule.predicate],
                via: GeneratedVia::Content,
                detail,
            });
        }
    }

    let base = relative.basename();
    let maybe_minified = MAYBE_MINIFIED
        .iter()
        .any(|extension| base.len() > extension.len() && ends_with_ci(base, extension));
    if !maybe_minified || lines.is_empty() {
        return None;
    }

    let total: usize = lines.iter().map(String::len).sum();
    let mean = total / lines.len();
    if mean > MINIFIED_MEAN_LINE_LENGTH {
        return Some(ContentEvidence::LinguistGenerated {
            predicate: LINGUIST_GENERATED_PREDICATES[MINIFIED_PREDICATE],
            via: GeneratedVia::Content,
            detail: format!(
                "mean line length {mean} over {} lines, above the {MINIFIED_MEAN_LINE_LENGTH}-character threshold",
                lines.len()
            ),
        });
    }

    let tail = lines.len().saturating_sub(2);
    for line in &lines[tail..] {
        let anchored = SOURCE_MAP_PREFIXES
            .iter()
            .any(|prefix| line.starts_with(prefix));
        if anchored || line.contains(SOURCE_URL_NEEDLE) {
            return Some(ContentEvidence::LinguistGenerated {
                predicate: LINGUIST_GENERATED_PREDICATES[SOURCE_MAP_PREDICATE],
                via: GeneratedVia::Content,
                detail: line.trim().to_string(),
            });
        }
    }
    None
}

fn ends_with_ci(haystack: &str, suffix: &str) -> bool {
    haystack.len() >= suffix.len()
        && haystack[haystack.len() - suffix.len()..].eq_ignore_ascii_case(suffix)
}

// ---------------------------------------------------------------------------
// .gitattributes
// ---------------------------------------------------------------------------

/// The three attributes this module reads.
///
/// `filter=lfs` is not a Linguist attribute at all; it is here because §6.12
/// lists LFS-tracked files as a counter-signal and §9.3 2b reads the same line
/// for the same reason. A pointer file whose real content is on an LFS server is
/// not a file whose absence git can undo.
const ATTRIBUTE_NAMES: [&str; 3] = ["linguist-vendored", "linguist-generated", "filter=lfs"];

/// One `.gitattributes` line, resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
struct AttributeRule {
    /// Repo-relative directory the pattern is relative to; `""` at the root.
    directory: String,
    /// Depth of `directory`, which is git's precedence order.
    depth: usize,
    /// Line number within the file, which is precedence within one file.
    line: usize,
    /// Repo-relative path of the declaring file.
    declared_in: PathBuf,
    /// The pattern as written.
    pattern: String,
    /// Which attribute the line sets or unsets.
    attribute: &'static str,
    /// `true` sets, `false` unsets — and an unset *rescues*.
    set: bool,
}

impl AttributeRule {
    fn matches(&self, relative: &RelativePath) -> bool {
        let Some(rest) = strip_directory(relative.as_str(), &self.directory) else {
            return false;
        };
        glob_matches(&self.pattern, rest)
    }
}

/// The part of `path` below `directory`, or `None` if it is not below it.
fn strip_directory<'a>(path: &'a str, directory: &str) -> Option<&'a str> {
    if directory.is_empty() {
        return Some(path);
    }
    let rest = path.strip_prefix(directory)?;
    rest.strip_prefix('/')
}

/// Match one gitattributes pattern against a path relative to the declaring
/// directory.
///
/// git's own rules, with the exceptions git documents for attributes: negation
/// is forbidden, and **a pattern that matches a directory does not recursively
/// match the paths inside it** — so `libs/` marks the directory entry and
/// nothing else, while `libs/**` marks the contents. That distinction is not a
/// pedantic one: writing the first and expecting the second is a common enough
/// mistake that reproducing git's behaviour is the only defensible choice.
fn glob_matches(pattern: &str, path: &str) -> bool {
    if pattern.ends_with('/') {
        // Directory-only, and this gate judges files.
        return false;
    }
    let anchored = pattern.contains('/');
    let pattern = pattern.strip_prefix('/').unwrap_or(pattern);
    if anchored {
        glob_segment(pattern.as_bytes(), path.as_bytes())
    } else {
        let basename = path.rsplit('/').next().unwrap_or(path);
        glob_segment(pattern.as_bytes(), basename.as_bytes())
    }
}

/// Backtracking glob: `*` and `?` do not cross `/`, `**` does.
fn glob_segment(pattern: &[u8], text: &[u8]) -> bool {
    if pattern.is_empty() {
        return text.is_empty();
    }
    match pattern[0] {
        b'*' if pattern.len() > 1 && pattern[1] == b'*' => {
            let mut rest = &pattern[2..];
            if rest.first() == Some(&b'/') {
                // `**/` also matches zero directories.
                if glob_segment(&rest[1..], text) {
                    return true;
                }
                rest = &rest[1..];
            }
            for split in 0..=text.len() {
                if glob_segment(rest, &text[split..]) {
                    return true;
                }
            }
            false
        }
        b'*' => {
            for split in 0..=text.len() {
                if text[..split].contains(&b'/') {
                    break;
                }
                if glob_segment(&pattern[1..], &text[split..]) {
                    return true;
                }
            }
            false
        }
        b'?' => !text.is_empty() && text[0] != b'/' && glob_segment(&pattern[1..], &text[1..]),
        b'[' => {
            let Some(close) = pattern.iter().position(|byte| *byte == b']') else {
                return false;
            };
            if text.is_empty() {
                return false;
            }
            let mut class = &pattern[1..close];
            let negated = class.first() == Some(&b'!') || class.first() == Some(&b'^');
            if negated {
                class = &class[1..];
            }
            let mut hit = false;
            let mut index = 0;
            while index < class.len() {
                if index + 2 < class.len() && class[index + 1] == b'-' {
                    if text[0] >= class[index] && text[0] <= class[index + 2] {
                        hit = true;
                    }
                    index += 3;
                } else {
                    if text[0] == class[index] {
                        hit = true;
                    }
                    index += 1;
                }
            }
            hit != negated && glob_segment(&pattern[close + 1..], &text[1..])
        }
        b'\\' if pattern.len() > 1 => {
            !text.is_empty() && text[0] == pattern[1] && glob_segment(&pattern[2..], &text[1..])
        }
        literal => {
            !text.is_empty() && text[0] == literal && glob_segment(&pattern[1..], &text[1..])
        }
    }
}

/// Parse one `.gitattributes` into [`AttributeRule`]s.
fn parse_attributes(text: &str, declared_in: &Path, directory: &str, out: &mut Vec<AttributeRule>) {
    let depth = directory.split('/').filter(|part| !part.is_empty()).count();
    for (line, raw) in text.lines().enumerate() {
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let mut tokens = trimmed.split_whitespace();
        let Some(pattern) = tokens.next() else {
            continue;
        };
        let pattern = pattern.trim_matches('"');
        for token in tokens {
            for attribute in ATTRIBUTE_NAMES {
                let Some(set) = attribute_state(token, attribute) else {
                    continue;
                };
                out.push(AttributeRule {
                    directory: directory.to_string(),
                    depth,
                    line,
                    declared_in: declared_in.to_path_buf(),
                    pattern: pattern.to_string(),
                    attribute,
                    set,
                });
            }
        }
    }
}

/// Does `token` set or unset `attribute`?
///
/// git spells "set" as the bare name or `=true`, and "unset" as a `-` prefix or
/// `=false`; `!` means unspecified, which for our purposes is the same as
/// unset — the repository has declined to say, so our own tables decide.
fn attribute_state(token: &str, attribute: &'static str) -> Option<bool> {
    let (name, value) = match token.split_once('=') {
        Some((name, value)) => (name, Some(value)),
        None => (token, None),
    };
    // `filter=lfs` is itself a name=value attribute, so it is matched whole.
    if attribute.contains('=') {
        if token == attribute {
            return Some(true);
        }
        if let Some(stripped) = token.strip_prefix('-') {
            if stripped == attribute.split('=').next()? {
                return Some(false);
            }
        }
        if name == attribute.split('=').next()? && value.is_some_and(str::is_empty) {
            return Some(false);
        }
        return None;
    }
    if name == attribute {
        return Some(match value {
            None | Some("true") => true,
            Some(_) => false,
        });
    }
    if let Some(stripped) = name.strip_prefix('-') {
        if stripped == attribute {
            return Some(false);
        }
    }
    if let Some(stripped) = name.strip_prefix('!') {
        if stripped == attribute {
            return Some(false);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// .gitmodules
// ---------------------------------------------------------------------------

/// Read the submodule paths a repository declares.
///
/// Only the root `.gitmodules` is read, because that is the only one git itself
/// consults.
fn read_submodules(root: &Path) -> Result<Vec<(String, PathBuf)>> {
    let declared_in = Path::new(".gitmodules");
    let absolute = root.join(declared_in);
    let text = match fs::read_to_string(&absolute) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => {
            return Err(Error::Io {
                path: absolute,
                source,
            })
        }
    };
    let mut out = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') || trimmed.starts_with(';') {
            continue;
        }
        let Some((key, value)) = trimmed.split_once('=') else {
            continue;
        };
        if key.trim() != "path" {
            continue;
        }
        let value = value.trim();
        if !value.is_empty() {
            out.push((value.to_string(), declared_in.to_path_buf()));
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// the .gitattributes walk
// ---------------------------------------------------------------------------

struct AttributeWalk {
    root: PathBuf,
    limit: usize,
    visited: usize,
    /// `(repo-relative path of the .gitattributes, its directory)`.
    files: Vec<(PathBuf, String)>,
    incomplete: Option<(usize, usize)>,
}

impl AttributeWalk {
    fn descend(&mut self, directory: &Path, relative: &str, depth: usize) -> Result<()> {
        if self.incomplete.is_some() {
            return Ok(());
        }
        let reader = match fs::read_dir(directory) {
            Ok(reader) => reader,
            Err(source) if depth == 0 => {
                return Err(Error::Io {
                    path: directory.to_path_buf(),
                    source,
                })
            }
            // Below the root, a directory that will not list is a directory a
            // `.gitattributes` could be hiding in.
            Err(_) => {
                self.incomplete = Some((self.visited, self.limit));
                return Ok(());
            }
        };

        let mut subdirectories = Vec::new();
        for entry in reader {
            self.visited += 1;
            if self.visited > self.limit {
                self.incomplete = Some((self.visited, self.limit));
                return Ok(());
            }
            let Ok(entry) = entry else {
                self.incomplete = Some((self.visited, self.limit));
                return Ok(());
            };
            let name = entry.file_name().to_string_lossy().into_owned();
            // `file_type` does not follow symlinks, so a symlinked directory is
            // never descended into and a loop is impossible.
            let Ok(file_type) = entry.file_type() else {
                self.incomplete = Some((self.visited, self.limit));
                return Ok(());
            };
            let child = if relative.is_empty() {
                name.clone()
            } else {
                format!("{relative}/{name}")
            };
            if file_type.is_dir() {
                // §9.3 0b, through the shared classifier rather than a name
                // comparison: a linked worktree and a submodule carry `.git` as
                // a FILE, and a bare `vendor/foo.git/` carries none at all.
                if !crate::boundary::classify(&self.root.join(&child)).stops_the_walk() {
                    subdirectories.push(child);
                }
            } else if name == ".gitattributes" {
                self.files
                    .push((PathBuf::from(&child), relative.to_string()));
            }
        }

        for child in subdirectories {
            let absolute = self.root.join(&child);
            self.descend(&absolute, &child, depth + 1)?;
            if self.incomplete.is_some() {
                return Ok(());
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// census
// ---------------------------------------------------------------------------

/// How much of Linguist's `vendor.yml` this module can express.
///
/// Returns `(patterns translated, patterns declared unsupported, patterns
/// upstream)`. The first two must sum to the third: a pattern that is neither
/// translated nor declared would be one that was quietly dropped, which is the
/// failure mode vendoring exists to prevent.
#[must_use]
pub fn vendor_rule_census() -> (usize, usize, usize) {
    let mut translated: Vec<usize> = VENDOR_MATCHERS.iter().map(|(index, _)| *index).collect();
    translated.sort_unstable();
    translated.dedup();
    (
        translated.len(),
        VENDOR_UNSUPPORTED.len(),
        LINGUIST_VENDOR_PATTERNS.len(),
    )
}

/// The same census for `generated.rb`.
#[must_use]
pub fn generated_rule_census() -> (usize, usize, usize) {
    let mut translated: Vec<usize> = GENERATED_PATH_MATCHERS
        .iter()
        .map(|(index, _)| *index)
        .chain(GENERATED_CONTENT_RULES.iter().map(|rule| rule.predicate))
        .chain([MINIFIED_PREDICATE, SOURCE_MAP_PREDICATE])
        .collect();
    translated.sort_unstable();
    translated.dedup();
    (
        translated.len(),
        GENERATED_UNSUPPORTED.len(),
        LINGUIST_GENERATED_PREDICATES.len(),
    )
}

// ---------------------------------------------------------------------------
// Linguist generated.rb — the tables
// ---------------------------------------------------------------------------

/// Every predicate in upstream `generated.rb`'s `generated?` chain, verbatim and
/// in upstream order.
///
/// Vendored from `github/linguist@af6f772786199696e4d07d618c9c5b625a1a03f0`,
/// `lib/linguist/generated.rb` (MIT). The index of a name here is its identity
/// everywhere else in this module.
pub const LINGUIST_GENERATED_PREDICATES: [&str; 74] = [
    "xcode_file?",
    "intellij_file?",
    "cocoapods?",
    "carthage_build?",
    "generated_graphql_relay?",
    "generated_net_designer_file?",
    "generated_net_specflow_feature_file?",
    "composer_lock?",
    "cargo_lock?",
    "cargo_orig?",
    "deno_lock?",
    "flake_lock?",
    "bazel_lock?",
    "node_modules?",
    "go_vendor?",
    "go_lock?",
    "package_resolved?",
    "poetry_lock?",
    "pdm_lock?",
    "uv_lock?",
    "pixi_lock?",
    "esy_lock?",
    "npm_shrinkwrap_or_package_lock?",
    "pnpm_lock?",
    "bun_lock?",
    "terraform_lock?",
    "generated_yarn_plugnplay?",
    "godeps?",
    "generated_by_zephir?",
    "htmlcov?",
    "minified_files?",
    "has_source_map?",
    "source_map?",
    "compiled_coffeescript?",
    "generated_parser?",
    "generated_net_docfile?",
    "generated_postscript?",
    "compiled_cython_file?",
    "pipenv_lock?",
    "gradle_wrapper?",
    "maven_wrapper?",
    "mise_lock?",
    "secrets_baseline?",
    "julia_manifest?",
    "generated_go?",
    "generated_protocol_buffer_from_go?",
    "generated_protocol_buffer?",
    "generated_javascript_protocol_buffer?",
    "generated_typescript_protocol_buffer?",
    "generated_twirp_ruby?",
    "generated_apache_thrift?",
    "generated_jni_header?",
    "vcr_cassette?",
    "generated_antlr?",
    "generated_module?",
    "generated_unity3d_meta?",
    "generated_racc?",
    "generated_jflex?",
    "generated_grammarkit?",
    "generated_roxygen2?",
    "generated_html?",
    "generated_jison?",
    "generated_grpc_cpp?",
    "generated_dart?",
    "generated_perl_ppport_header?",
    "generated_gamemakerstudio?",
    "generated_gimp?",
    "generated_visualstudio6?",
    "generated_haxe?",
    "generated_jooq?",
    "generated_pascal_tlb?",
    "generated_sorbet_rbi?",
    "generated_mysql_view_definition_format?",
    "generated_sqlx_query?",
];

/// Predicates this module does **not** implement, by index into
/// [`LINGUIST_GENERATED_PREDICATES`], each with why.
///
/// Every one of them needs something a literal matcher cannot express — a
/// character class, an unbounded wildcard, or a scoring heuristic — and this
/// crate carries no regex engine. Listing them is the point: an approximated
/// rule that quietly under-matches is worse than a rule that is absent and
/// known to be absent.
pub const GENERATED_UNSUPPORTED: [(usize, &str); 13] = [
    (
        14,
        "go_vendor? — matches a domain-shaped path component (`\\.(com|edu|…)`)",
    ),
    (21, "esy_lock? — optional `(\\w+\\.)?` prefix"),
    (
        32,
        "source_map? — content alternative needs `^{\"version\":\\d+,`",
    ),
    (
        33,
        "compiled_coffeescript? — a scoring heuristic over every line",
    ),
    (
        34,
        "generated_parser? — a regex over the first five lines joined",
    ),
    (
        36,
        "generated_postscript? — `%%Creator:` alternation over ten lines",
    ),
    (41, "mise_lock? — optional `(\\.[^/]+)?` infix"),
    (43, "julia_manifest? — optional `(-v\\d+\\.\\d+)?` infix"),
    (
        60,
        "generated_html? — parses `<meta name=generator>` attributes",
    ),
    (
        63,
        "generated_dart? — `generated code\\W{2,3}do not modify`",
    ),
    (
        65,
        "generated_gamemakerstudio? — `^\\d\\.\\d\\.\\d.+\\|\\{`",
    ),
    (
        66,
        "generated_gimp? — `GIMP [a-zA-Z0-9\\- ]+ C-Source image dump`",
    ),
    (73, "generated_sqlx_query? — `query-[a-f\\d]{64}\\.json`"),
];

/// `generated.rb` predicates that decide on the pathname alone.
const GENERATED_PATH_MATCHERS: &[(usize, Matcher)] = &[
    // xcode_file? — `['.nib', '.xcworkspacedata', '.xcuserstate'].include?(extname)`
    (0, Matcher::BasenameSuffix(".nib")),
    (0, Matcher::BasenameSuffix(".xcworkspacedata")),
    (0, Matcher::BasenameSuffix(".xcuserstate")),
    // intellij_file? — `(?:^|\/)\.idea\/`
    (1, Matcher::Anchored(".idea/")),
    // cocoapods? — `(^Pods|\/Pods)\/`
    (2, Matcher::Anchored("Pods/")),
    // carthage_build? — `(^|\/)Carthage\/Build\/`
    (3, Matcher::Anchored("Carthage/Build/")),
    // generated_graphql_relay? — `__generated__\/`
    (4, Matcher::Contains("__generated__/")),
    // generated_net_designer_file? — `\.designer\.(cs|vb)$` /i
    (5, Matcher::BasenameSuffixCi(".designer.cs")),
    (5, Matcher::BasenameSuffixCi(".designer.vb")),
    // generated_net_specflow_feature_file? — `\.feature\.cs$` /i
    (6, Matcher::BasenameSuffixCi(".feature.cs")),
    // composer_lock? — `composer\.lock`
    (7, Matcher::Contains("composer.lock")),
    // cargo_lock? — `Cargo\.lock`
    (8, Matcher::Contains("Cargo.lock")),
    // cargo_orig? — `Cargo\.toml\.orig`
    (9, Matcher::Contains("Cargo.toml.orig")),
    // deno_lock? — `deno\.lock`
    (10, Matcher::Contains("deno.lock")),
    // flake_lock? — `(^|\/)flake\.lock$`
    (11, Matcher::BasenameExact("flake.lock")),
    // bazel_lock? — `(^|\/)MODULE\.bazel\.lock$`
    (12, Matcher::BasenameExact("MODULE.bazel.lock")),
    // node_modules? — `node_modules\/`
    (13, Matcher::Contains("node_modules/")),
    // go_lock? — `(Gopkg|glide)\.lock`
    (15, Matcher::Contains("Gopkg.lock")),
    (15, Matcher::Contains("glide.lock")),
    // package_resolved? — `Package\.resolved`
    (16, Matcher::Contains("Package.resolved")),
    // poetry_lock? — `poetry\.lock`
    (17, Matcher::Contains("poetry.lock")),
    // pdm_lock? — `pdm\.lock`
    (18, Matcher::Contains("pdm.lock")),
    // uv_lock? — `uv\.lock`
    (19, Matcher::Contains("uv.lock")),
    // pixi_lock? — `pixi\.lock`
    (20, Matcher::Contains("pixi.lock")),
    // npm_shrinkwrap_or_package_lock? — `npm-shrinkwrap\.json` | `package-lock\.json`
    (22, Matcher::Contains("npm-shrinkwrap.json")),
    (22, Matcher::Contains("package-lock.json")),
    // pnpm_lock? — `pnpm-lock\.yaml`
    (23, Matcher::Contains("pnpm-lock.yaml")),
    // bun_lock? — `(?:^|\/)bun\.lockb?$`
    (24, Matcher::BasenameExact("bun.lock")),
    (24, Matcher::BasenameExact("bun.lockb")),
    // terraform_lock? — `(?:^|\/)\.terraform\.lock\.hcl$`
    (25, Matcher::BasenameExact(".terraform.lock.hcl")),
    // generated_yarn_plugnplay? — `(^|\/)\.pnp\..*$`
    (26, Matcher::BasenamePrefix(".pnp.")),
    // godeps? — `Godeps\/`
    (27, Matcher::Contains("Godeps/")),
    // generated_by_zephir? — `.\.zep\.(?:c|h|php)$`
    (28, Matcher::BasenameSuffix(".zep.c")),
    (28, Matcher::BasenameSuffix(".zep.h")),
    (28, Matcher::BasenameSuffix(".zep.php")),
    // htmlcov? — `(?:^|\/)htmlcov\/`
    (29, Matcher::Anchored("htmlcov/")),
    // pipenv_lock? — `Pipfile\.lock`
    (38, Matcher::Contains("Pipfile.lock")),
    // gradle_wrapper? — `(?:^|\/)gradlew(?:\.bat)?$` /i
    (39, Matcher::BasenameExactCi("gradlew")),
    (39, Matcher::BasenameExactCi("gradlew.bat")),
    // maven_wrapper? — `(?:^|\/)mvnw(?:\.cmd)?$` /i
    (40, Matcher::BasenameExactCi("mvnw")),
    (40, Matcher::BasenameExactCi("mvnw.cmd")),
    // secrets_baseline? — `(?:^|\/)\.secrets\.baseline$`
    (42, Matcher::BasenameExact(".secrets.baseline")),
    // generated_pascal_tlb? — `_tlb\.pas$` /i
    (70, Matcher::BasenameSuffixCi("_tlb.pas")),
];

/// `generated.rb` predicates that read the file.
///
/// Transcribed one-for-one from upstream: the extension guard, the
/// `lines.count >` guard, and the literal markers at the line positions the Ruby
/// indexes.
const GENERATED_CONTENT_RULES: &[GeneratedContentRule] = &[
    GeneratedContentRule {
        predicate: 35, // generated_net_docfile?
        suffixes: &[".xml"],
        suffix_case_insensitive: true,
        min_lines: 3,
        checks: &[
            (LineSelector::At(1), TextMatch::ContainsAll(&["<doc>"])),
            (LineSelector::At(2), TextMatch::ContainsAll(&["<assembly>"])),
            (
                LineSelector::FromEnd(2),
                TextMatch::ContainsAll(&["</doc>"]),
            ),
        ],
    },
    GeneratedContentRule {
        predicate: 37, // compiled_cython_file?
        suffixes: &[".c", ".cpp"],
        suffix_case_insensitive: false,
        min_lines: 1,
        checks: &[(
            LineSelector::At(0),
            TextMatch::ContainsAll(&["Generated by Cython"]),
        )],
    },
    GeneratedContentRule {
        predicate: 44, // generated_go?
        suffixes: &[".go"],
        suffix_case_insensitive: false,
        min_lines: 1,
        checks: &[(
            LineSelector::First(40),
            TextMatch::StartsWithAny(&["// Code generated "]),
        )],
    },
    GeneratedContentRule {
        predicate: 45, // generated_protocol_buffer_from_go?
        suffixes: &[".proto"],
        suffix_case_insensitive: false,
        min_lines: 1,
        checks: &[(
            LineSelector::First(20),
            TextMatch::ContainsAll(&["This file was autogenerated by go-to-protobuf"]),
        )],
    },
    GeneratedContentRule {
        predicate: 46, // generated_protocol_buffer?
        suffixes: &[".py", ".java", ".h", ".cc", ".cpp", ".m", ".rb", ".php"],
        suffix_case_insensitive: false,
        min_lines: 1,
        checks: &[(
            LineSelector::First(3),
            TextMatch::ContainsAll(&["Generated by the protocol buffer compiler.  DO NOT EDIT!"]),
        )],
    },
    GeneratedContentRule {
        predicate: 47, // generated_javascript_protocol_buffer?
        suffixes: &[".js"],
        suffix_case_insensitive: false,
        min_lines: 6,
        checks: &[(
            LineSelector::At(5),
            TextMatch::ContainsAll(&["GENERATED CODE -- DO NOT EDIT!"]),
        )],
    },
    GeneratedContentRule {
        predicate: 48, // generated_typescript_protocol_buffer?
        suffixes: &[".ts"],
        suffix_case_insensitive: false,
        min_lines: 4,
        checks: &[(
            LineSelector::At(0),
            TextMatch::ContainsAll(&["Code generated by protoc-gen-ts_proto. DO NOT EDIT."]),
        )],
    },
    GeneratedContentRule {
        predicate: 49, // generated_twirp_ruby?
        suffixes: &[".rb"],
        suffix_case_insensitive: false,
        min_lines: 1,
        checks: &[(
            LineSelector::First(3),
            TextMatch::ContainsAll(&["Code generated by protoc-gen-twirp_ruby", "DO NOT EDIT."]),
        )],
    },
    GeneratedContentRule {
        predicate: 50, // generated_apache_thrift?
        suffixes: &[
            ".rb", ".py", ".go", ".js", ".m", ".java", ".h", ".cc", ".cpp", ".php",
        ],
        suffix_case_insensitive: false,
        min_lines: 0,
        checks: &[(
            LineSelector::First(6),
            TextMatch::ContainsAll(&["Autogenerated by Thrift Compiler"]),
        )],
    },
    GeneratedContentRule {
        predicate: 51, // generated_jni_header?
        suffixes: &[".h"],
        suffix_case_insensitive: false,
        min_lines: 2,
        checks: &[
            (
                LineSelector::At(0),
                TextMatch::ContainsAll(&["/* DO NOT EDIT THIS FILE - it is machine generated */"]),
            ),
            (
                LineSelector::At(1),
                TextMatch::ContainsAll(&["#include <jni.h>"]),
            ),
        ],
    },
    GeneratedContentRule {
        predicate: 52, // vcr_cassette?
        suffixes: &[".yml"],
        suffix_case_insensitive: false,
        min_lines: 2,
        checks: &[(
            LineSelector::FromEnd(2),
            TextMatch::ContainsAll(&["recorded_with: VCR"]),
        )],
    },
    GeneratedContentRule {
        predicate: 53, // generated_antlr?
        suffixes: &[".g"],
        suffix_case_insensitive: false,
        min_lines: 2,
        checks: &[(
            LineSelector::At(1),
            TextMatch::ContainsAll(&["generated by Xtest"]),
        )],
    },
    GeneratedContentRule {
        predicate: 54, // generated_module?
        suffixes: &[".mod"],
        suffix_case_insensitive: false,
        min_lines: 1,
        checks: &[(
            LineSelector::At(0),
            TextMatch::ContainsAny(&["PCBNEW-LibModule-V", "GFORTRAN module version '"]),
        )],
    },
    GeneratedContentRule {
        predicate: 55, // generated_unity3d_meta?
        suffixes: &[".meta"],
        suffix_case_insensitive: false,
        min_lines: 1,
        checks: &[(
            LineSelector::At(0),
            TextMatch::ContainsAll(&["fileFormatVersion: "]),
        )],
    },
    GeneratedContentRule {
        predicate: 56, // generated_racc?
        suffixes: &[".rb"],
        suffix_case_insensitive: false,
        min_lines: 2,
        checks: &[(
            LineSelector::At(2),
            TextMatch::StartsWithAny(&["# This file is automatically generated by Racc"]),
        )],
    },
    GeneratedContentRule {
        predicate: 57, // generated_jflex?
        suffixes: &[".java"],
        suffix_case_insensitive: false,
        min_lines: 1,
        checks: &[(
            LineSelector::At(0),
            TextMatch::StartsWithAny(&["/* The following code was generated by JFlex "]),
        )],
    },
    GeneratedContentRule {
        predicate: 58, // generated_grammarkit?
        suffixes: &[".java"],
        suffix_case_insensitive: false,
        min_lines: 1,
        checks: &[(
            LineSelector::At(0),
            TextMatch::StartsWithAny(&[
                "// This is a generated file. Not intended for manual editing.",
            ]),
        )],
    },
    GeneratedContentRule {
        predicate: 59, // generated_roxygen2?
        suffixes: &[".Rd"],
        suffix_case_insensitive: false,
        min_lines: 1,
        checks: &[(
            LineSelector::At(0),
            TextMatch::ContainsAll(&["% Generated by roxygen2: do not edit by hand"]),
        )],
    },
    GeneratedContentRule {
        predicate: 61, // generated_jison?
        suffixes: &[".js"],
        suffix_case_insensitive: false,
        min_lines: 1,
        checks: &[(
            LineSelector::At(0),
            TextMatch::StartsWithAny(&[
                "/* parser generated by jison ",
                "/* generated by jison-lex ",
            ]),
        )],
    },
    GeneratedContentRule {
        predicate: 62, // generated_grpc_cpp?
        suffixes: &[".cpp", ".hpp", ".h", ".cc"],
        suffix_case_insensitive: false,
        min_lines: 1,
        checks: &[(
            LineSelector::At(0),
            TextMatch::StartsWithAny(&["// Generated by the gRPC"]),
        )],
    },
    GeneratedContentRule {
        predicate: 64, // generated_perl_ppport_header?
        suffixes: &["ppport.h"],
        suffix_case_insensitive: false,
        min_lines: 10,
        checks: &[(
            LineSelector::At(8),
            TextMatch::ContainsAll(&["Automatically created by Devel::PPPort"]),
        )],
    },
    GeneratedContentRule {
        predicate: 67, // generated_visualstudio6?
        suffixes: &[".dsp"],
        suffix_case_insensitive: true,
        min_lines: 0,
        checks: &[(
            LineSelector::First(3),
            TextMatch::ContainsAll(&["# Microsoft Developer Studio Generated Build File"]),
        )],
    },
    GeneratedContentRule {
        predicate: 68, // generated_haxe?
        suffixes: &[".js", ".py", ".lua", ".cpp", ".h", ".java", ".cs", ".php"],
        suffix_case_insensitive: false,
        min_lines: 0,
        checks: &[(
            LineSelector::First(3),
            TextMatch::ContainsAll(&["Generated by Haxe"]),
        )],
    },
    GeneratedContentRule {
        predicate: 69, // generated_jooq?
        suffixes: &[".java"],
        suffix_case_insensitive: true,
        min_lines: 0,
        checks: &[(
            LineSelector::First(2),
            TextMatch::ContainsAll(&["This file is generated by jOOQ."]),
        )],
    },
    GeneratedContentRule {
        predicate: 71, // generated_sorbet_rbi?
        suffixes: &[".rbi"],
        suffix_case_insensitive: true,
        // Upstream guards with `lines.count >= 5`.
        min_lines: 4,
        checks: &[
            (LineSelector::At(0), TextMatch::StartsWithAny(&["# typed:"])),
            (
                LineSelector::At(2),
                TextMatch::ContainsAll(&["DO NOT EDIT MANUALLY"]),
            ),
            (
                LineSelector::At(4),
                TextMatch::StartsWithAny(&[
                    "# Please run `bin/tapioca",
                    "# Please instead update this file by running `bin/tapioca",
                ]),
            ),
        ],
    },
    GeneratedContentRule {
        predicate: 72, // generated_mysql_view_definition_format?
        suffixes: &[".frm"],
        suffix_case_insensitive: true,
        // Upstream indexes `lines[0]` with no guard, which raises on an empty
        // file; requiring one line is the same answer without the exception.
        min_lines: 0,
        checks: &[(LineSelector::At(0), TextMatch::ContainsAll(&["TYPE=VIEW"]))],
    },
];

// ---------------------------------------------------------------------------
// Linguist vendor.yml — the tables
// ---------------------------------------------------------------------------

/// Every pattern in upstream `vendor.yml`, verbatim and in upstream order.
///
/// Vendored from `github/linguist@af6f772786199696e4d07d618c9c5b625a1a03f0`, `lib/linguist/vendor.yml` (MIT). The
/// index of a pattern here is its identity everywhere else in this module, and
/// [`ContentEvidence::LinguistVendored`] quotes the string so a surprising veto
/// can be checked against the upstream file rather than against this
/// translation of it.
pub const LINGUIST_VENDOR_PATTERNS: [&str; 168] = [
    "(^|/)cache/",
    "^[Dd]ependencies/",
    "(^|/)dist/",
    "^deps/",
    "(^|/)configure$",
    "(^|/)config\\.guess$",
    "(^|/)config\\.sub$",
    "(^|/)aclocal\\.m4",
    "(^|/)libtool\\.m4",
    "(^|/)ltoptions\\.m4",
    "(^|/)ltsugar\\.m4",
    "(^|/)ltversion\\.m4",
    "(^|/)lt~obsolete\\.m4",
    "(^|/)dotnet-install\\.(ps1|sh)$",
    "(^|/)cpplint\\.py",
    "(^|/)node_modules/",
    "(^|/)\\.yarn/releases/",
    "(^|/)\\.yarn/plugins/",
    "(^|/)\\.yarn/sdks/",
    "(^|/)\\.yarn/versions/",
    "(^|/)\\.yarn/unplugged/",
    "(^|/)_esy$",
    "(^|/)bower_components/",
    "^rebar$",
    "(^|/)erlang\\.mk",
    "(^|/)Godeps/_workspace/",
    "(^|/)testdata/",
    "(^|/)\\.indent\\.pro",
    "(\\.|-)min\\.(js|css)$",
    "([^\\s]*)import\\.(css|less|scss|styl)$",
    "(^|/)bootstrap([^/.]*)(\\..*)?\\.(js|css|less|scss|styl)$",
    "(^|/)custom\\.bootstrap([^\\s]*)(js|css|less|scss|styl)$",
    "(^|/)font-?awesome\\.(css|less|scss|styl)$",
    "(^|/)font-?awesome/.*\\.(css|less|scss|styl)$",
    "(^|/)foundation\\.(css|less|scss|styl)$",
    "(^|/)normalize\\.(css|less|scss|styl)$",
    "(^|/)skeleton\\.(css|less|scss|styl)$",
    "(^|/)[Bb]ourbon/.*\\.(css|less|scss|styl)$",
    "(^|/)animate\\.(css|less|scss|styl)$",
    "(^|/)materialize\\.(css|less|scss|styl|js)$",
    "(^|/)select2/.*\\.(css|scss|js)$",
    "(^|/)bulma\\.(css|sass|scss)$",
    "(3rd|[Tt]hird)[-_]?[Pp]arty/",
    "(^|/)vendors?/",
    "(^|/)[Ee]xtern(als?)?/",
    "(^|/)[Vv]+endor/",
    "^debian/",
    "(^|/)run\\.n$",
    "(^|/)bootstrap-datepicker/",
    "(^|/)jquery([^.]*)\\.js$",
    "(^|/)jquery\\-\\d\\.\\d+(\\.\\d+)?\\.js$",
    "(^|/)jquery\\-ui(\\-\\d\\.\\d+(\\.\\d+)?)?(\\.\\w+)?\\.(js|css)$",
    "(^|/)jquery\\.(ui|effects)\\.([^.]*)\\.(js|css)$",
    "(^|/)jquery\\.fn\\.gantt\\.js",
    "(^|/)jquery\\.fancybox\\.(js|css)",
    "(^|/)fuelux\\.js",
    "(^|/)jquery\\.fileupload(-\\w+)?\\.js$",
    "(^|/)jquery\\.dataTables\\.js",
    "(^|/)bootbox\\.js",
    "(^|/)pdf\\.worker\\.js",
    "(^|/)slick\\.\\w+.js$",
    "(^|/)Leaflet\\.Coordinates-\\d+\\.\\d+\\.\\d+\\.src\\.js$",
    "(^|/)leaflet\\.draw-src\\.js",
    "(^|/)leaflet\\.draw\\.css",
    "(^|/)Control\\.FullScreen\\.css",
    "(^|/)Control\\.FullScreen\\.js",
    "(^|/)leaflet\\.spin\\.js",
    "(^|/)wicket-leaflet\\.js",
    "(^|/)\\.sublime-project",
    "(^|/)\\.sublime-workspace",
    "(^|/)\\.vscode/",
    "(^|/)prototype(.*)\\.js$",
    "(^|/)effects\\.js$",
    "(^|/)controls\\.js$",
    "(^|/)dragdrop\\.js$",
    "(.*?)\\.d\\.ts$",
    "(^|/)mootools([^.]*)\\d+\\.\\d+.\\d+([^.]*)\\.js$",
    "(^|/)dojo\\.js$",
    "(^|/)MochiKit\\.js$",
    "(^|/)yahoo-([^.]*)\\.js$",
    "(^|/)yui([^.]*)\\.js$",
    "(^|/)ckeditor\\.js$",
    "(^|/)tiny_mce([^.]*)\\.js$",
    "(^|/)tiny_mce/(langs|plugins|themes|utils)",
    "(^|/)ace-builds/",
    "(^|/)fontello(.*?)\\.css$",
    "(^|/)MathJax/",
    "(^|/)Chart\\.js$",
    "(^|/)[Cc]ode[Mm]irror/(\\d+\\.\\d+/)?(lib|mode|theme|addon|keymap|demo)",
    "(^|/)shBrush([^.]*)\\.js$",
    "(^|/)shCore\\.js$",
    "(^|/)shLegacy\\.js$",
    "(^|/)angular([^.]*)\\.js$",
    "(^|\\/)d3(\\.v\\d+)?([^.]*)\\.js$",
    "(^|/)react(-[^.]*)?\\.js$",
    "(^|/)flow-typed/.*\\.js$",
    "(^|/)modernizr\\-\\d\\.\\d+(\\.\\d+)?\\.js$",
    "(^|/)modernizr\\.custom\\.\\d+\\.js$",
    "(^|/)knockout-(\\d+\\.){3}(debug\\.)?js$",
    "(^|/)docs?/_?(build|themes?|templates?|static)/",
    "(^|/)admin_media/",
    "(^|/)env/",
    "(^|/)fabfile\\.py$",
    "(^|/)waf$",
    "(^|/)\\.osx$",
    "\\.xctemplate/",
    "\\.imageset/",
    "(^|/)Carthage/",
    "(^|/)Sparkle/",
    "(^|/)Crashlytics\\.framework/",
    "(^|/)Fabric\\.framework/",
    "(^|/)BuddyBuildSDK\\.framework/",
    "(^|/)Realm\\.framework",
    "(^|/)RealmSwift\\.framework",
    "(^|/)\\.gitattributes$",
    "(^|/)\\.gitignore$",
    "(^|/)\\.gitmodules$",
    "(^|/)gradlew$",
    "(^|/)gradlew\\.bat$",
    "(^|/)gradle/wrapper/",
    "(^|/)mvnw$",
    "(^|/)mvnw\\.cmd$",
    "(^|/)\\.mvn/wrapper/",
    "-vsdoc\\.js$",
    "\\.intellisense\\.js$",
    "(^|/)jquery([^.]*)\\.validate(\\.unobtrusive)?\\.js$",
    "(^|/)jquery([^.]*)\\.unobtrusive\\-ajax\\.js$",
    "(^|/)[Mm]icrosoft([Mm]vc)?([Aa]jax|[Vv]alidation)(\\.debug)?\\.js$",
    "(^|/)[Pp]ackages\\/.+\\.\\d+\\/",
    "(^|/)extjs/.*?\\.js$",
    "(^|/)extjs/.*?\\.xml$",
    "(^|/)extjs/.*?\\.txt$",
    "(^|/)extjs/.*?\\.html$",
    "(^|/)extjs/.*?\\.properties$",
    "(^|/)extjs/\\.sencha/",
    "(^|/)extjs/docs/",
    "(^|/)extjs/builds/",
    "(^|/)extjs/cmd/",
    "(^|/)extjs/examples/",
    "(^|/)extjs/locale/",
    "(^|/)extjs/packages/",
    "(^|/)extjs/plugins/",
    "(^|/)extjs/resources/",
    "(^|/)extjs/src/",
    "(^|/)extjs/welcome/",
    "(^|/)html5shiv\\.js$",
    "(^|/)[Tt]ests?/fixtures/",
    "(^|/)[Ss]pecs?/fixtures/",
    "(^|/)cordova([^.]*)\\.js$",
    "(^|/)cordova\\-\\d\\.\\d(\\.\\d)?\\.js$",
    "(^|/)foundation(\\..*)?\\.js$",
    "(^|/)Vagrantfile$",
    "(^|/)\\.[Dd][Ss]_[Ss]tore$",
    "(^|/)inst/extdata/",
    "(^|/)octicons\\.css",
    "(^|/)sprockets-octicons\\.scss",
    "(^|/)activator$",
    "(^|/)activator\\.bat$",
    "(^|/)proguard\\.pro$",
    "(^|/)proguard-rules\\.pro$",
    "(^|/)puphpet/",
    "(^|/)\\.google_apis/",
    "(^|/)Jenkinsfile$",
    "(^|/)\\.gitpod\\.Dockerfile$",
    "(^|/)\\.github/",
    "(^|/)\\.obsidian/",
    "(^|/)\\.teamcity/",
    "(^|/)xvba_modules/",
];

/// Patterns this module does **not** match, by index into
/// [`LINGUIST_VENDOR_PATTERNS`], each with why.
///
/// All of them need a regex engine this crate does not carry: an unbounded
/// wildcard, a negated character class, or a digit shorthand. They are almost
/// entirely bundled-JavaScript filename rules (`jquery([^.]*)\.js$`,
/// `angular([^.]*)\.js$`, the `extjs/.*?` family), which cost recall of the
/// vendored class and nothing else — a missed vendored file is a file the tool
/// declines to report, never one it wrongly deletes.
pub const VENDOR_UNSUPPORTED: [(usize, &str); 41] = [
    (29, "negated class"),        // ([^\s]*)import\.(css|less|scss|styl)$
    (30, "negated class"),        // (^|/)bootstrap([^/.]*)(\..*)?\.(js|css|less|scss|styl)$
    (31, "negated class"),        // (^|/)custom\.bootstrap([^\s]*)(js|css|less|scss|styl)$
    (33, "metacharacter '.'"),    // (^|/)font-?awesome/.*\.(css|less|scss|styl)$
    (37, "metacharacter '.'"),    // (^|/)[Bb]ourbon/.*\.(css|less|scss|styl)$
    (40, "metacharacter '.'"),    // (^|/)select2/.*\.(css|scss|js)$
    (45, "unbounded quantifier"), // (^|/)[Vv]+endor/
    (49, "negated class"),        // (^|/)jquery([^.]*)\.js$
    (50, "character shorthand"),  // (^|/)jquery\-\d\.\d+(\.\d+)?\.js$
    (51, "character shorthand"),  // (^|/)jquery\-ui(\-\d\.\d+(\.\d+)?)?(\.\w+)?\.(js|css)$
    (52, "negated class"),        // (^|/)jquery\.(ui|effects)\.([^.]*)\.(js|css)$
    (56, "character shorthand"),  // (^|/)jquery\.fileupload(-\w+)?\.js$
    (60, "character shorthand"),  // (^|/)slick\.\w+.js$
    (61, "character shorthand"),  // (^|/)Leaflet\.Coordinates-\d+\.\d+\.\d+\.src\.js$
    (71, "metacharacter '.'"),    // (^|/)prototype(.*)\.js$
    (75, "metacharacter '.'"),    // (.*?)\.d\.ts$
    (76, "negated class"),        // (^|/)mootools([^.]*)\d+\.\d+.\d+([^.]*)\.js$
    (79, "negated class"),        // (^|/)yahoo-([^.]*)\.js$
    (80, "negated class"),        // (^|/)yui([^.]*)\.js$
    (82, "negated class"),        // (^|/)tiny_mce([^.]*)\.js$
    (85, "metacharacter '.'"),    // (^|/)fontello(.*?)\.css$
    (88, "character shorthand"), // (^|/)[Cc]ode[Mm]irror/(\d+\.\d+/)?(lib|mode|theme|addon|keymap|demo)
    (89, "negated class"),       // (^|/)shBrush([^.]*)\.js$
    (92, "negated class"),       // (^|/)angular([^.]*)\.js$
    (93, "character shorthand"), // (^|\/)d3(\.v\d+)?([^.]*)\.js$
    (94, "negated class"),       // (^|/)react(-[^.]*)?\.js$
    (95, "metacharacter '.'"),   // (^|/)flow-typed/.*\.js$
    (96, "character shorthand"), // (^|/)modernizr\-\d\.\d+(\.\d+)?\.js$
    (97, "character shorthand"), // (^|/)modernizr\.custom\.\d+\.js$
    (98, "character shorthand"), // (^|/)knockout-(\d+\.){3}(debug\.)?js$
    (125, "negated class"),      // (^|/)jquery([^.]*)\.validate(\.unobtrusive)?\.js$
    (126, "negated class"),      // (^|/)jquery([^.]*)\.unobtrusive\-ajax\.js$
    (128, "metacharacter '.'"),  // (^|/)[Pp]ackages\/.+\.\d+\/
    (129, "metacharacter '.'"),  // (^|/)extjs/.*?\.js$
    (130, "metacharacter '.'"),  // (^|/)extjs/.*?\.xml$
    (131, "metacharacter '.'"),  // (^|/)extjs/.*?\.txt$
    (132, "metacharacter '.'"),  // (^|/)extjs/.*?\.html$
    (133, "metacharacter '.'"),  // (^|/)extjs/.*?\.properties$
    (148, "negated class"),      // (^|/)cordova([^.]*)\.js$
    (149, "character shorthand"), // (^|/)cordova\-\d\.\d(\.\d)?\.js$
    (150, "metacharacter '.'"),  // (^|/)foundation(\..*)?\.js$
];

/// The translated half: literal matchers, paired with the index of the upstream
/// pattern they came from. One pattern expands to several matchers whenever its
/// regex holds a finite alternation or character class.
const VENDOR_MATCHERS: &[(usize, Matcher)] = &[
    // (^|/)cache/
    (0, Matcher::Anchored("cache/")),
    // ^[Dd]ependencies/
    (1, Matcher::Prefix("Dependencies/")),
    (1, Matcher::Prefix("dependencies/")),
    // (^|/)dist/
    (2, Matcher::Anchored("dist/")),
    // ^deps/
    (3, Matcher::Prefix("deps/")),
    // (^|/)configure$
    (4, Matcher::BasenameExact("configure")),
    // (^|/)config\.guess$
    (5, Matcher::BasenameExact("config.guess")),
    // (^|/)config\.sub$
    (6, Matcher::BasenameExact("config.sub")),
    // (^|/)aclocal\.m4
    (7, Matcher::Anchored("aclocal.m4")),
    // (^|/)libtool\.m4
    (8, Matcher::Anchored("libtool.m4")),
    // (^|/)ltoptions\.m4
    (9, Matcher::Anchored("ltoptions.m4")),
    // (^|/)ltsugar\.m4
    (10, Matcher::Anchored("ltsugar.m4")),
    // (^|/)ltversion\.m4
    (11, Matcher::Anchored("ltversion.m4")),
    // (^|/)lt~obsolete\.m4
    (12, Matcher::Anchored("lt~obsolete.m4")),
    // (^|/)dotnet-install\.(ps1|sh)$
    (13, Matcher::BasenameExact("dotnet-install.ps1")),
    (13, Matcher::BasenameExact("dotnet-install.sh")),
    // (^|/)cpplint\.py
    (14, Matcher::Anchored("cpplint.py")),
    // (^|/)node_modules/
    (15, Matcher::Anchored("node_modules/")),
    // (^|/)\.yarn/releases/
    (16, Matcher::Anchored(".yarn/releases/")),
    // (^|/)\.yarn/plugins/
    (17, Matcher::Anchored(".yarn/plugins/")),
    // (^|/)\.yarn/sdks/
    (18, Matcher::Anchored(".yarn/sdks/")),
    // (^|/)\.yarn/versions/
    (19, Matcher::Anchored(".yarn/versions/")),
    // (^|/)\.yarn/unplugged/
    (20, Matcher::Anchored(".yarn/unplugged/")),
    // (^|/)_esy$
    (21, Matcher::BasenameExact("_esy")),
    // (^|/)bower_components/
    (22, Matcher::Anchored("bower_components/")),
    // ^rebar$
    (23, Matcher::PathExact("rebar")),
    // (^|/)erlang\.mk
    (24, Matcher::Anchored("erlang.mk")),
    // (^|/)Godeps/_workspace/
    (25, Matcher::Anchored("Godeps/_workspace/")),
    // (^|/)testdata/
    (26, Matcher::Anchored("testdata/")),
    // (^|/)\.indent\.pro
    (27, Matcher::Anchored(".indent.pro")),
    // (\.|-)min\.(js|css)$
    (28, Matcher::BasenameSuffix(".min.js")),
    (28, Matcher::BasenameSuffix(".min.css")),
    (28, Matcher::BasenameSuffix("-min.js")),
    (28, Matcher::BasenameSuffix("-min.css")),
    // (^|/)font-?awesome\.(css|less|scss|styl)$
    (32, Matcher::BasenameExact("font-awesome.css")),
    (32, Matcher::BasenameExact("font-awesome.less")),
    (32, Matcher::BasenameExact("font-awesome.scss")),
    (32, Matcher::BasenameExact("font-awesome.styl")),
    (32, Matcher::BasenameExact("fontawesome.css")),
    (32, Matcher::BasenameExact("fontawesome.less")),
    (32, Matcher::BasenameExact("fontawesome.scss")),
    (32, Matcher::BasenameExact("fontawesome.styl")),
    // (^|/)foundation\.(css|less|scss|styl)$
    (34, Matcher::BasenameExact("foundation.css")),
    (34, Matcher::BasenameExact("foundation.less")),
    (34, Matcher::BasenameExact("foundation.scss")),
    (34, Matcher::BasenameExact("foundation.styl")),
    // (^|/)normalize\.(css|less|scss|styl)$
    (35, Matcher::BasenameExact("normalize.css")),
    (35, Matcher::BasenameExact("normalize.less")),
    (35, Matcher::BasenameExact("normalize.scss")),
    (35, Matcher::BasenameExact("normalize.styl")),
    // (^|/)skeleton\.(css|less|scss|styl)$
    (36, Matcher::BasenameExact("skeleton.css")),
    (36, Matcher::BasenameExact("skeleton.less")),
    (36, Matcher::BasenameExact("skeleton.scss")),
    (36, Matcher::BasenameExact("skeleton.styl")),
    // (^|/)animate\.(css|less|scss|styl)$
    (38, Matcher::BasenameExact("animate.css")),
    (38, Matcher::BasenameExact("animate.less")),
    (38, Matcher::BasenameExact("animate.scss")),
    (38, Matcher::BasenameExact("animate.styl")),
    // (^|/)materialize\.(css|less|scss|styl|js)$
    (39, Matcher::BasenameExact("materialize.css")),
    (39, Matcher::BasenameExact("materialize.less")),
    (39, Matcher::BasenameExact("materialize.scss")),
    (39, Matcher::BasenameExact("materialize.styl")),
    (39, Matcher::BasenameExact("materialize.js")),
    // (^|/)bulma\.(css|sass|scss)$
    (41, Matcher::BasenameExact("bulma.css")),
    (41, Matcher::BasenameExact("bulma.sass")),
    (41, Matcher::BasenameExact("bulma.scss")),
    // (3rd|[Tt]hird)[-_]?[Pp]arty/
    (42, Matcher::Contains("3rd-Party/")),
    (42, Matcher::Contains("3rd-party/")),
    (42, Matcher::Contains("3rd_Party/")),
    (42, Matcher::Contains("3rd_party/")),
    (42, Matcher::Contains("3rdParty/")),
    (42, Matcher::Contains("3rdparty/")),
    (42, Matcher::Contains("Third-Party/")),
    (42, Matcher::Contains("Third-party/")),
    (42, Matcher::Contains("Third_Party/")),
    (42, Matcher::Contains("Third_party/")),
    (42, Matcher::Contains("ThirdParty/")),
    (42, Matcher::Contains("Thirdparty/")),
    (42, Matcher::Contains("third-Party/")),
    (42, Matcher::Contains("third-party/")),
    (42, Matcher::Contains("third_Party/")),
    (42, Matcher::Contains("third_party/")),
    (42, Matcher::Contains("thirdParty/")),
    (42, Matcher::Contains("thirdparty/")),
    // (^|/)vendors?/
    (43, Matcher::Anchored("vendors/")),
    (43, Matcher::Anchored("vendor/")),
    // (^|/)[Ee]xtern(als?)?/
    (44, Matcher::Anchored("Externals/")),
    (44, Matcher::Anchored("External/")),
    (44, Matcher::Anchored("Extern/")),
    (44, Matcher::Anchored("externals/")),
    (44, Matcher::Anchored("external/")),
    (44, Matcher::Anchored("extern/")),
    // ^debian/
    (46, Matcher::Prefix("debian/")),
    // (^|/)run\.n$
    (47, Matcher::BasenameExact("run.n")),
    // (^|/)bootstrap-datepicker/
    (48, Matcher::Anchored("bootstrap-datepicker/")),
    // (^|/)jquery\.fn\.gantt\.js
    (53, Matcher::Anchored("jquery.fn.gantt.js")),
    // (^|/)jquery\.fancybox\.(js|css)
    (54, Matcher::Anchored("jquery.fancybox.js")),
    (54, Matcher::Anchored("jquery.fancybox.css")),
    // (^|/)fuelux\.js
    (55, Matcher::Anchored("fuelux.js")),
    // (^|/)jquery\.dataTables\.js
    (57, Matcher::Anchored("jquery.dataTables.js")),
    // (^|/)bootbox\.js
    (58, Matcher::Anchored("bootbox.js")),
    // (^|/)pdf\.worker\.js
    (59, Matcher::Anchored("pdf.worker.js")),
    // (^|/)leaflet\.draw-src\.js
    (62, Matcher::Anchored("leaflet.draw-src.js")),
    // (^|/)leaflet\.draw\.css
    (63, Matcher::Anchored("leaflet.draw.css")),
    // (^|/)Control\.FullScreen\.css
    (64, Matcher::Anchored("Control.FullScreen.css")),
    // (^|/)Control\.FullScreen\.js
    (65, Matcher::Anchored("Control.FullScreen.js")),
    // (^|/)leaflet\.spin\.js
    (66, Matcher::Anchored("leaflet.spin.js")),
    // (^|/)wicket-leaflet\.js
    (67, Matcher::Anchored("wicket-leaflet.js")),
    // (^|/)\.sublime-project
    (68, Matcher::Anchored(".sublime-project")),
    // (^|/)\.sublime-workspace
    (69, Matcher::Anchored(".sublime-workspace")),
    // (^|/)\.vscode/
    (70, Matcher::Anchored(".vscode/")),
    // (^|/)effects\.js$
    (72, Matcher::BasenameExact("effects.js")),
    // (^|/)controls\.js$
    (73, Matcher::BasenameExact("controls.js")),
    // (^|/)dragdrop\.js$
    (74, Matcher::BasenameExact("dragdrop.js")),
    // (^|/)dojo\.js$
    (77, Matcher::BasenameExact("dojo.js")),
    // (^|/)MochiKit\.js$
    (78, Matcher::BasenameExact("MochiKit.js")),
    // (^|/)ckeditor\.js$
    (81, Matcher::BasenameExact("ckeditor.js")),
    // (^|/)tiny_mce/(langs|plugins|themes|utils)
    (83, Matcher::Anchored("tiny_mce/langs")),
    (83, Matcher::Anchored("tiny_mce/plugins")),
    (83, Matcher::Anchored("tiny_mce/themes")),
    (83, Matcher::Anchored("tiny_mce/utils")),
    // (^|/)ace-builds/
    (84, Matcher::Anchored("ace-builds/")),
    // (^|/)MathJax/
    (86, Matcher::Anchored("MathJax/")),
    // (^|/)Chart\.js$
    (87, Matcher::BasenameExact("Chart.js")),
    // (^|/)shCore\.js$
    (90, Matcher::BasenameExact("shCore.js")),
    // (^|/)shLegacy\.js$
    (91, Matcher::BasenameExact("shLegacy.js")),
    // (^|/)docs?/_?(build|themes?|templates?|static)/
    (99, Matcher::Anchored("docs/_build/")),
    (99, Matcher::Anchored("docs/_themes/")),
    (99, Matcher::Anchored("docs/_theme/")),
    (99, Matcher::Anchored("docs/_templates/")),
    (99, Matcher::Anchored("docs/_template/")),
    (99, Matcher::Anchored("docs/_static/")),
    (99, Matcher::Anchored("docs/build/")),
    (99, Matcher::Anchored("docs/themes/")),
    (99, Matcher::Anchored("docs/theme/")),
    (99, Matcher::Anchored("docs/templates/")),
    (99, Matcher::Anchored("docs/template/")),
    (99, Matcher::Anchored("docs/static/")),
    (99, Matcher::Anchored("doc/_build/")),
    (99, Matcher::Anchored("doc/_themes/")),
    (99, Matcher::Anchored("doc/_theme/")),
    (99, Matcher::Anchored("doc/_templates/")),
    (99, Matcher::Anchored("doc/_template/")),
    (99, Matcher::Anchored("doc/_static/")),
    (99, Matcher::Anchored("doc/build/")),
    (99, Matcher::Anchored("doc/themes/")),
    (99, Matcher::Anchored("doc/theme/")),
    (99, Matcher::Anchored("doc/templates/")),
    (99, Matcher::Anchored("doc/template/")),
    (99, Matcher::Anchored("doc/static/")),
    // (^|/)admin_media/
    (100, Matcher::Anchored("admin_media/")),
    // (^|/)env/
    (101, Matcher::Anchored("env/")),
    // (^|/)fabfile\.py$
    (102, Matcher::BasenameExact("fabfile.py")),
    // (^|/)waf$
    (103, Matcher::BasenameExact("waf")),
    // (^|/)\.osx$
    (104, Matcher::BasenameExact(".osx")),
    // \.xctemplate/
    (105, Matcher::Contains(".xctemplate/")),
    // \.imageset/
    (106, Matcher::Contains(".imageset/")),
    // (^|/)Carthage/
    (107, Matcher::Anchored("Carthage/")),
    // (^|/)Sparkle/
    (108, Matcher::Anchored("Sparkle/")),
    // (^|/)Crashlytics\.framework/
    (109, Matcher::Anchored("Crashlytics.framework/")),
    // (^|/)Fabric\.framework/
    (110, Matcher::Anchored("Fabric.framework/")),
    // (^|/)BuddyBuildSDK\.framework/
    (111, Matcher::Anchored("BuddyBuildSDK.framework/")),
    // (^|/)Realm\.framework
    (112, Matcher::Anchored("Realm.framework")),
    // (^|/)RealmSwift\.framework
    (113, Matcher::Anchored("RealmSwift.framework")),
    // (^|/)\.gitattributes$
    (114, Matcher::BasenameExact(".gitattributes")),
    // (^|/)\.gitignore$
    (115, Matcher::BasenameExact(".gitignore")),
    // (^|/)\.gitmodules$
    (116, Matcher::BasenameExact(".gitmodules")),
    // (^|/)gradlew$
    (117, Matcher::BasenameExact("gradlew")),
    // (^|/)gradlew\.bat$
    (118, Matcher::BasenameExact("gradlew.bat")),
    // (^|/)gradle/wrapper/
    (119, Matcher::Anchored("gradle/wrapper/")),
    // (^|/)mvnw$
    (120, Matcher::BasenameExact("mvnw")),
    // (^|/)mvnw\.cmd$
    (121, Matcher::BasenameExact("mvnw.cmd")),
    // (^|/)\.mvn/wrapper/
    (122, Matcher::Anchored(".mvn/wrapper/")),
    // -vsdoc\.js$
    (123, Matcher::BasenameSuffix("-vsdoc.js")),
    // \.intellisense\.js$
    (124, Matcher::BasenameSuffix(".intellisense.js")),
    // (^|/)[Mm]icrosoft([Mm]vc)?([Aa]jax|[Vv]alidation)(\.debug)?\.js$
    (127, Matcher::BasenameExact("MicrosoftMvcAjax.debug.js")),
    (127, Matcher::BasenameExact("MicrosoftMvcAjax.js")),
    (127, Matcher::BasenameExact("MicrosoftMvcajax.debug.js")),
    (127, Matcher::BasenameExact("MicrosoftMvcajax.js")),
    (
        127,
        Matcher::BasenameExact("MicrosoftMvcValidation.debug.js"),
    ),
    (127, Matcher::BasenameExact("MicrosoftMvcValidation.js")),
    (
        127,
        Matcher::BasenameExact("MicrosoftMvcvalidation.debug.js"),
    ),
    (127, Matcher::BasenameExact("MicrosoftMvcvalidation.js")),
    (127, Matcher::BasenameExact("MicrosoftmvcAjax.debug.js")),
    (127, Matcher::BasenameExact("MicrosoftmvcAjax.js")),
    (127, Matcher::BasenameExact("Microsoftmvcajax.debug.js")),
    (127, Matcher::BasenameExact("Microsoftmvcajax.js")),
    (
        127,
        Matcher::BasenameExact("MicrosoftmvcValidation.debug.js"),
    ),
    (127, Matcher::BasenameExact("MicrosoftmvcValidation.js")),
    (
        127,
        Matcher::BasenameExact("Microsoftmvcvalidation.debug.js"),
    ),
    (127, Matcher::BasenameExact("Microsoftmvcvalidation.js")),
    (127, Matcher::BasenameExact("MicrosoftAjax.debug.js")),
    (127, Matcher::BasenameExact("MicrosoftAjax.js")),
    (127, Matcher::BasenameExact("Microsoftajax.debug.js")),
    (127, Matcher::BasenameExact("Microsoftajax.js")),
    (127, Matcher::BasenameExact("MicrosoftValidation.debug.js")),
    (127, Matcher::BasenameExact("MicrosoftValidation.js")),
    (127, Matcher::BasenameExact("Microsoftvalidation.debug.js")),
    (127, Matcher::BasenameExact("Microsoftvalidation.js")),
    (127, Matcher::BasenameExact("microsoftMvcAjax.debug.js")),
    (127, Matcher::BasenameExact("microsoftMvcAjax.js")),
    (127, Matcher::BasenameExact("microsoftMvcajax.debug.js")),
    (127, Matcher::BasenameExact("microsoftMvcajax.js")),
    (
        127,
        Matcher::BasenameExact("microsoftMvcValidation.debug.js"),
    ),
    (127, Matcher::BasenameExact("microsoftMvcValidation.js")),
    (
        127,
        Matcher::BasenameExact("microsoftMvcvalidation.debug.js"),
    ),
    (127, Matcher::BasenameExact("microsoftMvcvalidation.js")),
    (127, Matcher::BasenameExact("microsoftmvcAjax.debug.js")),
    (127, Matcher::BasenameExact("microsoftmvcAjax.js")),
    (127, Matcher::BasenameExact("microsoftmvcajax.debug.js")),
    (127, Matcher::BasenameExact("microsoftmvcajax.js")),
    (
        127,
        Matcher::BasenameExact("microsoftmvcValidation.debug.js"),
    ),
    (127, Matcher::BasenameExact("microsoftmvcValidation.js")),
    (
        127,
        Matcher::BasenameExact("microsoftmvcvalidation.debug.js"),
    ),
    (127, Matcher::BasenameExact("microsoftmvcvalidation.js")),
    (127, Matcher::BasenameExact("microsoftAjax.debug.js")),
    (127, Matcher::BasenameExact("microsoftAjax.js")),
    (127, Matcher::BasenameExact("microsoftajax.debug.js")),
    (127, Matcher::BasenameExact("microsoftajax.js")),
    (127, Matcher::BasenameExact("microsoftValidation.debug.js")),
    (127, Matcher::BasenameExact("microsoftValidation.js")),
    (127, Matcher::BasenameExact("microsoftvalidation.debug.js")),
    (127, Matcher::BasenameExact("microsoftvalidation.js")),
    // (^|/)extjs/\.sencha/
    (134, Matcher::Anchored("extjs/.sencha/")),
    // (^|/)extjs/docs/
    (135, Matcher::Anchored("extjs/docs/")),
    // (^|/)extjs/builds/
    (136, Matcher::Anchored("extjs/builds/")),
    // (^|/)extjs/cmd/
    (137, Matcher::Anchored("extjs/cmd/")),
    // (^|/)extjs/examples/
    (138, Matcher::Anchored("extjs/examples/")),
    // (^|/)extjs/locale/
    (139, Matcher::Anchored("extjs/locale/")),
    // (^|/)extjs/packages/
    (140, Matcher::Anchored("extjs/packages/")),
    // (^|/)extjs/plugins/
    (141, Matcher::Anchored("extjs/plugins/")),
    // (^|/)extjs/resources/
    (142, Matcher::Anchored("extjs/resources/")),
    // (^|/)extjs/src/
    (143, Matcher::Anchored("extjs/src/")),
    // (^|/)extjs/welcome/
    (144, Matcher::Anchored("extjs/welcome/")),
    // (^|/)html5shiv\.js$
    (145, Matcher::BasenameExact("html5shiv.js")),
    // (^|/)[Tt]ests?/fixtures/
    (146, Matcher::Anchored("Tests/fixtures/")),
    (146, Matcher::Anchored("Test/fixtures/")),
    (146, Matcher::Anchored("tests/fixtures/")),
    (146, Matcher::Anchored("test/fixtures/")),
    // (^|/)[Ss]pecs?/fixtures/
    (147, Matcher::Anchored("Specs/fixtures/")),
    (147, Matcher::Anchored("Spec/fixtures/")),
    (147, Matcher::Anchored("specs/fixtures/")),
    (147, Matcher::Anchored("spec/fixtures/")),
    // (^|/)Vagrantfile$
    (151, Matcher::BasenameExact("Vagrantfile")),
    // (^|/)\.[Dd][Ss]_[Ss]tore$
    (152, Matcher::BasenameExact(".DS_Store")),
    (152, Matcher::BasenameExact(".DS_store")),
    (152, Matcher::BasenameExact(".Ds_Store")),
    (152, Matcher::BasenameExact(".Ds_store")),
    (152, Matcher::BasenameExact(".dS_Store")),
    (152, Matcher::BasenameExact(".dS_store")),
    (152, Matcher::BasenameExact(".ds_Store")),
    (152, Matcher::BasenameExact(".ds_store")),
    // (^|/)inst/extdata/
    (153, Matcher::Anchored("inst/extdata/")),
    // (^|/)octicons\.css
    (154, Matcher::Anchored("octicons.css")),
    // (^|/)sprockets-octicons\.scss
    (155, Matcher::Anchored("sprockets-octicons.scss")),
    // (^|/)activator$
    (156, Matcher::BasenameExact("activator")),
    // (^|/)activator\.bat$
    (157, Matcher::BasenameExact("activator.bat")),
    // (^|/)proguard\.pro$
    (158, Matcher::BasenameExact("proguard.pro")),
    // (^|/)proguard-rules\.pro$
    (159, Matcher::BasenameExact("proguard-rules.pro")),
    // (^|/)puphpet/
    (160, Matcher::Anchored("puphpet/")),
    // (^|/)\.google_apis/
    (161, Matcher::Anchored(".google_apis/")),
    // (^|/)Jenkinsfile$
    (162, Matcher::BasenameExact("Jenkinsfile")),
    // (^|/)\.gitpod\.Dockerfile$
    (163, Matcher::BasenameExact(".gitpod.Dockerfile")),
    // (^|/)\.github/
    (164, Matcher::Anchored(".github/")),
    // (^|/)\.obsidian/
    (165, Matcher::Anchored(".obsidian/")),
    // (^|/)\.teamcity/
    (166, Matcher::Anchored(".teamcity/")),
    // (^|/)xvba_modules/
    (167, Matcher::Anchored("xvba_modules/")),
];
