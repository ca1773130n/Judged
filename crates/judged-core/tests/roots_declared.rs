//! The committed declared-roots file, §5.1 Tier C and §5.3.
//!
//! These tests are the specification of `.judged/roots.toml`: what parses, what
//! is refused, what a declaration materializes into, and — the part §5.3 says
//! nothing in the survey has — what the ruleset says about *itself* every run.
//!
//! Everything here goes through the public API only. A declared root that is
//! only correct when observed from inside the module is not a declared root.

use std::cell::RefCell;

use judged_core::roots::declared::{
    DeclaredRoots, RootRot, SuppressionKind, SuppressionStatus, Tier, ROOTS_FILE,
};

/// A file exercising every field, written the way a human would actually write
/// one — comments, blank lines, a mandatory reason on each entry.
const WELL_FORMED: &str = r#"
# Tier C: solicited, because no static analysis can find these (§5.1).

[[root]]
path = "app/jobs/**"
reason = "Sidekiq queues hold serialized class names; nothing in-repo names them"
kind = "external"
status = "accepted"

[[root]]
path = "reporting/apps.py"
reason = "Django AppConfig, found via INSTALLED_APPS; no source reference exists"
kind = "external"
status = "accepted"
expires = "2026-12-31"
"#;

fn parse(text: &str) -> DeclaredRoots {
    DeclaredRoots::parse(text).expect("fixture must parse")
}

/// `has_expired` cannot be imported here: `judged-ratchet` depends on
/// `judged-core`, so `judged-core` naming it back is a package cycle cargo
/// refuses. The predicate is therefore a parameter, and these tests supply a
/// stand-in — they pin the *plumbing* (that the answer is asked for, and that
/// it drives the verdict), never a second copy of the date rule.
fn never_expired(_expires: &str, _now: &str) -> bool {
    false
}

// ---------------------------------------------------------------------------
// Parsing: what a committed file may say
// ---------------------------------------------------------------------------

#[test]
fn a_committed_roots_file_parses_every_field() {
    let roots = parse(WELL_FORMED);
    let entries = roots.entries();
    assert_eq!(entries.len(), 2);

    assert_eq!(entries[0].pathspec, "app/jobs/**");
    assert_eq!(
        entries[0].reason,
        "Sidekiq queues hold serialized class names; nothing in-repo names them"
    );
    assert_eq!(entries[0].kind, SuppressionKind::External);
    assert_eq!(entries[0].status, SuppressionStatus::Accepted);
    assert_eq!(entries[0].expires, None);
    // The line a human jumps to is the `[[root]]` header, not the last key.
    assert_eq!(entries[0].line, 4);

    assert_eq!(entries[1].pathspec, "reporting/apps.py");
    assert_eq!(entries[1].expires.as_deref(), Some("2026-12-31"));
    assert_eq!(entries[1].line, 10);
}

#[test]
fn the_file_lives_where_it_is_reviewed_with_the_code() {
    // §5.3 point 4, cargo-machete's insight: manifest-colocated and committed,
    // so the declaration lands in the same PR as the code it protects. A path
    // under `~/.config` or `.git/` would be reviewed by nobody.
    assert_eq!(ROOTS_FILE, ".judged/roots.toml");
}

#[test]
fn an_absent_declaration_file_is_not_an_error() {
    // A repo with no Tier C roots is a normal repo, not a broken one. Absent
    // file and empty file must both mean "nothing declared" rather than fail.
    assert!(DeclaredRoots::parse("").unwrap().entries().is_empty());
    assert!(DeclaredRoots::parse("# nothing yet\n")
        .unwrap()
        .entries()
        .is_empty());
}

#[test]
fn every_entry_must_carry_a_reason() {
    // §5.3's first addition. An entry without one is unreviewable: the next
    // person cannot tell a live constraint from an abandoned workaround.
    let err = DeclaredRoots::parse(
        "[[root]]\npath = \"a.rb\"\nkind = \"external\"\nstatus = \"accepted\"\n",
    )
    .expect_err("a reasonless entry must not parse");
    assert_eq!(err.line, 1);
    let text = err.to_string();
    assert!(text.contains("reason"), "{text}");
    assert!(text.contains(ROOTS_FILE), "{text}");
}

#[test]
fn a_misspelled_key_is_refused_rather_than_ignored() {
    // The failure this prevents: `raeson` is dropped silently, the entry then
    // has no reason, and the mandatory-reason rule above is defeated by a typo.
    let err = DeclaredRoots::parse(
        "[[root]]\npath = \"a.rb\"\nraeson = \"x\"\nkind = \"external\"\nstatus = \"accepted\"\n",
    )
    .expect_err("an unknown key must not parse");
    assert_eq!(err.line, 3);
    assert!(err.to_string().contains("raeson"), "{err}");
}

#[test]
fn kind_and_status_take_sarif_spellings_and_nothing_else() {
    // §5.3 point 2 takes SARIF's suppression object verbatim, which means its
    // enumerations too. Accepting `insource` or `Accepted` would fork the
    // vocabulary from the spec the rest of the pipeline is held to (§9.2).
    for (bad, offending_line) in [
        (
            "[[root]]\npath = \"a\"\nreason = \"r\"\nkind = \"insource\"\nstatus = \"accepted\"\n",
            4,
        ),
        (
            "[[root]]\npath = \"a\"\nreason = \"r\"\nkind = \"external\"\nstatus = \"Accepted\"\n",
            5,
        ),
        (
            "[[root]]\npath = \"a\"\nreason = \"r\"\nkind = \"external\"\nstatus = \"muted\"\n",
            5,
        ),
    ] {
        let err = DeclaredRoots::parse(bad).expect_err("must reject non-SARIF spelling");
        assert_eq!(err.line, offending_line, "{err}");
    }

    let ok = parse(
        "[[root]]\npath = \"a\"\nreason = \"r\"\nkind = \"inSource\"\nstatus = \"underReview\"\n",
    );
    assert_eq!(ok.entries()[0].kind, SuppressionKind::InSource);
    assert_eq!(ok.entries()[0].status, SuppressionStatus::UnderReview);
}

#[test]
fn structural_mistakes_name_their_line() {
    for (bad, offending_line, needle) in [
        // A key that belongs to no entry: silently attaching it to the next
        // `[[root]]` would move a reason onto a declaration that never had one.
        ("path = \"a\"\n[[root]]\n", 1, "[[root]]"),
        // Some other table. TOML would accept it; we must not, because an
        // entry written under the wrong header protects nothing and says so
        // nowhere.
        ("[[roots]]\npath = \"a\"\n", 1, "roots"),
        ("[package]\nname = \"x\"\n", 1, "package"),
        // A repeated key: TOML forbids it, and the second value silently
        // winning would let a stale reason outlive its replacement.
        (
            "[[root]]\npath = \"a\"\npath = \"b\"\nreason = \"r\"\nkind = \"external\"\nstatus = \"accepted\"\n",
            3,
            "path",
        ),
        // Bare and single-quoted values are TOML but not this subset; guessing
        // at them is how a parser starts disagreeing with the spec it apes.
        (
            "[[root]]\npath = a.rb\nreason = \"r\"\nkind = \"external\"\nstatus = \"accepted\"\n",
            2,
            "quoted",
        ),
        // A path is what an entry is about; without one there is nothing to
        // match and nothing to lint.
        (
            "[[root]]\nreason = \"r\"\nkind = \"external\"\nstatus = \"accepted\"\n",
            1,
            "path",
        ),
    ] {
        let err = match DeclaredRoots::parse(bad) {
            Err(err) => err,
            Ok(_) => panic!("expected a parse error for {bad:?}"),
        };
        assert_eq!(err.line, offending_line, "{bad:?} -> {err}");
        assert!(err.to_string().contains(needle), "{bad:?} -> {err}");
    }
}

#[test]
fn comments_and_escapes_do_not_corrupt_a_reason() {
    // A reason is prose written by a human under review pressure. `#` and `"`
    // turn up in it constantly, and a parser that eats them silently truncates
    // the one field §5.3 makes mandatory.
    let roots = parse(concat!(
        "[[root]]  # the legacy queue\n",
        "path = \"a.rb\"\n",
        "reason = \"issue #215914: Zeitwerk cannot see a \\\"type\\\" column\"\n",
        "kind = \"external\"\n",
        "status = \"accepted\"\n",
    ));
    assert_eq!(
        roots.entries()[0].reason,
        r#"issue #215914: Zeitwerk cannot see a "type" column"#
    );
}

#[test]
fn an_unterminated_or_unknown_escape_is_refused() {
    for bad in [
        "[[root]]\npath = \"a\nreason = \"r\"\nkind = \"external\"\nstatus = \"accepted\"\n",
        "[[root]]\npath = \"a\\q\"\nreason = \"r\"\nkind = \"external\"\nstatus = \"accepted\"\n",
    ] {
        let err = DeclaredRoots::parse(bad).expect_err("must not parse");
        assert_eq!(err.line, 2, "{err}");
    }
}

// ---------------------------------------------------------------------------
// Matching: gitignore pathspec semantics (§5.3 point 1)
// ---------------------------------------------------------------------------

/// Build a one-entry ruleset around `pathspec` and report which candidates it
/// turns into roots.
fn matched(pathspec: &str, candidates: &[&str]) -> Vec<String> {
    let text = format!(
        "[[root]]\npath = \"{pathspec}\"\nreason = \"r\"\nkind = \"external\"\nstatus = \"accepted\"\n"
    );
    parse(&text)
        .materialize(candidates)
        .into_iter()
        .map(|seed| seed.path)
        .collect()
}

#[test]
fn a_single_star_stops_at_a_directory_separator() {
    let files = ["app/a.rb", "app/sub/b.rb", "other/c.rb"];
    assert_eq!(matched("app/*.rb", &files), vec!["app/a.rb"]);
}

#[test]
fn a_double_star_crosses_directory_separators() {
    let files = ["app/a.rb", "app/sub/deep/b.rb", "other/c.rb"];
    assert_eq!(
        matched("app/**", &files),
        vec!["app/a.rb", "app/sub/deep/b.rb"]
    );
}

#[test]
fn a_pattern_without_a_slash_matches_at_any_depth() {
    // gitignore's rule, and the reason it was chosen: nobody has to learn it.
    let files = [
        "conftest.py",
        "tests/conftest.py",
        "a/b/conftest.py",
        "x.py",
    ];
    assert_eq!(
        matched("conftest.py", &files),
        vec!["a/b/conftest.py", "conftest.py", "tests/conftest.py"]
    );
}

#[test]
fn a_pattern_with_a_slash_is_anchored_to_the_repo_root() {
    let files = ["reporting/apps.py", "vendor/reporting/apps.py"];
    assert_eq!(
        matched("reporting/apps.py", &files),
        vec!["reporting/apps.py"]
    );
    // A leading slash anchors a name that would otherwise float.
    assert_eq!(
        matched("/apps.py", &["apps.py", "a/apps.py"]),
        vec!["apps.py"]
    );
}

#[test]
fn naming_a_directory_declares_everything_under_it() {
    // git's behaviour: excluding a directory excludes its contents. A user who
    // writes `app/jobs` and gets nothing would reasonably call that a bug.
    let files = ["app/jobs/mailer.rb", "app/jobs/deep/nightly.rb", "app/x.rb"];
    assert_eq!(
        matched("app/jobs", &files),
        vec!["app/jobs/deep/nightly.rb", "app/jobs/mailer.rb"]
    );
    assert_eq!(
        matched("app/jobs/", &files),
        vec!["app/jobs/deep/nightly.rb", "app/jobs/mailer.rb"]
    );
}

#[test]
fn question_marks_and_character_classes_behave_as_git_does() {
    let files = ["a1.rb", "a12.rb", "b1.rb", "a/1.rb"];
    // `?` matches one character and never the separator.
    assert_eq!(matched("a?.rb", &files), vec!["a1.rb"]);
    assert_eq!(matched("?/1.rb", &files), vec!["a/1.rb"]);
    // Ranges and negated classes.
    assert_eq!(matched("[ab]1.rb", &files), vec!["a1.rb", "b1.rb"]);
    assert_eq!(matched("[!b]1.rb", &files), vec!["a1.rb"]);
    assert_eq!(matched("a[0-9].rb", &files), vec!["a1.rb"]);
}

#[test]
fn a_later_negation_carves_a_hole_in_an_earlier_pattern() {
    // §5.3 chose gitignore partly because negations work. Last match wins.
    let roots = parse(concat!(
        "[[root]]\npath = \"app/jobs/**\"\nreason = \"queue holds class names\"\n",
        "kind = \"external\"\nstatus = \"accepted\"\n",
        "[[root]]\npath = \"!app/jobs/legacy/**\"\nreason = \"queue drained 2026-06\"\n",
        "kind = \"external\"\nstatus = \"accepted\"\n",
    ));
    let files = ["app/jobs/nightly.rb", "app/jobs/legacy/old.rb"];
    let paths: Vec<String> = roots
        .materialize(&files)
        .into_iter()
        .map(|s| s.path)
        .collect();
    assert_eq!(paths, vec!["app/jobs/nightly.rb"]);
}

// ---------------------------------------------------------------------------
// Materializing: provenance is the load-bearing field
// ---------------------------------------------------------------------------

#[test]
fn every_seed_names_its_tier_and_the_line_that_declared_it() {
    // A root that does not say where it came from invites a caller to trust a
    // guess as though a manifest had declared it. Everything this module emits
    // is Tier C by construction — solicited from a human, confidence *none*.
    let roots = parse(WELL_FORMED);
    let seeds = roots.materialize(&["app/jobs/nightly.rb", "reporting/apps.py", "unrelated.py"]);
    assert_eq!(seeds.len(), 2);

    let job = &seeds[0];
    assert_eq!(job.path, "app/jobs/nightly.rb");
    assert_eq!(job.tier, Tier::C);
    assert_eq!(job.declared_at_line, 4);
    assert_eq!(job.pathspec, "app/jobs/**");
    assert_eq!(
        job.reason,
        "Sidekiq queues hold serialized class names; nothing in-repo names them"
    );
    assert_eq!(job.kind, SuppressionKind::External);
    assert_eq!(job.status, SuppressionStatus::Accepted);

    assert_eq!(seeds[1].path, "reporting/apps.py");
    assert_eq!(seeds[1].declared_at_line, 10);
}

#[test]
fn a_rejected_declaration_materializes_no_root() {
    // SARIF §3.35: a rejected suppression is not in effect. That is the third
    // state binary keep-lists cannot express — the entry stays in the file as
    // the durable record of a decision, so nobody re-litigates it, but it
    // protects nothing.
    let roots = parse(
        "[[root]]\npath = \"a.rb\"\nreason = \"claimed live; review found the flag retired\"\nkind = \"external\"\nstatus = \"rejected\"\n",
    );
    assert_eq!(roots.entries().len(), 1, "the record survives");
    assert!(
        roots.materialize(&["a.rb"]).is_empty(),
        "a rejected declaration must not protect anything"
    );
}

#[test]
fn an_under_review_declaration_still_protects() {
    // The safe direction while a human decides. Treating "not yet reviewed" as
    // "not a root" would delete the thing the question was about.
    let roots = parse(
        "[[root]]\npath = \"a.rb\"\nreason = \"ops say a runbook calls this; confirming\"\nkind = \"external\"\nstatus = \"underReview\"\n",
    );
    let seeds = roots.materialize(&["a.rb"]);
    assert_eq!(seeds.len(), 1);
    assert_eq!(seeds[0].status, SuppressionStatus::UnderReview);
}

#[test]
fn print_seeds_shows_a_human_every_root_and_where_it_came_from() {
    // ProGuard `-printseeds`, which §9.13 asks for by name.
    let report = parse(WELL_FORMED).print_seeds(&["app/jobs/nightly.rb", "reporting/apps.py"]);
    assert!(report.contains("app/jobs/nightly.rb"), "{report}");
    assert!(report.contains("reporting/apps.py"), "{report}");
    assert!(report.contains("Sidekiq queues hold"), "{report}");
    // Tier and file:line, so a reader can go argue with the declaration.
    assert!(report.contains(&format!("{ROOTS_FILE}:4")), "{report}");
    assert!(report.contains(&format!("{ROOTS_FILE}:10")), "{report}");
    assert!(report.contains('C'), "{report}");
}

#[test]
fn print_seeds_distinguishes_no_roots_from_no_run() {
    // §6.20: "no data" must be a distinct state from "zero executions". A
    // silent empty report reads as success to a human and to a CI log.
    let report = DeclaredRoots::parse("").unwrap().print_seeds::<&str>(&[]);
    assert!(
        report.to_lowercase().contains("no declared roots"),
        "an empty seed list must say so out loud, got: {report:?}"
    );
}

// ---------------------------------------------------------------------------
// Rot: the ruleset lints itself every run (§5.3's second addition)
// ---------------------------------------------------------------------------

#[test]
fn a_ruleset_that_still_earns_its_place_lints_clean() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("reporting")).unwrap();
    std::fs::write(dir.path().join("reporting/apps.py"), "").unwrap();
    std::fs::create_dir_all(dir.path().join("app/jobs")).unwrap();
    std::fs::write(dir.path().join("app/jobs/nightly.rb"), "").unwrap();

    let rot = parse(WELL_FORMED).lint(
        &["app/jobs/nightly.rb", "reporting/apps.py"],
        dir.path(),
        "2026-08-01T00:00:00Z",
        &never_expired,
    );
    assert!(rot.is_empty(), "{rot:?}");
}

#[test]
fn lint_flags_an_entry_that_matched_nothing() {
    // Periphery's superfluous-ignore warning, generalized. A pattern matching
    // nothing is a blind spot nobody is watching.
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("app/jobs")).unwrap();

    let rot = parse(
        "[[root]]\npath = \"app/jobs/**\"\nreason = \"queue\"\nkind = \"external\"\nstatus = \"accepted\"\n",
    )
    .lint(&["src/main.rs"], dir.path(), "2026-08-01T00:00:00Z", &never_expired);

    assert_eq!(
        rot,
        vec![RootRot::MatchedNothing {
            line: 1,
            pathspec: "app/jobs/**".to_string(),
        }]
    );
}

#[test]
fn lint_flags_an_entry_fully_shadowed_by_a_later_one() {
    // The same rule, doing work a naive "did any path match?" check misses: an
    // entry every one of whose paths a later entry decides is dead weight.
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("app/jobs")).unwrap();
    std::fs::write(dir.path().join("app/jobs/x.rb"), "").unwrap();

    let rot = parse(concat!(
        "[[root]]\npath = \"app/jobs/**\"\nreason = \"first\"\nkind = \"external\"\nstatus = \"accepted\"\n",
        "[[root]]\npath = \"app/**\"\nreason = \"second, wider\"\nkind = \"external\"\nstatus = \"accepted\"\n",
    ))
    .lint(&["app/jobs/x.rb"], dir.path(), "2026-08-01T00:00:00Z", &never_expired);

    assert_eq!(
        rot,
        vec![RootRot::MatchedNothing {
            line: 1,
            pathspec: "app/jobs/**".to_string(),
        }]
    );
}

#[test]
fn lint_flags_a_literal_entry_whose_file_is_gone() {
    // Vulture's executable-whitelist property, generalized: an entry naming a
    // path that is simply not there can never match again, whatever the
    // analyzers do. Only checkable for a literal path — a glob has no referent
    // to look for, and guessing at one would invent findings.
    let dir = tempfile::tempdir().unwrap();

    let rot = parse(
        "[[root]]\npath = \"reporting/apps.py\"\nreason = \"AppConfig\"\nkind = \"external\"\nstatus = \"accepted\"\n",
    )
    .lint(&["src/main.rs"], dir.path(), "2026-08-01T00:00:00Z", &never_expired);

    assert_eq!(
        rot,
        vec![RootRot::ReferentGone {
            line: 1,
            pathspec: "reporting/apps.py".to_string(),
        }]
    );
}

#[test]
fn lint_asks_the_caller_whether_an_entry_expired_and_reports_what_it_hears() {
    // The date rule has exactly one definition, `judged_ratchet::rot::has_expired`,
    // and this crate cannot name it without creating a package cycle. So it is
    // injected. This test pins that the entry's own `expires` and the caller's
    // `now` are what get asked about, and that the answer decides the verdict —
    // which is the whole contract a second copy would be able to break.
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.rb"), "").unwrap();

    let asked = RefCell::new(Vec::new());
    let oracle = |expires: &str, now: &str| {
        asked
            .borrow_mut()
            .push((expires.to_string(), now.to_string()));
        expires == "2020-01-01"
    };

    let text = concat!(
        "[[root]]\npath = \"a.rb\"\nreason = \"r\"\nkind = \"external\"\nstatus = \"accepted\"\nexpires = \"2020-01-01\"\n",
        "[[root]]\npath = \"a.rb\"\nreason = \"r\"\nkind = \"external\"\nstatus = \"accepted\"\nexpires = \"2099-01-01\"\n",
    );
    let roots = parse(text);
    let rot = roots.lint(&["a.rb"], dir.path(), "2026-08-01T00:00:00Z", &oracle);

    let questions = asked.borrow().clone();
    assert_eq!(
        questions,
        vec![
            ("2020-01-01".to_string(), "2026-08-01T00:00:00Z".to_string()),
            ("2099-01-01".to_string(), "2026-08-01T00:00:00Z".to_string()),
        ]
    );
    // The entry at line 1 is both expired and shadowed by the one at line 7.
    // Expiry outranks the empty match, so it reports once, as expired. The
    // line-7 entry is live and decides `a.rb`, so it reports nothing.
    assert_eq!(
        rot,
        vec![RootRot::Expired {
            line: 1,
            pathspec: "a.rb".to_string(),
            expires: "2020-01-01".to_string(),
        }]
    );
}

#[test]
fn an_entry_with_no_expiry_is_never_asked_about_one() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.rb"), "").unwrap();
    let asked = RefCell::new(0usize);
    let oracle = |_: &str, _: &str| {
        *asked.borrow_mut() += 1;
        true
    };
    let rot = parse(
        "[[root]]\npath = \"a.rb\"\nreason = \"r\"\nkind = \"external\"\nstatus = \"accepted\"\n",
    )
    .lint(&["a.rb"], dir.path(), "2026-08-01T00:00:00Z", &oracle);
    assert_eq!(*asked.borrow(), 0);
    assert!(rot.is_empty(), "{rot:?}");
}

#[test]
fn one_reason_per_entry_with_the_strongest_statement_winning() {
    // Same precedence, and same argument, as `judged_ratchet::rot::detect_rot`:
    // the remediation for all three is "delete the line", and three lines per
    // entry is how a rot report becomes something people skim past.
    let dir = tempfile::tempdir().unwrap();
    let always_expired = |_: &str, _: &str| true;

    // Referent gone *and* expired *and* matching nothing: only the first.
    let rot = parse(
        "[[root]]\npath = \"gone.rb\"\nreason = \"r\"\nkind = \"external\"\nstatus = \"accepted\"\nexpires = \"2020-01-01\"\n",
    )
    .lint(&["other.rb"], dir.path(), "2026-08-01T00:00:00Z", &always_expired);
    assert_eq!(
        rot,
        vec![RootRot::ReferentGone {
            line: 1,
            pathspec: "gone.rb".to_string()
        }]
    );

    // Present, expired, and matching nothing: expiry outranks the empty match,
    // because a deadline a human set is a stronger statement than a pattern
    // that happens not to fire in this run.
    std::fs::write(dir.path().join("here.rb"), "").unwrap();
    let rot = parse(
        "[[root]]\npath = \"here.rb\"\nreason = \"r\"\nkind = \"external\"\nstatus = \"accepted\"\nexpires = \"2020-01-01\"\n",
    )
    .lint(&["other.rb"], dir.path(), "2026-08-01T00:00:00Z", &always_expired);
    assert_eq!(
        rot,
        vec![RootRot::Expired {
            line: 1,
            pathspec: "here.rb".to_string(),
            expires: "2020-01-01".to_string()
        }]
    );
}

#[test]
fn a_rejected_entry_is_linted_like_any_other() {
    // It protects nothing, but it is still a line in a committed file making a
    // claim about a path. When that path goes, the record goes with it —
    // otherwise the file accumulates decisions about code nobody has anymore.
    let dir = tempfile::tempdir().unwrap();
    let rot = parse(
        "[[root]]\npath = \"gone.rb\"\nreason = \"reviewed, not a root\"\nkind = \"external\"\nstatus = \"rejected\"\n",
    )
    .lint(&["other.rb"], dir.path(), "2026-08-01T00:00:00Z", &never_expired);
    assert_eq!(
        rot,
        vec![RootRot::ReferentGone {
            line: 1,
            pathspec: "gone.rb".to_string()
        }]
    );
}

#[test]
fn rot_reports_are_actionable_without_rerunning_anything() {
    // Every variant must render the line to open and the pattern to look at.
    let rendered = [
        RootRot::MatchedNothing {
            line: 3,
            pathspec: "app/**".into(),
        },
        RootRot::ReferentGone {
            line: 9,
            pathspec: "gone.rb".into(),
        },
        RootRot::Expired {
            line: 12,
            pathspec: "a.rb".into(),
            expires: "next quarter".into(),
        },
    ];
    for r in &rendered {
        let text = r.to_string();
        assert!(text.contains(ROOTS_FILE), "{text}");
    }
    assert!(rendered[0].to_string().contains("app/**"));
    assert!(rendered[1].to_string().contains("gone.rb"));
    // The raw expiry text is carried through, so an author can see what they
    // wrote — `next quarter` is a date nobody can evaluate, and the report is
    // where that becomes visible.
    assert!(rendered[2].to_string().contains("next quarter"));
    assert!(rendered[0].to_string().contains(":3"));
}

#[test]
fn a_path_relative_lint_does_not_escape_the_repo() {
    // An absolute or parent-relative pathspec would make the referent check
    // ask about a file outside the working tree, and would make the whole file
    // unreviewable — the reader cannot tell what it protects.
    for bad in ["/etc/passwd", "../sibling/a.rb"] {
        let text = format!(
            "[[root]]\npath = \"{bad}\"\nreason = \"r\"\nkind = \"external\"\nstatus = \"accepted\"\n"
        );
        let parsed = DeclaredRoots::parse(&text);
        if bad.starts_with("..") {
            let err = parsed.expect_err("a parent-relative pathspec must not parse");
            assert!(err.to_string().contains(".."), "{err}");
        } else {
            // A leading `/` is gitignore's anchor, not an absolute path, and
            // must resolve inside the repo.
            let roots = parsed.expect("a leading slash is an anchor");
            assert_eq!(roots.entries()[0].pathspec, "/etc/passwd");
            assert_eq!(roots.materialize(&["etc/passwd"]).len(), 1);
        }
    }
}

#[test]
fn lint_and_materialize_agree_about_what_a_repo_root_is() {
    // The referent check joins against the repo root the caller passes; a
    // candidate list is repo-relative. If these two ever disagreed, every entry
    // would report `ReferentGone` on a healthy repo — the loudest possible
    // false positive, and one a caller would learn to ignore.
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("a/b")).unwrap();
    std::fs::write(dir.path().join("a/b/c.rb"), "").unwrap();
    let roots = parse(
        "[[root]]\npath = \"a/b/c.rb\"\nreason = \"r\"\nkind = \"external\"\nstatus = \"accepted\"\n",
    );
    assert_eq!(roots.materialize(&["a/b/c.rb"]).len(), 1);
    assert!(roots
        .lint(
            &["a/b/c.rb"],
            dir.path(),
            "2026-08-01T00:00:00Z",
            &never_expired
        )
        .is_empty());
    // And the same call with a `Vec<String>`, because callers have owned paths.
    let owned: Vec<String> = vec!["a/b/c.rb".to_string()];
    assert_eq!(roots.materialize(&owned).len(), 1);
}

#[test]
fn the_roots_a_reference_veto_structurally_cannot_rescue() {
    // Why this module exists. Gate 2 vetoes a removal when some file still
    // mentions the name — and the two survivors it cannot help are the ones
    // with no textual reference to find anywhere:
    //
    //   m10  `ReportingConfig` appears only in `apps.py`, its own declaration.
    //        Django reaches it from `INSTALLED_APPS` by convention. A Jest
    //        `__mocks__` file is substituted by the runner, named by nobody.
    //   m11  ORM fields read reflectively, never spelled in source.
    //
    // No amount of needle tuning finds a needle that is not there. These are
    // root-set failures, not veto failures, and the only repair is a human
    // saying so — which is what this file is.
    let roots = parse(concat!(
        "[[root]]\npath = \"reporting/apps.py\"\n",
        "reason = \"m10: Django loads ReportingConfig from INSTALLED_APPS; the name appears nowhere but its own declaration\"\n",
        "kind = \"external\"\nstatus = \"accepted\"\n",
        "[[root]]\npath = \"__mocks__/**\"\n",
        "reason = \"m10: Jest substitutes these by filename convention; nothing imports them\"\n",
        "kind = \"external\"\nstatus = \"accepted\"\n",
        "[[root]]\npath = \"billing/models.py\"\n",
        "reason = \"m11: fields are read reflectively by the ORM, never named in source\"\n",
        "kind = \"external\"\nstatus = \"accepted\"\n",
    ));

    let candidates = [
        "reporting/apps.py",
        "__mocks__/stripe.js",
        "billing/models.py",
        "billing/unused_helper.py",
    ];
    let rescued: Vec<String> = roots
        .materialize(&candidates)
        .into_iter()
        .map(|seed| seed.path)
        .collect();
    assert_eq!(
        rescued,
        vec![
            "__mocks__/stripe.js",
            "billing/models.py",
            "reporting/apps.py"
        ],
        "the three survivors must be declared roots, and nothing else swept in with them"
    );

    // And every one of them says out loud that it is a guess a human made,
    // never a fact a manifest stated — that is the whole contract of Tier C.
    for seed in roots.materialize(&candidates) {
        assert_eq!(seed.tier, Tier::C);
        assert!(!seed.reason.is_empty());
    }
}
