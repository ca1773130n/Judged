//! `judged mutants` — §10 E2, the suite that decides whether an auto-act tier
//! may exist at all.
//!
//! Nineteen known-live artifacts, each reachable through exactly **one**
//! mechanism, injected into nineteen throwaway repositories. The methodology is
//! muSE / Bonett et al. (ACM TOSEM 3439802), and §10 E2 states the grading rule
//! without softening it: *any "dead" verdict on an injected artifact is a hard
//! failure* — not a tuning opportunity.
//!
//! # What the exit code means, and what it deliberately does not
//!
//! Exit 0 requires `false_removal_count == 0` and nothing else. §11 R1
//! pre-commits that if that number is not zero, the auto-act tier is **deleted
//! from the design rather than tuned**, so it is the only number the gate may
//! depend on.
//!
//! That leaves a hole, and the report is built around admitting it: a system
//! under test that claims nothing is ever dead scores a perfect zero and passes
//! the gate. It is also useless. So decoy recall — how many genuinely-dead
//! files the SUT actually found — is printed on the line below the gate, every
//! time, in both renderings. §3.7 and §9.8 require a positive control on every
//! evidence artifact; this is the suite's own, printed rather than enforced,
//! because turning it into an exit code would let a fixture author raise a
//! green by planting easier decoys.

use std::path::{Path, PathBuf};

use judged_mutants::adapters::{deadcode, knip, shear, vulture};
use judged_mutants::fixtures;
use judged_mutants::mutant::{Ecosystem, Mutant};
use judged_mutants::runner::{run_suite, MutantReport, SuiteReport};
use judged_mutants::sut::{CommandSut, NaiveSut, RefusingSut, Sut, SutVerdict};
use serde_json::{json, Value};

use crate::args::{MutantsArgs, SutChoice};

/// Exit codes that mean a vulture run finished its analysis.
///
/// Load-bearing, not decorative: [`CommandSut`] discards the stdout of a run
/// that ended on any other code, so getting this wrong in either direction is a
/// silent scoring error. Too narrow and every productive run is refused; too
/// wide and a vulture that died on a syntax error is graded as one that found
/// nothing.
///
/// Measured rather than assumed, against **vulture 2.16**:
///
/// | Condition | Exit |
/// | --- | --- |
/// | No dead code found | 0 |
/// | Dead code found | 3 |
/// | Syntax error, or a target that does not exist | 1 |
/// | Unrecognized argument | 2 |
///
/// So the productive case is 3, not 0, and 1 has to stay out even though a
/// crashed vulture prints a plausible-looking empty result.
const VULTURE_COMPLETED_EXIT_CODES: [i32; 2] = [0, 3];

/// Exit codes that mean a knip run finished its analysis.
///
/// Measured against **knip 6.31.0** (`npx --yes knip@6 --reporter sarif
/// --no-progress --directory <repo>`), on this machine, in the fixture
/// repositories this suite materializes:
///
/// | Condition | Exit | stdout |
/// | --- | --- | --- |
/// | Analysis ran, nothing unused | 0 | a SARIF log with `"results":[]` |
/// | Analysis ran, unused files or dependencies found | 1 | a SARIF log with results |
/// | Unknown option, or a positional argument | 1 | knip's usage text |
/// | No `package.json` anywhere above the directory | 2 | a one-line pointer to `--help` |
///
/// So the productive case is **1**, and 1 is also how knip reports being called
/// wrongly. Those two are told apart by the parser rather than by the code:
/// [`knip::parse`] rejects anything that is not a SARIF log, so the usage text
/// becomes a hard error instead of an empty verdict. That is the split §6.20
/// demands — *"no data" must be a distinct state from "zero executions"* — and
/// it is why 1 can be declared healthy without declaring a misconfigured run
/// clean.
///
/// 2 stays out. It is what knip returns for the fixture repositories that hold
/// no `package.json` at all, which is most of the catalogue, and it is
/// indistinguishable from the fatal errors knip also exits 2 on.
const KNIP_COMPLETED_EXIT_CODES: [i32; 2] = [0, 1];

/// Exit codes that mean a deadcode run finished its analysis.
///
/// Measured against **`golang.org/x/tools/cmd/deadcode`** with go1.26.2 on
/// darwin/arm64, using the argv [`crate::args`] declares:
///
/// | Condition | Exit | stdout |
/// | --- | --- | --- |
/// | Analysis ran, nothing dead | 0 | the four bytes `null` |
/// | Analysis ran, dead functions found | 0 | the `Package` JSON array |
/// | Target directory holds no Go files | 1 | empty |
/// | Target is not inside a Go module | 1 | empty |
/// | A package fails to parse or type-check | 1 | empty |
/// | Unknown flag, or no package pattern at all | 2 | usage text on stderr |
///
/// **Only 0 is a completed run**, and it covers both the productive and the
/// empty case — deadcode gives a caller no exit code by which to tell "I
/// analyzed your program and it is all reachable" from "I refused". The stdout
/// does: `null` versus an array. That distinction is the adapter's, not the
/// harness's.
///
/// Note what 1 collides with, and why nothing may be added to this list to make
/// the suite quieter. "This repository has no Go in it" and "your Go does not
/// compile" are the same code and the same empty stdout. Declaring 1 healthy
/// would hand [`deadcode::verdict_from_stdout`] an empty stream for a run that
/// never analyzed anything — and an empty stream parses to no claims, which is
/// zero false removals, which is a passing gate.
const DEADCODE_COMPLETED_EXIT_CODES: [i32; 1] = [0];

/// Exit codes that mean a cargo-shear run finished its analysis.
///
/// Measured against **cargo-shear** (`--format json <repo>`), built from source
/// for this round because it needs a newer rustc than this repository pins:
///
/// | Condition | Exit | stdout |
/// | --- | --- | --- |
/// | Analysis ran, nothing found | 0 | `{"summary":{"errors":0,...},"findings":[]}` |
/// | Analysis ran, warning-severity findings only (unlinked files) | 0 | the JSON document |
/// | Analysis ran, error-severity findings (unused dependencies) | 1 | the JSON document |
/// | Unknown or malformed command-line argument | 1 | empty |
/// | No `Cargo.toml`, or `cargo metadata` failed | 2 | `error: Metadata error: ...` |
///
/// Two things here are worth stating out loud, because both are the sort of
/// detail a remembered exit-code table gets wrong.
///
/// First, cargo-shear's exit code depends on the **severity** of what it found,
/// not on whether it found anything: an unlinked file is a warning and exits 0,
/// while an unused dependency is an error and exits 1. A list of `[0]` would
/// therefore discard exactly the findings §4.1 cares most about.
///
/// Second, 1 is shared with "you called me wrongly", as it is for knip, and is
/// separated the same way — by the parser. [`shear::parse_output`] rejects an
/// empty stream and rejects the plain-text `error:` line, so neither can arrive
/// as a clean verdict. 2 stays out for the same reason knip's does: it is what
/// every non-Rust fixture produces, and it is also what a broken `Cargo.toml`
/// produces.
const SHEAR_COMPLETED_EXIT_CODES: [i32; 2] = [0, 1];

/// Run the catalogue and render the result.
pub fn run(args: &MutantsArgs) -> (String, i32) {
    // Before the fixtures, before git, before anything that takes a second: an
    // analyzer that is not on this machine must stop the run here.
    //
    // This is the whole feature's failure mode, and it is not hypothetical.
    // `CommandSut::run` turns a spawn failure into `Ok(SutVerdict::default())`
    // — an empty verdict — so a missing binary would be graded as a SUT that
    // claimed nothing dead. Nineteen classes of nothing is a false-removal
    // count of zero, which is the gate's only input, which is exit 0 and
    // "GATE PASSED". A green build certifying that an absent tool is safe to
    // trust is §6.20's disarming failure exactly, and §3.7 records that this
    // shape — an artifact reporting ~0% for everything, then believed — is how
    // every catastrophic failure in this space presented.
    if let Err(refusal) = preflight(&args.sut) {
        return (render_refusal(&refusal, &args.sut, args.json), 2);
    }

    let mutants = fixtures::all();
    // Captured before the run, because grading returns ids and ecosystems but
    // not the mechanism — and the mechanism is the whole point of a failure.
    // "m08 failed" is a bug report; "m08 failed: referenced only from a CI
    // workflow step" is a design finding.
    let catalogue: Vec<(String, String, String)> = mutants
        .iter()
        .map(|m| {
            (
                m.id().to_string(),
                m.mechanism().to_string(),
                m.research_ref().to_string(),
            )
        })
        .collect();

    let sut = build_sut(&args.sut);

    let report = match run_suite(sut.as_ref(), &mutants) {
        Ok(report) => report,
        // A crashed harness is not a passing harness. §3.7: every catastrophic
        // failure in this space presented as an artifact reporting ~0% for
        // everything, which was then trusted.
        Err(error) => {
            return (
                render_refusal(
                    &Refusal {
                        headline: "the E2 suite did not complete".to_string(),
                        detail: error.to_string(),
                        remedy: foreign_ecosystem_hint(&args.sut, &mutants),
                    },
                    &args.sut,
                    args.json,
                ),
                2,
            )
        }
    };

    // The gate, and only the gate (§10 E2, §11 R1).
    let code = if report.false_removal_count == 0 {
        0
    } else {
        1
    };

    let rendered = if args.json {
        render_json(&report, &catalogue, &args.sut)
    } else {
        render_text(&report, &catalogue, &args.sut)
    };
    (rendered, code)
}

// ---------------------------------------------------------------------------
// Refusing to run, which is not the same as running and finding nothing
// ---------------------------------------------------------------------------

/// Why the suite did not produce a verdict.
///
/// A structure rather than a formatted string, because the same refusal has to
/// be rendered twice — once for a person and once for whatever reads `--json` —
/// and the JSON rendering is the one that would otherwise quietly turn a
/// refusal into a result.
struct Refusal {
    /// One line, in the log tail a human actually reads.
    headline: String,
    /// What was looked for and what was found instead.
    detail: String,
    /// What to do about it. §9.13's presentation rules are about what somebody
    /// can do next; "vulture is missing" without "install it like this" makes
    /// the reader go and find out.
    remedy: Option<String>,
}

/// Refuse to grade a SUT whose analyzer is not on this machine.
fn preflight(choice: &SutChoice) -> Result<(), Refusal> {
    // The two in-process controls start no subprocess, so there is nothing to
    // look for and nothing that can be missing.
    //
    // `probe_program` rather than `argv[0]`: for `--sut deadcode` the argv
    // begins with `sh`, and a preflight satisfied by a shell would report every
    // machine as having deadcode installed.
    let Some(program) = choice.probe_program() else {
        return Ok(());
    };

    if locate(&program).is_some() {
        return Ok(());
    }

    // Two failures that the word "missing" would blur into one. A name that is
    // not on PATH is a tool to install; a path that is not there is a typo, or
    // a build that has not run, or the wrong working directory. Reporting the
    // second as the first describes a search that never happened and sends the
    // reader to install something they already have.
    if program.contains(std::path::MAIN_SEPARATOR) {
        return Err(Refusal {
            headline: format!("there is no analyzer at `{program}`"),
            detail: format!(
                "`{program}` has a directory in it, so it was used as the path it is rather \
                 than searched for by name. Nothing is at that path, resolved against the \
                 directory judged was started in ({}).",
                std::env::current_dir()
                    .map(|d| d.display().to_string())
                    .unwrap_or_else(|_| "unknown".to_string())
            ),
            remedy: Some(format!(
                "Check the path, or build it first. A bare name — `{}` with no directory — is \
                 looked up on PATH instead.",
                Path::new(&program)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| program.clone())
            )),
        });
    }

    let searched = match std::env::var_os("PATH") {
        Some(path) => {
            let count = std::env::split_paths(&path).count();
            format!(
                "Looked for `{program}` in the {count} director{} on PATH; it is in none of them.",
                if count == 1 { "y" } else { "ies" }
            )
        }
        None => format!("Looked for `{program}`, but PATH is not set at all."),
    };

    Err(Refusal {
        headline: format!("the analyzer `{program}` is not installed"),
        detail: searched,
        remedy: Some(install_hint(&program)),
    })
}

/// The one thing a reader needs when a language-specific analyzer stops the
/// suite, and cannot get from the tool's own message.
///
/// Measured this round, and worth stating because it separates the four named
/// analyzers into two kinds. Vulture *tolerates* a repository with no Python in
/// it: it finds no `.py` files, prints nothing and exits 0, so the nineteen
/// classes run to completion and the twelve it cannot read are marked
/// `[NOT READ by this SUT]` in the report. The other three refuse the directory
/// instead —
///
/// | Tool | On a repository outside its ecosystem | Exit |
/// | --- | --- | --- |
/// | knip | `ERROR: Unable to find package.json` | 2 |
/// | cargo-shear | `could not find Cargo.toml in <dir> or any parent directory` | 2 |
/// | deadcode | `no Go files in <dir>` / `cannot find main module` | 1 |
///
/// — and since [`judged_mutants::runner::run_suite`] runs every class against
/// the selected SUT and treats a SUT error as fatal, the first foreign class
/// ends the run. The suite therefore cannot currently produce a score for these
/// three, and that is reported rather than worked around.
///
/// Widening [`KNIP_COMPLETED_EXIT_CODES`] and friends to swallow those codes
/// would produce a score, and it is exactly the silent scoring error this
/// module exists to prevent: knip and cargo-shear both exit 2 for a *broken*
/// project as well as for an absent one, and deadcode's 1 covers "no Go here"
/// and "your Go does not compile" alike. Each of those states leaves stdout
/// empty or non-report, which parses to no claims, which is zero false
/// removals, which is a passing gate.
///
/// The wording is conditional on purpose. This hint is attached to *every*
/// incomplete run of a language-specific SUT, including one that failed inside
/// its own ecosystem for an unrelated reason, and asserting a cause it cannot
/// check would send the reader past a real bug.
fn foreign_ecosystem_hint(choice: &SutChoice, mutants: &[Box<dyn Mutant>]) -> Option<String> {
    let langs = reads(choice)?;
    let foreign = mutants
        .iter()
        .filter(|mutant| !was_read(choice, mutant.ecosystem()))
        .count();
    if foreign == 0 {
        return None;
    }

    let spoken: Vec<&str> = langs.iter().map(|lang| ecosystem(*lang)).collect();
    Some(format!(
        "`{}` reads {}, and {foreign} of {} classes in the catalogue are outside that. The suite \
         runs every class against the selected analyzer, so if the class named above is one of \
         those, the tool was handed a repository it cannot analyze — which is not a broken \
         install. Analyzers split on what they then do: vulture finds no Python and exits 0, so \
         its run completes and those classes are simply marked unread, while knip, cargo-shear \
         and deadcode refuse the directory and exit non-zero. Those exit codes are deliberately \
         not declared healthy, because each is shared with a genuine analysis failure whose \
         output is equally empty — accepting them would score a crashed run as a clean one \
         (§6.20). Until the runner can skip a class its analyzer declares it cannot read, grade \
         these on a catalogue they can read.",
        choice.label(),
        spoken.join(" and "),
        mutants.len(),
    ))
}

/// Where `program` is, if it is anywhere.
///
/// A name is looked up on `PATH`; anything containing a separator is taken as
/// the path it looks like, so `--sut command -- ./tools/analyze` works without
/// a flag to say so.
///
/// Existence, not executability. A file that is present but not executable
/// fails later at spawn time, and that failure is loud on its own; treating it
/// as "not installed" here would print an install command at somebody whose
/// problem is a permission bit.
fn locate(program: &str) -> Option<PathBuf> {
    let candidate = Path::new(program);
    if program.contains(std::path::MAIN_SEPARATOR) {
        return candidate.is_file().then(|| candidate.to_path_buf());
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|directory| directory.join(program))
        .find(|candidate| candidate.is_file())
}

/// How to get `program`, when we know. Only ever called with a bare name.
fn install_hint(program: &str) -> String {
    match program {
        "vulture" => "Install it with `pipx install vulture`, or `pip install vulture` into the \
                      environment judged runs in. It needs Python."
            .to_string(),
        // The missing thing is npx, not knip: `--sut knip` runs
        // `npx --yes knip@6`, and npx fetches knip itself. Saying "install
        // knip" would send the reader to `npm i -g knip`, which pins a
        // different version than the one the suite grades.
        "npx" => "`npx` ships with Node.js — install Node 20 or newer (`brew install node`, or \
                  https://nodejs.org). knip itself does not need installing: `--sut knip` runs \
                  `npx --yes knip@6`, which fetches the pinned version on first use and needs \
                  network access to do it."
            .to_string(),
        "deadcode" => "Install it with `go install golang.org/x/tools/cmd/deadcode@latest`, then \
                       put `$(go env GOPATH)/bin` on PATH — that last step is the one that is \
                       usually missing. It also needs the Go toolchain at run time, because it \
                       loads the program from source."
            .to_string(),
        "cargo-shear" => "Install it with `cargo install cargo-shear`. Judged runs the binary \
                          directly rather than as `cargo shear`, so it has to be on PATH by that \
                          name. Note that recent versions need a newer rustc than this repository \
                          pins; `cargo install --locked cargo-shear` with a toolchain of its own \
                          is the way round that."
            .to_string(),
        other => format!(
            "Install `{other}` and put it on PATH, or give its path instead: \
             `--sut command -- /path/to/{other} [args...]`."
        ),
    }
}

/// A refusal, rendered for whoever asked.
fn render_refusal(refusal: &Refusal, choice: &SutChoice, json: bool) -> String {
    // Deliberately worded to avoid the strings a gate result is made of. A log
    // scanner, or a human skimming, must not be able to find the words that
    // mean "the suite ran and cleared the bar" anywhere in a report where it
    // did neither.
    const WHY: &str = "No verdict was reached and no class was graded. This is a refusal rather \
                       than a result on purpose: an analyzer that never ran claims nothing dead, \
                       which is zero false removals, which is the number that clears the release \
                       gate. Grading it would certify a tool that was not here (§3.7, §6.20).";

    if json {
        let document = json!({
            "sut": choice.label(),
            "refused": true,
            "reason": refusal.headline,
            "detail": refusal.detail,
            "remedy": refusal.remedy,
            "why_this_is_not_a_result": WHY,
        });
        // Note what is absent: `gate_passed`, `false_removal_count`, `mutants`.
        // A consumer reaching for them gets nothing rather than a zero, because
        // a zero here and a zero from a real clean run are the same bytes.
        return match serde_json::to_string_pretty(&document) {
            Ok(text) => format!("{text}\n"),
            Err(error) => format!("{{\"refused\":true,\"reason\":\"{error}\"}}\n"),
        };
    }

    let mut out = format!(
        "REFUSED — {} (exit 2)\n\n  {}\n",
        refusal.headline, refusal.detail
    );
    if let Some(remedy) = &refusal.remedy {
        out.push_str(&format!("  {remedy}\n"));
    }
    out.push_str(&format!("\n{WHY}\n"));
    out
}

/// The SUT the report is about.
fn build_sut(choice: &SutChoice) -> Box<dyn Sut> {
    match choice {
        SutChoice::Naive => Box::new(NaiveSut),
        SutChoice::Refusing => Box::new(RefusingSut),
        SutChoice::Vulture => Box::new(
            CommandSut::new("vulture", "vulture", vulture::verdict_from_stdout)
                .with_success_exit_codes(VULTURE_COMPLETED_EXIT_CODES)
                // §9.2's other non-SARIF clause: every adapter declares the
                // finding classes it structurally cannot emit, so the
                // orchestrator knows when the tool's silence means anything.
                .with_cannot_emit([vulture::CAPABILITY_ENVELOPE]),
        ),
        // The three below share a shape with vulture and differ in one respect
        // worth naming: each takes its argv from [`SutChoice::external_argv`]
        // rather than repeating the program here, because for `deadcode` the
        // program is a shell and the analyzer's name lives in the argv.
        SutChoice::Knip => Box::new(
            external(choice, knip::parse)
                .with_success_exit_codes(KNIP_COMPLETED_EXIT_CODES)
                .with_cannot_emit([knip::CAPABILITY_ENVELOPE]),
        ),
        SutChoice::Deadcode => Box::new(
            external(choice, deadcode::verdict_from_stdout)
                .with_success_exit_codes(DEADCODE_COMPLETED_EXIT_CODES)
                .with_cannot_emit([deadcode::CAPABILITY_ENVELOPE]),
        ),
        SutChoice::Shear => Box::new(
            external(choice, shear::verdict_from_stdout)
                .with_success_exit_codes(SHEAR_COMPLETED_EXIT_CODES)
                .with_cannot_emit([shear::CAPABILITY_ENVELOPE]),
        ),
        SutChoice::Command(argv) => {
            let (program, args) = argv
                .split_first()
                .expect("argv is non-empty by construction");
            // No `with_success_exit_codes`: an arbitrary analyzer's exit codes
            // are not interpretable, so the strict default stands and a
            // non-zero exit is treated as a run that failed rather than as a
            // run that found things. Somebody who knows better can say so out
            // loud with `-- sh -c 'mytool "$@"; true' --`.
            //
            // Its stdout is parsed as vulture's format because that is the only
            // adapter that exists. The usage text says so; guessing a format
            // from the output would be the adapter being cleverer than the
            // tool, which §9.2's adapter rules forbid in both directions.
            Box::new(
                CommandSut::new(
                    choice.label(),
                    program.clone(),
                    vulture::verdict_from_stdout,
                )
                .with_args(args.to_vec()),
            )
        }
    }
}

/// A [`CommandSut`] built from a named SUT's declared argv.
///
/// One place where `external_argv()` is split into program and arguments, so
/// that a SUT whose analyzer is not `argv[0]` — `deadcode`, which is run through
/// a one-line `sh -c` because it takes package patterns rather than a directory
/// — cannot end up with a different argv here than the one
/// [`SutChoice::probe_program`] was checked against.
fn external(choice: &SutChoice, parse: fn(&str) -> judged_core::Result<SutVerdict>) -> CommandSut {
    let argv = choice
        .external_argv()
        .expect("a named external SUT declares an argv");
    let (program, args) = argv
        .split_first()
        .expect("a named external SUT's argv is non-empty by construction");
    CommandSut::new(choice.label(), program.clone(), parse).with_args(args.to_vec())
}

/// What the reader has to know about the translation before they read a number
/// produced through it.
///
/// §9.2's second non-SARIF clause requires every adapter to declare the finding
/// classes it structurally cannot emit; the vulture adapter also states which
/// half of a verdict it fills and which it leaves empty, and calls the resulting
/// count a lower bound. A score reported without that is a score somebody will
/// read as vulture's blast radius when it is the adapter's floor on it.
struct Disclosure {
    envelope: &'static str,
    mapping: &'static str,
}

/// The escape hatch's envelope, which is that there isn't one.
const UNDECLARED_ENVELOPE: &str = "\
capability envelope: NOT DECLARED. This analyzer was named on the command line, \
so nothing is known about what it structurally cannot emit, and its silence is \
therefore not evidence about anything. A low false-removal count here bounds \
this run only (§9.2).";

fn disclosure(choice: &SutChoice) -> Option<Disclosure> {
    match choice {
        // The controls are this repository's own code, described where they are
        // defined; there is no third-party translation to disclose.
        SutChoice::Naive | SutChoice::Refusing => None,
        SutChoice::Vulture => Some(Disclosure {
            envelope: vulture::CAPABILITY_ENVELOPE,
            mapping: vulture::MAPPING_DECISION,
        }),
        SutChoice::Knip => Some(Disclosure {
            envelope: knip::CAPABILITY_ENVELOPE,
            mapping: knip::MAPPING_DECISION,
        }),
        SutChoice::Deadcode => Some(Disclosure {
            envelope: deadcode::CAPABILITY_ENVELOPE,
            mapping: deadcode::MAPPING_DECISION,
        }),
        SutChoice::Shear => Some(Disclosure {
            envelope: shear::CAPABILITY_ENVELOPE,
            mapping: shear::MAPPING_DECISION,
        }),
        // Its stdout is read by the vulture adapter, so the mapping decision
        // applies verbatim; the envelope cannot.
        SutChoice::Command(_) => Some(Disclosure {
            envelope: UNDECLARED_ENVELOPE,
            mapping: vulture::MAPPING_DECISION,
        }),
    }
}

/// The ecosystems a SUT can actually read, or `None` if it reads everything.
///
/// This exists so the report can tell "ran, looked, and claimed nothing" apart
/// from "never opened a file here". They render identically otherwise — zero
/// false removals, zero decoys — and the second is not a result. §6.20 states
/// the rule in general ("no data" must be a distinct state from "zero
/// executions"); this is that rule applied to Judged's own scoreboard, which
/// would otherwise let a Python-only tool look competent at Rust.
fn reads(choice: &SutChoice) -> Option<&'static [Ecosystem]> {
    match choice {
        // Both controls are language-agnostic by construction: NaiveSut walks
        // every source extension the catalogue uses, and RefusingSut declines
        // uniformly rather than out of incapacity.
        SutChoice::Naive | SutChoice::Refusing => None,
        // Vulture is a Python AST tool. It cannot parse Rust, Go or TypeScript,
        // so on those classes it is not scoring — it is absent.
        SutChoice::Vulture => Some(&[Ecosystem::Python, Ecosystem::Polyglot]),
        // Knip resolves a JavaScript/TypeScript module graph from
        // `package.json` and `tsconfig.json`. `Polyglot` is claimed because
        // every polyglot fixture in the catalogue contains a JS or TS half —
        // that is what makes it polyglot — and knip genuinely parses it. It is
        // *not* a claim that knip understands the other half; §10 E2 grades a
        // single mechanism per class, and a class knip fails because the
        // reference lives in the non-JS half is knip failing, not knip being
        // absent.
        SutChoice::Knip => Some(&[Ecosystem::TypeScript, Ecosystem::Polyglot]),
        // deadcode loads a Go program from source and needs a Go module. Go is
        // the whole of its reach — and deliberately *not* Polyglot: unlike a
        // JS bundler-shaped tool, deadcode has no partial reading of a
        // repository whose Go half is absent. It refuses the directory
        // outright.
        SutChoice::Deadcode => Some(&[Ecosystem::Go]),
        // cargo-shear reads a cargo workspace: `Cargo.toml` plus the crate
        // sources cargo metadata points it at. Same reasoning as deadcode for
        // leaving Polyglot out — with no manifest there is nothing for
        // `cargo metadata` to answer with, so it never opens a file.
        SutChoice::Shear => Some(&[Ecosystem::Rust]),
        // An arbitrary command declares nothing, so we must not assume it read
        // anything OR that it failed to. Unknown competence is not a claim.
        SutChoice::Command(_) => None,
    }
}

/// Whether this SUT could read anything in this mutant's ecosystem.
///
/// `Polyglot` counts as readable for any SUT that reads at least one language,
/// because a polyglot fixture contains a Python half the tool genuinely does
/// parse — the grade there is earned, not vacuous.
fn was_read(choice: &SutChoice, of: Ecosystem) -> bool {
    reads(choice).is_none_or(|langs| langs.contains(&of))
}

/// The catalogue's own spelling for an ecosystem.
fn ecosystem(ecosystem: Ecosystem) -> &'static str {
    match ecosystem {
        Ecosystem::Python => "python",
        Ecosystem::TypeScript => "typescript",
        Ecosystem::Rust => "rust",
        Ecosystem::Go => "go",
        Ecosystem::Polyglot => "polyglot",
    }
}

/// Mechanism and research reference for a mutant id.
fn lookup<'a>(catalogue: &'a [(String, String, String)], id: &str) -> (&'a str, &'a str) {
    catalogue
        .iter()
        .find(|(candidate, _, _)| candidate == id)
        .map(|(_, mechanism, research)| (mechanism.as_str(), research.as_str()))
        .unwrap_or(("(mechanism not declared)", "(unreferenced)"))
}

/// Classes that removed something live, in catalogue order.
///
/// Ordered by mutant id, never by how much each one removed — §9.13 invariant 3
/// applies to this report exactly as it applies to the ratchet's.
fn failing_classes(report: &SuiteReport) -> Vec<&str> {
    report
        .reports
        .iter()
        .filter(|r| !r.false_removals.is_empty())
        .map(|r| r.mutant_id.as_str())
        .collect()
}

fn totals(report: &SuiteReport) -> (usize, usize, usize) {
    let passed = report.reports.iter().filter(|r| r.passed).count();
    let decoys_found = report.reports.iter().map(|r| r.decoys_found).sum();
    let decoys_total = report.reports.iter().map(|r| r.decoys_total).sum();
    (passed, decoys_found, decoys_total)
}

fn render_text(
    report: &SuiteReport,
    catalogue: &[(String, String, String)],
    choice: &SutChoice,
) -> String {
    let classes = report.reports.len();
    let (passed, decoys_found, decoys_total) = totals(report);

    let mut out = format!(
        "judged mutants — §10 E2, {classes} injected liveness mechanisms, SUT `{}`\n\n\
         \x20 Any \"dead\" verdict on an injected live artifact is a hard failure, not a tuning\n\
         \x20 opportunity (§10 E2). Decoys are genuinely-dead files planted beside them, so that\n\
         \x20 a tool which refuses to answer cannot score a perfect run.\n\n",
        choice.label()
    );

    // Printed above the table rather than below the summary. §9.13 budgets a
    // human ten seconds and puts the numbers that decide something in the log
    // tail, so a page of adapter prose goes where it answers "which grading am
    // I looking at" — before the rows — and not where it would push the gate
    // line off the bottom of a CI log.
    if let Some(disclosure) = disclosure(choice) {
        out.push_str(&format!(
            "{}\n\n{}\n\n",
            disclosure.envelope, disclosure.mapping
        ));
    }

    for row in &report.reports {
        let (mechanism, research) = lookup(catalogue, &row.mutant_id);
        out.push_str(&mutant_line(
            row,
            mechanism,
            was_read(choice, row.ecosystem),
        ));
        for removed in &row.false_removals {
            // Indented under its class and spelled out, because this line is
            // the finding: a live artifact the tool would have deleted, and the
            // documented incident class it came from.
            out.push_str(&format!("       removed live: {removed}   [{research}]\n"));
        }
    }

    // Summary lines are unindented: they are what a CI log tail shows, and what
    // a human reads in the ten seconds §9.13 budgets.
    out.push_str(&format!(
        "\n{classes} classes: {passed} passed, {} failed\n",
        classes - passed
    ));
    // Stated as its own line rather than a footnote, because it is the single
    // number most likely to be misread out of this report. A Python-only tool
    // scored against 19 classes has genuinely been measured on far fewer, and
    // a summary that does not say so invites "vulture only broke 4 of 19".
    let unread = report
        .reports
        .iter()
        .filter(|row| !was_read(choice, row.ecosystem))
        .count();
    if unread > 0 {
        out.push_str(&format!(
            "not measured: {unread} of {classes} classes are outside this SUT's languages — \
             it opened no file in them, so neither its passes nor its failures there are results\n"
        ));
    }
    out.push_str(&format!(
        "decoy recall: {decoys_found} of {decoys_total} genuinely-dead files found\n"
    ));
    out.push_str(&format!(
        "false removals: {} — {}\n",
        report.false_removal_count,
        if report.false_removal_count == 0 {
            "GATE PASSED (§10 E2 gates releases on this number, and on nothing else)"
        } else {
            "GATE FAILED (§11 R1: if this is not zero, the auto-act tier is deleted \
             from the design rather than tuned)"
        }
    ));

    let failing = failing_classes(report);
    if failing.is_empty() {
        if decoys_found == 0 {
            out.push_str(
                "note: this SUT removed nothing at all, so it cleared the gate without \
                 demonstrating it can find anything. Zero false removals is also the score of a \
                 tool that refuses to answer.\n",
            );
        }
    } else {
        out.push_str(&format!(
            "classes with false removals: {}\n",
            failing.join(", ")
        ));
    }
    out
}

/// One row of the table: id, verdict, ecosystem, the two counts, the mechanism.
fn mutant_line(row: &MutantReport, mechanism: &str, was_read: bool) -> String {
    format!(
        "  {id}  {verdict:4}  {ecosystem:10}  {false_removals} false  {found}/{total} decoys  {mechanism}{note}\n",
        id = row.mutant_id,
        verdict = if row.passed { "pass" } else { "FAIL" },
        ecosystem = ecosystem(row.ecosystem),
        false_removals = row.false_removals.len(),
        found = row.decoys_found,
        total = row.decoys_total,
        // Not a verdict column, deliberately: the gate arithmetic is unchanged
        // and a false removal here would still be a false removal. This only
        // stops a reader crediting the tool for a class it never opened.
        note = if was_read { "" } else { "  [NOT READ by this SUT]" },
    )
}

/// The same report, for something that is not a person.
///
/// Emitted as the whole of stdout so it can be piped straight into `jq`. Keys
/// are snake_case rather than the SARIF-style camelCase used on the wire
/// elsewhere, because this is Judged's own report about its own suite and not
/// an interchange format anyone else defines.
fn render_json(
    report: &SuiteReport,
    catalogue: &[(String, String, String)],
    choice: &SutChoice,
) -> String {
    let (passed, decoys_found, decoys_total) = totals(report);

    let mutants: Vec<Value> = report
        .reports
        .iter()
        .map(|row| {
            let (mechanism, research) = lookup(catalogue, &row.mutant_id);
            json!({
                "id": row.mutant_id,
                "ecosystem": ecosystem(row.ecosystem),
                "mechanism": mechanism,
                "research_ref": research,
                "passed": row.passed,
                "false_removals": row.false_removals,
                "decoys_found": row.decoys_found,
                "decoys_total": row.decoys_total,
            })
        })
        .collect();

    let document = json!({
        "sut": choice.label(),
        // Absent for the two in-process controls, present for anything that
        // went through an adapter. A consumer that records `false_removal_count`
        // without it has recorded a number stripped of what bounds it.
        "adapter": disclosure(choice).map(|d| json!({
            "capability_envelope": d.envelope,
            "mapping_decision": d.mapping,
        })),
        "classes": report.reports.len(),
        "passed_classes": passed,
        "false_removal_count": report.false_removal_count,
        "gate_passed": report.false_removal_count == 0,
        "decoys_found": decoys_found,
        "decoys_total": decoys_total,
        "classes_with_false_removals": failing_classes(report),
        "mutants": mutants,
    });

    match serde_json::to_string_pretty(&document) {
        Ok(text) => format!("{text}\n"),
        // Unreachable for a document built from owned strings and integers, and
        // reported rather than unwrapped so that an impossible failure is still
        // a message instead of a panic (AGENTS.md rule 12).
        Err(error) => format!("{{\"error\":\"could not serialize the E2 report: {error}\"}}\n"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalogue() -> Vec<(String, String, String)> {
        vec![(
            "m01".to_string(),
            "a dotted path in a settings string".to_string(),
            "§10 E2 class 1".to_string(),
        )]
    }

    fn suite(false_removals: Vec<String>, decoys_found: usize, decoys_total: usize) -> SuiteReport {
        let count = false_removals.len();
        SuiteReport {
            sut_name: "test".to_string(),
            reports: vec![MutantReport {
                mutant_id: "m01".to_string(),
                ecosystem: Ecosystem::Python,
                passed: count == 0 && decoys_found == decoys_total,
                false_removals,
                decoys_found,
                decoys_total,
            }],
            false_removal_count: count,
        }
    }

    /// A one-row suite in an ecosystem the caller chooses, for the not-read tests.
    fn suite_in(ecosystem: Ecosystem) -> SuiteReport {
        let mut report = suite(Vec::new(), 0, 1);
        report.reports[0].ecosystem = ecosystem;
        report
    }

    #[test]
    fn a_class_the_sut_cannot_read_is_marked_and_counted_out() {
        // Vulture is a Python AST tool. Graded against a Rust fixture it opens
        // no file, claims nothing, and renders — without this marker —
        // identically to a tool that read the code and correctly kept it.
        // §6.20: "no data" must be a distinct state from "zero executions".
        let text = render_text(
            &suite_in(Ecosystem::Rust),
            &catalogue(),
            &SutChoice::Vulture,
        );

        assert!(
            text.contains("[NOT READ by this SUT]"),
            "a Rust class graded by vulture must be marked unread; got {text}"
        );
        assert!(
            text.contains("not measured: 1 of 1 classes"),
            "the summary must carry the denominator, or '4 of 19' is the reading people take; \
             got {text}"
        );
    }

    #[test]
    fn a_class_the_sut_can_read_carries_no_such_marker() {
        // The other half, and the one that keeps the marker meaningful: if it
        // appeared on rows the tool genuinely analyzed, it would stop carrying
        // information and start being noise a reader learns to skip.
        let python = render_text(
            &suite_in(Ecosystem::Python),
            &catalogue(),
            &SutChoice::Vulture,
        );
        assert!(!python.contains("NOT READ"), "got {python}");
        assert!(!python.contains("not measured"), "got {python}");

        // Polyglot fixtures contain a Python half vulture really does parse, so
        // a grade there is earned rather than vacuous.
        let polyglot = render_text(
            &suite_in(Ecosystem::Polyglot),
            &catalogue(),
            &SutChoice::Vulture,
        );
        assert!(!polyglot.contains("NOT READ"), "got {polyglot}");

        // And the language-agnostic controls are never marked, in any ecosystem.
        let naive = render_text(&suite_in(Ecosystem::Go), &catalogue(), &SutChoice::Naive);
        assert!(!naive.contains("NOT READ"), "got {naive}");
    }

    #[test]
    fn every_ecosystem_is_claimed_by_exactly_the_analyzers_that_parse_it() {
        // The language map decides two things a report cannot recover from
        // being wrong about. Too narrow and a genuine false removal is excused
        // as "not measured"; too wide and a tool is credited with passing a
        // class it never opened. Both are stated here as the full matrix rather
        // than as spot checks, because the failure is a *missing* or *extra*
        // entry and a spot check on the entries that exist cannot see either.
        let expected: &[(SutChoice, &[Ecosystem])] = &[
            (
                SutChoice::Vulture,
                &[Ecosystem::Python, Ecosystem::Polyglot],
            ),
            (
                SutChoice::Knip,
                &[Ecosystem::TypeScript, Ecosystem::Polyglot],
            ),
            (SutChoice::Deadcode, &[Ecosystem::Go]),
            (SutChoice::Shear, &[Ecosystem::Rust]),
        ];

        for (choice, langs) in expected {
            assert_eq!(
                reads(choice),
                Some(*langs),
                "`--sut {}` reads the wrong set of languages",
                choice.label()
            );

            for ecosystem in [
                Ecosystem::Python,
                Ecosystem::TypeScript,
                Ecosystem::Rust,
                Ecosystem::Go,
                Ecosystem::Polyglot,
            ] {
                assert_eq!(
                    was_read(choice, ecosystem),
                    langs.contains(&ecosystem),
                    "`--sut {}` on a {} class: the marker and the language map disagree",
                    choice.label(),
                    self::ecosystem(ecosystem),
                );
            }
        }

        // And the ones that declare nothing keep declaring nothing. An
        // arbitrary command has unknown competence, which is not a claim in
        // either direction.
        for choice in [
            SutChoice::Naive,
            SutChoice::Refusing,
            SutChoice::Command(vec!["mytool".to_string()]),
        ] {
            assert_eq!(reads(&choice), None, "got a language claim for {choice:?}");
        }
    }

    #[test]
    fn a_go_only_analyzer_is_marked_absent_on_every_other_ecosystem() {
        // The concrete consequence of the map above, at the surface a human
        // reads. deadcode graded against a Rust fixture opens no file, claims
        // nothing, and would otherwise render identically to a tool that read
        // the code and correctly kept it (§6.20).
        let text = render_text(
            &suite_in(Ecosystem::Rust),
            &catalogue(),
            &SutChoice::Deadcode,
        );
        assert!(text.contains("[NOT READ by this SUT]"), "got {text}");
        assert!(text.contains("not measured: 1 of 1 classes"), "got {text}");

        // Its own ecosystem carries no marker, or the marker stops carrying
        // information and starts being noise a reader learns to skip.
        let go = render_text(&suite_in(Ecosystem::Go), &catalogue(), &SutChoice::Deadcode);
        assert!(!go.contains("NOT READ"), "got {go}");
        assert!(!go.contains("not measured"), "got {go}");
    }

    #[test]
    fn the_declared_completed_exit_codes_are_the_measured_ones() {
        // Measured tables live on the constants; this pins the values so that a
        // later edit has to go and change the documented measurement too.
        //
        // The direction of each mistake is the reason this is a test rather
        // than a comment. A set that is too narrow refuses every productive run
        // — knip and cargo-shear both report findings *by* exiting 1. A set
        // that is too wide grades a crash as a clean scan: deadcode returns 1
        // for "this is not a Go module" and for "your Go does not compile"
        // alike, with empty stdout both times.
        assert_eq!(VULTURE_COMPLETED_EXIT_CODES, [0, 3]);
        assert_eq!(KNIP_COMPLETED_EXIT_CODES, [0, 1]);
        assert_eq!(DEADCODE_COMPLETED_EXIT_CODES, [0]);
        assert_eq!(SHEAR_COMPLETED_EXIT_CODES, [0, 1]);

        // knip is the one tool that states its own health bit, so the CLI's
        // copy must not disagree with it.
        assert_eq!(
            KNIP_COMPLETED_EXIT_CODES.as_slice(),
            knip::SUCCESS_EXIT_CODES,
            "the CLI and the knip adapter disagree about which exits are healthy"
        );

        // No named analyzer may declare 2 healthy. Every one of these tools
        // uses it for "I could not run here at all" — no package.json, no
        // Cargo.toml, an unparseable command line — and every one of those
        // states has an empty or non-report stdout that parses to no claims,
        // which is zero false removals, which is a passing gate.
        for codes in [
            VULTURE_COMPLETED_EXIT_CODES.as_slice(),
            KNIP_COMPLETED_EXIT_CODES.as_slice(),
            DEADCODE_COMPLETED_EXIT_CODES.as_slice(),
            SHEAR_COMPLETED_EXIT_CODES.as_slice(),
        ] {
            assert!(!codes.contains(&2), "exit 2 declared healthy in {codes:?}");
        }
    }

    #[test]
    fn every_named_analyzer_discloses_an_envelope_and_a_mapping() {
        // §9.2's second non-SARIF clause. A SUT wired without a disclosure
        // publishes a number with nothing bounding it, and the omission is
        // invisible in the report — there is simply one less paragraph.
        for choice in [
            SutChoice::Vulture,
            SutChoice::Knip,
            SutChoice::Deadcode,
            SutChoice::Shear,
        ] {
            let disclosure = disclosure(&choice)
                .unwrap_or_else(|| panic!("{} discloses nothing", choice.label()));
            assert!(
                !disclosure.envelope.trim().is_empty(),
                "{} has an empty capability envelope",
                choice.label()
            );
            assert!(
                !disclosure.mapping.trim().is_empty(),
                "{} has an empty mapping decision",
                choice.label()
            );
        }
    }

    #[test]
    fn a_clean_gate_with_no_decoy_recall_says_so_rather_than_reading_as_success() {
        // The hole the gate deliberately leaves. A report that printed only
        // "GATE PASSED" here would be endorsing a tool that has never called
        // anything dead.
        let text = render_text(&suite(Vec::new(), 0, 3), &catalogue(), &SutChoice::Refusing);

        assert!(text.contains("false removals: 0"), "got {text}");
        assert!(text.contains("GATE PASSED"), "got {text}");
        assert!(text.contains("decoy recall: 0 of 3"), "got {text}");
        assert!(text.contains("removed nothing at all"), "got {text}");
    }

    #[test]
    fn a_false_removal_names_the_class_the_mechanism_and_the_artifact() {
        let text = render_text(
            &suite(vec!["app/tasks/nightly.py".to_string()], 1, 1),
            &catalogue(),
            &SutChoice::Naive,
        );

        assert!(text.contains("GATE FAILED"), "got {text}");
        assert!(
            text.contains("classes with false removals: m01"),
            "got {text}"
        );
        assert!(text.contains("app/tasks/nightly.py"), "got {text}");
        assert!(text.contains("§10 E2 class 1"), "got {text}");
        assert!(
            text.contains("a dotted path in a settings string"),
            "the mechanism is the finding; without it a failure is just an id"
        );
    }

    #[test]
    fn the_summary_lines_start_at_column_zero() {
        // What CI shows is the tail of the log, and what a human greps for is a
        // line start. Indenting these would bury the only two numbers that
        // decide anything.
        let text = render_text(
            &suite(vec!["x".to_string()], 0, 1),
            &catalogue(),
            &SutChoice::Naive,
        );

        for expected in [
            "false removals: ",
            "decoy recall: ",
            "classes with false removals: ",
        ] {
            assert!(
                text.lines().any(|line| line.starts_with(expected)),
                "no line starts with `{expected}`; got {text}"
            );
        }
    }
}
