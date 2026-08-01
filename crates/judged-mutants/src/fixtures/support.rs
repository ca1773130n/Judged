//! Shared scaffolding for the nineteen E2 fixture test modules.
//!
//! Every fixture asks the same three questions of the repository it just
//! materialized — *is it a real committed repo*, *is its declared ground truth
//! actually on disk*, and *is each decoy really unreferenced* — plus one or two
//! hardness assertions unique to its class. The first three, and the search
//! primitives the hardness assertions are written in, live here so that there
//! is exactly one implementation of each to read, audit and fix.
//!
//! # The two search primitives, and why there are two
//!
//! The fixtures were written against two incompatible whole-repo searches, and
//! the difference is load-bearing rather than stylistic:
//!
//! - [`tree`] walks the working tree and yields **raw bytes** for every file.
//!   This is the whole-repo literal veto §6.2 calls mandatory — *"whole-repo
//!   literal veto over every file type (§6.20)"* — and it is what the veto has
//!   to be, because a path or a symbol name survives into a compiled artifact,
//!   a pickle, or a bundled asset unchanged. It sees ignored files and binary
//!   files, and it never fails on either.
//! - [`files_mentioning`] shells out to `git grep -I --fixed-strings`, which
//!   skips binary files and ignored files. That is *not* a weaker version of
//!   the same thing: it is the model of the plain textual search a mutant is
//!   claimed to defeat. Assertions of the form "nothing in the repository
//!   spells this filename" are statements about what a text search can find,
//!   so they are written with the text search.
//!
//! A third dialect existed and is gone: a tree walk that read every file with
//! `read_to_string(..).expect("fixture is UTF-8")`. It panics on the first
//! binary file, so a fixture that grows one (m16 already ships a pickle) would
//! take the suite down rather than answer the question — and, worse, a `-I`
//! style skip written as a panic-or-nothing is exactly §6.20's failure mode:
//! *"no data" must be a distinct state from "zero executions," and it must
//! never flow into a deadness score.* Bytes are the honest representation, so
//! [`tree`] returns bytes and callers that want text semantics get
//! [`mentions`] and [`occurrences`] over them.

use std::path::Path;
use std::process::Command;

use judged_core::git::Repo;
use tempfile::TempDir;

use crate::mutant::{GroundTruth, Mutant};

/// Materialize `mutant` into a throwaway directory and open the repository it
/// built.
///
/// The returned [`TempDir`] owns the fixture: bind it (`let (_dir, repo, ..)`),
/// because dropping it deletes the tree the other two values describe.
pub(super) fn materialize(mutant: &dyn Mutant) -> (TempDir, Repo, GroundTruth) {
    let dir = TempDir::new().expect("create a tempdir for the fixture");
    let truth = mutant
        .materialize(dir.path())
        .unwrap_or_else(|error| panic!("{} materializes: {error}", mutant.id()));
    let repo = Repo::discover(dir.path()).expect("a materialized fixture is a git working tree");
    (dir, repo, truth)
}

/// Every file in the working tree except `.git`, as `(repo-relative path,
/// bytes)`, sorted by path.
///
/// Bytes, not text: see the module documentation. This is the primitive for
/// "nothing *anywhere* names X", including inside files a text search skips.
pub(super) fn tree(root: &Path) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

fn walk(dir: &Path, root: &Path, out: &mut Vec<(String, Vec<u8>)>) {
    for entry in std::fs::read_dir(dir).expect("read fixture directory") {
        let path = entry.expect("read directory entry").path();
        if path.file_name().is_some_and(|name| name == ".git") {
            continue;
        }
        if path.is_dir() {
            walk(&path, root, out);
        } else {
            let rel = path
                .strip_prefix(root)
                .expect("path is under the fixture root")
                .to_string_lossy()
                .into_owned();
            out.push((rel, std::fs::read(&path).expect("read fixture file")));
        }
    }
}

/// Whether `needle` occurs anywhere in `haystack`, byte for byte.
///
/// Panics on an empty needle rather than answering `false`: see
/// [`reject_empty`].
pub(super) fn mentions(haystack: &[u8], needle: &str) -> bool {
    reject_empty(needle);
    let needle = needle.as_bytes();
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

/// How many non-overlapping times `needle` occurs in `haystack`.
///
/// Non-overlapping so that this counts what `str::matches(..).count()` counts;
/// the fixtures that use it are pinning an exact occurrence count.
pub(super) fn occurrences(haystack: &[u8], needle: &str) -> usize {
    reject_empty(needle);
    let needle = needle.as_bytes();
    let mut count = 0;
    let mut at = 0;
    while at + needle.len() <= haystack.len() {
        if &haystack[at..at + needle.len()] == needle {
            count += 1;
            at += needle.len();
        } else {
            at += 1;
        }
    }
    count
}

/// Refuse a blank search term.
///
/// Every needle in this catalogue is a filename stem, a symbol, or a literal
/// the fixture wrote itself, so a blank one means the caller derived it wrong.
/// Answering "found nothing" would turn every "nothing in the repository names
/// this" assertion into one that cannot fail — the exact defect these fixtures
/// exist to catch in other tools, so it fails loudly here instead.
fn reject_empty(needle: &str) {
    assert!(!needle.is_empty(), "empty needle: nothing to search for");
}

/// The same test `git grep` applies before refusing to search a file as text:
/// a NUL byte near the start.
///
/// A fixture that asserts "a text search cannot see this" has to prove the file
/// really is one a text search skips, rather than assume it.
pub(super) fn looks_binary(bytes: &[u8]) -> bool {
    bytes.iter().take(8000).any(|byte| *byte == 0)
}

/// Every file in `root` a plain textual search would report as containing
/// `needle`, repo-relative.
///
/// Deliberately `git grep --fixed-strings`: the claim under test is that the
/// artifact survives *a plain textual search*, so the check has to be a plain
/// textual search and not a smarter one. `git grep` also skips `.git/`, where
/// the committed blobs would otherwise match everything.
pub(super) fn files_mentioning(root: &Path, needle: &str) -> Vec<String> {
    let output = Command::new("git")
        .args(["grep", "-I", "-l", "--untracked", "--fixed-strings", needle])
        .current_dir(root)
        .output()
        .expect("git grep should run inside a materialized fixture");
    String::from_utf8(output.stdout)
        .expect("git prints repo-relative paths as UTF-8")
        .lines()
        .map(str::to_string)
        .collect()
}

/// [`files_mentioning`], minus the artifact's own file.
pub(super) fn references_outside(root: &Path, needle: &str, artifact: &str) -> Vec<String> {
    files_mentioning(root, needle)
        .into_iter()
        .filter(|hit| hit != artifact)
        .collect()
}

/// Assert the fixture is a real repository with `paths` in `HEAD`.
///
/// `HEAD` rather than the index: recoverability class is part of what the suite
/// exercises, and a file that was staged but never committed has a different
/// one. A blob SHA at `HEAD` exists only if a commit contains it, so this
/// asserts "real repository" and "committed" together.
pub(super) fn assert_committed(repo: &Repo, paths: &[&str]) {
    assert!(
        repo.root().join(".git").is_dir(),
        "expected a git directory at {}",
        repo.root().display()
    );
    for path in paths {
        assert!(
            repo.blob_sha(Path::new(path))
                .expect("blob_sha query succeeds")
                .is_some(),
            "{path} must be present in HEAD; the fixture must be committed, not \
             just written to disk"
        );
    }
}

/// Assert every path the ground truth declares is a repo-relative file that is
/// really there, and that there is at least one decoy.
///
/// The decoy check is the suite's own positive control (§3.7): without a
/// genuinely-dead file planted beside the live one, a tool that refuses to call
/// anything dead scores a perfect run and looks indistinguishable from a tool
/// that works.
pub(super) fn assert_ground_truth_is_on_disk(repo: &Repo, truth: &GroundTruth) {
    assert!(
        !truth.decoy_dead_paths.is_empty(),
        "without a decoy, a tool that claims nothing passes this mutant for free"
    );
    for path in truth.live_paths.iter().chain(&truth.decoy_dead_paths) {
        assert!(path.is_relative(), "{path:?} must be repo-relative");
        assert!(
            repo.root().join(path).is_file(),
            "ground truth names {} but it is not on disk",
            path.display()
        );
    }
    assert_decoy_symbols_are_well_formed(repo.root(), truth);
}

/// Assert the decoys' *symbol* route is declared, paired, and real.
///
/// A decoy is found when a tool claims its path **or** a symbol it defines. The
/// second half only measures anything if every fixture declares it, so this is
/// the guard that a new fixture cannot quietly ship without one: a decoy with no
/// declared symbol is invisible to every symbol-level analyzer, and its zero
/// then reads as "found nothing" rather than "was never asked" (§6.20).
///
/// What is deliberately *not* asserted: that a decoy symbol appears in no other
/// file. The equivalent statement about a decoy's filename is
/// [`assert_decoys_are_unreferenced`], and it works because a filename is a
/// distinctive literal. A symbol name is not — m18's decoy defines `export`,
/// which is also a substring of `android:exported` in the manifest next door.
/// A byte search cannot tell a symbol from a prefix of an XML attribute, so
/// asserting it would be asserting something the search cannot actually check.
/// Grading matches whole symbol segments, which is where that distinction
/// belongs.
fn assert_decoy_symbols_are_well_formed(root: &Path, truth: &GroundTruth) {
    // Index-aligned, so a short list is not "the rest have no symbol": it is a
    // fixture that stopped declaring, and its later decoys silently become
    // unreachable to every symbol-level tool.
    assert_eq!(
        truth.decoy_dead_symbols.len(),
        truth.decoy_dead_paths.len(),
        "declares {} decoys but {} decoy symbols; the two lists are paired by \
         index, and `\"\"` is how a decoy with no symbol route says so",
        truth.decoy_dead_paths.len(),
        truth.decoy_dead_symbols.len(),
    );

    let mut seen = std::collections::BTreeSet::new();
    for path in &truth.decoy_dead_paths {
        assert!(
            seen.insert(path.as_path()),
            "decoy {} is declared twice; recall counts decoy files, so a \
             duplicate inflates both halves of the rate and changes what it means",
            path.display()
        );
    }

    for (path, symbol) in truth
        .decoy_dead_paths
        .iter()
        .zip(truth.decoy_dead_symbols.iter())
    {
        // `""` is a decoy with no symbol route — a bash script, an nginx
        // config, a minified bundle. There is nothing to pair it against.
        if symbol.is_empty() {
            continue;
        }

        // The pairing is positional, so nothing about the types catches a
        // reordered array. Reading the file is what does.
        let bytes = std::fs::read(root.join(path)).expect("read the decoy named by ground truth");
        assert!(
            mentions(&bytes, symbol),
            "decoy symbol {symbol:?} does not appear in the decoy it is paired \
             with ({}); `decoy_dead_symbols` is index-aligned with \
             `decoy_dead_paths`",
            path.display()
        );

        assert!(
            !truth.live_symbols.iter().any(|live| live == symbol),
            "{symbol:?} is declared both live and dead; one claim would then be \
             a false removal and a decoy find at once, and the gate's two \
             numbers would contradict each other"
        );
    }
}

/// Assert no file anywhere in the working tree names a decoy's stem.
///
/// A decoy some mechanism secretly reaches is not dead, and grading a correct
/// refusal as a miss would make the whole run meaningless. Searched over raw
/// bytes and over every file — including ignored and binary ones — because a
/// reference that only survives in a compiled or encoded file is still a
/// reference (§6.2).
pub(super) fn assert_decoys_are_unreferenced(repo: &Repo, truth: &GroundTruth) {
    let tree = tree(repo.root());
    for decoy in &truth.decoy_dead_paths {
        let stem = decoy
            .file_stem()
            .and_then(|stem| stem.to_str())
            .expect("decoy has a UTF-8 stem");
        for (path, bytes) in &tree {
            if Path::new(path) == decoy.as_path() {
                continue;
            }
            assert!(
                !mentions(bytes, stem),
                "{path} references the decoy {stem:?}, so it is not dead"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An empty needle is a caller bug — a stem or a live symbol that came out
    /// blank. Answering "no, nothing mentions it" would let every "nothing
    /// names this" assertion in the catalogue pass vacuously, which is the one
    /// failure this module exists to prevent (AGENTS.md rule 12, "Fail
    /// Loudly"; the same discipline §6.20 demands of an analyzer that finds no
    /// data).
    #[test]
    #[should_panic(expected = "empty needle")]
    fn mentions_refuses_an_empty_needle() {
        mentions(b"the haystack is irrelevant", "");
    }

    #[test]
    #[should_panic(expected = "empty needle")]
    fn occurrences_refuses_an_empty_needle() {
        occurrences(b"the haystack is irrelevant", "");
    }

    /// A throwaway tree containing `files`, for the ground-truth shape tests.
    fn on_disk(files: &[(&str, &str)]) -> TempDir {
        let dir = TempDir::new().expect("create a tempdir");
        for (relative, body) in files {
            let path = dir.path().join(relative);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("create parent directory");
            }
            std::fs::write(&path, body).expect("write file");
        }
        dir
    }

    fn decoy_truth(decoys: &[(&str, &str)], live_symbols: &[&str]) -> GroundTruth {
        GroundTruth {
            live_paths: Vec::new(),
            live_symbols: live_symbols.iter().map(|s| (*s).to_string()).collect(),
            decoy_dead_paths: decoys
                .iter()
                .map(|(path, _)| std::path::PathBuf::from(path))
                .collect(),
            decoy_dead_symbols: decoys
                .iter()
                .map(|(_, symbol)| (*symbol).to_string())
                .collect(),
        }
    }

    #[test]
    fn a_well_formed_decoy_declaration_is_accepted() {
        // Both shapes the catalogue actually uses: a decoy with a symbol route,
        // and one without. `""` is a declaration that no symbol-level tool can
        // reach this file — a bash script, an nginx config — and not a fixture
        // that forgot.
        let dir = on_disk(&[
            ("app/dead.py", "def hang_indent(text):\n    return text\n"),
            ("scripts/old.sh", "#!/usr/bin/env bash\necho hi\n"),
        ]);
        assert_decoy_symbols_are_well_formed(
            dir.path(),
            &decoy_truth(
                &[("app/dead.py", "hang_indent"), ("scripts/old.sh", "")],
                &["LiveThing"],
            ),
        );
    }

    #[test]
    #[should_panic(expected = "declares 2 decoys but 1 decoy symbol")]
    fn a_decoy_with_no_entry_at_all_is_a_fixture_bug() {
        // The lists are index-aligned, so a short one is not "the rest have no
        // symbol" — it is a fixture that stopped declaring halfway, and every
        // decoy past the end silently becomes unreachable to a symbol-level
        // tool. Loud, because the symptom is a number that is merely lower.
        let dir = on_disk(&[("app/dead.py", "def x():\n    pass\n"), ("app/gone.py", "")]);
        let mut truth = decoy_truth(&[("app/dead.py", "x"), ("app/gone.py", "y")], &[]);
        truth.decoy_dead_symbols.pop();
        assert_decoy_symbols_are_well_formed(dir.path(), &truth);
    }

    #[test]
    #[should_panic(expected = "does not appear in the decoy it is paired with")]
    fn a_symbol_paired_with_the_wrong_decoy_is_a_fixture_bug() {
        // What a reordered array looks like. The pairing is positional, so
        // nothing about the types catches it; without this check the fixture
        // would compile, run, and credit the wrong file.
        let dir = on_disk(&[
            ("app/dead.py", "def hang_indent(text):\n    return text\n"),
            ("app/gone.py", "def to_hex(rgb):\n    return rgb\n"),
        ]);
        assert_decoy_symbols_are_well_formed(
            dir.path(),
            &decoy_truth(
                &[("app/dead.py", "to_hex"), ("app/gone.py", "hang_indent")],
                &[],
            ),
        );
    }

    #[test]
    #[should_panic(expected = "is declared both live and dead")]
    fn a_symbol_that_is_both_live_and_a_decoy_is_a_fixture_bug() {
        // The two halves of the grade would then disagree about the same claim:
        // it is a false removal *and* a decoy find. A gate whose inputs
        // contradict each other reports a number nobody can act on.
        let dir = on_disk(&[("app/dead.py", "def render(x):\n    return x\n")]);
        assert_decoy_symbols_are_well_formed(
            dir.path(),
            &decoy_truth(&[("app/dead.py", "render")], &["render"]),
        );
    }

    #[test]
    #[should_panic(expected = "is declared twice")]
    fn the_same_decoy_declared_twice_is_a_fixture_bug() {
        // Recall counts decoy files, so a duplicate inflates the denominator and
        // the numerator together and quietly changes what the rate means.
        let dir = on_disk(&[("app/dead.py", "def x():\n    pass\n")]);
        assert_decoy_symbols_are_well_formed(
            dir.path(),
            &decoy_truth(&[("app/dead.py", "x"), ("app/dead.py", "x")], &[]),
        );
    }
}
