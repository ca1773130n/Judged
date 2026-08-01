//! Tier B — roots a framework creates by convention (§5.1).
//!
//! A Tier B root is a file some framework turns into an entry point **with no
//! source reference anywhere**. Django ≥3.2 instantiates the single `AppConfig`
//! subclass it finds in `<app>/apps.py`; Jest substitutes `__mocks__/<pkg>.js`
//! for a package with no `jest.mock()` call. In both cases the edge lives in the
//! framework's loader, not in the repository, so a call graph, a compiler index
//! and a module resolver all agree the file is unreachable — and all three are
//! *right*. E2 class m10 is exactly this shape, and it is one of the two the
//! reference veto of Gate 2 could not rescue: `ReportingConfig` occurs once in
//! its repository, at its own declaration, so there is no needle to search for.
//! That is not a veto failure. It is a root-set failure, and this is the layer
//! that owns it.
//!
//! This module does not decide what is reachable. It materializes what the
//! convention declares, records the evidence, and prints it for a human
//! ([`ConventionScan::printseeds`], §9.13).
//!
//! # A convention that fires without its framework fabricates roots
//!
//! §5.1 rates Tier B "correct only if framework **and version** detected
//! correctly", so detection is a precondition here rather than a heuristic: no
//! rule runs until its framework has been found, and every root carries the
//! [`Detection`] that licensed it. A rule that fired on layout alone would make
//! every repository containing a file called `settings.py` a Django project, and
//! a cleaner buried in fabricated roots stops deleting anything at all — which
//! is the same product as no cleaner.
//!
//! One corollary is easy to get wrong and is enforced here: **evidence may not
//! justify itself.** `settings.py` and `apps.py` are Django *roots*; neither is
//! accepted as proof that Django is present, because a rule whose own output is
//! its justification is a tautology. Proof has to come from a manifest
//! dependency, a framework-generated marker file such as `manage.py`, or a
//! config section such as `[tool.pytest.ini_options]` — see [`SIGNATURES`].
//!
//! # The registry is deliberately small, and says where it stops
//!
//! §11 R2 asks whether a framework registry is a moat or an unbounded
//! liability, and the prior art is discouraging: knip needs 178 plugins and a
//! full-time maintainer, depcheck died at 4.9k stars with 116 open issues of
//! exactly this debt, and knip's Next.js plugin already branches on `app/`
//! versus `src/app/` because the convention changed between majors. So this
//! module ships five plugins and makes the shape extensible — one [`Framework`]
//! variant, one row in [`SIGNATURES`], one [`ConventionPlugin`] impl — rather
//! than racing toward coverage.
//!
//! Being small is only honest if the gaps are reported. A framework that is
//! detected but has no plugin, and a root list that is visible but unresolvable,
//! are both emitted as [`KnownUnknown`]s and both set
//! [`ConventionScan::tier_capped`]. §9.5 caps the tier in precisely those cases,
//! and can only do so because this module says them out loud. That is §6.20's
//! rule applied to root discovery: *"no data" must be a distinct state from
//! "zero"*. Silence about a framework we recognized but cannot analyze would be
//! read downstream as "this framework contributes no roots", which is how a
//! convention-loaded file gets deleted.
//!
//! # What this layer cannot do
//!
//! Three limits, stated because a limit a caller cannot see is indistinguishable
//! from a bug:
//!
//! - **Coverage is per rule, not per framework.** A plugin existing for Django
//!   does not mean every Django convention is implemented, and
//!   [`ConventionScan::tier_capped`] does not claim otherwise — it reports the
//!   frameworks with *no* plugin, not the rules a plugin is missing. E2 class
//!   m11 is the live example: a Django model's fields, read reflectively by the
//!   ORM, are convention-live symbols inside an already-live file, and no rule
//!   here enumerates them.
//! - **Roots are files.** A root may name the symbol the framework loads (m10's
//!   `AppConfig`), but a convention whose unit *is* a symbol set — m11 again —
//!   is not expressible in this module's output.
//! - **A bespoke loader is not a convention.** E2 class m03 imports every
//!   `plugins/*.py` it finds; there is no framework to detect and no published
//!   rule to know, and inventing a `plugins/` rule to appear to cover it is the
//!   treadmill §11 R2 warns about. m03 is rescued, if at all, by the
//!   reflection-primitive signal of §6.1 in the veto layer.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::{Error, Result};

/// Directories never walked.
///
/// Every entry is either version-control metadata or build/vendor output, and
/// the reason to skip them is correctness before speed: a root materialized
/// from `node_modules/next/app/page.js` belongs to a dependency, not to this
/// repository, and would be reported against a path no human here can act on.
const SKIPPED_DIRS: [&str; 13] = [
    ".git",
    ".hg",
    "node_modules",
    "target",
    "dist",
    "build",
    ".venv",
    "venv",
    "__pycache__",
    ".tox",
    ".mypy_cache",
    ".pytest_cache",
    ".next",
];

// ---------------------------------------------------------------------------
// Provenance
// ---------------------------------------------------------------------------

/// The §5.1 provenance tier a root came from.
///
/// Carried as a value on every root rather than implied by which module
/// produced it, because a mixed root set is exactly where the distinction
/// matters: a caller holding a `Vec` of roots must be able to tell a manifest's
/// declaration from a guess about a framework without knowing how the `Vec` was
/// assembled. This module only ever emits
/// [`ProvenanceTier::ConventionInferable`]; the other two variants exist so the
/// vocabulary is complete where it is read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ProvenanceTier {
    /// A build system or deploy target already reads this file to find roots.
    MachineDeclared,
    /// A framework's file layout makes this file an entry point. Correct only
    /// if the framework and its version were detected correctly.
    ConventionInferable,
    /// The live set is determined by data or intent outside the repository.
    /// Must be solicited from a human.
    Undiscoverable,
}

impl ProvenanceTier {
    /// `"A"`, `"B"` or `"C"` — the labels §5.1 uses, and what `-printseeds`
    /// output leads with.
    pub fn label(self) -> &'static str {
        match self {
            ProvenanceTier::MachineDeclared => "A",
            ProvenanceTier::ConventionInferable => "B",
            ProvenanceTier::Undiscoverable => "C",
        }
    }
}

// ---------------------------------------------------------------------------
// Frameworks
// ---------------------------------------------------------------------------

/// A framework this module can *recognize*.
///
/// Recognizing is not the same as covering: [`Framework::has_plugin`] separates
/// the five whose conventions are implemented from the ones that exist here
/// only so that finding them produces a [`KnownUnknown`] instead of silence.
/// Adding recognition without coverage is cheap and honest; adding coverage is
/// the expensive half, and §11 R2 is the argument for doing it slowly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Framework {
    // Covered by a plugin.
    Django,
    Pytest,
    Jest,
    NextJs,
    Rails,
    // Recognized only. Each of these has conventions this module does not know,
    // so detecting one is a reported gap.
    Flask,
    FastApi,
    Celery,
    Airflow,
    Nuxt,
    SvelteKit,
    Vue,
    Angular,
    Sidekiq,
}

impl Framework {
    /// The name used in reports. Lower-case and stable, since it is matched in
    /// tests and read by humans.
    pub fn name(self) -> &'static str {
        match self {
            Framework::Django => "django",
            Framework::Pytest => "pytest",
            Framework::Jest => "jest",
            Framework::NextJs => "next",
            Framework::Rails => "rails",
            Framework::Flask => "flask",
            Framework::FastApi => "fastapi",
            Framework::Celery => "celery",
            Framework::Airflow => "airflow",
            Framework::Nuxt => "nuxt",
            Framework::SvelteKit => "sveltekit",
            Framework::Vue => "vue",
            Framework::Angular => "angular",
            Framework::Sidekiq => "sidekiq",
        }
    }

    /// Whether this module knows this framework's conventions. `false` means a
    /// detection turns into a [`KnownUnknown`] and caps the tier (§9.5).
    pub fn has_plugin(self) -> bool {
        plugin_for(self).is_some()
    }
}

/// How a framework's presence can be proved.
///
/// The rows are the whole registry. A new framework is one variant on
/// [`Framework`] plus one row here; coverage for it is one [`ConventionPlugin`]
/// impl in [`PLUGINS`]. Nothing else in this module needs to change, which is
/// the "extensible shape" §11 R2 asks for in place of a race to 178 plugins.
///
/// Note what is *absent*: no row names a file that is also a root. `conftest.py`
/// does not prove pytest, `apps.py` does not prove Django. Evidence has to be
/// independent of the thing it licenses.
const SIGNATURES: &[Signature] = &[
    Signature {
        framework: Framework::Django,
        packages: &["django"],
        markers: &["manage.py"],
        config_sections: &[],
    },
    Signature {
        framework: Framework::Pytest,
        packages: &["pytest"],
        markers: &["pytest.ini"],
        config_sections: &[
            ("pyproject.toml", "[tool.pytest.ini_options]"),
            ("tox.ini", "[pytest]"),
            ("setup.cfg", "[tool:pytest]"),
        ],
    },
    Signature {
        framework: Framework::Jest,
        packages: &["jest"],
        markers: &[
            "jest.config.js",
            "jest.config.mjs",
            "jest.config.cjs",
            "jest.config.ts",
            "jest.config.json",
        ],
        config_sections: &[("package.json", "\"jest\":")],
    },
    Signature {
        framework: Framework::NextJs,
        packages: &["next"],
        markers: &["next.config.js", "next.config.mjs", "next.config.ts"],
        config_sections: &[],
    },
    Signature {
        framework: Framework::Rails,
        packages: &["rails"],
        markers: &["config/application.rb", "bin/rails"],
        config_sections: &[],
    },
    // Recognized, not covered.
    Signature {
        framework: Framework::Flask,
        packages: &["flask"],
        markers: &[],
        config_sections: &[],
    },
    Signature {
        framework: Framework::FastApi,
        packages: &["fastapi"],
        markers: &[],
        config_sections: &[],
    },
    Signature {
        framework: Framework::Celery,
        packages: &["celery"],
        markers: &[],
        config_sections: &[],
    },
    Signature {
        framework: Framework::Airflow,
        packages: &["apache-airflow"],
        markers: &[],
        config_sections: &[],
    },
    Signature {
        framework: Framework::Nuxt,
        packages: &["nuxt"],
        markers: &["nuxt.config.ts", "nuxt.config.js"],
        config_sections: &[],
    },
    Signature {
        framework: Framework::SvelteKit,
        packages: &["@sveltejs/kit"],
        markers: &[],
        config_sections: &[],
    },
    Signature {
        framework: Framework::Vue,
        packages: &["vue"],
        markers: &[],
        config_sections: &[],
    },
    Signature {
        framework: Framework::Angular,
        packages: &["@angular/core"],
        markers: &["angular.json"],
        config_sections: &[],
    },
    Signature {
        framework: Framework::Sidekiq,
        packages: &["sidekiq"],
        markers: &[],
        config_sections: &[],
    },
];

/// One framework's detection rules. See [`SIGNATURES`].
struct Signature {
    framework: Framework,
    /// Distribution or package names, matched case-insensitively against the
    /// dependencies declared in any manifest in the tree.
    packages: &'static [&'static str],
    /// Files whose presence the framework itself creates or requires. Matched
    /// against the whole relative path or against a trailing path segment, so a
    /// framework nested inside a monorepo is still found.
    markers: &'static [&'static str],
    /// `(file, literal)` pairs: the file exists and contains the literal.
    config_sections: &'static [(&'static str, &'static str)],
}

// ---------------------------------------------------------------------------
// Evidence
// ---------------------------------------------------------------------------

/// What kind of thing was read to produce a piece of evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EvidenceKind {
    /// A dependency declared in a package manifest — the strongest form,
    /// because it is the only one that also carries a version requirement.
    ManifestDependency,
    /// A file the framework itself generates or requires, such as `manage.py`.
    MarkerFile,
    /// A framework's own table inside a shared config file, such as
    /// `[tool.pytest.ini_options]` in `pyproject.toml`.
    ConfigSection,
    /// A literal list of roots written in a settings module, such as Django's
    /// `INSTALLED_APPS`.
    SettingsList,
    /// The same list, loaded from a data file rather than written inline.
    ConfigFile,
}

impl EvidenceKind {
    /// Stable label for `-printseeds` output.
    pub fn label(self) -> &'static str {
        match self {
            EvidenceKind::ManifestDependency => "manifest-dependency",
            EvidenceKind::MarkerFile => "marker-file",
            EvidenceKind::ConfigSection => "config-section",
            EvidenceKind::SettingsList => "settings-list",
            EvidenceKind::ConfigFile => "config-file",
        }
    }
}

/// A single, checkable fact: this file said this thing.
///
/// The path is repo-relative so a report reads the same wherever the scan ran,
/// and `detail` is what a human should look at once they open the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Evidence {
    kind: EvidenceKind,
    path: PathBuf,
    detail: String,
}

impl Evidence {
    fn new(kind: EvidenceKind, path: impl Into<PathBuf>, detail: impl Into<String>) -> Evidence {
        Evidence {
            kind,
            path: path.into(),
            detail: detail.into(),
        }
    }

    pub fn kind(&self) -> EvidenceKind {
        self.kind
    }

    /// Repo-relative path of the file the evidence was read from.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// What in that file constitutes the evidence.
    pub fn detail(&self) -> &str {
        &self.detail
    }
}

/// A framework, found, with the proof and the version requirement that was
/// declared alongside it.
///
/// `declared_version` is `None` whenever the framework was proved by something
/// that does not carry a version — a marker file, a config section. §5.1 makes
/// version part of Tier B's correctness condition, so that `None` is a real
/// caveat and is printed as `version-unknown` rather than dropped: it is the
/// difference between "we know this is Next.js 15, whose router lives in
/// `app/`" and "we accept both `app/` and `src/app/` because we cannot tell".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Detection {
    framework: Framework,
    evidence: Evidence,
    declared_version: Option<String>,
}

impl Detection {
    pub fn framework(&self) -> Framework {
        self.framework
    }

    pub fn evidence(&self) -> &Evidence {
        &self.evidence
    }

    /// The version *requirement* as declared, e.g. `">=4.2"` or `"^29.7.0"` —
    /// not a resolved version, which only a lockfile or an installed
    /// environment knows.
    pub fn declared_version(&self) -> Option<&str> {
        self.declared_version.as_deref()
    }
}

// ---------------------------------------------------------------------------
// Roots
// ---------------------------------------------------------------------------

/// Which convention fired.
///
/// Named per rule rather than per framework so a report can say *why* a file is
/// a root, and so the fire rate of each rule can be measured rather than
/// guessed — §11 R2 wants the shape of the precision curve as registry size
/// grows, which is unanswerable without per-rule attribution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Rule {
    /// A package named by `INSTALLED_APPS`.
    DjangoInstalledApp,
    /// The `AppConfig` subclass Django instantiates from `<app>/apps.py`.
    DjangoAppConfig,
    /// `<app>/management/commands/<name>.py`, invoked by name from a shell.
    DjangoManagementCommand,
    /// A URLConf, named by a settings string this module does not resolve.
    DjangoUrlConf,
    /// A settings module, named by `DJANGO_SETTINGS_MODULE` at deploy time.
    DjangoSettingsModule,
    /// `conftest.py`, imported by pytest itself.
    PytestConftest,
    /// A module matching pytest's default `python_files` patterns.
    PytestTestModule,
    /// A file under `__mocks__/`, substituted by the Jest runner.
    JestManualMock,
    /// `app/**/page.{js,jsx,ts,tsx}`.
    NextAppRouterPage,
    /// `app/**/layout.{js,jsx,ts,tsx}`.
    NextAppRouterLayout,
    /// `app/**/route.{js,jsx,ts,tsx}`.
    NextAppRouterRoute,
    /// The rest of the app router's reserved filenames (§5.1 lists them):
    /// `template`, `default`, `error`, `global-error`, `loading`, `not-found`.
    NextAppRouterSpecialFile,
    /// `config/routes.rb`.
    RailsRoutes,
    /// `config/initializers/*.rb`, auto-run at boot.
    RailsInitializer,
    /// `app/jobs/**/*.rb`, named by serialized queue payloads.
    RailsJob,
}

impl Rule {
    /// Stable `framework/rule` label for `-printseeds` output.
    pub fn label(self) -> &'static str {
        match self {
            Rule::DjangoInstalledApp => "django/installed-app",
            Rule::DjangoAppConfig => "django/appconfig",
            Rule::DjangoManagementCommand => "django/management-command",
            Rule::DjangoUrlConf => "django/urlconf",
            Rule::DjangoSettingsModule => "django/settings-module",
            Rule::PytestConftest => "pytest/conftest",
            Rule::PytestTestModule => "pytest/test-module",
            Rule::JestManualMock => "jest/manual-mock",
            Rule::NextAppRouterPage => "next/app-router-page",
            Rule::NextAppRouterLayout => "next/app-router-layout",
            Rule::NextAppRouterRoute => "next/app-router-route",
            Rule::NextAppRouterSpecialFile => "next/app-router-special-file",
            Rule::RailsRoutes => "rails/routes",
            Rule::RailsInitializer => "rails/initializer",
            Rule::RailsJob => "rails/job",
        }
    }
}

/// One materialized Tier B root.
///
/// Every field except `symbol` and `source` is mandatory, and there is no
/// public constructor: a root cannot be fabricated without the detection that
/// licensed it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConventionRoot {
    path: PathBuf,
    symbol: Option<String>,
    rule: Rule,
    detection: Detection,
    source: Option<Evidence>,
}

impl ConventionRoot {
    /// Repo-relative path of the root.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The symbol the framework loads, where the convention names one — the
    /// `AppConfig` subclass, for instance. This is frequently the only place
    /// that name exists outside its own declaration, which is exactly why m10
    /// is unreachable for a literal veto.
    pub fn symbol(&self) -> Option<&str> {
        self.symbol.as_deref()
    }

    pub fn rule(&self) -> Rule {
        self.rule
    }

    pub fn framework(&self) -> Framework {
        self.detection.framework
    }

    /// Why we believe the framework is present at all.
    pub fn detection(&self) -> &Detection {
        &self.detection
    }

    /// For rules that read a list, the file that named *this* root — a settings
    /// module or the config file its list was loaded from. `None` for rules
    /// that fire on layout alone, where the path is its own justification.
    pub fn source(&self) -> Option<&Evidence> {
        self.source.as_ref()
    }

    /// Always [`ProvenanceTier::ConventionInferable`]. It is a method rather
    /// than an assumption a caller has to make.
    pub fn tier(&self) -> ProvenanceTier {
        ProvenanceTier::ConventionInferable
    }
}

// ---------------------------------------------------------------------------
// Known unknowns
// ---------------------------------------------------------------------------

/// Why this module knows that it does not know something.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnknownReason {
    /// The framework was detected and this module has no plugin for it. Its
    /// convention roots — however many there are — are all missing.
    NoPlugin,
    /// A list of roots was found but is computed at run time, and no data file
    /// holding it could be resolved. `setting` names the list.
    UnresolvedRootList { setting: String },
}

impl UnknownReason {
    /// One sentence a human can act on, printed by
    /// [`ConventionScan::printseeds`].
    pub fn message(&self) -> String {
        match self {
            UnknownReason::NoPlugin => {
                "framework detected, no plugin — its convention roots are missing".to_string()
            }
            UnknownReason::UnresolvedRootList { setting } => {
                format!("{setting} is computed at run time and no data file holding it was found")
            }
        }
    }
}

/// A gap this module found and is reporting rather than swallowing.
///
/// §6.20's rule, applied to root discovery: *"no data" must be a distinct state
/// from "zero"*. A framework detected with no plugin contributes no roots, and
/// a downstream consumer that cannot tell that apart from "this framework has
/// no roots" will delete a convention-loaded file. §9.5 caps the tier on this
/// signal; [`ConventionScan::tier_capped`] is the boolean form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnownUnknown {
    framework: Framework,
    reason: UnknownReason,
    evidence: Evidence,
}

impl KnownUnknown {
    pub fn framework(&self) -> Framework {
        self.framework
    }

    pub fn reason(&self) -> &UnknownReason {
        &self.reason
    }

    /// Where to look. For [`UnknownReason::NoPlugin`] this is the detection's
    /// own evidence; for an unresolved list it is the file the list was in.
    pub fn evidence(&self) -> &Evidence {
        &self.evidence
    }
}

// ---------------------------------------------------------------------------
// The scan
// ---------------------------------------------------------------------------

/// Everything one scan found, and everything it knows it missed.
///
/// Fields are private and there is no public constructor, so a scan result
/// cannot be assembled by anything other than [`scan`].
#[derive(Debug, Clone)]
pub struct ConventionScan {
    roots: Vec<ConventionRoot>,
    detections: Vec<Detection>,
    known_unknowns: Vec<KnownUnknown>,
    files_scanned: usize,
}

impl ConventionScan {
    /// Every materialized root, sorted by path then rule.
    pub fn roots(&self) -> &[ConventionRoot] {
        &self.roots
    }

    /// Every framework found, covered or not, in [`Framework`] order.
    pub fn detections(&self) -> &[Detection] {
        &self.detections
    }

    /// Every gap. Empty means "we understood everything we recognized", not
    /// "we recognized everything".
    pub fn known_unknowns(&self) -> &[KnownUnknown] {
        &self.known_unknowns
    }

    /// How many files the walk actually visited. A scan that reports roots over
    /// a corpus of zero files has not looked (§6.20).
    pub fn files_scanned(&self) -> usize {
        self.files_scanned
    }

    /// Whether §9.5 must cap the tier for this repository.
    pub fn tier_capped(&self) -> bool {
        !self.known_unknowns.is_empty()
    }

    /// The `-printseeds` dump §9.13 asks for: one line per root, one per known
    /// unknown, each carrying its tier, its rule, its framework and the
    /// evidence, so the classification can be audited before anything acts on
    /// it. Roots lead with their tier label, known unknowns with `?`.
    pub fn printseeds(&self) -> String {
        let mut out = String::new();
        for root in &self.roots {
            let symbol = match root.symbol() {
                Some(symbol) => format!("::{symbol}"),
                None => String::new(),
            };
            let version = root
                .detection
                .declared_version
                .as_deref()
                .unwrap_or("version-unknown");
            let source = match root.source() {
                Some(source) => format!(
                    "; named by {} {}",
                    source.kind().label(),
                    source.path().display()
                ),
                None => String::new(),
            };
            out.push_str(&format!(
                "{} {}{}  {}  {} {} via {} {}{}\n",
                root.tier().label(),
                root.path().display(),
                symbol,
                root.rule().label(),
                root.framework().name(),
                version,
                root.detection.evidence.kind().label(),
                root.detection.evidence.path().display(),
                source,
            ));
        }
        for unknown in &self.known_unknowns {
            out.push_str(&format!(
                "? {}  {} — tier capped (§9.5)  via {} {}\n",
                unknown.framework().name(),
                unknown.reason().message(),
                unknown.evidence().kind().label(),
                unknown.evidence().path().display(),
            ));
        }
        out
    }
}

/// Materialize every Tier B root under `root`.
///
/// Takes a directory, not a repository: a convention is a claim about layout on
/// disk, and requiring a git index to make it would invent a precondition the
/// convention does not have.
///
/// # Errors
///
/// Any failure to walk the tree or read a file it listed. This never degrades
/// to an empty result: "this repository has no entry points" is the most
/// dangerous sentence this module could utter, and it must never be the way an
/// unreadable directory presents (§6.20).
pub fn scan(root: &Path) -> Result<ConventionScan> {
    let tree = Tree::walk(root)?;

    let detections = detect(&tree)?;
    let mut sink = Sink::default();
    for detection in detections.values() {
        match plugin_for(detection.framework) {
            Some(plugin) => plugin.roots(&tree, detection, &mut sink)?,
            None => sink.unknown(
                detection.framework,
                UnknownReason::NoPlugin,
                detection.evidence.clone(),
            ),
        }
    }

    let mut roots = sink.roots;
    roots.sort_by(|a, b| a.path.cmp(&b.path).then(a.rule.cmp(&b.rule)));

    Ok(ConventionScan {
        roots,
        detections: detections.into_values().collect(),
        known_unknowns: sink.unknowns,
        files_scanned: tree.files.len(),
    })
}

/// Collects what the plugins produce, refusing duplicates.
///
/// Two settings modules naming the same app must not produce the same root
/// twice; a report that lists a file three times reads as three findings.
#[derive(Default)]
struct Sink {
    roots: Vec<ConventionRoot>,
    unknowns: Vec<KnownUnknown>,
    seen: BTreeSet<(PathBuf, Rule)>,
}

impl Sink {
    fn root(
        &mut self,
        path: &str,
        symbol: Option<String>,
        rule: Rule,
        detection: &Detection,
        source: Option<Evidence>,
    ) {
        let path = PathBuf::from(path);
        if !self.seen.insert((path.clone(), rule)) {
            return;
        }
        self.roots.push(ConventionRoot {
            path,
            symbol,
            rule,
            detection: detection.clone(),
            source,
        });
    }

    /// A root that fires on layout alone, where the path is its own
    /// justification and there is no list to attribute it to.
    fn layout_root(&mut self, path: &str, rule: Rule, detection: &Detection) {
        self.root(path, None, rule, detection, None);
    }

    fn unknown(&mut self, framework: Framework, reason: UnknownReason, evidence: Evidence) {
        let unknown = KnownUnknown {
            framework,
            reason,
            evidence,
        };
        if !self.unknowns.contains(&unknown) {
            self.unknowns.push(unknown);
        }
    }
}

// ---------------------------------------------------------------------------
// The tree
// ---------------------------------------------------------------------------

/// Every file under the scan root, as `/`-separated relative paths, sorted.
///
/// Sorted because every downstream decision — which settings module is read
/// first, which config file resolves a list — must not depend on directory
/// traversal order, or two runs over the same repository disagree.
struct Tree {
    root: PathBuf,
    files: Vec<String>,
}

impl Tree {
    fn walk(root: &Path) -> Result<Tree> {
        let mut files = Vec::new();
        walk_dir(root, "", &mut files)?;
        files.sort();
        Ok(Tree {
            root: root.to_path_buf(),
            files,
        })
    }

    /// The first file whose path is `name` or ends in `/name`. The suffix match
    /// is what finds a Django project nested inside a monorepo.
    fn find_suffix(&self, name: &str) -> Option<&str> {
        let suffix = format!("/{name}");
        self.files
            .iter()
            .find(|file| file.as_str() == name || file.ends_with(&suffix))
            .map(String::as_str)
    }

    fn contains(&self, rel: &str) -> bool {
        self.files.iter().any(|file| file == rel)
    }

    /// Read a file the walk listed.
    ///
    /// Lossy UTF-8: a settings module in an unexpected encoding must not abort
    /// the scan, and the identifiers this module looks for are ASCII. A failure
    /// to *read* is still an error — the file was listed, so it exists.
    fn read(&self, rel: &str) -> Result<String> {
        let path = self.root.join(rel);
        let bytes = std::fs::read(&path).map_err(|source| Error::Io { path, source })?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }
}

fn walk_dir(root: &Path, rel: &str, out: &mut Vec<String>) -> Result<()> {
    let dir = if rel.is_empty() {
        root.to_path_buf()
    } else {
        root.join(rel)
    };
    let entries = std::fs::read_dir(&dir).map_err(|source| Error::Io {
        path: dir.clone(),
        source,
    })?;
    for entry in entries {
        let entry = entry.map_err(|source| Error::Io {
            path: dir.clone(),
            source,
        })?;
        let name = entry.file_name();
        // A non-UTF-8 filename cannot match any convention, all of which are
        // spelled in ASCII, so skipping it loses nothing.
        let Some(name) = name.to_str() else { continue };
        // `file_type` on a `DirEntry` does not follow symlinks, so a symlinked
        // directory is neither descended into nor able to create a cycle.
        let kind = entry.file_type().map_err(|source| Error::Io {
            path: entry.path(),
            source,
        })?;
        let child = if rel.is_empty() {
            name.to_string()
        } else {
            format!("{rel}/{name}")
        };
        if kind.is_dir() {
            if SKIPPED_DIRS.contains(&name) {
                continue;
            }
            walk_dir(root, &child, out)?;
        } else if kind.is_file() {
            out.push(child);
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Detection
// ---------------------------------------------------------------------------

/// One dependency, as some manifest declared it.
struct Declared {
    name: String,
    version: Option<String>,
    manifest: String,
    section: String,
}

/// Find every framework in the tree, best evidence first.
///
/// "Best" is [`EvidenceKind`] order: a manifest dependency beats a marker file
/// beats a config section, because only the first carries a version, and §5.1
/// makes version part of Tier B's correctness condition.
fn detect(tree: &Tree) -> Result<BTreeMap<Framework, Detection>> {
    let declared = declared_dependencies(tree)?;
    let mut found: BTreeMap<Framework, Detection> = BTreeMap::new();

    for signature in SIGNATURES {
        let mut best: Option<Detection> = None;

        for dependency in &declared {
            if signature
                .packages
                .iter()
                .any(|package| package.eq_ignore_ascii_case(&dependency.name))
            {
                best = Some(Detection {
                    framework: signature.framework,
                    evidence: Evidence::new(
                        EvidenceKind::ManifestDependency,
                        dependency.manifest.as_str(),
                        format!("{} in {}", dependency.name, dependency.section),
                    ),
                    declared_version: dependency.version.clone(),
                });
                break;
            }
        }

        if best.is_none() {
            for marker in signature.markers {
                if let Some(path) = tree.find_suffix(marker) {
                    best = Some(Detection {
                        framework: signature.framework,
                        evidence: Evidence::new(
                            EvidenceKind::MarkerFile,
                            path,
                            format!("{marker} is generated by {}", signature.framework.name()),
                        ),
                        declared_version: None,
                    });
                    break;
                }
            }
        }

        if best.is_none() {
            for (file, needle) in signature.config_sections {
                let Some(path) = tree.find_suffix(file) else {
                    continue;
                };
                let path = path.to_string();
                if tree.read(&path)?.contains(needle) {
                    best = Some(Detection {
                        framework: signature.framework,
                        evidence: Evidence::new(
                            EvidenceKind::ConfigSection,
                            path,
                            (*needle).to_string(),
                        ),
                        declared_version: None,
                    });
                    break;
                }
            }
        }

        if let Some(detection) = best {
            found.insert(signature.framework, detection);
        }
    }

    Ok(found)
}

/// Every dependency declared by every manifest in the tree.
fn declared_dependencies(tree: &Tree) -> Result<Vec<Declared>> {
    let mut declared = Vec::new();
    for file in &tree.files {
        let name = basename(file);
        let parsed = match name {
            "pyproject.toml" => parse_pyproject(&tree.read(file)?),
            "package.json" => parse_package_json(&tree.read(file)?),
            "Gemfile" => parse_gemfile(&tree.read(file)?),
            _ if name.starts_with("requirements") && name.ends_with(".txt") => {
                parse_requirements(&tree.read(file)?)
            }
            _ => continue,
        };
        for (name, version, section) in parsed {
            declared.push(Declared {
                name,
                version,
                manifest: file.clone(),
                section,
            });
        }
    }
    Ok(declared)
}

/// PEP 621 `dependencies` arrays, `[project.optional-dependencies]` groups, and
/// Poetry's `name = "spec"` tables.
///
/// A line-oriented reader rather than a TOML parse, because this crate does not
/// depend on a TOML library and a dependency name is the only thing being
/// extracted. The consequence is stated rather than hidden: a dependency spelled
/// with an inline table (`django = { version = "^4.2" }`) yields no version, and
/// a version this module cannot read is reported as `version-unknown`, never
/// guessed.
fn parse_pyproject(text: &str) -> Vec<(String, Option<String>, String)> {
    let mut out = Vec::new();
    let mut section = String::new();
    let mut array: Option<String> = None;
    let mut buffer = String::new();

    for line in text.lines() {
        let trimmed = line.trim();

        if let Some(open) = array.clone() {
            buffer.push_str(line);
            if trimmed.contains(']') {
                for entry in string_literals(&buffer) {
                    let (name, version) = split_requirement(&entry);
                    out.push((name, version, open.clone()));
                }
                array = None;
                buffer.clear();
            }
            continue;
        }

        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            section = trimmed.trim_matches(['[', ']']).to_string();
            continue;
        }

        let Some((key, value)) = trimmed.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();

        let is_dependency_array =
            key == "dependencies" || section == "project.optional-dependencies";
        if is_dependency_array && value.starts_with('[') {
            let label = if section.is_empty() {
                "dependencies".to_string()
            } else {
                format!("[{section}] {key}")
            };
            if value.contains(']') {
                for entry in string_literals(value) {
                    let (name, version) = split_requirement(&entry);
                    out.push((name, version, label.clone()));
                }
            } else {
                array = Some(label);
                buffer.push_str(value);
            }
            continue;
        }

        // Poetry-style tables: `django = "^4.2"`.
        if section.starts_with("tool.poetry") && section.ends_with("dependencies") {
            let version = string_literals(value).into_iter().next();
            out.push((key.to_string(), version, format!("[{section}]")));
        }
    }
    out
}

/// `dependencies` and `devDependencies` from a `package.json`.
fn parse_package_json(text: &str) -> Vec<(String, Option<String>, String)> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
        // An unparseable `package.json` declares nothing this module can read.
        // It is not evidence of absence, and it is not treated as any: the
        // framework simply goes undetected, exactly as if the file were not
        // there.
        return Vec::new();
    };
    let mut out = Vec::new();
    for section in ["dependencies", "devDependencies", "peerDependencies"] {
        let Some(map) = value.get(section).and_then(serde_json::Value::as_object) else {
            continue;
        };
        for (name, spec) in map {
            out.push((
                name.clone(),
                spec.as_str().map(str::to_string),
                section.to_string(),
            ));
        }
    }
    out
}

/// One requirement per line, `#` comments and `-r`/`-e` directives skipped.
fn parse_requirements(text: &str) -> Vec<(String, Option<String>, String)> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() || line.starts_with('-') {
            continue;
        }
        let (name, version) = split_requirement(line);
        if !name.is_empty() {
            out.push((name, version, "requirements".to_string()));
        }
    }
    out
}

/// `gem "rails", "~> 7.1"` lines.
fn parse_gemfile(text: &str) -> Vec<(String, Option<String>, String)> {
    let mut out = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with("gem ") && !trimmed.starts_with("gem\t") {
            continue;
        }
        let mut literals = string_literals(trimmed).into_iter();
        let Some(name) = literals.next() else {
            continue;
        };
        out.push((name, literals.next(), "Gemfile".to_string()));
    }
    out
}

/// Split a PEP 508-ish requirement into its distribution name and the rest.
///
/// `celery[redis]>=5.3` is `("celery", Some(">=5.3"))`: extras are part of the
/// requirement, not of the name, and a version requirement is not a version.
fn split_requirement(requirement: &str) -> (String, Option<String>) {
    let requirement = requirement.trim();
    let end = requirement
        .find(|c: char| "<>=!~^;,()[]@ \t".contains(c))
        .unwrap_or(requirement.len());
    let (name, rest) = requirement.split_at(end);
    let mut rest = rest.trim();
    // Skip an extras group, which sits between the name and the specifier.
    if let Some(stripped) = rest.strip_prefix('[') {
        rest = stripped
            .split_once(']')
            .map_or("", |(_, after)| after)
            .trim();
    }
    let version = if rest.is_empty() {
        None
    } else {
        Some(rest.to_string())
    };
    (name.trim().to_lowercase(), version)
}

/// Every single- or double-quoted string in `text`, in order, with `#` comments
/// skipped. Used wherever a real parser is not warranted.
fn string_literals(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '#' => {
                for c in chars.by_ref() {
                    if c == '\n' {
                        break;
                    }
                }
            }
            '"' | '\'' => {
                let quote = c;
                let mut literal = String::new();
                let mut escaped = false;
                for c in chars.by_ref() {
                    if escaped {
                        literal.push(c);
                        escaped = false;
                    } else if c == '\\' {
                        escaped = true;
                    } else if c == quote {
                        break;
                    } else {
                        literal.push(c);
                    }
                }
                out.push(literal);
            }
            _ => {}
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Plugins
// ---------------------------------------------------------------------------

/// One framework's conventions.
///
/// Deliberately crate-private. §11 R2's warning is about maintenance burden,
/// not about third-party extensibility, and freezing this as public API before
/// the shape of the registry is known would be the more expensive mistake. The
/// extension point is a new impl in [`PLUGINS`].
trait ConventionPlugin: Sync {
    fn framework(&self) -> Framework;

    /// Emit this framework's roots, and any gap that stopped the plugin short.
    /// `detection` is the proof the framework is present; it is a parameter
    /// rather than something the plugin looks up, so that no plugin can run
    /// without one.
    fn roots(&self, tree: &Tree, detection: &Detection, sink: &mut Sink) -> Result<()>;
}

/// The covered frameworks. Everything else recognized by [`SIGNATURES`] becomes
/// a [`KnownUnknown`] when found.
const PLUGINS: &[&dyn ConventionPlugin] = &[&Django, &Pytest, &Jest, &NextJs, &Rails];

fn plugin_for(framework: Framework) -> Option<&'static dyn ConventionPlugin> {
    PLUGINS
        .iter()
        .find(|plugin| plugin.framework() == framework)
        .copied()
}

// --- Django ----------------------------------------------------------------

struct Django;

impl ConventionPlugin for Django {
    fn framework(&self) -> Framework {
        Framework::Django
    }

    fn roots(&self, tree: &Tree, detection: &Detection, sink: &mut Sink) -> Result<()> {
        let settings: Vec<String> = tree
            .files
            .iter()
            .filter(|file| is_settings_module(file))
            .cloned()
            .collect();

        for module in &settings {
            sink.layout_root(module, Rule::DjangoSettingsModule, detection);
        }
        for module in &settings {
            self.installed_apps(tree, detection, sink, module)?;
        }

        for file in &tree.files {
            if basename(file) == "apps.py" {
                for symbol in app_config_subclasses(&tree.read(file)?) {
                    sink.root(file, Some(symbol), Rule::DjangoAppConfig, detection, None);
                }
            }
            if is_management_command(file) {
                sink.layout_root(file, Rule::DjangoManagementCommand, detection);
            }
            if basename(file) == "urls.py" {
                sink.layout_root(file, Rule::DjangoUrlConf, detection);
            }
        }
        Ok(())
    }
}

impl Django {
    /// Materialize `INSTALLED_APPS`, following it into a data file when the
    /// list is computed rather than written inline — which it routinely is, for
    /// per-environment app sets. When it cannot be followed, that is reported
    /// (§6.20), because an unresolved list read as an empty one silently drops
    /// every app in it.
    fn installed_apps(
        &self,
        tree: &Tree,
        detection: &Detection,
        sink: &mut Sink,
        module: &str,
    ) -> Result<()> {
        let text = tree.read(module)?;
        let parsed = parse_list_assignment(&text, "INSTALLED_APPS");
        if !parsed.assigned {
            return Ok(());
        }

        let settings_evidence = Evidence::new(EvidenceKind::SettingsList, module, "INSTALLED_APPS");
        for entry in &parsed.entries {
            self.emit_app(tree, detection, sink, entry, &settings_evidence);
        }
        if parsed.all_literal {
            return Ok(());
        }

        match find_list_in_json(tree, "installed_apps")? {
            Some((file, entries)) => {
                let evidence = Evidence::new(EvidenceKind::ConfigFile, file, "installed_apps");
                for entry in &entries {
                    self.emit_app(tree, detection, sink, entry, &evidence);
                }
            }
            None => sink.unknown(
                Framework::Django,
                UnknownReason::UnresolvedRootList {
                    setting: "INSTALLED_APPS".to_string(),
                },
                settings_evidence,
            ),
        }
        Ok(())
    }

    /// Turn one `INSTALLED_APPS` entry into a root, if the package it names
    /// lives in this repository. `django.contrib.contenttypes` does not, and a
    /// third-party package is not a root of the repository being scanned.
    fn emit_app(
        &self,
        tree: &Tree,
        detection: &Detection,
        sink: &mut Sink,
        entry: &str,
        source: &Evidence,
    ) {
        let (package, symbol) = split_app_entry(entry);
        let base = package.replace('.', "/");
        for candidate in [format!("{base}/__init__.py"), format!("{base}.py")] {
            if tree.contains(&candidate) {
                sink.root(
                    &candidate,
                    symbol,
                    Rule::DjangoInstalledApp,
                    detection,
                    Some(source.clone()),
                );
                return;
            }
        }
    }
}

/// `settings.py`, or any `.py` inside a `settings/` package.
fn is_settings_module(rel: &str) -> bool {
    if basename(rel) == "settings.py" {
        return true;
    }
    rel.ends_with(".py") && rel.split('/').rev().nth(1) == Some("settings")
}

/// `<app>/management/commands/<name>.py`, excluding the package marker.
fn is_management_command(rel: &str) -> bool {
    if !rel.ends_with(".py") || basename(rel) == "__init__.py" {
        return false;
    }
    let parts: Vec<&str> = rel.split('/').collect();
    parts.len() >= 3
        && parts[parts.len() - 2] == "commands"
        && parts[parts.len() - 3] == "management"
}

/// `"reporting.apps.ReportingConfig"` is the package `reporting` plus the class
/// `ReportingConfig`; `"reporting"` is the package alone.
fn split_app_entry(entry: &str) -> (String, Option<String>) {
    let parts: Vec<&str> = entry.split('.').collect();
    if parts.len() >= 3 && parts[parts.len() - 2] == "apps" {
        (
            parts[..parts.len() - 2].join("."),
            Some(parts[parts.len() - 1].to_string()),
        )
    } else {
        (entry.to_string(), None)
    }
}

/// Every class in `text` whose base list mentions `AppConfig`.
///
/// A textual match rather than a parse. The failure direction is deliberate:
/// this over-reports rather than under-reports, and an over-reported root costs
/// disk while an under-reported one costs an incident (§1.3).
fn app_config_subclasses(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        let Some(rest) = line.trim_start().strip_prefix("class ") else {
            continue;
        };
        let Some((name, bases)) = rest.split_once('(') else {
            continue;
        };
        let Some((bases, _)) = bases.split_once(')') else {
            continue;
        };
        if bases.contains("AppConfig") {
            let name = name.trim();
            if !name.is_empty() {
                out.push(name.to_string());
            }
        }
    }
    out
}

/// What a Python list assignment yielded.
struct ListAssignment {
    /// The name was assigned at least once. `false` means it only appears in a
    /// comment or a read, and there is nothing to resolve.
    assigned: bool,
    /// Every assignment to the name was a literal list, so `entries` is the
    /// whole list and nothing is missing.
    all_literal: bool,
    entries: Vec<String>,
}

/// Read `NAME = [...]` (and `NAME += [...]`) out of Python source.
///
/// Anchored at the start of a line so that a mention inside a comment or an
/// expression is not mistaken for the definition. Anything that is not a
/// bracketed literal — a function call, a config lookup, a comprehension —
/// leaves `all_literal` false, which is what turns into a reported gap rather
/// than a silently short list.
fn parse_list_assignment(text: &str, name: &str) -> ListAssignment {
    let mut result = ListAssignment {
        assigned: false,
        all_literal: true,
        entries: Vec::new(),
    };

    let mut offset = 0usize;
    for line in text.split_inclusive('\n') {
        let indent = line.len() - line.trim_start().len();
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix(name) {
            let rest_trimmed = rest.trim_start();
            let is_assignment = (rest_trimmed.starts_with('=') && !rest_trimmed.starts_with("=="))
                || rest_trimmed.starts_with("+=");
            if is_assignment {
                result.assigned = true;
                let eq = offset + indent + name.len() + (rest.len() - rest_trimmed.len());
                let after_eq = if rest_trimmed.starts_with("+=") {
                    eq + 2
                } else {
                    eq + 1
                };
                match bracketed_literals(&text[after_eq..]) {
                    Some(entries) => result.entries.extend(entries),
                    None => result.all_literal = false,
                }
            }
        }
        offset += line.len();
    }
    result
}

/// Collect the string literals of a bracketed literal starting at the first
/// non-whitespace character of `text`, or `None` if it does not start one or
/// never closes.
fn bracketed_literals(text: &str) -> Option<Vec<String>> {
    let mut chars = text.char_indices();
    let mut start = None;
    for (index, c) in chars.by_ref() {
        if c.is_whitespace() {
            continue;
        }
        if c == '[' || c == '(' {
            start = Some(index);
        }
        break;
    }
    let start = start?;

    let mut depth = 0usize;
    let mut end = None;
    let mut rest = text[start..].char_indices();
    while let Some((index, c)) = rest.next() {
        match c {
            '[' | '(' => depth += 1,
            ']' | ')' => {
                depth -= 1;
                if depth == 0 {
                    end = Some(start + index);
                    break;
                }
            }
            '"' | '\'' => {
                let quote = c;
                let mut escaped = false;
                let mut closed = false;
                for (_, c) in rest.by_ref() {
                    if escaped {
                        escaped = false;
                    } else if c == '\\' {
                        escaped = true;
                    } else if c == quote {
                        closed = true;
                        break;
                    }
                }
                if !closed {
                    return None;
                }
            }
            _ => {}
        }
    }
    let end = end?;
    Some(string_literals(&text[start..=end]))
}

/// Find an array of strings stored under `key` in any JSON file in the tree.
///
/// JSON only, and that limit is load-bearing rather than incidental: a YAML or
/// TOML app list is *not* resolved here and therefore surfaces as a
/// [`KnownUnknown`], which is the honest outcome. Files that do not parse are
/// skipped — an unrelated malformed JSON file is not evidence about Django one
/// way or the other.
fn find_list_in_json(tree: &Tree, key: &str) -> Result<Option<(String, Vec<String>)>> {
    for file in &tree.files {
        if !file.ends_with(".json") {
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&tree.read(file)?) else {
            continue;
        };
        if let Some(entries) = find_string_array(&value, key) {
            return Ok(Some((file.clone(), entries)));
        }
    }
    Ok(None)
}

/// Depth-first search for an object key equal to `key`, case-insensitively,
/// whose value is an array of strings.
fn find_string_array(value: &serde_json::Value, key: &str) -> Option<Vec<String>> {
    match value {
        serde_json::Value::Object(map) => {
            for (name, child) in map {
                if name.eq_ignore_ascii_case(key) {
                    if let Some(array) = child.as_array() {
                        let entries: Vec<String> = array
                            .iter()
                            .filter_map(|item| item.as_str().map(str::to_string))
                            .collect();
                        if entries.len() == array.len() && !entries.is_empty() {
                            return Some(entries);
                        }
                    }
                }
                if let Some(found) = find_string_array(child, key) {
                    return Some(found);
                }
            }
            None
        }
        serde_json::Value::Array(items) => {
            items.iter().find_map(|item| find_string_array(item, key))
        }
        _ => None,
    }
}

// --- pytest ----------------------------------------------------------------

struct Pytest;

impl ConventionPlugin for Pytest {
    fn framework(&self) -> Framework {
        Framework::Pytest
    }

    /// `conftest.py` is imported by pytest itself, and the test modules are
    /// collected by filename. `python_files` is configurable; this implements
    /// pytest's documented default (`test_*.py` and `*_test.py`) and does not
    /// read an override, so a repository that customizes it has roots this
    /// plugin misses.
    fn roots(&self, tree: &Tree, detection: &Detection, sink: &mut Sink) -> Result<()> {
        for file in &tree.files {
            let name = basename(file);
            if name == "conftest.py" {
                sink.layout_root(file, Rule::PytestConftest, detection);
            } else if name.ends_with(".py")
                && (name.starts_with("test_") || name.ends_with("_test.py"))
            {
                sink.layout_root(file, Rule::PytestTestModule, detection);
            }
        }
        Ok(())
    }
}

// --- Jest ------------------------------------------------------------------

struct Jest;

impl ConventionPlugin for Jest {
    fn framework(&self) -> Framework {
        Framework::Jest
    }

    /// A file under any `__mocks__/` directory. The directory name is the
    /// entire registration: Jest substitutes a root `__mocks__/<package>.js`
    /// for a node_modules package with no `jest.mock()` call anywhere, so there
    /// is nothing else in the repository to find. This is m10's JavaScript
    /// half.
    fn roots(&self, tree: &Tree, detection: &Detection, sink: &mut Sink) -> Result<()> {
        for file in &tree.files {
            if file.split('/').any(|part| part == "__mocks__") {
                sink.layout_root(file, Rule::JestManualMock, detection);
            }
        }
        Ok(())
    }
}

// --- Next.js ---------------------------------------------------------------

struct NextJs;

impl ConventionPlugin for NextJs {
    fn framework(&self) -> Framework {
        Framework::NextJs
    }

    /// The app router's reserved filenames, under both `app/` and `src/app/`.
    ///
    /// Both, because knip's own Next.js plugin branches on exactly that
    /// difference — the convention changed between majors (§11 R2) — and a
    /// declared range like `^15.0.0` does not say which layout a given build
    /// resolved. Accepting both over-reports by at most the files of a layout
    /// the project does not use, which is the safe direction.
    fn roots(&self, tree: &Tree, detection: &Detection, sink: &mut Sink) -> Result<()> {
        for file in &tree.files {
            if !file.starts_with("app/") && !file.starts_with("src/app/") {
                continue;
            }
            let name = basename(file);
            let Some((stem, extension)) = name.rsplit_once('.') else {
                continue;
            };
            if !matches!(extension, "js" | "jsx" | "ts" | "tsx") {
                continue;
            }
            let rule = match stem {
                "page" => Rule::NextAppRouterPage,
                "layout" => Rule::NextAppRouterLayout,
                "route" => Rule::NextAppRouterRoute,
                "template" | "default" | "error" | "global-error" | "loading" | "not-found" => {
                    Rule::NextAppRouterSpecialFile
                }
                _ => continue,
            };
            sink.layout_root(file, rule, detection);
        }
        Ok(())
    }
}

// --- Rails -----------------------------------------------------------------

struct Rails;

impl ConventionPlugin for Rails {
    fn framework(&self) -> Framework {
        Framework::Rails
    }

    /// The router is read by the framework, initializers are auto-run at boot,
    /// and jobs are named by serialized queue payloads rather than by callers —
    /// the last being E2 class m15's shape as well as this one's.
    fn roots(&self, tree: &Tree, detection: &Detection, sink: &mut Sink) -> Result<()> {
        for file in &tree.files {
            if file == "config/routes.rb" {
                sink.layout_root(file, Rule::RailsRoutes, detection);
            } else if file.starts_with("config/initializers/") && file.ends_with(".rb") {
                sink.layout_root(file, Rule::RailsInitializer, detection);
            } else if file.starts_with("app/jobs/") && file.ends_with(".rb") {
                sink.layout_root(file, Rule::RailsJob, detection);
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------

fn basename(rel: &str) -> &str {
    rel.rsplit('/').next().unwrap_or(rel)
}
