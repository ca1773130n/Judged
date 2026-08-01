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

use judged_core::veto::literal::NeedleStrategy;
use judged_mutants::sut::DEFAULT_NEEDLES;
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

/// `judged mutants [--sut naive|refusing|vulture|knip|deadcode|shear|command]
/// [--veto] [--json] [-- <argv>]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MutantsArgs {
    /// Which system under test to grade.
    pub sut: SutChoice,
    /// Run §9.3's Gate 2 over every claim the SUT makes, and report the trade.
    ///
    /// Composable with every `--sut`, because the thing being measured is not a
    /// tool but a **combination**: §11 R1 asks whether any signal combination
    /// clears the catalogue, and §9.1's architecture is an analyzer orchestrated
    /// as a bounded accuser with a veto behind it. A bare analyzer's
    /// false-removal count is what an accuser does unprotected, which is not the
    /// question.
    pub veto: bool,
    /// Which needles Gate 2a derives from a claimed **path** (§11 R8).
    ///
    /// Defaults to `judged_mutants::sut::DEFAULT_NEEDLES`, so an unadorned
    /// `--veto` is the shipped configuration and the sweep's baseline row is the
    /// build people run.
    ///
    /// Selectable because R8 asks for a measurement of this axis rather than an
    /// argument about it, and a swept number is evidence only if a reader can
    /// re-derive it. `VetoedSut::with_needles` existed from the start and no
    /// command line reached it, which made the first published sweep
    /// unreproducible from the repository that published it — for an evaluation
    /// the same as not having run it.
    ///
    /// It reaches path claims only. A **symbol** claim is judged by
    /// `VetoedSut::symbol_needles`, a fixed strategy, so this flag moves the
    /// path-claim rows of a sweep and leaves the symbol-claim rows exactly where
    /// they were. That is a limit of the axis, and stating it is the difference
    /// between a sweep and a table.
    pub needles: NeedleStrategy,
    /// Emit the report as JSON instead of as text.
    pub json: bool,
}

/// The needle strategies `--needles` accepts, spelled the way the report spells
/// them back.
///
/// The spellings are `mutants_cmd::describe_needles`'s output rather than short
/// nicknames, and that round trip is the point: the string in a swept table's
/// row and the string on the command line that produced it are the same string,
/// so a reader can check a published number against the run that made it
/// without a translation table nobody maintains.
const NEEDLE_STRATEGIES: [(&str, NeedleStrategy); 4] = [
    ("basename", NeedleStrategy::BASENAME_ONLY),
    ("basename+stem", NeedleStrategy::WITH_STEM),
    ("basename+stem+parent-dir", NeedleStrategy::WITH_PARENT_DIR),
    ("basename+stem+parent-dir+symbol", NeedleStrategy::MAXIMAL),
];

/// How [`DEFAULT_NEEDLES`] is spelled on the command line.
///
/// Looked up rather than written down a second time. A hard-coded string here
/// would be a copy of the default that nothing keeps in step with it, and the
/// first person to move `DEFAULT_NEEDLES` would ship a usage message naming the
/// old one. The `expect` is the loud version of the same guard: a default that
/// cannot be asked for by name is a configuration nobody can reproduce, which is
/// the whole failure this flag exists to fix.
fn default_needles_spelling() -> &'static str {
    NEEDLE_STRATEGIES
        .iter()
        .find(|(_, strategy)| *strategy == DEFAULT_NEEDLES)
        .map(|(spelling, _)| *spelling)
        .expect("DEFAULT_NEEDLES must be one of the strategies `--needles` accepts")
}

/// The systems under test this build can grade.
///
/// The first two are controls we wrote ourselves, which is exactly what makes
/// them insufficient: a suite that has only ever graded its own controls bounds
/// the harness and says nothing about any real tool. The rest are how a real
/// analyzer gets in — four by name because §4.1 already has published claims to
/// compare against, and [`SutChoice::Command`] for everything else, so that
/// adding a tool does not require a code change.
///
/// The four named analyzers were chosen to span the catalogue rather than to
/// flatter it. Vulture reads Python, knip reads JavaScript and TypeScript,
/// deadcode reads Go and cargo-shear reads Rust; between them they open every
/// ecosystem §10 E2 injects into. §11 R1 makes "does an auto-act tier exist at
/// all" the highest-risk open question in the design and names E2 as what
/// answers it, and an answer obtained from one Python tool is an answer about
/// Python.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SutChoice {
    /// §7.5's heuristic, faithfully reproduced. The suite's positive control.
    Naive,
    /// Claims nothing is ever dead. The negative control.
    Refusing,
    /// Vulture, invoked at its own defaults (§4.1).
    Vulture,
    /// Knip, reporting SARIF (§4.1, §9.2).
    Knip,
    /// `golang.org/x/tools/cmd/deadcode`, reporting JSON (§2.1, §4.1).
    Deadcode,
    /// cargo-shear, reporting JSON (§4.1).
    Shear,
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

/// The argv `--sut knip` runs.
///
/// Three things here are decisions rather than defaults.
///
/// **`--reporter sarif`.** Knip's default reporter is `symbols`, a
/// human-oriented table with no stable grammar; the SARIF reporter is the one
/// [`judged_mutants::adapters::knip`] parses, and it is the interchange format
/// §9.2 already standardises on. Pinned against
/// [`judged_mutants::adapters::knip::RECOMMENDED_ARGS`] by a test below, so the
/// two cannot drift apart.
///
/// **`--directory` last, with no value.** [`judged_mutants::sut::CommandSut`]
/// appends the canonicalized fixture repository as the final argument, and knip
/// — unlike vulture — takes no positional argument at all. Measured, knip
/// 6.31.0: a trailing path produces `Unexpected argument '<path>'. This command
/// does not take positional arguments` and exit 1, which is *also* knip's
/// "issues found" code. Ending the argv with the flag makes the appended path
/// the value of `-D/--directory`, which is knip's own spelling for the
/// directory to run in.
///
/// **`npx --yes`.** Knip is distributed on npm and has no system package; `npx`
/// is how it is run. `--yes` because [`judged_mutants::sut::CommandSut`] gives
/// the child no stdin, so an install confirmation prompt would be answered by
/// EOF. The version is pinned — `knip@6` — because an analyzer whose version
/// floats produces a score that cannot be compared with itself a month later.
const KNIP_ARGV: &[&str] = &[
    "npx",
    "--yes",
    "knip@6",
    "--reporter",
    "sarif",
    "--no-progress",
    "--directory",
];

/// The argv `--sut deadcode` runs, and the one place a shell is involved.
///
/// deadcode takes **Go package patterns**, not a directory. A directory names
/// exactly the package in it, so the repository root — which in every Go layout
/// worth testing holds `go.mod` and no `.go` files — is not a package at all.
/// Measured against x/tools, inside a materialized `m12`:
///
/// ```text
/// $ deadcode -json /path/to/m12
/// -: no Go files in /path/to/m12
/// deadcode: packages contain errors
/// $ echo $?
/// 1
/// $ deadcode -json /path/to/m12/...
/// [ ... the package array ... ]
/// $ echo $?
/// 0
/// ```
///
/// The pattern deadcode needs is therefore the appended path with `/...` after
/// it, and [`judged_mutants::sut::CommandSut`] appends the path unmodified. A
/// one-line `sh -c` is how the appended argument reaches the tool in the shape
/// the tool documents: `$1` is the repository, `$1/...` is the pattern.
///
/// The alternative — passing `./...` and letting the appended root ride along
/// as a second pattern — was measured and rejected: it is the failing case
/// above, so `--sut deadcode` would have failed on every class including the
/// only Go one.
///
/// `exec` so the shell is replaced rather than kept as a parent, which keeps
/// the exit code deadcode's own rather than a shell's rendering of it. It also
/// means [`SutChoice::probe_program`] has to name `deadcode` rather than
/// `argv[0]`; see there.
const DEADCODE_ARGV: &[&str] = &["sh", "-c", r#"exec deadcode -json "$1/...""#, "deadcode"];

/// The argv `--sut shear` runs.
///
/// `--format json` for the same reason knip gets `--reporter sarif`: the human
/// format has no grammar to parse. No `--offline`, no `--locked`, no
/// `--deny-warnings` — cargo-shear as people run it, so the result is
/// comparable to §4.1's account of it rather than to a configuration we chose.
///
/// No path argument: cargo-shear takes the project directory as its single
/// positional `PATH`, which is exactly what
/// [`judged_mutants::sut::CommandSut`] appends. Of the three tools added here
/// it is the only one whose CLI already matches the harness's calling
/// convention.
///
/// `--fix` is what this must never be, and it is in [`REFUSED_FLAGS`] so that
/// nobody can add it from the command line either.
const SHEAR_ARGV: &[&str] = &["cargo-shear", "--format", "json"];

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
            SutChoice::Knip => "knip".to_string(),
            SutChoice::Deadcode => "deadcode".to_string(),
            // The tool's own name, not the flag's. `--sut shear` is a short
            // spelling; a report that said `shear` would name a program nobody
            // can install.
            SutChoice::Shear => "cargo-shear".to_string(),
            SutChoice::Command(argv) => argv.join(" "),
        }
    }

    /// The analyzer's argv, or `None` for the two in-process controls.
    pub fn external_argv(&self) -> Option<Vec<String>> {
        let fixed = match self {
            SutChoice::Naive | SutChoice::Refusing => return None,
            SutChoice::Vulture => VULTURE_ARGV,
            SutChoice::Knip => KNIP_ARGV,
            SutChoice::Deadcode => DEADCODE_ARGV,
            SutChoice::Shear => SHEAR_ARGV,
            SutChoice::Command(argv) => return Some(argv.clone()),
        };
        Some(fixed.iter().map(|word| (*word).to_string()).collect())
    }

    /// The binary whose absence means this SUT cannot run at all.
    ///
    /// Almost always `argv[0]`, and deliberately not *defined* as `argv[0]`.
    /// [`DEADCODE_ARGV`] runs `sh`, which is on every machine this builds on,
    /// so a preflight that looked at `argv[0]` would find a shell, declare the
    /// analyzer present, and hand the suite a run in which `deadcode` was never
    /// found. That run exits non-zero and is refused later — but by then the
    /// message names `sh`, and the operator is told a shell is missing.
    ///
    /// The preflight is the one guard between "this analyzer is not installed"
    /// and a false-removal count of zero, so it points at the analyzer.
    pub fn probe_program(&self) -> Option<String> {
        match self {
            // The wrapper's payload. `sh` is not what has to be installed.
            SutChoice::Deadcode => Some("deadcode".to_string()),
            // `expect` rather than a fallible lookup on purpose. An empty argv
            // here would make this `None`, which the preflight reads as "an
            // in-process control, nothing to check" — and skipping the
            // preflight is the one failure this whole path exists to prevent. A
            // panic on an impossible input is louder and safer than a silent
            // skip (AGENTS.md rule 12).
            other => other.external_argv().map(|argv| {
                argv.first()
                    .expect("an external SUT's argv is non-empty by construction")
                    .clone()
            }),
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
    judged mutants [--sut <sut>] [--veto [--needles <strategy>]] [--json]
    judged mutants --sut command [--veto] [--json] -- <analyzer> [args...]

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
        vulture                Python. The real analyzer, at its own defaults
                               (§4.1). Needs `vulture` on PATH.
        knip                   JavaScript/TypeScript, via `npx --yes knip@6`,
                               reporting SARIF. Needs `npx` on PATH; npx fetches
                               knip itself on first use.
        deadcode               Go, via `deadcode -json <repo>/...`. Needs
                               `deadcode` on PATH and a working Go toolchain.
        shear                  Rust, via `cargo-shear --format json <repo>`.
                               Needs `cargo-shear` on PATH.
        command                An arbitrary analyzer, given after `--`. It runs
                               once per fixture repository, from inside it, with
                               the repository path appended as the last argument,
                               and must exit 0. Its stdout is parsed as vulture's
                               format, the only format the escape hatch reads.
    --veto                     Run §9.3's Gate 2 over every claim the SUT makes,
                               and report the trade. Composable with every --sut.
                               A veto can only RESCUE, never nominate, so this can
                               only ever shrink a claim set. The suite is run
                               TWICE — once bare, once gated — because the number
                               that matters is the difference: how many false
                               removals were prevented, and how many decoys were
                               lost paying for them. Both are printed.
    --needles <strategy>       Which needles Gate 2a derives from a claimed PATH
                               (§11 R8). Requires --veto; refused without it,
                               because there would be no gate to configure.
                               Default: basename+stem. One of:
        basename               The file's own name. The floor: Gate 2a cannot be
                               narrowed past it.
        basename+stem          ...plus the name without its extension. SHIPPED.
        basename+stem+parent-dir
                               ...plus the containing directory's name. §11 R8
                               expects this one to dominate the flag rate: it
                               fires on `src`, `app` and `dist`, and such a hit
                               reads in the report exactly like a real reference.
        basename+stem+parent-dir+symbol
                               ...plus the claimed symbol's own name.
                               Each spelling is what the report prints back in
                               its `needles` field, so a swept number and the
                               command that produced it name one configuration.
                               A SYMBOL claim is judged by a fixed strategy this
                               flag does not reach.
    --json                     Machine-readable report. Under --veto it also
                               carries the conflict list: for every blocked claim,
                               which needle fired and in which file (§9.13, §7.3).

    Exit 0 only when the false-removal count is zero; 2 if the suite could not
    be run at all — including when the selected analyzer is not installed. An
    absent analyzer claims nothing dead, which scores zero false removals, so it
    is refused rather than graded.

    The four named analyzers each read one ecosystem. Classes outside it are
    marked [NOT READ by this SUT] and subtracted from the denominator, because a
    tool that opened no file in a language has not passed there — it was absent.

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
    // Off by default, so that the number the suite has always reported keeps
    // meaning what it meant: what the analyzer does unprotected.
    let mut veto = false;
    let mut needles: Option<&str> = None;

    while let Some(word) = rest.next() {
        match word {
            "--sut" => sut = value("--sut", &mut rest)?,
            "--json" => json = true,
            "--veto" => veto = true,
            "--needles" => needles = Some(value("--needles", &mut rest)?),
            other => return Err(usage(format!("`{other}` is not a `judged mutants` flag."))),
        }
    }

    // Refused rather than ignored. `--needles` configures a gate; without
    // `--veto` there is no gate, so accepting it would let somebody publish a
    // sweep in which every row was the same ungated run — a table of identical
    // numbers, produced by command lines that each looked like they had asked
    // for something different.
    if needles.is_some() && !veto {
        return Err(usage(
            "`--needles` configures Gate 2a, which only runs under `--veto`. Without it there is \
             no gate to configure and the flag would change nothing about the run — so it is \
             refused rather than accepted and ignored."
                .to_string(),
        ));
    }
    let needles = match needles {
        None => DEFAULT_NEEDLES,
        Some(given) => NEEDLE_STRATEGIES
            .iter()
            .find(|(spelling, _)| *spelling == given)
            .map(|(_, strategy)| *strategy)
            .ok_or_else(|| {
                usage(format!(
                    "`--needles` accepts {}, got `{given}`. Each spelling is what the report \
                     prints back in its `needles` field, so a swept number and the command that \
                     produced it name the same configuration. The default is `{}` \
                     (DEFAULT_NEEDLES). Widening to `parent-dir` is the §11 R8 case: the \
                     directory needle fires on names like `src`, `app` and `dist`, and a hit on \
                     one of those is not distinguishable in the report from a real reference.",
                    NEEDLE_STRATEGIES
                        .iter()
                        .map(|(spelling, _)| format!("`{spelling}`"))
                        .collect::<Vec<_>>()
                        .join(", "),
                    default_needles_spelling(),
                ))
            })?,
    };

    let sut = match sut {
        "naive" => in_process(SutChoice::Naive, tail)?,
        "refusing" => in_process(SutChoice::Refusing, tail)?,
        "vulture" => in_process(SutChoice::Vulture, tail)?,
        "knip" => in_process(SutChoice::Knip, tail)?,
        "deadcode" => in_process(SutChoice::Deadcode, tail)?,
        "shear" => in_process(SutChoice::Shear, tail)?,
        "command" => SutChoice::Command(analyzer_argv(tail)?),
        other => {
            return Err(usage(format!(
                "`--sut` accepts `naive`, `refusing`, `vulture`, `knip`, `deadcode`, `shear` or \
                 `command`, got `{other}`. `naive` and `refusing` are the two controls that ship \
                 with the suite; `vulture` (Python), `knip` (JavaScript/TypeScript), `deadcode` \
                 (Go) and `shear` (Rust) run those installed analyzers at their own defaults; \
                 `command` runs whatever argv follows `--`."
            )))
        }
    };

    Ok(MutantsArgs {
        sut,
        veto,
        needles,
        json,
    })
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
        // Off unless asked for. The false-removal count this suite has always
        // published is what an analyzer does UNPROTECTED, and turning the veto
        // on by default would silently redefine it.
        assert!(!args.veto);
    }

    #[test]
    fn veto_composes_with_every_sut() {
        // §11 R1 asks whether any signal COMBINATION clears the catalogue, so
        // the veto is not a property of one analyzer and cannot be offered for
        // only some of them.
        for sut in ["naive", "refusing", "vulture", "knip", "deadcode", "shear"] {
            let args = mutants_args(&format!("mutants --sut {sut} --veto"));
            assert!(args.veto, "--veto must compose with --sut {sut}");
        }

        let args = mutants_args("mutants --sut command --veto --json -- vulture");
        assert!(args.veto);
        assert!(args.json);
        assert_eq!(
            args.sut,
            SutChoice::Command(vec!["vulture".to_string()]),
            "--veto before `--` must not be swallowed into the analyzer's argv"
        );
    }

    #[test]
    fn the_needle_strategy_is_selectable_and_defaults_to_the_shipped_one() {
        // §11 R8 asks for a measurement of this axis rather than an argument
        // about it. `VetoedSut::with_needles` has always existed; until this
        // flag, no command line reached it, so a published sweep could not be
        // re-derived from the repository that published it.
        assert_eq!(
            mutants_args("mutants --veto").needles,
            NeedleStrategy::WITH_STEM,
            "the default must be the shipped DEFAULT_NEEDLES, so the sweep's \
             baseline row is the shipped build"
        );

        for (spelling, expected) in [
            ("basename", NeedleStrategy::BASENAME_ONLY),
            ("basename+stem", NeedleStrategy::WITH_STEM),
            ("basename+stem+parent-dir", NeedleStrategy::WITH_PARENT_DIR),
            ("basename+stem+parent-dir+symbol", NeedleStrategy::MAXIMAL),
        ] {
            assert_eq!(
                mutants_args(&format!("mutants --veto --needles {spelling}")).needles,
                expected,
                "`--needles {spelling}`"
            );
        }
    }

    #[test]
    fn a_needle_strategy_without_a_veto_is_refused_rather_than_ignored() {
        // Accepting it silently would let a sweep be published in which every
        // row was the same ungated run.
        let message = usage_error("mutants --needles basename");

        assert!(
            message.contains("--veto"),
            "the refusal has to say which flag turns the gate on; got {message}"
        );
    }

    #[test]
    fn an_unknown_needle_strategy_names_the_ones_that_exist() {
        let message = usage_error("mutants --veto --needles everything");

        for known in [
            "basename",
            "basename+stem",
            "basename+stem+parent-dir",
            "basename+stem+parent-dir+symbol",
        ] {
            assert!(
                message.contains(known),
                "the unknown-strategy message is this flag's discovery surface \
                 and must name `{known}`; got {message}"
            );
        }
    }

    #[test]
    fn mutants_accepts_both_controls_and_json() {
        assert_eq!(
            mutants_args("mutants --sut refusing").sut,
            SutChoice::Refusing
        );
        assert_eq!(mutants_args("mutants --sut naive").sut, SutChoice::Naive);
        assert!(mutants_args("mutants --json").json);

        // `periphery` rather than `knip`: knip stood in for "a tool judged has
        // heard of but cannot run" until knip became one of the options.
        let message = usage_error("mutants --sut periphery");
        for known in [
            "naive", "refusing", "vulture", "knip", "deadcode", "shear", "command",
        ] {
            assert!(
                message.contains(known),
                "the unknown-SUT message is the discovery surface for this flag \
                 and must name `{known}`; got {message}"
            );
        }
    }

    #[test]
    fn each_named_analyzer_is_reachable_by_its_own_word() {
        assert_eq!(mutants_args("mutants --sut knip").sut, SutChoice::Knip);
        assert_eq!(
            mutants_args("mutants --sut deadcode").sut,
            SutChoice::Deadcode
        );
        assert_eq!(mutants_args("mutants --sut shear").sut, SutChoice::Shear);
    }

    #[test]
    fn knip_is_run_through_npx_at_the_pinned_version_reporting_sarif() {
        // Three properties, and each one is load-bearing.
        let argv = SutChoice::Knip
            .external_argv()
            .expect("knip runs an external program");

        // 1. The reporter the adapter parses. Knip's default is a human table
        //    with no grammar; `judged_mutants::adapters::knip` reads SARIF and
        //    rejects anything else, so a run without this flag is a hard error
        //    rather than a wrong number — but it is also a wasted run.
        assert!(
            argv.windows(judged_mutants::adapters::knip::RECOMMENDED_ARGS.len())
                .any(|window| window == judged_mutants::adapters::knip::RECOMMENDED_ARGS),
            "the argv must carry the adapter's own recommended arguments \
             contiguously, or the CLI and the adapter can drift apart; got {argv:?}"
        );

        // 2. A pinned version. An analyzer that floats produces a score that
        //    cannot be compared with itself a month later, which is the one
        //    thing an evidence artifact must be able to do.
        assert!(
            argv.contains(&"knip@6".to_string()),
            "knip must be version-pinned; got {argv:?}"
        );

        // 3. `--directory` last, so that the repository path `CommandSut`
        //    appends becomes its value. Measured against knip 6.31.0: a
        //    trailing positional path is refused with `This command does not
        //    take positional arguments` and exit 1 — which is also knip's
        //    "issues found" code, so the mistake would not even look like one.
        assert_eq!(
            argv.last().map(String::as_str),
            Some("--directory"),
            "the appended repository path has to land on a flag that takes a \
             directory; got {argv:?}"
        );
    }

    #[test]
    fn deadcode_is_given_a_recursive_package_pattern_not_a_directory() {
        // deadcode takes Go package patterns. A bare directory names only the
        // package in it, and a repository root holding `go.mod` and no `.go`
        // files is not a package — measured, that is `no Go files in <dir>` and
        // exit 1. The pattern it needs is `<repo>/...`, and `CommandSut`
        // appends `<repo>` unmodified, so the argv has to place it.
        let argv = SutChoice::Deadcode
            .external_argv()
            .expect("deadcode runs an external program");

        assert!(
            argv.iter().any(|word| word.contains("$1/...")),
            "the appended repository path must reach deadcode as a recursive \
             package pattern; got {argv:?}"
        );
        // Joined rather than matched word-by-word: the flag lives inside the
        // `sh -c` script, which is one argv word.
        assert!(
            argv.join(" ").contains("deadcode -json"),
            "without -json deadcode prints its human format, which the adapter \
             rejects — and rejects with a message about the missing flag only \
             because the adapter looks for exactly this; got {argv:?}"
        );
    }

    #[test]
    fn the_binary_preflight_looks_for_the_analyzer_not_for_the_shell() {
        // The one case where `argv[0]` is the wrong thing to check. `--sut
        // deadcode` runs `sh`, which exists everywhere; a preflight satisfied by
        // that would declare deadcode installed on a machine that has never had
        // it, and the run would then be refused with a message about a missing
        // shell. The preflight is the only guard between "not installed" and a
        // false-removal count of zero, so it points at the analyzer.
        assert_eq!(
            SutChoice::Deadcode
                .external_argv()
                .map(|argv| argv[0].clone()),
            Some("sh".to_string())
        );
        assert_eq!(
            SutChoice::Deadcode.probe_program(),
            Some("deadcode".to_string())
        );

        // Everywhere else it is `argv[0]`, and must stay so.
        for choice in [SutChoice::Vulture, SutChoice::Knip, SutChoice::Shear] {
            assert_eq!(
                choice.probe_program(),
                choice.external_argv().map(|argv| argv[0].clone()),
                "{} should be probed by its own argv[0]",
                choice.label()
            );
        }
        assert_eq!(
            SutChoice::Command(vec!["mytool".to_string(), "--flag".to_string()]).probe_program(),
            Some("mytool".to_string())
        );

        // And the controls have nothing to probe, or `judged mutants` would
        // start refusing to run its own controls.
        assert_eq!(SutChoice::Naive.probe_program(), None);
        assert_eq!(SutChoice::Refusing.probe_program(), None);
    }

    #[test]
    fn a_report_names_the_analyzer_a_reader_would_have_to_install() {
        // `--sut shear` is a short flag spelling; `shear` is not a program.
        assert_eq!(SutChoice::Shear.label(), "cargo-shear");
        assert_eq!(SutChoice::Knip.label(), "knip");
        assert_eq!(SutChoice::Deadcode.label(), "deadcode");
    }

    #[test]
    fn no_named_analyzer_is_handed_a_flag_that_deletes() {
        // §9.13 invariant 1 and §9.2's read-only rule, applied to the argvs this
        // module builds rather than to the ones a user types. Every one of these
        // tools ships a destructive mode — knip has `--fix` and
        // `--allow-remove-files`, cargo-shear has `--fix` — and the argv here is
        // the one place they could be switched on without going through the
        // command line at all.
        for choice in [
            SutChoice::Vulture,
            SutChoice::Knip,
            SutChoice::Deadcode,
            SutChoice::Shear,
        ] {
            let argv = choice.external_argv().unwrap_or_default().join(" ");
            for refused in REFUSED_FLAGS {
                assert!(
                    !argv.contains(refused),
                    "`--sut {}` would run `{argv}`, which carries {refused}",
                    choice.label()
                );
            }
        }
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
        for control in ["naive", "refusing", "vulture", "knip", "deadcode", "shear"] {
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
