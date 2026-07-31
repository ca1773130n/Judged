//! Class 5 — an error-handling module reached only on failure *(debloat Issue 5)*.
//!
//! **The mechanism.** `ledger/writer.py` appends a line to a file. The only
//! reference in the repository to `ledger/recovery.py` and its
//! `quarantine_partial_write` sits inside the `except OSError` branch of that
//! one function — the import statement itself is written there, which is how
//! the shape occurs in the wild. On a host whose filesystem is working, the
//! branch never executes.
//!
//! **Why every other signal misses it.** This class is not aimed at the import
//! graph, which sees the reference plainly. It is aimed at every signal derived
//! from *execution*. §3.4 measured the consequence: dynamic debloaters falsely
//! removed up to 94% of must-retain code, and Issue 5 is the specific finding —
//! "essential error logging and handling functions are frequently removed since
//! they are very difficult to exercise with test cases". The suite shipped in
//! this fixture covers `append_entry` end to end and never once reaches the
//! handler, so coverage, a tests-still-pass oracle and a production tracer all
//! agree the module is dead. They are all wrong, and the day they are proven
//! wrong is the day the disk is already full.
//!
//! **What is supposed to catch it.** Nothing dynamic — that is the point. A
//! tool must treat "never observed executing" as *absence of evidence* rather
//! than as evidence of death (§3.4, "tests are not usage"), and fall back on
//! the static reference it can plainly see. The tests below pin the two
//! properties that make that the only correct reading: the sole reference lives
//! in a failure branch, and no test in the repository names it.

use std::path::{Path, PathBuf};

use judged_core::git::Repo;
use judged_core::{Error, Result};

use crate::mutant::{Ecosystem, GroundTruth, Mutant};

/// Coverage-guided debloaters removed exactly this and shipped it. The
/// handler runs on the day the system is already broken, which is the worst
/// possible day to discover it was deleted.
pub struct ErrorPathOnly;

/// Repo-relative path of the artifact that is alive and looks dead.
const LIVE: &str = "ledger/recovery.py";

/// The symbol inside [`LIVE`] that the failure branch calls.
const LIVE_SYMBOL: &str = "quarantine_partial_write";

/// The one file in the repository that names [`LIVE_SYMBOL`], and it does so
/// only from inside `except OSError`.
const MECHANISM: &str = "ledger/writer.py";

/// The line in [`MECHANISM`] that opens the failure branch. Every reference to
/// the live symbol must appear after it; the test asserts exactly that, so the
/// fixture cannot decay into an ordinary top-level import.
const FAILURE_BRANCH: &str = "except OSError as cause:";

/// Files written into the mutant repository, as `(repo-relative path, body)`.
const FILES: &[(&str, &str)] = &[
    (
        "pyproject.toml",
        r#"[project]
name = "ledger"
version = "0.1.0"
requires-python = ">=3.11"

[build-system]
requires = ["setuptools>=68"]
build-backend = "setuptools.build_meta"
"#,
    ),
    ("ledger/__init__.py", "\"\"\"Append-only ledger.\"\"\"\n"),
    (
        MECHANISM,
        r#""""The ledger writer: reachable, exercised, fully covered.

Importing the recovery path inside the except branch is deliberate and
idiomatic -- it is expensive to import and unwanted on the happy path. It is
also what puts the module below out of reach of every execution-derived signal.
"""

from pathlib import Path


def append_entry(ledger: Path, entry: str) -> int:
    """Append one entry; return the ledger's new size in bytes."""
    try:
        with ledger.open("a", encoding="utf-8") as handle:
            handle.write(entry + "\n")
        return ledger.stat().st_size
    except OSError as cause:
        # Reached only when the filesystem refuses the write: ENOSPC, EROFS, a
        # revoked NFS handle. No practical test suite produces those, which is
        # debloat Issue 5 in one line.
        from ledger.recovery import quarantine_partial_write

        return quarantine_partial_write(ledger, cause)
"#,
    ),
    (
        LIVE,
        r#""""LIVE. Called only from the except branch of ledger.writer.

Deleting this module breaks no import at collection time, fails no test, and
changes no covered line. It turns a full disk from a logged, recoverable
incident into a truncated ledger plus a traceback inside the handler that was
supposed to clean it up.
"""

from pathlib import Path


def quarantine_partial_write(ledger: Path, cause: OSError) -> int:
    """Move a half-written ledger aside so the next append starts clean.

    Returns the number of bytes rescued, or -1 when even the rescue failed.
    """
    quarantined = ledger.with_name(ledger.name + ".partial")
    try:
        rescued = ledger.stat().st_size
        ledger.replace(quarantined)
    except OSError:
        return -1
    print(f"rescued {rescued} bytes into {quarantined}: {cause}")
    return rescued
"#,
    ),
    (
        "tests/test_writer.py",
        r#""""The whole test suite. It covers append_entry end to end.

There is no test for the failure branch, and writing one would need a
read-only mount or a genuinely full filesystem. That is not an oversight in
the fixture; it is the finding the fixture encodes (§3.4 Issue 5).
"""

from pathlib import Path

from ledger.writer import append_entry


def test_append_entry_grows_the_ledger(tmp_path: Path) -> None:
    ledger = tmp_path / "ledger.txt"
    assert append_entry(ledger, "opened") == 7
    assert append_entry(ledger, "closed") == 14
"#,
    ),
    (
        "ledger/legacy_fixed_width.py",
        r#""""DEAD DECOY. The fixed-width export was replaced by CSV two years ago.
Nothing imports it, no string names it, no branch reaches it.
"""


def format_row(fields: list[str], width: int = 12) -> str:
    return "".join(field.ljust(width) for field in fields)
"#,
    ),
    (
        "ledger/unused_locale_map.py",
        r#""""DEAD DECOY. A second one on purpose: decoy recall is a rate, and one
decoy cannot tell a tool that reasoned from a tool that guessed once.
"""

DECIMAL_COMMA = {"de", "fr", "es"}
"#,
    ),
];

impl ErrorPathOnly {
    /// Repo-relative paths of the genuinely-dead files planted here.
    const DECOYS: [&'static str; 2] = [
        "ledger/legacy_fixed_width.py",
        "ledger/unused_locale_map.py",
    ];
}

impl Mutant for ErrorPathOnly {
    fn id(&self) -> &str {
        "m05"
    }
    fn ecosystem(&self) -> Ecosystem {
        Ecosystem::Python
    }
    fn mechanism(&self) -> &str {
        "recovery handler reached only on the failure path"
    }
    fn research_ref(&self) -> &str {
        "§10 E2 class 5"
    }
    fn materialize(&self, dir: &Path) -> Result<GroundTruth> {
        let repo = Repo::init(dir)?;
        for (rel, body) in FILES {
            let path = repo.root().join(rel);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(|source| Error::Io {
                    path: parent.to_path_buf(),
                    source,
                })?;
            }
            std::fs::write(&path, body).map_err(|source| Error::Io { path, source })?;
        }
        // Committed, not merely written: recoverability class (§8.1, Gate 0g)
        // is part of what the suite exercises, and an uncommitted fixture would
        // model every file as `Untracked` — the class with no recovery path.
        repo.add_all()?;
        repo.commit("m05: ledger writer whose recovery module only the failure branch reaches")?;

        Ok(GroundTruth {
            live_paths: vec![PathBuf::from(LIVE)],
            live_symbols: vec![LIVE_SYMBOL.to_string()],
            decoy_dead_paths: Self::DECOYS.iter().map(PathBuf::from).collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use judged_core::git::Repo;
    use std::process::Command;

    /// Every file in `root` whose bytes contain `needle`, repo-relative.
    ///
    /// Deliberately `git grep --fixed-strings`: the claim under test is about
    /// what a *plain textual search* can find, so the check has to be a plain
    /// textual search and not a smarter one. `git grep` also skips `.git/`,
    /// where the committed blobs would otherwise match everything.
    fn files_mentioning(root: &Path, needle: &str) -> Vec<String> {
        let output = Command::new("git")
            .args(["grep", "-I", "-l", "--untracked", "--fixed-strings", needle])
            .current_dir(root)
            .output()
            .expect("git grep should run inside a materialized fixture");
        String::from_utf8(output.stdout)
            .expect("fixture files are UTF-8")
            .lines()
            .map(str::to_string)
            .collect()
    }

    fn materialize_into_tempdir() -> (tempfile::TempDir, GroundTruth) {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let truth = ErrorPathOnly
            .materialize(dir.path())
            .expect("m05 materializes");
        (dir, truth)
    }

    #[test]
    fn m05_is_a_real_git_repository_whose_live_artifact_is_committed() {
        let (dir, _truth) = materialize_into_tempdir();
        let repo = Repo::discover(dir.path()).expect("fixture is a git working tree");

        // A blob SHA at HEAD exists only if a commit contains it, so this
        // asserts "real repository" and "one commit" together. Recoverability
        // class (Gate 0g) is part of what the suite exercises, so it matters
        // that the fixture is committed rather than merely initialised.
        assert!(
            repo.blob_sha(Path::new(LIVE))
                .expect("blob_sha query succeeds")
                .is_some(),
            "{LIVE} must be present in HEAD"
        );
    }

    #[test]
    fn m05_ground_truth_names_files_that_are_really_there() {
        let (dir, truth) = materialize_into_tempdir();

        assert_eq!(truth.live_paths, vec![Path::new(LIVE).to_path_buf()]);
        assert_eq!(truth.live_symbols, vec![LIVE_SYMBOL.to_string()]);
        assert_eq!(truth.decoy_dead_paths.len(), ErrorPathOnly::DECOYS.len());

        for path in truth.live_paths.iter().chain(&truth.decoy_dead_paths) {
            assert!(
                dir.path().join(path).is_file(),
                "ground truth names {} but it is not on disk",
                path.display()
            );
        }
    }

    #[test]
    fn m05_the_handler_is_named_only_inside_the_failure_branch() {
        let (dir, _truth) = materialize_into_tempdir();

        // One caller, and it is not a test. If a second file ever names the
        // handler, the mutant has stopped testing one mechanism.
        assert_eq!(
            files_mentioning(dir.path(), LIVE_SYMBOL),
            vec![LIVE.to_string(), MECHANISM.to_string()],
            "only the definition and the one failure branch may name the handler"
        );

        // And the reference is genuinely below the `except`, not a top-level
        // import that merely happens to share a file with one. This is what
        // keeps the handler off the happy path, and therefore out of coverage.
        let source = std::fs::read_to_string(dir.path().join(MECHANISM))
            .expect("mechanism file is readable");
        let branch = source
            .find(FAILURE_BRANCH)
            .expect("the mechanism file must open a failure branch");
        let reference = source
            .find(&format!("import {LIVE_SYMBOL}"))
            .expect("the mechanism file must import the handler");
        assert!(
            reference > branch,
            "the handler must be referenced only after `{FAILURE_BRANCH}`"
        );
    }

    #[test]
    fn m05_no_test_in_the_repository_can_reach_the_handler() {
        let (dir, _truth) = materialize_into_tempdir();

        // §3.4's "tests are not usage" runs in this direction too: the suite
        // must not accidentally cover the handler, or the mutant would be
        // solvable by running the tests — precisely the signal the debloating
        // study showed to be unsound.
        for file in files_mentioning(dir.path(), LIVE_SYMBOL) {
            assert!(
                !file.starts_with("tests/"),
                "{file} reaches the handler; the suite must not be able to"
            );
        }
    }

    #[test]
    fn m05_decoys_are_named_nowhere_at_all() {
        let (dir, truth) = materialize_into_tempdir();

        for decoy in &truth.decoy_dead_paths {
            let stem = decoy
                .file_stem()
                .and_then(|s| s.to_str())
                .expect("decoy has a UTF-8 stem");
            let mentions = files_mentioning(dir.path(), stem);
            assert!(
                mentions.iter().all(|f| Path::new(f) == decoy),
                "a decoy that anything mentions is not a decoy; {stem} appears in {mentions:?}"
            );
        }
    }
}
