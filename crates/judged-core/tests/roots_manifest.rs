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
