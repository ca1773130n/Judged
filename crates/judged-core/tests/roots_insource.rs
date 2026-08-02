//! In-source Tier A roots (§5.2, R1 determination §7 item 4).
//!
//! Each marker is asserted to fire on the spelling the ecosystem actually uses,
//! and — the half that matters — asserted **not** to fire on the neighbouring
//! thing it must leave alone. A root source that fires on everything materializes
//! a root set that rescues every candidate, which is a constant function wearing
//! the word "root", and §5.1's whole point is that a root without checkable
//! provenance is worse than no root.

use std::path::{Path, PathBuf};

use judged_core::roots::insource::{scan, InSourceRoot, Marker};

fn repo(files: &[(&str, &str)]) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::Builder::new()
        .prefix("judged-insource-")
        .tempdir()
        .expect("scratch");
    for (name, body) in files {
        let path = dir.path().join(name);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(&path, body).expect("write");
    }
    let root = dir.path().to_path_buf();
    (dir, root)
}

fn symbols(found: &[InSourceRoot], marker: Marker) -> Vec<&str> {
    found
        .iter()
        .filter(|root| root.marker() == marker)
        .filter_map(InSourceRoot::symbol)
        .collect()
}

/// §4.1 records `//go:linkname` as the reason `x/tools/cmd/deadcode` reports a
/// symbol *"spuriously as dead"*, which makes it the single most load-bearing
/// entry in this module — m12 is that failure in fixture form.
#[test]
fn go_linkname_declares_both_names_it_binds() {
    let (_dir, root) = repo(&[(
        "internal/sampler/drain.go",
        "package sampler\n\n//go:linkname drain runtime.sampler_drain\nfunc drain() {}\n\
         \nfunc unrelated() {}\n",
    )]);
    let found = scan(&root).expect("scans");

    let bound = symbols(&found, Marker::GoLinkname);
    assert!(bound.contains(&"drain"), "the local name: {bound:?}");
    assert!(
        bound.contains(&"sampler_drain"),
        "and the name it is bound to, qualified in the directive: {bound:?}"
    );
    assert!(
        !bound.contains(&"unrelated"),
        "a function in the same file that no directive names is not a root"
    );
}

/// cgo's entry point. The consumer is outside the build by construction.
#[test]
fn go_export_declares_the_symbol_on_the_directive_line() {
    let (_dir, root) = repo(&[(
        "cmd/libtelemetry/abi.go",
        "package main\n\n//export TelemetryFlush\nfunc TelemetryFlush() {}\n\n\
         func internalOnly() {}\n",
    )]);
    let found = scan(&root).expect("scans");

    assert_eq!(symbols(&found, Marker::GoExport), vec!["TelemetryFlush"]);
}

/// The Rust attributes, each bound to the item it annotates and not to the next
/// one along.
#[test]
fn rust_attributes_bind_to_the_item_they_annotate() {
    let (_dir, root) = repo(&[(
        "src/ffi.rs",
        "#[no_mangle]\npub extern \"C\" fn ledger_amortize() {}\n\n\
         #[used]\n#[allow(dead_code)]\nstatic KEEP_ME: u8 = 0;\n\n\
         #[ctor]\nfn run_before_main() {}\n\n\
         #[export_name = \"ledger_v2_amortize\"]\npub extern \"C\" fn amortize_v2() {}\n\n\
         fn ordinary() {}\n",
    )]);
    let found = scan(&root).expect("scans");

    assert_eq!(
        symbols(&found, Marker::RustNoMangle),
        vec!["ledger_amortize"]
    );
    // Through an intervening attribute, which is ordinary.
    assert_eq!(symbols(&found, Marker::RustUsed), vec!["KEEP_ME"]);
    assert_eq!(symbols(&found, Marker::RustCtor), vec!["run_before_main"]);

    // Both spellings: the ABI name an outside consumer links against, and the
    // Rust name the item has here.
    let exported = symbols(&found, Marker::RustExportName);
    assert!(exported.contains(&"ledger_v2_amortize"), "{exported:?}");
    assert!(exported.contains(&"amortize_v2"), "{exported:?}");

    assert!(
        !found.iter().any(|root| root.symbol() == Some("ordinary")),
        "an unannotated function is not a root"
    );
}

/// An attribute separated from the next item by a blank line annotates nothing,
/// for the same reason Gate 3f stops there: running past it attributes a marker
/// to an unrelated declaration.
#[test]
fn an_attribute_does_not_reach_past_a_blank_line() {
    let (_dir, root) = repo(&[(
        "src/stray.rs",
        "#[no_mangle]\n\npub extern \"C\" fn not_annotated_by_it() {}\n",
    )]);
    let found = scan(&root).expect("scans");

    assert!(
        symbols(&found, Marker::RustNoMangle).is_empty(),
        "{found:?}"
    );
}

/// §5.2: *"a `.pth` file is an entry point with no caller anywhere"*, and `site`
/// executes only the lines beginning with `import`. Everything else on a line is
/// a `sys.path` entry, not code.
#[test]
fn a_pth_file_is_a_root_and_so_is_every_module_it_imports() {
    let (_dir, root) = repo(&[(
        "vendor/site-packages/zzz_ledger_bootstrap.pth",
        "import ledger_startup_hook\n/opt/extra/site-packages\nimport telemetry; telemetry.init()\n",
    )]);
    let found = scan(&root).expect("scans");

    let pth: Vec<&InSourceRoot> = found
        .iter()
        .filter(|r| r.marker() == Marker::PythonPth)
        .collect();

    assert!(
        pth.iter().any(|r| r.symbol().is_none()),
        "the file itself is the entry point, so it is a root with no symbol"
    );
    let modules = symbols(&found, Marker::PythonPth);
    assert!(modules.contains(&"ledger_startup_hook"));
    assert!(
        modules.contains(&"telemetry"),
        "`import x; x.init()` runs too: {modules:?}"
    );
    assert!(
        !modules.iter().any(|m| m.starts_with('/')),
        "a bare line is a sys.path entry, not an import: {modules:?}"
    );
}

/// `sitecustomize.py` is imported by name at interpreter start and referenced by
/// nothing.
#[test]
fn sitecustomize_is_a_root_by_its_name_alone() {
    let (_dir, root) = repo(&[
        ("vendor/site-packages/sitecustomize.py", "import ledger\n"),
        ("vendor/site-packages/ordinary.py", "import ledger\n"),
    ]);
    let found = scan(&root).expect("scans");

    let files: Vec<&Path> = found
        .iter()
        .filter(|r| r.marker() == Marker::PythonSiteCustomize)
        .map(InSourceRoot::file)
        .collect();
    assert_eq!(
        files,
        vec![Path::new("vendor/site-packages/sitecustomize.py")]
    );
}

/// A repository with none of the markers materializes none of these roots.
///
/// The mirror of every test above, and the one that would fail first if a
/// matcher were widened until it fired on ordinary code.
#[test]
fn ordinary_source_declares_no_in_source_roots() {
    let (_dir, root) = repo(&[
        (
            "src/lib.rs",
            "pub fn add(a: u8, b: u8) -> u8 {\n    a + b\n}\n",
        ),
        ("main.go", "package main\n\nfunc main() {}\n"),
        ("app/util.py", "def wrap(s):\n    return s.strip()\n"),
        (
            "README.md",
            "We use `#[no_mangle]` and `//export` in the FFI layer.\n",
        ),
    ]);

    assert!(
        scan(&root).expect("scans").is_empty(),
        "prose naming a marker is not a declaration, and ordinary code carries none"
    );
}

/// §5.2 names more in-source sources than §7 item 4 asked for, and the ones left
/// out are recorded here rather than left to be rediscovered.
///
/// `//go:embed`, `//go:wasmexport`, `#[wasm_bindgen]` and `#[pyo3::pymodule]`
/// are all real root sources and all unimplemented. A root rule with no fixture
/// exercising it is a rule nothing measures, so they wait for a class that
/// exercises them — which is §7 item 7's out-of-sample catalogue, not this
/// change.
#[test]
fn the_in_source_sources_still_unimplemented_are_named() {
    let (_dir, root) = repo(&[(
        "assets.go",
        "package main\n\n//go:embed static/*\nvar assets embed.FS\n\n\
         //go:wasmexport run\nfunc run() {}\n",
    )]);

    assert!(
        scan(&root).expect("scans").is_empty(),
        "if this starts failing, `//go:embed` or `//go:wasmexport` was implemented \
         and this test should become an assertion about what it declares"
    );
}

/// Source embedded in a string literal is text, not a declaration.
///
/// This is the defect that nearly shipped, kept as a test because it was found
/// by measurement rather than by reasoning. Run against the Judged repository —
/// which has no FFI and declares none of these roots — the first version of this
/// module reported five in-source roots and **every one was wrong**: four from a
/// test file's escaped literals and one from a fixture's raw string, with one
/// symbol reported as `ledger_v2_amortize\`, a line-continuation backslash that
/// had escaped the literal.
///
/// Both spellings are covered here because they failed for different reasons and
/// the raw-string one survived the first fix.
#[test]
fn a_marker_inside_a_string_literal_is_not_a_declaration() {
    let (_dir, root) = repo(&[
        (
            // A fixture module, exactly the shape `judged-mutants` uses: real
            // Rust whose payload is other Rust, at column zero, inside `r#"…"#`.
            "src/fixtures/m19.rs",
            "pub fn materialize(dir: &Path) -> Result<()> {\n    write(\n        dir,\n        \
             LIVE_FILE,\n        r#\"use std::os::raw::c_double;\n\n#[no_mangle]\n\
             pub extern \"C\" fn ledger_amortize(x: c_double) -> c_double {\n    x\n}\n\"#,\n    )\n}\n",
        ),
        (
            // And the escaped form, with a line continuation — the one that
            // produced a symbol name ending in a backslash.
            "tests/example.rs",
            "const SOURCE: &str = \n         \"#[export_name = \\\"ledger_v2\\\"]\\npub extern \\\"C\\\" fn amortize_v2() {}\\n\\n\\\n         fn ordinary() {}\\n\";\n",
        ),
    ]);

    let found = scan(&root).expect("scans");
    assert!(
        found.is_empty(),
        "a repository whose Rust merely quotes these markers declares no roots: {found:?}"
    );
}

/// And the mirror, so the blanking cannot pass by removing everything: real code
/// in the same repository is still read.
#[test]
fn blanking_string_literals_does_not_hide_a_real_declaration() {
    let (_dir, root) = repo(&[
        (
            "src/fixtures/m19.rs",
            "const SRC: &str = r#\"#[no_mangle]\npub extern \"C\" fn quoted_only() {}\n\"#;\n",
        ),
        (
            "src/ffi.rs",
            "#[no_mangle]\npub extern \"C\" fn genuinely_exported() {}\n",
        ),
    ]);

    let found = scan(&root).expect("scans");
    assert_eq!(
        symbols(&found, Marker::RustNoMangle),
        vec!["genuinely_exported"]
    );
}
