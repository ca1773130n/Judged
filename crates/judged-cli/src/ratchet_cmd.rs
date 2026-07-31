//! `judged ratchet` — §9.14, the thing built first.
//!
//! Baseline the current state; fail CI only on **new** findings. Zero deletion
//! risk, zero configuration burden. §0.5: *"a reaper that never stops the inflow
//! is bailing a boat."*
//!
//! This module is the process boundary and nothing else. Every decision it
//! reports belongs to [`judged_ratchet`]: what counts as new, what counts as
//! rot, and — critically — what the exit code is. `exit_code` is called rather
//! than reimplemented, so that the CLI cannot drift away from the crate that
//! owns the contract.
//!
//! # Why the logs are merged before they are judged
//!
//! `--sarif` is repeatable because a real repository is scanned by several
//! adapters (§9.2: adapters emit SARIF, the orchestrator only ever reads it).
//! Those logs are concatenated into one run before the diff, because
//! [`judged_ratchet::detect_rot`] asks "does anything in this run carry that
//! fingerprint" — and judged per log, a finding knip reported and vulture did
//! not would be rot in vulture's log. That would make a multi-adapter ratchet
//! permanently red for reasons nobody can act on, which is how a gate gets
//! switched off.
//!
//! Health is assessed **per run** first, before the merge, so that a refusal
//! names the log and the tool that caused it rather than a synthetic merged
//! one. §6.20's failure mode is a crashed analyzer that nobody notices; an
//! unattributed refusal is barely better.

use std::path::{Path, PathBuf};

use judged_core::sarif::{
    assess_run_health, Artifact, Invocation, Run, RunHealth, SarifLog, SarifResult, Tool,
};
use judged_core::{Error, Result};
use judged_ratchet::{exit_code, ratchet, Baseline, BaselineEntry, RatchetOutcome, RotReason};

use crate::args::RatchetArgs;
use crate::clock::now_rfc3339;

/// Render a ratchet invocation into a report and an exit code.
///
/// Every error is a refusal. A ratchet that could not read its baseline, could
/// not find its SARIF log, or could not parse it has established nothing —
/// reporting that as 0 would be the exact conflation §9.2 says knip, vulture,
/// ts-prune, Go deadcode and Periphery all make.
pub fn run(args: &RatchetArgs) -> (String, i32) {
    match execute(args) {
        Ok(outcome) => outcome,
        Err(error) => (refusal_block(&[error.to_string()]), 2),
    }
}

/// One SARIF run, with enough provenance to name it in a refusal.
struct LoadedRun {
    label: String,
    run: Run,
}

fn execute(args: &RatchetArgs) -> Result<(String, i32)> {
    let cwd = std::env::current_dir().map_err(|source| Error::Io {
        path: PathBuf::from("."),
        source,
    })?;
    let repo = judged_core::git::Repo::discover(&cwd)?;
    let baseline_path = resolve(&args.baseline, repo.root());
    let baseline = Baseline::load(&baseline_path)?;
    let loaded = load_runs(&args.sarif)?;

    // Gate 1: is any of this worth judging? Per run, before the merge, so the
    // reason names a file a human can open.
    let mut refusals = Vec::new();
    let mut degradations = Vec::new();
    let mut expected_total = 0usize;
    for entry in &loaded {
        // Absent an operator-supplied expectation there is NO denominator, and
        // we say so rather than inventing one.
        //
        // The tempting default is `entry.run.artifacts.len()` — hold the run to
        // the universe it declared for itself. That is self-referential and it
        // silently disarms the control: a tool that loaded one file, crashed
        // out of the other thirty-nine, and emitted a single artifact declares
        // a universe of one, scans one of one, and scores 100% coverage. That
        // is the documented knip-fails-to-load-vite.config.ts shape (§6.20),
        // i.e. precisely the input the control exists to catch, passing the
        // control. §9.2 is explicit that the gate is
        // |analysisTarget| >= 0.8 x |candidate files for that language| — a
        // denominator the ANALYZER CANNOT SUPPLY, because it is the count of
        // files that should have been scanned, not the count that were.
        //
        // So: zero, which `assess_run_health` reports as Degraded ("coverage
        // could not be validated"), never Healthy. Judged would rather say it
        // does not know than assert a coverage ratio it computed from the very
        // output whose completeness is in question.
        let expected = args.expected_targets.unwrap_or(0);
        expected_total += expected;
        match assess_run_health(&entry.run, expected) {
            RunHealth::Failed { reasons } => refusals.extend(attribute(&entry.label, reasons)),
            RunHealth::Degraded { reasons } => {
                degradations.extend(attribute(&entry.label, reasons))
            }
            RunHealth::Healthy => {}
        }
    }
    if !refusals.is_empty() {
        // Before `--update`, deliberately. Rewriting the baseline from a run
        // that reported nothing because it died would erase the accepted
        // backlog and leave CI green over a repository nobody has analyzed
        // since — the single most destructive thing this binary could do.
        return Ok((refusal_block(&refusals), 2));
    }

    let merged = merge(&loaded);
    let now = now_rfc3339();

    if args.update {
        return update(&baseline, &baseline_path, &merged, &degradations, &now);
    }

    let outcome = ratchet(&baseline, &merged, &repo, expected_total, &now);
    let code = exit_code(&outcome);

    let mut report = header(&loaded, merged.results.len(), &baseline_path);
    report.push_str(&degradation_block(&degradations));
    report.push_str(&verdict_block(
        strip_degradation(&outcome),
        &args.sarif,
        &baseline_path,
    ));
    Ok((report, code))
}

// ---------------------------------------------------------------------------
// Reading
// ---------------------------------------------------------------------------

/// Read every `--sarif` path and flatten the logs into runs.
///
/// A log carrying an empty `runs` array is refused rather than skipped. SARIF
/// permits it, and it records that nothing was analyzed — which §6.20 insists is
/// a different state from "nothing was found", and the only one of the two that
/// must never pass a build.
fn load_runs(paths: &[PathBuf]) -> Result<Vec<LoadedRun>> {
    let mut loaded = Vec::new();
    for path in paths {
        let text = std::fs::read_to_string(path).map_err(|source| Error::Io {
            path: path.clone(),
            source,
        })?;
        let log: SarifLog = serde_json::from_str(&text).map_err(|source| Error::Json {
            context: path.display().to_string(),
            source,
        })?;
        for (index, run) in log.runs.into_iter().enumerate() {
            loaded.push(LoadedRun {
                label: format!("{} run {index} (tool `{}`)", path.display(), run.tool.name),
                run,
            });
        }
    }

    if loaded.is_empty() {
        let named = paths
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(Error::Sarif(format!(
            "{named} contained no runs. An empty `runs` array records that nothing was analyzed, \
             which is not the same as nothing being found (§6.20); a ratchet cannot pass a build \
             on it"
        )));
    }
    Ok(loaded)
}

/// Concatenate the runs into the single run the diff is taken against.
///
/// The synthetic tool name exists only so the merged value is well formed. It
/// is never quoted in a report: health was already assessed per run, and every
/// reason a reader sees carries the real log and the real tool.
fn merge(loaded: &[LoadedRun]) -> Run {
    let mut names: Vec<&str> = loaded.iter().map(|e| e.run.tool.name.as_str()).collect();
    names.dedup();

    let mut invocations: Vec<Invocation> = Vec::new();
    let mut artifacts: Vec<Artifact> = Vec::new();
    let mut results: Vec<SarifResult> = Vec::new();
    for entry in loaded {
        invocations.extend(entry.run.invocations.iter().cloned());
        artifacts.extend(entry.run.artifacts.iter().cloned());
        results.extend(entry.run.results.iter().cloned());
    }

    Run {
        tool: Tool {
            name: names.join(", "),
            version: None,
        },
        invocations,
        artifacts,
        results,
        baseline_guid: None,
    }
}

/// Relative baseline paths resolve against the repository root, not the working
/// directory, so `judged ratchet` behaves the same from a subdirectory as it
/// does from the top. The baseline is a property of the repository (§9.4).
fn resolve(path: &Path, root: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    }
}

fn attribute(label: &str, reasons: Vec<String>) -> Vec<String> {
    reasons
        .into_iter()
        .map(|reason| format!("{label}: {reason}"))
        .collect()
}

/// The verdict inside a [`RatchetOutcome::Degraded`], which [`ratchet`]
/// documents as wrapping at most once. Degradation is rendered from the
/// per-run pass instead, which attributes it to a log.
fn strip_degradation(outcome: &RatchetOutcome) -> &RatchetOutcome {
    match outcome {
        RatchetOutcome::Degraded { verdict, .. } => verdict,
        other => other,
    }
}

// ---------------------------------------------------------------------------
// --update
// ---------------------------------------------------------------------------

/// Rewrite the baseline from this run, preserving what a human put there.
///
/// [`Baseline::from_results`] stamps every entry with `first_seen = now`, which
/// is right for a first baseline and wrong for a rewrite: the committed file is
/// reviewed in the PR that changes it (§9.4), and resetting every timestamp
/// turns a two-line diff into a whole-file one. So the fresh entries are built
/// from the run and then re-annotated from the old file.
///
/// An `expires` is carried forward **unchanged**, including one that has already
/// passed. It is a deadline a human set; a mechanical rewrite has no standing to
/// extend it, and silently doing so is precisely how §9.14's "permanent amnesty
/// list" forms. The report says how many such entries there are so the loop is
/// visible rather than mysterious.
fn update(
    previous: &Baseline,
    path: &Path,
    merged: &Run,
    degradations: &[String],
    now: &str,
) -> Result<(String, i32)> {
    let fresh = Baseline::from_results(&merged.results, now);

    let mut entries: Vec<BaselineEntry> = Vec::with_capacity(fresh.entries().len());
    let mut carried = 0usize;
    let mut still_expired = 0usize;
    for entry in fresh.entries() {
        let mut entry = entry.clone();
        if let Some(old) = previous
            .entries()
            .iter()
            .find(|old| old.fingerprint == entry.fingerprint)
        {
            entry.first_seen = old.first_seen.clone();
            entry.expires = old.expires.clone();
            entry.justification = old.justification.clone();
            entry.symbol = old.symbol.clone();
            carried += 1;
            // `has_expired`, not `e <= now`. They differ on an unevaluable
            // date, and that difference decides whether `--update` can launder
            // a passed deadline into a permanent amnesty (§9.14).
            if old
                .expires
                .as_deref()
                .is_some_and(|e| judged_ratchet::has_expired(e, now))
            {
                still_expired += 1;
            }
        }
        entries.push(entry);
    }
    let accepted = entries.len();
    let added = accepted - carried;
    // Counted from the old side rather than as `previous.len() - carried`. The
    // baseline is hand-merged (§9.4), so two lines can carry the same
    // fingerprint; `carried` counts deduplicated fresh entries, and subtracting
    // it would report one dropped line where two were.
    let dropped = previous
        .entries()
        .iter()
        .filter(|old| {
            !fresh
                .entries()
                .iter()
                .any(|kept| kept.fingerprint == old.fingerprint)
        })
        .count();

    Baseline::new(entries).save(path)?;

    let mut report = format!("judged ratchet --update — rewrote {}\n\n", path.display());
    report.push_str(&degradation_block(degradations));
    if !degradations.is_empty() {
        report.push_str(
            "  A partial run baselines only what it scanned. Findings in the parts it missed \
             will read as NEW the next time it recovers.\n\n",
        );
    }
    // `first_seen`, not `firstSeen`: the baseline is snake_case on disk, and a
    // report that renames the field a reader is about to go and look at is a
    // report that costs them a minute.
    report.push_str(&format!(
        "  {accepted} accepted: {added} new, {carried} carried forward with the original \
         first_seen\n"
    ));
    report.push_str(&format!(
        "  {dropped} dropped: nothing in this run carried the fingerprint\n"
    ));
    if still_expired > 0 {
        report.push_str(&format!(
            "  {still_expired} kept an expiry that has already passed. --update will not extend a \
             deadline a human set; edit or delete those lines.\n"
        ));
    }
    report.push_str(
        "\nCommit the baseline. It is reviewed in the pull request that changes it (§9.4).\n",
    );
    Ok((report, 0))
}

// ---------------------------------------------------------------------------
// Reporting
// ---------------------------------------------------------------------------

fn header(loaded: &[LoadedRun], findings: usize, baseline_path: &Path) -> String {
    format!(
        "judged ratchet — {} {}, {} {}, against {}\n\n",
        loaded.len(),
        plural(loaded.len(), "run", "runs"),
        findings,
        plural(findings, "finding", "findings"),
        baseline_path.display()
    )
}

fn plural(count: usize, one: &str, many: &str) -> String {
    if count == 1 { one } else { many }.to_string()
}

/// The only path to exit 2.
fn refusal_block(reasons: &[String]) -> String {
    let mut block = String::from("REFUSED — this run cannot be judged (exit 2)\n\n");
    for reason in reasons {
        block.push_str(&format!("  {reason}\n"));
    }
    block.push_str(
        "\nNothing was compared against the baseline, and nothing was written. A refusal is not \
         a pass: an analyzer that died reports zero findings, and a ratchet that recorded that as \
         \"nothing new\" would be permanently disarmed while staying green (§6.20, §9.2).\n",
    );
    block
}

/// Degradation is reported and then ignored by the exit code, exactly as
/// [`exit_code`] does. §9.2 requires partial degradation to cap the tier for
/// affected paths, not to fail a build the analyzers did not fail — but a green
/// ratchet over half a repository is §6.20's outcome that nobody ever notices,
/// so it is said out loud.
fn degradation_block(reasons: &[String]) -> String {
    if reasons.is_empty() {
        return String::new();
    }
    let mut block = String::from(
        "DEGRADED — the verdict below stands, but it does not cover the whole repository\n\n",
    );
    for reason in reasons {
        block.push_str(&format!("  {reason}\n"));
    }
    block.push('\n');
    block
}

fn verdict_block(outcome: &RatchetOutcome, sarif: &[PathBuf], baseline_path: &Path) -> String {
    match outcome {
        RatchetOutcome::Clean => "clean: no new findings, no baseline rot\n".to_string(),
        RatchetOutcome::NewFindings(findings) => new_findings_block(findings, sarif),
        RatchetOutcome::Rot(reasons) => rot_block(reasons, sarif, baseline_path),
        RatchetOutcome::Refused { reason } => refusal_block(&[reason.clone()]),
        // `ratchet` wraps at most once and the caller already unwrapped, so this
        // arm is unreachable. Rendering it rather than panicking keeps a future
        // change to that invariant a formatting bug instead of a crash.
        RatchetOutcome::Degraded { verdict, .. } => verdict_block(verdict, sarif, baseline_path),
    }
}

/// New findings, **sorted by rule id** — §9.13 invariant 3: *"Sort by
/// confidence, never by bytes reclaimed. Size is anti-correlated with safety."*
/// A 4 GB `node_modules` is free; a 4 GB fine-tuned checkpoint representing 300
/// GPU-hours is the most expensive object on the machine, and ranking by size
/// puts it where a tired human's eye lands first. Size is not rendered here at
/// all, not even as a dim secondary column, because this build has no size to
/// render — a SARIF result carries none.
fn new_findings_block(findings: &[SarifResult], sarif: &[PathBuf]) -> String {
    let mut rows: Vec<(String, String, String)> = findings
        .iter()
        .map(|f| (f.rule_id.clone(), location(f), f.message.clone()))
        .collect();
    // Rule id first, then location, then message: a total order, so the report
    // is byte-identical between runs and can be diffed in CI.
    rows.sort();

    let rule_width = rows.iter().map(|(r, _, _)| r.len()).max().unwrap_or(0);
    let where_width = rows.iter().map(|(_, w, _)| w.len()).max().unwrap_or(0);

    let mut block = format!(
        "NEW FINDINGS ({}) — sorted by rule id, never by bytes reclaimed (§9.13 invariant 3)\n\n",
        rows.len()
    );
    for (rule, at, message) in &rows {
        block.push_str(&format!(
            "  {rule:rule_width$}  {at:where_width$}  {message}\n"
        ));
    }
    block.push_str(&format!(
        "\nFix them, or accept them into the baseline: judged ratchet{} --update\n",
        sarif_flags(sarif)
    ));
    block
}

/// Rot, in **baseline file order** rather than sorted.
///
/// The remediation is "delete these lines", so the order that helps is the order
/// they appear in the file being edited. That is not a departure from §9.13
/// invariant 3, which forbids ordering by bytes reclaimed; nothing here is
/// ordered by size.
fn rot_block(reasons: &[RotReason], sarif: &[PathBuf], baseline_path: &Path) -> String {
    let rows: Vec<(&str, String, &str)> = reasons
        .iter()
        .map(|reason| match reason {
            RotReason::NeverMatched { fingerprint } => (
                "never-matched",
                fingerprint.clone(),
                "nothing in this run carries this fingerprint; the amnesty protects nothing",
            ),
            RotReason::ReferentGone { uri } => (
                "referent-gone",
                uri.clone(),
                "the file this entry points at is gone; it can never match again",
            ),
            RotReason::Expired {
                fingerprint,
                expires,
            } => (
                "expired",
                format!("{fingerprint} (expires {expires})"),
                "the deadline on this entry has passed, or is not a date we can read",
            ),
        })
        .collect();

    let kind_width = rows.iter().map(|(k, _, _)| k.len()).max().unwrap_or(0);
    let subject_width = rows.iter().map(|(_, s, _)| s.len()).max().unwrap_or(0);

    let mut block = format!(
        "BASELINE ROT ({}) — entries that no longer earn their place, in baseline file order\n\n",
        rows.len()
    );
    for (kind, subject, why) in &rows {
        block.push_str(&format!(
            "  {kind:kind_width$}  {subject:subject_width$}  {why}\n"
        ));
    }
    block.push_str(&format!(
        "\nDelete those lines from {}, or rewrite it: judged ratchet{} --update\n\
         A baseline nobody prunes is hope written down (§9.14, SWE@Google Ch. 15).\n",
        baseline_path.display(),
        sarif_flags(sarif)
    ));
    block
}

/// Echo the `--sarif` flags back, so the suggested command is one a reader can
/// paste rather than reconstruct.
fn sarif_flags(sarif: &[PathBuf]) -> String {
    sarif
        .iter()
        .map(|p| format!(" --sarif {}", p.display()))
        .collect()
}

/// `uri:line`, or just the uri, or `(repository)` for a project-level finding.
///
/// The line number is display only. §9.2 is explicit that fingerprints must be
/// content-derived and never line-based; it is shown because a human needs to
/// open the file, and it is never an input to identity.
fn location(result: &SarifResult) -> String {
    match result.locations.first() {
        Some(location) => match location.start_line {
            Some(line) => format!("{}:{line}", location.uri),
            None => location.uri.clone(),
        },
        None => "(repository)".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use judged_core::sarif::{Level, Location};

    fn result(rule_id: &str, uri: &str, line: Option<u32>) -> SarifResult {
        SarifResult {
            rule_id: rule_id.to_string(),
            level: Level::Warning,
            message: "unused".to_string(),
            locations: vec![Location {
                uri: uri.to_string(),
                start_line: line,
            }],
            partial_fingerprints: Default::default(),
            baseline_state: None,
            suppressions: Vec::new(),
        }
    }

    #[test]
    fn a_project_level_finding_still_renders_a_location() {
        let mut repo_level = result("unused-dependency", "", None);
        repo_level.locations.clear();

        assert_eq!(location(&repo_level), "(repository)");
        assert_eq!(location(&result("r", "src/a.ts", None)), "src/a.ts");
        assert_eq!(location(&result("r", "src/a.ts", Some(9))), "src/a.ts:9");
    }

    #[test]
    fn findings_render_in_rule_id_order_whatever_order_they_arrived_in() {
        // §9.13 invariant 3, at the level that actually emits the bytes.
        let block = new_findings_block(
            &[
                result("zz-late", "src/z.ts", None),
                result("aa-early", "src/a.ts", None),
            ],
            &[PathBuf::from("knip.sarif")],
        );

        let early = block.find("aa-early").unwrap_or(usize::MAX);
        let late = block.find("zz-late").unwrap_or(0);
        assert!(early < late, "got {block}");
    }

    #[test]
    fn a_refusal_never_borrows_the_vocabulary_of_a_pass() {
        let block = refusal_block(&["knip.sarif run 0: it died".to_string()]);

        assert!(block.contains("REFUSED"), "got {block}");
        assert!(!block.contains("clean:"), "got {block}");
    }
}
