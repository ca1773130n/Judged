//! Behavioural tests for Gate 1 classes 1g–1k — content whose *provenance*
//! forbids removal (§9.3).
//!
//! Every test here builds a real directory tree on disk and asks the real gate.
//! Nothing is stubbed, because three of the five classes are decided by things
//! that only exist on a filesystem: a `.gitattributes` in a subdirectory
//! overriding one at the root, a `.gitmodules` naming a submodule, and the set
//! of *siblings* that decides which migration in a sequence is the newest.
//!
//! Two tests are the point of the file:
//!
//! - [`the_newest_django_migration_is_ineligible_and_is_named_the_newest`] —
//!   §6.12's inversion. The newest migration has zero inbound references from
//!   any symbol, path or grep signal, every fresh environment works perfectly
//!   after deleting it, and every deployed environment holds a
//!   `django_migrations` row naming a file that is gone. A green test suite is
//!   not weak evidence here; it is structurally incapable of seeing the
//!   failure, because the oracle builds its world from the post-deletion state.
//! - [`a_truncated_attributes_walk_vetoes_every_candidate`] — the vendored /
//!   generated classification is a *hard exclusion that runs first* (§9.12). A
//!   run that could not finish reading `.gitattributes` has not cleared
//!   anything, and §6.20's rule applies to the substrate exactly as it applies
//!   to an analyzer: a search that did not finish is a hit, never an absence.

use std::fs;
use std::path::{Path, PathBuf};

use judged_core::gate1::content::{
    ContentClass, ContentEvidence, ContentGate, ContentVerdict, GeneratedVia, SequenceRank,
    SequenceScheme,
};

// ---------------------------------------------------------------------------
// scaffolding
// ---------------------------------------------------------------------------

/// A throwaway tree, canonicalized once.
///
/// macOS hands out temp roots under `/var/folders/…`, a symlink to
/// `/private/var/folders/…`. The gate stores its root and strips it from
/// absolute candidates, so an unresolved fixture path would fail on exactly the
/// platform these tests run on.
struct Fixture {
    _guard: tempfile::TempDir,
    root: PathBuf,
}

impl Fixture {
    fn new(label: &str) -> Fixture {
        let guard = tempfile::Builder::new()
            .prefix(&format!("judged-gate1-content-{label}-"))
            .tempdir()
            .expect("create temp dir");
        let root = fs::canonicalize(guard.path()).expect("canonicalize temp dir");
        Fixture {
            _guard: guard,
            root,
        }
    }

    /// Write `contents` at repo-relative `path`, creating parents.
    fn file(&self, path: &str, contents: &str) -> &Fixture {
        let absolute = self.root.join(path);
        if let Some(parent) = absolute.parent() {
            fs::create_dir_all(parent).expect("create parent directories");
        }
        fs::write(&absolute, contents).expect("write fixture file");
        self
    }

    fn root(&self) -> &Path {
        &self.root
    }

    fn gate(&self) -> ContentGate {
        ContentGate::build(&self.root).expect("build content gate")
    }

    fn judge(&self, path: &str) -> ContentVerdict {
        self.gate().judge(Path::new(path)).expect("judge candidate")
    }
}

/// Assert a candidate is ineligible under `class`, and hand back its evidence.
#[track_caller]
fn ineligible(verdict: &ContentVerdict, class: ContentClass) -> &ContentEvidence {
    match verdict {
        ContentVerdict::Ineligible {
            class: actual,
            evidence,
        } => {
            assert_eq!(
                *actual,
                class,
                "expected class {} ({}), got {} ({}) with evidence {evidence}",
                class.tag(),
                class.label(),
                actual.tag(),
                actual.label(),
            );
            evidence
        }
        ContentVerdict::Abstain => panic!(
            "expected Gate 1 {} ({}), got Abstain",
            class.tag(),
            class.label()
        ),
    }
}

#[track_caller]
fn assert_abstains(verdict: &ContentVerdict, why: &str) {
    match verdict {
        ContentVerdict::Abstain => {}
        ContentVerdict::Ineligible { class, evidence } => panic!(
            "expected Abstain ({why}), got {} ({}): {evidence}",
            class.tag(),
            class.label()
        ),
    }
}

// ---------------------------------------------------------------------------
// 1j — vendored / generated / submodule / LFS. Runs FIRST (§9.12).
// ---------------------------------------------------------------------------

#[test]
fn node_modules_is_ineligible_and_quotes_the_linguist_pattern() {
    let fixture = Fixture::new("node-modules");
    fixture.file("node_modules/left-pad/index.js", "module.exports = 1;\n");

    let verdict = fixture.judge("node_modules/left-pad/index.js");
    let evidence = ineligible(&verdict, ContentClass::Provenance);

    match evidence {
        ContentEvidence::LinguistVendored { pattern } => {
            assert_eq!(*pattern, "(^|/)node_modules/");
        }
        other => panic!("expected LinguistVendored, got {other:?}"),
    }
}

#[test]
fn a_vendor_directory_is_ineligible_at_any_depth() {
    let fixture = Fixture::new("vendor-depth");
    fixture.file(
        "services/api/vendor/github.com/pkg/errors.go",
        "package pkg\n",
    );

    let verdict = fixture.judge("services/api/vendor/github.com/pkg/errors.go");
    ineligible(&verdict, ContentClass::Provenance);
}

#[test]
fn every_third_party_spelling_upstream_accepts_is_ineligible() {
    let fixture = Fixture::new("third-party");
    for directory in [
        "third_party",
        "third-party",
        "thirdparty",
        "Third_Party",
        "3rdparty",
        "3rd-party",
    ] {
        let path = format!("{directory}/zlib/zlib.c");
        fixture.file(&path, "int main(void) { return 0; }\n");
        let verdict = fixture.judge(&path);
        ineligible(&verdict, ContentClass::Provenance);
    }
}

#[test]
fn a_source_file_whose_name_merely_contains_vendor_is_not_vendored() {
    let fixture = Fixture::new("vendor-lookalike");
    fixture.file("src/vendor_client.rs", "pub fn ship() {}\n");

    assert_abstains(
        &fixture.judge("src/vendor_client.rs"),
        "`vendor` is a path component in the upstream pattern, not a substring of a basename",
    );
}

#[test]
fn cargo_lock_is_generated_by_path() {
    let fixture = Fixture::new("cargo-lock");
    fixture.file("Cargo.lock", "version = 4\n");

    let verdict = fixture.judge("Cargo.lock");
    let evidence = ineligible(&verdict, ContentClass::Provenance);
    match evidence {
        ContentEvidence::LinguistGenerated { predicate, via, .. } => {
            assert_eq!(*predicate, "cargo_lock?");
            assert_eq!(*via, GeneratedVia::Path);
        }
        other => panic!("expected LinguistGenerated, got {other:?}"),
    }
}

#[test]
fn a_go_file_carrying_the_do_not_edit_marker_is_generated() {
    let fixture = Fixture::new("generated-go");
    fixture.file(
        "api/user.pb.go",
        "// Code generated by protoc-gen-go. DO NOT EDIT.\n\npackage api\n",
    );

    let verdict = fixture.judge("api/user.pb.go");
    let evidence = ineligible(&verdict, ContentClass::Provenance);
    match evidence {
        ContentEvidence::LinguistGenerated { predicate, via, .. } => {
            assert_eq!(*predicate, "generated_go?");
            assert_eq!(*via, GeneratedVia::Content);
        }
        other => panic!("expected LinguistGenerated, got {other:?}"),
    }
}

#[test]
fn a_hand_written_go_file_of_the_same_shape_is_not_generated() {
    let fixture = Fixture::new("handwritten-go");
    fixture.file("api/user.go", "package api\n\ntype User struct{}\n");

    assert_abstains(
        &fixture.judge("api/user.go"),
        "no generated marker in the first forty lines",
    );
}

#[test]
fn a_minified_javascript_file_is_generated_by_average_line_length() {
    let fixture = Fixture::new("minified");
    let long = format!("!function(e){{{}}}(window);\n", "a=a+1;".repeat(40));
    fixture.file("public/app.min-bundle.js", &long);

    let verdict = fixture.judge("public/app.min-bundle.js");
    let evidence = ineligible(&verdict, ContentClass::Provenance);
    match evidence {
        ContentEvidence::LinguistGenerated { predicate, via, .. } => {
            assert_eq!(*predicate, "minified_files?");
            assert_eq!(*via, GeneratedVia::Content);
        }
        other => panic!("expected LinguistGenerated, got {other:?}"),
    }
}

#[test]
fn the_same_javascript_unminified_is_not_generated() {
    let fixture = Fixture::new("unminified");
    let short: String = std::iter::repeat_n("a = a + 1;\n", 40).collect();
    fixture.file("public/app-bundle.js", &short);

    assert_abstains(
        &fixture.judge("public/app-bundle.js"),
        "average line length is far under Linguist's 110-character threshold",
    );
}

#[test]
fn a_gitattributes_linguist_vendored_line_marks_the_tree_it_names() {
    let fixture = Fixture::new("attr-vendored");
    fixture
        .file(".gitattributes", "libs/** linguist-vendored\n")
        .file("libs/acme/acme.js", "export const acme = 1;\n");

    let verdict = fixture.judge("libs/acme/acme.js");
    let evidence = ineligible(&verdict, ContentClass::Provenance);
    match evidence {
        ContentEvidence::GitAttribute {
            attribute,
            pattern,
            declared_in,
        } => {
            assert_eq!(*attribute, "linguist-vendored");
            assert_eq!(pattern, "libs/**");
            assert_eq!(declared_in, Path::new(".gitattributes"));
        }
        other => panic!("expected GitAttribute, got {other:?}"),
    }
}

#[test]
fn a_nested_gitattributes_is_read_as_well_as_the_root_one() {
    let fixture = Fixture::new("attr-nested");
    fixture
        .file(".gitattributes", "*.md text\n")
        .file(
            "apps/web/.gitattributes",
            "generated/* linguist-generated\n",
        )
        .file("apps/web/generated/schema.ts", "export type Q = never;\n");

    let verdict = fixture.judge("apps/web/generated/schema.ts");
    let evidence = ineligible(&verdict, ContentClass::Provenance);
    match evidence {
        ContentEvidence::GitAttribute {
            attribute,
            declared_in,
            ..
        } => {
            assert_eq!(*attribute, "linguist-generated");
            assert_eq!(declared_in, Path::new("apps/web/.gitattributes"));
        }
        other => panic!("expected GitAttribute, got {other:?}"),
    }
}

#[test]
fn unsetting_the_attribute_deeper_in_the_tree_rescues_the_path() {
    let fixture = Fixture::new("attr-unset");
    fixture
        .file(".gitattributes", "libs/** linguist-vendored\n")
        .file("libs/keep/.gitattributes", "* -linguist-vendored\n")
        .file("libs/keep/ours.js", "export const ours = 1;\n")
        .file("libs/other/theirs.js", "export const theirs = 1;\n");

    assert_abstains(
        &fixture.judge("libs/keep/ours.js"),
        "the deeper .gitattributes unsets the attribute the root one set",
    );
    ineligible(
        &fixture.judge("libs/other/theirs.js"),
        ContentClass::Provenance,
    );
}

#[test]
fn a_later_line_about_a_different_attribute_does_not_cancel_an_earlier_one() {
    // Git resolves attributes independently: the last line to mention
    // *linguist-vendored* decides linguist-vendored, and a later line about
    // `filter` has no bearing on it. Resolving them as one stack silently
    // rescues everything a repository happens to mention after the line that
    // marked it.
    let fixture = Fixture::new("attr-independent");
    fixture
        .file(
            ".gitattributes",
            "libs/** linguist-vendored\n*.rs -filter\n*.js text\n",
        )
        .file("libs/acme/acme.rs", "pub fn acme() {}\n")
        .file("libs/acme/acme.js", "export const acme = 1;\n");

    for name in ["libs/acme/acme.rs", "libs/acme/acme.js"] {
        let verdict = fixture.judge(name);
        match ineligible(&verdict, ContentClass::Provenance) {
            ContentEvidence::GitAttribute { attribute, .. } => {
                assert_eq!(*attribute, "linguist-vendored", "for {name}");
            }
            other => panic!("{name}: expected GitAttribute, got {other:?}"),
        }
    }
}

#[test]
fn a_trailing_slash_attribute_pattern_does_not_reach_the_files_inside() {
    let fixture = Fixture::new("attr-trailing-slash");
    fixture
        .file(".gitattributes", "libs/ linguist-vendored\n")
        .file("libs/acme/acme.js", "export const acme = 1;\n");

    assert_abstains(
        &fixture.judge("libs/acme/acme.js"),
        "gitattributes patterns that match a directory do not recursively match inside it",
    );
}

#[test]
fn an_lfs_tracked_path_is_ineligible() {
    let fixture = Fixture::new("lfs");
    fixture
        .file(
            ".gitattributes",
            "*.psd filter=lfs diff=lfs merge=lfs -text\n",
        )
        .file("design/logo.psd", "\u{1}\u{2}\u{3}");

    let verdict = fixture.judge("design/logo.psd");
    let evidence = ineligible(&verdict, ContentClass::Provenance);
    match evidence {
        ContentEvidence::GitAttribute { attribute, .. } => assert_eq!(*attribute, "filter=lfs"),
        other => panic!("expected GitAttribute, got {other:?}"),
    }
}

#[test]
fn a_path_declared_in_gitmodules_is_ineligible() {
    let fixture = Fixture::new("submodule");
    fixture
        .file(
            ".gitmodules",
            "[submodule \"deps/openssl\"]\n\tpath = deps/openssl\n\turl = https://example.invalid/openssl.git\n",
        )
        .file("deps/openssl/crypto/aes.c", "int aes(void) { return 0; }\n");

    let verdict = fixture.judge("deps/openssl/crypto/aes.c");
    let evidence = ineligible(&verdict, ContentClass::Provenance);
    match evidence {
        ContentEvidence::Submodule { declared_in } => {
            assert_eq!(declared_in, Path::new(".gitmodules"));
        }
        other => panic!("expected Submodule, got {other:?}"),
    }
}

#[test]
fn a_truncated_attributes_walk_vetoes_every_candidate() {
    let fixture = Fixture::new("truncated-walk");
    fixture
        .file("src/a.rs", "pub fn a() {}\n")
        .file("src/b/c.rs", "pub fn c() {}\n")
        .file("src/b/d/e.rs", "pub fn e() {}\n");

    let gate = ContentGate::build_with_entry_limit(fixture.root(), 1).expect("build gate");
    assert!(
        gate.attributes_incomplete(),
        "a walk limited to one entry cannot have read the whole tree"
    );

    let verdict = gate.judge(Path::new("src/a.rs")).expect("judge candidate");
    let evidence = ineligible(&verdict, ContentClass::Provenance);
    match evidence {
        ContentEvidence::AttributesIncomplete { limit, .. } => assert_eq!(*limit, 1),
        other => panic!("expected AttributesIncomplete, got {other:?}"),
    }

    let complete = ContentGate::build(fixture.root()).expect("build gate");
    assert!(!complete.attributes_incomplete());
    assert_abstains(
        &complete.judge(Path::new("src/a.rs")).expect("judge"),
        "the same file, once the walk can finish",
    );
}

// ---------------------------------------------------------------------------
// 1k — migrations. Categorically ineligible (§6.12).
// ---------------------------------------------------------------------------

#[test]
fn the_newest_django_migration_is_ineligible_and_is_named_the_newest() {
    let fixture = Fixture::new("django-newest");
    fixture
        .file("myapp/migrations/__init__.py", "")
        .file("myapp/migrations/0001_initial.py", "# initial\n")
        .file(
            "myapp/migrations/0041_add_email.py",
            "dependencies = [('myapp', '0001_initial')]\n",
        )
        .file(
            "myapp/migrations/0042_add_index.py",
            "dependencies = [('myapp', '0041_add_email')]\n",
        );

    let verdict = fixture.judge("myapp/migrations/0042_add_index.py");
    let evidence = ineligible(&verdict, ContentClass::Migration);
    match evidence {
        ContentEvidence::OrderedSequence {
            scheme,
            ordinal,
            rank,
        } => {
            assert_eq!(*scheme, SequenceScheme::ZeroPaddedOrdinal);
            assert_eq!(ordinal, "0042");
            assert_eq!(
                *rank,
                SequenceRank::Newest,
                "0042 is the newest of 0001/0041/0042 — the one nothing references"
            );
        }
        other => panic!("expected OrderedSequence, got {other:?}"),
    }
}

#[test]
fn an_earlier_migration_in_the_same_sequence_is_ineligible_but_ranked_earlier() {
    let fixture = Fixture::new("django-earlier");
    fixture
        .file("myapp/migrations/0001_initial.py", "# initial\n")
        .file("myapp/migrations/0002_add_index.py", "# second\n");

    let verdict = fixture.judge("myapp/migrations/0001_initial.py");
    let evidence = ineligible(&verdict, ContentClass::Migration);
    match evidence {
        ContentEvidence::OrderedSequence { rank, ordinal, .. } => {
            assert_eq!(ordinal, "0001");
            assert_eq!(*rank, SequenceRank::Earlier);
        }
        other => panic!("expected OrderedSequence, got {other:?}"),
    }
}

#[test]
fn a_rails_timestamp_migration_is_ineligible() {
    let fixture = Fixture::new("rails");
    fixture
        .file("db/migrate/20230115120000_create_users.rb", "# users\n")
        .file("db/migrate/20240817093012_add_index.rb", "# index\n");

    let verdict = fixture.judge("db/migrate/20240817093012_add_index.rb");
    let evidence = ineligible(&verdict, ContentClass::Migration);
    match evidence {
        ContentEvidence::OrderedSequence {
            scheme,
            ordinal,
            rank,
        } => {
            assert_eq!(*scheme, SequenceScheme::Timestamp);
            assert_eq!(ordinal, "20240817093012");
            assert_eq!(*rank, SequenceRank::Newest);
        }
        other => panic!("expected OrderedSequence, got {other:?}"),
    }
}

#[test]
fn a_flyway_versioned_script_is_ineligible() {
    let fixture = Fixture::new("flyway");
    fixture
        .file("sql/V1__baseline.sql", "create table t (id int);\n")
        .file("sql/V2_1__add_column.sql", "alter table t add c int;\n");

    let verdict = fixture.judge("sql/V2_1__add_column.sql");
    let evidence = ineligible(&verdict, ContentClass::Migration);
    match evidence {
        ContentEvidence::OrderedSequence {
            scheme,
            ordinal,
            rank,
        } => {
            assert_eq!(*scheme, SequenceScheme::FlywayVersion);
            assert_eq!(ordinal, "2_1");
            assert_eq!(*rank, SequenceRank::Newest);
        }
        other => panic!("expected OrderedSequence, got {other:?}"),
    }
}

#[test]
fn an_alembic_hash_named_revision_is_ineligible_because_of_its_directory() {
    let fixture = Fixture::new("alembic");
    fixture.file(
        "migrations/versions/8f3a1c92be04_add_users.py",
        "down_revision = 'a1b2c3d4e5f6'\n",
    );

    let verdict = fixture.judge("migrations/versions/8f3a1c92be04_add_users.py");
    let evidence = ineligible(&verdict, ContentClass::Migration);
    match evidence {
        ContentEvidence::MigrationDirectory { component, .. } => {
            assert_eq!(component, "migrations");
        }
        other => panic!("expected MigrationDirectory, got {other:?}"),
    }
}

#[test]
fn a_migration_inside_a_vendored_tree_reports_the_vendored_class() {
    let fixture = Fixture::new("order-1j-before-1k");
    fixture.file(
        "vendor/acme/migrations/0007_thing.py",
        "dependencies = []\n",
    );

    let verdict = fixture.judge("vendor/acme/migrations/0007_thing.py");
    ineligible(&verdict, ContentClass::Provenance);
}

// ---------------------------------------------------------------------------
// 1i — legal.
// ---------------------------------------------------------------------------

#[test]
fn the_canonical_legal_filenames_are_ineligible() {
    let fixture = Fixture::new("legal-names");
    for name in [
        "LICENSE",
        "LICENSE.md",
        "LICENSE-MIT",
        "LICENCE.txt",
        "COPYING",
        "COPYING.LESSER",
        "NOTICE",
        "AUTHORS",
        "CONTRIBUTORS",
        "PATENTS",
        "UNLICENSE",
        "THIRD_PARTY_NOTICES.txt",
        "THIRD-PARTY-NOTICES",
    ] {
        fixture.file(name, "Permission is hereby granted…\n");
        let verdict = fixture.judge(name);
        let evidence = ineligible(&verdict, ContentClass::Legal);
        assert!(
            matches!(evidence, ContentEvidence::LegalDocument { .. }),
            "{name}: expected LegalDocument, got {evidence:?}"
        );
    }
}

#[test]
fn two_byte_identical_per_package_licences_are_both_ineligible() {
    let fixture = Fixture::new("legal-duplicates");
    let text = "MIT License\n\nCopyright (c) 2026 Example\n";
    fixture
        .file("packages/alpha/LICENSE", text)
        .file("packages/beta/LICENSE", text);

    // §6.15: 6 of 6 content-identical groups measured on a real repository were
    // unsafe to delete, and an identical LICENSE per package is a legal
    // requirement rather than duplication. A deduplicator that keeps one copy is
    // removing a compliance artifact from the other package.
    ineligible(
        &fixture.judge("packages/alpha/LICENSE"),
        ContentClass::Legal,
    );
    ineligible(&fixture.judge("packages/beta/LICENSE"), ContentClass::Legal);
}

#[test]
fn an_sbom_is_ineligible() {
    let fixture = Fixture::new("sbom");
    for name in ["sbom.spdx.json", "bom.cdx.json", "manifest.spdx"] {
        fixture.file(name, "{}\n");
        ineligible(&fixture.judge(name), ContentClass::Legal);
    }
}

#[test]
fn an_spdx_header_makes_a_source_file_ineligible() {
    let fixture = Fixture::new("spdx-header");
    fixture.file(
        "src/driver.c",
        "// SPDX-License-Identifier: GPL-2.0-only\n#include <linux/module.h>\n",
    );

    let verdict = fixture.judge("src/driver.c");
    let evidence = ineligible(&verdict, ContentClass::Legal);
    match evidence {
        ContentEvidence::SpdxHeader { line, declaration } => {
            assert_eq!(*line, 1);
            assert_eq!(declaration, "GPL-2.0-only");
        }
        other => panic!("expected SpdxHeader, got {other:?}"),
    }
}

#[test]
fn a_document_about_licensing_is_not_a_licence() {
    let fixture = Fixture::new("legal-lookalike");
    fixture.file(
        "docs/how-we-license.md",
        "# How we license\n\nWe use MIT.\n",
    );

    assert_abstains(
        &fixture.judge("docs/how-we-license.md"),
        "prose about licensing is not a licence file",
    );
}

// ---------------------------------------------------------------------------
// 1h — session and scratch state.
// ---------------------------------------------------------------------------

#[test]
fn an_r_session_workspace_is_ineligible() {
    let fixture = Fixture::new("r-session");
    for name in [".RData", ".Rhistory", "analysis/.RData"] {
        fixture.file(name, "binary-ish\n");
        let verdict = fixture.judge(name);
        let evidence = ineligible(&verdict, ContentClass::SessionState);
        assert!(
            matches!(evidence, ContentEvidence::SessionArtifact { .. }),
            "{name}: expected SessionArtifact, got {evidence:?}"
        );
    }
}

#[test]
fn backup_and_reject_suffixes_are_ineligible() {
    let fixture = Fixture::new("backups");
    for name in [
        "src/main.rs.bak",
        "config/settings.py.orig",
        "src/lib.rs.rej",
        "notes.txt~",
    ] {
        fixture.file(name, "possibly the last copy\n");
        ineligible(&fixture.judge(name), ContentClass::SessionState);
    }
}

#[test]
fn editor_state_directories_are_ineligible() {
    let fixture = Fixture::new("editor-state");
    for name in [
        ".history/src/main.rs",
        ".ipynb_checkpoints/notebook-checkpoint.ipynb",
        ".vs/solution.suo",
    ] {
        fixture.file(name, "{}\n");
        ineligible(&fixture.judge(name), ContentClass::SessionState);
    }
}

#[test]
fn the_editor_directories_linguist_already_claims_are_reported_as_1j() {
    // Not a quirk to work around: §9.12 puts the vendored/generated
    // classification first as a hard exclusion, and Linguist's own files list
    // both of these — `.idea/` in generated.rb's `intellij_file?`, `.vscode/`
    // in vendor.yml. The verdict is identical either way; only the class
    // reported differs, and 1h deliberately does not restate them.
    let fixture = Fixture::new("editor-state-1j");
    fixture
        .file(".idea/workspace.xml", "<project/>\n")
        .file(".vscode/settings.json", "{}\n");

    let idea = fixture.judge(".idea/workspace.xml");
    match ineligible(&idea, ContentClass::Provenance) {
        ContentEvidence::LinguistGenerated { predicate, .. } => {
            assert_eq!(*predicate, "intellij_file?");
        }
        other => panic!("expected LinguistGenerated, got {other:?}"),
    }

    let code = fixture.judge(".vscode/settings.json");
    match ineligible(&code, ContentClass::Provenance) {
        ContentEvidence::LinguistVendored { pattern } => {
            assert_eq!(*pattern, "(^|/)\\.vscode/");
        }
        other => panic!("expected LinguistVendored, got {other:?}"),
    }
}

#[test]
fn a_dated_document_is_not_a_migration() {
    // The regression that produced the leading-zero requirement on the ordinal
    // scheme: `2026-01.pdf` in an upload directory, and a dated report, both
    // read as zero-padded migration ordinals before it.
    let fixture = Fixture::new("dated-documents");
    fixture
        .file("docs/2026-07-31-handoff.md", "# handoff\n")
        .file("reports/20260131-summary.md", "# summary\n");

    for name in ["docs/2026-07-31-handoff.md", "reports/20260131-summary.md"] {
        assert_abstains(
            &fixture.judge(name),
            "a date in a filename is not a position in a migration sequence",
        );
    }
}

#[test]
fn a_document_named_backup_is_not_a_backup_file() {
    let fixture = Fixture::new("backup-lookalike");
    fixture.file("docs/backup.md", "# Backup runbook\n");

    assert_abstains(
        &fixture.judge("docs/backup.md"),
        "the class is a suffix on a real file, not a word in a name",
    );
}

// ---------------------------------------------------------------------------
// 1g — user-generated and uploaded content.
// ---------------------------------------------------------------------------

#[test]
fn the_magento_media_tree_is_ineligible() {
    let fixture = Fixture::new("magento-media");
    fixture.file("media/catalog/product/a/b/widget.jpg", "\u{ff}\u{d8}\u{ff}");

    let verdict = fixture.judge("media/catalog/product/a/b/widget.jpg");
    let evidence = ineligible(&verdict, ContentClass::UserContent);
    assert!(
        matches!(evidence, ContentEvidence::UploadPath { .. }),
        "expected UploadPath, got {evidence:?}"
    );
}

#[test]
fn framework_upload_roots_are_ineligible() {
    let fixture = Fixture::new("upload-roots");
    for name in [
        "storage/app/invoices/2026-01.pdf",
        "wp-content/uploads/2026/01/photo.png",
        "public/uploads/avatar.png",
        "public/system/attachments/1/original.pdf",
        "var/uploads/scan.tiff",
    ] {
        fixture.file(name, "user data\n");
        ineligible(&fixture.judge(name), ContentClass::UserContent);
    }
}

#[test]
fn a_rust_module_named_media_is_not_user_content() {
    let fixture = Fixture::new("media-lookalike");
    fixture.file("src/media.rs", "pub fn transcode() {}\n");

    assert_abstains(
        &fixture.judge("src/media.rs"),
        "the class is a directory of uploads, not a file whose stem is `media`",
    );
}

// ---------------------------------------------------------------------------
// general contract
// ---------------------------------------------------------------------------

#[test]
fn an_ordinary_source_file_abstains() {
    let fixture = Fixture::new("ordinary");
    fixture.file("src/interpreter.rs", "pub fn run() {}\n");

    assert_abstains(
        &fixture.judge("src/interpreter.rs"),
        "Gate 1 classes 1g–1k have nothing to say about ordinary source",
    );
}

#[test]
fn provenance_runs_only_class_1j() {
    let fixture = Fixture::new("provenance-only");
    fixture
        .file("LICENSE", "MIT\n")
        .file("node_modules/dep/index.js", "module.exports = 1;\n");

    let gate = fixture.gate();

    assert_abstains(
        &gate.provenance(Path::new("LICENSE")).expect("provenance"),
        "a licence is 1i, and the 1j hard exclusion must not claim it",
    );
    ineligible(
        &gate.judge(Path::new("LICENSE")).expect("judge"),
        ContentClass::Legal,
    );
    ineligible(
        &gate
            .provenance(Path::new("node_modules/dep/index.js"))
            .expect("provenance"),
        ContentClass::Provenance,
    );
}

#[test]
fn evidence_display_names_the_rule_that_fired() {
    let fixture = Fixture::new("display");
    fixture.file("node_modules/dep/index.js", "module.exports = 1;\n");

    let verdict = fixture.judge("node_modules/dep/index.js");
    let rendered = ineligible(&verdict, ContentClass::Provenance).to_string();
    assert!(
        rendered.contains("(^|/)node_modules/"),
        "evidence must quote the upstream rule, got {rendered:?}"
    );
}

#[test]
fn an_absolute_path_inside_the_repo_is_accepted() {
    let fixture = Fixture::new("absolute");
    fixture.file("myapp/migrations/0003_thing.py", "# three\n");

    let absolute = fixture.root().join("myapp/migrations/0003_thing.py");
    let verdict = fixture
        .gate()
        .judge(&absolute)
        .expect("judge absolute candidate");
    ineligible(&verdict, ContentClass::Migration);
}

#[test]
fn a_path_that_no_longer_exists_still_gets_its_path_verdict() {
    let fixture = Fixture::new("missing");
    fixture.file("src/main.rs", "fn main() {}\n");
    let gate = fixture.gate();

    ineligible(
        &gate
            .judge(Path::new("node_modules/gone/index.js"))
            .expect("judge missing vendored path"),
        ContentClass::Provenance,
    );
    assert_abstains(
        &gate
            .judge(Path::new("src/gone.rs"))
            .expect("judge missing ordinary path"),
        "a file that is not there has no content to sniff, and 1p owns non-existence",
    );
}

#[test]
fn an_upstream_pattern_anchored_at_both_ends_names_one_file_and_not_a_prefix() {
    // Regression. `^rebar$` names the Erlang build script at the repository
    // root and nothing else. Translating it as a prefix vetoed every `rebar.*`
    // in the tree — 125 wrong hits in a 46,000-path differential against the
    // upstream regexes, which is how it was found.
    let fixture = Fixture::new("anchored-both-ends");
    fixture
        .file("rebar", "#!/usr/bin/env escript\n")
        .file("rebar.rs", "pub fn rebar() {}\n")
        .file("tools/rebar", "#!/usr/bin/env escript\n");

    ineligible(&fixture.judge("rebar"), ContentClass::Provenance);
    assert_abstains(
        &fixture.judge("rebar.rs"),
        "`^rebar$` is anchored at both ends: `rebar.rs` is a different file",
    );
    assert_abstains(
        &fixture.judge("tools/rebar"),
        "`^rebar$` is anchored at the repository root, not at any component",
    );
}

#[test]
fn the_vendored_rule_table_accounts_for_every_upstream_pattern() {
    // The point of vendoring rather than inventing (§6.3): the table has to be
    // provably the upstream file, and the residue it cannot express has to be
    // visible rather than quietly dropped.
    let (supported, unsupported, total) = judged_core::gate1::content::vendor_rule_census();
    assert_eq!(
        supported + unsupported,
        total,
        "every upstream vendor.yml pattern is either matched or declared unsupported"
    );
    assert!(
        supported > unsupported,
        "expected the supported majority of the upstream file, got {supported} of {total}"
    );
}

#[test]
fn the_generated_predicate_table_accounts_for_every_upstream_predicate() {
    let (supported, unsupported, total) = judged_core::gate1::content::generated_rule_census();
    assert_eq!(supported + unsupported, total);
    assert!(
        supported > unsupported,
        "expected the supported majority of generated.rb, got {supported} of {total}"
    );
}
