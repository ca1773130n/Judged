//! Gate 2b and 2c (§9.3) — the reachability veto, tested against the shapes the
//! E2 suite already uses.
//!
//! Two mutant classes are the reason this module exists, so they are the tests:
//!
//! - **m03** discovers plugins by scanning a directory. The plugin's name is
//!   spelled nowhere — the loader interpolates a stem it read off the
//!   filesystem — so 2a's literal veto has nothing to match on. Only 2c can
//!   rescue it, and only by rooting the *whole directory* the scan reaches
//!   (§6.12: "treat the entire matched directory as rooted").
//! - **m08** names a release script exactly once, in a `run:` body, and a uWSGI
//!   config exactly once, in a Dockerfile `COPY` source. That is 2b.
//!
//! Every test states which direction its failure costs. A veto that fires too
//! often costs recall; one that fires too rarely costs an incident (§1.3), and
//! the malformed-manifest tests are the ones that hold that line: a manifest we
//! could not read has told us *nothing* about what it names, and reading that
//! as "names nothing" is §6.20 in miniature.

use std::path::{Path, PathBuf};

use judged_core::veto::reachability::{Reachability, Verdict, VetoReason};
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// The two E2 shapes, transcribed from the fixtures.
// ---------------------------------------------------------------------------

/// m03 — `pluginhost/loader.py` lists `plugins/*.py` at startup and imports
/// every module it finds. `docs/notes.md` is added here, outside every
/// enumerated directory, so the suite can prove the veto is not a constant.
const M03: &[(&str, &str)] = &[
    (
        "pyproject.toml",
        "[project]\nname = \"pluginhost\"\nversion = \"0.1.0\"\nrequires-python = \">=3.11\"\n",
    ),
    ("pluginhost/__init__.py", ""),
    (
        "pluginhost/loader.py",
        r#"# Import every module that happens to be sitting in plugins/.
import importlib
from pathlib import Path

PLUGIN_DIR = Path(__file__).with_name("plugins")


def load_all():
    for path in sorted(PLUGIN_DIR.glob("*.py")):
        if path.stem.startswith("_"):
            continue
        yield importlib.import_module(f"{__package__}.plugins.{path.stem}")
"#,
    ),
    ("pluginhost/plugins/__init__.py", ""),
    (
        "pluginhost/plugins/tsvwriter.py",
        r#"# Registered purely by being a *.py file inside plugins/.

EXTENSION = ".tsv"


def emit(rows):
    return "\t".join(str(cell) for cell in rows)
"#,
    ),
    (
        "pluginhost/main.py",
        "from .loader import load_all\n\n\ndef main():\n    for module in load_all():\n        print(module.__name__)\n",
    ),
    (
        "pluginhost/textwrap_helper.py",
        "# Left behind when the report renderer moved to Jinja.\n\n\ndef hang_indent(text, width=72):\n    return text\n",
    ),
    (
        "docs/notes.md",
        "# Notes\n\nNothing here enumerates a directory at runtime.\n",
    ),
];

/// m08 — the release script is named once in a CI `run:` body, the uWSGI config
/// once in a Dockerfile `COPY` source, and the two decoys are named nowhere.
const M08: &[(&str, &str)] = &[
    (
        "pyproject.toml",
        r#"[project]
name = "svc"
version = "0.1.0"
requires-python = ">=3.11"

[build-system]
requires = ["setuptools>=68"]
build-backend = "setuptools.build_meta"
"#,
    ),
    ("svc/__init__.py", "\"\"\"The service.\"\"\"\n"),
    (
        "svc/main.py",
        r#""""The whole application, as far as any Python tool can see."""


def handle(request: dict) -> dict:
    return {"ok": True, "path": request.get("path", "/")}
"#,
    ),
    (".github/workflows/ci.yml", CI_WORKFLOW),
    (
        "scripts/verify_release.sh",
        "#!/usr/bin/env bash\nset -euo pipefail\n\ntest -f pyproject.toml\necho \"release artifacts verified\"\n",
    ),
    (
        "Dockerfile",
        r#"FROM python:3.11-slim

WORKDIR /app
COPY pyproject.toml ./
COPY svc/ ./svc/

# The only reference in this repository to the uWSGI config.
COPY deploy/uwsgi.ini /etc/uwsgi/uwsgi.ini

CMD ["uwsgi", "--ini", "/etc/uwsgi/uwsgi.ini"]
"#,
    ),
    (
        "deploy/uwsgi.ini",
        "; LIVE. Copied into the image by the Dockerfile.\n[uwsgi]\nmodule = svc.main\nprocesses = 4\n",
    ),
    (
        "scripts/old_benchmark.sh",
        "#!/usr/bin/env bash\n# DEAD DECOY. No workflow and no Dockerfile names it.\nset -euo pipefail\n",
    ),
    (
        "deploy/unused_nginx.conf",
        "# DEAD DECOY. Sits beside a config that only a COPY line keeps alive.\nserver {\n    listen 8081;\n}\n",
    ),
];

/// m08's workflow, verbatim: the release script appears in the `run:` body and
/// nowhere else in the repository.
const CI_WORKFLOW: &str = r#"name: ci

on:
  push:
    branches: [main]

jobs:
  release:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - name: Verify the release artifacts
        run: bash scripts/verify_release.sh
"#;

// ---------------------------------------------------------------------------
// Harness.
// ---------------------------------------------------------------------------

/// Materialize a tree and scan it. The `TempDir` is returned because dropping
/// it deletes the tree.
fn scan(files: &[(&str, &str)]) -> (TempDir, Reachability) {
    let dir = tempfile::tempdir().expect("temporary directory");
    for (relative, body) in files {
        write_bytes(dir.path(), relative, body.as_bytes());
    }
    let scanned = Reachability::scan(dir.path());
    (dir, scanned)
}

fn write_bytes(root: &Path, relative: &str, bytes: &[u8]) {
    let path = root.join(relative);
    std::fs::create_dir_all(path.parent().expect("a relative path has a parent"))
        .expect("create parent directory");
    std::fs::write(&path, bytes).unwrap_or_else(|error| panic!("write {relative}: {error}"));
}

/// Assert `candidate` is vetoed and hand back the reason, so each test can say
/// *why* it expected the rescue rather than merely that one happened.
#[track_caller]
fn vetoed(scanned: &Reachability, candidate: &str) -> VetoReason {
    match scanned.verdict(Path::new(candidate)) {
        Verdict::Vetoed { reason } => reason,
        Verdict::Clear => panic!(
            "{candidate} was not vetoed; a missed veto is the error direction that \
             costs an incident (§1.3)"
        ),
    }
}

#[track_caller]
fn cleared(scanned: &Reachability, candidate: &str) {
    assert_eq!(
        scanned.verdict(Path::new(candidate)),
        Verdict::Clear,
        "{candidate} was vetoed; this file is reachable by nothing, so the veto \
         is costing recall here"
    );
}

#[track_caller]
fn enumerated(reason: &VetoReason) -> (&str, &Path, &Path) {
    match reason {
        VetoReason::EnumeratedDirectory {
            construct,
            found_in,
            rooted,
        } => (construct.as_str(), found_in.as_path(), rooted.as_path()),
        other => panic!("expected a 2c directory-enumeration veto, got {other:?}"),
    }
}

#[track_caller]
fn manifest(reason: &VetoReason) -> (&Path, &Path) {
    match reason {
        VetoReason::ManifestPath { manifest, rooted } => (manifest.as_path(), rooted.as_path()),
        other => panic!("expected a 2b manifest-path veto, got {other:?}"),
    }
}

#[track_caller]
fn incomplete(reason: &VetoReason) -> (&Path, &str) {
    match reason {
        VetoReason::IncompleteRead { path, detail } => (path.as_path(), detail.as_str()),
        other => panic!("expected an incomplete-read veto, got {other:?}"),
    }
}

/// One table case: a label, the tree to materialize, the file that must be
/// rescued, and a control file in the same tree that must not be.
type Case = (
    &'static str,
    &'static [(&'static str, &'static str)],
    &'static str,
    &'static str,
);

// ---------------------------------------------------------------------------
// 2c — glob reachability.
// ---------------------------------------------------------------------------

/// The whole point of m03: the plugin is reachable *because of where it sits on
/// disk*, and for no other reason. Nothing names it, so only the directory the
/// loader enumerates can rescue it.
#[test]
fn m03_the_plugin_only_a_directory_scan_can_reach_is_vetoed() {
    let (_dir, scanned) = scan(M03);

    let reason = vetoed(&scanned, "pluginhost/plugins/tsvwriter.py");
    let (construct, found_in, rooted) = enumerated(&reason);

    assert!(
        construct.contains("glob"),
        "the loader enumerates with glob(); the veto named {construct:?}"
    );
    assert_eq!(found_in, Path::new("pluginhost/loader.py"));
    assert_eq!(
        rooted,
        Path::new("pluginhost/plugins"),
        "§6.12 roots the entire matched directory, not the files the pattern \
         obviously names"
    );
}

/// Rooting a whole directory rescues the dead file sitting in it too. That is a
/// recall cost, it is the direction §1.3 says to err in, and it is recorded
/// here rather than hidden.
#[test]
fn m03_the_decoy_beside_the_loader_is_vetoed_too_and_that_is_the_recall_cost() {
    let (_dir, scanned) = scan(M03);

    let reason = vetoed(&scanned, "pluginhost/textwrap_helper.py");
    let (_, found_in, rooted) = enumerated(&reason);

    assert_eq!(found_in, Path::new("pluginhost/loader.py"));
    assert_eq!(
        rooted,
        Path::new("pluginhost"),
        "the enumerating file's own directory is rooted because the enumeration \
         target cannot be resolved statically"
    );
}

/// If everything were vetoed the module would be worthless, so prove a
/// directory no construct reaches gets no veto at all.
#[test]
fn m03_a_directory_no_construct_reaches_gets_no_veto() {
    let (_dir, scanned) = scan(M03);
    cleared(&scanned, "docs/notes.md");
}

/// A construct rescues the file it is written in. Deleting a loader is a real
/// hazard, so this is deliberate — and stated, because it is also why a
/// genuinely dead loader will never be reported.
#[test]
fn the_enumerating_file_rescues_itself() {
    let (_dir, scanned) = scan(M03);
    let reason = vetoed(&scanned, "pluginhost/loader.py");
    let (_, found_in, _) = enumerated(&reason);
    assert_eq!(found_in, Path::new("pluginhost/loader.py"));
}

/// The construct vocabulary, one ecosystem at a time. Each case plants the
/// construct, the asset only that construct can reach, and a control file in a
/// directory the construct cannot reach.
#[test]
fn dynamic_enumeration_shapes_root_the_directory_they_reach() {
    let cases: &[Case] = &[
        (
            "go:embed",
            &[
                (
                    "internal/server/assets.go",
                    "package server\n\nimport \"embed\"\n\n//go:embed static/*\nvar files embed.FS\n",
                ),
                ("internal/server/static/index.html", "<h1>hi</h1>\n"),
                ("cmd/tool/main.go", "package main\n\nfunc main() {}\n"),
            ],
            "internal/server/static/index.html",
            "cmd/tool/main.go",
        ),
        (
            "include_str!",
            &[
                (
                    "src/render.rs",
                    "pub const TEMPLATE: &str = include_str!(\"templates/page.html\");\n",
                ),
                ("src/templates/page.html", "<html></html>\n"),
                ("bench/data.txt", "0\n"),
            ],
            "src/templates/page.html",
            "bench/data.txt",
        ),
        (
            "require.context",
            &[
                (
                    "web/src/registry.js",
                    "const modules = require.context('./widgets', true, /\\.vue$/);\nexport default modules;\n",
                ),
                ("web/src/widgets/Chart.vue", "<template></template>\n"),
                ("web/public/robots.txt", "User-agent: *\n"),
            ],
            "web/src/widgets/Chart.vue",
            "web/public/robots.txt",
        ),
        (
            "Dir[",
            &[
                (
                    "config/initializers/load_tasks.rb",
                    "Dir[Rails.root.join('lib/tasks/*.rake')].each { |f| load f }\n",
                ),
                ("config/initializers/audit.rb", "# loaded by the same sweep\n"),
                ("app/models/user.rb", "class User; end\n"),
            ],
            "config/initializers/audit.rb",
            "app/models/user.rb",
        ),
        (
            "os.walk",
            &[
                (
                    "etl/discover.py",
                    "import os\n\nfor root, dirs, files in os.walk('feeds'):\n    print(root)\n",
                ),
                ("etl/feeds/daily.yaml", "kind: feed\n"),
                ("web/index.html", "<html></html>\n"),
            ],
            "etl/feeds/daily.yaml",
            "web/index.html",
        ),
        (
            "importlib.resources",
            &[
                (
                    "app/schema.py",
                    "from importlib.resources import files\n\nDATA = files('app.schemas')\n",
                ),
                ("app/schemas/order.json", "{}\n"),
                ("tools/report.py", "print('report')\n"),
            ],
            "app/schemas/order.json",
            "tools/report.py",
        ),
        (
            "Bundle.module",
            &[
                (
                    "Sources/Feature/Theme.swift",
                    "import Foundation\n\nlet bundle = Bundle.module\n",
                ),
                ("Sources/Feature/Resources/theme.json", "{}\n"),
                ("Tests/FeatureTests/fixture.json", "{}\n"),
            ],
            "Sources/Feature/Resources/theme.json",
            "Tests/FeatureTests/fixture.json",
        ),
    ];

    for (label, files, rescued, control) in cases {
        let (_dir, scanned) = scan(files);
        let reason = vetoed(&scanned, rescued);
        let (construct, _, _) = enumerated(&reason);
        assert!(
            !construct.is_empty(),
            "{label}: the veto must name the construct it fired on"
        );
        cleared(&scanned, control);
    }
}

/// A construct in a repository-root file cannot be bounded to any subdirectory:
/// the enumeration may reach anything. The sound reading roots the repository,
/// and the cost of that is total for this repository — so it is asserted, not
/// discovered later.
#[test]
fn a_construct_in_a_root_level_file_roots_the_whole_repository() {
    let (_dir, scanned) = scan(&[
        (
            "build.py",
            "from glob import glob\n\nSOURCES = glob('*/*.c')\n",
        ),
        ("docs/design.md", "# Design\n"),
        ("src/main.c", "int main(void) { return 0; }\n"),
    ]);

    let reason = vetoed(&scanned, "docs/design.md");
    let (_, found_in, rooted) = enumerated(&reason);
    assert_eq!(found_in, Path::new("build.py"));
    assert_eq!(
        rooted,
        Path::new(""),
        "the repository root itself is rooted"
    );
}

// ---------------------------------------------------------------------------
// 2b — manifest paths.
// ---------------------------------------------------------------------------

/// m08's uWSGI config: named once, by a `COPY` source path, in a language no
/// Python analyzer reads. The process that opens it does not exist until the
/// image runs.
#[test]
fn m08_the_uwsgi_config_named_only_by_a_dockerfile_copy_is_vetoed() {
    let (_dir, scanned) = scan(M08);

    let reason = vetoed(&scanned, "deploy/uwsgi.ini");
    let (named_by, rooted) = manifest(&reason);

    assert_eq!(named_by, Path::new("Dockerfile"));
    assert_eq!(rooted, Path::new("deploy/uwsgi.ini"));
}

/// A `COPY` of a directory roots the directory, so every file under it is
/// rescued — the same "root the directory" rule 2c uses.
#[test]
fn m08_a_dockerfile_copy_of_a_directory_roots_everything_under_it() {
    let (_dir, scanned) = scan(M08);

    let reason = vetoed(&scanned, "svc/main.py");
    let (named_by, rooted) = manifest(&reason);

    assert_eq!(named_by, Path::new("Dockerfile"));
    assert_eq!(rooted, Path::new("svc"));
}

/// m08's release script: named once, in a `run:` body. §5.2 lists exactly this
/// line under CI roots.
#[test]
fn m08_the_release_script_named_only_in_a_ci_run_body_is_vetoed() {
    let (_dir, scanned) = scan(M08);

    let reason = vetoed(&scanned, "scripts/verify_release.sh");
    let (named_by, rooted) = manifest(&reason);

    assert_eq!(named_by, Path::new(".github/workflows/ci.yml"));
    assert_eq!(rooted, Path::new("scripts/verify_release.sh"));
}

/// The discrimination the E2 suite is actually measuring: the decoy beside a
/// rescued file must not be rescued with it. `scripts/` is not rooted just
/// because one script inside it is named.
#[test]
fn m08_both_decoys_get_no_veto() {
    let (_dir, scanned) = scan(M08);
    cleared(&scanned, "scripts/old_benchmark.sh");
    cleared(&scanned, "deploy/unused_nginx.conf");
}

/// A workflow that writes `with:` as a flow mapping says exactly what the
/// three-line spelling says, and GitHub reads the two identically. A reading
/// that only understands `key:` at the start of a line sees the whole brace as
/// one opaque value of `with`, never looks inside it, and reports the artifact
/// as reachable by nothing — a miss, which is the direction that costs an
/// incident (§1.3). This is the defect that rejected five of the nine
/// out-of-sample repositories the first time a YAML subset was hand-written.
#[test]
fn a_flow_mapping_names_a_path_exactly_as_the_block_spelling_does() {
    let (_dir, scanned) = scan(&[
        (
            ".github/workflows/report.yml",
            "jobs:\n  r:\n    steps:\n      - {uses: actions/upload-artifact@v4, \
             with: {name: coverage, path: reports/coverage.xml}}\n",
        ),
        ("reports/coverage.xml", "<coverage/>\n"),
        ("reports-old/coverage.xml", "<coverage/>\n"),
    ]);

    let reason = vetoed(&scanned, "reports/coverage.xml");
    let (named_by, rooted) = manifest(&reason);
    assert_eq!(named_by, Path::new(".github/workflows/report.yml"));
    assert_eq!(rooted, Path::new("reports/coverage.xml"));

    cleared(&scanned, "reports-old/coverage.xml");
}

/// A trailing comment is not part of the value. Reading it as one does not
/// merely add noise: `dist/` and `dist/  # the tarball` are different strings,
/// so the directory that was actually named is never rooted and the artifact
/// under it is reported as dead.
#[test]
fn a_trailing_comment_is_not_part_of_the_path_it_follows() {
    let (_dir, scanned) = scan(&[
        (
            ".gitlab-ci.yml",
            "build:\n  script:\n    - make\n  artifacts:\n    paths:\n      \
             - dist/  # everything the release job uploads\n",
        ),
        ("dist/app.tar", "tar\n"),
        ("tmp/app.tar", "tar\n"),
    ]);

    let reason = vetoed(&scanned, "dist/app.tar");
    let (named_by, rooted) = manifest(&reason);
    assert_eq!(named_by, Path::new(".gitlab-ci.yml"));
    assert_eq!(rooted, Path::new("dist"));

    cleared(&scanned, "tmp/app.tar");
}

/// A comment *line* inside a `paths:` block is not a path either. It rescues
/// nothing — no candidate is ever named `# the build output` — but it lands in
/// the evidence a human has to audit, and §6.20's whole complaint is that a
/// veto layer nobody reads is a veto layer nobody trusts.
#[test]
fn a_comment_line_inside_a_paths_block_is_not_rooted_as_a_directory() {
    let (_dir, scanned) = scan(&[
        (
            ".gitlab-ci.yml",
            "build:\n  artifacts:\n    paths:\n      # the build output, uploaded by \
             the release job\n      - dist/\n",
        ),
        ("dist/app.tar", "tar\n"),
    ]);

    let reason = vetoed(&scanned, "dist/app.tar");
    let (_, rooted) = manifest(&reason);
    assert_eq!(rooted, Path::new("dist"));

    let junk: Vec<String> = scanned
        .roots()
        .map(|(path, _)| path.display().to_string())
        .filter(|path| path.contains('#') || path.contains("build output"))
        .collect();
    assert!(
        junk.is_empty(),
        "a comment was rooted as though it were a directory: {junk:?}"
    );
}

/// A `path:` block scalar is the shape every cache step in the wild uses, and
/// most of what it lists is not a repository path at all. Measured against this
/// repository, the naive reading rooted `|` and `~/.cargo/registry` alongside
/// `target/`. Those rescue nothing — no candidate is ever named `|` — but they
/// land in the evidence a human has to audit, and a veto layer nobody reads is
/// a veto layer nobody trusts.
#[test]
fn a_cache_path_block_roots_the_repository_directory_and_no_junk() {
    let (_dir, scanned) = scan(&[
        (
            ".github/workflows/ci.yml",
            "jobs:\n  build:\n    steps:\n      - uses: actions/cache@v4\n        with:\n          \
             path: |\n            ~/.cargo/registry\n            $RUNNER_TEMP/cache\n            \
             target/\n",
        ),
        ("target/debug/app", "binary\n"),
        ("notes/scratch.md", "# scratch\n"),
    ]);

    let reason = vetoed(&scanned, "target/debug/app");
    let (named_by, rooted) = manifest(&reason);
    assert_eq!(named_by, Path::new(".github/workflows/ci.yml"));
    assert_eq!(rooted, Path::new("target"));

    cleared(&scanned, "notes/scratch.md");

    let junk: Vec<String> = scanned
        .roots()
        .map(|(path, _)| path.display().to_string())
        .filter(|path| path == "|" || path.starts_with('~') || path.starts_with('$'))
        .collect();
    assert!(
        junk.is_empty(),
        "the block indicator and paths outside the repository were rooted as \
         though they were files: {junk:?}"
    );
}

/// The rest of the 2b vocabulary. Each case is a manifest, the file it names,
/// and a control the manifest does not name.
#[test]
fn manifest_shapes_root_the_paths_they_name() {
    let cases: &[Case] = &[
        (
            "package.json#files",
            &[
                (
                    "package.json",
                    "{\n  \"name\": \"pkg\",\n  \"files\": [\"dist\", \"bin/cli.js\"]\n}\n",
                ),
                ("dist/index.js", "export default 1;\n"),
                ("src/index.ts", "export default 1;\n"),
            ],
            "dist/index.js",
            "src/index.ts",
        ),
        (
            ".dockerignore negation",
            &[
                (
                    ".dockerignore",
                    "*\n!config/prod.env\n!svc/\n",
                ),
                ("config/prod.env", "MODE=prod\n"),
                ("config/dev.env", "MODE=dev\n"),
            ],
            "config/prod.env",
            "config/dev.env",
        ),
        (
            "MANIFEST.in graft",
            &[
                ("MANIFEST.in", "graft locale\ninclude README.rst\n"),
                ("locale/de/LC_MESSAGES/app.po", "msgid \"\"\n"),
                ("scratch/old.po", "msgid \"\"\n"),
            ],
            "locale/de/LC_MESSAGES/app.po",
            "scratch/old.po",
        ),
        (
            "pyproject include",
            &[
                (
                    "pyproject.toml",
                    "[tool.poetry]\nname = \"app\"\ninclude = [\"app/templates/*.html\"]\n",
                ),
                ("app/templates/base.html", "<html></html>\n"),
                ("app/static/old.css", "body {}\n"),
            ],
            "app/templates/base.html",
            "app/static/old.css",
        ),
        (
            ".gitattributes lfs directory pattern",
            &[
                (
                    ".gitattributes",
                    "models/*.pt filter=lfs diff=lfs merge=lfs -text\n",
                ),
                ("models/encoder.pt", "weights\n"),
                ("notes/encoder.md", "# encoder\n"),
            ],
            "models/encoder.pt",
            "notes/encoder.md",
        ),
        (
            ".gitattributes lfs suffix pattern",
            &[
                (".gitattributes", "*.onnx filter=lfs diff=lfs merge=lfs -text\n"),
                ("weights/big.onnx", "graph\n"),
                ("weights/README.md", "# weights\n"),
            ],
            "weights/big.onnx",
            "weights/README.md",
        ),
        (
            "gitlab artifacts.paths",
            &[
                (
                    ".gitlab-ci.yml",
                    "build:\n  script:\n    - make\n  artifacts:\n    paths:\n      - dist/\n",
                ),
                ("dist/app.tar", "tar\n"),
                ("tmp/app.tar", "tar\n"),
            ],
            "dist/app.tar",
            "tmp/app.tar",
        ),
        (
            "upload-artifact with: path",
            &[
                (
                    ".github/workflows/report.yml",
                    "jobs:\n  r:\n    steps:\n      - uses: actions/upload-artifact@v4\n        with:\n          name: coverage\n          path: reports/coverage.xml\n",
                ),
                ("reports/coverage.xml", "<coverage/>\n"),
                ("reports-old/coverage.xml", "<coverage/>\n"),
            ],
            "reports/coverage.xml",
            "reports-old/coverage.xml",
        ),
    ];

    for (label, files, rescued, control) in cases {
        let (_dir, scanned) = scan(files);
        let reason = vetoed(&scanned, rescued);
        let (named_by, _) = manifest(&reason);
        assert!(
            !named_by.as_os_str().is_empty(),
            "{label}: the veto must name the manifest it read"
        );
        cleared(&scanned, control);
    }
}

// ---------------------------------------------------------------------------
// Soundness: an incomplete read is a hit, never an absence (§6.20).
// ---------------------------------------------------------------------------

/// A workflow that does not load has told us nothing about what it names.
/// Treating that as "names nothing" is the inversion that converts the safety
/// net into the deletion trigger, so it vetoes every candidate — including the
/// decoys, which is what makes the assertion sharp.
#[test]
fn a_workflow_that_does_not_parse_vetoes_every_candidate() {
    let malformed: &[(&str, &str)] = &[
        (
            "a tab in the indentation",
            "jobs:\n  release:\n\truns-on: ubuntu-latest\n",
        ),
        (
            "an unterminated quoted scalar",
            "jobs:\n  release:\n    name: \"verify the release\n    runs-on: ubuntu-latest\n",
        ),
        (
            "an unclosed flow sequence",
            "on:\n  push:\n    branches: [main\n",
        ),
        // The three above are the defects a hand-written structural check can
        // see. These are the ones only a parser can: each is a real YAML error,
        // and each was silently readable as "this file names nothing".
        (
            "a mapping value where none may appear",
            "jobs:\n  release:\n    runs-on: ubuntu-latest: extra\n",
        ),
        (
            "a dedent onto a column no line opened",
            "jobs:\n  release:\n      name: verify\n    runs-on: ubuntu-latest\n",
        ),
        (
            "a flow sequence closed by the wrong bracket",
            "on:\n  push:\n    branches: [main}\n",
        ),
    ];

    for (label, body) in malformed {
        let mut files: Vec<(&str, &str)> = M08.to_vec();
        for entry in &mut files {
            if entry.0 == ".github/workflows/ci.yml" {
                entry.1 = body;
            }
        }
        let (_dir, scanned) = scan(&files);

        for candidate in [
            "scripts/old_benchmark.sh",
            "deploy/unused_nginx.conf",
            "svc/main.py",
        ] {
            let reason = vetoed(&scanned, candidate);
            let (path, detail) = incomplete(&reason);
            assert_eq!(
                path,
                Path::new(".github/workflows/ci.yml"),
                "{label}: the veto must name the manifest it could not read"
            );
            assert!(
                !detail.is_empty(),
                "{label}: the veto must say what went wrong"
            );
        }
    }
}

/// The same rule for bytes we cannot decode: a manifest that is not UTF-8 has
/// not told us it names nothing.
#[test]
fn a_workflow_that_is_not_valid_utf8_vetoes_every_candidate() {
    let dir = tempfile::tempdir().expect("temporary directory");
    for (relative, body) in M08 {
        write_bytes(dir.path(), relative, body.as_bytes());
    }
    write_bytes(
        dir.path(),
        ".github/workflows/ci.yml",
        &[b'r', b'u', b'n', b':', b' ', 0xff, 0xfe, b'\n'],
    );

    let scanned = Reachability::scan(dir.path());
    let reason = vetoed(&scanned, "scripts/old_benchmark.sh");
    let (path, _) = incomplete(&reason);
    assert_eq!(path, Path::new(".github/workflows/ci.yml"));
}

/// …and the necessary counterweight: a *binary* is not a manifest. If every
/// PNG in a repository blanket-vetoed it, the gate would be a no-op dressed up
/// as a safety mechanism.
#[test]
fn a_binary_file_that_is_not_valid_utf8_vetoes_nothing() {
    let dir = tempfile::tempdir().expect("temporary directory");
    for (relative, body) in M08 {
        write_bytes(dir.path(), relative, body.as_bytes());
    }
    write_bytes(
        dir.path(),
        "assets/logo.png",
        &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0xff, 0xd8],
    );

    let scanned = Reachability::scan(dir.path());
    assert!(
        scanned.incomplete().is_empty(),
        "a binary is scanned as bytes, completely; it is not an incomplete read"
    );
    cleared(&scanned, "scripts/old_benchmark.sh");
}

/// The one recall cost this module's YAML reading carries, pinned here so it
/// cannot be lost.
///
/// A multi-line flow sequence whose closing `]` sits at or before the column of
/// the key that opened it is accepted by PyYAML and by ruamel, and is what
/// pip's `.pre-commit-config.yaml` and the OpenTelemetry demo's `compose.yaml`
/// both write. YAML 1.2 rule 137 requires flow content in a block context to be
/// indented past its parent, and `saphyr-parser` enforces that, so both files
/// are refused. Measured over the nine-repository out-of-sample corpus that is
/// two files, and it takes those two repositories with it: an incomplete read
/// vetoes everything.
///
/// It stays a veto anyway. The alternative is to guess at what an unreadable
/// manifest names, which is §6.20's inversion and the reason this module exists
/// — and the veto is at least loud, naming the file and the line, where a guess
/// would be silent. If the parser is ever bumped past this strictness, this
/// test fails, and that failure is the prompt to delete it and take the recall
/// back.
#[test]
fn a_flow_sequence_closing_at_its_parents_column_is_refused_and_that_is_the_documented_cost() {
    let (_dir, scanned) = scan(&[
        (
            ".pre-commit-config.yaml",
            "repos:\n- repo: local\n  hooks:\n  - id: mypy\n    \
             additional_dependencies: [\n      'nox==2024.03.02',\n    ]\n",
        ),
        ("noxfile.py", "import nox\n"),
    ]);

    let reason = vetoed(&scanned, "noxfile.py");
    let (path, detail) = incomplete(&reason);
    assert_eq!(path, Path::new(".pre-commit-config.yaml"));
    assert!(
        detail.contains("line 7"),
        "a refusal a human cannot locate is a refusal they will read as a bug; \
         the detail must name the line, got {detail:?}"
    );
}

/// An anchor's content is written once and referenced by an alias, and this
/// module does not resolve them. Where an alias stands somewhere the module
/// would have ignored anyway, nothing is lost — the anchor's own definition is
/// visited in its own right. Where it stands as the value of `paths:` it is
/// §6.20 exactly: the value is real, we did not read it, and reporting the
/// artifact underneath as reachable by nothing would be a search that did not
/// look, dressed up as a search that found nothing.
#[test]
fn an_alias_where_a_path_belongs_is_an_incomplete_read_not_an_empty_one() {
    let (_dir, scanned) = scan(&[
        (
            ".gitlab-ci.yml",
            ".artifact_paths: &artifact_paths\n  - dist/\n\nbuild:\n  script:\n    - make\n  \
             artifacts:\n    paths: *artifact_paths\n",
        ),
        ("dist/app.tar", "tar\n"),
    ]);

    let reason = vetoed(&scanned, "dist/app.tar");
    let (path, detail) = incomplete(&reason);
    assert_eq!(path, Path::new(".gitlab-ci.yml"));
    assert!(
        detail.contains("alias"),
        "the veto must say what it could not resolve, got {detail:?}"
    );
}

/// `package.json` is parsed as JSON rather than scraped, so a syntax error is a
/// read that did not complete.
#[test]
fn a_package_json_that_does_not_parse_vetoes_every_candidate() {
    let (_dir, scanned) = scan(&[
        ("package.json", "{ \"files\": [\"dist\", }\n"),
        ("src/index.ts", "export default 1;\n"),
    ]);

    let reason = vetoed(&scanned, "src/index.ts");
    let (path, detail) = incomplete(&reason);
    assert_eq!(path, Path::new("package.json"));
    assert!(
        detail.contains("JSON") || detail.contains("json"),
        "the detail should say what failed to parse, got {detail:?}"
    );
}

/// A candidate the scan never covered is an absence of *looking*, not an
/// absence of references.
#[test]
fn a_candidate_outside_the_scanned_tree_is_a_veto_not_an_absence() {
    let (_dir, scanned) = scan(M08);

    let reason = vetoed(&scanned, "../elsewhere/config.ini");
    let (path, _) = incomplete(&reason);
    assert_eq!(path, Path::new("../elsewhere/config.ini"));
}

// ---------------------------------------------------------------------------
// The shape of the layer itself.
// ---------------------------------------------------------------------------

/// A veto is absorbing: more evidence can only add vetoes, never withdraw one.
/// Adding a directory-walking tool to m08 must not un-rescue anything already
/// rescued by a manifest.
#[test]
fn a_veto_is_absorbing_more_evidence_never_takes_one_away() {
    let candidates = [
        "deploy/uwsgi.ini",
        "scripts/verify_release.sh",
        "svc/main.py",
        "scripts/old_benchmark.sh",
        "deploy/unused_nginx.conf",
    ];

    let dir = tempfile::tempdir().expect("temporary directory");
    for (relative, body) in M08 {
        write_bytes(dir.path(), relative, body.as_bytes());
    }
    let before = Reachability::scan(dir.path());
    let vetoed_before: Vec<&str> = candidates
        .iter()
        .copied()
        .filter(|candidate| before.verdict(Path::new(candidate)).is_veto())
        .collect();
    assert!(
        !vetoed_before.is_empty() && vetoed_before.len() < candidates.len(),
        "the fixture must start with some rescued and some not, got {vetoed_before:?}"
    );

    write_bytes(
        dir.path(),
        "tools/collect.py",
        b"import os\n\nfor root, _, files in os.walk('deploy'):\n    print(root)\n",
    );
    let after = Reachability::scan(dir.path());

    for candidate in vetoed_before {
        assert!(
            after.verdict(Path::new(candidate)).is_veto(),
            "{candidate} lost its veto when evidence was added; a veto is absorbing"
        );
    }
}

/// The layer has no way to say "dead". `Clear` is the only non-veto
/// verdict and it carries no claim, which is what keeps Gate 2 unable to
/// nominate a candidate (§9.1).
#[test]
fn a_clear_verdict_carries_no_claim_and_is_the_only_non_veto_verdict() {
    let dir = tempfile::tempdir().expect("temporary directory");
    let scanned = Reachability::scan(dir.path());

    let verdict = scanned.verdict(Path::new("anything.rs"));
    assert_eq!(verdict, Verdict::Clear);
    assert!(!verdict.is_veto());
    assert!(verdict.reason().is_none());

    let roots: Vec<PathBuf> = scanned
        .roots()
        .map(|(path, _reason)| path.to_path_buf())
        .collect();
    assert!(
        roots.is_empty(),
        "an empty repository roots nothing, got {roots:?}"
    );
}
