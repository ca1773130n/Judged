//! Tier A root materialization (§5.1, §5.2).
//!
//! Every test here asks one of two questions. *Did the parser find the root the
//! manifest declared, and can it say exactly which key declared it?* (§9.13
//! wants `-printseeds` output a human can audit: "package.json declares this" is
//! not auditable, "package.json#exports./client" is.) And: *does a manifest we
//! could not read fail loudly?* A malformed manifest has told us nothing about
//! that project's roots, so reporting "declares no roots" would make every entry
//! point in it a deletion candidate — §6.20 in a new costume.

use judged_core::roots::manifest::{
    parse_cargo_toml, parse_dockerfile, parse_github_workflow, parse_go_mod, parse_go_source,
    parse_package_json, parse_pyproject_toml, parse_setup_cfg, scan, Declaration, ManifestRoots,
    RootKind, RootTarget, Tier,
};
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

/// The single root whose origin key is `key`, or a panic naming every key that
/// *was* found — a bare `unwrap` on a missing root tells you nothing about why.
#[track_caller]
fn root_at<'a>(roots: &'a ManifestRoots, key: &str) -> &'a judged_core::roots::manifest::Root {
    let matches: Vec<_> = roots
        .roots()
        .iter()
        .filter(|r| r.origin().key() == key)
        .collect();
    match matches.len() {
        1 => matches[0],
        0 => panic!(
            "no root at key {key:?}; found keys: {:?}",
            roots
                .roots()
                .iter()
                .map(|r| r.origin().key())
                .collect::<Vec<_>>()
        ),
        n => panic!("{n} roots at key {key:?}, expected exactly one"),
    }
}

#[track_caller]
fn assert_path(roots: &ManifestRoots, key: &str, expected: &str) {
    let root = root_at(roots, key);
    assert_eq!(
        root.target(),
        &RootTarget::Path(PathBuf::from(expected)),
        "root {key:?} should target path {expected:?}"
    );
}

fn keys(roots: &ManifestRoots) -> Vec<&str> {
    roots.roots().iter().map(|r| r.origin().key()).collect()
}

/// A package.json exercising every key §5.2 lists for JS/TS.
const FULL_PACKAGE_JSON: &str = r##"{
  "name": "@acme/widget",
  "version": "1.0.0",
  "main": "./dist/index.cjs",
  "module": "./dist/index.mjs",
  "browser": { "./dist/node.js": "./dist/browser.js", "fs": false },
  "types": "./dist/index.d.ts",
  "bin": { "widget": "./bin/cli.js", "widget-dev": "./bin/dev.js" },
  "exports": {
    ".": { "import": "./dist/index.mjs", "require": "./dist/index.cjs" },
    "./client": { "browser": "./dist/client.browser.js", "default": "./dist/client.js" },
    "./legacy": null,
    "./polyfill": ["./dist/polyfill.mjs", "./dist/polyfill.cjs"],
    "./package.json": "./package.json"
  },
  "imports": { "#internal/log": "./src/log.js" },
  "workspaces": ["packages/*"],
  "files": ["dist", "README.md"],
  "scripts": { "build": "tsc -p .", "postinstall": "node ./scripts/postinstall.js" },
  "sideEffects": false
}"##;

// ---------------------------------------------------------------------------
// package.json
// ---------------------------------------------------------------------------

#[test]
fn package_json_records_main_module_browser_and_types() {
    let roots = parse_package_json(Path::new("package.json"), FULL_PACKAGE_JSON).unwrap();

    assert_path(&roots, "main", "dist/index.cjs");
    assert_path(&roots, "module", "dist/index.mjs");
    assert_path(&roots, "types", "dist/index.d.ts");
    // A `browser` map replaces one module with another. Both sides are real
    // files: the key is the module being replaced, the value the replacement.
    assert_path(&roots, "browser./dist/node.js", "dist/browser.js");

    // `"fs": false` is a request to stub a module out, not a path. It must not
    // appear as a root pointing at a file named "false".
    assert!(
        !keys(&roots).contains(&"browser.fs"),
        "the `fs: false` browser stub is not a root; keys were {:?}",
        keys(&roots)
    );

    assert_eq!(root_at(&roots, "main").kind(), RootKind::LibraryEntry);
    assert_eq!(root_at(&roots, "main").tier(), Tier::A);
}

#[test]
fn package_json_records_every_bin_entry_under_its_own_name() {
    let roots = parse_package_json(Path::new("package.json"), FULL_PACKAGE_JSON).unwrap();

    assert_path(&roots, "bin.widget", "bin/cli.js");
    assert_path(&roots, "bin.widget-dev", "bin/dev.js");
    assert_eq!(root_at(&roots, "bin.widget").kind(), RootKind::Executable);
}

#[test]
fn package_json_bin_may_be_a_bare_string() {
    let roots = parse_package_json(
        Path::new("package.json"),
        r#"{"name": "w", "bin": "./cli.js"}"#,
    )
    .unwrap();

    assert_path(&roots, "bin", "cli.js");
    assert_eq!(root_at(&roots, "bin").kind(), RootKind::Executable);
}

#[test]
fn package_json_records_every_leaf_of_the_nested_conditional_exports_map() {
    let roots = parse_package_json(Path::new("package.json"), FULL_PACKAGE_JSON).unwrap();

    // §9.13: the key must locate the leaf, not just the file.
    assert_path(&roots, "exports..import", "dist/index.mjs");
    assert_path(&roots, "exports..require", "dist/index.cjs");
    assert_path(&roots, "exports./client.browser", "dist/client.browser.js");
    assert_path(&roots, "exports./client.default", "dist/client.js");
    assert_path(&roots, "exports./package.json", "package.json");

    // An array leaf is a fallback list; every element is reachable.
    assert_path(&roots, "exports./polyfill[0]", "dist/polyfill.mjs");
    assert_path(&roots, "exports./polyfill[1]", "dist/polyfill.cjs");

    // `null` blocks a subpath. It declares the absence of an export, so it is
    // not a root — and must not become a path.
    assert!(
        !keys(&roots)
            .iter()
            .any(|k| k.starts_with("exports./legacy")),
        "a null exports leaf is not a root; keys were {:?}",
        keys(&roots)
    );
}

#[test]
fn package_json_records_subpath_imports() {
    let roots = parse_package_json(Path::new("package.json"), FULL_PACKAGE_JSON).unwrap();
    assert_path(&roots, "imports.#internal/log", "src/log.js");
}

#[test]
fn package_json_workspaces_and_files_stay_globs() {
    let roots = parse_package_json(Path::new("package.json"), FULL_PACKAGE_JSON).unwrap();

    // "packages/*" is a pattern, not a path. Calling it a path would invite a
    // caller to stat it.
    assert_eq!(
        root_at(&roots, "workspaces[0]").target(),
        &RootTarget::Glob("packages/*".to_string())
    );
    assert_eq!(
        root_at(&roots, "workspaces[0]").kind(),
        RootKind::WorkspaceMember
    );
    assert_eq!(
        root_at(&roots, "files[0]").target(),
        &RootTarget::Glob("dist".to_string())
    );
    assert_eq!(
        root_at(&roots, "files[1]").target(),
        &RootTarget::Glob("README.md".to_string())
    );
    assert_eq!(root_at(&roots, "files[0]").kind(), RootKind::PackagedFile);
}

#[test]
fn package_json_workspaces_may_be_an_object() {
    let roots = parse_package_json(
        Path::new("package.json"),
        r#"{"workspaces": {"packages": ["apps/*"], "nohoist": ["**/x"]}}"#,
    )
    .unwrap();

    assert_eq!(
        root_at(&roots, "workspaces.packages[0]").target(),
        &RootTarget::Glob("apps/*".to_string())
    );
}

#[test]
fn package_json_scripts_are_commands_not_paths() {
    let roots = parse_package_json(Path::new("package.json"), FULL_PACKAGE_JSON).unwrap();

    assert_eq!(
        root_at(&roots, "scripts.build").target(),
        &RootTarget::Command("tsc -p .".to_string())
    );
    assert_eq!(
        root_at(&roots, "scripts.postinstall").target(),
        &RootTarget::Command("node ./scripts/postinstall.js".to_string())
    );
    assert_eq!(root_at(&roots, "scripts.build").kind(), RootKind::Command);
}

#[test]
fn side_effects_false_is_a_declaration_that_modules_may_be_dropped() {
    let roots = parse_package_json(Path::new("package.json"), FULL_PACKAGE_JSON).unwrap();

    // The inverse of a root: it tells a downstream tier that dropping an
    // unreferenced module is sanctioned here. It is not itself a root.
    assert_eq!(
        roots.declarations(),
        &[Declaration::TreeShakable {
            origin: origin("package.json", "sideEffects")
        }]
    );
    assert!(
        !keys(&roots).contains(&"sideEffects"),
        "sideEffects: false declares no root; keys were {:?}",
        keys(&roots)
    );
}

#[test]
fn side_effects_array_declares_both_roots_and_droppability() {
    let roots = parse_package_json(
        Path::new("package.json"),
        r#"{"sideEffects": ["./src/polyfill.js", "*.css"]}"#,
    )
    .unwrap();

    // §5.2: "a module listed in sideEffects is a declared root".
    assert_eq!(
        root_at(&roots, "sideEffects[0]").target(),
        &RootTarget::Glob("src/polyfill.js".to_string())
    );
    assert_eq!(
        root_at(&roots, "sideEffects[1]").target(),
        &RootTarget::Glob("*.css".to_string())
    );
    // And everything *not* listed is declared droppable.
    assert_eq!(
        roots.declarations(),
        &[Declaration::TreeShakableExcept {
            origin: origin("package.json", "sideEffects"),
            globs: vec!["src/polyfill.js".to_string(), "*.css".to_string()],
        }]
    );
}

#[test]
fn side_effects_true_declares_nothing() {
    let roots = parse_package_json(Path::new("package.json"), r#"{"sideEffects": true}"#).unwrap();
    assert!(roots.declarations().is_empty());
    assert!(roots.roots().is_empty());
}

#[test]
fn paths_resolve_against_the_manifests_own_directory() {
    let roots = parse_package_json(
        Path::new("packages/ui/package.json"),
        r#"{"main": "./src/index.js", "bin": {"ui": "bin/ui.js"}, "files": ["dist"]}"#,
    )
    .unwrap();

    assert_path(&roots, "main", "packages/ui/src/index.js");
    assert_path(&roots, "bin.ui", "packages/ui/bin/ui.js");
    assert_eq!(
        root_at(&roots, "files[0]").target(),
        &RootTarget::Glob("packages/ui/dist".to_string())
    );
    assert_eq!(
        root_at(&roots, "main").origin().file(),
        Path::new("packages/ui/package.json")
    );
}

#[test]
fn origin_renders_as_file_hash_key() {
    let roots = parse_package_json(Path::new("package.json"), FULL_PACKAGE_JSON).unwrap();
    assert_eq!(
        root_at(&roots, "exports./client.browser")
            .origin()
            .to_string(),
        "package.json#exports./client.browser"
    );
}

// ---------------------------------------------------------------------------
// soundness: a manifest we could not read is an error, never an empty root list
// ---------------------------------------------------------------------------

#[test]
fn malformed_package_json_is_an_error_not_an_empty_root_list() {
    let err = parse_package_json(Path::new("packages/ui/package.json"), r#"{"main": "./a.js",}"#)
        .expect_err("trailing comma is not JSON; a silent empty root set would make every entry point in this workspace a deletion candidate");

    let rendered = err.to_string();
    assert!(
        rendered.contains("packages/ui/package.json"),
        "the error must name the manifest it failed on, got {rendered:?}"
    );
}

#[test]
fn package_json_that_is_not_an_object_is_an_error() {
    assert!(parse_package_json(Path::new("package.json"), "[]").is_err());
    assert!(parse_package_json(Path::new("package.json"), "null").is_err());
}

#[test]
fn a_root_key_of_the_wrong_type_is_an_error_not_a_skipped_key() {
    // npm would ignore this; we must not, because "main is a number" means we
    // do not know what this package's entry point is.
    let err = parse_package_json(Path::new("package.json"), r#"{"main": 42}"#)
        .expect_err("`main: 42` is not an entry point we can report");
    assert!(
        err.to_string().contains("main"),
        "error should name the key, got {err}"
    );

    assert!(parse_package_json(Path::new("package.json"), r#"{"scripts": ["build"]}"#).is_err());
    assert!(parse_package_json(Path::new("package.json"), r#"{"files": "dist"}"#).is_err());
    assert!(parse_package_json(Path::new("package.json"), r#"{"sideEffects": 0}"#).is_err());
}

#[test]
fn an_empty_package_json_declares_no_roots_and_that_is_not_an_error() {
    // The distinction the whole module turns on: "parsed, and it declares
    // nothing" is a real answer; "could not parse" is not.
    let roots = parse_package_json(Path::new("package.json"), "{}").unwrap();
    assert!(roots.roots().is_empty());
    assert!(roots.declarations().is_empty());
}

// ---------------------------------------------------------------------------
// -printseeds (§9.13)
// ---------------------------------------------------------------------------

#[test]
fn printseeds_shows_tier_kind_origin_and_target_on_one_line_each() {
    let roots = parse_package_json(
        Path::new("package.json"),
        r#"{"name":"w","bin":"./cli.js"}"#,
    )
    .unwrap();
    let seeds = roots.printseeds();
    assert_eq!(seeds, "A\texecutable\tpackage.json#bin\tcli.js\n");
}

fn origin(file: &str, key: &str) -> judged_core::roots::manifest::Origin {
    judged_core::roots::manifest::Origin::new(file, key)
}

// ---------------------------------------------------------------------------
// pyproject.toml (§5.2, Python)
// ---------------------------------------------------------------------------

const FULL_PYPROJECT: &str = r#"
# A comment, and a blank line, before anything.
[build-system]
requires = ["setuptools>=61", "wheel"]

[project]
name = "acme"
version = "0.1.0"
dependencies = [
    "requests",   # trailing comma and a comment inside a multi-line array
    "click",
]

[project.scripts]
acme = "acme.cli:main"

[project.gui-scripts]
acme-gui = "acme.gui:run"

[project.entry-points."flake8.extension"]
ACME = "acme.lint:Plugin"

[project.entry-points.pytest11]
acme_plugin = 'acme.pytest_plugin'
"#;

#[test]
fn pyproject_records_scripts_and_gui_scripts_as_executables() {
    let roots = parse_pyproject_toml(Path::new("pyproject.toml"), FULL_PYPROJECT).unwrap();

    assert_eq!(
        root_at(&roots, "project.scripts.acme").target(),
        &RootTarget::Reference("acme.cli:main".to_string())
    );
    assert_eq!(
        root_at(&roots, "project.scripts.acme").kind(),
        RootKind::Executable
    );
    assert_eq!(
        root_at(&roots, "project.gui-scripts.acme-gui").target(),
        &RootTarget::Reference("acme.gui:run".to_string())
    );
    assert_eq!(
        root_at(&roots, "project.gui-scripts.acme-gui").kind(),
        RootKind::Executable
    );
}

#[test]
fn pyproject_records_every_entry_point_group() {
    let roots = parse_pyproject_toml(Path::new("pyproject.toml"), FULL_PYPROJECT).unwrap();

    // §4.1: a package whose only consumer is an entry-point group is
    // structurally invisible to a dependency checker. The group name has a dot
    // in it, so the key has to quote it or it cannot be read back.
    assert_eq!(
        root_at(&roots, r#"project.entry-points."flake8.extension".ACME"#).target(),
        &RootTarget::Reference("acme.lint:Plugin".to_string())
    );
    assert_eq!(
        root_at(&roots, r#"project.entry-points."flake8.extension".ACME"#).kind(),
        RootKind::PluginEntryPoint
    );
    // A literal string is a string.
    assert_eq!(
        root_at(&roots, "project.entry-points.pytest11.acme_plugin").target(),
        &RootTarget::Reference("acme.pytest_plugin".to_string())
    );
}

#[test]
fn pyproject_declaring_no_entry_points_is_not_an_error() {
    let roots =
        parse_pyproject_toml(Path::new("pyproject.toml"), "[project]\nname = \"acme\"\n").unwrap();
    assert!(roots.is_empty());
}

#[test]
fn malformed_pyproject_is_an_error_not_an_empty_root_list() {
    for (label, bad) in [
        ("unterminated table header", "[project\nname = \"a\"\n"),
        ("missing value", "[project]\nname =\n"),
        ("unquoted garbage value", "[project]\nname = @@@\n"),
        ("unterminated string", "[project]\nname = \"acme\n"),
        ("duplicate key", "[project]\nname = \"a\"\nname = \"b\"\n"),
        (
            "key outside any table",
            "[project.scripts]\nacme = \"a:b\"\n]\n",
        ),
    ] {
        let result = parse_pyproject_toml(Path::new("pyproject.toml"), bad);
        assert!(
            result.is_err(),
            "{label}: expected an error, got {result:?}"
        );
        assert!(
            result.unwrap_err().to_string().contains("pyproject.toml"),
            "{label}: the error must name the manifest"
        );
    }
}

// ---------------------------------------------------------------------------
// Cargo.toml (§5.2, Rust)
// ---------------------------------------------------------------------------

const FULL_CARGO: &str = r#"
[package]
name = "widget"
version = "0.1.0"
edition = "2021"

[lib]
name = "widget"
path = "src/lib.rs"
crate-type = ["cdylib", "rlib"]

[[bin]]
name = "widget"
path = "src/main.rs"

[[bin]]
name = "helper"

[[example]]
name = "demo"
path = "examples/demo.rs"

[[bench]]
name = "throughput"
harness = false

[[test]]
name = "integration"
path = "tests/integration.rs"
"#;

#[test]
fn cargo_records_lib_bin_example_bench_and_test_targets() {
    let roots = parse_cargo_toml(Path::new("Cargo.toml"), FULL_CARGO).unwrap();

    assert_path(&roots, "lib.path", "src/lib.rs");
    assert_eq!(root_at(&roots, "lib.path").kind(), RootKind::LibraryEntry);

    assert_path(&roots, "bin[0].path", "src/main.rs");
    assert_eq!(root_at(&roots, "bin[0].path").kind(), RootKind::Executable);

    assert_path(&roots, "example[0].path", "examples/demo.rs");
    assert_eq!(
        root_at(&roots, "example[0].path").kind(),
        RootKind::DevTarget
    );

    assert_path(&roots, "test[0].path", "tests/integration.rs");
    assert_eq!(root_at(&roots, "test[0].path").kind(), RootKind::DevTarget);
}

#[test]
fn a_cargo_target_with_no_path_records_the_name_cargo_will_resolve() {
    let roots = parse_cargo_toml(Path::new("Cargo.toml"), FULL_CARGO).unwrap();

    // Cargo locates this by target auto-discovery. Inventing `src/bin/helper.rs`
    // here would be a guess dressed as a declaration; the file itself is found
    // by `scan`'s implicit sweep.
    assert_eq!(
        root_at(&roots, "bin[1].name").target(),
        &RootTarget::Reference("helper".to_string())
    );
    assert_eq!(
        root_at(&roots, "bench[0].name").target(),
        &RootTarget::Reference("throughput".to_string())
    );
    assert_eq!(root_at(&roots, "bench[0].name").kind(), RootKind::DevTarget);
}

#[test]
fn a_cdylib_crate_type_records_that_the_consumer_is_outside_the_build_graph() {
    let roots = parse_cargo_toml(Path::new("Cargo.toml"), FULL_CARGO).unwrap();

    // §5.2: cdylib/staticlib means the consumer is outside the crate graph
    // entirely, so "nothing in this workspace calls it" stops being evidence.
    assert_eq!(
        roots.declarations(),
        &[Declaration::ConsumerOutsideBuildGraph {
            origin: origin("Cargo.toml", "lib.crate-type[0]"),
            crate_type: "cdylib".to_string(),
        }]
    );
}

#[test]
fn an_rlib_only_crate_type_declares_nothing_extra() {
    let roots = parse_cargo_toml(
        Path::new("Cargo.toml"),
        "[lib]\npath = \"src/lib.rs\"\ncrate-type = [\"rlib\"]\n",
    )
    .unwrap();
    assert!(roots.declarations().is_empty());
}

#[test]
fn a_staticlib_crate_type_is_recorded_under_the_underscore_spelling_too() {
    let roots = parse_cargo_toml(
        Path::new("ffi/Cargo.toml"),
        "[lib]\ncrate_type = [\"staticlib\"]\n",
    )
    .unwrap();
    assert_eq!(
        roots.declarations(),
        &[Declaration::ConsumerOutsideBuildGraph {
            origin: origin("ffi/Cargo.toml", "lib.crate_type[0]"),
            crate_type: "staticlib".to_string(),
        }]
    );
}

#[test]
fn cargo_paths_resolve_against_the_manifests_directory() {
    let roots = parse_cargo_toml(
        Path::new("crates/judged-core/Cargo.toml"),
        "[lib]\npath = \"src/lib.rs\"\n",
    )
    .unwrap();
    assert_path(&roots, "lib.path", "crates/judged-core/src/lib.rs");
}

#[test]
fn malformed_cargo_toml_is_an_error_not_an_empty_root_list() {
    for (label, bad) in [
        ("unterminated array", "[lib]\ncrate-type = [\"cdylib\"\n"),
        (
            "array of tables header not closed",
            "[[bin]\nname = \"x\"\n",
        ),
        (
            "bare word where a value belongs",
            "[package]\nname = widget\n",
        ),
        ("path of the wrong type", "[lib]\npath = 3\n"),
    ] {
        let result = parse_cargo_toml(Path::new("Cargo.toml"), bad);
        assert!(
            result.is_err(),
            "{label}: expected an error, got {result:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// setup.cfg (§5.2, Python)
// ---------------------------------------------------------------------------

#[test]
fn setup_cfg_records_console_scripts_and_other_entry_point_groups() {
    let cfg = "\
[metadata]
name = acme

[options]
packages = find:

[options.entry_points]
console_scripts =
    acme = acme.cli:main
    acme-admin = acme.admin:main
pytest11 =
    acme = acme.plugin
";
    let roots = parse_setup_cfg(Path::new("setup.cfg"), cfg).unwrap();

    assert_eq!(
        root_at(&roots, "options.entry_points.console_scripts.acme").target(),
        &RootTarget::Reference("acme.cli:main".to_string())
    );
    assert_eq!(
        root_at(&roots, "options.entry_points.console_scripts.acme").kind(),
        RootKind::Executable
    );
    assert_eq!(
        root_at(&roots, "options.entry_points.console_scripts.acme-admin").target(),
        &RootTarget::Reference("acme.admin:main".to_string())
    );
    assert_eq!(
        root_at(&roots, "options.entry_points.pytest11.acme").kind(),
        RootKind::PluginEntryPoint
    );
}

#[test]
fn malformed_setup_cfg_is_an_error_not_an_empty_root_list() {
    for (label, bad) in [
        (
            "a bare line that is neither section nor key",
            "[metadata]\nname = acme\nnonsense\n",
        ),
        ("an unterminated section header", "[metadata\nname = acme\n"),
        ("a value before any section", "name = acme\n"),
    ] {
        let result = parse_setup_cfg(Path::new("setup.cfg"), bad);
        assert!(
            result.is_err(),
            "{label}: expected an error, got {result:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// Go (§5.2)
// ---------------------------------------------------------------------------

#[test]
fn go_mod_records_the_module_path() {
    let go_mod = "\
module github.com/acme/widget

go 1.22

require (
\tgithub.com/x/y v1.0.0 // indirect
)

replace github.com/x/y => ../y
";
    let roots = parse_go_mod(Path::new("go.mod"), go_mod).unwrap();
    assert_eq!(
        root_at(&roots, "module").target(),
        &RootTarget::Reference("github.com/acme/widget".to_string())
    );
    assert_eq!(root_at(&roots, "module").kind(), RootKind::LibraryEntry);
}

#[test]
fn a_go_mod_with_no_module_directive_is_an_error() {
    // Without it we do not know the import path of anything in this tree, which
    // is not the same as "this tree declares no roots".
    assert!(parse_go_mod(Path::new("go.mod"), "go 1.22\n").is_err());
    assert!(parse_go_mod(Path::new("go.mod"), "modul github.com/x\n").is_err());
}

#[test]
fn a_go_file_declaring_package_main_is_an_executable_root() {
    let src = "\
// Command widget does a thing.
/* a block comment
   mentioning package other */
package main

import \"fmt\"

func main() { fmt.Println(\"hi\") }
";
    let roots = parse_go_source(Path::new("cmd/widget/main.go"), src).unwrap();
    assert_path(&roots, "package", "cmd/widget/main.go");
    assert_eq!(root_at(&roots, "package").kind(), RootKind::Executable);
}

#[test]
fn a_go_file_in_a_library_package_declares_no_root() {
    let roots =
        parse_go_source(Path::new("internal/widget/widget.go"), "package widget\n").unwrap();
    assert!(roots.is_empty());
}

#[test]
fn a_go_file_with_no_package_clause_is_an_error() {
    // Every Go file has one. A file without it did not parse.
    assert!(parse_go_source(Path::new("x.go"), "import \"fmt\"\n").is_err());
}

// ---------------------------------------------------------------------------
// Dockerfile (§5.2, containers)
// ---------------------------------------------------------------------------

const FULL_DOCKERFILE: &str = r#"# syntax=docker/dockerfile:1
FROM node:20 AS builder
WORKDIR /app
COPY package.json package-lock.json ./
COPY \
    src ./src
RUN npm ci && npm run build

FROM node:20-slim
COPY --from=builder /app/dist /app/dist
ADD scripts/entrypoint.sh /entrypoint.sh
ENTRYPOINT ["/entrypoint.sh"]
CMD node /app/dist/server.js
"#;

#[test]
fn dockerfile_records_copy_and_add_sources_but_not_their_destination() {
    let roots = parse_dockerfile(Path::new("Dockerfile"), FULL_DOCKERFILE).unwrap();

    // Keyed by line, because a Dockerfile has no other addressable structure
    // and a line number is what a human checks against the file.
    assert_eq!(
        root_at(&roots, "copy@4[0]").target(),
        &RootTarget::Glob("package.json".to_string())
    );
    assert_eq!(
        root_at(&roots, "copy@4[1]").target(),
        &RootTarget::Glob("package-lock.json".to_string())
    );
    assert_eq!(root_at(&roots, "copy@4[0]").kind(), RootKind::PackagedFile);

    // A line continuation is one instruction.
    assert_eq!(
        root_at(&roots, "copy@5[0]").target(),
        &RootTarget::Glob("src".to_string())
    );

    assert_eq!(
        root_at(&roots, "add@11[0]").target(),
        &RootTarget::Glob("scripts/entrypoint.sh".to_string())
    );

    // `./` is the destination, never a source.
    assert!(
        !keys(&roots).iter().any(|k| k == &"copy@4[2]"),
        "the destination is not a source; keys were {:?}",
        keys(&roots)
    );
}

#[test]
fn a_copy_from_another_build_stage_names_no_repository_file() {
    let roots = parse_dockerfile(Path::new("Dockerfile"), FULL_DOCKERFILE).unwrap();

    // `COPY --from=builder /app/dist` reads out of an earlier stage's
    // filesystem, not out of this repository. Recording it as a repo path would
    // manufacture a root for a file that does not exist here.
    assert!(
        !keys(&roots).iter().any(|k| k.starts_with("copy@10")),
        "a --from copy is not a repo root; keys were {:?}",
        keys(&roots)
    );
}

#[test]
fn dockerfile_records_both_the_exec_form_and_the_shell_form() {
    let roots = parse_dockerfile(Path::new("Dockerfile"), FULL_DOCKERFILE).unwrap();

    assert_eq!(
        root_at(&roots, "entrypoint@12").target(),
        &RootTarget::Command("/entrypoint.sh".to_string())
    );
    assert_eq!(
        root_at(&roots, "entrypoint@12").kind(),
        RootKind::ContainerEntry
    );
    assert_eq!(
        root_at(&roots, "cmd@13").target(),
        &RootTarget::Command("node /app/dist/server.js".to_string())
    );
}

#[test]
fn dockerfile_heredocs_are_a_body_not_a_run_of_bad_instructions() {
    let df = "FROM alpine\nRUN <<EOF\napt-get update\nNOT_AN_INSTRUCTION\nEOF\nCMD [\"sh\"]\n";
    let roots = parse_dockerfile(Path::new("Dockerfile"), df).unwrap();
    assert_eq!(
        root_at(&roots, "cmd@6").target(),
        &RootTarget::Command("sh".to_string())
    );
}

#[test]
fn malformed_dockerfile_is_an_error_not_an_empty_root_list() {
    for (label, bad, because) in [
        (
            "truncated exec form",
            "FROM a\nCMD [\"node\",\n",
            "JSON array",
        ),
        (
            "unknown instruction",
            "FROM a\nNOTANINSTRUCTION x\n",
            "not a Dockerfile instruction",
        ),
        (
            "instruction with no arguments",
            "FROM a\nCOPY\n",
            "no arguments",
        ),
        ("no FROM at all", "COPY a b\n", "FROM"),
    ] {
        let err = match parse_dockerfile(Path::new("Dockerfile"), bad) {
            Err(err) => err,
            Ok(roots) => panic!(
                "{label}: expected an error, got {} roots",
                roots.roots().len()
            ),
        };
        assert!(
            err.to_string().contains(because),
            "{label}: expected the error to mention {because:?}, got {err}"
        );
    }
}

// ---------------------------------------------------------------------------
// .github/workflows/*.yml (§5.2, CI)
// ---------------------------------------------------------------------------

const FULL_WORKFLOW: &str = r#"name: CI
on:
  push:
    branches: [main, "release/*"]
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - name: Install
        run: npm ci
      - name: Build
        run: |
          npm run build
          node scripts/verify.js
      - uses: ./.github/actions/report
        with:
          path: out
"#;

#[test]
fn a_workflow_records_every_run_body_with_the_step_that_holds_it() {
    let roots =
        parse_github_workflow(Path::new(".github/workflows/ci.yml"), FULL_WORKFLOW).unwrap();

    assert_eq!(
        root_at(&roots, "jobs.build.steps[1].run").target(),
        &RootTarget::Command("npm ci".to_string())
    );
    assert_eq!(
        root_at(&roots, "jobs.build.steps[1].run").kind(),
        RootKind::Command
    );

    // A literal block scalar keeps its newlines: the body is a script, and
    // folding it would change what it runs.
    assert_eq!(
        root_at(&roots, "jobs.build.steps[2].run").target(),
        &RootTarget::Command("npm run build\nnode scripts/verify.js\n".to_string())
    );
}

#[test]
fn a_workflow_distinguishes_a_local_action_from_a_published_one() {
    let roots =
        parse_github_workflow(Path::new(".github/workflows/ci.yml"), FULL_WORKFLOW).unwrap();

    assert_eq!(
        root_at(&roots, "jobs.build.steps[0].uses").target(),
        &RootTarget::Reference("actions/checkout@v4".to_string())
    );
    assert_eq!(
        root_at(&roots, "jobs.build.steps[0].uses").kind(),
        RootKind::CiAction
    );

    // GitHub resolves a `./` action against the repository root, not against
    // the workflow's own directory.
    assert_eq!(
        root_at(&roots, "jobs.build.steps[3].uses").target(),
        &RootTarget::Path(PathBuf::from(".github/actions/report"))
    );
}

#[test]
fn a_workflow_with_no_run_or_uses_declares_nothing_and_is_not_an_error() {
    let roots = parse_github_workflow(Path::new(".github/workflows/x.yml"), "name: CI\non: push\n")
        .unwrap();
    assert!(roots.is_empty());
}

#[test]
fn malformed_workflow_yaml_is_an_error_not_an_empty_root_list() {
    for (label, bad, because) in [
        (
            "tab used for indentation",
            "jobs:\n\tbuild:\n\t\truns-on: x\n",
            "tab",
        ),
        (
            "dedent to a column that was never opened",
            "a:\n    b: 1\n  c: 2\n",
            "indentation",
        ),
        ("an anchor we do not model", "a: &base\n  b: 1\n", "anchors"),
        ("an alias we do not model", "a:\n  b: *base\n", "anchors"),
        ("a second document", "a: 1\n---\nb: 2\n", "document"),
        (
            "unterminated quoted scalar",
            "a: \"unclosed\n",
            "unterminated",
        ),
    ] {
        let result = parse_github_workflow(Path::new(".github/workflows/ci.yml"), bad);
        let err = match result {
            Err(err) => err,
            Ok(roots) => panic!(
                "{label}: expected an error, got {} roots",
                roots.roots().len()
            ),
        };
        assert!(
            err.to_string().contains(because),
            "{label}: expected the error to mention {because:?}, got {err}"
        );
    }
}

// ---------------------------------------------------------------------------
// scan: the whole repository
// ---------------------------------------------------------------------------

fn write(root: &Path, rel: &str, content: &str) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}

fn corpus() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(root, "package.json", r#"{"name":"w","bin":"./cli.js"}"#);
    // The file the `bin` above declares. Without it the fixture asserted that a
    // scan resolves a path to a file the fixture never created, which is the
    // §4.3 defect written into a test.
    write(root, "cli.js", "");
    write(
        root,
        "pyproject.toml",
        "[project.scripts]\nacme = \"acme.cli:main\"\n",
    );
    write(
        root,
        "Cargo.toml",
        "[package]\nname = \"w\"\nversion = \"0.1.0\"\n",
    );
    write(root, "src/main.rs", "fn main() {}\n");
    write(root, "src/lib.rs", "");
    write(root, "src/bin/tool.rs", "fn main() {}\n");
    write(root, "build.rs", "fn main() {}\n");
    write(root, "go.mod", "module example.com/w\n\ngo 1.22\n");
    write(root, "cmd/app/main.go", "package main\n");
    write(root, "internal/lib/lib.go", "package lib\n");
    write(root, "app/__main__.py", "");
    write(root, "app/wsgi.py", "");
    write(root, "app/asgi.py", "");
    write(root, "manage.py", "");
    write(root, "conftest.py", "");
    write(root, "proj/celery.py", "");
    write(root, "Dockerfile", "FROM alpine\nCMD [\"/app/run\"]\n");
    write(
        root,
        ".github/workflows/ci.yml",
        "jobs:\n  t:\n    steps:\n      - run: cargo test\n",
    );
    // Must never be read: it is not this repository's source.
    write(
        root,
        "node_modules/dep/package.json",
        r#"{"bin":"./nope.js"}"#,
    );
    dir
}

#[test]
fn scan_reads_every_manifest_family_in_the_tree() {
    let dir = corpus();
    let roots = scan(dir.path()).unwrap();

    assert_path(&roots, "bin", "cli.js");
    assert_eq!(
        root_at(&roots, "project.scripts.acme").target(),
        &RootTarget::Reference("acme.cli:main".to_string())
    );
    assert_eq!(
        root_at(&roots, "module").target(),
        &RootTarget::Reference("example.com/w".to_string())
    );
    assert_eq!(
        root_at(&roots, "cmd@2").target(),
        &RootTarget::Command("/app/run".to_string())
    );
    assert_eq!(
        root_at(&roots, "jobs.t.steps[0].run").target(),
        &RootTarget::Command("cargo test".to_string())
    );
}

#[test]
fn scan_finds_the_implicit_files_no_manifest_key_names() {
    let dir = corpus();
    let roots = scan(dir.path()).unwrap();

    for (file, key) in [
        ("src/main.rs", "cargo:default-bin"),
        ("src/lib.rs", "cargo:default-lib"),
        ("src/bin/tool.rs", "cargo:src-bin"),
        ("build.rs", "cargo:build-script"),
        ("app/__main__.py", "python:dash-m"),
        ("conftest.py", "pytest:conftest"),
        ("cmd/app/main.go", "package"),
    ] {
        let root = root_at(&roots, key);
        assert_eq!(
            root.origin().file(),
            Path::new(file),
            "{key} should come from {file}"
        );
    }

    // A Go file in a library package is not an entry point.
    assert!(
        roots
            .roots()
            .iter()
            .all(|r| r.origin().file() != Path::new("internal/lib/lib.go")),
        "a non-main package declares no root"
    );
}

#[test]
fn a_framework_named_file_is_tier_b_because_it_is_only_right_if_the_framework_is() {
    let dir = corpus();
    let roots = scan(dir.path()).unwrap();

    // §5.1: a file that is an entry point *because of what it is called* is
    // convention-inferable, not machine-declared. `wsgi.py` is only a root if
    // something outside the repo is configured to import it, and a root that
    // claimed Tier A here would invite a caller to trust a guess.
    for key in [
        "wsgi:callable",
        "asgi:callable",
        "django:manage",
        "celery:app",
    ] {
        assert_eq!(
            root_at(&roots, key).tier(),
            Tier::B,
            "{key} is a framework convention"
        );
    }
    // Whereas Cargo genuinely reads src/main.rs, with no framework to detect.
    assert_eq!(root_at(&roots, "cargo:default-bin").tier(), Tier::A);
}

#[test]
fn only_cargos_own_bin_layouts_are_implicit_binaries() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "Cargo.toml",
        "[package]\nname = \"w\"\nversion = \"0.1.0\"\n",
    );
    write(dir.path(), "src/bin/tool.rs", "fn main() {}\n");
    write(dir.path(), "src/bin/multi/main.rs", "fn main() {}\n");
    // A module *inside* a multi-file binary is not itself a target.
    write(dir.path(), "src/bin/multi/helper.rs", "pub fn help() {}\n");

    let roots = scan(dir.path()).unwrap();
    let bins: Vec<&Path> = roots
        .roots()
        .iter()
        .filter(|r| r.origin().key() == "cargo:src-bin")
        .map(|r| r.origin().file())
        .collect();

    assert_eq!(
        bins,
        vec![
            Path::new("src/bin/multi/main.rs"),
            Path::new("src/bin/tool.rs")
        ],
        "only `src/bin/<name>.rs` and `src/bin/<name>/main.rs` are targets"
    );
}

#[test]
fn scan_does_not_read_vendored_dependencies() {
    let dir = corpus();
    let roots = scan(dir.path()).unwrap();

    assert!(
        roots
            .sources()
            .iter()
            .all(|s| !s.starts_with("node_modules")),
        "node_modules is not this repository's source; sources were {:?}",
        roots.sources()
    );
    assert!(judged_core::roots::manifest::SKIPPED_DIRECTORIES.contains(&"node_modules"));
}

#[test]
fn scan_reports_every_manifest_it_read() {
    let dir = corpus();
    let roots = scan(dir.path()).unwrap();

    for expected in [
        "package.json",
        "pyproject.toml",
        "Cargo.toml",
        "go.mod",
        "Dockerfile",
    ] {
        assert!(
            roots.sources().iter().any(|s| s == Path::new(expected)),
            "{expected} should be listed as read; sources were {:?}",
            roots.sources()
        );
    }
}

#[test]
fn one_malformed_manifest_fails_the_whole_scan() {
    // The rule the module exists for. A scan that shrugged off the broken
    // manifest and returned the other 40 roots would report a root set that is
    // missing exactly the entry points of the package it could not read.
    let dir = corpus();
    write(dir.path(), "packages/api/package.json", r#"{"bin": }"#);

    let err = scan(dir.path()).expect_err("a scan over a broken manifest must not succeed");
    assert!(
        err.to_string().contains("packages/api/package.json"),
        "the error must name the manifest, got {err}"
    );
}

#[test]
fn scan_of_an_empty_directory_is_an_empty_root_set_not_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let roots = scan(dir.path()).unwrap();
    assert!(roots.is_empty());
    assert!(roots.sources().is_empty());
}

#[test]
fn printseeds_keeps_one_line_per_root_even_when_the_target_is_a_script() {
    // A `run:` body is a shell script with newlines in it. A line-oriented
    // report that lets them through stops being line-oriented, and the reader
    // can no longer tell a second root from the second line of the first one.
    let roots =
        parse_github_workflow(Path::new(".github/workflows/ci.yml"), FULL_WORKFLOW).unwrap();
    let seeds = roots.printseeds();

    assert_eq!(
        seeds.lines().count(),
        roots.roots().len(),
        "one line per root, got:\n{seeds}"
    );
    assert!(
        seeds.contains(r"npm run build\nnode scripts/verify.js\n"),
        "the script should survive escaped, got:\n{seeds}"
    );
}

/// A positive control (§0 ranks these first): the parsers meet this
/// repository's own manifests, which no fixture in this file was written to
/// flatter. If it fails, a real `Cargo.toml` or the real CI workflow stopped
/// parsing — fix the parser, not this test.
#[test]
fn scanning_this_repository_reads_its_real_manifests() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let roots = scan(&repo).unwrap();

    assert!(
        roots
            .sources()
            .iter()
            .any(|s| s == Path::new(".github/workflows/ci.yml")),
        "the CI workflow should have been read; sources were {:?}",
        roots.sources()
    );
    assert!(
        roots
            .roots()
            .iter()
            .any(|r| r.origin().key() == "cargo:default-bin"),
        "judged-cli's src/main.rs is a Cargo default binary"
    );
}

// ---------------------------------------------------------------------------
// the out-of-sample corpus (docs/evals/2026-08-02-out-of-sample-corpus.md §4.1)
// ---------------------------------------------------------------------------
//
// Nine real repositories were scanned; seven contained a manifest the parsers
// rejected, and in every case the file was valid. Five constructs accounted for
// all seven, and each one below is that construct in the file it was found in
// — the whole file, byte for byte, at the commit named, so that a fixture
// cannot quietly become a paraphrase of what the construct was assumed to be.
//
// A test here fails if the parser rejects a valid manifest. That is the mirror
// of the malformed-input tests above, and both directions have to hold at once:
// refusing a file we cannot read is the policy, and refusing a file we can read
// is a defect that empties the root set of an entire repository.

/// `Cargo.toml` from BurntSushi/ripgrep, verbatim at `435f59fc4b43af3ab32f34d53fa34978f393fe52`.
const RIPGREP_CARGO_TOML: &str = r##"[package]
name = "ripgrep"
version = "15.2.0"  #:version
authors = ["Andrew Gallant <jamslam@gmail.com>"]
description = """
ripgrep is a line-oriented search tool that recursively searches the current
directory for a regex pattern while respecting gitignore rules. ripgrep has
first class support on Windows, macOS and Linux.
"""
documentation = "https://github.com/BurntSushi/ripgrep"
homepage = "https://github.com/BurntSushi/ripgrep"
repository = "https://github.com/BurntSushi/ripgrep"
keywords = ["regex", "grep", "egrep", "search", "pattern"]
categories = ["command-line-utilities", "text-processing"]
license = "Unlicense OR MIT"
exclude = [
  "HomebrewFormula",
  "/.github/",
  "/ci/",
  "/pkg/brew",
  "/benchsuite/",
  "/scripts/",
  "/crates/fuzz",
]
build = "build.rs"
autotests = false
edition.workspace = true
rust-version.workspace = true

[[bin]]
bench = false
path = "crates/core/main.rs"
name = "rg"

[[test]]
name = "integration"
path = "tests/tests.rs"

[workspace]
members = [
  "crates/globset",
  "crates/grep",
  "crates/cli",
  "crates/index",
  "crates/matcher",
  "crates/pcre2",
  "crates/printer",
  "crates/regex",
  "crates/searcher",
  "crates/ignore",
]

[workspace.package]
edition = "2024"
rust-version = "1.96"

[dependencies]
anyhow = "1.0.75"
bstr = "1.7.0"
grep = { version = "0.4.1", path = "crates/grep" }
grep-index = { version = "0.0.1", path = "crates/index", optional = true }
ignore = { version = "0.4.29", path = "crates/ignore" }
lexopt = "0.3.0"
log = "0.4.5"
serde_json = "1.0.23"
termcolor = "1.4.0"
textwrap = { version = "0.16.0", default-features = false }

[target.'cfg(all(target_env = "musl", target_pointer_width = "64"))'.dependencies.tikv-jemallocator]
version = "0.7.0"

[dev-dependencies]
serde = "1.0.77"
serde_derive = "1.0.77"
walkdir = "2"

[features]
pcre2 = ["grep/pcre2"]
# This provides opt-in support for indexing. This is currently in active
# development and may have very serious bugs. Use at your own risk.
unstable-index = ["dep:grep-index"]

[profile.release]
debug = 1

[profile.release-lto]
inherits = "release"
opt-level = 3
debug = "none"
strip = "symbols"
debug-assertions = false
overflow-checks = false
lto = "fat"
panic = "abort"
incremental = false
codegen-units = 1

[profile.deb]
inherits = "release-lto"

[package.metadata.deb]
features = ["pcre2"]
section = "utils"
assets = [
  ["target/release/rg", "usr/bin/", "755"],
  ["COPYING", "usr/share/doc/ripgrep/", "644"],
  ["LICENSE-MIT", "usr/share/doc/ripgrep/", "644"],
  ["UNLICENSE", "usr/share/doc/ripgrep/", "644"],
  ["CHANGELOG.md", "usr/share/doc/ripgrep/CHANGELOG", "644"],
  ["README.md", "usr/share/doc/ripgrep/README", "644"],
  ["FAQ.md", "usr/share/doc/ripgrep/FAQ", "644"],
  # The man page is automatically generated by ripgrep's build process, so
  # this file isn't actually committed. Instead, to create a dpkg, either
  # create a deployment/deb directory and copy the man page to it, or use the
  # 'ci/build-deb' script.
  ["deployment/deb/rg.1", "usr/share/man/man1/rg.1", "644"],
  # Similarly for shell completions.
  ["deployment/deb/rg.bash", "usr/share/bash-completion/completions/rg", "644"],
  ["deployment/deb/rg.fish", "usr/share/fish/vendor_completions.d/rg.fish", "644"],
  ["deployment/deb/_rg", "usr/share/zsh/vendor-completions/", "644"],
]
extended-description = """\
ripgrep (rg) recursively searches your current directory for a regex pattern.
By default, ripgrep will respect your .gitignore and automatically skip hidden
files/directories and binary files.
"""
"##;

/// `go.mod` from kubernetes/sample-controller, verbatim at `3e50bfd72c521dd4b1b9d832b9f2e6254b4ff148`.
const SAMPLE_CONTROLLER_GO_MOD: &str = r##"// This is a generated file. Do not edit directly.

module k8s.io/sample-controller

go 1.26.0

godebug default=go1.26

require (
	golang.org/x/time v0.15.0
	k8s.io/api v0.0.0-20260721190412-6e4e0381102b
	k8s.io/apimachinery v0.0.0-20260721185639-d7ad413f224b
	k8s.io/client-go v0.0.0-20260721191433-184dcc9d4e03
	k8s.io/code-generator v0.0.0-20260721193427-82c4ba9373f9
	k8s.io/klog/v2 v2.140.0
	k8s.io/kube-openapi v0.0.0-20260721132016-d427ff9ee9ad
	k8s.io/utils v0.0.0-20260626114624-be93311217bd
	sigs.k8s.io/structured-merge-diff/v6 v6.4.2
)

require (
	github.com/davecgh/go-spew v1.1.2-0.20180830191138-d8f796af33cc // indirect
	github.com/emicklei/go-restful/v3 v3.13.0 // indirect
	github.com/fxamacker/cbor/v2 v2.9.1 // indirect
	github.com/go-logr/logr v1.4.3 // indirect
	github.com/go-openapi/jsonpointer v1.0.0 // indirect
	github.com/go-openapi/jsonreference v1.0.0 // indirect
	github.com/go-openapi/swag v0.27.1 // indirect
	github.com/go-openapi/swag/cmdutils v0.27.1 // indirect
	github.com/go-openapi/swag/conv v0.27.1 // indirect
	github.com/go-openapi/swag/fileutils v0.27.1 // indirect
	github.com/go-openapi/swag/jsonutils v0.27.1 // indirect
	github.com/go-openapi/swag/loading v0.27.1 // indirect
	github.com/go-openapi/swag/mangling v0.27.1 // indirect
	github.com/go-openapi/swag/netutils v0.27.1 // indirect
	github.com/go-openapi/swag/pools v0.27.1 // indirect
	github.com/go-openapi/swag/stringutils v0.27.1 // indirect
	github.com/go-openapi/swag/typeutils v0.27.1 // indirect
	github.com/go-openapi/swag/yamlutils v0.27.1 // indirect
	github.com/google/gnostic-models v0.7.0 // indirect
	github.com/google/uuid v1.6.0 // indirect
	github.com/json-iterator/go v1.1.12 // indirect
	github.com/modern-go/concurrent v0.0.0-20180306012644-bacd9c7ef1dd // indirect
	github.com/modern-go/reflect2 v1.0.3-0.20250322232337-35a7c28c31ee // indirect
	github.com/munnerz/goautoneg v0.0.0-20191010083416-a7dc8b61c822 // indirect
	github.com/pmezard/go-difflib v1.0.1-0.20181226105442-5d4384ee4fb2 // indirect
	github.com/spf13/pflag v1.0.10 // indirect
	github.com/x448/float16 v0.8.4 // indirect
	go.yaml.in/yaml/v2 v2.4.4 // indirect
	go.yaml.in/yaml/v3 v3.0.4 // indirect
	golang.org/x/mod v0.37.0 // indirect
	golang.org/x/net v0.57.0 // indirect
	golang.org/x/oauth2 v0.36.0 // indirect
	golang.org/x/sync v0.22.0 // indirect
	golang.org/x/sys v0.47.0 // indirect
	golang.org/x/term v0.45.0 // indirect
	golang.org/x/text v0.40.0 // indirect
	golang.org/x/tools v0.47.0 // indirect
	google.golang.org/protobuf v1.36.12-0.20260120151049-f2248ac996af // indirect
	gopkg.in/evanphx/json-patch.v4 v4.13.0 // indirect
	gopkg.in/inf.v0 v0.9.1 // indirect
	k8s.io/gengo/v2 v2.0.0-20260408192533-25e2208e0dc3 // indirect
	sigs.k8s.io/json v0.0.0-20250730193827-2d320260d730 // indirect
	sigs.k8s.io/randfill v1.0.0 // indirect
	sigs.k8s.io/yaml v1.6.0 // indirect
)
"##;

/// `.github/workflows/golangci-lint.yml` from prometheus/node_exporter, verbatim at `b401dcfc667cee0a5d29232bab51a8ce1c58ec07`.
const NODE_EXPORTER_GOLANGCI_LINT_YML: &str = r##"---
# This action is synced from https://github.com/prometheus/prometheus
name: golangci-lint
on:
  push:
    branches: [main, master, 'release-*']
    paths:
      - "go.sum"
      - "go.mod"
      - "**.go"
      - "scripts/errcheck_excludes.txt"
      - ".github/workflows/golangci-lint.yml"
      - ".golangci.yml"
    tags: ['v*']
  pull_request:

permissions:  # added using https://github.com/step-security/secure-repo
  contents: read

jobs:
  golangci:
    permissions:
      contents: read  # for actions/checkout to fetch code
      pull-requests: read  # for golangci/golangci-lint-action to fetch pull requests
    name: lint
    runs-on: ubuntu-latest
    steps:
      - name: Checkout repository
        uses: actions/checkout@9c091bb21b7c1c1d1991bb908d89e4e9dddfe3e0 # v7.0.0
        with:
          persist-credentials: false
      - name: Install Go
        uses: actions/setup-go@924ae3a1cded613372ab5595356fb5720e22ba16 # v6.5.0
        with:
          go-version: 1.26.x
      - name: Install snmp_exporter/generator dependencies
        run: sudo apt-get update && sudo apt-get -y install libsnmp-dev
        if: github.repository == 'prometheus/snmp_exporter'
      - name: Get golangci-lint version
        id: golangci-lint-version
        run: echo "version=$(make print-golangci-lint-version)" >> $GITHUB_OUTPUT
      - name: Lint
        uses: golangci/golangci-lint-action@ba0d7d2ec06a0ea1cb5fa41b2e4a3ab91d21278a # v9.3.0
        with:
          args: --verbose
          version: ${{ steps.golangci-lint-version.outputs.version }}
"##;

/// `.github/workflows/test.yml` from spf13/cobra, verbatim at `adbc8813901bba65827259daa8e22ff94ec1f30e`.
const COBRA_TEST_YML: &str = r##"name: Test

on:
  push:
  pull_request:
  workflow_dispatch:

env:
  GO111MODULE: on

permissions:
  contents: read

jobs:


  lic-headers:
    runs-on: ubuntu-latest
    steps:

      - uses: actions/checkout@v4

      - run: >-
          docker run
          -v $(pwd):/wrk -w /wrk
          ghcr.io/google/addlicense
          -c 'The Cobra Authors'
          -y '2013-2023'
          -l apache
          -ignore '.github/**'
          -check
          .


  golangci-lint:
    permissions:
      contents: read  # for actions/checkout to fetch code
      pull-requests: read  # for golangci/golangci-lint-action to fetch pull requests
    runs-on: ubuntu-latest
    steps:

      - uses: actions/checkout@v4

      - uses: actions/setup-go@v6
        with:
          go-version: '^1.22'
          check-latest: true
          cache: true

      - uses: golangci/golangci-lint-action@v8.0.0
        with:
          version: latest
          args: --verbose


  test-unix:
    strategy:
      fail-fast: false
      matrix:
        platform:
        - ubuntu
        - macOS
        go:
        - 17
        - 18
        - 19
        - 20
        - 21
        - 22
        - 23
        - 24
    name: '${{ matrix.platform }} | 1.${{ matrix.go }}.x'
    runs-on: ${{ matrix.platform }}-latest
    # macOS runner environments require compiled binaries to contain an LC_UUID load command.
    # Go 1.24+ natively resolves this by writing LC_UUID in Go's internal linker.
    # For Go < 1.24, we pass -ldflags=-linkmode=external on macOS to force external linking via the system linker.
    env:
      GO_TEST_FLAGS: ${{ matrix.platform == 'macOS' && matrix.go < 24 && '-ldflags=-linkmode=external' || '' }}
    steps:

    - uses: actions/checkout@v4

    - uses: actions/setup-go@v6
      with:
        go-version: 1.${{ matrix.go }}.x
        cache: true

    - run: |
        export GOBIN=$HOME/go/bin
        go install github.com/kyoh86/richgo@latest
        go install github.com/mitchellh/gox@latest

    - run: RICHGO_FORCE_COLOR=1 PATH=$HOME/go/bin/:$PATH make richtest


  test-win:
    name: MINGW64
    defaults:
      run:
        shell: msys2 {0}
    runs-on: windows-latest
    steps:

    - shell: bash
      run: git config --global core.autocrlf input

    - uses: msys2/setup-msys2@v2
      with:
        msystem: MINGW64
        update: true
        install: >
          git
          make
          unzip
          mingw-w64-x86_64-go

    - uses: actions/checkout@v4

    - uses: actions/cache@v4
      with:
        path: ~/go/pkg/mod
        key: ${{ runner.os }}-${{ matrix.go }}-${{ hashFiles('**/go.sum') }}
        restore-keys: ${{ runner.os }}-${{ matrix.go }}-

    - run: |
        export GOBIN=$HOME/go/bin
        go install github.com/kyoh86/richgo@latest
        go install github.com/mitchellh/gox@latest

    - run: RICHGO_FORCE_COLOR=1 PATH=$HOME/go/bin:$PATH make richtest
"##;

/// `.github/workflows/run-tests.yml` from psf/requests, verbatim at `414f0513c33883adf6f2b46901d4f0b38a455851`.
const REQUESTS_RUN_TESTS_YML: &str = r##"name: Tests

on: [push, pull_request]

permissions:
  contents: read

jobs:
  build:
    runs-on: ${{ matrix.os }}
    timeout-minutes: 10
    strategy:
      fail-fast: false
      matrix:
        python-version: ["3.10", "3.11", "3.12", "3.13", "3.14", "3.14t", "3.15-dev", "pypy-3.11"]
        os: [ubuntu-22.04, macOS-latest, windows-latest]
        # Pypy-3.11 can't install openssl-sys with rust
        # which prevents us from testing in GHA.
        exclude:
        - { python-version: "pypy-3.11", os: "windows-latest" }

    steps:
    - uses: actions/checkout@9c091bb21b7c1c1d1991bb908d89e4e9dddfe3e0 # v7.0.0
      with:
        persist-credentials: false
    - name: Set up Python ${{ matrix.python-version }}
      uses: actions/setup-python@5fda3b95a4ea91299a34e894583c3862153e4b97 # v7.0.0
      with:
        python-version: ${{ matrix.python-version }}
        cache: 'pip'
        allow-prereleases: true
    - name: Install dependencies
      env:
        PYO3_USE_ABI3_FORWARD_COMPATIBILITY: ${{ matrix.python-version == '3.15-dev' && '1' || '' }}
      run: |
        make
    - name: Run tests
      run: |
        make ci

  no_chardet:
    name: "No Character Detection"
    runs-on: ubuntu-latest
    strategy:
      fail-fast: true

    steps:
      - uses: actions/checkout@9c091bb21b7c1c1d1991bb908d89e4e9dddfe3e0
        with:
          persist-credentials: false
      - name: 'Set up Python 3.10'
        uses: actions/setup-python@5fda3b95a4ea91299a34e894583c3862153e4b97
        with:
          python-version: '3.10'
      - name: Install dependencies
        run: |
          make
          python -m pip uninstall -y "charset_normalizer" "chardet"
      - name: Run tests
        run: |
          make ci

  urllib3:
    name: 'urllib3 1.x'
    runs-on: 'ubuntu-latest'
    strategy:
      fail-fast: true

    steps:
      - uses: actions/checkout@9c091bb21b7c1c1d1991bb908d89e4e9dddfe3e0
        with:
          persist-credentials: false
      - name: 'Set up Python 3.10'
        uses: actions/setup-python@5fda3b95a4ea91299a34e894583c3862153e4b97
        with:
          python-version: '3.10'
      - name: Install dependencies
        run: |
          make
          python  -m pip install "urllib3<2"
      - name: Run tests
        run: |
          make ci
"##;

/// Corpus defect (c): a line continuation inside a multi-line basic string.
///
/// `Cargo.toml:122` opens the Debian `extended-description` with `"""\`. A
/// trailing backslash before a newline is TOML 1.0: it trims the newline and
/// the leading whitespace of the next line. Rejecting it cost ripgrep every one
/// of its 76 Tier A roots.
#[test]
fn ripgreps_debian_extended_description_is_a_valid_multi_line_string() {
    let roots = parse_cargo_toml(Path::new("Cargo.toml"), RIPGREP_CARGO_TOML)
        .expect("ripgrep's Cargo.toml is valid TOML and must parse");

    assert_path(&roots, "bin[0].path", "crates/core/main.rs");
    assert_eq!(root_at(&roots, "bin[0].path").kind(), RootKind::Executable);
    assert_path(&roots, "test[0].path", "tests/tests.rs");
}

/// Corpus defect (d): `godebug` is a real `go.mod` directive, since Go 1.23.
///
/// `go.mod:7` of sample-controller is `godebug default=go1.26`, in a file whose
/// first line says it is generated. Rejecting the directive cost the repository
/// its whole root set.
#[test]
fn sample_controllers_go_mod_declares_a_godebug_directive() {
    let roots = parse_go_mod(Path::new("go.mod"), SAMPLE_CONTROLLER_GO_MOD)
        .expect("a `godebug` directive is valid go.mod and must parse");

    assert_eq!(
        root_at(&roots, "module").target(),
        &RootTarget::Reference("k8s.io/sample-controller".to_string())
    );
}

/// Corpus defect (a): a key whose only value on its line is a trailing comment.
///
/// `golangci-lint.yml:17` is `permissions:  # added using ...`, and the mapping
/// that belongs to it opens on the next line. Reading the comment as the
/// scalar value of the key made the nested block arrive where no block was
/// expected. Three of the seven rejected repositories were this construct.
#[test]
fn node_exporters_permissions_key_carries_only_a_trailing_comment() {
    let workflow = Path::new(".github/workflows/golangci-lint.yml");
    let roots = parse_github_workflow(workflow, NODE_EXPORTER_GOLANGCI_LINT_YML)
        .expect("a trailing comment after a key is valid YAML and must parse");

    assert_eq!(
        root_at(&roots, "jobs.golangci.steps[2].run").target(),
        &RootTarget::Command(
            "sudo apt-get update && sudo apt-get -y install libsnmp-dev".to_string()
        )
    );
    assert_eq!(
        root_at(&roots, "jobs.golangci.steps[0].uses").target(),
        &RootTarget::Reference(
            "actions/checkout@9c091bb21b7c1c1d1991bb908d89e4e9dddfe3e0".to_string()
        )
    );
    // The comment is not part of the value it follows.
    assert!(
        !keys(&roots).contains(&"permissions"),
        "`permissions:` declares no root; its trailing comment is not a scalar"
    );
}

/// Corpus defect (b): `jobs.<id>.defaults.run` is a mapping; a step's `run` is
/// a command.
///
/// `test.yml:98` sets a default shell for the whole job. Both keys are spelled
/// `run`, and treating every one of them as a step command rejected the file.
#[test]
fn cobras_job_default_run_is_a_mapping_not_a_command() {
    let workflow = Path::new(".github/workflows/test.yml");
    let roots = parse_github_workflow(workflow, COBRA_TEST_YML)
        .expect("`defaults.run` is a mapping in the Actions schema and must parse");

    // The command steps of the job that carries the default are still roots.
    assert_eq!(
        root_at(&roots, "jobs.test-win.steps[0].run").target(),
        &RootTarget::Command("git config --global core.autocrlf input".to_string())
    );
    assert_eq!(
        root_at(&roots, "jobs.test-win.steps[5].run").target(),
        &RootTarget::Command(
            "RICHGO_FORCE_COLOR=1 PATH=$HOME/go/bin:$PATH make richtest".to_string()
        )
    );
    // The default shell is a setting, not a command anything runs.
    assert!(
        !keys(&roots).contains(&"jobs.test-win.defaults.run"),
        "a job's default `run` block declares no command; keys were {:?}",
        keys(&roots)
    );
}

/// Corpus defect (e): a flow mapping is a mapping.
///
/// `run-tests.yml:20` excludes one matrix combination with
/// `- { python-version: "pypy-3.11", os: "windows-latest" }`. The same data as
/// a block mapping parsed; written inline it did not, and the 27 Tier A roots
/// of the file were lost with it.
#[test]
fn requests_excludes_a_matrix_combination_with_a_flow_mapping() {
    let workflow = Path::new(".github/workflows/run-tests.yml");
    let roots = parse_github_workflow(workflow, REQUESTS_RUN_TESTS_YML)
        .expect("a flow mapping is valid YAML and must parse");

    // Every step in this job is written *after* the flow mapping on line 20,
    // so reading any of them proves the parser got past it.
    assert_eq!(
        root_at(&roots, "jobs.build.steps[0].uses").target(),
        &RootTarget::Reference(
            "actions/checkout@9c091bb21b7c1c1d1991bb908d89e4e9dddfe3e0".to_string()
        )
    );
    assert_eq!(
        root_at(&roots, "jobs.build.steps[3].run").target(),
        &RootTarget::Command("make ci\n".to_string())
    );
}

// ---------------------------------------------------------------------------
// Cargo target auto-discovery (§5.2, Rust)
// ---------------------------------------------------------------------------

#[test]
fn cargo_auto_discovers_test_bench_and_example_targets() {
    // §5.2 lists `[[test]]`, `[[bench]]` and `[[example]]` among Cargo's Tier A
    // targets, and Cargo finds all three on disk without any key naming them.
    // A test binary nothing declares is still a binary Cargo builds.
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "Cargo.toml",
        "[package]\nname = \"w\"\nversion = \"0.1.0\"\n",
    );
    write(dir.path(), "tests/integration.rs", "");
    write(dir.path(), "tests/multi/main.rs", "");
    write(dir.path(), "benches/throughput.rs", "");
    write(dir.path(), "benches/multi/main.rs", "");
    write(dir.path(), "examples/demo.rs", "");
    write(dir.path(), "examples/multi/main.rs", "");

    let roots = scan(dir.path()).unwrap();
    let found = |key: &str| -> Vec<&Path> {
        roots
            .roots()
            .iter()
            .filter(|r| r.origin().key() == key)
            .map(|r| r.origin().file())
            .collect()
    };

    assert_eq!(
        found("cargo:test"),
        vec![
            Path::new("tests/integration.rs"),
            Path::new("tests/multi/main.rs")
        ]
    );
    assert_eq!(
        found("cargo:bench"),
        vec![
            Path::new("benches/multi/main.rs"),
            Path::new("benches/throughput.rs")
        ]
    );
    assert_eq!(
        found("cargo:example"),
        vec![
            Path::new("examples/demo.rs"),
            Path::new("examples/multi/main.rs")
        ]
    );
    let example = roots
        .roots()
        .iter()
        .find(|r| r.origin().file() == Path::new("examples/demo.rs"))
        .expect("examples/demo.rs is a root");
    assert_eq!(
        example.kind(),
        RootKind::DevTarget,
        "an example is a development target, not a shipped executable"
    );
    assert_eq!(
        example.tier(),
        Tier::A,
        "Cargo builds it, nobody guessed it"
    );
}

#[test]
fn a_module_inside_a_multi_file_test_target_is_not_itself_a_target() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "Cargo.toml",
        "[package]\nname = \"w\"\nversion = \"0.1.0\"\n",
    );
    write(dir.path(), "tests/multi/main.rs", "");
    write(dir.path(), "tests/multi/helper.rs", "");
    // The conventional shared-code layout: Cargo compiles it into whichever
    // target declares `mod common`, and never as a target of its own.
    write(dir.path(), "tests/common/mod.rs", "");

    let roots = scan(dir.path()).unwrap();
    let tests: Vec<&Path> = roots
        .roots()
        .iter()
        .filter(|r| r.origin().key() == "cargo:test")
        .map(|r| r.origin().file())
        .collect();

    assert_eq!(tests, vec![Path::new("tests/multi/main.rs")]);
}

#[test]
fn auto_discovery_a_manifest_switches_off_is_not_discovered() {
    // ripgrep's own `Cargo.toml` carries `autotests = false` and declares the
    // one test target it wants. Emitting `tests/*.rs` anyway would invent a
    // target Cargo does not build — a guess wearing a declaration's clothes,
    // and the §9.5 shape where a fabricated root hides a real gap.
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "Cargo.toml", RIPGREP_CARGO_TOML);
    write(dir.path(), "tests/tests.rs", "");
    write(dir.path(), "tests/data.rs", "");
    write(dir.path(), "crates/core/main.rs", "");

    let roots = scan(dir.path()).unwrap();
    assert!(
        !keys(&roots).contains(&"cargo:test"),
        "`autotests = false` turns test auto-discovery off; keys were {:?}",
        keys(&roots)
    );
    // The target the manifest does declare is still there.
    assert_path(&roots, "test[0].path", "tests/tests.rs");
}

// ---------------------------------------------------------------------------
// the build context, and roots that name nothing
// (docs/evals/2026-08-02-out-of-sample-corpus.md §4.3, §4.4)
// ---------------------------------------------------------------------------

/// `src/ad/Dockerfile` from `open-telemetry/opentelemetry-demo` at the corpus
/// commit `f7408a50`, byte for byte (`git hash-object` =
/// `1b94014da0906d7b7b9498aabb20216f83979bb4`).
///
/// This is the manifest that produced the §4.3 defect: 99 of otel-demo's 130
/// `packaged_file` roots named a path that does not exist, because every `COPY`
/// source was rebased onto the Dockerfile's own directory while the build
/// context is the repository root. `COPY ./src/ad/settings.gradle*` became
/// `src/ad/src/ad/settings.gradle*`.
const OTEL_DEMO_AD_DOCKERFILE: &str = r#"# Copyright The OpenTelemetry Authors
# SPDX-License-Identifier: Apache-2.0


FROM --platform=${BUILDPLATFORM} docker.io/library/eclipse-temurin:24.0.2_12-jdk@sha256:7493205ffe6caa8074fa8a06a276bb1c5ac41d3dd0fd43a0db66d7f776e80b3e AS builder
WORKDIR /usr/src/app/

COPY ./src/ad/gradlew* ./src/ad/settings.gradle* ./src/ad/build.gradle ./
COPY ./src/ad/gradle ./gradle

RUN chmod +x ./gradlew
RUN ./gradlew
RUN ./gradlew downloadRepos

COPY ./src/ad/ ./
COPY ./pb/ ./proto
RUN chmod +x ./gradlew
RUN ./gradlew installDist -PprotoSourceDir=./proto

# -----------------------------------------------------------------------------

FROM docker.io/library/eclipse-temurin:24.0.2_12-jre@sha256:8cb2387a28af84cf0db0948d9c67d4480192f4e567027a3963f145d218e8b4f2

ARG OTEL_JAVA_AGENT_VERSION
WORKDIR /usr/src/app/

COPY --from=builder /usr/src/app/ ./
ADD --chmod=644 https://github.com/open-telemetry/opentelemetry-java-instrumentation/releases/download/v$OTEL_JAVA_AGENT_VERSION/opentelemetry-javaagent.jar /usr/src/app/opentelemetry-javaagent.jar
ENV JAVA_TOOL_OPTIONS="-javaagent:/usr/src/app/opentelemetry-javaagent.jar -Xmx200m"

EXPOSE ${AD_PORT}
EXPOSE ${AD_PROMETHEUS_PORT}
ENTRYPOINT [ "./build/install/opentelemetry-demo-ad/bin/Ad" ]
"#;

/// The paths `OTEL_DEMO_AD_DOCKERFILE` names, laid out where otel-demo really
/// has them: under the repository root, not under `src/ad/`.
fn otel_demo_ad_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(root, "src/ad/Dockerfile", OTEL_DEMO_AD_DOCKERFILE);
    write(root, "src/ad/gradlew", "");
    write(root, "src/ad/gradlew.bat", "");
    write(root, "src/ad/settings.gradle", "");
    write(root, "src/ad/build.gradle", "");
    write(root, "src/ad/gradle/wrapper/gradle-wrapper.properties", "");
    write(root, "src/ad/src/main/java/oteldemo/AdService.java", "");
    write(root, "pb/demo.proto", "");
    dir
}

#[test]
fn a_copy_source_is_relative_to_the_build_context_not_to_the_dockerfile() {
    // §4.3. Docker resolves a COPY source against the build context, and the
    // Dockerfile does not declare the context. otel-demo builds every service
    // from the repository root, so its sources are already repo-relative and
    // rebasing them onto `src/ad/` names nothing.
    let dir = otel_demo_ad_repo();
    let roots = scan(dir.path()).unwrap();

    assert_eq!(
        root_at(&roots, "copy@8[1]").target(),
        &RootTarget::Glob("src/ad/settings.gradle*".to_string()),
        "the exact root §4.3 quotes"
    );
    assert_eq!(
        root_at(&roots, "copy@8[0]").target(),
        &RootTarget::Glob("src/ad/gradlew*".to_string())
    );
    assert_eq!(
        root_at(&roots, "copy@8[2]").target(),
        &RootTarget::Path(PathBuf::from("src/ad/build.gradle")),
        "a source with no metacharacter that resolves is a path, not a glob"
    );
    assert_eq!(
        root_at(&roots, "copy@9[0]").target(),
        &RootTarget::Path(PathBuf::from("src/ad/gradle"))
    );
    assert_eq!(
        root_at(&roots, "copy@15[0]").target(),
        &RootTarget::Path(PathBuf::from("src/ad"))
    );
    assert_eq!(
        root_at(&roots, "copy@16[0]").target(),
        &RootTarget::Path(PathBuf::from("pb")),
        "`COPY ./pb/` is repo-relative too; rebasing gave `src/ad/pb`"
    );

    for root in roots.roots() {
        assert!(
            !root.target().to_string().contains("src/ad/src/ad"),
            "the doubled prefix is back: {} -> {}",
            root.origin(),
            root.target()
        );
    }
}

#[test]
fn an_add_from_a_url_is_not_a_file_in_this_repository() {
    // Line 28 of the real Dockerfile fetches the OpenTelemetry Java agent over
    // HTTPS. It used to be recorded as the repo path
    // `src/ad/https:/github.com/.../opentelemetry-javaagent.jar` — a file that
    // exists nowhere, spelled as though it were checked in. Same reasoning as
    // `COPY --from`: recording it manufactures a root for a file that is not
    // here.
    let dir = otel_demo_ad_repo();
    let roots = scan(dir.path()).unwrap();

    assert!(
        !keys(&roots).iter().any(|k| k.starts_with("add@28")),
        "a remote URL names no repository file; keys were {:?}",
        keys(&roots)
    );
    for root in roots.roots() {
        assert!(
            !root.target().to_string().contains("https:"),
            "a URL leaked into a target: {} -> {}",
            root.origin(),
            root.target()
        );
    }
}

#[test]
fn parsing_a_nested_dockerfile_with_no_tree_to_ask_invents_no_context() {
    // The public parser has no repository to resolve against, and the context
    // is not in the file. It must say so rather than guess — guessing is what
    // produced §4.3, and a caller who only has the bytes has no way to tell a
    // guess from a fact.
    let roots = parse_dockerfile(Path::new("src/ad/Dockerfile"), OTEL_DEMO_AD_DOCKERFILE).unwrap();

    assert_eq!(
        root_at(&roots, "copy@8[1]").target(),
        &RootTarget::Unresolved("./src/ad/settings.gradle*".to_string()),
        "the Dockerfile's own spelling, with no directory invented for it"
    );
    for root in roots.roots() {
        assert!(
            !root.target().to_string().contains("src/ad/src/ad"),
            "a doubled prefix from the parser alone: {} -> {}",
            root.origin(),
            root.target()
        );
    }

    // A Dockerfile at the repository root needs no tree: both candidate
    // contexts are the same directory, so nothing can be doubled.
    let at_root = parse_dockerfile(Path::new("Dockerfile"), FULL_DOCKERFILE).unwrap();
    assert_eq!(
        root_at(&at_root, "copy@4[0]").target(),
        &RootTarget::Glob("package.json".to_string())
    );
}

#[test]
fn a_dockerfile_built_from_its_own_directory_still_rebases() {
    // The other half of §4.3: rebasing is right when the context *is* the
    // Dockerfile's directory, and the fix must not throw that away. Here the
    // sources resolve under `services/api/` and nowhere else.
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(
        root,
        "services/api/Dockerfile",
        "FROM python:3.12\nCOPY ./app ./app\nCOPY requirements.txt .\n",
    );
    write(root, "services/api/app/main.py", "");
    write(root, "services/api/requirements.txt", "");

    let roots = scan(root).unwrap();
    assert_path(&roots, "copy@2[0]", "services/api/app");
    assert_path(&roots, "copy@3[0]", "services/api/requirements.txt");
}

#[test]
fn a_copy_source_that_resolves_under_neither_context_is_not_given_one() {
    // Neither `services/api/dist` nor `dist` exists, so which one the manifest
    // meant is unknown. Inventing the deeper of the two is exactly how §4.3
    // produced 99 roots naming nothing; the honest answer is the source as the
    // Dockerfile spells it, marked unresolved.
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(
        root,
        "services/api/Dockerfile",
        "FROM node:20\nCOPY ./dist ./dist\n",
    );

    let roots = scan(root).unwrap();
    assert_eq!(
        root_at(&roots, "copy@2[0]").target(),
        &RootTarget::Unresolved("./dist".to_string())
    );
}

#[test]
fn a_whole_tree_copy_names_the_build_context_not_the_empty_string() {
    // §4.4. djangoproject's `Dockerfile:41` is `COPY . .`, and it produced a
    // root whose target was the empty string. An empty target is not a root.
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(root, "Dockerfile", "FROM python:3.12\nCOPY . .\n");
    write(root, "manage.py", "");

    let roots = scan(root).unwrap();
    assert_eq!(
        root_at(&roots, "copy@2[0]").target(),
        &RootTarget::Path(PathBuf::from(".")),
        "a whole-tree copy ships the build context, and at the repo root that is `.`"
    );
}

#[test]
fn a_declared_path_that_names_nothing_is_not_emitted_as_a_resolved_path() {
    // The general rule §4.3 asks for. `dist/index.js` is a perfectly good
    // declaration — it is what npm will publish — but in this checkout it names
    // nothing, and a root set that spells it as a resolved path invites a
    // caller to believe the file was found.
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(
        root,
        "package.json",
        r#"{"name":"w","main":"dist/index.js","types":"src/index.ts"}"#,
    );
    write(root, "src/index.ts", "");

    let roots = scan(root).unwrap();
    assert_eq!(
        root_at(&roots, "main").target(),
        &RootTarget::Unresolved("dist/index.js".to_string()),
        "the declaration survives; the claim that it resolves does not"
    );
    assert_path(&roots, "types", "src/index.ts");

    // And the parser on its own still says what the manifest says: resolution
    // is a question about a tree, and `parse_package_json` is not given one.
    let parsed = parse_package_json(
        Path::new("package.json"),
        r#"{"name":"w","main":"dist/index.js"}"#,
    )
    .unwrap();
    assert_eq!(
        root_at(&parsed, "main").target(),
        &RootTarget::Path(PathBuf::from("dist/index.js"))
    );
}

#[test]
fn a_declared_path_that_escapes_the_repository_is_not_emitted_as_a_resolved_path() {
    // `join_rel` keeps a `..` that has nothing to pop, so a path leaving the
    // repository stays visibly escaped. It must not also be stat-ed: whatever
    // sits beside the repository is not this repository's root.
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(
        root,
        "package.json",
        r#"{"name":"w","main":"../outside.js"}"#,
    );

    let roots = scan(root).unwrap();
    assert_eq!(
        root_at(&roots, "main").target(),
        &RootTarget::Unresolved("../outside.js".to_string())
    );
}

#[test]
fn an_unresolved_target_is_still_a_root_and_still_prints() {
    // §9.13: it has to stay auditable. The point is that it is not spelled as a
    // resolved path, not that it disappears.
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(
        root,
        "package.json",
        r#"{"name":"w","main":"dist/index.js"}"#,
    );

    let roots = scan(root).unwrap();
    let seeds = roots.printseeds();
    assert!(
        seeds.contains("package.json#main\tdist/index.js"),
        "the declaration must still be readable in -printseeds output, got:\n{seeds}"
    );
    assert_eq!(root_at(&roots, "main").tier(), Tier::A);
}
