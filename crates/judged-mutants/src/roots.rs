//! The materialized root set, and the root set as a rescue source (§5, §9.13).
//!
//! [`judged_core::roots`] holds the three tiers as three modules with three
//! vocabularies: a manifest's `Root`, a convention's `ConventionRoot`, a
//! declaration's `Seed`. This is the assembler that turns them into **one
//! homogeneous list a human can read top to bottom**, which is what
//! `roots::manifest::Tier`'s own documentation says the three-variant enum
//! exists for, and what ProGuard's `-printseeds` — asked for by name in §9.13 —
//! actually prints.
//!
//! # What this does not do
//!
//! It does not decide what is reachable. §1.2: you cannot infer the closed
//! world, you can only have it declared. So this materializes what was
//! **declared**, records where each root came from, and shows it to a human
//! before anything acts on it — the reason Nix ships `--print-roots` and
//! ProGuard ships `-printseeds`.
//!
//! # Provenance is the load-bearing field
//!
//! Every root carries its §5.1 tier. A convention-derived root is a guess about
//! a framework and is labelled as one ([`Tier::B`]); a root that does not say
//! which tier it came from is worse than no root, because it invites a caller to
//! trust a guessed convention as though a manifest had declared it.
//!
//! # Why it lives in this crate
//!
//! Because this is where it is *measured*. [`RootedSut`] makes the root set a
//! rescue source the E2 suite can grade, alongside Gate 2, and §11 R1 asks
//! whether any signal **combination** clears the catalogue. Its natural home is
//! `judged_core::roots` once that module has an assembler; nothing here depends
//! on the E2 catalogue, so moving it is a re-export away.

use std::cell::RefCell;
use std::fmt;
use std::path::{Path, PathBuf};

use judged_core::roots::{convention, declared, insource, manifest};
use judged_core::Result;

use crate::sut::{ClaimKind, Sut, SutVerdict, SymbolClaim};
use crate::Ecosystem;

// ---------------------------------------------------------------------------
// Provenance
// ---------------------------------------------------------------------------

/// Which of §5.1's three tiers a root came from.
///
/// The three do not deserve equal trust, and the difference is not cosmetic:
/// Tier A is a fact read out of a file a build system already reads, Tier B is
/// correct only if a framework *and its version* were detected correctly, and
/// Tier C is exactly as good as the person who wrote it down.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Tier {
    /// Machine-declared: a build system or deploy target already reads this file
    /// to find roots.
    A,
    /// Convention-inferable: a framework's layout or annotations make a file an
    /// entry point with no source reference anywhere.
    B,
    /// Undiscoverable: solicited from a human and recorded in
    /// [`declared::ROOTS_FILE`].
    C,
}

impl Tier {
    /// `"A"`, `"B"` or `"C"` — what a `-printseeds` line leads with.
    pub fn label(self) -> &'static str {
        match self {
            Tier::A => "A",
            Tier::B => "B",
            Tier::C => "C",
        }
    }

    /// The one sentence a reader needs before trusting a root of this tier.
    pub fn caveat(self) -> &'static str {
        match self {
            Tier::A => {
                "machine-declared: a build system or deploy target already reads this \
                        file to find roots"
            }
            Tier::B => {
                "convention-inferable: correct only if the framework AND its version \
                        were detected correctly — this is a guess about a framework"
            }
            Tier::C => {
                "undiscoverable: solicited from a human and committed; confidence in \
                        the derivation is none"
            }
        }
    }
}

impl fmt::Display for Tier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// One materialized root, whichever tier produced it.
///
/// Fields are private and there is no public constructor: a root cannot be
/// fabricated without one of the three tiers having produced it, and it can
/// never be spelled without the tier that says where its confidence comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Root {
    tier: Tier,
    rule: String,
    origin: String,
    origin_file: PathBuf,
    target: String,
    path: Option<String>,
    symbol: Option<String>,
    detail: String,
}

impl Root {
    /// Which §5.1 tier this root's confidence comes from.
    pub fn tier(&self) -> Tier {
        self.tier
    }

    /// What kind of root it is, or which convention fired: `executable`,
    /// `django/appconfig`, `declared`. Per rule rather than per framework, so a
    /// report can say *why* a file is a root and §11 R2's per-rule fire rate is
    /// measurable rather than guessed.
    pub fn rule(&self) -> &str {
        &self.rule
    }

    /// The exact file and key it came from, rendered `<file>#<key>`. §9.13 wants
    /// show-roots output a human can check against the repository, and a key
    /// that only names the file is not checkable.
    pub fn origin(&self) -> &str {
        &self.origin
    }

    /// Just the file half of [`Root::origin`], repo-relative, so a caller can
    /// open it without parsing the rendering.
    pub fn origin_file(&self) -> &Path {
        &self.origin_file
    }

    /// What the declaration points at, as it was written.
    pub fn target(&self) -> &str {
        &self.target
    }

    /// The repo-relative path this root names, when it names one. `None` for a
    /// glob, a command line or a name something other than a path lookup
    /// resolves — recording those as paths is how a cleaner "resolves"
    /// `npm run build` to a file called `npm`.
    pub fn path(&self) -> Option<&str> {
        self.path.as_deref()
    }

    /// The symbol a framework loads by name, where the convention names one —
    /// the `AppConfig` subclass, for instance. Frequently the only place that
    /// name exists outside its own declaration, which is exactly why a literal
    /// veto cannot rescue it.
    pub fn symbol(&self) -> Option<&str> {
        self.symbol.as_deref()
    }

    /// The whole provenance in a sentence somebody can act on.
    pub fn detail(&self) -> &str {
        &self.detail
    }
}

// ---------------------------------------------------------------------------
// What could not be resolved
// ---------------------------------------------------------------------------

/// A gap this materializer found and is reporting rather than swallowing.
///
/// §6.20's rule applied to root discovery: *"no data" must be a distinct state
/// from "zero"*. A root list that shows only successes hides exactly the gaps a
/// reader needs — a framework whose roots are all missing looks identical to a
/// framework that has none.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gap {
    /// Which kind of gap, so a consumer can match on it rather than on prose.
    pub kind: GapKind,
    /// What the gap is about: a framework name, a manifest path, a pathspec.
    pub subject: String,
    /// What is missing, and what to do about it.
    pub detail: String,
}

/// The kinds of gap. Each one is a place where a root that exists in the world
/// does not exist in this list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum GapKind {
    /// A manifest could not be read or could not be parsed. **Tier A is empty
    /// as a result**: `manifest::scan` fails the whole scan rather than
    /// returning the other packages' roots and quietly dropping the broken
    /// package's, because a root set missing exactly the entry points of the one
    /// package nobody could read is the most dangerous possible answer.
    ManifestUnreadable,
    /// The convention scan could not walk the tree. Tier B is empty as a result.
    ConventionScanFailed,
    /// A framework was detected and there is no plugin for it. Its convention
    /// roots — however many there are — are all missing (§9.5 caps the tier on
    /// this signal).
    FrameworkWithoutPlugin,
    /// A list of roots was found but is computed at run time and no data file
    /// holding it could be resolved.
    UnresolvedRootList,
    /// `.judged/roots.toml` did not parse. Tier C is empty as a result, and a
    /// dropped Tier C root is a deletion nobody vetoed.
    DeclaredRootsMalformed,
    /// A declared entry decided nothing: its referent is gone, its deadline
    /// passed, or it matched no candidate in this run. Periphery's
    /// superfluous-ignore warning, generalized — a pattern protecting nothing is
    /// a blind spot nobody is watching.
    DeclaredRootRot,
}

impl GapKind {
    /// Stable lower-case label, for reports and for a consumer matching on it.
    pub fn as_str(self) -> &'static str {
        match self {
            GapKind::ManifestUnreadable => "manifest-unreadable",
            GapKind::ConventionScanFailed => "convention-scan-failed",
            GapKind::FrameworkWithoutPlugin => "framework-without-plugin",
            GapKind::UnresolvedRootList => "unresolved-root-list",
            GapKind::DeclaredRootsMalformed => "declared-roots-malformed",
            GapKind::DeclaredRootRot => "declared-root-rot",
        }
    }
}

/// A framework the convention scan recognized, and whether its conventions are
/// actually implemented.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Detected {
    /// Lower-case framework name, as the scanner spells it.
    pub framework: String,
    /// The version **requirement** as declared — not a resolved version, which
    /// only a lockfile knows. `None` whenever the proof carried no version, and
    /// §5.1 makes version part of Tier B's correctness condition, so that `None`
    /// is a real caveat rather than a missing field.
    pub version: Option<String>,
    /// Whether a plugin implements this framework's conventions. `false` means
    /// its roots are missing and a [`GapKind::FrameworkWithoutPlugin`] was
    /// emitted beside it.
    pub covered: bool,
    /// The file the detection was read from, and what in it.
    pub evidence: String,
}

// ---------------------------------------------------------------------------
// The set
// ---------------------------------------------------------------------------

/// Everything the three tiers declared, plus everything they could not resolve.
#[derive(Debug, Clone, Default)]
pub struct RootSet {
    roots: Vec<Root>,
    gaps: Vec<Gap>,
    declarations: Vec<String>,
    detections: Vec<Detected>,
    manifests_read: Vec<PathBuf>,
    files_scanned: usize,
    /// Kept so [`RootSet::lint_declared`] can be asked later, by a caller that
    /// has a clock and an expiry rule in hand. Rot detection is a reporting
    /// concern: [`declared::DeclaredRoots::materialize`] never consults
    /// `expires`, so a rotted entry still protects and cannot silently change a
    /// rescue.
    declared: declared::DeclaredRoots,
    candidates: Vec<String>,
}

impl RootSet {
    /// Every root, Tier A first, then B, then C; within a tier, by path.
    pub fn roots(&self) -> &[Root] {
        &self.roots
    }

    /// The roots of one tier, in the same order.
    pub fn tier(&self, tier: Tier) -> impl Iterator<Item = &Root> {
        self.roots.iter().filter(move |root| root.tier == tier)
    }

    /// Everything that could not be resolved. Empty means "we resolved
    /// everything we recognized", never "we recognized everything".
    pub fn gaps(&self) -> &[Gap] {
        &self.gaps
    }

    /// §5.2's non-root statements — `sideEffects`, a `cdylib` `crate-type` —
    /// rendered. They name no root, and they change what a downstream tier may
    /// conclude: one widens what may be deleted, the other says the evidence for
    /// deleting is missing.
    pub fn declarations(&self) -> &[String] {
        &self.declarations
    }

    /// Every framework recognized, covered or not.
    pub fn detections(&self) -> &[Detected] {
        &self.detections
    }

    /// Every manifest that was successfully read.
    pub fn manifests_read(&self) -> &[PathBuf] {
        &self.manifests_read
    }

    /// How many files the convention walk visited. A root set reported over a
    /// corpus of zero files has not looked (§6.20), and a caller that wants to
    /// tell a thin repository from an unread one asks this.
    pub fn files_scanned(&self) -> usize {
        self.files_scanned
    }

    /// The root that declares `path`, if one does.
    ///
    /// Exact, normalized path equality and nothing looser. Containment — "this
    /// file is *under* a root directory" — is deliberately not implemented: a
    /// package being an entry point does not make every file in it one, and a
    /// containment rule over a directory root would rescue an entire tree,
    /// which is a constant function wearing a rule's name.
    pub fn rescues_path(&self, path: &str) -> Option<&Root> {
        let wanted = normalize_str(path);
        self.roots
            .iter()
            .find(|root| root.path.as_deref() == Some(wanted.as_str()))
    }

    /// The root that declares the symbol `name`, if one does.
    ///
    /// A symbol is rescued when a root **names it** — the `AppConfig` subclass
    /// Django instantiates, and nothing else. Being *declared in* a file that is
    /// a root is not enough, and that restraint is the whole difference between
    /// a rule and an over-firing one: `reporting/apps.py` is an entry point, but
    /// a helper function that happens to live in it is not, and a layer that
    /// rescued everything in a root file would rescue decoys by the same rule.
    ///
    /// Matching is by trailing segment as well as by equality, because ground
    /// truth spells a symbol bare and a tool spells it however its ecosystem
    /// does (`ledger.dunning.DunningConfig`). The same rule
    /// [`crate::runner`] grades with, for the same reason.
    pub fn rescues_symbol(&self, name: &str) -> Option<&Root> {
        self.roots.iter().find(|root| {
            root.symbol
                .as_deref()
                .is_some_and(|declared| names_same_symbol(name, declared))
        })
    }

    /// Lint the declared roots against reality — §5.3's rot detection, which is
    /// the thing it says nothing in the survey has.
    ///
    /// Split out rather than done in [`materialize`] because it needs a clock
    /// and an expiry rule, and because it is a *reporting* concern only: nothing
    /// here can change which roots were materialized. `has_expired` is supplied
    /// rather than reimplemented — pass `judged_ratchet::rot::has_expired`, so
    /// that one definition of "expired" governs both files.
    pub fn lint_declared(
        &self,
        repo_root: &Path,
        now: &str,
        has_expired: &dyn Fn(&str, &str) -> bool,
    ) -> Vec<Gap> {
        self.declared
            .lint(&self.candidates, repo_root, now, has_expired)
            .into_iter()
            .map(|rot| Gap {
                kind: GapKind::DeclaredRootRot,
                subject: format!("{}:{}", declared::ROOTS_FILE, rot.line()),
                detail: rot.to_string(),
            })
            .collect()
    }

    /// ProGuard `-printseeds`, which §9.13 asks for by name: every root, grouped
    /// by the tier that earned it, then everything that could not be resolved.
    ///
    /// Tab-separated within a line and sorted within a tier, so it diffs cleanly
    /// between runs. The gaps are not an appendix: a root list that shows only
    /// successes hides exactly what a reader needs in order to know how much of
    /// the entry surface is missing.
    pub fn printseeds(&self) -> String {
        let mut out = String::new();
        out.push_str("# judged show-roots — the materialized root set (§5.1, §9.13)\n");
        out.push_str("# tier\trule\torigin\ttarget\n");

        for tier in [Tier::A, Tier::B, Tier::C] {
            let roots: Vec<&Root> = self.tier(tier).collect();
            out.push_str(&format!(
                "\n# tier {} — {} ({} root{})\n",
                tier.label(),
                tier.caveat(),
                roots.len(),
                if roots.len() == 1 { "" } else { "s" }
            ));
            if roots.is_empty() {
                // §6.20 again, one line down: an empty tier is either an empty
                // tier or a tier that failed, and the gaps below say which.
                out.push_str("# (none)\n");
                continue;
            }
            for root in roots {
                out.push_str(&format!(
                    "{}\t{}\t{}\t{}\n",
                    root.tier.label(),
                    root.rule,
                    escape(&root.origin),
                    escape(&root.target),
                ));
            }
        }

        out.push_str(&format!(
            "\n# could not resolve ({} gap{})\n",
            self.gaps.len(),
            if self.gaps.len() == 1 { "" } else { "s" }
        ));
        if self.gaps.is_empty() {
            out.push_str(
                "# (none) — every framework recognized was covered, every manifest parsed, and \
                 every declared entry decided something. That is not the same as having \
                 recognized everything.\n",
            );
        }
        for gap in &self.gaps {
            out.push_str(&format!(
                "?\t{}\t{}\t{}\n",
                gap.kind.as_str(),
                escape(&gap.subject),
                escape(&gap.detail)
            ));
        }
        out
    }
}

/// Write `text` with the characters that would break a tab-separated line
/// spelled out, so every root stays on exactly one row. A CI `run:` body is a
/// shell script, and letting its newlines through would mean a reader could no
/// longer tell a second root from the second line of the first one.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(c),
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Materializing
// ---------------------------------------------------------------------------

/// Materialize every root the three tiers of §5.1 declare for `repo_root`.
///
/// `candidates` are the repo-relative paths this run is considering, which is
/// what the Tier C pathspecs are matched against: a declaration is a statement
/// about paths, and turning it into a root requires knowing which paths are on
/// the table. Pass every tracked file to see the whole Tier C surface, or a
/// claim set to see only what it protects in this run.
///
/// # Failure is a gap, never an exception
///
/// A tier that fails contributes zero roots **and** one loud [`Gap`]. That is
/// the opposite of [`crate::sut::VetoedSut`], which refuses to run at all when
/// Gate 2 cannot open the repository, and the difference is the direction each
/// failure points. A missing veto makes an ungated run report itself as gated —
/// a claim survives that should have been rescued, and the report says it was
/// checked. A missing tier here makes the rescue layer *weaker*: fewer rescues,
/// more surviving claims, a higher false-removal count, a redder gate. That
/// cannot manufacture a green, and the gap is printed either way.
pub fn materialize<S: AsRef<str>>(repo_root: &Path, candidates: &[S]) -> RootSet {
    let mut set = RootSet {
        candidates: candidates
            .iter()
            .map(|c| normalize_str(c.as_ref()))
            .collect(),
        ..RootSet::default()
    };

    collect_manifests(repo_root, &mut set);
    collect_insource(repo_root, &mut set);
    collect_conventions(repo_root, &mut set);
    collect_declared(repo_root, &mut set);

    // Tier order first, then path, so the dump reads top to bottom in order of
    // decreasing confidence and diffs cleanly between runs.
    set.roots.sort_by(|a, b| {
        a.tier
            .cmp(&b.tier)
            .then_with(|| a.path.cmp(&b.path))
            .then_with(|| a.rule.cmp(&b.rule))
            .then_with(|| a.origin.cmp(&b.origin))
            .then_with(|| a.symbol.cmp(&b.symbol))
    });
    set.roots.dedup();
    set
}

/// Tier A: everything a manifest declares.
/// Tier A roots declared by a marker in source rather than by a manifest
/// (§5.2): Go's `//go:linkname` and `//export`, Rust's `#[no_mangle]`,
/// `#[used]`, `#[export_name]` and `#[ctor]`, Python's `.pth` files and
/// `sitecustomize.py`.
///
/// Tier A rather than B because none of these is a guess about a framework.
/// `#[no_mangle]` does not suggest the symbol might be exported; it instructs
/// the linker to emit it under that name. See `judged_core::roots::insource`.
///
/// A scan that cannot list a directory is a gap rather than a silent zero, for
/// the reason every other collector here records one: a root source that found
/// nothing and a root source that could not look are the §6.20 pair.
fn collect_insource(repo_root: &Path, set: &mut RootSet) {
    let found = match insource::scan(repo_root) {
        Ok(found) => found,
        Err(error) => {
            set.gaps.push(Gap {
                kind: GapKind::ManifestUnreadable,
                subject: repo_root.display().to_string(),
                detail: format!(
                    "{error}. The in-source root scan (§5.2) did not complete, so every \
                     `//export`, `#[no_mangle]`, `.pth` and `sitecustomize.py` root is \
                     missing from this list."
                ),
            });
            return;
        }
    };

    for root in found {
        set.roots.push(Root {
            tier: Tier::A,
            rule: root.marker().as_str().to_string(),
            origin: root.origin(),
            origin_file: root.file().to_path_buf(),
            // A `.pth` and a `sitecustomize.py` ARE the entry point, so the file
            // is the root. Every other marker declares a symbol and says nothing
            // about whether the file around it is reachable — recording the file
            // as a root there would rescue a whole module on evidence about one
            // function.
            path: match root.symbol() {
                None => Some(normalize_path(root.file())),
                Some(_) => None,
            },
            symbol: root.symbol().map(str::to_string),
            target: root.target().to_string(),
            detail: format!(
                "{} declares it at line {}, read by {}",
                root.file().display(),
                root.line(),
                root.marker().reader()
            ),
        });
    }
}

fn collect_manifests(repo_root: &Path, set: &mut RootSet) {
    let scanned = match manifest::scan(repo_root) {
        Ok(scanned) => scanned,
        Err(error) => {
            set.gaps.push(Gap {
                kind: GapKind::ManifestUnreadable,
                subject: error.path().display().to_string(),
                detail: format!(
                    "{error}. One unreadable manifest fails the whole Tier A scan, so EVERY \
                     machine-declared root is missing from this list — not just this package's."
                ),
            });
            return;
        }
    };

    for root in scanned.roots() {
        let target = root.target().to_string();
        set.roots.push(Root {
            tier: match root.tier() {
                manifest::Tier::A => Tier::A,
                manifest::Tier::B => Tier::B,
                manifest::Tier::C => Tier::C,
            },
            rule: root.kind().as_str().to_string(),
            origin: root.origin().to_string(),
            origin_file: root.origin().file().to_path_buf(),
            path: match root.target() {
                manifest::RootTarget::Path(path) => Some(normalize_path(path)),
                // A glob is not expanded and a command is not a file. Recording
                // either as a path is how a cleaner "resolves" `npm run build`
                // to a file called `npm`.
                _ => None,
            },
            symbol: None,
            detail: format!(
                "{} declares it at {}",
                root.origin().file().display(),
                root.origin().key()
            ),
            target,
        });
    }

    for declaration in scanned.declarations() {
        set.declarations.push(match declaration {
            manifest::Declaration::TreeShakable { origin } => format!(
                "{origin}: the package declares a bundler may drop any module nothing imports \
                 — the inverse of a root"
            ),
            manifest::Declaration::TreeShakableExcept { origin, globs } => format!(
                "{origin}: only {globs:?} have side effects; everything else is declared droppable"
            ),
            manifest::Declaration::ConsumerOutsideBuildGraph { origin, crate_type } => format!(
                "{origin}: `{crate_type}` — the consumer is outside the crate graph entirely, so \
                 \"nothing in this workspace calls it\" is not evidence about this target"
            ),
        });
    }

    set.manifests_read = scanned.sources().to_vec();
}

/// Tier B: everything a framework's convention makes an entry point.
fn collect_conventions(repo_root: &Path, set: &mut RootSet) {
    let scanned = match convention::scan(repo_root) {
        Ok(scanned) => scanned,
        Err(error) => {
            set.gaps.push(Gap {
                kind: GapKind::ConventionScanFailed,
                subject: repo_root.display().to_string(),
                detail: format!(
                    "{error}. Every convention root is missing from this list. \"This repository \
                     has no entry points\" is the most dangerous sentence a scan can utter, and it \
                     must never be how an unreadable directory presents (§6.20)."
                ),
            });
            return;
        }
    };

    set.files_scanned = scanned.files_scanned();

    for detection in scanned.detections() {
        set.detections.push(Detected {
            framework: detection.framework().name().to_string(),
            version: detection.declared_version().map(str::to_string),
            covered: detection.framework().has_plugin(),
            evidence: format!(
                "{} {}: {}",
                detection.evidence().kind().label(),
                detection.evidence().path().display(),
                detection.evidence().detail()
            ),
        });
    }

    for root in scanned.roots() {
        // The file that named *this* root when a rule read a list — a settings
        // module, or the config file its list came from — and otherwise the
        // proof that the framework is present at all. Either way it is a file a
        // reader can open and argue with, which is what §9.13 asks for.
        let evidence = root.source().unwrap_or_else(|| root.detection().evidence());
        let version = root
            .detection()
            .declared_version()
            .unwrap_or("version-unknown");
        set.roots.push(Root {
            tier: Tier::B,
            rule: root.rule().label().to_string(),
            origin: format!("{}#{}", evidence.path().display(), evidence.detail()),
            origin_file: evidence.path().to_path_buf(),
            target: match root.symbol() {
                Some(symbol) => format!("{}::{symbol}", root.path().display()),
                None => root.path().display().to_string(),
            },
            path: Some(normalize_path(root.path())),
            symbol: root.symbol().map(str::to_string),
            detail: format!(
                "{} {version} loads it by convention ({}), proved by {} {}",
                root.framework().name(),
                root.rule().label(),
                root.detection().evidence().kind().label(),
                root.detection().evidence().path().display(),
            ),
        });
    }

    for unknown in scanned.known_unknowns() {
        set.gaps.push(Gap {
            kind: match unknown.reason() {
                convention::UnknownReason::NoPlugin => GapKind::FrameworkWithoutPlugin,
                convention::UnknownReason::UnresolvedRootList { .. } => GapKind::UnresolvedRootList,
            },
            subject: unknown.framework().name().to_string(),
            detail: format!(
                "{} — tier capped (§9.5); evidence: {} {}",
                unknown.reason().message(),
                unknown.evidence().kind().label(),
                unknown.evidence().path().display(),
            ),
        });
    }
}

/// Tier C: everything a human wrote down in [`declared::ROOTS_FILE`].
fn collect_declared(repo_root: &Path, set: &mut RootSet) {
    let path = repo_root.join(declared::ROOTS_FILE);
    // An absent file is a repository with no Tier C roots, which is a normal
    // repository — `parse("")`, not an error and not a gap.
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
        Err(error) => {
            set.gaps.push(Gap {
                kind: GapKind::DeclaredRootsMalformed,
                subject: declared::ROOTS_FILE.to_string(),
                detail: format!(
                    "{} could not be read: {error}. Every Tier C root is missing, and a dropped \
                     Tier C root is a deletion nobody vetoed.",
                    path.display()
                ),
            });
            return;
        }
    };

    let parsed = match declared::DeclaredRoots::parse(&text) {
        Ok(parsed) => parsed,
        Err(malformed) => {
            set.gaps.push(Gap {
                kind: GapKind::DeclaredRootsMalformed,
                subject: format!("{}:{}", declared::ROOTS_FILE, malformed.line),
                detail: format!(
                    "{malformed}. Every Tier C root is missing: a parse failure is fatal rather \
                     than skipped, because a dropped entry is a declared root that silently stops \
                     protecting anything."
                ),
            });
            return;
        }
    };

    for seed in parsed.materialize(&set.candidates) {
        set.roots.push(Root {
            tier: Tier::C,
            rule: "declared".to_string(),
            origin: format!("{}:{}", declared::ROOTS_FILE, seed.declared_at_line),
            origin_file: PathBuf::from(declared::ROOTS_FILE),
            target: seed.pathspec.clone(),
            path: Some(normalize_str(&seed.path)),
            symbol: None,
            detail: format!("declared {} ({}): {}", seed.status, seed.kind, seed.reason),
        });
    }
    set.declared = parsed;
}

/// Separators an analyzer may use to qualify a symbol name.
///
/// The same set [`crate::runner`] and [`crate::sut`] use, and for the same
/// reason: a convention names a symbol bare, and a tool spells it however its
/// ecosystem does.
const SYMBOL_SEPARATORS: [&str; 4] = ["::", ".", "/", "#"];

/// Whether `claimed` and `declared` name the same symbol.
///
/// Trailing-segment matching can only ever find MORE rescues than equality,
/// never fewer, which is the direction a layer that may only rescue is allowed
/// to be wrong in.
fn names_same_symbol(claimed: &str, declared: &str) -> bool {
    claimed == declared
        || SYMBOL_SEPARATORS
            .iter()
            .any(|sep| claimed.ends_with(&format!("{sep}{declared}")))
}

/// A repo-relative path as the forward-slashed string both sides are keyed on.
fn normalize_path(path: &Path) -> String {
    path.components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn normalize_str(path: &str) -> String {
    normalize_path(Path::new(path))
}

/// `path` rendered relative to `repo_root`, forward-slashed.
///
/// An analyzer may spell a claim absolutely, and comparing that raw against a
/// repo-relative root yields no match at all — which presents as a rescue layer
/// that never fires, the same silent-disabling shape [`crate::runner`] normalizes
/// against.
fn relative_to(path: &Path, repo_root: &Path) -> String {
    normalize_path(path.strip_prefix(repo_root).unwrap_or(path))
}

// ---------------------------------------------------------------------------
// The rescue layer
// ---------------------------------------------------------------------------

/// One claim the root set rescued, with the provenance that rescued it.
///
/// §9.13 asks for a conflict list rather than a score, and §7.3 records that the
/// best-validated prior art in the document — IntelliJ's Safe Delete — shows the
/// *usage list*. A rescue that cannot say which tier and which rule fired is a
/// score wearing a longer name, and for Tier B it is worse than that: an
/// unlabelled convention rescue is a guess about a framework presenting as a
/// fact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RescuedClaim {
    /// The claim, spelled exactly as the analyzer spelled it.
    pub claim: String,
    /// Whether that was a path or a symbol.
    pub kind: ClaimKind,
    /// Which §5.1 tier the root came from. **The load-bearing field.**
    pub tier: Tier,
    /// Which rule fired: `executable`, `django/appconfig`, `declared`.
    pub rule: String,
    /// The exact file and key that declared it.
    pub origin: String,
    /// The file half of the origin, so a reader can open it.
    pub origin_file: Option<PathBuf>,
    /// What the root points at, as declared.
    pub target: String,
    /// The whole reason, in a sentence somebody can act on.
    pub detail: String,
}

/// What the root set did during one call to the inner SUT.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootsRun {
    /// The repository the root set was materialized over.
    pub repo: PathBuf,
    /// Claims the analyzer made.
    pub claimed: usize,
    /// Claims that were not roots.
    pub survived: usize,
    /// Every claim that was, with its provenance. In the order the analyzer made
    /// them, never sorted by anything that would flatter the layer (§9.13
    /// invariant 3).
    pub rescued: Vec<RescuedClaim>,
    /// How many roots were materialized over this repository.
    pub roots_materialized: usize,
    /// Everything the materializer could not resolve. Carried per run rather
    /// than summarized, because a rescue count next to an unreported gap is the
    /// §6.20 shape: a layer that found five roots and missed a framework's
    /// entire convention set looks exactly like one that found five roots.
    pub gaps: Vec<Gap>,
}

/// Any [`Sut`], with the materialized root set (§5) run over every claim it
/// makes.
///
/// # A pure filter, and nothing else
///
/// The accuser runs first, and every survivor is checked against the root set.
/// Nothing here can add a claim, promote one, or turn a rescue into an
/// accusation — the only operation is dropping, and `tests/roots_gate.rs`
/// asserts the subset relation on the claim sets rather than on their sizes.
///
/// # Why this is a different layer from the veto, not a wider veto
///
/// Gate 2 asks whether anything in the repository *names* the candidate. m10 is
/// built so that nothing does: `ReportingConfig` occurs in `reporting/apps.py`
/// and nowhere else, because Django finds it by scanning that file for an
/// `AppConfig` subclass. No tuning of the needles reaches that, because there is
/// no second occurrence to find. The root set answers a different question —
/// *was this declared an entry point* — and it is the only layer that can.
pub struct RootedSut {
    inner: Box<dyn Sut>,
    name: String,
    /// One entry per call to [`Sut::run`], in call order. `RefCell` because
    /// [`Sut::run`] takes `&self` and this wrapper additionally has to record
    /// what it did; single-threaded by construction, since
    /// [`crate::runner::run_suite`] drives one mutant at a time.
    runs: RefCell<Vec<RootsRun>>,
}

impl RootedSut {
    /// `inner`, with every claim checked against the root set.
    pub fn new(inner: Box<dyn Sut>) -> RootedSut {
        let name = format!("{}+roots", inner.name());
        RootedSut {
            inner,
            name,
            runs: RefCell::new(Vec::new()),
        }
    }

    /// What the root set did, one entry per call to the inner SUT, in call
    /// order.
    pub fn runs(&self) -> Vec<RootsRun> {
        self.runs.borrow().clone()
    }
}

impl Sut for RootedSut {
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

        // The claimed paths are the candidate set the Tier C pathspecs are
        // matched against: this run is considering exactly these, and a
        // declaration only becomes a root against a candidate.
        let candidates: Vec<String> = claims
            .claimed_dead_paths
            .iter()
            .map(|path| relative_to(path, repo))
            .collect();
        let set = materialize(repo, &candidates);

        let claimed = claims.claimed_dead_paths.len() + claims.claimed_dead_symbols.len();
        let mut rescued: Vec<RescuedClaim> = Vec::new();
        let mut claimed_dead_paths: Vec<PathBuf> = Vec::new();
        let mut claimed_dead_symbols: Vec<SymbolClaim> = Vec::new();

        for path in &claims.claimed_dead_paths {
            match set.rescues_path(&relative_to(path, repo)) {
                Some(root) => {
                    rescued.push(record(path.display().to_string(), ClaimKind::Path, root))
                }
                None => claimed_dead_paths.push(path.clone()),
            }
        }
        for symbol in &claims.claimed_dead_symbols {
            match set.rescues_symbol(symbol.name()) {
                Some(root) => {
                    rescued.push(record(symbol.name().to_string(), ClaimKind::Symbol, root))
                }
                None => claimed_dead_symbols.push(symbol.clone()),
            }
        }

        let survived = claimed_dead_paths.len() + claimed_dead_symbols.len();
        self.runs.borrow_mut().push(RootsRun {
            repo: repo.to_path_buf(),
            claimed,
            survived,
            rescued,
            roots_materialized: set.roots().len(),
            gaps: set.gaps().to_vec(),
        });

        Ok(SutVerdict {
            claimed_dead_paths,
            claimed_dead_symbols,
        })
    }
}

/// One rescue, recorded with everything a reader needs to check it.
fn record(claim: String, kind: ClaimKind, root: &Root) -> RescuedClaim {
    RescuedClaim {
        claim,
        kind,
        tier: root.tier,
        rule: root.rule.clone(),
        origin: root.origin.clone(),
        origin_file: Some(root.origin_file.clone()),
        target: root.target.clone(),
        detail: format!(
            "{} [tier {}: {}]",
            root.detail,
            root.tier,
            root.tier.caveat()
        ),
    }
}
