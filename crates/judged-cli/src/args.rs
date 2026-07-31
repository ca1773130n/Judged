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

/// `judged mutants [--sut naive|refusing] [--json]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MutantsArgs {
    /// Which system under test to grade.
    pub sut: SutChoice,
    /// Emit the report as JSON instead of as text.
    pub json: bool,
}

/// The two systems under test this build ships.
///
/// Both are controls, and neither is a cleaner. There is no third option yet
/// because there is no cleaner yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SutChoice {
    /// §7.5's heuristic, faithfully reproduced. The suite's positive control.
    Naive,
    /// Claims nothing is ever dead. The negative control.
    Refusing,
}

impl SutChoice {
    /// The spelling accepted on the command line.
    pub fn as_str(self) -> &'static str {
        match self {
            SutChoice::Naive => "naive",
            SutChoice::Refusing => "refusing",
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
    judged mutants [--sut naive|refusing] [--json]

RATCHET (§9.14) — baseline today's findings, fail CI only on new ones.
    --sarif <path>             A SARIF 2.1.0 log to judge. Repeatable, required.
    --baseline <path>          Committed baseline. Default: .judged/baseline.jsonl
    --update                   Rewrite the baseline from this run. Default is CHECK.
    --expected-targets <n>     Files the analyzer should have scanned (§9.2
                               positive control). Default: as many as it declared.

    Exit 0 clean; 1 new findings or baseline rot; 2 refused to judge.

MUTANTS (§10 E2) — inject 19 known-live artifacts and see what gets called dead.
    --sut naive|refusing       Which control to grade. Default: naive.
    --json                     Machine-readable report.

    Exit 0 only when the false-removal count is zero.

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
    for word in &words {
        if let Some(refused) = REFUSED_FLAGS.iter().find(|flag| *flag == word) {
            return Err(refusal(refused));
        }
    }

    if words.iter().any(|w| is_help(w)) {
        return Ok(Invocation::Help);
    }

    let mut rest = words.iter().map(String::as_str);
    match rest.next() {
        Some("ratchet") => parse_ratchet(rest).map(Invocation::Ratchet),
        Some("mutants") => parse_mutants(rest).map(Invocation::Mutants),
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

fn parse_mutants<'a>(mut rest: impl Iterator<Item = &'a str>) -> Result<MutantsArgs, UsageError> {
    // The default is the positive control, not the negative one. A bare
    // `judged mutants` that exited 0 because it graded a SUT which claims
    // nothing would be a green result nobody earned.
    let mut sut = SutChoice::Naive;
    let mut json = false;

    while let Some(word) = rest.next() {
        match word {
            "--sut" => {
                sut = match value("--sut", &mut rest)? {
                    "naive" => SutChoice::Naive,
                    "refusing" => SutChoice::Refusing,
                    other => {
                        return Err(usage(format!(
                            "`--sut` accepts `naive` or `refusing`, got `{other}`. Those are the \
                             two controls this build ships; there is no cleaner to grade yet."
                        )))
                    }
                }
            }
            "--json" => json = true,
            other => return Err(usage(format!("`{other}` is not a `judged mutants` flag."))),
        }
    }

    Ok(MutantsArgs { sut, json })
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
