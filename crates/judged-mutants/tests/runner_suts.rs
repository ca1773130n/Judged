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
//!
//! [`CommandSut`] is neither control but the thing they exist to make
//! trustworthy: an arbitrary external analyzer, graded without Judged knowing
//! anything about it. Its tests are almost entirely about failure, because §9.2
//! and §6.20 say the only outcome that must never occur is a crashed tool
//! scoring like a careful one.

use std::fs;
use std::path::{Path, PathBuf};

use judged_core::{Error, Result};
use judged_mutants::sut::{CommandSut, NaiveSut, RefusingSut, Sut, SutVerdict};
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

// ---------------------------------------------------------------------------
// The capability envelope — §9.2's first non-SARIF clause.
//
// "Every adapter declares which finding classes it can and structurally CANNOT
// emit ... This is what lets the orchestrator know when silence means
// anything." Silence is the default output of every broken analyzer, so an
// undeclared blind spot is indistinguishable from a clean bill of health.
// ---------------------------------------------------------------------------

/// A SUT that declares nothing, to pin the trait default.
struct BareSut;

impl Sut for BareSut {
    fn name(&self) -> &str {
        "bare"
    }
    fn run(&self, _repo: &Path) -> Result<SutVerdict> {
        Ok(SutVerdict::default())
    }
}

#[test]
fn a_sut_that_declares_nothing_has_an_empty_capability_envelope() {
    // The default has to be "I claim no structural blind spots", not "I claim
    // total blindness": the envelope is an assertion the SUT author makes, and
    // a default that invented one would be putting words in their mouth.
    assert!(
        BareSut.cannot_emit().is_empty(),
        "the default envelope must be empty; got {:?}",
        BareSut.cannot_emit()
    );
}

#[test]
fn refusing_sut_declares_that_its_silence_is_never_evidence() {
    let dir = repo(&[("src/main.rs", "fn main() {}\n")]);

    let verdict = RefusingSut.run(dir.path()).expect("refusing sut runs");
    let envelope = RefusingSut.cannot_emit();

    assert_eq!(verdict, SutVerdict::default());
    assert!(
        !envelope.is_empty(),
        "a SUT whose verdict is unconditionally empty scores zero false \
         removals — a perfect result. Without a declared envelope that score \
         reads as competence, which is exactly the confusion §9.2 exists to \
         prevent"
    );
}

#[test]
fn naive_sut_declares_the_two_blind_spots_its_own_tests_prove() {
    // Not decorative. `naive_sut_claims_an_exported_symbol_...` and
    // `naive_sut_spares_conventional_entry_points` demonstrate these two
    // structural limits; the envelope is the machine-readable form of the same
    // facts, so the orchestrator can tell "scanned it, nothing there" from
    // "never looked".
    let envelope = NaiveSut.cannot_emit();

    assert!(
        envelope.iter().any(|class| class.contains("symbol")),
        "declarations are only scanned in parsed extensions, so silence about \
         a symbol elsewhere is not evidence; got {envelope:?}"
    );
    assert!(
        envelope.iter().any(|class| class.contains("entry point")),
        "entry-point-named artifacts are never claimed, so silence about one \
         is not evidence; got {envelope:?}"
    );
}

#[test]
fn capability_envelopes_stay_a_short_list_of_strings() {
    // §9.2 asks for a declaration, not a taxonomy. A blank or essay-length
    // entry is not a declaration anyone can act on.
    for envelope in [NaiveSut.cannot_emit(), RefusingSut.cannot_emit()] {
        for class in &envelope {
            assert!(!class.trim().is_empty(), "an empty class declares nothing");
            assert!(
                class.len() <= 200,
                "an envelope entry is a short declaration, not prose: {class:?}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// `CommandSut` — grading an arbitrary external tool.
//
// Exercised with `/bin/sh` scripts rather than a real analyzer, so the failure
// modes below are reproduced exactly and on demand. That makes this module
// Unix-only; the implementation itself is plain `std::process`.
// ---------------------------------------------------------------------------
#[cfg(unix)]
mod command_sut {
    use super::*;

    /// Write an executable `/bin/sh` script into `dir` and return its path.
    fn script(dir: &TempDir, name: &str, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;

        let path = dir.path().join(name);
        fs::write(&path, format!("#!/bin/sh\n{body}\n")).expect("write script");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod");
        path
    }

    /// Every non-blank line of stdout is a claimed dead path. Blank stdout is a
    /// legitimate "I found nothing", which is the distinction under test.
    fn parse_paths(stdout: &str) -> Result<SutVerdict> {
        Ok(SutVerdict {
            claimed_dead_paths: stdout
                .lines()
                .filter(|line| !line.trim().is_empty())
                .map(PathBuf::from)
                .collect(),
            claimed_dead_symbols: Vec::new(),
        })
    }

    /// Every non-blank line of stdout is a claimed dead *symbol*.
    ///
    /// Used by the tests that observe argv and the working directory, because
    /// `claimed_dead_paths` is normalized against the repo root on the way out
    /// and would not report back the absolute strings those tests are checking.
    fn parse_symbols(stdout: &str) -> Result<SutVerdict> {
        Ok(SutVerdict {
            claimed_dead_paths: Vec::new(),
            claimed_dead_symbols: stdout
                .lines()
                .filter(|line| !line.trim().is_empty())
                .map(str::to_string)
                .collect(),
        })
    }

    /// Rejects anything that does not look like a bare path. Stands in for a
    /// real adapter's JSON decode: output it cannot read is an error.
    fn parse_strict(stdout: &str) -> Result<SutVerdict> {
        for line in stdout.lines().filter(|line| !line.trim().is_empty()) {
            if line.contains(' ') {
                return Err(Error::Sut {
                    sut: "strict".into(),
                    message: format!("unreadable line {line:?}"),
                });
            }
        }
        parse_paths(stdout)
    }

    /// The repo path as the child process will see it. macOS hands out
    /// `/var/folders/...` temp dirs that are a symlink to `/private/var/...`,
    /// and `pwd -P` resolves them.
    fn real(dir: &TempDir) -> String {
        dir.path()
            .canonicalize()
            .expect("canonicalize repo")
            .to_string_lossy()
            .into_owned()
    }

    fn claimed(verdict: &SutVerdict) -> Vec<String> {
        verdict
            .claimed_dead_paths
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn an_exit_zero_run_that_finds_nothing_is_an_empty_verdict() {
        // The one case in this module that is allowed to be empty, and the
        // reason every other case must not be.
        let bin = TempDir::new().expect("bin dir");
        let dir = repo(&[("src/main.rs", "fn main() {}\n")]);
        let sut = CommandSut::new("clean", script(&bin, "clean.sh", "exit 0"), parse_paths);

        let verdict = sut
            .run(dir.path())
            .expect("a clean exit-0 run must succeed");

        assert_eq!(
            verdict,
            SutVerdict::default(),
            "a tool that ran fine and found nothing claims nothing"
        );
    }

    #[test]
    fn stdout_is_handed_to_the_supplied_parser() {
        let bin = TempDir::new().expect("bin dir");
        let dir = repo(&[("src/main.rs", "fn main() {}\n")]);
        let sut = CommandSut::new(
            "finder",
            script(&bin, "finder.sh", "printf 'a/b.py\\nc/d.py\\n'"),
            parse_paths,
        );

        let verdict = sut.run(dir.path()).expect("sut runs");

        assert_eq!(claimed(&verdict), vec!["a/b.py", "c/d.py"]);
        assert_eq!(sut.name(), "finder");
    }

    #[test]
    fn the_command_runs_inside_the_repo_and_is_handed_its_path() {
        let bin = TempDir::new().expect("bin dir");
        let dir = repo(&[("src/main.rs", "fn main() {}\n")]);
        let sut = CommandSut::new(
            "locator",
            script(&bin, "locator.sh", "pwd -P\nprintf '%s\\n' \"$1\""),
            parse_symbols,
        );

        let verdict = sut.run(dir.path()).expect("sut runs");

        let here = real(&dir);
        assert_eq!(
            verdict.claimed_dead_symbols,
            vec![here.clone(), here],
            "the contract is both: cwd is the fixture repo, and the repo path \
             is the last argument"
        );
    }

    #[test]
    fn configured_args_come_before_the_repo_path() {
        let bin = TempDir::new().expect("bin dir");
        let dir = repo(&[("src/main.rs", "fn main() {}\n")]);
        let sut = CommandSut::new(
            "flagged",
            script(
                &bin,
                "flagged.sh",
                "printf '%s\\n%s\\n%s\\n' \"$1\" \"$2\" \"$3\"",
            ),
            parse_symbols,
        )
        .with_args(["--json", "--quiet"]);

        let verdict = sut.run(dir.path()).expect("sut runs");

        assert_eq!(
            verdict.claimed_dead_symbols,
            vec!["--json".to_string(), "--quiet".to_string(), real(&dir)],
            "an adapter's flags must not displace the repo path"
        );
    }

    #[test]
    fn absolute_claimed_paths_are_made_repo_relative() {
        // A real analyzer echoes back the path it was given, so being handed an
        // absolute repo path means absolute findings. `SutVerdict` documents its
        // paths as repo-relative, and the parser cannot fix this itself — it
        // only ever sees stdout and has no idea where the repo is. If the
        // absolute form escaped to the grader it would strip against nothing,
        // match no live path, and the run would score clean: a false removal
        // silently converted into a pass.
        let bin = TempDir::new().expect("bin dir");
        let dir = repo(&[("src/main.rs", "fn main() {}\n")]);
        let sut = CommandSut::new(
            "echoing",
            script(
                &bin,
                "echoing.sh",
                "printf '%s/app/dead.py\\n' \"$1\"\nprintf 'app/other.py\\n'",
            ),
            parse_paths,
        );

        let verdict = sut.run(dir.path()).expect("sut runs");

        assert_eq!(
            claimed(&verdict),
            vec!["app/dead.py", "app/other.py"],
            "absolute findings under the repo must arrive repo-relative, and \
             already-relative ones must be left alone"
        );
    }

    #[test]
    fn a_claimed_path_outside_the_repo_is_a_loud_error() {
        // §9.3 gate 0c: "reject any candidate whose realpath is not a repo
        // descendant." Passing it through would be worse than useless — it can
        // never match ground truth, so it would quietly vanish from the score
        // while the tool went on record as wanting to delete /etc/passwd.
        let bin = TempDir::new().expect("bin dir");
        let dir = repo(&[("src/main.rs", "fn main() {}\n")]);

        for (label, line) in [
            ("an absolute path elsewhere", "/etc/passwd"),
            ("a relative path that climbs out", "../../etc/passwd"),
        ] {
            let sut = CommandSut::new(
                "escapee",
                script(&bin, "escapee.sh", &format!("printf '{line}\\n'")),
                parse_paths,
            );

            match sut.run(dir.path()) {
                Ok(verdict) => panic!(
                    "{label} was accepted instead of rejected: {:?}",
                    verdict.claimed_dead_paths
                ),
                Err(error) => {
                    let text = error.to_string();
                    assert!(text.contains("escapee"), "must name the SUT; got {text}");
                    assert!(
                        text.contains("passwd"),
                        "must name the path it refused; got {text}"
                    );
                }
            }
        }
    }

    #[test]
    fn claiming_the_repository_root_itself_is_a_loud_error() {
        // Stripping the root off the root leaves an empty path, which matches
        // no live artifact and would score as claiming nothing — the most
        // destructive verdict a cleaner can reach arriving as a perfect one.
        let bin = TempDir::new().expect("bin dir");
        let dir = repo(&[("src/main.rs", "fn main() {}\n")]);
        let sut = CommandSut::new(
            "greedy",
            script(&bin, "greedy.sh", "printf '%s\\n' \"$1\""),
            parse_paths,
        );

        let error = sut
            .run(dir.path())
            .expect_err("deleting the whole repo is not an empty verdict");

        let text = error.to_string();
        assert!(text.contains("greedy"), "must name the SUT; got {text}");
        assert!(
            text.contains("root"),
            "must say what was claimed; got {text}"
        );
    }

    #[test]
    fn a_non_zero_exit_is_a_loud_error_and_not_an_empty_verdict() {
        let bin = TempDir::new().expect("bin dir");
        let dir = repo(&[("src/main.rs", "fn main() {}\n")]);
        let sut = CommandSut::new(
            "brokes",
            script(
                &bin,
                "brokes.sh",
                "printf 'boom: config not found\\n' >&2\nexit 2",
            ),
            parse_paths,
        );

        let error = sut
            .run(dir.path())
            .expect_err("a non-zero exit must not be graded");

        let text = error.to_string();
        assert!(text.contains("brokes"), "must name the SUT; got {text}");
        assert!(text.contains("exit"), "must say it exited; got {text}");
        assert!(
            text.contains('2'),
            "must report the exit status; got {text}"
        );
        assert!(
            text.contains("boom: config not found"),
            "must carry what the tool said on stderr, or the failure is not \
             actionable — that line is how a missing plugin is told apart from \
             a broken fixture; got {text}"
        );
    }

    #[test]
    fn stdout_written_before_a_non_zero_exit_is_still_an_error() {
        // The dangerous shape, and the reason exit status is checked before the
        // parser is ever called: an analyzer that emitted perfectly parseable
        // output and then died has NOT finished the analysis. Parsing what it
        // managed to print yields a short, plausible, wrong verdict.
        let bin = TempDir::new().expect("bin dir");
        let dir = repo(&[("src/main.rs", "fn main() {}\n")]);
        let sut = CommandSut::new(
            "half",
            script(&bin, "half.sh", "printf 'a/b.py\\n'\nexit 1"),
            parse_paths,
        );

        let error = sut
            .run(dir.path())
            .expect_err("parseable stdout does not redeem a failed run");

        let text = error.to_string();
        assert!(text.contains("half"), "must name the SUT; got {text}");
        assert!(text.contains("exit"), "must say it exited; got {text}");
        assert!(
            text.contains('1'),
            "must report the exit status; got {text}"
        );
        assert!(
            text.contains("stdout"),
            "must say the partial output was discarded rather than leave a \
             reader wondering whether it was used; got {text}"
        );
    }

    #[test]
    fn a_command_that_cannot_be_spawned_is_a_loud_error() {
        let bin = TempDir::new().expect("bin dir");
        let dir = repo(&[("src/main.rs", "fn main() {}\n")]);
        let missing = bin.path().join("no-such-analyzer");
        let sut = CommandSut::new("absent", missing, parse_paths);

        let error = sut
            .run(dir.path())
            .expect_err("an uninstalled tool must not grade as claiming nothing");

        let text = error.to_string();
        assert!(text.contains("absent"), "must name the SUT; got {text}");
        assert!(
            text.contains("no-such-analyzer"),
            "must name the program that could not be run; got {text}"
        );
    }

    #[test]
    fn a_command_killed_by_a_signal_is_a_loud_error() {
        // No exit code exists at all here, so a `code() == Some(0)` check that
        // used `unwrap_or(0)` would read this as success.
        let bin = TempDir::new().expect("bin dir");
        let dir = repo(&[("src/main.rs", "fn main() {}\n")]);
        let sut = CommandSut::new(
            "doomed",
            script(&bin, "doomed.sh", "printf 'a/b.py\\n'\nkill -9 $$"),
            parse_paths,
        );

        let error = sut
            .run(dir.path())
            .expect_err("a killed analyzer must not be graded");

        let text = error.to_string();
        assert!(text.contains("doomed"), "must name the SUT; got {text}");
        assert!(
            text.contains("signal"),
            "must say it died rather than exited; got {text}"
        );
    }

    #[test]
    fn a_long_multibyte_stderr_line_is_truncated_without_panicking() {
        // Tracebacks are long and analyzers are not required to speak ASCII, so
        // the stderr tail gets cut mid-line — and a byte-indexed cut can land
        // inside a multibyte character. Panicking there would abort the run
        // *while it was reporting a tool failure*, turning an actionable error
        // into a harness crash. `x` shifts the ellipses off the 3-byte grid so
        // the 300-byte limit is guaranteed to fall mid-character.
        let bin = TempDir::new().expect("bin dir");
        let dir = repo(&[("src/main.rs", "fn main() {}\n")]);
        let sut = CommandSut::new(
            "verbose",
            script(
                &bin,
                "verbose.sh",
                "{ printf 'x'; for i in $(seq 1 200); do printf '…'; done; printf '\\n'; } >&2\nexit 1",
            ),
            parse_paths,
        );

        let error = sut
            .run(dir.path())
            .expect_err("a non-zero exit is still an error");

        let text = error.to_string();
        assert!(text.contains("verbose"), "must name the SUT; got {text}");
        assert!(
            text.contains('…'),
            "must carry the start of the stderr line; got {text}"
        );
        assert!(
            text.len() < 1_200,
            "the tail is a hint, not the whole log; got {} bytes",
            text.len()
        );
    }

    #[test]
    fn stdout_the_parser_cannot_read_is_an_error() {
        let bin = TempDir::new().expect("bin dir");
        let dir = repo(&[("src/main.rs", "fn main() {}\n")]);
        let sut = CommandSut::new(
            "chatty",
            script(&bin, "chatty.sh", "printf 'warning: index is stale\\n'"),
            parse_strict,
        );

        let error = sut
            .run(dir.path())
            .expect_err("unreadable stdout must not degrade to an empty verdict");

        let text = error.to_string();
        assert!(text.contains("chatty"), "must name the SUT; got {text}");
        assert!(
            text.contains("index is stale"),
            "must carry what the parser choked on; got {text}"
        );
    }

    #[test]
    fn stdout_that_is_not_utf8_is_an_error() {
        // `parse_paths` accepts anything, so the only thing that can reject
        // this is `CommandSut` itself. Lossy decoding would hand the parser
        // replacement characters and get back a confident, corrupt verdict.
        let bin = TempDir::new().expect("bin dir");
        let dir = repo(&[("src/main.rs", "fn main() {}\n")]);
        let sut = CommandSut::new(
            "garbled",
            script(&bin, "garbled.sh", "printf 'a/\\377\\376b.py\\n'"),
            parse_paths,
        );

        let error = sut
            .run(dir.path())
            .expect_err("undecodable stdout must not be parsed");

        let text = error.to_string();
        assert!(text.contains("garbled"), "must name the SUT; got {text}");
        assert!(
            text.contains("UTF-8") || text.contains("utf-8"),
            "must say why it could not be read; got {text}"
        );
    }

    #[test]
    fn an_adapter_may_declare_which_non_zero_exits_mean_success() {
        // §9.2: "adapters compute a health bit; the orchestrator never reads a
        // raw exit code." Ruff-shaped tools exit non-zero *because* they found
        // something. The allowance is per-SUT and opt-in, so the default stays
        // the strict one every test above depends on.
        let bin = TempDir::new().expect("bin dir");
        let dir = repo(&[("src/main.rs", "fn main() {}\n")]);
        let path = script(&bin, "findings.sh", "printf 'a/b.py\\n'\nexit 3");

        let declared =
            CommandSut::new("declared", path.clone(), parse_paths).with_success_exit_codes([0, 3]);
        let undeclared = CommandSut::new("undeclared", path, parse_paths);

        assert_eq!(
            claimed(
                &declared
                    .run(dir.path())
                    .expect("exit 3 was declared healthy")
            ),
            vec!["a/b.py"]
        );
        assert!(
            undeclared.run(dir.path()).is_err(),
            "an undeclared non-zero exit stays an error; the allowance must be \
             opt-in per SUT and never inferred"
        );
    }

    #[test]
    fn a_declared_success_code_still_does_not_excuse_a_signal() {
        let bin = TempDir::new().expect("bin dir");
        let dir = repo(&[("src/main.rs", "fn main() {}\n")]);
        let sut = CommandSut::new(
            "permissive",
            script(&bin, "permissive.sh", "kill -9 $$"),
            parse_paths,
        )
        .with_success_exit_codes([0, 1, 2, 3]);

        assert!(
            sut.run(dir.path()).is_err(),
            "a process that never reached an exit status cannot have declared one"
        );
    }

    #[test]
    fn no_failure_mode_is_reachable_as_an_empty_verdict() {
        // The point of the whole module, stated once. An empty `SutVerdict`
        // grades as zero false removals — a perfect score — so every way a tool
        // can fail must be distinguishable from the one way it can legitimately
        // stay silent. §6.20: "'no data' must be a distinct state from 'zero
        // executions'."
        let bin = TempDir::new().expect("bin dir");
        let dir = repo(&[("src/main.rs", "fn main() {}\n")]);

        let clean = CommandSut::new("clean", script(&bin, "ok.sh", "exit 0"), parse_paths);
        assert_eq!(
            clean.run(dir.path()).expect("clean run"),
            SutVerdict::default(),
            "the legitimate empty verdict"
        );

        let failures: Vec<(&str, CommandSut)> = vec![
            (
                "non-zero exit",
                CommandSut::new("f1", script(&bin, "f1.sh", "exit 2"), parse_paths),
            ),
            (
                "stdout then non-zero exit",
                CommandSut::new(
                    "f2",
                    script(&bin, "f2.sh", "printf 'x.py\\n'\nexit 1"),
                    parse_paths,
                ),
            ),
            (
                "missing binary",
                CommandSut::new("f3", bin.path().join("absent"), parse_paths),
            ),
            (
                "killed by a signal",
                CommandSut::new("f4", script(&bin, "f4.sh", "kill -9 $$"), parse_paths),
            ),
            (
                "unparseable stdout",
                CommandSut::new(
                    "f5",
                    script(&bin, "f5.sh", "printf 'oh no\\n'"),
                    parse_strict,
                ),
            ),
        ];

        for (label, sut) in &failures {
            match sut.run(dir.path()) {
                Err(_) => {}
                Ok(verdict) => panic!(
                    "{label} produced a verdict instead of an error: {verdict:?} — \
                     a crashed analyzer must never be able to earn a perfect score"
                ),
            }
        }
    }

    #[test]
    fn a_command_sut_carries_the_capability_envelope_it_was_given() {
        // Judged knows nothing about the tool, so it cannot infer the envelope.
        // §9.2's own example is the one under test here.
        let bin = TempDir::new().expect("bin dir");
        let plain = CommandSut::new("plain", script(&bin, "p.sh", "exit 0"), parse_paths);
        assert!(
            plain.cannot_emit().is_empty(),
            "an undeclared envelope is empty, not invented"
        );

        let declared = CommandSut::new("vulture-ish", script(&bin, "v.sh", "exit 0"), parse_paths)
            .with_cannot_emit([
                "cross-module references: global name-set difference cannot see \
                 them, so silence is not evidence",
            ]);
        assert_eq!(declared.cannot_emit().len(), 1);
        assert!(declared.cannot_emit()[0].contains("cross-module"));
    }

    #[test]
    fn the_command_sut_does_not_write_into_the_repo() {
        // §9.2's second non-SARIF clause: adapters are read-only, the
        // orchestrator owns 100% of mutations. `CommandSut` cannot stop a tool
        // from writing, but it must not add any writes of its own — no report
        // file dropped next to the sources, which would then be a candidate.
        let bin = TempDir::new().expect("bin dir");
        let dir = repo(&[("src/main.rs", "fn main() {}\n")]);
        let before = fs::read_dir(dir.path()).expect("read repo").count();

        let sut = CommandSut::new(
            "reader",
            script(&bin, "r.sh", "printf 'a.py\\n'"),
            parse_paths,
        );
        sut.run(dir.path()).expect("sut runs");

        assert_eq!(
            fs::read_dir(dir.path()).expect("read repo").count(),
            before,
            "the harness must not leave anything behind in the fixture repo"
        );
    }
}
