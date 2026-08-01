//! Gate 1 classes 1l–1p — contracts with the outside world, and the tool
//! protecting itself from itself.
//!
//! Five properties decide whether this layer is sound, and each has its own
//! section below:
//!
//! 1. **1l is a predicate about external readership, not a proxy for it.**
//!    §6.11 names the trap by name: a size floor "hard-exclude files under 64
//!    bytes" saves `__init__.py` and `.nojekyll`, and `CNAME` at ~20 bytes is
//!    exactly that class — but the floor is a heuristic about size when the real
//!    predicate is *read by something outside the repository*. So the tests here
//!    classify the same contract at 20 bytes and at 200 000 bytes and demand the
//!    same verdict, and classify a 20-byte ordinary source file and demand no
//!    contract refusal at all.
//! 2. **Ignore status is per FILE, never per directory** (§6.17). A `!` negation
//!    re-includes one file inside an ignored tree; its siblings stay ignored.
//!    The tests assert both halves on the same directory.
//! 3. **The negation probe must work on TRACKED files.** Everything §6.17
//!    measured — `.vscode/settings.json`, `var/logs/.gitkeep`,
//!    `/media/**/.htaccess` — is checked in. A probe that only answers for
//!    untracked paths answers for none of them.
//! 4. **The tool's own veto list and evidence are never-touch** (§6.22), and
//!    pruning the ledger cannot co-occur with a deletion.
//! 5. **The unknown defaults to KEEP** (1p). A file whose type cannot be
//!    determined is not a candidate, and the refusal is *reported* as such
//!    rather than being an absence of opinion.

use std::path::PathBuf;

use judged_core::gate1::contracts::{
    platform_contracts, review_plan, ContractClass, ContractGate, ContractVerdict, Disposition,
    FailureMode, PlanReview, Refusal, ToolArtifactKind, TypeSignal,
};
use judged_core::git::Repo;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// Build a real git repository holding `files`.
///
/// Real, because class 1m's only sound implementation is to ask git — §6.17's
/// experimental result is that *"git itself is per-file careful; every naive
/// `rm -rf`-on-ignored-directories reimplementation is not"*, and a fake repo
/// would test our reimplementation of the thing we deliberately did not
/// reimplement.
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
    (dir, repo)
}

/// The same, with everything git will accept staged and committed.
///
/// Property 3 above: the files §6.17 measured are all checked in, so every 1m
/// test that matters runs against an index.
fn committed_repo_with(files: &[(&str, &[u8])]) -> (TempDir, Repo) {
    let (dir, repo) = repo_with(files);
    repo.add_all().expect("git add --all");
    repo.commit("fixture").expect("git commit");
    (dir, repo)
}

fn classify(repo: &Repo, rel: &str) -> ContractVerdict {
    ContractGate::new(repo)
        .classify(std::path::Path::new(rel))
        .unwrap_or_else(|e| panic!("classify {rel}: {e}"))
}

/// Every class the verdict refuses under, sorted and deduplicated.
fn classes(verdict: &ContractVerdict) -> Vec<ContractClass> {
    let mut out: Vec<ContractClass> = verdict.reasons().iter().map(Refusal::class).collect();
    out.sort();
    out.dedup();
    out
}

fn failure_mode(verdict: &ContractVerdict) -> FailureMode {
    verdict
        .reasons()
        .iter()
        .find_map(|r| match r {
            Refusal::PlatformContract(c) => Some(c.failure_mode()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no 1l refusal in {:?}", classes(verdict)))
}

// ---------------------------------------------------------------------------
// 1l — platform contracts (§6.11)
// ---------------------------------------------------------------------------

/// Every row of §6.11's table refuses, wherever the file sits in the tree.
#[test]
fn every_platform_contract_file_is_never_touch() {
    let paths: &[&str] = &[
        "CNAME",
        "docs/CNAME",
        ".nojekyll",
        "_redirects",
        "_headers",
        "vercel.json",
        "netlify.toml",
        "static.json",
        "apple-app-site-association",
        ".well-known/apple-app-site-association",
        ".well-known/assetlinks.json",
        "public/.well-known/assetlinks.json",
        "robots.txt",
        ".well-known/security.txt",
        "CODEOWNERS",
        ".github/CODEOWNERS",
        ".github/dependabot.yml",
        "renovate.json",
        ".github/workflows/nightly-backup.yml",
        "VERSION",
        ".python-version",
        ".nvmrc",
        ".ruby-version",
        ".node-version",
        ".tool-versions",
        "runtime.txt",
        "Procfile",
        ".buildpacks",
        ".well-known/acme-challenge/8xR2Qb",
        ".well-known/pki-validation/godaddy.html",
        ".well-known/apple-developer-merchantid-domain-association",
        ".well-known/microsoft-identity-association.json",
        ".well-known/openid-configuration",
        ".well-known/change-password",
        "ads.txt",
        "app-ads.txt",
        ".htaccess",
        "web.config",
        "manifest.webmanifest",
        "manifest.json",
        "service-worker.js",
        "_config.yml",
        "Staticfile",
        ".platform/nginx/conf.d/proxy.conf",
        "app.yaml",
        "fly.toml",
        "render.yaml",
        "railway.json",
        ".gitattributes",
        ".github/FUNDING.yml",
        ".github/ISSUE_TEMPLATE/bug.md",
        ".github/actions/setup/action.yml",
        "codecov.yml",
        ".coveragerc",
        "sonar-project.properties",
    ];
    let files: Vec<(&str, &[u8])> = paths.iter().map(|p| (*p, b"x\n".as_slice())).collect();
    let (_dir, repo) = committed_repo_with(&files);

    for path in paths {
        let verdict = classify(&repo, path);
        assert_eq!(
            verdict.disposition(),
            Disposition::NeverTouch,
            "{path} must be never-touch"
        );
        assert!(
            classes(&verdict).contains(&ContractClass::PlatformContract),
            "{path} must refuse under 1l, got {:?}",
            classes(&verdict)
        );
    }
}

/// The five failures §6.11 names because they are not a 404.
#[test]
fn the_named_silent_failures_are_classified_as_what_they_are() {
    let (_dir, repo) = committed_repo_with(&[
        ("CNAME", b"www.example.com\n"),
        (".well-known/acme-challenge/8xR2Qb", b"token\n"),
        ("ads.txt", b"example.com, 1234, DIRECT\n"),
        ("CODEOWNERS", b"* @team\n"),
        ("apple-app-site-association", b"{}\n"),
    ]);

    // Deleting CNAME removes the GitHub-side binding while DNS still points at
    // GitHub: the dangling-DNS condition that enables subdomain takeover.
    assert_eq!(
        failure_mode(&classify(&repo, "CNAME")),
        FailureMode::SecurityEvent
    );
    // Automated TLS renewal stops; it surfaces weeks later as an expired cert.
    assert_eq!(
        failure_mode(&classify(&repo, ".well-known/acme-challenge/8xR2Qb")),
        FailureMode::RenewalStopsUntilExpiry
    );
    // Ad inventory becomes unauthorized: revenue goes to zero, no error anywhere.
    assert_eq!(
        failure_mode(&classify(&repo, "ads.txt")),
        FailureMode::DeliveryStopsSilently
    );
    // Required-review enforcement disappears. The failure is that CI stops failing.
    assert_eq!(
        failure_mode(&classify(&repo, "CODEOWNERS")),
        FailureMode::ControlSilentlyRemoved
    );
    // Universal Links break for apps already on other people's phones.
    assert_eq!(
        failure_mode(&classify(&repo, "apple-app-site-association")),
        FailureMode::ShippedClientsBreak
    );
}

/// §6.11's size-floor trap, tested directly.
///
/// The verdict must be a function of *who reads the file*, never of how big it
/// is. Three cases pin that down: the 20-byte contract refuses, the 200 000-byte
/// contract refuses identically, and a 20-byte ordinary source file — exactly
/// what a "hard-exclude under 64 bytes" rule would save for the wrong reason —
/// carries no contract refusal at all.
#[test]
fn the_predicate_is_external_readership_not_file_size() {
    let big = vec![b'a'; 200_000];
    let (_dir, repo) = committed_repo_with(&[
        ("CNAME", b"www.example.com\n"), // 16 bytes
        ("docs/CNAME", big.as_slice()),  // 200 000 bytes
        ("src/util.py", b"X = 1\n"),     // 6 bytes, ordinary source
        ("src/__init__.py", b""),        // 0 bytes, ordinary source
    ]);

    let tiny = classify(&repo, "CNAME");
    let huge = classify(&repo, "docs/CNAME");
    assert_eq!(tiny.disposition(), Disposition::NeverTouch);
    assert_eq!(huge.disposition(), Disposition::NeverTouch);
    assert_eq!(
        classes(&tiny),
        classes(&huge),
        "the same contract at 16 and 200 000 bytes must classify identically"
    );
    assert_eq!(failure_mode(&tiny), failure_mode(&huge));

    // A size floor saves these two for a reason that has nothing to do with
    // them. 1l must not claim them; they are ordinary, typed source files.
    for ordinary in ["src/util.py", "src/__init__.py"] {
        let verdict = classify(&repo, ordinary);
        assert!(
            !classes(&verdict).contains(&ContractClass::PlatformContract),
            "{ordinary} is not a platform contract"
        );
        assert_eq!(
            verdict.disposition(),
            Disposition::NoObjection,
            "{ordinary} has a determined type and no contract: 1l–1p have no say"
        );
    }
}

/// Membership in the registry requires naming the reader outside the repository
/// and the failure deleting it causes. This is the predicate, encoded: an entry
/// that cannot name its external consumer cannot exist.
#[test]
fn every_registry_entry_names_its_external_consumer_and_its_failure() {
    let entries = platform_contracts();
    assert!(
        entries.len() > 40,
        "§6.11 is a long table: {}",
        entries.len()
    );
    for entry in entries {
        assert!(
            !entry.consumer().trim().is_empty(),
            "{} names no external consumer",
            entry.pattern()
        );
        assert!(
            !entry.effect().trim().is_empty(),
            "{} does not say what deleting it does",
            entry.pattern()
        );
    }
    // No duplicate matchers: two rows for one path make the reported failure
    // mode depend on registry order, which is not a property anyone should have
    // to know to read a report.
    let mut patterns: Vec<&str> = entries.iter().map(|e| e.pattern()).collect();
    patterns.sort_unstable();
    let before = patterns.len();
    patterns.dedup();
    assert_eq!(before, patterns.len(), "duplicate matcher in the registry");
}

// ---------------------------------------------------------------------------
// 1m — un-ignored by a `!` negation (§6.17)
// ---------------------------------------------------------------------------

/// Magento's `/media/*` plus its negation carve-outs, the shape §6.17 measured
/// and E2 class m13 is built from.
#[test]
fn a_negation_un_ignore_is_never_touch_with_the_deciding_rule_as_evidence() {
    let (_dir, repo) = committed_repo_with(&[
        (
            ".gitignore",
            b"/media/*\n!/media/customer\n!/media/customer/.htaccess\n".as_slice(),
        ),
        ("media/customer/.htaccess", b"deny from all\n"),
        ("media/catalog/product.png", b"\x89PNG\r\n\x1a\nrest"),
    ]);

    let kept = classify(&repo, "media/customer/.htaccess");
    assert_eq!(kept.disposition(), Disposition::NeverTouch);
    let negation = kept
        .reasons()
        .iter()
        .find_map(|r| match r {
            Refusal::NegationUnIgnored(n) => Some(n),
            _ => None,
        })
        .expect("1m refusal");
    assert_eq!(negation.source(), PathBuf::from(".gitignore"));
    assert_eq!(negation.line(), 3);
    assert_eq!(negation.pattern(), "!/media/customer/.htaccess");

    // §6.11's own note on this row: the file is simultaneously ignored-by-pattern
    // and un-ignored-by-negation, and it is also an Apache request-routing
    // contract. Both classes must be reported, not just whichever ran first.
    assert_eq!(
        classes(&kept),
        vec![
            ContractClass::PlatformContract,
            ContractClass::NegationUnIgnored
        ]
    );
}

/// Ignore status belongs to a file, never to the directory above it.
#[test]
fn ignore_status_is_per_file_never_per_directory() {
    let (_dir, repo) = committed_repo_with(&[
        (
            ".gitignore",
            b".vscode/*\n!.vscode/settings.json\n!.vscode/tasks.json\n".as_slice(),
        ),
        (".vscode/settings.json", b"{}\n"),
        (".vscode/tasks.json", b"{}\n"),
    ]);
    // Both survivors are 1m.
    for kept in [".vscode/settings.json", ".vscode/tasks.json"] {
        assert!(
            classes(&classify(&repo, kept)).contains(&ContractClass::NegationUnIgnored),
            "{kept} is un-ignored by a negation"
        );
    }

    // A sibling in the same ignored directory, matched only by the ignoring
    // pattern, is not 1m. A directory-level answer would claim it is.
    let (_dir2, repo2) = repo_with(&[
        (
            ".gitignore",
            b".vscode/*\n!.vscode/settings.json\n".as_slice(),
        ),
        (".vscode/settings.json", b"{}\n"),
        (".vscode/scratch.json", b"{}\n"),
    ]);
    assert!(
        !classes(&classify(&repo2, ".vscode/scratch.json"))
            .contains(&ContractClass::NegationUnIgnored),
        "an ordinarily-ignored sibling is not un-ignored by a negation"
    );
}

/// The probe must answer for files that are in the index.
///
/// This is the whole population §6.17 measured: `.vscode/settings.json`,
/// `var/logs/.gitkeep`, `/media/**/.htaccess` are all checked in. Measured
/// against git 2.50.1, `git check-ignore -vz --stdin --non-matching` reports an
/// *empty* pattern for every tracked path — it consults the index and answers
/// "not ignored" without saying which rule decided. Only `--no-index` reports
/// the deciding rule, so only `--no-index` can see the negation.
#[test]
fn negations_are_detected_on_tracked_files() {
    let (_dir, repo) = committed_repo_with(&[
        (
            ".gitignore",
            b"var/logs/*\n!var/logs/.gitkeep\nvar/cache/*\n!var/cache/.gitkeep\n".as_slice(),
        ),
        ("var/logs/.gitkeep", b""),
        ("var/cache/.gitkeep", b""),
        ("var/logs/app.log", b"noise\n"),
    ]);
    assert!(
        repo.is_tracked(std::path::Path::new("var/logs/.gitkeep"))
            .expect("is_tracked"),
        "the fixture must actually be committed, or this test proves nothing"
    );
    for kept in ["var/logs/.gitkeep", "var/cache/.gitkeep"] {
        let verdict = classify(&repo, kept);
        assert_eq!(
            verdict.disposition(),
            Disposition::NeverTouch,
            "{kept}: a placeholder whose entire purpose is to exist"
        );
        assert!(
            classes(&verdict).contains(&ContractClass::NegationUnIgnored),
            "{kept}"
        );
    }
}

/// A negation in a nested `.gitignore` is reported against that file.
#[test]
fn a_nested_gitignore_negation_names_its_own_file() {
    let (_dir, repo) = committed_repo_with(&[
        (".gitignore", b"*.tmp\n".as_slice()),
        ("sub/.gitignore", b"*\n!keepme.txt\n".as_slice()),
        ("sub/keepme.txt", b"kept\n"),
    ]);
    let verdict = classify(&repo, "sub/keepme.txt");
    let negation = verdict
        .reasons()
        .iter()
        .find_map(|r| match r {
            Refusal::NegationUnIgnored(n) => Some(n),
            _ => None,
        })
        .expect("1m refusal");
    assert_eq!(negation.source(), PathBuf::from("sub/.gitignore"));
    assert_eq!(negation.line(), 2);
    assert_eq!(negation.pattern(), "!keepme.txt");
}

/// A repository with no ignore rules at all produces no 1m refusals — the class
/// must not degenerate into "everything", which is the failure mode of a veto
/// that cannot say no.
#[test]
fn no_negation_means_no_1m_refusal() {
    let (_dir, repo) = committed_repo_with(&[("src/main.rs", b"fn main() {}\n")]);
    let verdict = classify(&repo, "src/main.rs");
    assert!(classes(&verdict).is_empty(), "{:?}", classes(&verdict));
    assert_eq!(verdict.disposition(), Disposition::NoObjection);
}

// ---------------------------------------------------------------------------
// 1n — the keep manifest and the deletion ledger (§6.22)
// ---------------------------------------------------------------------------

/// The keep manifest is the first entry in its own never-touch list.
///
/// §6.22: it accumulates entries and looks stale, so an agent told to clean up
/// prunes the veto list and the *next* run deletes everything a human
/// previously vetoed.
#[test]
fn the_keep_manifest_and_the_deletion_ledger_are_never_touch() {
    let (_dir, repo) = committed_repo_with(&[
        (".judged/keep.toml", b"[[keep]]\npath = \"src/x.rs\"\n"),
        (".judged/ledger.jsonl", b"{}\n"),
        (".judged/anything-else", b"x\n"),
    ]);
    for path in [
        ".judged/keep.toml",
        ".judged/ledger.jsonl",
        ".judged/anything-else",
    ] {
        let verdict = classify(&repo, path);
        assert_eq!(verdict.disposition(), Disposition::NeverTouch, "{path}");
        assert!(
            classes(&verdict).contains(&ContractClass::ToolLedger),
            "{path} must refuse under 1n, got {:?}",
            classes(&verdict)
        );
    }
}

/// Pruning the ledger and deleting anything cannot happen in the same run.
#[test]
fn a_run_that_edits_the_ledger_may_not_also_delete() {
    let edits = vec![PathBuf::from(".judged/keep.toml")];
    let deletions = vec![PathBuf::from("src/dead.rs")];

    assert_eq!(
        review_plan(&edits, &deletions),
        PlanReview::RefusedCoOccurrence {
            edit: PathBuf::from(".judged/keep.toml"),
            deletion: PathBuf::from("src/dead.rs"),
        }
    );
    // Either half alone is fine.
    assert_eq!(review_plan(&edits, &[]), PlanReview::Permitted);
    assert_eq!(review_plan(&[], &deletions), PlanReview::Permitted);
}

/// The ledger and the evidence may never appear in a deletion set at all.
#[test]
fn a_plan_may_not_delete_the_ledger_or_the_evidence() {
    for (path, class) in [
        (".judged/keep.toml", ContractClass::ToolLedger),
        (".coverage", ContractClass::ToolEvidence),
    ] {
        assert_eq!(
            review_plan(&[], &[PathBuf::from(path)]),
            PlanReview::RefusedSelfDeletion {
                path: PathBuf::from(path),
                class,
            },
            "{path}"
        );
    }
}

// ---------------------------------------------------------------------------
// 1o — the tool's own evidence (§6.22)
// ---------------------------------------------------------------------------

/// Every one of these is a canonical junk pattern *and* the next run's
/// evidence. Removing them makes run N+1 strictly less informed, and nothing
/// detects that the cause was the cleaner — confidence degrades monotonically
/// toward more aggressive deletion.
#[test]
fn the_tools_own_evidence_is_never_touch_although_it_is_canonical_junk() {
    let paths: &[&str] = &[
        ".coverage",
        "coverage.xml",
        "lcov.info",
        "jacoco.exec",
        "target/debug/deps/judged-1a2b.profraw",
        "default.profdata",
        "build/CMakeFiles/foo.dir/x.gcda",
        "build/CMakeFiles/foo.dir/x.gcno",
        ".nyc_output/processinfo/index.json",
        "coverage/lcov-report/index.html",
        "htmlcov/index.html",
        ".turbo/cache/abc.tar.zst",
    ];
    let files: Vec<(&str, &[u8])> = paths.iter().map(|p| (*p, b"x\n".as_slice())).collect();
    let (_dir, repo) = repo_with(&files);

    for path in paths {
        let verdict = classify(&repo, path);
        assert_eq!(verdict.disposition(), Disposition::NeverTouch, "{path}");
        let artifact = verdict
            .reasons()
            .iter()
            .find_map(|r| match r {
                Refusal::ToolArtifact(a) if a.kind() == ToolArtifactKind::Evidence => Some(a),
                _ => None,
            })
            .unwrap_or_else(|| panic!("{path} must refuse under 1o, got {:?}", classes(&verdict)));
        // The collision is the point: each of these is *also* on every junk
        // list ever written, which is exactly why the class has to exist.
        assert!(
            artifact.also_canonical_junk(),
            "{path} is evidence AND canonical junk; the entry must say so"
        );
    }
}

// ---------------------------------------------------------------------------
// 1p — the unknown defaults to KEEP
// ---------------------------------------------------------------------------

/// The rule the other fifteen classes exist to make affordable.
#[test]
fn an_unrecognised_file_is_kept() {
    let (_dir, repo) = committed_repo_with(&[
        // No extension, no magic bytes, no path signal. Plain ASCII, which is
        // not a type: knowing a file is text does not tell you what reads it.
        ("misc/notes", b"hello world\n"),
        // An extension nobody recognises is not a determined type either.
        ("misc/thing.xyzzy", b"hello world\n"),
        // Empty, so there is nothing to sniff.
        ("misc/blank", b""),
    ]);
    for path in ["misc/notes", "misc/thing.xyzzy", "misc/blank"] {
        let verdict = classify(&repo, path);
        assert_eq!(
            verdict.disposition(),
            Disposition::NeverTouch,
            "{path} has an undeterminable type and must be kept"
        );
        assert_eq!(verdict.type_signal(), None, "{path}");
        assert_eq!(
            classes(&verdict),
            vec![ContractClass::UnknownType],
            "{path}: the refusal must be reported as 1p, not left as silence"
        );
    }
}

/// The three ways a type gets determined, each isolated so it is the only one
/// that can be firing.
#[test]
fn a_determined_type_is_not_refused_by_1p() {
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    png.extend_from_slice(&[0u8; 32]);
    let (_dir, repo) = committed_repo_with(&[
        // Extension only.
        ("src/main.rs", b"fn main() {}\n"),
        // Magic bytes only: the extension is deliberately unrecognisable, so a
        // pass would prove nothing unless the header is what decided it.
        ("assets/logo.qqq", png.as_slice()),
        // A shebang, on a file with no extension.
        ("scripts/deploy", b"#!/usr/bin/env bash\nset -e\n"),
        // A name the ecosystem gives a fixed meaning, no extension.
        ("Makefile", b"all:\n\techo hi\n"),
    ]);

    let by_extension = classify(&repo, "src/main.rs");
    assert!(matches!(
        by_extension.type_signal(),
        Some(TypeSignal::Extension(_))
    ));
    let by_magic = classify(&repo, "assets/logo.qqq");
    assert!(
        matches!(by_magic.type_signal(), Some(TypeSignal::Magic(_))),
        "the PNG header must determine the type when the extension cannot: {:?}",
        by_magic.type_signal()
    );
    let by_shebang = classify(&repo, "scripts/deploy");
    assert!(matches!(
        by_shebang.type_signal(),
        Some(TypeSignal::Magic(_))
    ));
    let by_name = classify(&repo, "Makefile");
    assert!(matches!(
        by_name.type_signal(),
        Some(TypeSignal::PathName(_))
    ));

    for verdict in [&by_extension, &by_magic, &by_shebang, &by_name] {
        assert_eq!(verdict.disposition(), Disposition::NoObjection);
        assert!(!classes(verdict).contains(&ContractClass::UnknownType));
    }
}

/// 1p is a fallback, not an override: it stacks with whatever else fired.
///
/// A file deliberately carved back out of an ignored tree, whose type nothing
/// can determine, is kept for *two* reasons, and a report that dropped either
/// would mislead — "somebody wrote a `!` rule for this" and "we do not know what
/// it is" are different arguments and a human needs both.
#[test]
fn an_unknown_type_stacks_with_the_other_classes_rather_than_replacing_them() {
    let (_dir, repo) = committed_repo_with(&[
        (".gitignore", b"data/*\n!data/snapshot\n".as_slice()),
        // No extension, no magic bytes, no path signal.
        ("data/snapshot", b"opaque\n"),
    ]);
    let verdict = classify(&repo, "data/snapshot");
    assert_eq!(
        classes(&verdict),
        vec![ContractClass::NegationUnIgnored, ContractClass::UnknownType]
    );
    assert_eq!(verdict.type_signal(), None);
}

/// Being a platform contract IS knowing what a file is, so 1l and 1p never
/// co-fire.
///
/// `CNAME`, `Procfile`, `.nvmrc` and `.well-known/change-password` carry no
/// extension and no magic bytes, and a verdict reading "GitHub Pages reads this"
/// *and* "we cannot determine what this is" contradicts itself. Location is the
/// type here — it is what the external reader keys on.
///
/// The safety argument for that entanglement, which is what makes it allowable:
/// consulting the contract registry inside type determination can never flip a
/// verdict toward deletion, because every path it recognises has already pushed
/// a 1l refusal. It can only remove a redundant reason from a file that is
/// refused either way.
#[test]
fn a_recognised_contract_is_never_also_reported_as_an_unknown_type() {
    let (_dir, repo) = committed_repo_with(&[
        ("CNAME", b"www.example.com\n"),
        ("Procfile", b"web: ./serve\n"),
        (".nvmrc", b"22\n"),
        (".well-known/change-password", b"nothing\n"),
    ]);
    for path in ["CNAME", "Procfile", ".nvmrc", ".well-known/change-password"] {
        let verdict = classify(&repo, path);
        assert_eq!(
            classes(&verdict),
            vec![ContractClass::PlatformContract],
            "{path}"
        );
        assert!(
            matches!(verdict.type_signal(), Some(TypeSignal::PathName(_))),
            "{path}: the contract itself determines the type, got {:?}",
            verdict.type_signal()
        );
        assert_eq!(verdict.disposition(), Disposition::NeverTouch, "{path}");
    }
}

// ---------------------------------------------------------------------------
// Composition
// ---------------------------------------------------------------------------

/// `NoObjection` requires *both* halves: no class fired and the type was
/// determined. It is the only verdict that lets a later gate have an opinion,
/// so it must be hard to reach by accident.
#[test]
fn no_objection_requires_no_class_and_a_determined_type() {
    let (_dir, repo) = committed_repo_with(&[
        ("src/lib.rs", b"pub fn f() {}\n"),
        ("CNAME", b"x.example.com\n"),
        ("misc/unknown", b"?\n"),
    ]);
    assert_eq!(
        classify(&repo, "src/lib.rs").disposition(),
        Disposition::NoObjection
    );
    // A class fired.
    assert_eq!(
        classify(&repo, "CNAME").disposition(),
        Disposition::NeverTouch
    );
    // The type did not resolve.
    assert_eq!(
        classify(&repo, "misc/unknown").disposition(),
        Disposition::NeverTouch
    );
}

/// A path outside the working tree is an error, never a silent verdict. §6.20:
/// "no data" must be a distinct state from "zero".
#[test]
fn a_path_outside_the_working_tree_is_refused_loudly() {
    let (_dir, repo) = committed_repo_with(&[("src/lib.rs", b"pub fn f() {}\n")]);
    let outside = ContractGate::new(&repo).classify(std::path::Path::new("../elsewhere/x.rs"));
    assert!(
        outside.is_err(),
        "a path escaping the working tree must not be classified"
    );
}
