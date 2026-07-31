//! End-to-end behaviour of the `judged` binary, driven as a subprocess.
//!
//! Every test here spawns the real executable against a real git working tree
//! and asserts on the **exit code** as well as on what a human would read. That
//! pairing is the point. §9.2 records that knip, vulture, ts-prune, Go deadcode
//! and Periphery all conflate "clean" with "crashed before doing anything", and
//! a test that only reads stdout cannot tell those apart either. Ruff's contract
//! — 0 clean, 1 violations, 2 abnormal termination — is what CI actually
//! branches on, so it is what these tests actually check.
//!
//! Written against the process boundary rather than against
//! [`judged_ratchet::ratchet`] or [`judged_mutants::runner::run_suite`], both of
//! which their own crates already cover. What is untested until here is the
//! wiring: whether the exit code the ratchet computed is the one the process
//! returns, and whether the report a human reads names the finding they have to
//! act on.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use judged_core::git::Repo;
use judged_ratchet::baseline::BASELINE_PATH;
use serde_json::{json, Value};
use tempfile::{Builder, TempDir};

// ---------------------------------------------------------------------------
// Scratch space
// ---------------------------------------------------------------------------

/// A throwaway directory that deletes itself when the test ends.
///
/// The label is only a prefix on the directory name, so a directory left behind
/// by a hard abort still says which case it came from.
fn scratch(label: &str) -> TempDir {
    Builder::new()
        .prefix(&format!("judged-cli-{label}-"))
        .tempdir()
        .expect("scratch directory must be creatable")
}

/// Write `body` to a path relative to `dir`, creating parents.
fn write(dir: &Path, relative: &str, body: &str) -> PathBuf {
    let path = dir.join(relative);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("parent directory must be creatable");
    }
    std::fs::write(&path, body).expect("scratch file must be writable");
    path
}

fn read(dir: &Path, relative: &str) -> String {
    std::fs::read_to_string(dir.join(relative)).expect("file must be readable")
}

// ---------------------------------------------------------------------------
// Driving the binary
// ---------------------------------------------------------------------------

/// What the process said and what it returned.
struct Run {
    code: i32,
    stdout: String,
}

impl Run {
    /// Assert an exit code, quoting the whole report when it does not match.
    /// A bare `assert_eq!(run.code, 1)` fails with two integers and no clue.
    fn expect_code(&self, expected: i32, why: &str) -> &Run {
        assert_eq!(
            self.code, expected,
            "expected exit {expected} ({why}), got {}. Report was:\n{}",
            self.code, self.stdout
        );
        self
    }

    fn expect_says(&self, needle: &str) -> &Run {
        assert!(
            self.stdout.contains(needle),
            "report should mention `{needle}`. Report was:\n{}",
            self.stdout
        );
        self
    }

    fn expect_silent_about(&self, needle: &str) -> &Run {
        assert!(
            !self.stdout.contains(needle),
            "report should NOT mention `{needle}`. Report was:\n{}",
            self.stdout
        );
        self
    }

    /// Byte offset of a needle, for asserting one thing is printed before
    /// another.
    fn offset_of(&self, needle: &str) -> usize {
        self.stdout
            .find(needle)
            .unwrap_or_else(|| panic!("`{needle}` is not in the report:\n{}", self.stdout))
    }
}

fn judged(cwd: &Path, args: &[&str]) -> Run {
    let output: Output = Command::new(env!("CARGO_BIN_EXE_judged"))
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("the judged binary must be runnable");

    let mut stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    // Usage errors and panics land on stderr; folding both in means a test that
    // asserts on the report cannot silently pass because the real message went
    // to the other stream.
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !stderr.is_empty() {
        stdout.push_str(&stderr);
    }

    Run {
        code: output.status.code().unwrap_or(-1),
        stdout,
    }
}

// ---------------------------------------------------------------------------
// SARIF fixtures
//
// Written as real JSON documents rather than as `judged_core::sarif` values,
// because what these tests exercise is the CLI's reader — including its
// tolerance for the fields a real tool omits.
// ---------------------------------------------------------------------------

/// One finding, with an adapter-supplied `judged/v1` fingerprint.
///
/// Supplying the fingerprint rather than letting the CLI derive one keeps the
/// baseline entries in these tests legible: `judged/v1:aaaa` is a join key a
/// reader can follow by eye.
fn finding(rule_id: &str, uri: &str, digest: &str, message: &str) -> Value {
    json!({
        "ruleId": rule_id,
        "level": "warning",
        "message": message,
        "locations": [{ "uri": uri, "startLine": 12 }],
        "partialFingerprints": { "judged/v1": digest },
    })
}

/// A SARIF log with one run that declares every `target` as an
/// `analysisTarget`, so the §9.2 positive control is satisfied.
fn sarif_log(
    tool: &str,
    execution_successful: bool,
    targets: &[&str],
    results: Vec<Value>,
) -> String {
    let log = json!({
        "version": "2.1.0",
        "runs": [{
            "tool": { "name": tool, "version": "1.0.0" },
            "invocations": [{ "executionSuccessful": execution_successful }],
            "artifacts": targets.iter().map(|uri| json!({
                "locationUri": uri,
                "roles": ["analysisTarget"],
            })).collect::<Vec<_>>(),
            "results": results,
        }],
    });
    serde_json::to_string_pretty(&log).expect("fixture SARIF must serialize")
}

/// One JSONL baseline line.
///
/// Snake_case, unlike the SARIF fixtures above. `BaselineEntry` carries no
/// `rename_all`, so the committed baseline is snake_case while the interchange
/// format is camelCase — the two files are read by different audiences and only
/// one of them is a standard.
fn baseline_line(fingerprint: &str, rule_id: &str, uri: &str, first_seen: &str) -> String {
    json!({
        "fingerprint": fingerprint,
        "rule_id": rule_id,
        "uri": uri,
        "first_seen": first_seen,
    })
    .to_string()
}

/// A git working tree with `src/a.ts`, `src/b.ts` and `src/c.ts` on disk.
///
/// The files have to genuinely exist: `judged_ratchet::rot` reports an entry
/// whose referent is missing as `ReferentGone`, so a fixture that only wrote
/// SARIF would make every test look like a rot test.
fn repo_with_sources(label: &str) -> TempDir {
    let scratch = scratch(label);
    Repo::init(scratch.path()).expect("scratch must be a git working tree");
    for name in ["a", "b", "c"] {
        write(
            scratch.path(),
            &format!("src/{name}.ts"),
            "export const x = 1;\n",
        );
    }
    scratch
}

// ---------------------------------------------------------------------------
// judged ratchet
// ---------------------------------------------------------------------------

#[test]
fn a_clean_ratchet_run_exits_zero() {
    let repo = repo_with_sources("clean");
    write(
        repo.path(),
        "knip.sarif",
        &sarif_log(
            "knip",
            true,
            &["src/a.ts", "src/b.ts", "src/c.ts"],
            vec![finding(
                "unused-export",
                "src/a.ts",
                "aaaa",
                "export `x` is never imported",
            )],
        ),
    );
    write(
        repo.path(),
        BASELINE_PATH,
        &format!(
            "{}\n",
            baseline_line(
                "judged/v1:aaaa",
                "unused-export",
                "src/a.ts",
                "2026-01-01T00:00:00Z"
            )
        ),
    );

    let run = judged(repo.path(), &["ratchet", "--sarif", "knip.sarif"]);

    run.expect_code(
        0,
        "every finding is already baselined and nothing has rotted",
    )
    .expect_says("clean")
    .expect_silent_about("NEW FINDINGS")
    .expect_silent_about("BASELINE ROT");
}

#[test]
fn a_relative_baseline_resolves_against_the_repository_root_not_the_working_directory() {
    // The baseline is checked in, one per repository (§9.14, following
    // `deprecation_toolkit`), so `.judged/baseline.jsonl` has to mean the same
    // file wherever the command is typed — and in a monorepo it is typed from a
    // package directory far more often than from the top. Resolved against the
    // working directory instead, it finds nothing: every already-accepted
    // finding comes back as NEW (a red build nobody can act on), and `--update`
    // writes a second baseline into a subdirectory no reviewer will ever see.
    //
    // Every other test in this file runs with the working directory at the repo
    // root, where the two resolutions are indistinguishable.
    let repo = repo_with_sources("subdir");
    write(
        repo.path(),
        "knip.sarif",
        &sarif_log(
            "knip",
            true,
            &["src/a.ts", "src/b.ts", "src/c.ts"],
            vec![finding(
                "unused-export",
                "src/a.ts",
                "aaaa",
                "export `x` is never imported",
            )],
        ),
    );
    write(
        repo.path(),
        BASELINE_PATH,
        &format!(
            "{}\n",
            baseline_line(
                "judged/v1:aaaa",
                "unused-export",
                "src/a.ts",
                "2026-01-01T00:00:00Z"
            )
        ),
    );
    let nested = repo.path().join("services/api");
    std::fs::create_dir_all(&nested).expect("subdirectory must be creatable");

    // The SARIF path is absolute, so the only path whose resolution is under
    // test here is the baseline's.
    let sarif = repo.path().join("knip.sarif");
    let sarif = sarif.to_str().expect("scratch paths are UTF-8");
    let run = judged(&nested, &["ratchet", "--sarif", sarif]);

    run.expect_code(
        0,
        "the one finding is baselined at the repository root, and the working \
         directory does not change which findings are new",
    )
    .expect_says("clean");
    // The report names the file it consulted, so a reader can tell which
    // baseline was read without inferring it from the verdict.
    let root = std::fs::canonicalize(repo.path()).expect("scratch path must canonicalize");
    run.expect_says(&root.join(BASELINE_PATH).display().to_string());

    // Writing goes to the same place reading does.
    judged(&nested, &["ratchet", "--sarif", sarif, "--update"]).expect_code(
        0,
        "rewriting the baseline is the remediation, not a failure",
    );
    assert!(
        !nested.join(BASELINE_PATH).exists(),
        "--update from a subdirectory left a second baseline at {}. Two baseline \
         files is one amnesty list nobody reviews (§9.14)",
        nested.join(BASELINE_PATH).display()
    );
    assert!(
        read(repo.path(), BASELINE_PATH).contains("judged/v1:aaaa"),
        "the repository's own baseline must be the one that was rewritten"
    );
}

#[test]
fn a_new_finding_exits_one_and_is_named_in_the_report() {
    let repo = repo_with_sources("new-finding");
    write(
        repo.path(),
        "knip.sarif",
        &sarif_log(
            "knip",
            true,
            &["src/a.ts", "src/b.ts", "src/c.ts"],
            vec![
                finding(
                    "unused-export",
                    "src/a.ts",
                    "aaaa",
                    "export `x` is never imported",
                ),
                finding(
                    "unused-file",
                    "src/b.ts",
                    "bbbb",
                    "file is not reachable from any entry point",
                ),
            ],
        ),
    );
    write(
        repo.path(),
        BASELINE_PATH,
        &format!(
            "{}\n",
            baseline_line(
                "judged/v1:aaaa",
                "unused-export",
                "src/a.ts",
                "2026-01-01T00:00:00Z"
            )
        ),
    );

    let run = judged(repo.path(), &["ratchet", "--sarif", "knip.sarif"]);

    run.expect_code(1, "one finding is not in the baseline")
        .expect_says("NEW FINDINGS")
        .expect_says("unused-file")
        .expect_says("src/b.ts")
        // The baselined finding is not the developer's problem, and printing it
        // beside the new one is how a ratchet report becomes something people
        // stop reading. §9.14: block the inflow without demanding the backlog.
        .expect_silent_about("src/a.ts");
}

#[test]
fn new_findings_are_sorted_by_rule_id_never_by_size() {
    // §9.13 invariant 3. Size is anti-correlated with safety, so ranking by it
    // puts the most dangerous candidate where a tired human's eye lands first.
    // The big file here carries the alphabetically-later rule, so a
    // size-descending report and a rule-id-ascending report disagree.
    let repo = repo_with_sources("sorted");
    write(
        repo.path(),
        "src/huge.ts",
        &"export const pad = 1;\n".repeat(4096),
    );
    write(repo.path(), "src/tiny.ts", "1\n");
    write(
        repo.path(),
        "knip.sarif",
        &sarif_log(
            "knip",
            true,
            &["src/huge.ts", "src/tiny.ts"],
            vec![
                finding("zz-unused-file", "src/huge.ts", "hhhh", "4 MB and unused"),
                finding("aa-unused-export", "src/tiny.ts", "tttt", "two bytes"),
            ],
        ),
    );

    let run = judged(repo.path(), &["ratchet", "--sarif", "knip.sarif"]);

    run.expect_code(1, "neither finding is baselined");
    assert!(
        run.offset_of("aa-unused-export") < run.offset_of("zz-unused-file"),
        "findings must be ordered by rule id, never by bytes reclaimed (§9.13 \
         invariant 3). Report was:\n{}",
        run.stdout
    );
}

#[test]
fn a_rotted_baseline_entry_exits_one_and_says_why() {
    let repo = repo_with_sources("rot");
    write(
        repo.path(),
        "knip.sarif",
        &sarif_log(
            "knip",
            true,
            &["src/a.ts", "src/b.ts", "src/c.ts"],
            vec![finding(
                "unused-export",
                "src/a.ts",
                "aaaa",
                "export `x` is never imported",
            )],
        ),
    );
    write(
        repo.path(),
        BASELINE_PATH,
        &format!(
            "{}\n{}\n{}\n",
            // Still matched by the run: not rot.
            baseline_line(
                "judged/v1:aaaa",
                "unused-export",
                "src/a.ts",
                "2026-01-01T00:00:00Z"
            ),
            // Nothing in this run carries this fingerprint. The amnesty is
            // protecting nothing.
            baseline_line(
                "judged/v1:cccc",
                "unused-export",
                "src/c.ts",
                "2026-01-01T00:00:00Z"
            ),
            // The file itself is gone, so the entry can never match again.
            baseline_line(
                "judged/v1:dddd",
                "unused-file",
                "src/deleted-last-year.ts",
                "2026-01-01T00:00:00Z"
            ),
        ),
    );

    let run = judged(repo.path(), &["ratchet", "--sarif", "knip.sarif"]);

    run.expect_code(
        1,
        "the baseline carries entries that no longer earn their place",
    )
    .expect_says("BASELINE ROT")
    .expect_says("judged/v1:cccc")
    .expect_says("src/deleted-last-year.ts")
    // The remediation is the opposite of fixing code, so the report has to
    // say which one it is asking for.
    .expect_says("--update");
}

#[test]
fn an_expired_baseline_entry_is_rot_and_the_expiry_is_quoted_back() {
    let repo = repo_with_sources("expired");
    write(
        repo.path(),
        "knip.sarif",
        &sarif_log(
            "knip",
            true,
            &["src/a.ts"],
            vec![finding("unused-export", "src/a.ts", "aaaa", "still here")],
        ),
    );
    write(
        repo.path(),
        BASELINE_PATH,
        &format!(
            "{}\n",
            json!({
                "fingerprint": "judged/v1:aaaa",
                "rule_id": "unused-export",
                "uri": "src/a.ts",
                "first_seen": "2020-01-01T00:00:00Z",
                "expires": "2021-01-01",
            })
        ),
    );

    let run = judged(repo.path(), &["ratchet", "--sarif", "knip.sarif"]);

    run.expect_code(1, "a deadline a human set has passed")
        .expect_says("BASELINE ROT")
        .expect_says("2021-01-01");
}

#[test]
fn a_crashed_analyzer_is_refused_with_exit_two() {
    // §6.20's failure mode, and the reason exit 2 exists at all: a crashed
    // analyzer emits zero results, the ratchet records "nothing new", and the
    // gate is permanently disarmed while staying green.
    let repo = repo_with_sources("crashed");
    write(
        repo.path(),
        "crash.sarif",
        &sarif_log("knip", false, &["src/a.ts", "src/b.ts", "src/c.ts"], vec![]),
    );

    let run = judged(repo.path(), &["ratchet", "--sarif", "crash.sarif"]);

    run.expect_code(2, "a run that did not succeed cannot be judged")
        .expect_says("REFUSED")
        .expect_says("executionSuccessful")
        .expect_says("crash.sarif")
        // A refusal must never be reported in the vocabulary of a pass.
        .expect_silent_about("clean:");
}

#[test]
fn a_sarif_log_with_no_runs_at_all_is_refused() {
    let repo = repo_with_sources("no-runs");
    write(
        repo.path(),
        "empty.sarif",
        r#"{"version":"2.1.0","runs":[]}"#,
    );

    let run = judged(repo.path(), &["ratchet", "--sarif", "empty.sarif"]);

    run.expect_code(
        2,
        "no run means no evidence, which is not the same as no findings",
    )
    .expect_says("REFUSED");
}

#[test]
fn a_missing_sarif_file_is_refused_rather_than_read_as_empty() {
    let repo = repo_with_sources("missing-sarif");

    let run = judged(repo.path(), &["ratchet", "--sarif", "nope.sarif"]);

    run.expect_code(2, "a file that is not there is not a clean run")
        .expect_says("nope.sarif");
}

#[test]
fn update_rewrites_the_baseline_and_keeps_the_original_first_seen() {
    let repo = repo_with_sources("update");
    write(
        repo.path(),
        "knip.sarif",
        &sarif_log(
            "knip",
            true,
            &["src/a.ts", "src/b.ts", "src/c.ts"],
            vec![
                finding("unused-export", "src/a.ts", "aaaa", "old news"),
                finding("unused-file", "src/b.ts", "bbbb", "brand new"),
            ],
        ),
    );
    write(
        repo.path(),
        BASELINE_PATH,
        &format!(
            "{}\n{}\n",
            baseline_line(
                "judged/v1:aaaa",
                "unused-export",
                "src/a.ts",
                "2019-03-04T05:06:07Z"
            ),
            // Rot: this one is not in the run, so --update must drop it.
            baseline_line(
                "judged/v1:cccc",
                "unused-export",
                "src/c.ts",
                "2019-03-04T05:06:07Z"
            ),
        ),
    );

    let run = judged(
        repo.path(),
        &["ratchet", "--sarif", "knip.sarif", "--update"],
    );
    run.expect_code(
        0,
        "rewriting the baseline is the remediation, not a failure",
    )
    .expect_says(BASELINE_PATH);

    let written = read(repo.path(), BASELINE_PATH);
    assert!(
        written.contains("judged/v1:aaaa") && written.contains("judged/v1:bbbb"),
        "both findings in the run must be accepted; got:\n{written}"
    );
    assert!(
        !written.contains("judged/v1:cccc"),
        "an entry nothing in the run matches is rot and must not survive an \
         --update; got:\n{written}"
    );
    assert!(
        written.contains("2019-03-04T05:06:07Z"),
        "an entry that was already accepted must keep its original firstSeen — \
         resetting the stability clock on every update is how a baseline stops \
         being reviewable (§9.4); got:\n{written}"
    );
    assert_eq!(
        written.lines().filter(|l| !l.trim().is_empty()).count(),
        2,
        "one JSONL line per accepted finding; got:\n{written}"
    );

    // And the ratchet it just wrote must be the one CI passes.
    judged(repo.path(), &["ratchet", "--sarif", "knip.sarif"])
        .expect_code(0, "the baseline was just rewritten from this exact run");
}

#[test]
fn update_carries_a_passed_deadline_forward_instead_of_laundering_it() {
    // §9.14 names the permanent amnesty list as the ratchet's known failure
    // mode, and `--update` is the mechanism that would build one: run it often
    // enough and every deadline a human set gets quietly renewed. So an
    // `expires` is carried through unchanged, counted out loud in the report,
    // and left failing the very next run.
    //
    // The second entry is the case that motivates counting them with
    // `judged_ratchet::has_expired` rather than with a local `expires <= now`.
    // `next quarter` is a date nothing can evaluate; it sorts after any real
    // timestamp, so the local spelling reads it as *not yet due* and renews it
    // forever, while `has_expired` — the same predicate the rot report uses —
    // treats an unevaluable deadline as passed.
    let repo = repo_with_sources("update-expired");
    write(
        repo.path(),
        "knip.sarif",
        &sarif_log(
            "knip",
            true,
            &["src/a.ts", "src/b.ts"],
            vec![
                finding("unused-export", "src/a.ts", "aaaa", "still here"),
                finding("unused-file", "src/b.ts", "bbbb", "also still here"),
            ],
        ),
    );
    write(
        repo.path(),
        BASELINE_PATH,
        &format!(
            "{}\n{}\n",
            json!({
                "fingerprint": "judged/v1:aaaa",
                "rule_id": "unused-export",
                "uri": "src/a.ts",
                "first_seen": "2020-01-01T00:00:00Z",
                "expires": "2021-01-01",
            }),
            json!({
                "fingerprint": "judged/v1:bbbb",
                "rule_id": "unused-file",
                "uri": "src/b.ts",
                "first_seen": "2020-01-01T00:00:00Z",
                "expires": "next quarter",
            }),
        ),
    );

    let run = judged(
        repo.path(),
        &["ratchet", "--sarif", "knip.sarif", "--update"],
    );

    run.expect_code(0, "rewriting the baseline is not itself a failure")
        .expect_says("2 kept an expiry that has already passed");

    let written = read(repo.path(), BASELINE_PATH);
    assert!(
        written.contains("2021-01-01") && written.contains("next quarter"),
        "both deadlines must survive the rewrite verbatim: a mechanical rewrite \
         has no standing to extend or drop a date a human typed. Got:\n{written}"
    );

    // The property all of the above exists to protect: passing `--update` over
    // an expired entry does not buy it another pass.
    judged(repo.path(), &["ratchet", "--sarif", "knip.sarif"])
        .expect_code(
            1,
            "an --update must never turn an expired amnesty into a green build",
        )
        .expect_says("BASELINE ROT")
        .expect_says("2021-01-01")
        .expect_says("next quarter");
}

#[test]
fn update_refuses_to_rewrite_the_baseline_from_a_crashed_run() {
    // The most destructive thing this binary could do. A crashed analyzer
    // reports zero findings; baselining that would erase the accepted backlog
    // and leave a green CI over a repository nobody has analyzed since.
    let repo = repo_with_sources("update-crashed");
    write(
        repo.path(),
        "crash.sarif",
        &sarif_log("knip", false, &["src/a.ts", "src/b.ts", "src/c.ts"], vec![]),
    );
    let original = format!(
        "{}\n",
        baseline_line(
            "judged/v1:aaaa",
            "unused-export",
            "src/a.ts",
            "2019-03-04T05:06:07Z"
        )
    );
    write(repo.path(), BASELINE_PATH, &original);

    let run = judged(
        repo.path(),
        &["ratchet", "--sarif", "crash.sarif", "--update"],
    );

    run.expect_code(2, "a refused run must not be allowed to rewrite anything")
        .expect_says("REFUSED");
    assert_eq!(
        read(repo.path(), BASELINE_PATH),
        original,
        "the baseline must be byte-identical after a refused --update"
    );
}

#[test]
fn several_sarif_logs_are_judged_as_one_run() {
    // Two adapters over the same repository. A fingerprint reported by knip and
    // absent from vulture's log is not rot, and treating each log separately
    // would report it as such — which is how a multi-adapter ratchet becomes
    // permanently red for reasons nobody can act on.
    let repo = repo_with_sources("multi");
    write(
        repo.path(),
        "knip.sarif",
        &sarif_log(
            "knip",
            true,
            &["src/a.ts"],
            vec![finding("unused-export", "src/a.ts", "aaaa", "ts side")],
        ),
    );
    write(
        repo.path(),
        "vulture.sarif",
        &sarif_log(
            "vulture",
            true,
            &["src/b.ts"],
            vec![finding("unused-function", "src/b.ts", "bbbb", "py side")],
        ),
    );
    write(
        repo.path(),
        BASELINE_PATH,
        &format!(
            "{}\n{}\n",
            baseline_line(
                "judged/v1:aaaa",
                "unused-export",
                "src/a.ts",
                "2026-01-01T00:00:00Z"
            ),
            baseline_line(
                "judged/v1:bbbb",
                "unused-function",
                "src/b.ts",
                "2026-01-01T00:00:00Z"
            ),
        ),
    );

    judged(
        repo.path(),
        &[
            "ratchet",
            "--sarif",
            "knip.sarif",
            "--sarif",
            "vulture.sarif",
        ],
    )
    .expect_code(
        0,
        "each log's findings baseline the other's; neither is rot",
    )
    .expect_says("clean");
}

#[test]
fn a_short_scanned_universe_is_reported_without_changing_the_exit_code() {
    // §9.2's positive control. Degradation caps the tier for affected paths; it
    // does not fail a build the analyzers did not fail, so the operator has to
    // be told in words rather than through an exit code.
    let repo = repo_with_sources("degraded");
    write(
        repo.path(),
        "knip.sarif",
        &sarif_log(
            "knip",
            true,
            &["src/a.ts"],
            vec![finding("unused-export", "src/a.ts", "aaaa", "one of many")],
        ),
    );
    write(
        repo.path(),
        BASELINE_PATH,
        &format!(
            "{}\n",
            baseline_line(
                "judged/v1:aaaa",
                "unused-export",
                "src/a.ts",
                "2026-01-01T00:00:00Z"
            )
        ),
    );

    let run = judged(
        repo.path(),
        &[
            "ratchet",
            "--sarif",
            "knip.sarif",
            "--expected-targets",
            "1000",
        ],
    );

    run.expect_code(0, "a degraded run is still a run; its verdict stands")
        .expect_says("DEGRADED")
        .expect_says("analysisTarget");
}

// ---------------------------------------------------------------------------
// judged mutants — the headline result
// ---------------------------------------------------------------------------

/// The §10 E2 catalogue size. Hard-coded rather than read from
/// `fixtures::all()`, so that a catalogue that silently shrinks fails here.
const E2_CLASSES: usize = 19;

/// The class rows of a text report — the lines shaped `  mNN  pass  ...`.
///
/// Rows, not substring occurrences. `stdout.matches("  m")` also matches the
/// mechanism prose in a row's tail — `decoys  module name ...` on m02 and
/// `decoys  model field ...` on m11 — so it over-counts: 21 hits against 19
/// required rows, which is exactly enough slack to hide two classes that were
/// never printed at all.
fn class_rows(stdout: &str) -> Vec<&str> {
    stdout
        .lines()
        .filter(|line| {
            let bytes = line.as_bytes();
            // The rows under a class are indented further (`       removed
            // live: ...`) and the summary lines start at column zero, so only a
            // class row can match.
            bytes.len() > 5
                && bytes.starts_with(b"  m")
                && bytes[3].is_ascii_digit()
                && bytes[4].is_ascii_digit()
        })
        .collect()
}

/// Assert the report carries one row for every class in the catalogue, each
/// named.
///
/// A silently dropped class is the failure this guards: it removes a row from
/// the table and a line from the totals, and what is left still reads like a
/// complete, passing report.
fn expect_every_class_is_reported(run: &Run) {
    let rows = class_rows(&run.stdout);
    assert_eq!(
        rows.len(),
        E2_CLASSES,
        "the report carries {} class rows, not {E2_CLASSES}. A mutant that never \
         reaches the report reads as a pass the SUT never earned. Report was:\n{}",
        rows.len(),
        run.stdout
    );
    for n in 1..=E2_CLASSES {
        let id = format!("m{n:02}");
        assert!(
            rows.iter().any(|row| row[2..].starts_with(&id)),
            "no row for class {id}; the report names {:?}. Report was:\n{}",
            rows.iter().map(|row| &row[2..5]).collect::<Vec<_>>(),
            run.stdout
        );
    }
}

#[test]
fn mutants_refusing_exits_zero_because_it_removes_nothing() {
    let repo = scratch("mutants-refusing");

    let run = judged(repo.path(), &["mutants", "--sut", "refusing"]);

    run.expect_code(
        0,
        "the gate is false removals, and a SUT that claims nothing has none",
    )
    .expect_says("refusing")
    .expect_says("false removals: 0")
    // Zero false removals is also the score of a tool that refuses to answer,
    // so the report has to print decoy recall beside the gate or the exit 0
    // reads as an endorsement.
    .expect_says("decoy recall");

    expect_every_class_is_reported(&run);
}

#[test]
fn mutants_naive_exits_non_zero_and_names_the_classes_it_failed() {
    // The headline result of the whole E2 body of work. §9.8: if breaking the
    // build does not break the gate, the gate is not a gate — so the naive
    // cleaner, which is §7.5's heuristic reproduced faithfully, has to come out
    // red, and the report has to say which injected liveness mechanisms caught
    // it.
    let repo = scratch("mutants-naive");

    let run = judged(repo.path(), &["mutants", "--sut", "naive"]);

    run.expect_code(1, "a cleaner that removes live files must not pass")
        .expect_says("naive")
        .expect_says("GATE FAILED")
        .expect_says("classes with false removals:");

    // A red report is no more allowed to be short than a green one: a class
    // dropped here would be a mechanism this cleaner was never shown to survive.
    expect_every_class_is_reported(&run);

    assert!(
        !run.stdout.contains("false removals: 0"),
        "the naive cleaner scored zero false removals across the whole \
         catalogue. The fixtures have gone soft and every green result this \
         suite has ever produced is unsupported. Report was:\n{}",
        run.stdout
    );

    // Naming the classes is the requirement, not just failing. A red gate that
    // does not say which mechanism defeated the tool cannot be acted on.
    let named: Vec<String> = (1..=E2_CLASSES)
        .map(|n| format!("m{n:02}"))
        .filter(|id| {
            run.stdout
                .lines()
                .any(|line| line.starts_with("classes with false removals:") && line.contains(id))
        })
        .collect();
    assert!(
        named.len() >= 5,
        "at least the five classes whose only reference is definitionally not \
         source (m03, m08, m09, m13, m18) must catch a source-only cleaner; the \
         report named {named:?}. Report was:\n{}",
        run.stdout
    );
}

#[test]
fn the_two_controls_disagree_about_the_gate_and_agree_about_nothing_else() {
    // Run side by side, because the pair is the evidence. Either control alone
    // proves nothing: refusing-passes says the harness does not invent
    // failures, naive-fails says the catalogue can still fail, and only both
    // together say the suite measures something.
    let repo = scratch("mutants-pair");

    let refusing = judged(repo.path(), &["mutants", "--sut", "refusing"]);
    let naive = judged(repo.path(), &["mutants", "--sut", "naive"]);

    refusing.expect_code(0, "no false removals");
    naive.expect_code(1, "false removals");
    assert_ne!(
        refusing.stdout, naive.stdout,
        "the two controls produced identical reports, so the report is not \
         reading the SUT at all"
    );
}

#[test]
fn mutants_defaults_to_the_control_that_fails() {
    // A bare `judged mutants` must never be mistakable for a passing real tool.
    let repo = scratch("mutants-default");

    judged(repo.path(), &["mutants"])
        .expect_code(1, "the default SUT is the positive control")
        .expect_says("naive");
}

#[test]
fn mutants_json_carries_the_gate_and_every_class() {
    let repo = scratch("mutants-json");

    let run = judged(repo.path(), &["mutants", "--sut", "refusing", "--json"]);
    run.expect_code(0, "--json changes the rendering, never the verdict");

    let report: Value = serde_json::from_str(run.stdout.trim())
        .unwrap_or_else(|e| panic!("--json must emit JSON: {e}\nGot:\n{}", run.stdout));

    assert_eq!(report["sut"], json!("refusing"));
    assert_eq!(report["false_removal_count"], json!(0));
    assert_eq!(report["gate_passed"], json!(true));

    let mutants = report["mutants"]
        .as_array()
        .unwrap_or_else(|| panic!("`mutants` must be an array; got {report}"));
    assert_eq!(mutants.len(), E2_CLASSES);
    let ids: Vec<&str> = mutants.iter().filter_map(|m| m["id"].as_str()).collect();
    let expected: Vec<String> = (1..=E2_CLASSES).map(|n| format!("m{n:02}")).collect();
    assert_eq!(
        ids, expected,
        "classes must be emitted in catalogue order, sorted by mutant id \
         (§9.13 invariant 3)"
    );
    assert!(
        mutants[0]["mechanism"]
            .as_str()
            .is_some_and(|m| !m.is_empty()),
        "each class must carry the one liveness mechanism it injects; got {}",
        mutants[0]
    );
}

// ---------------------------------------------------------------------------
// The invariant that has no subcommand
// ---------------------------------------------------------------------------

#[test]
fn the_binary_has_no_flag_that_deletes() {
    // §9.13 invariant 1, at the process boundary. Checked here as well as in
    // the parser's own tests because this is the surface a user or a script
    // actually reaches, and "unrecognized argument" would read as an oversight
    // somebody should helpfully fix.
    let repo = repo_with_sources("no-delete");

    for flag in ["--fix", "--delete", "--clean", "--quarantine", "--apply"] {
        let run = judged(repo.path(), &["ratchet", "--sarif", "knip.sarif", flag]);
        run.expect_code(2, "a deletion-shaped flag is a usage error, not a no-op")
            .expect_says(flag)
            .expect_says("§9.13");
    }
}

#[test]
fn there_are_two_subcommands_and_the_help_says_so() {
    let repo = scratch("help");

    let run = judged(repo.path(), &["--help"]);
    run.expect_code(0, "help was asked for, not provoked")
        .expect_says("judged ratchet")
        .expect_says("judged mutants");

    judged(repo.path(), &["clean"])
        .expect_code(2, "there is no `judged clean`")
        .expect_says("ratchet");
}
