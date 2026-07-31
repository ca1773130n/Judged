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
use std::sync::atomic::{AtomicU32, Ordering};

use judged_core::git::Repo;
use judged_ratchet::baseline::BASELINE_PATH;
use serde_json::{json, Value};

// ---------------------------------------------------------------------------
// Scratch space
//
// `tempfile` is not a dependency of this crate. The few bytes of temp-directory
// handling these tests need live here instead, matching what
// `judged-ratchet/tests/ratchet.rs` already does.
// ---------------------------------------------------------------------------

/// A uniquely named directory that deletes itself when the test ends.
struct Scratch {
    path: PathBuf,
}

impl Scratch {
    fn new(label: &str) -> Scratch {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "judged-cli-{}-{label}-{unique}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("scratch directory must be creatable");
        Scratch { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }

    /// Write `body` to a repo-relative path, creating parents.
    fn write(&self, relative: &str, body: &str) -> PathBuf {
        let path = self.path.join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("parent directory must be creatable");
        }
        std::fs::write(&path, body).expect("scratch file must be writable");
        path
    }

    fn read(&self, relative: &str) -> String {
        std::fs::read_to_string(self.path.join(relative)).expect("file must be readable")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
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
fn repo_with_sources(label: &str) -> Scratch {
    let scratch = Scratch::new(label);
    Repo::init(scratch.path()).expect("scratch must be a git working tree");
    for name in ["a", "b", "c"] {
        scratch.write(&format!("src/{name}.ts"), "export const x = 1;\n");
    }
    scratch
}

// ---------------------------------------------------------------------------
// judged ratchet
// ---------------------------------------------------------------------------

#[test]
fn a_clean_ratchet_run_exits_zero() {
    let repo = repo_with_sources("clean");
    repo.write(
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
    repo.write(
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
fn a_new_finding_exits_one_and_is_named_in_the_report() {
    let repo = repo_with_sources("new-finding");
    repo.write(
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
    repo.write(
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
    repo.write("src/huge.ts", &"export const pad = 1;\n".repeat(4096));
    repo.write("src/tiny.ts", "1\n");
    repo.write(
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
    repo.write(
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
    repo.write(
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
    repo.write(
        "knip.sarif",
        &sarif_log(
            "knip",
            true,
            &["src/a.ts"],
            vec![finding("unused-export", "src/a.ts", "aaaa", "still here")],
        ),
    );
    repo.write(
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
    repo.write(
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
    repo.write("empty.sarif", r#"{"version":"2.1.0","runs":[]}"#);

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
    repo.write(
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
    repo.write(
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

    let written = repo.read(BASELINE_PATH);
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
fn update_refuses_to_rewrite_the_baseline_from_a_crashed_run() {
    // The most destructive thing this binary could do. A crashed analyzer
    // reports zero findings; baselining that would erase the accepted backlog
    // and leave a green CI over a repository nobody has analyzed since.
    let repo = repo_with_sources("update-crashed");
    repo.write(
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
    repo.write(BASELINE_PATH, &original);

    let run = judged(
        repo.path(),
        &["ratchet", "--sarif", "crash.sarif", "--update"],
    );

    run.expect_code(2, "a refused run must not be allowed to rewrite anything")
        .expect_says("REFUSED");
    assert_eq!(
        repo.read(BASELINE_PATH),
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
    repo.write(
        "knip.sarif",
        &sarif_log(
            "knip",
            true,
            &["src/a.ts"],
            vec![finding("unused-export", "src/a.ts", "aaaa", "ts side")],
        ),
    );
    repo.write(
        "vulture.sarif",
        &sarif_log(
            "vulture",
            true,
            &["src/b.ts"],
            vec![finding("unused-function", "src/b.ts", "bbbb", "py side")],
        ),
    );
    repo.write(
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
    repo.write(
        "knip.sarif",
        &sarif_log(
            "knip",
            true,
            &["src/a.ts"],
            vec![finding("unused-export", "src/a.ts", "aaaa", "one of many")],
        ),
    );
    repo.write(
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

#[test]
fn mutants_refusing_exits_zero_because_it_removes_nothing() {
    let repo = Scratch::new("mutants-refusing");

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

    assert_eq!(
        run.stdout.matches("  m").count().min(E2_CLASSES),
        E2_CLASSES,
        "every one of the {E2_CLASSES} classes must appear in the report; a \
         silently skipped mutant reads as a pass the SUT never earned. Report \
         was:\n{}",
        run.stdout
    );
}

#[test]
fn mutants_naive_exits_non_zero_and_names_the_classes_it_failed() {
    // The headline result of the whole E2 body of work. §9.8: if breaking the
    // build does not break the gate, the gate is not a gate — so the naive
    // cleaner, which is §7.5's heuristic reproduced faithfully, has to come out
    // red, and the report has to say which injected liveness mechanisms caught
    // it.
    let repo = Scratch::new("mutants-naive");

    let run = judged(repo.path(), &["mutants", "--sut", "naive"]);

    run.expect_code(1, "a cleaner that removes live files must not pass")
        .expect_says("naive")
        .expect_says("GATE FAILED")
        .expect_says("classes with false removals:");

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
    let repo = Scratch::new("mutants-pair");

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
    let repo = Scratch::new("mutants-default");

    judged(repo.path(), &["mutants"])
        .expect_code(1, "the default SUT is the positive control")
        .expect_says("naive");
}

#[test]
fn mutants_json_carries_the_gate_and_every_class() {
    let repo = Scratch::new("mutants-json");

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
    let repo = Scratch::new("help");

    let run = judged(repo.path(), &["--help"]);
    run.expect_code(0, "help was asked for, not provoked")
        .expect_says("judged ratchet")
        .expect_says("judged mutants");

    judged(repo.path(), &["clean"])
        .expect_code(2, "there is no `judged clean`")
        .expect_says("ratchet");
}
