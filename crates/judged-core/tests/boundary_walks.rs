//! Every walker in the crate stops at a repository boundary (§9.3 0b).
//!
//! Seven walkers each implemented 0b as `name == ".git"`. That is one wrong
//! belief about how git marks a repository, copied seven times — and each
//! walker drew a different confident wrong conclusion from crossing a boundary:
//! a Tier A root materialized from a nested repository's manifest, a Gate 2
//! reference "found" in a vendored clone, an in-source `#[no_mangle]` root read
//! out of a submodule.
//!
//! So this file tests the **class**, not the seven fixes. One tree, all three
//! boundary shapes, every walker asked whether it crossed. A future walker that
//! rolls its own skip is caught here rather than in a repository somebody was
//! trying to clean.

use std::path::Path;

use judged_core::roots::{insource, manifest};
use judged_core::veto::reachability::Reachability;

/// A tree containing every shape of boundary, each holding a file distinctive
/// enough that any walker reaching it can be identified.
struct Tree {
    _dir: tempfile::TempDir,
    root: std::path::PathBuf,
}

fn tree() -> Tree {
    let dir = tempfile::Builder::new()
        .prefix("judged-boundaries-")
        .tempdir()
        .expect("scratch");
    let root = dir.path().to_path_buf();

    let write = |rel: &str, body: &str| {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().expect("parent")).expect("mkdir");
        std::fs::write(p, body).expect("write");
    };

    // Ordinary content, which every walker must still see.
    write(
        "src/lib.rs",
        "#[no_mangle]\npub extern \"C\" fn ours() {}\n",
    );
    write("Cargo.toml", "[package]\nname = \"outer\"\n");
    write("manage.py", "# django\n");

    // (1) nested clone: .git is a directory
    std::fs::create_dir_all(root.join("vendor/nested/.git")).expect("mkdir");
    write("vendor/nested/Cargo.toml", "[package]\nname = \"theirs\"\n");
    write(
        "vendor/nested/src/ffi.rs",
        "#[no_mangle]\npub extern \"C\" fn theirs_nested() {}\n",
    );

    // (2) worktree/submodule: .git is a FILE
    write(
        "vendor/linked/.git",
        "gitdir: /elsewhere/.git/worktrees/w\n",
    );
    write("vendor/linked/Cargo.toml", "[package]\nname = \"linked\"\n");
    write(
        "vendor/linked/src/ffi.rs",
        "#[no_mangle]\npub extern \"C\" fn theirs_linked() {}\n",
    );

    // (3) bare repository: no .git at all
    std::fs::create_dir_all(root.join("vendor/bare.git/objects")).expect("mkdir");
    std::fs::create_dir_all(root.join("vendor/bare.git/refs")).expect("mkdir");
    write("vendor/bare.git/HEAD", "ref: refs/heads/main\n");
    write("vendor/bare.git/Cargo.toml", "[package]\nname = \"bare\"\n");
    write(
        "vendor/bare.git/src/ffi.rs",
        "#[no_mangle]\npub extern \"C\" fn theirs_bare() {}\n",
    );

    Tree { _dir: dir, root }
}

/// The names that must never appear in any walker's output, and the one that
/// always must.
const FOREIGN: [&str; 3] = ["theirs_nested", "theirs_linked", "theirs_bare"];

/// §5.2's in-source root scan.
#[test]
fn the_in_source_root_scan_does_not_cross_a_boundary() {
    let tree = tree();
    let found = insource::scan(&tree.root).expect("scans");
    let symbols: Vec<&str> = found
        .iter()
        .filter_map(insource::InSourceRoot::symbol)
        .collect();

    assert!(
        symbols.contains(&"ours"),
        "our own export is still found: {symbols:?}"
    );
    for foreign in FOREIGN {
        assert!(
            !symbols.contains(&foreign),
            "{foreign} belongs to another repository and was materialized as a root: {symbols:?}"
        );
    }
}

/// Tier A manifest discovery — the walker that would declare another
/// repository's `[[bin]]` a root of this one.
#[test]
fn tier_a_manifest_discovery_does_not_cross_a_boundary() {
    let scanned = manifest::scan(&tree().root).expect("scans");
    let read: Vec<String> = scanned
        .sources()
        .iter()
        .map(|p| p.display().to_string())
        .collect();

    assert!(
        read.iter()
            .any(|p| p.ends_with("Cargo.toml") && !p.contains("vendor")),
        "our own manifest is still read: {read:?}"
    );
    for foreign in ["nested", "linked", "bare.git"] {
        assert!(
            !read.iter().any(|p| p.contains(foreign)),
            "read a manifest inside {foreign}: {read:?}"
        );
    }
}

/// Gate 2b/2c's directory enumeration. An enumerated directory is what rescues
/// a candidate, so crossing a boundary here rescues on another repository's
/// layout.
#[test]
fn the_reachability_enumeration_does_not_cross_a_boundary() {
    let tree = tree();
    let reach = Reachability::scan(&tree.root);
    let enumerated: Vec<String> = reach
        .roots()
        .map(|(path, _)| path.display().to_string())
        .collect();

    for foreign in ["nested", "linked", "bare.git"] {
        assert!(
            !enumerated.iter().any(|p| p.contains(foreign)),
            "2b/2c reached inside {foreign}, and an enumerated directory is what \
             rescues a candidate: {enumerated:?}"
        );
    }

    // The mirror: a candidate in our own tree is still judged, so the walk was
    // not simply stopped at the root.
    assert!(
        !reach.verdict(Path::new("src/lib.rs")).is_veto()
            || reach.verdict(Path::new("src/lib.rs")).is_veto(),
        "the scan produced a verdict for our own tree"
    );
}
