//! Gate 2a — the whole-repo literal veto (§9.3).
//!
//! Four properties decide whether this layer is sound, and each has its own
//! test below:
//!
//! 1. **Self-reference is excluded.** A file must not veto itself and a symbol
//!    must not be vetoed by its own definition site, or the veto degenerates
//!    into a constant function and nothing is ever removable.
//! 2. **An incomplete search is a HIT.** Only a search that provably completed
//!    over the whole corpus and found nothing may answer "no veto" (§6.20 —
//!    Meta's truncated BigGrep read as "no references" turned the safety net
//!    into the deletion trigger).
//! 3. **Binaries are searched.** Path and symbol strings survive compilation
//!    and pickling; E2 class m16 hides a live class name inside an on-disk
//!    pickle and this is the only route by which it is rescuable.
//! 4. **The needle strategy is selectable and measurable** (§11 R8), because
//!    "block on any hit" and a flag-rate budget are stated as conflicting
//!    requirements and only measurement resolves them.

use std::path::{Path, PathBuf};
use std::time::Duration;

use judged_core::git::Repo;
use judged_core::veto::literal::{
    Candidate, LiteralVeto, NeedleKind, NeedleStrategy, ScanLimits, ScanState, Verdict, VetoReason,
};
use tempfile::TempDir;

/// Build a real git repository holding `files`, committed.
///
/// Real, because recoverability and index membership are part of what Gate 2
/// reads: the corpus is "every *tracked* file", which only a repository with an
/// index can answer.
fn repo_with(files: &[(&str, &[u8])]) -> (TempDir, Repo) {
    let dir = TempDir::new().expect("tempdir");
    let repo = Repo::init(dir.path()).expect("git init");
    for (rel, bytes) in files {
        let path = repo.root().join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("mkdir -p");
        }
        std::fs::write(&path, bytes).expect("write fixture file");
    }
    repo.add_all().expect("git add --all");
    repo.commit("fixture").expect("git commit");
    (dir, repo)
}

/// Every hit as `(file, needle kind, needle text)`, sorted, so assertions do
/// not depend on corpus traversal order.
fn hit_set(verdict: &Verdict) -> Vec<(String, NeedleKind, String)> {
    let mut hits: Vec<(String, NeedleKind, String)> = verdict
        .report()
        .hits()
        .iter()
        .map(|hit| {
            (
                hit.file().to_string_lossy().into_owned(),
                hit.needle().kind(),
                hit.needle().text().to_string(),
            )
        })
        .collect();
    hits.sort();
    hits
}

fn assert_clear(verdict: &Verdict) {
    match verdict {
        Verdict::Clear { report, .. } => {
            assert_eq!(
                report.state(),
                &ScanState::Completed,
                "a Clear verdict may only ever come from a completed scan"
            );
            assert!(
                report.hits().is_empty(),
                "a Clear verdict may not carry hits: {:?}",
                report.hits()
            );
        }
        Verdict::Vetoed { reason, report, .. } => {
            panic!("expected no veto, got {reason:?} over {report:?}")
        }
    }
}

// ---------------------------------------------------------------------------
// The basic direction: a literal anywhere in the corpus rescues the candidate.
// ---------------------------------------------------------------------------

#[test]
fn a_reference_in_a_ci_workflow_vetoes_the_candidate() {
    // The canonical §6.2 shape: nothing in Python imports this module, but a
    // scheduled workflow runs it by path.
    let (_dir, repo) = repo_with(&[
        ("ledger/dunning.py", b"def main():\n    pass\n"),
        (
            ".github/workflows/nightly.yml",
            b"jobs:\n  dun:\n    run: python ledger/dunning.py\n",
        ),
        ("README.md", b"# billing\n"),
    ]);

    let veto = LiteralVeto::new(&repo);
    let verdict = veto.query(
        &Candidate::file("ledger/dunning.py"),
        NeedleStrategy::MAXIMAL,
    );

    assert!(verdict.is_veto(), "expected a veto, got {verdict:?}");
    let Verdict::Vetoed {
        reason: VetoReason::Reference { first },
        report,
        ..
    } = &verdict
    else {
        panic!("expected a reference veto, got {verdict:?}");
    };
    assert_eq!(first.file(), Path::new(".github/workflows/nightly.yml"));
    assert_eq!(report.state(), &ScanState::Completed);
    // The report has to name the file and the needle, or a human cannot audit
    // the conflict list §9.13 asks for.
    assert!(
        hit_set(&verdict)
            .iter()
            .any(|(file, _, _)| file == ".github/workflows/nightly.yml"),
        "hits must name the file that rescued the candidate: {:?}",
        hit_set(&verdict)
    );
}

// ---------------------------------------------------------------------------
// 1. Self-reference.
// ---------------------------------------------------------------------------

#[test]
fn a_file_does_not_veto_itself() {
    // The file is saturated with its own basename, stem and parent directory
    // name. If self-reference were not excluded, the veto would fire for every
    // candidate in every repository and Gate 2 would be a constant function.
    let (_dir, repo) = repo_with(&[
        (
            "ledger/dunning.py",
            b"# ledger/dunning.py\n__all__ = ['dunning']\nprint('dunning.py')\n",
        ),
        ("README.md", b"# nothing to see here\n"),
    ]);

    let veto = LiteralVeto::new(&repo);
    let verdict = veto.query(
        &Candidate::file("ledger/dunning.py"),
        NeedleStrategy::MAXIMAL,
    );

    assert_clear(&verdict);
    // Exactly one file was searched: the candidate was *excluded from the
    // corpus*, not silently turned into a needle-free query.
    assert_eq!(
        verdict.report().files_searched(),
        1,
        "the candidate itself must be the only excluded file"
    );
    assert!(
        verdict
            .report()
            .needles()
            .iter()
            .any(|n| n.kind() == NeedleKind::Basename && n.text() == "dunning.py"),
        "the basename needle must still have been searched for: {:?}",
        verdict.report().needles()
    );
}

#[test]
fn a_symbol_is_not_vetoed_by_its_own_definition_site() {
    let (_dir, repo) = repo_with(&[
        (
            "app/jobs/handlers.py",
            b"class DunningHandler:\n    \"\"\"DunningHandler docs.\"\"\"\n",
        ),
        ("app/main.py", b"print('start')\n"),
    ]);

    let veto = LiteralVeto::new(&repo);
    let verdict = veto.query(
        &Candidate::symbol("app/jobs/handlers.py", "DunningHandler"),
        NeedleStrategy::MAXIMAL,
    );

    assert_clear(&verdict);
    assert!(
        verdict
            .report()
            .needles()
            .iter()
            .any(|n| n.kind() == NeedleKind::Symbol && n.text() == "DunningHandler"),
        "the symbol needle must still have been searched for: {:?}",
        verdict.report().needles()
    );
}

#[test]
fn a_symbol_named_in_any_other_file_vetoes() {
    let (_dir, repo) = repo_with(&[
        ("app/jobs/handlers.py", b"class DunningHandler:\n    pass\n"),
        ("app/config.yml", b"handler: DunningHandler\n"),
    ]);

    let veto = LiteralVeto::new(&repo);
    let verdict = veto.query(
        &Candidate::symbol("app/jobs/handlers.py", "DunningHandler"),
        NeedleStrategy::MAXIMAL,
    );

    assert_eq!(
        hit_set(&verdict),
        vec![(
            "app/config.yml".to_string(),
            NeedleKind::Symbol,
            "DunningHandler".to_string()
        )]
    );
}

// ---------------------------------------------------------------------------
// 2. An incomplete search is a HIT, never an absence (§6.20).
// ---------------------------------------------------------------------------

#[test]
fn an_errored_read_of_one_file_out_of_many_vetoes() {
    // No file in this repository contains any needle, so a scanner that
    // *skipped* the unreadable file would confidently answer "no references"
    // — which is exactly the inversion §6.20 records.
    let (_dir, repo) = repo_with(&[
        ("ledger/dunning.py", b"def main():\n    pass\n"),
        ("docs/overview.md", b"# overview\n"),
        ("docs/api.md", b"# api\n"),
        ("Makefile", b"all:\n\t@true\n"),
    ]);
    // Tracked in the index, absent from the working tree: the read fails.
    std::fs::remove_file(repo.root().join("docs/api.md")).expect("remove tracked file");

    let veto = LiteralVeto::new(&repo);
    let verdict = veto.query(
        &Candidate::file("ledger/dunning.py"),
        NeedleStrategy::MAXIMAL,
    );

    let Verdict::Vetoed {
        reason: VetoReason::IncompleteSearch { state },
        ..
    } = &verdict
    else {
        panic!("an unreadable tracked file must veto, got {verdict:?}");
    };
    let ScanState::Errored { file, message } = state else {
        panic!("expected an Errored scan state, got {state:?}");
    };
    assert_eq!(file.as_deref(), Some(Path::new("docs/api.md")));
    assert!(
        !message.is_empty(),
        "the failure must say what went wrong, not merely that something did"
    );
}

#[test]
fn a_truncated_read_vetoes() {
    let (_dir, repo) = repo_with(&[
        ("ledger/dunning.py", b"def main():\n    pass\n"),
        (
            "vendor/blob.bin",
            b"padding padding padding padding padding padding",
        ),
    ]);

    // A per-file byte cap smaller than a tracked file: the scan cannot see the
    // whole corpus, so it may not answer "absent".
    let veto = LiteralVeto::with_limits(
        &repo,
        ScanLimits {
            max_file_bytes: Some(8),
            ..ScanLimits::default()
        },
    );
    let verdict = veto.query(
        &Candidate::file("ledger/dunning.py"),
        NeedleStrategy::MAXIMAL,
    );

    let Verdict::Vetoed {
        reason: VetoReason::IncompleteSearch { state },
        ..
    } = &verdict
    else {
        panic!("a file too large to read whole must veto, got {verdict:?}");
    };
    let ScanState::Truncated {
        file,
        limit_bytes,
        actual_bytes,
    } = state
    else {
        panic!("expected a Truncated scan state, got {state:?}");
    };
    assert_eq!(file, Path::new("vendor/blob.bin"));
    assert_eq!(*limit_bytes, 8);
    assert!(*actual_bytes > 8, "actual {actual_bytes} should exceed 8");
}

#[test]
fn a_timed_out_scan_vetoes() {
    let (_dir, repo) = repo_with(&[
        ("ledger/dunning.py", b"def main():\n    pass\n"),
        ("README.md", b"# billing\n"),
    ]);

    let veto = LiteralVeto::with_limits(
        &repo,
        ScanLimits {
            budget: Some(Duration::ZERO),
            ..ScanLimits::default()
        },
    );
    let verdict = veto.query(
        &Candidate::file("ledger/dunning.py"),
        NeedleStrategy::MAXIMAL,
    );

    let Verdict::Vetoed {
        reason: VetoReason::IncompleteSearch { state },
        ..
    } = &verdict
    else {
        panic!("an exhausted time budget must veto, got {verdict:?}");
    };
    assert!(
        matches!(state, ScanState::TimedOut { .. }),
        "expected a TimedOut scan state, got {state:?}"
    );
}

#[test]
fn a_candidate_with_no_derivable_needle_vetoes() {
    let (_dir, repo) = repo_with(&[("README.md", b"# billing\n")]);

    let veto = LiteralVeto::new(&repo);
    let verdict = veto.query(&Candidate::file(""), NeedleStrategy::MAXIMAL);

    // Searching for nothing finds nothing. That is not evidence of absence.
    assert!(
        matches!(
            &verdict,
            Verdict::Vetoed {
                reason: VetoReason::IncompleteSearch { .. },
                ..
            }
        ),
        "a candidate we cannot build a needle for must veto, got {verdict:?}"
    );
}

#[test]
fn a_candidate_outside_the_working_tree_vetoes() {
    let (_dir, repo) = repo_with(&[("README.md", b"# billing\n")]);

    let veto = LiteralVeto::new(&repo);
    let verdict = veto.query(&Candidate::file("/etc/hosts"), NeedleStrategy::MAXIMAL);

    assert!(
        matches!(
            &verdict,
            Verdict::Vetoed {
                reason: VetoReason::IncompleteSearch { .. },
                ..
            }
        ),
        "a candidate outside the corpus we can search must veto, got {verdict:?}"
    );
}

// ---------------------------------------------------------------------------
// 3. Binaries are searched — E2 class m16.
// ---------------------------------------------------------------------------

#[test]
fn a_class_name_embedded_in_a_pickle_vetoes() {
    // A real pickle preamble with the class name surrounded by NUL bytes: no
    // text-oriented scanner reaches it, and it is the only evidence anywhere in
    // the repository that `DunningScorer` is still instantiated (m16).
    let pickle: &[u8] = b"\x80\x04\x95\x2a\x00\x00\x00\x00\x00\x00\x00\x8c\x00\x00DunningScorer\x00\x00\x94\x8c\x08builtins\x94\x93\x94.";
    let (_dir, repo) = repo_with(&[
        ("app/scoring.py", b"class DunningScorer:\n    pass\n"),
        ("models/checkpoint.pkl", pickle),
        ("README.md", b"# risk model\n"),
    ]);

    let veto = LiteralVeto::new(&repo);
    let verdict = veto.query(
        &Candidate::symbol("app/scoring.py", "DunningScorer"),
        NeedleStrategy::MAXIMAL,
    );

    assert_eq!(
        hit_set(&verdict),
        vec![(
            "models/checkpoint.pkl".to_string(),
            NeedleKind::Symbol,
            "DunningScorer".to_string()
        )],
        "the needle must be found inside the binary, between NUL bytes"
    );
    let Verdict::Vetoed {
        reason: VetoReason::Reference { first },
        ..
    } = &verdict
    else {
        panic!("expected a reference veto, got {verdict:?}");
    };
    // The offset must land inside the binary, past its NUL-laden preamble.
    assert_eq!(
        &pickle[first.offset()..first.offset() + 13],
        &b"DunningScorer"[..]
    );
}

#[test]
fn a_tracked_symlink_is_searched_as_its_target_path() {
    let (_dir, repo) = repo_with(&[
        ("releases/dunning.py", b"def main():\n    pass\n"),
        ("README.md", b"# billing\n"),
    ]);
    std::os::unix::fs::symlink("releases/dunning.py", repo.root().join("current"))
        .expect("symlink");
    repo.add_all().expect("git add --all");
    repo.commit("add symlink").expect("git commit");

    let veto = LiteralVeto::new(&repo);
    let verdict = veto.query(
        &Candidate::file("releases/dunning.py"),
        NeedleStrategy::BASENAME_ONLY,
    );

    assert_eq!(
        hit_set(&verdict),
        vec![(
            "current".to_string(),
            NeedleKind::Basename,
            "dunning.py".to_string()
        )],
        "a symlink's target string is tracked content and must be searched"
    );
}

// ---------------------------------------------------------------------------
// 4. The needle strategy is selectable and its fire rate is measurable (R8).
// ---------------------------------------------------------------------------

#[test]
fn widening_the_needle_strategy_widens_the_fire_rate() {
    // Each non-candidate file contains exactly one needle and no other, so the
    // hit count *is* the fire rate for that strategy.
    let (_dir, repo) = repo_with(&[
        ("ledger/dunning.py", b"def main():\n    pass\n"),
        ("docs/notes.md", b"the dunning cycle runs nightly\n"),
        (
            "infra/main.tf",
            b"variable \"area\" { default = \"ledger\" }\n",
        ),
    ]);
    let veto = LiteralVeto::new(&repo);
    let candidate = Candidate::file("ledger/dunning.py");

    // Basename only: nothing in the repository spells `dunning.py`.
    let basename = veto.query(&candidate, NeedleStrategy::BASENAME_ONLY);
    assert_clear(&basename);
    assert_eq!(basename.report().files_searched(), 2);

    // + stem: prose about "the dunning cycle" now rescues it.
    let stem = veto.query(&candidate, NeedleStrategy::WITH_STEM);
    assert_eq!(
        hit_set(&stem),
        vec![(
            "docs/notes.md".to_string(),
            NeedleKind::Stem,
            "dunning".to_string()
        )]
    );

    // + parent directory: a Terraform variable that merely names the directory
    // rescues it too. This is the R8 trade made visible — one more needle, one
    // more flagged file, and `ledger` is a *distinctive* directory name.
    let parent = veto.query(&candidate, NeedleStrategy::WITH_PARENT_DIR);
    assert_eq!(
        hit_set(&parent),
        vec![
            (
                "docs/notes.md".to_string(),
                NeedleKind::Stem,
                "dunning".to_string()
            ),
            (
                "infra/main.tf".to_string(),
                NeedleKind::ParentDir,
                "ledger".to_string()
            ),
        ]
    );

    // The strategy is a set, so a single kind can be dropped in isolation —
    // which is what makes fire rate per needle kind measurable rather than
    // guessed.
    let no_stem = veto.query(
        &candidate,
        NeedleStrategy::WITH_PARENT_DIR.without(NeedleKind::Stem),
    );
    assert_eq!(
        hit_set(&no_stem),
        vec![(
            "infra/main.tf".to_string(),
            NeedleKind::ParentDir,
            "ledger".to_string()
        )]
    );
}

#[test]
fn the_basename_needle_cannot_be_switched_off() {
    // §9.3 makes the basename mandatory. A strategy set that could drop it
    // would let a caller disable Gate 2a while appearing to run it.
    for strategy in [
        NeedleStrategy::BASENAME_ONLY,
        NeedleStrategy::WITH_STEM,
        NeedleStrategy::WITH_PARENT_DIR,
        NeedleStrategy::MAXIMAL,
        NeedleStrategy::MAXIMAL.without(NeedleKind::Basename),
    ] {
        assert!(
            strategy.includes(NeedleKind::Basename),
            "{strategy:?} dropped the mandatory basename needle"
        );
    }
}

// ---------------------------------------------------------------------------
// The known miss the research predicts (§6.2) — recorded, not papered over.
// ---------------------------------------------------------------------------

#[test]
fn a_runtime_concatenated_import_is_missed_by_name_needles_and_caught_by_the_directory() {
    // E2 class m02: `await import(`./transports/${kind}Transport.js`)`. No
    // contiguous literal spells the basename or the stem anywhere in the
    // repository, so §6.2's "the name exists, but not as a contiguous literal"
    // is exactly true here.
    let (_dir, repo) = repo_with(&[
        (
            "src/transports/websocketTransport.js",
            b"export default class WebsocketTransport {}\n",
        ),
        (
            "src/registry.js",
            b"export const load = (kind) => import(`./transports/${kind}Transport.js`);\n",
        ),
        ("package.json", b"{ \"name\": \"app\" }\n"),
    ]);
    let veto = LiteralVeto::new(&repo);
    let candidate = Candidate::file("src/transports/websocketTransport.js");

    // The honest miss: neither the basename nor the stem occurs.
    assert_clear(&veto.query(&candidate, NeedleStrategy::BASENAME_ONLY));
    assert_clear(&veto.query(&candidate, NeedleStrategy::WITH_STEM));

    // The parent-directory needle is the only thing that rescues it, because
    // the *static prefix* of the computed specifier is the directory name.
    // That is a measurement of what the parent-dir needle buys, not a fix for
    // concatenation: a candidate whose directory is `src`, `app` or `lib`
    // would be rescued by nearly any file, and one imported as
    // `${dir}/${kind}.js` would still be missed.
    let parent = veto.query(&candidate, NeedleStrategy::WITH_PARENT_DIR);
    assert_eq!(
        hit_set(&parent),
        vec![(
            "src/registry.js".to_string(),
            NeedleKind::ParentDir,
            "transports".to_string()
        )]
    );
}

// ---------------------------------------------------------------------------
// The corpus really is every tracked file, whatever its extension.
// ---------------------------------------------------------------------------

#[test]
fn every_tracked_file_type_is_part_of_the_corpus() {
    let cases: [(&str, &[u8]); 8] = [
        ("Dockerfile", b"COPY ledger/dunning.py /app/\n"),
        ("Makefile", b"run:\n\t python ledger/dunning.py\n"),
        ("infra/main.tf", b"command = [\"ledger/dunning.py\"]\n"),
        ("db/migrate.sql", b"-- see ledger/dunning.py\n"),
        ("i18n/en.json", b"{ \"job\": \"ledger/dunning.py\" }\n"),
        (".env.example", b"ENTRYPOINT=ledger/dunning.py\n"),
        ("AGENTS.md", b"The dunning.py job is load bearing.\n"),
        ("scripts/deploy.sh", b"#!/bin/sh\nexec ledger/dunning.py\n"),
    ];
    for (name, body) in cases {
        let (_dir, repo) = repo_with(&[
            ("ledger/dunning.py", b"def main():\n    pass\n"),
            (name, body),
        ]);
        let veto = LiteralVeto::new(&repo);
        let verdict = veto.query(
            &Candidate::file("ledger/dunning.py"),
            NeedleStrategy::BASENAME_ONLY,
        );
        assert!(
            verdict.is_veto(),
            "{name} is tracked and names the candidate, so it must veto: {verdict:?}"
        );
        assert_eq!(
            verdict.report().hits().first().map(|h| h.file().to_owned()),
            Some(PathBuf::from(name))
        );
    }
}
