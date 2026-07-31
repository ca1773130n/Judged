//! The command line, and the flags that are refused on purpose.
//!
//! Hand-rolled rather than delegated to an argument-parsing crate. There are two
//! subcommands and six flags between them, and one of the flags is a list of
//! spellings that must be **rejected with an explanation** rather than reported
//! as unknown — §9.13 invariant 1 is a property of the interface, so it is
//! encoded in the interface's own code and pinned by its own test rather than
//! left as an absence somebody might helpfully fill in later.
//!
//! §9.13 also fixes the shape of the surface: CLI-first, exit codes as the
//! contract, no IDE mode, no PR bot in this build.

use std::path::PathBuf;

use judged_ratchet::baseline::BASELINE_PATH;

/// Flag spellings that are refused with an explanation instead of an
/// "unrecognized argument" shrug.
///
/// §9.13 invariant 1: *"There is no `--fix`, and there is no flag that
/// deletes."* Knip's own FAQ warns against Knip's own `--fix`, and every agent
/// skill surveyed in §7.5 that shipped one shipped `rm -rf`. Neither subcommand
/// here writes to the working tree at all, so none of these could be honoured
/// even in principle — but "unknown flag" reads as an oversight, and an
/// oversight is something a future contributor fixes.
///
/// `--quarantine` is on the list even though §9.13 names quarantine as the one
/// permitted mutating primitive, because the same sentence says `reap` is a
/// separate **verb**, never a flag on the analysis command. Accepting it here as
/// a flag would concede exactly the shape the invariant forbids.
const REFUSED_FLAGS: &[&str] = &[
    "--fix",
    "--autofix",
    "--auto-fix",
    "--delete",
    "--remove",
    "--rm",
    "--clean",
    "--reap",
    "--quarantine",
    "--prune",
    "--apply",
    "--force",
];

/// What the process was asked to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Invocation {
    /// §9.14 — baseline today's findings, fail CI only on new ones.
    Ratchet(RatchetArgs),
    /// §10 E2 — the mutation-injection suite.
    Mutants(MutantsArgs),
    /// Usage text, requested rather than provoked. Exits 0.
    Help,
}

/// `judged ratchet --sarif <path>... [--baseline <path>] [--update]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RatchetArgs {
    /// SARIF logs to judge. Repeatable; at least one is required, because a
    /// ratchet run over no evidence is the §6.20 shape — a clean result that
    /// means nothing.
    pub sarif: Vec<PathBuf>,
    /// Where the committed baseline lives. Relative paths resolve against the
    /// repository root, not the working directory, so that the same command
    /// works from a subdirectory.
    pub baseline: PathBuf,
    /// Rewrite the baseline instead of checking against it. **Default is
    /// check** — §9.14's whole proposition is that the default run is the one
    /// CI makes.
    pub update: bool,
    /// The §9.2 positive control's denominator: how many files the operator
    /// believes the analyzer should have scanned. Absent means "as many as the
    /// run itself declared it saw", which catches a tool that enumerated a
    /// universe and then scanned almost none of it but cannot catch one that
    /// under-declared the universe too.
    pub expected_targets: Option<usize>,
}

/// `judged mutants [--sut naive|refusing|vulture|command] [--json] [-- <argv>]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MutantsArgs {
    /// Which system under test to grade.
    pub sut: SutChoice,
    /// Emit the report as JSON instead of as text.
    pub json: bool,
}

/// The systems under test this build can grade.
///
/// The first two are controls we wrote ourselves, which is exactly what makes
/// them insufficient: a suite that has only ever graded its own controls bounds
/// the harness and says nothing about any real tool. The other two are how a
/// real analyzer gets in — [`SutChoice::Vulture`] by name because §4.1 already
/// has published numbers to compare against, and [`SutChoice::Command`] for
/// everything else, so that adding a tool does not require a code change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SutChoice {
    /// §7.5's heuristic, faithfully reproduced. The suite's positive control.
    Naive,
    /// Claims nothing is ever dead. The negative control.
    Refusing,
    /// Vulture, invoked at its own defaults (§4.1).
    Vulture,
    /// An arbitrary analyzer, given after `--`. Never empty.
    Command(Vec<String>),
}

/// The argv `--sut vulture` runs.
///
/// Vulture's **own** defaults, deliberately: no `--min-confidence`, no
/// `--sort-by-size`, nothing tuned. §4.1's measurement (44 true positives
/// against 644 false positives across 9 repos) is a measurement of vulture as
/// people actually run it, and a score obtained by first finding a threshold
/// that suits our fixtures would not be comparable to it — or to anything.
/// Tuning is available, and it is spelled
/// `--sut command -- vulture --min-confidence 100`, which is honest about being
/// a different experiment.
///
/// No path argument: [`judged_mutants::sut::CommandSut`] appends the fixture
/// repository as the last argument and runs from inside it, so adding `.` here
/// would hand vulture the same directory twice.
const VULTURE_ARGV: &[&str] = &["vulture"];

impl SutChoice {
    /// How the report names this SUT.
    ///
    /// A `String` rather than a `&'static str` because the escape hatch's name
    /// is the command line it was given. Printing `command` there would produce
    /// reports that cannot be told apart, which for an evidence artifact is the
    /// same as producing no report.
    pub fn label(&self) -> String {
        match self {
            SutChoice::Naive => "naive".to_string(),
            SutChoice::Refusing => "refusing".to_string(),
            SutChoice::Vulture => "vulture".to_string(),
            SutChoice::Command(argv) => argv.join(" "),
        }
    }

    /// The analyzer's argv, or `None` for the two in-process controls.
    pub fn external_argv(&self) -> Option<Vec<String>> {
        match self {
            SutChoice::Naive | SutChoice::Refusing => None,
            SutChoice::Vulture => Some(VULTURE_ARGV.iter().map(|w| w.to_string()).collect()),
            SutChoice::Command(argv) => Some(argv.clone()),
        }
    }
}

/// A command line that could not be turned into an [`Invocation`].
///
/// Carries the finished message rather than a code, because every usage failure
/// exits 2 — Ruff's contract, adopted wholesale in
/// [`judged_ratchet::exit_code`]: 0 clean, 1 findings, 2 abnormal termination.
/// A malformed command line found nothing; it is not clean.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageError {
    pub message: String,
}

/// Usage text. Printed for `--help` and quoted in usage errors.
pub const USAGE: &str = "\
judged — evidence about what a repository is still using.

USAGE:
    judged ratchet --sarif <path>... [--baseline <path>] [--update]
                                     [--expected-targets <n>]
    judged mutants [--sut <sut>] [--json]
    judged mutants --sut command [--json] -- <analyzer> [args...]

RATCHET (§9.14) — baseline today's findings, fail CI only on new ones.
    --sarif <path>             A SARIF 2.1.0 log to judge. Repeatable, required.
    --baseline <path>          Committed baseline. Default: .judged/baseline.jsonl
    --update                   Rewrite the baseline from this run. Default is CHECK.
    --expected-targets <n>     Files the analyzer should have scanned (§9.2
                               positive control). Default: as many as it declared.

    Exit 0 clean; 1 new findings or baseline rot; 2 refused to judge.

MUTANTS (§10 E2) — inject 19 known-live artifacts and see what gets called dead.
    --sut <sut>                Which system under test to grade. Default: naive.
        naive                  A deliberately bad cleaner. The positive control:
                               if it ever passes, the suite is theatre.
        refusing               Calls nothing dead. The negative control.
        vulture                The real analyzer, at its own defaults (§4.1).
        command                An arbitrary analyzer, given after `--`. It runs
                               once per fixture repository, from inside it, with
                               the repository path appended as the last argument,
                               and must exit 0. Its stdout is parsed as vulture's
                               format, the only adapter that exists so far.
    --json                     Machine-readable report.

    Exit 0 only when the false-removal count is zero; 2 if the suite could not
    be run at all — including when the selected analyzer is not installed. An
    absent analyzer claims nothing dead, which scores zero false removals, so it
    is refused rather than graded.

Judged runs an analyzer to READ it. Adapters are read-only (§9.2), so a
deletion-shaped flag is refused wherever it appears — including inside the argv
after `--`, which judged would otherwise hand to a tool and let it edit the
fixture repository.

Neither subcommand writes to the working tree. There is no --fix and no flag
that deletes (§9.13 invariant 1).
";

/// Turn a command line (without `argv[0]`) into an [`Invocation`].
pub fn parse<I>(argv: I) -> Result<Invocation, UsageError>
where
    I: IntoIterator<Item = String>,
{
    let words: Vec<String> = argv.into_iter().collect();

    // The refusal sweep runs before the subcommand is even identified, so that
    // `judged mutants --fix` is refused for the same documented reason as
    // `judged ratchet --fix` rather than as an unknown flag on a subcommand
    // that happens not to have one.
    //
    // It deliberately sweeps *past* `--` as well, so a deletion flag cannot be
    // smuggled into the analyzer's own argv. §9.2's second non-SARIF clause
    // makes adapters read-only and gives the orchestrator 100% of mutations;
    // handing `some-linter --fix` to a subprocess that runs inside a fixture
    // repository would concede that. The cost is that an analyzer with an
    // innocent `--force` cannot be run without renaming judged's list, and that
    // is the right way round: the refusal explains itself, whereas a fixture
    // repository quietly edited underneath the grader does not.
    for word in &words {
        if let Some(refused) = REFUSED_FLAGS.iter().find(|flag| *flag == word) {
            return Err(refusal(refused));
        }
    }

    // Everything after the first `--` belongs to the analyzer, not to judged.
    let (head, tail): (&[String], Option<&[String]>) = match words.iter().position(|w| w == "--") {
        Some(at) => (&words[..at], Some(&words[at + 1..])),
        None => (&words[..], None),
    };

    // Help is looked for in judged's own words only. `-- some-linter --help` is
    // a question for some-linter, and answering it with judged's usage text
    // would be judged talking over the tool the user is trying to debug.
    if head.iter().any(|w| is_help(w)) {
        return Ok(Invocation::Help);
    }

    let mut rest = head.iter().map(String::as_str);
    match rest.next() {
        Some("ratchet") => {
            if tail.is_some() {
                return Err(usage(
                    "`--` is only meaningful for `judged mutants --sut command`, which runs the \
                     words after it as an analyzer. `judged ratchet` reads SARIF logs off disk \
                     and starts no subprocess."
                        .to_string(),
                ));
            }
            parse_ratchet(rest).map(Invocation::Ratchet)
        }
        Some("mutants") => parse_mutants(rest, tail).map(Invocation::Mutants),
        Some(unknown) => Err(usage(format!(
            "`{unknown}` is not a judged subcommand. There are two: `ratchet` and `mutants`."
        ))),
        None => Err(usage(
            "no subcommand. There are two: `ratchet` and `mutants`.".to_string(),
        )),
    }
}

fn is_help(word: &str) -> bool {
    matches!(word, "--help" | "-h" | "help")
}

fn usage(message: String) -> UsageError {
    UsageError { message }
}

/// The §9.13 invariant-1 refusal, quoted back with the flag that provoked it.
fn refusal(flag: &str) -> UsageError {
    usage(format!(
        "`{flag}` does not exist, and its absence is deliberate. §9.13 invariant 1: there is no \
         --fix and there is no flag that deletes. Neither judged subcommand writes to the working \
         tree; `ratchet --update` rewrites the committed baseline and nothing else. Quarantine, \
         if it ever ships, is a separate verb — never a flag on an analysis command."
    ))
}

/// Take the value that follows `flag`, or explain that one was expected.
fn value<'a>(flag: &str, rest: &mut impl Iterator<Item = &'a str>) -> Result<&'a str, UsageError> {
    rest.next()
        .ok_or_else(|| usage(format!("`{flag}` expects a value.")))
}

fn parse_ratchet<'a>(mut rest: impl Iterator<Item = &'a str>) -> Result<RatchetArgs, UsageError> {
    let mut sarif = Vec::new();
    let mut baseline = PathBuf::from(BASELINE_PATH);
    let mut update = false;
    let mut expected_targets = None;

    while let Some(word) = rest.next() {
        match word {
            "--sarif" => sarif.push(PathBuf::from(value("--sarif", &mut rest)?)),
            "--baseline" => baseline = PathBuf::from(value("--baseline", &mut rest)?),
            "--update" => update = true,
            "--expected-targets" => {
                let raw = value("--expected-targets", &mut rest)?;
                expected_targets = Some(raw.parse::<usize>().map_err(|_| {
                    usage(format!(
                        "`--expected-targets` wants a count of files, got `{raw}`."
                    ))
                })?);
            }
            other => return Err(usage(format!("`{other}` is not a `judged ratchet` flag."))),
        }
    }

    if sarif.is_empty() {
        // Not a defaultable argument. A ratchet run with no evidence would
        // compare an empty result set against the baseline and report either a
        // clean repository or wholesale rot — both of them the §6.20 shape,
        // where a tool that did nothing is indistinguishable from one that
        // found nothing.
        return Err(usage(
            "`judged ratchet` needs at least one `--sarif <path>`; a ratchet run over no \
             evidence cannot tell a clean repository from an analyzer that never started."
                .to_string(),
        ));
    }

    Ok(RatchetArgs {
        sarif,
        baseline,
        update,
        expected_targets,
    })
}

fn parse_mutants<'a>(
    mut rest: impl Iterator<Item = &'a str>,
    tail: Option<&[String]>,
) -> Result<MutantsArgs, UsageError> {
    // The default is the positive control, not the negative one. A bare
    // `judged mutants` that exited 0 because it graded a SUT which claims
    // nothing would be a green result nobody earned.
    let mut sut = "naive";
    let mut json = false;

    while let Some(word) = rest.next() {
        match word {
            "--sut" => sut = value("--sut", &mut rest)?,
            "--json" => json = true,
            other => return Err(usage(format!("`{other}` is not a `judged mutants` flag."))),
        }
    }

    let sut = match sut {
        "naive" => in_process(SutChoice::Naive, tail)?,
        "refusing" => in_process(SutChoice::Refusing, tail)?,
        "vulture" => in_process(SutChoice::Vulture, tail)?,
        "command" => SutChoice::Command(analyzer_argv(tail)?),
        other => {
            return Err(usage(format!(
                "`--sut` accepts `naive`, `refusing`, `vulture` or `command`, got `{other}`. \
                 `naive` and `refusing` are the two controls that ship with the suite; `vulture` \
                 runs the installed vulture at its own defaults; `command` runs whatever argv \
                 follows `--`."
            )))
        }
    };

    Ok(MutantsArgs { sut, json })
}

/// A SUT that takes no command line of its own, having checked it was not given
/// one anyway.
///
/// Ignoring a stray `-- some-linter` would run the naive control while the
/// operator believed they were grading some-linter, and the report would say
/// `naive` in a line nobody reads twice.
fn in_process(choice: SutChoice, tail: Option<&[String]>) -> Result<SutChoice, UsageError> {
    match tail {
        None => Ok(choice),
        Some(_) => Err(usage(format!(
            "`--sut {}` runs no external program, so the argv after `--` would be silently \
             dropped. Use `--sut command -- <analyzer> [args...]` to grade that program instead.",
            choice.label()
        ))),
    }
}

/// The analyzer's argv, which must exist and must not be empty.
fn analyzer_argv(tail: Option<&[String]>) -> Result<Vec<String>, UsageError> {
    match tail {
        Some(argv) if !argv.is_empty() => Ok(argv.to_vec()),
        // Both spellings of the same mistake, and neither may degrade into a
        // run. An empty analyzer command line executes nothing, and a SUT that
        // executed nothing claims nothing dead — which the gate would read as a
        // perfect score.
        _ => Err(usage(
            "`--sut command` needs the analyzer's command line after `--`, e.g. \
             `judged mutants --sut command -- vulture --min-confidence 100`. It is run once per \
             fixture repository, from inside that repository, with the repository's path appended \
             as the last argument, and its output is read — never acted on."
                .to_string(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_words(line: &str) -> Result<Invocation, UsageError> {
        parse(line.split_whitespace().map(str::to_string))
    }

    fn ratchet_args(line: &str) -> RatchetArgs {
        match parse_words(line) {
            Ok(Invocation::Ratchet(args)) => args,
            other => panic!("`{line}` should parse as a ratchet invocation, got {other:?}"),
        }
    }

    fn mutants_args(line: &str) -> MutantsArgs {
        match parse_words(line) {
            Ok(Invocation::Mutants(args)) => args,
            other => panic!("`{line}` should parse as a mutants invocation, got {other:?}"),
        }
    }

    fn usage_error(line: &str) -> String {
        match parse_words(line) {
            Err(error) => error.message,
            other => panic!("`{line}` should be a usage error, got {other:?}"),
        }
    }

    #[test]
    fn ratchet_defaults_to_checking_the_committed_baseline() {
        let args = ratchet_args("ratchet --sarif knip.sarif");

        assert_eq!(args.sarif, vec![PathBuf::from("knip.sarif")]);
        assert_eq!(args.baseline, PathBuf::from(BASELINE_PATH));
        assert!(
            !args.update,
            "the default must be CHECK; §9.14's proposition is that the default \
             run is the one CI makes"
        );
        assert_eq!(args.expected_targets, None);
    }

    #[test]
    fn sarif_is_repeatable_and_keeps_the_order_it_was_given() {
        let args = ratchet_args("ratchet --sarif knip.sarif --sarif vulture.sarif");

        assert_eq!(
            args.sarif,
            vec![PathBuf::from("knip.sarif"), PathBuf::from("vulture.sarif")]
        );
    }

    #[test]
    fn ratchet_without_any_sarif_is_a_usage_error() {
        // A ratchet run over no evidence would compare an empty result set
        // against the baseline and report every entry as rot, or — with an empty
        // baseline — report a clean repository. Both are the §6.20 shape.
        let message = usage_error("ratchet");

        assert!(message.contains("--sarif"), "got {message}");
    }

    #[test]
    fn ratchet_accepts_an_explicit_baseline_and_update() {
        let args = ratchet_args("ratchet --sarif a.sarif --baseline custom/base.jsonl --update");

        assert_eq!(args.baseline, PathBuf::from("custom/base.jsonl"));
        assert!(args.update);
    }

    #[test]
    fn expected_targets_must_be_a_number() {
        let args = ratchet_args("ratchet --sarif a.sarif --expected-targets 1203");
        assert_eq!(args.expected_targets, Some(1203));

        let message = usage_error("ratchet --sarif a.sarif --expected-targets lots");
        assert!(message.contains("--expected-targets"), "got {message}");
    }

    #[test]
    fn mutants_defaults_to_the_positive_control() {
        // Deliberately not `refusing`. A bare `judged mutants` that exits 0 by
        // grading a system under test which claims nothing would be a green
        // result nobody earned — §6.20 in miniature, inside our own tool.
        let args = mutants_args("mutants");

        assert_eq!(args.sut, SutChoice::Naive);
        assert!(!args.json);
    }

    #[test]
    fn mutants_accepts_both_controls_and_json() {
        assert_eq!(
            mutants_args("mutants --sut refusing").sut,
            SutChoice::Refusing
        );
        assert_eq!(mutants_args("mutants --sut naive").sut, SutChoice::Naive);
        assert!(mutants_args("mutants --json").json);

        let message = usage_error("mutants --sut knip");
        assert!(message.contains("naive"), "got {message}");
        assert!(message.contains("refusing"), "got {message}");
        assert!(message.contains("vulture"), "got {message}");
        assert!(message.contains("command"), "got {message}");
    }

    #[test]
    fn vulture_runs_vulture_at_its_own_defaults() {
        // Not `--min-confidence 0`, and not `100`. §4.1's published numbers are
        // for vulture as shipped, and a score obtained after picking a
        // threshold that suits our own fixtures is comparable to nothing.
        let args = mutants_args("mutants --sut vulture");

        assert_eq!(args.sut, SutChoice::Vulture);
        // No path argument: `CommandSut` appends the fixture repository itself.
        assert_eq!(args.sut.external_argv(), Some(vec!["vulture".to_string()]));
    }

    #[test]
    fn the_two_controls_start_no_subprocess() {
        // The distinction the installed-analyzer preflight branches on. If
        // either control reported an external program, `judged mutants` would
        // start refusing to run its own controls the moment that name was not
        // on PATH.
        assert_eq!(SutChoice::Naive.external_argv(), None);
        assert_eq!(SutChoice::Refusing.external_argv(), None);
    }

    #[test]
    fn the_escape_hatch_takes_everything_after_the_double_dash() {
        let args = mutants_args("mutants --sut command -- vulture --min-confidence 100");

        assert_eq!(
            args.sut,
            SutChoice::Command(
                ["vulture", "--min-confidence", "100"]
                    .iter()
                    .map(|w| w.to_string())
                    .collect()
            )
        );
        // The report has to be able to tell two escape-hatch runs apart, so the
        // label is the command line rather than the word `command`.
        assert_eq!(args.sut.label(), "vulture --min-confidence 100");
    }

    #[test]
    fn judged_flags_after_the_double_dash_belong_to_the_analyzer() {
        // `--json` here is the analyzer's flag, not judged's. Consuming it
        // would change judged's output because of a word aimed at another
        // program.
        let args = mutants_args("mutants --sut command -- some-linter --json");

        assert!(!args.json, "`--json` after `--` is not judged's flag");
        assert_eq!(
            args.sut,
            SutChoice::Command(vec!["some-linter".to_string(), "--json".to_string()])
        );
    }

    #[test]
    fn an_analyzers_own_help_is_not_answered_with_judged_usage() {
        assert!(matches!(
            parse_words("mutants --sut command -- some-linter --help"),
            Ok(Invocation::Mutants(_))
        ));
        // judged's own `--help` still wins when it is judged's word.
        assert_eq!(
            parse_words("mutants --help --sut command -- some-linter"),
            Ok(Invocation::Help)
        );
    }

    #[test]
    fn an_empty_analyzer_command_line_is_refused_in_both_spellings() {
        // A `Command` SUT with no program would execute nothing, and a SUT that
        // executed nothing claims nothing dead — a false-removal count of zero,
        // which is a passing gate.
        for line in ["mutants --sut command", "mutants --sut command --"] {
            let message = usage_error(line);
            assert!(
                message.contains("--"),
                "`{line}` must say where the command goes; got {message}"
            );
        }
    }

    #[test]
    fn an_argv_given_to_a_sut_that_cannot_use_it_is_an_error_not_a_shrug() {
        for control in ["naive", "refusing", "vulture"] {
            let message = usage_error(&format!("mutants --sut {control} -- some-linter"));
            assert!(
                message.contains("--sut command"),
                "`--sut {control} -- some-linter` must point at the flag that would \
                 have run some-linter, not silently grade {control}; got {message}"
            );
        }

        let message = usage_error("ratchet --sarif a.sarif -- some-linter");
        assert!(message.contains("ratchet"), "got {message}");
    }

    #[test]
    fn a_deletion_flag_inside_an_analyzer_argv_is_still_refused() {
        // §9.2: adapters are read-only and the orchestrator owns 100% of
        // mutations. An analyzer handed its own `--fix` would edit the fixture
        // repository, and the only thing E2 guarantees about that repository is
        // that the sole change to it is the mutant.
        for flag in REFUSED_FLAGS {
            let message = usage_error(&format!("mutants --sut command -- some-linter {flag}"));
            assert!(
                message.contains(flag) && message.contains("§9.13"),
                "refusing {flag} after `--` must quote it and cite the invariant; got {message}"
            );
        }
    }

    #[test]
    fn there_is_no_flag_that_deletes() {
        // §9.13 invariant 1, as executable behaviour. Every spelling a future
        // contributor might reach for has to come back with the reason it does
        // not exist, not with "unrecognized argument" — which reads as an
        // oversight somebody should fix.
        for flag in REFUSED_FLAGS {
            let message = usage_error(&format!("ratchet --sarif a.sarif {flag}"));
            assert!(
                message.contains(flag),
                "refusing {flag} must quote it back; got {message}"
            );
            assert!(
                message.contains("§9.13"),
                "refusing {flag} must cite the invariant it is enforcing; got {message}"
            );
        }
    }

    #[test]
    fn a_deletion_flag_is_refused_even_on_a_subcommand_that_never_had_one() {
        let message = usage_error("mutants --fix");

        assert!(message.contains("--fix"), "got {message}");
        assert!(message.contains("§9.13"), "got {message}");
    }

    #[test]
    fn there_are_exactly_two_subcommands() {
        assert!(matches!(
            parse_words("ratchet --sarif a.sarif"),
            Ok(Invocation::Ratchet(_))
        ));
        assert!(matches!(parse_words("mutants"), Ok(Invocation::Mutants(_))));

        for absent in ["clean", "reap", "e2", "quarantine", "why-alive"] {
            let message = usage_error(absent);
            assert!(
                message.contains("ratchet") && message.contains("mutants"),
                "an unknown subcommand must name the two that exist; got {message}"
            );
        }
    }

    #[test]
    fn help_is_available_with_and_without_a_subcommand() {
        assert_eq!(parse_words("--help"), Ok(Invocation::Help));
        assert_eq!(parse_words("-h"), Ok(Invocation::Help));
        assert_eq!(parse_words("help"), Ok(Invocation::Help));
        assert_eq!(parse_words("ratchet --help"), Ok(Invocation::Help));
        assert_eq!(parse_words("mutants --help"), Ok(Invocation::Help));
    }

    #[test]
    fn an_empty_command_line_is_a_usage_error_not_a_default_run() {
        // Running the suite, or the ratchet, because someone typed the bare
        // binary name would make `judged` do work nobody asked for.
        let message = usage_error("");

        assert!(message.contains("ratchet"), "got {message}");
    }

    #[test]
    fn a_flag_that_needs_a_value_says_so_when_it_does_not_get_one() {
        for line in [
            "ratchet --sarif",
            "ratchet --sarif a.sarif --baseline",
            "ratchet --sarif a.sarif --expected-targets",
            "mutants --sut",
        ] {
            let message = usage_error(line);
            assert!(
                message.contains("expects a value"),
                "`{line}` should name the missing value; got {message}"
            );
        }
    }
}
