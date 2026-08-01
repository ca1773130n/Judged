//! Tier B — framework conventions that make a file an entry point (§5.1).
//!
//! Five properties decide whether this layer is sound, and each has its own
//! test below:
//!
//! 1. **It rescues the shapes nothing else can.** E2 class m10 is a Django
//!    `AppConfig` and a Jest `__mocks__` file, neither of which is named
//!    anywhere in its repository. Gate 2's literal veto structurally cannot
//!    reach them — there is no needle — so if Tier B does not produce them,
//!    nothing does.
//! 2. **A convention never fires without its framework.** §5.1 rates Tier B
//!    "correct only if framework + version detected correctly". A rule that
//!    fires on layout alone fabricates roots, and a cleaner drowning in
//!    fabricated roots deletes nothing at all.
//! 3. **Evidence never justifies itself.** `settings.py` is a Django root, but
//!    it may not be the proof that Django is present, or the rule becomes a
//!    tautology that fires on any repository containing that filename.
//! 4. **A detected framework with no plugin is a KNOWN UNKNOWN, not silence.**
//!    §9.5 caps the tier in exactly that case and can only do so if this module
//!    says so out loud. The same holds for a root list this module can see but
//!    cannot resolve (§6.20: "no data" is a distinct state from "zero").
//! 5. **Provenance survives to the human.** §9.13 asks for `-printseeds`: every
//!    root prints its tier, its rule, its framework and the evidence.
//!
//! The two repository shapes under test are transcribed from the real E2
//! fixtures — `crates/judged-mutants/src/fixtures/m10_framework_convention.rs`
//! and `m03_plugin_dir_scan.rs`. They are transcribed rather than imported
//! because `judged-mutants` depends on `judged-core`, and this crate's
//! manifest is not this test's to change.

use std::path::Path;

use judged_core::roots::convention::{
    scan, ConventionRoot, ConventionScan, EvidenceKind, Framework, ProvenanceTier, Rule,
    UnknownReason,
};
use tempfile::TempDir;

/// Materialize `files` into a fresh temporary directory.
///
/// No git repository: a Tier B root is a claim about **layout on disk**, and
/// requiring an index to make that claim would be inventing a precondition the
/// convention does not have.
fn tree(files: &[(&str, &str)]) -> TempDir {
    let dir = TempDir::new().expect("tempdir");
    for (rel, body) in files {
        let path = dir.path().join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("mkdir -p");
        }
        std::fs::write(&path, body).expect("write fixture file");
    }
    dir
}

/// Every root path, sorted, as forward-slashed strings.
fn root_paths(scan: &ConventionScan) -> Vec<String> {
    let mut paths: Vec<String> = scan
        .roots()
        .iter()
        .map(|root| root.path().to_string_lossy().replace('\\', "/"))
        .collect();
    paths.sort();
    paths
}

/// The single root at `rel`, or a failure naming everything that *was* found.
fn root_at<'a>(scan: &'a ConventionScan, rel: &str) -> &'a ConventionRoot {
    scan.roots()
        .iter()
        .find(|root| root.path() == Path::new(rel))
        .unwrap_or_else(|| {
            panic!(
                "no convention root for {rel}; roots were {:?}, detections {:?}",
                root_paths(scan),
                scan.detections()
                    .iter()
                    .map(|d| d.framework().name())
                    .collect::<Vec<_>>()
            )
        })
}

/// E2 class m10, transcribed. Two frameworks, two conventions, and neither
/// live file is named by anything in the tree.
fn m10_framework_convention() -> TempDir {
    tree(&[
        (
            "pyproject.toml",
            "[project]\nname = \"billing\"\nversion = \"0.1.0\"\n\
             dependencies = [\"django>=4.2\"]\n",
        ),
        (
            "billing/settings.py",
            "INSTALLED_APPS = [\n    \"django.contrib.contenttypes\",\n    \"reporting\",\n]\n\n\
             SECRET_KEY = \"fixture-only\"\n",
        ),
        ("reporting/__init__.py", ""),
        (
            "reporting/apps.py",
            "from django.apps import AppConfig\n\n\n\
             class ReportingConfig(AppConfig):\n    name = \"reporting\"\n    \
             verbose_name = \"Reporting\"\n\n    def ready(self):\n        pass\n",
        ),
        (
            "package.json",
            "{\n  \"name\": \"billing-web\",\n  \"version\": \"0.1.0\",\n  \"private\": true,\n  \
             \"scripts\": { \"test\": \"jest\" },\n  \"dependencies\": { \"redis\": \"^4.6.0\" },\n  \
             \"devDependencies\": { \"jest\": \"^29.7.0\" }\n}\n",
        ),
        (
            "src/cache.js",
            "const { createClient } = require(\"redis\");\nmodule.exports = { warm: null };\n",
        ),
        (
            "tests/cache.test.js",
            "const { warm } = require(\"../src/cache\");\n",
        ),
        (
            "__mocks__/redis.js",
            "// Stands in for the real client during tests.\nconst store = new Map();\n\
             module.exports = { createClient: () => store };\n",
        ),
        // The decoys. Genuinely dead in the fixture's ground truth.
        (
            "reporting/textwrap_helper.py",
            "def hang_indent(text, width=72):\n    return text\n",
        ),
        (
            "src/color_utils.js",
            "function toHex(rgb) {\n  return rgb;\n}\nmodule.exports = { toHex };\n",
        ),
    ])
}

// ---------------------------------------------------------------------------
// 1. The shapes nothing else can reach
// ---------------------------------------------------------------------------

/// m10's Django half. `ReportingConfig` occurs exactly once in the repository,
/// at its own declaration site, so a literal veto has no needle to search for.
/// The convention — Django ≥3.2 instantiates the single `AppConfig` subclass in
/// `<app>/apps.py` — is the only thing that makes this file an entry point.
#[test]
fn m10_django_appconfig_is_a_root_with_its_symbol() {
    let dir = m10_framework_convention();
    let scan = scan(dir.path()).expect("scan m10");

    let root = root_at(&scan, "reporting/apps.py");
    assert_eq!(root.rule(), Rule::DjangoAppConfig);
    assert_eq!(root.framework(), Framework::Django);
    assert_eq!(
        root.symbol(),
        Some("ReportingConfig"),
        "the class Django instantiates must be named on the root, since it is \
         named nowhere else in the repository"
    );
}

/// m10's JavaScript half. Jest substitutes a root `__mocks__/<package>.js` for
/// the real package with no `jest.mock()` call anywhere; the directory name is
/// the entire registration.
#[test]
fn m10_jest_manual_mock_is_a_root() {
    let dir = m10_framework_convention();
    let scan = scan(dir.path()).expect("scan m10");

    let root = root_at(&scan, "__mocks__/redis.js");
    assert_eq!(root.rule(), Rule::JestManualMock);
    assert_eq!(root.framework(), Framework::Jest);
}

/// The rest of m10's Django surface: the settings module itself, and the app
/// package named by `INSTALLED_APPS`. `django.contrib.contenttypes` is not a
/// root *of this repository* — it does not live here.
#[test]
fn m10_installed_apps_entry_and_settings_module_are_roots() {
    let dir = m10_framework_convention();
    let scan = scan(dir.path()).expect("scan m10");

    let installed = root_at(&scan, "reporting/__init__.py");
    assert_eq!(installed.rule(), Rule::DjangoInstalledApp);
    assert_eq!(
        installed.source().map(|source| source.kind()),
        Some(EvidenceKind::SettingsList),
        "the list was literal in the settings module, and the root says which \
         file named it"
    );
    assert_eq!(
        installed.source().map(|source| source.path()),
        Some(Path::new("billing/settings.py"))
    );
    assert_eq!(
        root_at(&scan, "billing/settings.py").rule(),
        Rule::DjangoSettingsModule
    );
    assert!(
        !root_paths(&scan)
            .iter()
            .any(|path| path.contains("contenttypes")),
        "a third-party app is not a root of this repository: {:?}",
        root_paths(&scan)
    );
}

/// The whole point of the exercise: the two live files are rescued and the two
/// decoys are not. A layer that rescued everything would be indistinguishable
/// from refusing to delete anything.
#[test]
fn m10_decoys_are_not_roots() {
    let dir = m10_framework_convention();
    let scan = scan(dir.path()).expect("scan m10");
    let paths = root_paths(&scan);

    for decoy in ["reporting/textwrap_helper.py", "src/color_utils.js"] {
        assert!(
            !paths.contains(&decoy.to_string()),
            "{decoy} is a genuinely dead decoy and must not be claimed as a root; \
             roots were {paths:?}"
        );
    }
    assert!(!paths.contains(&"src/cache.js".to_string()), "{paths:?}");
}

// ---------------------------------------------------------------------------
// 2 and 3. A convention never fires without its framework, and evidence never
//          justifies itself
// ---------------------------------------------------------------------------

/// Every convention shape in this module's repertoire, in one tree, with no
/// manifest declaring any framework at all.
///
/// Nothing may fire. Note in particular that `billing/settings.py` contains
/// `INSTALLED_APPS` and `reporting/apps.py` contains an `AppConfig` subclass:
/// both are Django *roots*, and neither may serve as the *evidence* that Django
/// is present. A rule whose own output is its justification fires on every
/// repository that happens to use the filename.
#[test]
fn conventions_do_not_fire_without_a_detected_framework() {
    let dir = tree(&[
        (
            "billing/settings.py",
            "INSTALLED_APPS = [\n    \"reporting\",\n]\n",
        ),
        ("reporting/__init__.py", ""),
        (
            "reporting/apps.py",
            "class ReportingConfig(AppConfig):\n    name = \"reporting\"\n",
        ),
        ("conftest.py", "import sys\n"),
        ("tests/test_billing.py", "def test_x():\n    pass\n"),
        ("__mocks__/redis.js", "module.exports = {};\n"),
        ("app/page.tsx", "export default function Page() {}\n"),
        (
            "config/routes.rb",
            "Rails.application.routes.draw do\nend\n",
        ),
    ]);
    let scan = scan(dir.path()).expect("scan an undeclared tree");

    assert_eq!(
        root_paths(&scan),
        Vec::<String>::new(),
        "layout alone is not evidence; every one of these rules must stay silent"
    );
    assert!(scan.detections().is_empty(), "{:?}", scan.printseeds());
    assert!(
        scan.known_unknowns().is_empty(),
        "an undetected framework is not a known unknown — there is nothing to know"
    );
}

/// The inverse control for the test above: the identical Django layout with one
/// line of manifest added fires every Django rule. Without this, "nothing fired"
/// above would be consistent with a module that never fires at all.
#[test]
fn the_same_layout_fires_once_the_framework_is_declared() {
    let dir = tree(&[
        (
            "pyproject.toml",
            "[project]\nname = \"billing\"\ndependencies = [\"django>=4.2\"]\n",
        ),
        (
            "billing/settings.py",
            "INSTALLED_APPS = [\n    \"reporting\",\n]\n",
        ),
        ("reporting/__init__.py", ""),
        (
            "reporting/apps.py",
            "class ReportingConfig(AppConfig):\n    name = \"reporting\"\n",
        ),
    ]);
    let scan = scan(dir.path()).expect("scan a declared tree");

    assert_eq!(
        root_paths(&scan),
        vec![
            "billing/settings.py".to_string(),
            "reporting/__init__.py".to_string(),
            "reporting/apps.py".to_string(),
        ]
    );
}

/// §5.1: Tier B is "correct only if framework **and version** detected
/// correctly". The declared version requirement is therefore carried on the
/// evidence, not discarded — knip's Next.js plugin already branches on `app/`
/// versus `src/app/` because the convention changed between majors (§11 R2).
#[test]
fn every_root_carries_its_framework_evidence_and_declared_version() {
    let dir = m10_framework_convention();
    let scan = scan(dir.path()).expect("scan m10");

    let django = root_at(&scan, "reporting/apps.py").detection();
    assert_eq!(django.evidence().kind(), EvidenceKind::ManifestDependency);
    assert_eq!(django.evidence().path(), Path::new("pyproject.toml"));
    assert_eq!(django.declared_version(), Some(">=4.2"));

    let jest = root_at(&scan, "__mocks__/redis.js").detection();
    assert_eq!(jest.evidence().kind(), EvidenceKind::ManifestDependency);
    assert_eq!(jest.evidence().path(), Path::new("package.json"));
    assert_eq!(jest.declared_version(), Some("^29.7.0"));
}

/// A root that does not say which tier it came from invites a caller to trust a
/// guessed convention as though a manifest had declared it (roots/mod.rs).
#[test]
fn every_convention_root_reports_tier_b() {
    let dir = m10_framework_convention();
    let scan = scan(dir.path()).expect("scan m10");

    assert!(!scan.roots().is_empty());
    for root in scan.roots() {
        assert_eq!(
            root.tier(),
            ProvenanceTier::ConventionInferable,
            "{} claimed tier {:?}",
            root.path().display(),
            root.tier()
        );
        assert_eq!(root.tier().label(), "B");
    }
}

// ---------------------------------------------------------------------------
// 4. Known unknowns, not silence
// ---------------------------------------------------------------------------

/// §11 R2 is the reason this registry is deliberately small: knip needs 178
/// plugins and a full-time maintainer, depcheck died at 4.9k stars with 116
/// open issues of exactly this debt. Being small is only honest if the gaps are
/// *reported*: §9.5 caps the tier when a framework is detected with no matching
/// plugin, and it can only do that if this module emits the signal.
#[test]
fn a_detected_framework_with_no_plugin_is_a_known_unknown_that_caps_the_tier() {
    let dir = tree(&[
        (
            "pyproject.toml",
            "[project]\nname = \"billing\"\n\
             dependencies = [\"django>=4.2\", \"celery>=5.3\"]\n",
        ),
        ("billing/settings.py", "INSTALLED_APPS = []\n"),
    ]);
    let scan = scan(dir.path()).expect("scan");

    let unknown = scan
        .known_unknowns()
        .iter()
        .find(|unknown| unknown.framework() == Framework::Celery)
        .unwrap_or_else(|| panic!("celery was detected but not reported as uncovered"));
    assert_eq!(unknown.reason(), &UnknownReason::NoPlugin);
    assert_eq!(unknown.evidence().path(), Path::new("pyproject.toml"));
    assert!(
        scan.tier_capped(),
        "a detected framework with no plugin must cap the tier (§9.5)"
    );

    // And the covered framework in the same repository is unaffected.
    assert!(scan
        .known_unknowns()
        .iter()
        .all(|unknown| unknown.framework() != Framework::Django));

    // The covered/recognized split is the registry's honesty, so pin it: a
    // framework this module recognizes but cannot analyze must answer `false`,
    // and that answer is what turns a detection into the gap above.
    assert!(Framework::Django.has_plugin());
    assert!(!Framework::Celery.has_plugin());
}

/// A repository whose frameworks are all covered must not cap the tier, or the
/// cap means nothing.
#[test]
fn a_fully_covered_repository_does_not_cap_the_tier() {
    let dir = m10_framework_convention();
    let scan = scan(dir.path()).expect("scan m10");

    assert!(
        !scan.tier_capped(),
        "nothing should be capped here: {:?}",
        scan.printseeds()
    );
    assert!(scan.known_unknowns().is_empty());
}

/// `INSTALLED_APPS` is routinely assembled at runtime — from a deploy config,
/// an environment split, a plugin discovery pass. The list still has to be
/// materialized, so the config file is followed and the root records *it* as
/// the evidence rather than the settings module.
#[test]
fn installed_apps_loaded_from_a_config_file_still_yields_roots() {
    let dir = tree(&[
        (
            "pyproject.toml",
            "[project]\nname = \"billing\"\ndependencies = [\"django>=4.2\"]\n",
        ),
        (
            "billing/settings.py",
            "import json, pathlib\n\
             CONFIG = json.loads(pathlib.Path(\"deploy/apps.json\").read_text())\n\
             INSTALLED_APPS = CONFIG[\"installed_apps\"]\n",
        ),
        (
            "deploy/apps.json",
            "{\n  \"installed_apps\": [\"reporting\"]\n}\n",
        ),
        ("reporting/__init__.py", ""),
    ]);
    let scan = scan(dir.path()).expect("scan");

    let root = root_at(&scan, "reporting/__init__.py");
    assert_eq!(root.rule(), Rule::DjangoInstalledApp);
    let source = root
        .source()
        .expect("a root read out of a list must name the file the list was in");
    assert_eq!(
        source.kind(),
        EvidenceKind::ConfigFile,
        "the list came from a data file, and the provenance must say so rather \
         than pointing at the settings module that did not contain it"
    );
    assert_eq!(source.path(), Path::new("deploy/apps.json"));
    assert_eq!(
        root.detection().evidence().kind(),
        EvidenceKind::ManifestDependency,
        "the framework claim is still the dependency; only the list moved"
    );
    assert!(
        scan.known_unknowns().is_empty(),
        "the list was resolved, so nothing is unknown: {:?}",
        scan.printseeds()
    );
}

/// The same repository with the config file gone. §6.20's rule — "no data" must
/// be a distinct state from "zero" — applies to root discovery exactly as it
/// applies to analyzer output: an `INSTALLED_APPS` this module can see but
/// cannot resolve is a reported gap, never an empty list.
#[test]
fn an_unresolvable_installed_apps_is_a_known_unknown_not_an_empty_list() {
    let dir = tree(&[
        (
            "pyproject.toml",
            "[project]\nname = \"billing\"\ndependencies = [\"django>=4.2\"]\n",
        ),
        (
            "billing/settings.py",
            "from billing.plugins import discover\nINSTALLED_APPS = discover()\n",
        ),
        ("reporting/__init__.py", ""),
    ]);
    let scan = scan(dir.path()).expect("scan");

    let unknown = scan
        .known_unknowns()
        .iter()
        .find(|unknown| matches!(unknown.reason(), UnknownReason::UnresolvedRootList { .. }))
        .unwrap_or_else(|| {
            panic!(
                "a computed INSTALLED_APPS must be reported, not silently read as empty: {:?}",
                scan.printseeds()
            )
        });
    assert_eq!(unknown.framework(), Framework::Django);
    assert_eq!(unknown.evidence().path(), Path::new("billing/settings.py"));
    assert!(scan.tier_capped());
}

/// E2 class m11, the other mutant Gate 2 could not rescue: a Django model whose
/// fields are read reflectively by the ORM and named nowhere else.
///
/// It is **not** rescued here, and the point of this test is that the module
/// does not pretend otherwise. Coverage is per rule, not per framework: Django
/// has a plugin, so nothing is reported as uncovered, yet no rule enumerates
/// model fields and this module's roots are files rather than symbol sets.
/// Anyone who later makes `tier_capped()` mean "every convention of every
/// detected framework is implemented" has to come here and change this.
#[test]
fn m11_reflective_model_fields_are_not_rescued_and_the_cap_does_not_claim_they_are() {
    let dir = tree(&[
        (
            "pyproject.toml",
            "[project]\nname = \"app\"\ndependencies = [\"django>=4.2\"]\n",
        ),
        (
            "app/models.py",
            "class RetentionPolicy:\n    tenant_slug = \"\"\n    retention_days = 0\n    \
             legal_hold_until = None\n",
        ),
        (
            "app/serialize.py",
            "def dump(model):\n    return {\n        name: getattr(model, name)\n        \
             for name in type(model).model_fields\n    }\n",
        ),
    ]);
    let scan = scan(dir.path()).expect("scan");

    assert_eq!(
        scan.detections().len(),
        1,
        "Django is present and detected, so the plugin did run: {:?}",
        scan.printseeds()
    );
    assert_eq!(scan.detections()[0].framework(), Framework::Django);
    assert_eq!(
        root_paths(&scan),
        Vec::<String>::new(),
        "the plugin ran over m11's tree and rescued nothing — no rule covers a \
         model file, and inventing one would be a guess about the ORM"
    );
    assert!(
        !scan.tier_capped(),
        "Django has a plugin, so nothing is uncovered — which is exactly why the \
         cap must not be read as a completeness claim: {:?}",
        scan.printseeds()
    );
}

/// E2 class m03: a bespoke loader that imports every `plugins/*.py` it finds.
/// There is no framework here, so there is no convention to know, and this
/// module must **not** invent a `plugins/` rule to appear to cover it — that is
/// precisely the treadmill §11 R2 warns about. m03 is rescued, if at all, by the
/// reflection-primitive signal of §6.1, which lives in the veto layer.
#[test]
fn m03_plugin_dir_scan_is_out_of_this_layers_reach_and_it_says_so() {
    let dir = tree(&[
        (
            "pyproject.toml",
            "[project]\nname = \"pluginhost\"\nversion = \"0.1.0\"\nrequires-python = \">=3.11\"\n",
        ),
        ("pluginhost/__init__.py", ""),
        (
            "pluginhost/loader.py",
            "import importlib\nfrom pathlib import Path\n\n\
             PLUGIN_DIR = Path(__file__).with_name(\"plugins\")\n\n\n\
             def load_all():\n    for path in sorted(PLUGIN_DIR.glob(\"*.py\")):\n        \
             yield importlib.import_module(f\"{__package__}.plugins.{path.stem}\")\n",
        ),
        ("pluginhost/plugins/__init__.py", ""),
        (
            "pluginhost/plugins/tsvwriter.py",
            "EXTENSION = \".tsv\"\n\n\ndef emit(rows):\n    return rows\n",
        ),
        (
            "pluginhost/main.py",
            "from .loader import load_all\n\n\ndef main():\n    for module in load_all():\n        \
             print(module.__name__)\n",
        ),
        (
            "pluginhost/textwrap_helper.py",
            "def hang_indent(text, width=72):\n    return text\n",
        ),
    ]);
    let scan = scan(dir.path()).expect("scan m03");

    assert_eq!(
        root_paths(&scan),
        Vec::<String>::new(),
        "no framework is declared, so no convention may fire"
    );
    assert!(scan.detections().is_empty());
    assert!(
        scan.files_scanned() >= 7,
        "the walk must have actually run: {} files",
        scan.files_scanned()
    );
}

// ---------------------------------------------------------------------------
// The rest of the repertoire
// ---------------------------------------------------------------------------

/// pytest collects `conftest.py` and `test_*.py`/`*_test.py` by name; nothing
/// imports them. Detection here comes from the `[tool.pytest.ini_options]`
/// table rather than a dependency, because that is how most repositories
/// declare it — and `conftest.py` itself is deliberately not accepted as
/// evidence.
#[test]
fn pytest_conftest_and_test_modules_are_roots() {
    let dir = tree(&[
        (
            "pyproject.toml",
            "[project]\nname = \"svc\"\n\n[tool.pytest.ini_options]\ntestpaths = [\"tests\"]\n",
        ),
        ("conftest.py", "import sys\n"),
        ("tests/conftest.py", "import pytest\n"),
        ("tests/test_billing.py", "def test_x():\n    pass\n"),
        ("tests/billing_test.py", "def test_y():\n    pass\n"),
        ("tests/helpers.py", "def helper():\n    pass\n"),
    ]);
    let scan = scan(dir.path()).expect("scan");

    assert_eq!(
        root_paths(&scan),
        vec![
            "conftest.py".to_string(),
            "tests/billing_test.py".to_string(),
            "tests/conftest.py".to_string(),
            "tests/test_billing.py".to_string(),
        ]
    );
    assert_eq!(root_at(&scan, "conftest.py").rule(), Rule::PytestConftest);
    assert_eq!(
        root_at(&scan, "tests/test_billing.py").rule(),
        Rule::PytestTestModule
    );
    assert_eq!(
        root_at(&scan, "conftest.py").detection().evidence().kind(),
        EvidenceKind::ConfigSection
    );
}

/// Next.js app router. Both `app/` and `src/app/` are accepted because knip's
/// own plugin branches on exactly that difference between majors (§11 R2) and
/// the declared range here — `^15.0.0` — does not tell us which layout a given
/// build resolved.
#[test]
fn next_app_router_files_are_roots_under_both_app_and_src_app() {
    let dir = tree(&[
        (
            "package.json",
            "{\"name\":\"web\",\"dependencies\":{\"next\":\"^15.0.0\",\"react\":\"^19.0.0\"}}\n",
        ),
        ("app/page.tsx", "export default function Page() {}\n"),
        (
            "app/dashboard/layout.tsx",
            "export default function L() {}\n",
        ),
        ("app/api/health/route.ts", "export function GET() {}\n"),
        (
            "app/dashboard/loading.tsx",
            "export default function S() {}\n",
        ),
        (
            "src/app/settings/page.jsx",
            "export default function P() {}\n",
        ),
        ("app/dashboard/chart.tsx", "export function Chart() {}\n"),
        ("lib/page.tsx", "export default function Nope() {}\n"),
    ]);
    let scan = scan(dir.path()).expect("scan");

    assert_eq!(
        root_paths(&scan),
        vec![
            "app/api/health/route.ts".to_string(),
            "app/dashboard/layout.tsx".to_string(),
            "app/dashboard/loading.tsx".to_string(),
            "app/page.tsx".to_string(),
            "src/app/settings/page.jsx".to_string(),
        ],
        "a component inside app/ is reached by an import and is not itself a \
         root; a page.tsx outside app/ is not a route at all"
    );
    assert_eq!(
        root_at(&scan, "app/page.tsx").rule(),
        Rule::NextAppRouterPage
    );
    assert_eq!(
        root_at(&scan, "app/api/health/route.ts").rule(),
        Rule::NextAppRouterRoute
    );
    assert_eq!(
        root_at(&scan, "app/dashboard/loading.tsx").rule(),
        Rule::NextAppRouterSpecialFile
    );
    assert_eq!(
        root_at(&scan, "app/page.tsx")
            .detection()
            .declared_version(),
        Some("^15.0.0"),
        "§5.1 makes the version half of Tier B's correctness condition"
    );
}

/// Rails: the router is read by the framework, initializers are auto-run, and
/// jobs are named by serialized payloads rather than by callers.
#[test]
fn rails_routes_initializers_and_jobs_are_roots() {
    let dir = tree(&[
        (
            "Gemfile",
            "source \"https://rubygems.org\"\ngem \"rails\", \"~> 7.1\"\n",
        ),
        (
            "config/routes.rb",
            "Rails.application.routes.draw do\nend\n",
        ),
        ("config/initializers/cors.rb", "# cors\n"),
        ("config/database.yml", "development:\n"),
        (
            "app/jobs/report_job.rb",
            "class ReportJob < ApplicationJob\nend\n",
        ),
        ("app/models/report.rb", "class Report\nend\n"),
    ]);
    let scan = scan(dir.path()).expect("scan");

    assert_eq!(
        root_paths(&scan),
        vec![
            "app/jobs/report_job.rb".to_string(),
            "config/initializers/cors.rb".to_string(),
            "config/routes.rb".to_string(),
        ]
    );
    assert_eq!(
        root_at(&scan, "config/routes.rb").detection().framework(),
        Framework::Rails
    );
    assert!(
        !root_paths(&scan).contains(&"Gemfile".to_string()),
        "the manifest is Tier A's business, not this module's"
    );
}

/// Django's remaining surface: `manage.py` alone is enough evidence (Django
/// generates it), management commands are invoked by name from a shell, and a
/// URLConf is named by a settings string this module does not resolve.
#[test]
fn django_management_commands_and_urlconfs_are_roots() {
    let dir = tree(&[
        ("manage.py", "import django\n"),
        ("billing/settings.py", "INSTALLED_APPS = []\n"),
        ("billing/urls.py", "urlpatterns = []\n"),
        ("reporting/__init__.py", ""),
        ("reporting/management/__init__.py", ""),
        ("reporting/management/commands/__init__.py", ""),
        (
            "reporting/management/commands/rebuild_index.py",
            "class Command:\n    def handle(self):\n        pass\n",
        ),
        ("reporting/helpers.py", "def helper():\n    pass\n"),
    ]);
    let scan = scan(dir.path()).expect("scan");

    assert_eq!(
        root_at(&scan, "reporting/management/commands/rebuild_index.py").rule(),
        Rule::DjangoManagementCommand
    );
    assert_eq!(
        root_at(&scan, "billing/urls.py").rule(),
        Rule::DjangoUrlConf
    );
    assert_eq!(
        root_at(&scan, "billing/urls.py")
            .detection()
            .evidence()
            .kind(),
        EvidenceKind::MarkerFile
    );
    assert!(!root_paths(&scan).contains(&"reporting/helpers.py".to_string()));
    assert!(
        !root_paths(&scan).contains(&"reporting/management/commands/__init__.py".to_string()),
        "the package marker is not a command"
    );
}

// ---------------------------------------------------------------------------
// 5. Provenance survives to the human
// ---------------------------------------------------------------------------

/// §9.13 asks for ProGuard's `-printseeds` by name: the classification is shown
/// to a human *before* anything acts on it. Every line therefore carries the
/// tier, the rule, the framework and the evidence — a root that prints without
/// its provenance is the failure mode roots/mod.rs opens with.
#[test]
fn printseeds_shows_tier_rule_framework_and_evidence_for_every_root() {
    let dir = m10_framework_convention();
    let scan = scan(dir.path()).expect("scan m10");
    let seeds = scan.printseeds();

    for root in scan.roots() {
        let prefix = format!("B {}", root.path().display());
        let line = seeds
            .lines()
            .find(|line| line.starts_with(&prefix))
            .unwrap_or_else(|| panic!("no seed line for {}\n{seeds}", root.path().display()));
        assert!(line.starts_with("B "), "line lacks its tier: {line}");
        assert!(
            line.contains(root.rule().label()),
            "line lacks its rule: {line}"
        );
        assert!(
            line.contains(root.framework().name()),
            "line lacks its framework: {line}"
        );
        assert!(
            line.contains(&*root.detection().evidence().path().to_string_lossy()),
            "line lacks its evidence: {line}"
        );
    }
    assert!(
        seeds.contains("ReportingConfig"),
        "the symbol is the whole reason the root exists:\n{seeds}"
    );
}

/// A known unknown prints too, and is visually distinct from a root. §9.5 reads
/// this to cap the tier; a human reads it to know what the tool did not know.
#[test]
fn printseeds_shows_known_unknowns_distinctly() {
    let dir = tree(&[(
        "pyproject.toml",
        "[project]\nname = \"billing\"\ndependencies = [\"celery>=5.3\"]\n",
    )]);
    let scan = scan(dir.path()).expect("scan");
    let seeds = scan.printseeds();

    let line = seeds
        .lines()
        .find(|line| line.starts_with("? "))
        .unwrap_or_else(|| panic!("no known-unknown line:\n{seeds}"));
    assert!(line.contains("celery"), "{line}");
    assert!(line.contains("no plugin"), "{line}");
}

// ---------------------------------------------------------------------------
// Fail loudly
// ---------------------------------------------------------------------------

/// §6.20's rule again, at the outermost boundary: a walk that could not run has
/// found nothing *because it did not look*. Returning an empty root set here
/// would tell a caller that a repository has no entry points, which is the most
/// dangerous sentence this module could utter.
#[test]
fn a_walk_that_cannot_run_is_an_error_not_an_empty_root_set() {
    let dir = TempDir::new().expect("tempdir");
    let missing = dir.path().join("no-such-directory");

    let error = scan(&missing).expect_err("scanning a missing directory must fail");
    let rendered = error.to_string();
    assert!(
        rendered.contains("no-such-directory"),
        "the failure must name the path it failed on: {rendered}"
    );
}
