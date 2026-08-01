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
// The adapters' own disclosure constants, asserted verbatim rather than
// paraphrased: a test that retyped the prose would keep passing after the
// adapter reworded what the report actually prints.
use judged_mutants::adapters::{deadcode, knip, shear};
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
    judged_with_path(cwd, args, None)
}

/// Drive the binary with `PATH` replaced.
///
/// Every test that touches an external analyzer has to control `PATH`, or it
/// asserts something about the machine it happens to be running on rather than
/// about `judged`. `None` inherits the ambient `PATH`; `Some(dir)` makes `dir`
/// the *entire* search path, which is how "this analyzer is not installed" is
/// made true on a developer laptop that has it installed.
fn judged_with_path(cwd: &Path, args: &[&str], path: Option<&Path>) -> Run {
    let mut command = Command::new(env!("CARGO_BIN_EXE_judged"));
    command.args(args).current_dir(cwd);
    if let Some(path) = path {
        command.env("PATH", path);
    }
    let output: Output = command
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
// judged mutants --veto — the accuser/veto pair §11 R1 actually asks about
// ---------------------------------------------------------------------------

/// The end-to-end trade, through the real binary, over the real catalogue.
///
/// The naive control is the SUT because it is the only one guaranteed to be on
/// every machine and the only one that removes enough for both halves of the
/// trade to be non-zero. What is asserted is the **shape of the honesty**: the
/// gated run publishes fewer removals than the bare one, publishes what that
/// cost, and names the pair rather than the analyzer.
#[test]
fn veto_reports_both_what_it_prevented_and_what_it_cost() {
    let repo = scratch("mutants-veto");

    let bare: Value = {
        let run = judged(repo.path(), &["mutants", "--sut", "naive", "--json"]);
        serde_json::from_str(run.stdout.trim()).expect("--json emits JSON")
    };
    let run = judged(
        repo.path(),
        &["mutants", "--sut", "naive", "--veto", "--json"],
    );
    let gated: Value =
        serde_json::from_str(run.stdout.trim()).expect("--json emits JSON under --veto too");

    assert_eq!(
        gated["sut"], "naive+veto",
        "the measured system is the pair; a report named for the analyzer alone \
         would be compared against a bare run as though the tool had improved"
    );
    assert!(
        bare["veto"].is_null(),
        "a bare run carries no veto key, so the two are told apart by presence"
    );

    let veto = &gated["veto"];
    let prevented = veto["false_removals_prevented"]
        .as_u64()
        .expect("false_removals_prevented is a number");
    let remaining = veto["false_removals_remaining"]
        .as_u64()
        .expect("false_removals_remaining is a number");
    let lost = veto["decoys_lost"]
        .as_u64()
        .expect("decoys_lost is a number");

    // Both columns are present and neither is derivable from the other. A report
    // carrying only the first would be selling the veto (§9.13).
    assert!(
        prevented > 0,
        "Gate 2 rescued nothing from the naive control across 19 classes, which \
         means it did not run; got {veto}"
    );
    assert!(
        lost > 0,
        "Gate 2 rescued live artifacts and cost nothing at all, which no \
         whole-repo literal search does; got {veto}"
    );
    assert_eq!(
        prevented + remaining,
        bare["false_removal_count"]
            .as_u64()
            .expect("the bare run reports a false-removal count"),
        "prevented + remaining must reconstruct the bare count exactly, or one \
         of the two is not what it says it is"
    );
    assert!(
        gated["false_removal_count"].as_u64() <= bare["false_removal_count"].as_u64(),
        "a veto may only rescue: {gated} against {bare}"
    );
    assert!(
        veto["decoys_found_gated"].as_u64() <= veto["decoys_found_bare"].as_u64(),
        "a veto cannot find a decoy the accuser missed; got {veto}"
    );

    // The conflict list §9.13 asks for: what fired, and where. Present for at
    // least one class, and every record that names a needle names a file too.
    let mut with_evidence = 0usize;
    for class in gated["mutants"].as_array().expect("mutants is an array") {
        let Some(blocked) = class["veto"]["blocked_claims"].as_array() else {
            continue;
        };
        for record in blocked {
            assert!(
                record["claim"].as_str().is_some_and(|c| !c.is_empty()),
                "a blocked claim names the claim it blocked; got {record}"
            );
            assert!(
                record["gate"].as_str().is_some(),
                "a blocked claim names the gate that fired; got {record}"
            );
            if record["needle"].as_str().is_some() {
                assert!(
                    record["found_in"].as_str().is_some(),
                    "a needle that fired fired IN a file, and a veto nobody can \
                     check is a veto nobody will trust; got {record}"
                );
                with_evidence += 1;
            }
        }
    }
    assert!(
        with_evidence > 0,
        "no blocked claim carried a needle and a file. §7.3's best-validated \
         prior art shows the usage list, not a probability."
    );

    // And the human rendering says the same two things, at column zero.
    let text = judged(repo.path(), &["mutants", "--sut", "naive", "--veto"]);
    for expected in ["veto prevented: ", "veto cost: "] {
        assert!(
            text.stdout.lines().any(|line| line.starts_with(expected)),
            "no line starts with `{expected}`; got {}",
            text.stdout
        );
    }
}

/// `--veto` composes with the negative control, and changes nothing about it.
///
/// A SUT that claims nothing has nothing for a filter to remove, so the gated
/// run must be the bare run in every number that matters. If it is not, the
/// veto is doing something other than filtering.
#[test]
fn veto_over_a_sut_that_claims_nothing_changes_nothing() {
    let repo = scratch("mutants-veto-refusing");

    let run = judged(
        repo.path(),
        &["mutants", "--sut", "refusing", "--veto", "--json"],
    );
    run.expect_code(0, "no claims, so no false removals, gated or not");

    let report: Value = serde_json::from_str(run.stdout.trim()).expect("--json emits JSON");
    assert_eq!(report["veto"]["false_removals_prevented"], json!(0));
    assert_eq!(report["veto"]["decoys_lost"], json!(0));
    assert_eq!(report["veto"]["decoys_found_gated"], json!(0));
    assert_eq!(
        report["false_removal_count"],
        json!(0),
        "got {}",
        run.stdout
    );
}

/// The flag has to be discoverable, and the usage text has to say what it costs
/// before somebody turns it on in CI.
#[test]
fn help_documents_the_veto_and_that_it_runs_the_suite_twice() {
    let repo = scratch("mutants-veto-help");

    let run = judged(repo.path(), &["--help"]);
    run.expect_says("--veto");
    run.expect_says("TWICE");
    run.expect_says("prevented");
}

/// §11 R8's axis is reachable from the command line, and the report echoes the
/// spelling it was given.
///
/// R8 asks for a measurement of the needle strategy rather than an argument
/// about it, and a swept number is only evidence if a reader can re-run the
/// sweep. `VetoedSut::with_needles` existed and no command line reached it, so
/// the first published sweep could not be re-derived from anything in this
/// repository — which for an evaluation is the same as not having run it.
///
/// The round-trip is the property under test, not merely that the flag parses:
/// every accepted spelling is exactly what `veto.needles` reports back, so a
/// table row and the command that produced it name the same configuration.
#[test]
fn the_needle_strategy_is_selectable_so_the_r8_sweep_can_be_re_derived() {
    let repo = scratch("mutants-veto-needles");

    for strategy in [
        "basename",
        "basename+stem",
        "basename+stem+parent-dir",
        "basename+stem+parent-dir+symbol",
    ] {
        let run = judged(
            repo.path(),
            &[
                "mutants",
                "--sut",
                "naive",
                "--veto",
                "--needles",
                strategy,
                "--json",
            ],
        );
        let report: Value = serde_json::from_str(run.stdout.trim())
            .unwrap_or_else(|error| panic!("--needles {strategy} should report JSON: {error}"));
        assert_eq!(
            report["veto"]["needles"],
            json!(strategy),
            "the report must name the strategy it was run under, verbatim"
        );
    }

    // The default is the shipped one, and asking for it explicitly is the same
    // run — otherwise the sweep's baseline row is not the shipped build.
    let implicit = judged(
        repo.path(),
        &["mutants", "--sut", "naive", "--veto", "--json"],
    );
    let implicit: Value = serde_json::from_str(implicit.stdout.trim()).expect("JSON");
    assert_eq!(implicit["veto"]["needles"], json!("basename+stem"));
}

/// A needle strategy with no gate to configure is refused, not ignored.
///
/// Silently accepting `--needles` without `--veto` would let somebody publish a
/// sweep in which nothing was ever swept: every row would be a bare run, every
/// row would be identical, and the command line would look like it had asked
/// for something. §6.20's shape again — a result that means nothing, wearing
/// the appearance of a result.
#[test]
fn a_needle_strategy_without_a_veto_is_refused_rather_than_quietly_ignored() {
    let repo = scratch("mutants-needles-no-veto");

    let run = judged(
        repo.path(),
        &["mutants", "--sut", "refusing", "--needles", "basename"],
    );
    run.expect_code(2, "a flag that configures a gate that is not running");
    run.expect_says("--veto");
}

/// An unknown strategy names the ones that exist rather than shrugging.
#[test]
fn an_unknown_needle_strategy_lists_the_ones_that_exist() {
    let repo = scratch("mutants-needles-unknown");

    let run = judged(
        repo.path(),
        &[
            "mutants",
            "--sut",
            "refusing",
            "--veto",
            "--needles",
            "everything",
        ],
    );
    run.expect_code(2, "an unknown needle strategy is a usage error");
    for known in [
        "basename",
        "basename+stem",
        "basename+stem+parent-dir",
        "basename+stem+parent-dir+symbol",
    ] {
        run.expect_says(known);
    }
}

// ---------------------------------------------------------------------------
// judged mutants against an external analyzer
//
// The suite has only ever graded two SUTs we wrote ourselves, which bounds the
// harness and nothing else. Pointing it at a real analyzer is what turns E2
// into evidence about §11 R1 — and it introduces the one failure mode the two
// in-process controls could never have: the analyzer is not on the machine.
//
// A tool that is not installed claims nothing dead. Claiming nothing dead is a
// false-removal count of zero, and zero false removals is GATE PASSED. So the
// single most likely way this feature goes wrong is that it reports a clean
// suite for a run that never happened — §6.20's disarming failure wearing the
// clothes of a green build. Every test in this section exists to make that
// impossible.
// ---------------------------------------------------------------------------

/// A directory containing nothing, used as the whole of `PATH`.
fn empty_path(label: &str) -> TempDir {
    scratch(label)
}

/// Put an executable shim called `name` on a fresh `PATH` directory.
///
/// A shim rather than the real analyzer, because a test that requires vulture
/// to be installed is a test that is skipped on most machines and therefore a
/// test that does not exist. What is under test here is `judged`'s wiring —
/// that it finds the program, runs it, and renders a report — not vulture's
/// analysis, which E2 will measure separately once the numbers are collected.
#[cfg(unix)]
fn shim(dir: &Path, name: &str, body: &str) {
    use std::os::unix::fs::PermissionsExt;

    let path = dir.join(name);
    std::fs::write(&path, body).expect("shim must be writable");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
        .expect("shim must be made executable");
}

/// Assert that a report is not a verdict — no gate line, in either direction.
///
/// Separate from the exit code because both halves are load-bearing. A run that
/// exits 2 while still printing "false removals: 0 — GATE PASSED" has published
/// a number somebody will quote out of the log.
fn expect_no_gate_result(run: &Run) {
    for forbidden in [
        "GATE PASSED",
        "GATE FAILED",
        "false removals:",
        "decoy recall:",
    ] {
        run.expect_silent_about(forbidden);
    }
}

#[test]
fn a_missing_analyzer_refuses_loudly_and_never_reports_a_clean_suite() {
    // THE test for this feature. An analyzer that is not installed produces no
    // findings, which is arithmetically identical to an analyzer that found
    // nothing wrong — and the gate reads only that number. If this run were
    // allowed to reach the grader it would print "false removals: 0 — GATE
    // PASSED" and exit 0, which is a green build certifying nothing at all.
    let repo = scratch("vulture-missing");
    let nothing = empty_path("vulture-missing-path");

    let run = judged_with_path(
        repo.path(),
        &["mutants", "--sut", "vulture"],
        Some(nothing.path()),
    );

    run.expect_code(2, "an analyzer that is not installed analyzed nothing")
        .expect_says("vulture")
        .expect_says("not installed")
        // Naming the binary is not enough to act on. §9.13's presentation rules
        // are about what a human can do next, and the next thing here is to
        // install it.
        .expect_says("pip install vulture");
    expect_no_gate_result(&run);
}

#[test]
fn a_missing_analyzer_refuses_in_json_too() {
    // The rendering that a script reads, and therefore the one that would
    // silently propagate a fabricated pass into a dashboard. `--json` changes
    // the rendering, never the verdict.
    let repo = scratch("vulture-missing-json");
    let nothing = empty_path("vulture-missing-json-path");

    let run = judged_with_path(
        repo.path(),
        &["mutants", "--sut", "vulture", "--json"],
        Some(nothing.path()),
    );

    run.expect_code(2, "--json must not launder a refusal into a result");
    assert!(
        !run.stdout.contains("\"gate_passed\": true"),
        "a refusal must not emit a passing gate. Report was:\n{}",
        run.stdout
    );
    assert!(
        !run.stdout.contains("\"false_removal_count\": 0"),
        "a refusal must not emit a false-removal count at all; zero here means \
         `the analyzer is absent`, and nothing downstream can tell that from \
         `the analyzer found nothing`. Report was:\n{}",
        run.stdout
    );
    run.expect_says("vulture").expect_says("not installed");
}

#[test]
fn a_missing_command_sut_names_the_binary_it_could_not_find() {
    let repo = scratch("command-missing");
    let nothing = empty_path("command-missing-path");

    let run = judged_with_path(
        repo.path(),
        &["mutants", "--sut", "command", "--", "no-such-analyzer", "."],
        Some(nothing.path()),
    );

    run.expect_code(2, "the escape hatch gets the same guard as the named tool")
        .expect_says("no-such-analyzer")
        .expect_says("not installed");
    expect_no_gate_result(&run);
}

#[test]
fn an_analyzer_given_as_a_path_is_reported_as_a_path_that_is_not_there() {
    // A bare name is looked up on PATH; something with a separator in it is the
    // path it looks like, and is not looked up at all. The message has to say
    // which of those happened. Telling somebody who typed `./tools/analyze`
    // that it was not found "in the 45 directories on PATH" describes a search
    // that never ran and sends them to fix the wrong thing — and "install it"
    // is not the remedy for a path they meant literally.
    let repo = scratch("command-path");

    let run = judged(
        repo.path(),
        &["mutants", "--sut", "command", "--", "./tools/analyze"],
    );

    run.expect_code(2, "a path that is not there analyzed nothing")
        .expect_says("./tools/analyze")
        // The false claim, pinned by the words that make it: a report of a
        // search that did not happen. The remedy below it may still mention
        // PATH, because saying "a bare name would be looked up on PATH" is true
        // and is the fix.
        .expect_silent_about("Looked for")
        .expect_silent_about("directories on PATH");
    expect_no_gate_result(&run);
}

#[cfg(unix)]
#[test]
fn an_analyzer_that_is_present_is_actually_run_and_graded() {
    // The other side of the guard, and the reason it cannot be implemented as
    // "always refuse". A shim that is on PATH must produce a real report with a
    // real gate line — otherwise the missing-analyzer test above would pass
    // against a feature that never works.
    //
    // The shim claims nothing, so the interesting assertion is not the gate
    // (which it clears trivially) but the decoy line beside it, which is what
    // stops a silent tool from reading as a good one.
    let repo = scratch("command-present");
    let bin = scratch("command-present-path");
    shim(bin.path(), "quiet-analyzer", "#!/bin/sh\nexit 0\n");
    let path = format!(
        "{}:{}",
        bin.path().display(),
        std::env::var("PATH").unwrap_or_default()
    );

    let run = judged_with_path(
        repo.path(),
        &["mutants", "--sut", "command", "--", "quiet-analyzer", "."],
        Some(Path::new(&path)),
    );

    run.expect_code(0, "an analyzer that claims nothing has no false removals")
        .expect_says("quiet-analyzer")
        .expect_says("false removals: 0")
        .expect_says("decoy recall: 0 of")
        .expect_says("removed nothing at all");
    expect_every_class_is_reported(&run);
}

#[cfg(unix)]
#[test]
fn vulture_is_run_by_name_when_it_is_on_path() {
    // `--sut vulture` has to resolve to the `vulture` binary and no other, so
    // the shim is named `vulture` and the assertion is that the report says so.
    let repo = scratch("vulture-present");
    let bin = scratch("vulture-present-path");
    shim(bin.path(), "vulture", "#!/bin/sh\nexit 0\n");
    let path = format!(
        "{}:{}",
        bin.path().display(),
        std::env::var("PATH").unwrap_or_default()
    );

    let run = judged_with_path(
        repo.path(),
        &["mutants", "--sut", "vulture"],
        Some(Path::new(&path)),
    );

    run.expect_code(0, "a silent analyzer removes nothing")
        .expect_says("vulture")
        .expect_says("decoy recall:");
    expect_every_class_is_reported(&run);
}

#[cfg(unix)]
#[test]
fn a_report_produced_through_an_adapter_carries_the_adapters_envelope() {
    // §9.2's second non-SARIF clause: every adapter declares the finding
    // classes the tool structurally cannot emit. That declaration is what makes
    // a low false-removal count readable — without it, a narrow tool and a safe
    // tool produce the same number, and the report cannot tell them apart. So
    // the envelope, and the decision about which half of a verdict the tool's
    // findings were mapped to, are printed rather than left in a source
    // comment.
    let repo = scratch("vulture-envelope");
    let bin = scratch("vulture-envelope-path");
    shim(bin.path(), "vulture", "#!/bin/sh\nexit 0\n");
    let path = format!(
        "{}:{}",
        bin.path().display(),
        std::env::var("PATH").unwrap_or_default()
    );

    let run = judged_with_path(
        repo.path(),
        &["mutants", "--sut", "vulture"],
        Some(Path::new(&path)),
    );

    run.expect_says("silence is not evidence")
        .expect_says("claimed_dead_paths");

    // Above the table, not below the summary. §9.13 budgets the reader ten
    // seconds and puts the deciding numbers in the log tail; a page of adapter
    // prose appended after them would push the gate line off the bottom of a CI
    // log, which is the one line that must always be visible.
    assert!(
        run.offset_of("silence is not evidence") < run.offset_of("false removals:"),
        "the envelope must be printed above the summary lines, not after them. \
         Report was:\n{}",
        run.stdout
    );

    // The escape hatch cannot borrow vulture's envelope: nothing is known about
    // an analyzer that was named on the command line, and claiming otherwise
    // would be the adapter asserting more than the tool told it.
    let run = judged_with_path(
        repo.path(),
        &["mutants", "--sut", "command", "--", "vulture"],
        Some(Path::new(&path)),
    );
    run.expect_says("capability envelope: NOT DECLARED")
        .expect_silent_about("global AST name-set difference");
}

#[cfg(unix)]
#[test]
fn the_json_report_carries_the_envelope_for_adapters_and_omits_it_for_controls() {
    let repo = scratch("adapter-json");
    let bin = scratch("adapter-json-path");
    shim(bin.path(), "vulture", "#!/bin/sh\nexit 0\n");
    let path = format!(
        "{}:{}",
        bin.path().display(),
        std::env::var("PATH").unwrap_or_default()
    );

    let run = judged_with_path(
        repo.path(),
        &["mutants", "--sut", "vulture", "--json"],
        Some(Path::new(&path)),
    );
    let report: Value = serde_json::from_str(run.stdout.trim())
        .unwrap_or_else(|e| panic!("--json must emit JSON: {e}\nGot:\n{}", run.stdout));

    assert_eq!(report["sut"], json!("vulture"));
    for key in ["capability_envelope", "mapping_decision"] {
        assert!(
            report["adapter"][key]
                .as_str()
                .is_some_and(|s| !s.is_empty()),
            "a machine consumer that records `false_removal_count` without \
             `adapter.{key}` has recorded a number stripped of what bounds it. \
             Got:\n{}",
            run.stdout
        );
    }

    // The two controls are this repository's own code, so there is no
    // third-party translation to disclose and the key is absent rather than
    // empty.
    let run = judged(repo.path(), &["mutants", "--sut", "refusing", "--json"]);
    let report: Value = serde_json::from_str(run.stdout.trim())
        .unwrap_or_else(|e| panic!("--json must emit JSON: {e}\nGot:\n{}", run.stdout));
    assert_eq!(report["adapter"], Value::Null);
}

#[test]
fn the_escape_hatch_needs_a_command_and_says_where_to_put_it() {
    let repo = scratch("command-empty");

    let run = judged(repo.path(), &["mutants", "--sut", "command"]);
    run.expect_code(2, "an empty analyzer command line is a usage error")
        .expect_says("--");
    expect_no_gate_result(&run);

    // `--` present but with nothing after it is the same mistake one keystroke
    // later, and must not degrade into "run the empty program".
    let run = judged(repo.path(), &["mutants", "--sut", "command", "--"]);
    run.expect_code(2, "`--` with no command after it is still no command")
        .expect_says("--");
    expect_no_gate_result(&run);
}

#[test]
fn an_analyzer_argv_may_not_smuggle_in_a_deletion_flag() {
    // §9.13 invariant 1 is a property of the whole process, not of judged's own
    // argv. §9.2's adapters are read-only clause says the analyzer is run to be
    // *read*, never to act: an analyzer invoked with its own --fix would edit
    // the fixture repository, and the one thing E2 depends on is that the only
    // thing that changed about that repository is the mutant we injected.
    let repo = scratch("command-fix");

    for flag in ["--fix", "--delete", "--apply"] {
        let run = judged(
            repo.path(),
            &["mutants", "--sut", "command", "--", "some-linter", flag],
        );
        run.expect_code(2, "judged does not run an analyzer that was told to write")
            .expect_says(flag)
            .expect_says("§9.13");
        expect_no_gate_result(&run);
    }
}

#[test]
fn an_unknown_sut_lists_every_one_that_exists() {
    let repo = scratch("sut-unknown");

    // `periphery` rather than `knip`: knip used to stand in for "a tool judged
    // has heard of but cannot run", and it is now one of the options. The
    // stand-in has to be a tool that is genuinely not wired, or this test
    // asserts nothing.
    let run = judged(repo.path(), &["mutants", "--sut", "periphery"]);
    run.expect_code(2, "an unknown SUT is a usage error");
    // The message is the discovery surface for this flag: an option missing
    // from it is an option nobody finds.
    for known in [
        "naive", "refusing", "vulture", "knip", "deadcode", "shear", "command",
    ] {
        run.expect_says(known);
    }
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

// ---------------------------------------------------------------------------
// The three ecosystem-specific analyzers: knip (JS/TS), deadcode (Go),
// cargo-shear (Rust).
//
// §11 R1 — whether an auto-act tier may exist at all — is answered by E2, and
// E2 answered against Vulture alone answers it for Python. These three are what
// open the other seven classes. Every test below is about the *wiring*: that
// the option exists, that an absent binary is refused rather than graded, and
// that the language map counts the classes the tool never opened out of the
// score.
// ---------------------------------------------------------------------------

/// The `PATH` a shim directory makes, with the machine's own `PATH` behind it.
///
/// The ambient tail matters for the shims: `/bin/sh` has to stay findable, and
/// so does whatever a wrapper argv execs.
fn path_with(dir: &Path) -> String {
    format!(
        "{}:{}",
        dir.display(),
        std::env::var("PATH").unwrap_or_default()
    )
}

/// A shim standing in for knip, printing knip's real empty-result log.
///
/// Captured verbatim from `npx --yes knip@6 --reporter sarif --no-progress
/// --directory <dir>` (knip 6.31.0) against a project with nothing unused. That
/// run exits 0, and the log is a complete SARIF document with an empty
/// `results` array — not an empty stream, which the adapter would reject.
const KNIP_EMPTY_SARIF_SHIM: &str = concat!(
    "#!/bin/sh\ncat <<'SARIF'\n",
    r#"{"$schema":"https://docs.oasis-open.org/sarif/sarif/v2.1.0/errata01/os/schemas/sarif-schema-2.1.0.json","version":"2.1.0","runs":[{"tool":{"driver":{"name":"knip","version":"6.31.0","semanticVersion":"6.31.0","informationUri":"https://knip.dev","rules":[]}},"results":[]}]}"#,
    "\nSARIF\nexit 0\n"
);

/// A shim standing in for deadcode, printing deadcode's real empty result.
///
/// Captured verbatim from `deadcode -json <dir>/...` against a Go module with
/// nothing dead: the literal four bytes `null`, with no trailing newline,
/// because Go marshals a nil slice as `null` rather than `[]`. Exit 0.
const DEADCODE_EMPTY_SHIM: &str = "#!/bin/sh\nprintf 'null'\nexit 0\n";

/// A shim standing in for cargo-shear, printing its real empty result.
///
/// Captured verbatim from `cargo-shear --format json <dir>` against a crate
/// with nothing unused. Exit 0, and note that the summary counters are present
/// and zero — the adapter rejects a document whose counters disagree with the
/// findings array.
const SHEAR_EMPTY_SHIM: &str = concat!(
    "#!/bin/sh\ncat <<'JSON'\n",
    "{\n  \"summary\": {\n    \"errors\": 0,\n    \"warnings\": 0,\n    \"fixed\": 0\n  },\n",
    "  \"findings\": []\n}\n",
    "JSON\nexit 0\n"
);

#[test]
fn every_named_analyzer_refuses_loudly_when_it_is_not_installed() {
    // The same guard vulture already has, extended to the three new options,
    // and the reason it is written before the wiring: `CommandSut::run` turns a
    // spawn failure into an empty verdict, an empty verdict is zero false
    // removals, and zero false removals is "GATE PASSED" and exit 0. A green
    // build certifying an analyzer that is not on the machine is §6.20's
    // disarming failure, so each new SUT has to be refused at preflight before
    // anything else runs.
    //
    // The install hint is asserted too. §9.13's presentation rules are about
    // what a human does next, and "deadcode is not installed" without the
    // command that installs it makes the reader go and find out.
    for (sut, binary, hint) in [
        ("knip", "npx", "Node.js"),
        (
            "deadcode",
            "deadcode",
            "go install golang.org/x/tools/cmd/deadcode",
        ),
        ("shear", "cargo-shear", "cargo install cargo-shear"),
    ] {
        let repo = scratch(&format!("{sut}-missing"));
        let nothing = empty_path(&format!("{sut}-missing-path"));

        let run = judged_with_path(
            repo.path(),
            &["mutants", "--sut", sut],
            Some(nothing.path()),
        );

        run.expect_code(2, "an analyzer that is not installed analyzed nothing")
            .expect_says(binary)
            .expect_says("not installed")
            .expect_says(hint);
        expect_no_gate_result(&run);
    }
}

#[test]
fn a_missing_analyzer_refuses_in_json_for_every_named_sut() {
    // The rendering a dashboard reads. `--json` changes the rendering, never
    // the verdict, and a refusal must not emit the keys a consumer would read
    // as a score — a zero here and a zero from a real clean run are the same
    // bytes.
    for sut in ["knip", "deadcode", "shear"] {
        let repo = scratch(&format!("{sut}-missing-json"));
        let nothing = empty_path(&format!("{sut}-missing-json-path"));

        let run = judged_with_path(
            repo.path(),
            &["mutants", "--sut", sut, "--json"],
            Some(nothing.path()),
        );

        run.expect_code(2, "--json must not launder a refusal into a result");
        assert!(
            !run.stdout.contains("\"gate_passed\""),
            "a refusal must not emit a gate verdict for --sut {sut}. Report was:\n{}",
            run.stdout
        );
        assert!(
            !run.stdout.contains("\"false_removal_count\""),
            "a refusal must not emit a false-removal count for --sut {sut}. Report was:\n{}",
            run.stdout
        );
    }
}

#[cfg(unix)]
#[test]
fn knip_is_run_and_its_unread_classes_are_counted_out_of_the_score() {
    // The other half of the guard: a shim on PATH must produce a real report,
    // or the refusal test above would pass against a feature that never works.
    //
    // The shim emits knip's real empty-result SARIF, captured from
    // `npx knip@6 --reporter sarif --no-progress` against a project with
    // nothing unused (knip 6.31.0, exit 0). It claims nothing, so what is
    // under test is the language map.
    //
    // Three of nineteen classes are ones knip can load: m02 and m10, whose
    // polyglot trees carry a package.json and a JS/TS half, and m14, which is
    // TypeScript outright. The other sixteen it cannot open at all.
    //
    // That sixteen was thirteen before this build, and the three-class
    // difference was a bug, not a rounding. The old map claimed knip reads
    // `Polyglot`, on the reasoning that a polyglot fixture always contains a JS
    // or TS half. Measured 2026-08-01 against knip 6.31.0 on the materialized
    // catalogue: m08 (Python + CI workflow), m13 (PHP) and m18 (Python +
    // Kotlin) contain no JS at all and no package.json, and knip exits 2 on
    // each with `Unable to find package.json`. Counting them as read is what
    // made `--sut knip` abort on m01 rather than produce a score.
    let repo = scratch("knip-present");
    let bin = scratch("knip-present-path");
    shim(bin.path(), "npx", KNIP_EMPTY_SARIF_SHIM);

    let run = judged_with_path(
        repo.path(),
        &["mutants", "--sut", "knip"],
        Some(Path::new(&path_with(bin.path()))),
    );

    run.expect_code(0, "an analyzer that claims nothing has no false removals")
        .expect_says("knip")
        .expect_says("[NOT READ by this SUT]")
        .expect_says("not measured: 16 of 19 classes")
        .expect_says("decoy recall:");
    expect_every_class_is_reported(&run);
}

#[cfg(unix)]
#[test]
fn deadcode_reads_go_and_nothing_else() {
    // One Go class in the catalogue (m12), so eighteen are unread. The shim
    // emits deadcode's real empty result — the literal four bytes `null`, which
    // is what Go's encoder writes for a nil slice — captured from
    // `deadcode -json` against a module with nothing dead (x/tools, exit 0).
    let repo = scratch("deadcode-present");
    let bin = scratch("deadcode-present-path");
    shim(bin.path(), "deadcode", DEADCODE_EMPTY_SHIM);

    let run = judged_with_path(
        repo.path(),
        &["mutants", "--sut", "deadcode"],
        Some(Path::new(&path_with(bin.path()))),
    );

    run.expect_code(0, "an analyzer that claims nothing has no false removals")
        .expect_says("deadcode")
        .expect_says("not measured: 18 of 19 classes");
    expect_every_class_is_reported(&run);
}

#[cfg(unix)]
#[test]
fn shear_reads_rust_and_nothing_else() {
    // Six Rust classes, so thirteen are unread. The shim emits cargo-shear's
    // real empty result, captured from `cargo-shear --format json` against a
    // crate with nothing unused (exit 0).
    let repo = scratch("shear-present");
    let bin = scratch("shear-present-path");
    shim(bin.path(), "cargo-shear", SHEAR_EMPTY_SHIM);

    let run = judged_with_path(
        repo.path(),
        &["mutants", "--sut", "shear"],
        Some(Path::new(&path_with(bin.path()))),
    );

    run.expect_code(0, "an analyzer that claims nothing has no false removals")
        .expect_says("cargo-shear")
        .expect_says("not measured: 13 of 19 classes");
    expect_every_class_is_reported(&run);
}

#[cfg(unix)]
#[test]
fn every_named_analyzer_discloses_its_capability_envelope_before_the_rows() {
    // §9.2's second non-SARIF clause: an adapter declares what the tool
    // structurally cannot emit, so the orchestrator knows when its silence
    // means anything. A score published without it is a score somebody reads as
    // the tool's blast radius when it is the adapter's floor on it.
    // Asserted against the adapters' own constants rather than against a phrase
    // retyped here, so that an adapter which reworded its envelope cannot leave
    // this test passing on prose the report no longer prints.
    for (sut, binary, body, envelope, mapping) in [
        (
            "knip",
            "npx",
            KNIP_EMPTY_SARIF_SHIM,
            knip::CAPABILITY_ENVELOPE,
            knip::MAPPING_DECISION,
        ),
        (
            "deadcode",
            "deadcode",
            DEADCODE_EMPTY_SHIM,
            deadcode::CAPABILITY_ENVELOPE,
            deadcode::MAPPING_DECISION,
        ),
        (
            "shear",
            "cargo-shear",
            SHEAR_EMPTY_SHIM,
            shear::CAPABILITY_ENVELOPE,
            shear::MAPPING_DECISION,
        ),
    ] {
        let repo = scratch(&format!("{sut}-envelope"));
        let bin = scratch(&format!("{sut}-envelope-path"));
        shim(bin.path(), binary, body);

        let run = judged_with_path(
            repo.path(),
            &["mutants", "--sut", sut],
            Some(Path::new(&path_with(bin.path()))),
        );

        run.expect_code(0, "the shim claims nothing")
            .expect_says(envelope)
            // The mapping decision too: it is what says which half of a verdict
            // the adapter fills, and a count read without it is read as the
            // tool's blast radius when it is the adapter's floor on it.
            .expect_says(mapping);
        assert!(
            run.offset_of(envelope) < run.offset_of("  m01"),
            "the envelope must be printed before the class rows for --sut {sut}, \
             or a CI log tail shows the numbers with nothing bounding them. \
             Report was:\n{}",
            run.stdout
        );
    }
}

/// Shims reproducing each analyzer's **measured** behaviour on both sides of
/// its own ecosystem boundary: the real refusal when its manifest is absent,
/// the real empty report when it is present.
///
/// The refusal half is captured, not invented. Measured 2026-08-01 against the
/// materialized catalogue:
///
/// | Tool | Outside its ecosystem | Exit |
/// | --- | --- | --- |
/// | knip 6.31.0 | `ERROR: Unable to find package.json` | 2 |
/// | cargo-shear | ``error: could not find `Cargo.toml` `` | 2 |
/// | deadcode (x/tools) | `deadcode: packages contain errors` | 1 |
///
/// Each of those codes is shared with a genuine analysis failure whose stdout
/// is equally empty, so none of them may be declared healthy (§6.20) — which is
/// precisely why a class outside the analyzer's languages has to be skipped
/// before the tool is spawned rather than tolerated afterwards.
///
/// The last argument is the repository, because
/// [`judged_mutants::sut::CommandSut`] appends it; deadcode's arrives as the
/// package pattern `<repo>/...`, so the `/...` is stripped back off.
#[cfg(unix)]
const KNIP_ECOSYSTEM_AWARE_SHIM: &str = concat!(
    "#!/bin/sh\nfor a in \"$@\"; do dir=\"$a\"; done\n",
    "if [ ! -f \"$dir/package.json\" ]; then\n",
    "  echo 'ERROR: Unable to find package.json' >&2\n  exit 2\nfi\n",
    "cat <<'SARIF'\n",
    r#"{"$schema":"https://docs.oasis-open.org/sarif/sarif/v2.1.0/errata01/os/schemas/sarif-schema-2.1.0.json","version":"2.1.0","runs":[{"tool":{"driver":{"name":"knip","version":"6.31.0","semanticVersion":"6.31.0","informationUri":"https://knip.dev","rules":[]}},"results":[]}]}"#,
    "\nSARIF\nexit 0\n"
);

#[cfg(unix)]
const DEADCODE_ECOSYSTEM_AWARE_SHIM: &str = concat!(
    "#!/bin/sh\nfor a in \"$@\"; do dir=\"$a\"; done\ndir=${dir%/...}\n",
    "if [ ! -f \"$dir/go.mod\" ]; then\n",
    "  echo 'deadcode: packages contain errors' >&2\n  exit 1\nfi\n",
    "printf 'null'\nexit 0\n"
);

#[cfg(unix)]
const SHEAR_ECOSYSTEM_AWARE_SHIM: &str = concat!(
    "#!/bin/sh\nfor a in \"$@\"; do dir=\"$a\"; done\n",
    "if [ ! -f \"$dir/Cargo.toml\" ]; then\n",
    "  echo 'error: could not find `Cargo.toml`' >&2\n  exit 2\nfi\n",
    "cat <<'JSON'\n",
    "{\n  \"summary\": {\n    \"errors\": 0,\n    \"warnings\": 0,\n    \"fixed\": 0\n  },\n",
    "  \"findings\": []\n}\n",
    "JSON\nexit 0\n"
);

#[cfg(unix)]
#[test]
fn a_language_specific_analyzer_completes_because_foreign_classes_are_skipped() {
    // THE defect this build fixes. `run_suite` used to hand all nineteen
    // fixtures to whichever analyzer was selected, and a language-specific tool
    // given a repository in the wrong language exits non-zero — knip 2,
    // cargo-shear 2, deadcode 1. `CommandSut` correctly refuses to call those
    // exit codes healthy, because each is shared with a genuine analysis
    // failure whose output is equally empty (§6.20), so the whole run aborted
    // on the first foreign class and `judged mutants` could grade exactly one
    // of the four analyzers it has adapters for.
    //
    // The fix is a declaration, not a wider exit-code list: a SUT says which
    // ecosystems it can read, and a class outside them is never materialized
    // and never handed over. What is asserted here is that the run now
    // *completes* — reaches a gate line for every one of the nineteen classes —
    // against shims that refuse a foreign repository exactly as the real tools
    // were measured doing.
    //
    // The unread counts are the second half of the assertion and they are not
    // cosmetic: they are what stops a skipped class being read as a passed one.
    for (sut, binary, body, unread) in [
        ("knip", "npx", KNIP_ECOSYSTEM_AWARE_SHIM, 16),
        ("deadcode", "deadcode", DEADCODE_ECOSYSTEM_AWARE_SHIM, 18),
        ("shear", "cargo-shear", SHEAR_ECOSYSTEM_AWARE_SHIM, 13),
    ] {
        let repo = scratch(&format!("{sut}-skips-foreign"));
        let bin = scratch(&format!("{sut}-skips-foreign-path"));
        shim(bin.path(), binary, body);

        let run = judged_with_path(
            repo.path(),
            &["mutants", "--sut", sut],
            Some(Path::new(&path_with(bin.path()))),
        );

        run.expect_code(
            0,
            "the analyzer completed every class it can read and claimed nothing in them",
        )
        .expect_says("[NOT READ by this SUT]")
        .expect_says(&format!("not measured: {unread} of 19 classes"))
        .expect_says("false removals: 0");
        // Every class still appears. A skipped class that vanished from the
        // report would be indistinguishable from one that was never in the
        // catalogue.
        expect_every_class_is_reported(&run);
    }
}

#[cfg(unix)]
#[test]
fn a_skipped_class_is_never_counted_as_a_pass() {
    // The whole risk of the skipping feature, stated as arithmetic. If "not
    // read" quietly became "passed", then narrowing an adapter's declared
    // languages would be a way to raise a green — and the narrowest possible
    // declaration would score a perfect run (§6.20: no data is not zero
    // findings).
    //
    // deadcode reads Go, and the catalogue holds exactly one Go class, so the
    // summary line must account for eighteen classes it did not attempt and
    // must not fold them into either column.
    let repo = scratch("deadcode-arithmetic");
    let bin = scratch("deadcode-arithmetic-path");
    shim(bin.path(), "deadcode", DEADCODE_ECOSYSTEM_AWARE_SHIM);

    let run = judged_with_path(
        repo.path(),
        &["mutants", "--sut", "deadcode"],
        Some(Path::new(&path_with(bin.path()))),
    );

    run.expect_code(0, "the one class it read produced no false removal");

    // The summary must be over the classes that were graded, not over all
    // nineteen. `1 class graded` is the honest denominator when eighteen were
    // never opened.
    let summary = run
        .stdout
        .lines()
        .find(|line| line.starts_with("19 classes:"))
        .unwrap_or_else(|| panic!("no class summary line. Report was:\n{}", run.stdout));
    assert!(
        summary.contains("1 graded"),
        "the summary must say how many classes were actually graded, or eighteen \
         unattempted classes are read as results. Got `{summary}`"
    );
    assert!(
        summary.contains("18 not read"),
        "the skipped classes must be counted in their own column, never in the \
         passed or failed one. Got `{summary}`"
    );
    // And the rows themselves must not claim a verdict they did not reach.
    // Read out of the verdict column rather than by searching the whole row:
    // several mechanisms are described in prose containing the word "passed"
    // (m02's is "…passed to importlib"), and a substring search over the row
    // would find those and pass for the wrong reason.
    let mut unread_rows = 0;
    for row in class_rows(&run.stdout) {
        if !row.contains("[NOT READ by this SUT]") {
            continue;
        }
        unread_rows += 1;
        let verdict = row
            .split_whitespace()
            .nth(1)
            .unwrap_or_else(|| panic!("row has no verdict column: `{row}`"));
        assert_eq!(
            verdict, "----",
            "a class the analyzer never opened carries the verdict `{verdict}`: `{row}`"
        );
    }
    assert_eq!(
        unread_rows, 18,
        "eighteen classes are outside Go and every one must be marked on its own row"
    );
}

#[cfg(unix)]
#[test]
fn an_analyzer_that_fails_inside_its_own_language_still_stops_the_run() {
    // The other direction, and the one that keeps skipping honest. Once foreign
    // classes are skipped, every class the analyzer is actually handed is one it
    // declared it can read — so a non-zero exit there is a genuine analysis
    // failure and must still abort the run rather than be waved through as
    // another skip.
    //
    // The shim refuses every repository, cargo-shear's measured
    // out-of-ecosystem message and exit code. Since the Python and polyglot
    // classes are now skipped, the class it stops on is the first Rust one,
    // m04 — not m01, which is where the pre-skip build died.
    let repo = scratch("shear-broken");
    let bin = scratch("shear-broken-path");
    shim(
        bin.path(),
        "cargo-shear",
        "#!/bin/sh\necho 'error: Metadata error: `cargo metadata` exited with an error: '\n\
         echo 'error: could not find `Cargo.toml`' >&2\nexit 2\n",
    );

    let run = judged_with_path(
        repo.path(),
        &["mutants", "--sut", "shear"],
        Some(Path::new(&path_with(bin.path()))),
    );

    run.expect_code(2, "a suite that did not finish is not a suite that passed")
        // The tool and the class it stopped on, which `CommandSut` already
        // supplies.
        .expect_says("cargo-shear")
        .expect_says("m04")
        // And the language it declares, so the reader can see that this failure
        // is *inside* the analyzer's own ecosystem and is therefore not a
        // language mismatch to be shrugged off.
        .expect_says("rust")
        .expect_says("13 of 19");
    expect_no_gate_result(&run);
}

// ---------------------------------------------------------------------------
// judged show-roots — ProGuard `-printseeds`, which §9.13 asks for by name
//
// The root set does not decide what is reachable. §1.2: you cannot infer the
// closed world, you can only have it declared. So what these tests hold the
// command to is not a count but an audit: every root says which §5.1 tier it
// came from and which file and key declared it, and everything the materializer
// could NOT resolve is printed beside the successes. A root list showing only
// what worked hides exactly the gaps a reader needs.
// ---------------------------------------------------------------------------

/// A repository with one root of each tier, and one gap.
///
/// - **A** — `package.json` declares a `bin`, which npm already reads to find an
///   entry point.
/// - **B** — a Django app: `INSTALLED_APPS` names the package, and the
///   `AppConfig` subclass in `apps.py` is named nowhere but its own declaration.
/// - **C** — `.judged/roots.toml` says a human knows something the repository
///   cannot show.
/// - the gap — `flask` is recognized and has no plugin, so its conventions are
///   all missing.
fn repo_with_roots_of_every_tier(label: &str) -> TempDir {
    let scratch = scratch(label);
    let root = scratch.path();
    Repo::init(root).expect("scratch must be a git working tree");

    write(
        root,
        "package.json",
        r#"{
  "name": "billing-web",
  "version": "0.1.0",
  "bin": { "billing": "bin/billing.js" }
}
"#,
    );
    write(root, "bin/billing.js", "#!/usr/bin/env node\n");
    write(
        root,
        "pyproject.toml",
        "[project]\nname = \"billing\"\nversion = \"0.1.0\"\n\
         dependencies = [\"django>=4.2\", \"flask>=3\"]\n",
    );
    write(
        root,
        "billing/settings.py",
        "INSTALLED_APPS = [\n    \"reporting\",\n]\n",
    );
    write(root, "reporting/__init__.py", "");
    write(
        root,
        "reporting/apps.py",
        "from django.apps import AppConfig\n\n\n\
         class ReportingConfig(AppConfig):\n    name = \"reporting\"\n",
    );
    write(root, "ops/restore.sh", "#!/bin/sh\necho restore\n");
    write(
        root,
        ".judged/roots.toml",
        "[[root]]\npath = \"ops/restore.sh\"\n\
         reason = \"run by hand during an incident; no caller exists in this repo\"\n\
         kind = \"external\"\nstatus = \"accepted\"\n",
    );
    scratch
}

#[test]
fn show_roots_prints_every_tier_with_the_file_and_key_that_declared_it() {
    let repo = repo_with_roots_of_every_tier("show-roots");

    let run = judged(repo.path(), &["show-roots"]);

    run.expect_code(0, "materializing a root set is a report, not a gate")
        // Tier A: npm already reads this key to find the entry point.
        .expect_says("package.json#bin.billing")
        // Tier B: the convention, named per RULE rather than per framework, so
        // the report says *why* the file is a root (§11 R2 wants each rule's
        // fire rate measurable rather than guessed).
        .expect_says("django/appconfig")
        .expect_says("reporting/apps.py")
        // Tier C: the line in the committed file a reader goes to in order to
        // argue with the declaration.
        .expect_says(".judged/roots.toml:1")
        .expect_says("ops/restore.sh");

    // Grouped by tier, in decreasing order of confidence, each group labelled
    // with the caveat that applies to it. A root that does not say which tier it
    // came from is worse than no root.
    assert!(
        run.offset_of("# tier A") < run.offset_of("# tier B")
            && run.offset_of("# tier B") < run.offset_of("# tier C"),
        "roots must be grouped A, B, C — the tiers do not deserve equal trust:\n{}",
        run.stdout
    );
    run.expect_says("correct only if the framework AND its version were detected correctly");
}

#[test]
fn show_roots_prints_what_it_could_not_resolve() {
    let repo = repo_with_roots_of_every_tier("show-roots-gaps");

    let run = judged(repo.path(), &["show-roots"]);

    // §6.20: "no data" must be a distinct state from "zero". Flask is
    // recognized and uncovered, so every convention root it would have
    // contributed is missing — and a report that showed only the roots it did
    // find would be indistinguishable from one where flask has no roots.
    run.expect_code(0, "a reported gap is not a refusal")
        .expect_says("could not resolve")
        .expect_says("framework-without-plugin")
        .expect_says("flask");
}

#[test]
fn show_roots_reports_a_declared_entry_that_matched_nothing() {
    let repo = scratch("show-roots-rot");
    Repo::init(repo.path()).expect("scratch must be a git working tree");
    write(repo.path(), "src/main.py", "print('hi')\n");
    write(
        repo.path(),
        ".judged/roots.toml",
        "[[root]]\npath = \"tools/gone.py\"\n\
         reason = \"kept for a migration that finished last year\"\n\
         kind = \"external\"\nstatus = \"accepted\"\n",
    );

    let run = judged(repo.path(), &["show-roots"]);

    // §5.3's rot detection: a suppression list without it is the off switch.
    // The entry names an exact path that is not there, so it can never match
    // again — the strongest statement available, and a pattern protecting
    // nothing is a blind spot nobody is watching.
    run.expect_code(0, "rot is reported, not refused")
        .expect_says("declared-root-rot")
        .expect_says("tools/gone.py");
}

#[test]
fn show_roots_malformed_declarations_are_a_loud_gap_not_a_shorter_list() {
    let repo = scratch("show-roots-malformed");
    Repo::init(repo.path()).expect("scratch must be a git working tree");
    write(repo.path(), "src/main.py", "print('hi')\n");
    // No `reason`, which §5.3 makes mandatory: an entry without one is
    // unreviewable.
    write(
        repo.path(),
        ".judged/roots.toml",
        "[[root]]\npath = \"src/main.py\"\nkind = \"external\"\n",
    );

    let run = judged(repo.path(), &["show-roots"]);

    run.expect_says("declared-roots-malformed")
        .expect_says("Every Tier C root is missing");
}

#[test]
fn show_roots_json_carries_the_tier_and_the_origin_of_every_root() {
    let repo = repo_with_roots_of_every_tier("show-roots-json");

    let run = judged(repo.path(), &["show-roots", "--json"]);
    run.expect_code(0, "the JSON rendering is the same report");

    let report: Value =
        serde_json::from_str(run.stdout.trim()).expect("--json emits a JSON document");

    let roots = report["roots"]
        .as_array()
        .unwrap_or_else(|| panic!("the report carries a roots array: {report}"));
    assert!(!roots.is_empty(), "the fixture declares roots: {report}");

    let mut tiers: Vec<&str> = Vec::new();
    for root in roots {
        let tier = root["tier"]
            .as_str()
            .unwrap_or_else(|| panic!("every root states its §5.1 tier: {root}"));
        assert!(
            ["A", "B", "C"].contains(&tier),
            "a root that does not say which tier it came from is worse than no \
             root: {root}"
        );
        assert!(
            root["origin"].as_str().is_some_and(|o| !o.is_empty()),
            "every root names the exact file and key it came from: {root}"
        );
        assert!(
            root["rule"].as_str().is_some_and(|r| !r.is_empty()),
            "every root says which rule produced it: {root}"
        );
        tiers.push(tier);
    }
    for tier in ["A", "B", "C"] {
        assert!(
            tiers.contains(&tier),
            "the fixture declares a tier {tier} root and the report has none: {report}"
        );
    }

    // The gaps are a first-class key, not prose. A consumer that reads
    // `roots` without `gaps` has read a list of successes.
    let gaps = report["gaps"]
        .as_array()
        .unwrap_or_else(|| panic!("the report carries a gaps array: {report}"));
    assert!(
        gaps.iter()
            .any(|gap| gap["kind"] == json!("framework-without-plugin")),
        "flask is recognized and uncovered: {report}"
    );
    assert_eq!(
        report["gap_count"],
        json!(gaps.len()),
        "the count and the list must agree: {report}"
    );
}

#[test]
fn show_roots_refuses_a_directory_it_could_not_read() {
    let repo = scratch("show-roots-missing");

    let run = judged(repo.path(), &["show-roots", "nowhere-at-all"]);

    // Zero roots over zero files read is the absence of a scan wearing the
    // digits of an empty repository (§6.20). It is refused rather than printed
    // as a clean, empty root set.
    run.expect_code(2, "a scan that read nothing has not produced a root set")
        .expect_says("REFUSED");
}

#[test]
fn help_documents_show_roots_as_printseeds() {
    let repo = scratch("show-roots-help");

    judged(repo.path(), &["--help"])
        .expect_code(0, "help is requested, not provoked")
        .expect_says("show-roots")
        .expect_says("printseeds");
}

// ---------------------------------------------------------------------------
// judged mutants --roots — the layer Gate 2 structurally cannot be
//
// §11 R1 asks whether any signal COMBINATION clears the catalogue. The veto and
// the root set answer different questions — "does anything name this" against
// "was this declared an entry point" — so they are separate flags. Four
// configurations, each measurable: bare, --veto, --roots, both. A combined-only
// flag would hide which layer earned a rescue.
// ---------------------------------------------------------------------------

#[test]
fn roots_rescue_independently_of_the_veto_and_each_layer_is_named() {
    let repo = scratch("mutants-roots");

    let bare: Value = {
        let run = judged(repo.path(), &["mutants", "--sut", "naive", "--json"]);
        serde_json::from_str(run.stdout.trim()).expect("--json emits JSON")
    };
    let run = judged(
        repo.path(),
        &["mutants", "--sut", "naive", "--roots", "--json"],
    );
    let rooted: Value =
        serde_json::from_str(run.stdout.trim()).expect("--json emits JSON under --roots too");

    assert_eq!(
        rooted["sut"], "naive+roots",
        "the measured system is the pair, and the name says which layer was in it"
    );
    assert!(
        bare["roots"].is_null(),
        "a bare run carries no roots key, so the two are told apart by presence"
    );
    assert!(
        bare["veto"].is_null() && rooted["veto"].is_null(),
        "--roots must not turn the veto on: the whole point is to attribute a \
         rescue to the layer that made it"
    );

    let roots = &rooted["roots"];
    let prevented = roots["false_removals_prevented"]
        .as_u64()
        .expect("false_removals_prevented is a number");
    assert!(
        prevented > 0,
        "the root set rescued nothing across 19 classes, which means it did not \
         run: {roots}"
    );
    assert_eq!(
        prevented + roots["false_removals_remaining"].as_u64().unwrap(),
        bare["false_removal_count"].as_u64().unwrap(),
        "prevented + remaining must reconstruct the bare count exactly"
    );
    assert!(
        rooted["false_removal_count"].as_u64() <= bare["false_removal_count"].as_u64(),
        "a root set may only rescue: {rooted} against {bare}"
    );
    assert!(
        roots["decoys_found_rescued"].as_u64() <= roots["decoys_found_bare"].as_u64(),
        "a rescue layer cannot find a decoy the accuser missed: {roots}"
    );

    // Every rescue names its §5.1 tier and the file and key that declared it. A
    // root that does not say which tier it came from invites a caller to trust a
    // guessed convention as though a manifest had declared it.
    let mut checked = 0usize;
    for class in rooted["mutants"].as_array().expect("mutants is an array") {
        let Some(rescued) = class["roots"]["rescued_claims"].as_array() else {
            continue;
        };
        for record in rescued {
            assert_eq!(record["layer"], json!("roots"), "got {record}");
            let tier = record["tier"]
                .as_str()
                .unwrap_or_else(|| panic!("every rescue states its tier: {record}"));
            assert!(["A", "B", "C"].contains(&tier), "got {record}");
            assert!(
                record["origin"].as_str().is_some_and(|o| !o.is_empty()),
                "every rescue names the file and key that declared it: {record}"
            );
            assert!(
                record["rule"].as_str().is_some_and(|r| !r.is_empty()),
                "every rescue says which rule fired: {record}"
            );
            checked += 1;
        }
    }
    assert!(checked > 0, "no class recorded a rescue: {rooted}");

    // And the human rendering states the trade at column zero, in the same shape
    // the veto states it.
    let text = judged(repo.path(), &["mutants", "--sut", "naive", "--roots"]);
    for expected in ["roots prevented: ", "roots cost: "] {
        assert!(
            text.stdout.lines().any(|line| line.starts_with(expected)),
            "no line starts with `{expected}`; got {}",
            text.stdout
        );
    }
}

/// m10 is the class this whole layer exists for.
///
/// `ReportingConfig` occurs in `reporting/apps.py` and in no other file, because
/// Django finds it by scanning that file for an `AppConfig` subclass. No tuning
/// of Gate 2a's needles reaches that — there is no second occurrence to find —
/// so if m10 is rescued at all it is rescued here, by a Tier B convention.
#[test]
fn the_report_says_which_layer_rescued_m10() {
    let repo = scratch("mutants-roots-m10");

    let run = judged(
        repo.path(),
        &["mutants", "--sut", "naive", "--roots", "--json"],
    );
    let report: Value = serde_json::from_str(run.stdout.trim()).expect("--json emits JSON");

    let m10 = report["mutants"]
        .as_array()
        .expect("mutants is an array")
        .iter()
        .find(|class| class["id"] == json!("m10"))
        .expect("the catalogue contains m10");

    let rescued = m10["roots"]["rescued_claims"]
        .as_array()
        .unwrap_or_else(|| panic!("m10 records what the root set rescued: {m10}"));
    let appconfig = rescued
        .iter()
        .find(|record| record["claim"] == json!("ReportingConfig"))
        .unwrap_or_else(|| {
            panic!(
                "m10's AppConfig subclass is a Tier B convention root and the \
                 only layer that can rescue it is this one: {m10}"
            )
        });
    assert_eq!(appconfig["tier"], json!("B"));
    assert_eq!(appconfig["rule"], json!("django/appconfig"));
}

/// Both layers at once, with each rescue still attributed to the layer that made
/// it. A combined number cannot answer §11 R1's question, which is about which
/// signals earn their place.
#[test]
fn both_layers_compose_and_the_report_attributes_each_rescue() {
    let repo = scratch("mutants-roots-veto");

    let run = judged(
        repo.path(),
        &["mutants", "--sut", "naive", "--roots", "--veto", "--json"],
    );
    let report: Value = serde_json::from_str(run.stdout.trim()).expect("--json emits JSON");

    assert_eq!(
        report["sut"], "naive+roots+veto",
        "the name states the whole stack, inner layer first"
    );
    let rescue = &report["rescue"];
    let layers = rescue["layers"]
        .as_array()
        .unwrap_or_else(|| panic!("the stack names its layers: {rescue}"));
    assert_eq!(
        layers
            .iter()
            .map(|layer| layer["name"].as_str().unwrap_or_default())
            .collect::<Vec<_>>(),
        vec!["roots", "veto"],
        "the root set is consulted first: a candidate that IS a root is not a \
         candidate at all, so there is nothing for a reference veto to weigh"
    );
    // Each layer's share, and the two must add up to the stack's total.
    let by_layer: u64 = layers
        .iter()
        .map(|layer| layer["false_removals_prevented"].as_u64().unwrap_or(0))
        .sum();
    assert_eq!(
        by_layer
            + rescue["false_removals_prevented_unattributed"]
                .as_u64()
                .unwrap_or(0),
        rescue["false_removals_prevented"].as_u64().unwrap(),
        "every prevented false removal is attributed to a layer, or counted as \
         unattributed — never quietly folded into a total: {rescue}"
    );
    assert_eq!(
        rescue["false_removals_prevented"].as_u64().unwrap()
            + rescue["false_removals_remaining"].as_u64().unwrap(),
        rescue["false_removals_bare"].as_u64().unwrap(),
    );
}

#[test]
fn help_documents_roots_as_a_layer_of_its_own() {
    let repo = scratch("mutants-roots-help");

    judged(repo.path(), &["--help"])
        .expect_code(0, "help is requested, not provoked")
        .expect_says("--roots")
        .expect_says("may only rescue");
}
