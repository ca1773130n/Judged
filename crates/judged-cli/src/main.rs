//! `judged` — the command-line entry point.
//!
//! Two subcommands, in the order §0.5 demands they be built: `judged ratchet`
//! (§9.14) baselines a repository and fails CI only on new findings, and
//! `judged mutants` (§10 E2) runs the mutation-injection suite that gates
//! whether an auto-act tier may exist at all.
//!
//! There is deliberately no `judged clean`, no `judged reap`, and no `--fix`
//! (§9.13 invariant 1). Neither subcommand opens the working tree for writing;
//! the only file this binary can ever write is the committed baseline, and only
//! when asked with `--update`.
//!
//! Exit codes are the contract, and they are Ruff's (§9.2): **0 clean, 1
//! findings, 2 abnormal termination**. §9.2 records that knip, vulture,
//! ts-prune, Go deadcode and Periphery all conflate "clean" with "crashed
//! before doing anything"; refusal is the only path to 2 here, and it is
//! reachable from a crashed analyzer, an unusable baseline, and a malformed
//! command line alike.

mod args;
mod clock;
mod mutants_cmd;
mod ratchet_cmd;

use std::io::Write;

use args::Invocation;

fn main() {
    let (report, code) = match args::parse(std::env::args().skip(1)) {
        Ok(Invocation::Help) => (args::USAGE.to_string(), 0),
        Ok(Invocation::Ratchet(args)) => ratchet_cmd::run(&args),
        Ok(Invocation::Mutants(args)) => mutants_cmd::run(&args),
        // A command line that could not be understood analyzed nothing, so it
        // takes the same exit as a refusal rather than the exit of a clean run.
        Err(error) => (format!("{}\n\n{}", error.message, args::USAGE), 2),
    };

    // Written in one shot rather than streamed: the report is a page of text,
    // and a partially-written report paired with an exit code is worse than
    // either alone. A closed pipe (`judged mutants | head`) is not a reason to
    // change the exit code — the analysis already happened and its verdict
    // stands.
    let stream = std::io::stdout();
    let mut stream = stream.lock();
    let _ = stream.write_all(report.as_bytes());
    let _ = stream.flush();

    std::process::exit(code);
}
