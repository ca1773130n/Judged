//! The two reference SUTs.
//!
//! [`RefusingSut`] is the negative control: it must never produce a false
//! removal, which proves the harness does not false-fail.
//!
//! [`NaiveSut`] is the positive control, and it is the more important of the
//! two. §9.8: *"if breaking the build does not break the gate, the gate is not a
//! gate."* Applied to the mutation suite, a deliberately naive cleaner that
//! passes would prove the fixtures had gone soft. These tests pin the exact bad
//! heuristic §7.5 documents in the shipped tools — basename-literal grep over
//! source files only, config and CI and markdown unparsed — so that the
//! positive control cannot quietly become competent through a well-meant edit.

use std::fs;
use std::path::{Path, PathBuf};

use judged_mutants::sut::{NaiveSut, RefusingSut, Sut, SutVerdict};
use tempfile::TempDir;

/// Build a throwaway repo from `(relative path, contents)` pairs.
fn repo(files: &[(&str, &str)]) -> TempDir {
    let dir = TempDir::new().expect("temp dir");
    for (rel, body) in files {
        let target = dir.path().join(rel);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).expect("mkdir");
        }
        fs::write(&target, body).expect("write");
    }
    dir
}

fn claimed_paths(verdict: &SutVerdict) -> Vec<String> {
    verdict
        .claimed_dead_paths
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect()
}

fn run_naive(dir: &Path) -> SutVerdict {
    NaiveSut.run(dir).expect("naive sut runs")
}

#[test]
fn refusing_sut_claims_nothing_even_in_a_repo_full_of_dead_files() {
    let dir = repo(&[
        ("src/main.rs", "fn main() {}\n"),
        ("src/orphan.rs", "pub fn never_called() {}\n"),
        ("dead/leftover.py", "def gone():\n    pass\n"),
    ]);

    let verdict = RefusingSut.run(dir.path()).expect("refusing sut runs");

    assert_eq!(RefusingSut.name(), "refusing");
    assert_eq!(
        verdict,
        SutVerdict::default(),
        "the negative control must claim nothing, ever"
    );
}

#[test]
fn naive_sut_claims_a_module_whose_only_reference_is_a_yaml_string() {
    // §10 E2 class 1, and the whole reason the positive control exists: the
    // reference is real, and it lives in a file the tool never opened.
    let dir = repo(&[
        ("app/main.py", "print('boot')\n"),
        (
            "app/tasks/nightly.py",
            "class NightlyTask:\n    def execute(self):\n        return 1\n",
        ),
        (
            "celery.yaml",
            "beat_schedule:\n  roll:\n    task: app.tasks.nightly.NightlyTask\n",
        ),
    ]);

    let verdict = run_naive(dir.path());

    assert!(
        claimed_paths(&verdict).contains(&"app/tasks/nightly.py".to_string()),
        "naive heuristic must miss the YAML reference; got {:?}",
        claimed_paths(&verdict)
    );
    assert!(
        verdict
            .claimed_dead_symbols
            .contains(&"NightlyTask".to_string()),
        "the class is named only in YAML, so a textual scan sees one occurrence; got {:?}",
        verdict.claimed_dead_symbols
    );
}

#[test]
fn naive_sut_spares_a_module_referenced_from_a_file_it_does_parse() {
    // The control has to be naive, not broken. If it claimed everything dead it
    // would fail every mutant for the wrong reason and prove nothing about the
    // fixtures.
    let dir = repo(&[
        (
            "app/main.py",
            "from app.tasks import nightly\n\nnightly.NightlyTask().execute()\n",
        ),
        (
            "app/tasks/nightly.py",
            "class NightlyTask:\n    def execute(self):\n        return 1\n",
        ),
    ]);

    let verdict = run_naive(dir.path());

    assert!(
        !claimed_paths(&verdict).contains(&"app/tasks/nightly.py".to_string()),
        "a plain in-source import must be seen; got {:?}",
        claimed_paths(&verdict)
    );
    assert!(
        !verdict
            .claimed_dead_symbols
            .contains(&"NightlyTask".to_string()),
        "a symbol used in another source file must not be claimed"
    );
}

#[test]
fn naive_sut_does_not_parse_ci_manifests_dockerfiles_or_markdown() {
    // §10 E2 classes 8 and 9. §7.5 records the same blind spot in the shipped
    // tools: grahama1970's SKIP_DIRS excludes build config from the reference
    // scan, and NickCrew's whole reference check is `grep "from './FILE'"`.
    let dir = repo(&[
        ("scripts/main.py", "print('entry')\n"),
        ("scripts/migrate.py", "def migrate():\n    pass\n"),
        ("scripts/smoke.py", "def smoke():\n    pass\n"),
        (
            ".github/workflows/ci.yml",
            "jobs:\n  run:\n    steps:\n      - run: python scripts/migrate.py\n",
        ),
        (
            "Dockerfile",
            "COPY scripts/smoke.py /app/\nRUN python /app/smoke.py\n",
        ),
        ("README.md", "```sh\npython scripts/migrate.py\n```\n"),
    ]);

    let claimed = claimed_paths(&run_naive(dir.path()));

    assert!(
        claimed.contains(&"scripts/migrate.py".to_string()),
        "CI and README references are invisible to the naive heuristic; got {claimed:?}"
    );
    assert!(
        claimed.contains(&"scripts/smoke.py".to_string()),
        "Dockerfile references are invisible to the naive heuristic; got {claimed:?}"
    );
}

#[test]
fn naive_sut_spares_conventional_entry_points() {
    // Every shipped tool has an entry-point notion; a control that lacked one
    // would be a strawman rather than a faithful reproduction of §7.5.
    let dir = repo(&[
        ("main.py", "pass\n"),
        ("src/lib.rs", "pub mod nothing;\n"),
        ("pkg/index.ts", "export {};\n"),
        ("pkg/orphan.ts", "export const x = 1;\n"),
    ]);

    let claimed = claimed_paths(&run_naive(dir.path()));

    for entry in ["main.py", "src/lib.rs", "pkg/index.ts"] {
        assert!(
            !claimed.contains(&entry.to_string()),
            "{entry} is a conventional entry point; got {claimed:?}"
        );
    }
    assert!(
        claimed.contains(&"pkg/orphan.ts".to_string()),
        "a genuinely unreferenced module must still be claimed; got {claimed:?}"
    );
}

#[test]
fn naive_sut_claims_an_exported_symbol_with_no_in_repo_caller() {
    // §10 E2 class 19: unfalsifiable from inside the repo by construction, so a
    // textual scan is guaranteed to get it wrong.
    let dir = repo(&[
        ("src/lib.rs", "pub mod abi;\n"),
        (
            "src/abi.rs",
            "#[no_mangle]\npub extern \"C\" fn judged_probe() -> i32 {\n    7\n}\n",
        ),
    ]);

    let verdict = run_naive(dir.path());

    assert!(
        verdict
            .claimed_dead_symbols
            .contains(&"judged_probe".to_string()),
        "an ABI export has no in-repo caller; got {:?}",
        verdict.claimed_dead_symbols
    );
}

#[test]
fn naive_sut_ignores_the_git_directory() {
    // Object files and packed refs contain arbitrary bytes, including the names
    // of files that really are dead. Treating them as references would make the
    // control accidentally safe.
    let dir = repo(&[
        ("src/main.rs", "fn main() {}\n"),
        ("src/orphan.rs", "pub fn gone() {}\n"),
        (".git/HEAD", "ref: refs/heads/main\n"),
        (".git/loose.rs", "orphan orphan orphan gone\n"),
    ]);

    let claimed = claimed_paths(&run_naive(dir.path()));

    assert!(
        claimed.contains(&"src/orphan.rs".to_string()),
        "history must not count as a live reference; got {claimed:?}"
    );
    assert!(
        !claimed.iter().any(|p| p.starts_with(".git/")),
        "nothing inside .git is a removal candidate; got {claimed:?}"
    );
}

#[test]
fn naive_sut_output_is_sorted_and_free_of_duplicates() {
    let dir = repo(&[
        ("main.py", "pass\n"),
        ("z_orphan.py", "def z_thing():\n    pass\n"),
        ("a_orphan.py", "def a_thing():\n    pass\n"),
        ("m_orphan.py", "def m_thing():\n    pass\n"),
    ]);

    let verdict = run_naive(dir.path());
    let claimed: Vec<PathBuf> = verdict.claimed_dead_paths.clone();
    let mut sorted = claimed.clone();
    sorted.sort();
    sorted.dedup();

    assert_eq!(
        claimed, sorted,
        "a report that reorders between runs cannot be diffed in CI"
    );

    let mut symbols = verdict.claimed_dead_symbols.clone();
    symbols.sort();
    symbols.dedup();
    assert_eq!(verdict.claimed_dead_symbols, symbols);

    // Same repo twice must give the same answer.
    assert_eq!(verdict, run_naive(dir.path()));
}
