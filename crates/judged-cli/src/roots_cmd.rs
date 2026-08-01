//! `judged show-roots` — ProGuard `-printseeds`, which §9.13 asks for by name.
//!
//! Reachability analysis is only meaningful under a closed-world assumption, and
//! §1.2 argues that a real repository is open-world in at least five independent
//! directions. **You cannot infer the closed world. You can only have it
//! declared.** So this command decides nothing: it materializes what was
//! declared, records where each root came from, and shows it to a human — the
//! job ProGuard's `-printseeds` and Nix's `--print-roots` do, and the reason
//! both exist is that a classification nobody can audit is a classification
//! nobody should act on.
//!
//! # Two halves, and the second is the one that gets dropped
//!
//! The roots are the easy half. The other half is everything the materializer
//! could **not** resolve: a framework it recognized and has no plugin for, a
//! manifest that would not parse, a declared entry that matched nothing. §6.20's
//! rule — *"no data" must be a distinct state from "zero"* — is what makes those
//! load-bearing. A framework detected with no plugin contributes zero roots, and
//! a report that prints only successes renders that identically to a framework
//! that genuinely has none. The first case means an entire convention's worth of
//! entry points is missing from the list; the second means nothing is. They must
//! not look the same.
//!
//! # Exit codes
//!
//! 0 when a root set was materialized, whatever it contains — a repository with
//! few roots is a normal repository, and a gap is a thing to read rather than a
//! build to fail. 2 when nothing could be read at all, because zero roots over
//! zero files is the absence of a scan wearing the digits of an empty
//! repository.

use std::path::Path;

use judged_mutants::roots::{self, Gap, Root, RootSet, Tier};
use judged_ratchet::rot::has_expired;
use serde_json::{json, Value};

use crate::args::ShowRootsArgs;
use crate::clock::now_rfc3339;

/// Materialize the root set for one repository and render it.
pub fn run(args: &ShowRootsArgs) -> (String, i32) {
    let root = match args.path.canonicalize() {
        Ok(root) => root,
        Err(error) => {
            return (
                refusal(
                    args,
                    &format!("`{}` could not be opened", args.path.display()),
                    &error.to_string(),
                    "Give a path to a repository, or run `judged show-roots` from inside one.",
                ),
                2,
            )
        }
    };
    if !root.is_dir() {
        return (
            refusal(
                args,
                &format!("`{}` is not a directory", args.path.display()),
                "A root set is materialized over a repository, and this path names something \
                 else.",
                "Give the repository's directory instead.",
            ),
            2,
        );
    }

    // Every tracked-looking file, because that is what a Tier C pathspec is
    // matched against. Passing a narrower set would silently shrink Tier C to
    // whatever this command happened to look at, which is the one tier whose
    // whole purpose is to name things nothing else can find.
    let candidates = walk(&root);
    let set = roots::materialize(&root, &candidates);

    // Rot is a reporting concern and is asked for here rather than inside the
    // materializer: `DeclaredRoots::materialize` never consults `expires`, so a
    // rotted entry still protects and cannot silently change a rescue. What it
    // needs is a clock, and the clock lives at the process boundary.
    let rot = set.lint_declared(&root, &now_rfc3339(), &has_expired);

    if set.files_scanned() == 0 && candidates.is_empty() {
        return (
            refusal(
                args,
                &format!("nothing was read under `{}`", root.display()),
                "The walk visited no file at all, so this report would say a repository has no \
                 entry points when what actually happened is that nothing was looked at. §6.20: \
                 \"no data\" must be a distinct state from \"zero\", and \"this repository has no \
                 entry points\" is the most dangerous sentence this command could utter.",
                "Check the path, and check that it is a repository rather than an empty \
                 directory.",
            ),
            2,
        );
    }

    let report = if args.json {
        render_json(&root, &set, &rot)
    } else {
        render_text(&root, &set, &rot)
    };
    (report, 0)
}

/// Every file under `root`, repo-relative and forward-slashed.
///
/// The same directories `judged_core::roots::manifest` skips are skipped here,
/// for the same reason: each of them holds code that belongs to somebody else's
/// repository or to a build, and a root found inside one is a root for a project
/// we are not cleaning. The cost is stated rather than hidden — a source
/// directory that happens to be called `vendor` or `target` is not walked, so a
/// Tier C pathspec pointing into one will report as matching nothing.
///
/// Not git-aware, deliberately. An ignored-but-present file is still a path a
/// declaration can name, and asking git first would make a Tier C entry report
/// as protecting nothing purely because its referent is untracked — which reads
/// in this report exactly like an entry that has rotted.
fn walk(root: &Path) -> Vec<String> {
    fn descend(root: &Path, prefix: &Path, out: &mut Vec<String>) {
        let Ok(entries) = std::fs::read_dir(root.join(prefix)) else {
            // Unreadable directories are not silently equivalent to empty ones,
            // and the caller says so: an empty result refuses rather than
            // reporting a clean, empty root set.
            return;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy().into_owned();
            let relative = prefix.join(&name);
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                if judged_core::roots::manifest::SKIPPED_DIRECTORIES.contains(&name.as_str()) {
                    continue;
                }
                descend(root, &relative, out);
            } else {
                out.push(
                    relative
                        .components()
                        .map(|c| c.as_os_str().to_string_lossy())
                        .collect::<Vec<_>>()
                        .join("/"),
                );
            }
        }
    }

    let mut out = Vec::new();
    descend(root, Path::new(""), &mut out);
    out.sort();
    out
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

fn render_text(root: &Path, set: &RootSet, rot: &[Gap]) -> String {
    let mut out = format!(
        "judged show-roots — the materialized root set for {}\n\n\
         \x20 This decides nothing. §1.2: you cannot infer the closed world, you can only have\n\
         \x20 it declared. What follows is what WAS declared, with the file and key that\n\
         \x20 declared it, so the classification can be audited before anything acts on it.\n\n",
        root.display()
    );

    out.push_str(&set.printseeds());

    // Rot gets its own sub-header rather than being appended to the list above,
    // and it is a counting decision rather than a layout one: the materializer's
    // own header states how many gaps IT found, and silently adding lines under
    // that number would make the report disagree with itself about how much is
    // missing. Same section, adjacent, separately counted.
    if !rot.is_empty() {
        out.push_str(&format!(
            "# ...and from linting the declared roots against reality ({} more)\n",
            rot.len()
        ));
        for gap in rot {
            out.push_str(&format!(
                "?\t{}\t{}\t{}\n",
                gap.kind.as_str(),
                gap.subject,
                gap.detail
            ));
        }
    }

    out.push('\n');
    if set.detections().is_empty() {
        out.push_str("frameworks: none recognized\n");
    } else {
        let spoken: Vec<String> = set
            .detections()
            .iter()
            .map(|detected| {
                format!(
                    "{} {}{}",
                    detected.framework,
                    detected.version.as_deref().unwrap_or("version-unknown"),
                    if detected.covered {
                        ""
                    } else {
                        // Named on the same line as the framework, because this
                        // is the difference between "we know this framework's
                        // roots" and "we know this framework is here".
                        " [NO PLUGIN — its convention roots are missing]"
                    }
                )
            })
            .collect();
        out.push_str(&format!("frameworks: {}\n", spoken.join(", ")));
    }

    out.push_str(&format!(
        "manifests read: {}\nfiles scanned: {}\n",
        set.manifests_read().len(),
        set.files_scanned(),
    ));

    for declaration in set.declarations() {
        out.push_str(&format!("declaration: {declaration}\n"));
    }

    // The summary lines last, because they are what a log tail shows and what a
    // human reads in the ten seconds §9.13 budgets. Both counts, always: a root
    // count printed without a gap count is a list of successes.
    out.push_str(&format!(
        "roots: {} — {} tier A, {} tier B, {} tier C\n",
        set.roots().len(),
        set.tier(Tier::A).count(),
        set.tier(Tier::B).count(),
        set.tier(Tier::C).count(),
    ));
    let gaps = set.gaps().len() + rot.len();
    out.push_str(&format!(
        "could not resolve: {gaps}{}\n",
        if gaps == 0 {
            " — every framework recognized was covered, every manifest parsed, and every \
             declared entry decided something. That is not the same as having recognized \
             everything."
        } else {
            " — each one is a place where a root that exists in the world is missing from the \
             list above"
        }
    ));
    out
}

fn render_json(root: &Path, set: &RootSet, rot: &[Gap]) -> String {
    let roots: Vec<Value> = set.roots().iter().map(root_json).collect();
    let gaps: Vec<Value> = set
        .gaps()
        .iter()
        .chain(rot.iter())
        .map(|gap| {
            json!({
                "kind": gap.kind.as_str(),
                "subject": gap.subject,
                "detail": gap.detail,
            })
        })
        .collect();

    let document = json!({
        "repository": root.display().to_string(),
        "roots": roots,
        "root_count": roots.len(),
        "tier_counts": {
            "A": set.tier(Tier::A).count(),
            "B": set.tier(Tier::B).count(),
            "C": set.tier(Tier::C).count(),
        },
        // Emitted whether or not it is empty, and beside the count, so a
        // consumer can require the key and notice a producer that predates it.
        // A dashboard that reads `roots` without `gaps` has recorded a list of
        // successes (§6.20).
        "gaps": gaps,
        "gap_count": gaps.len(),
        "frameworks": set.detections().iter().map(|detected| json!({
            "name": detected.framework,
            // The version REQUIREMENT as declared, not a resolved version, which
            // only a lockfile knows. `null` is a real caveat: §5.1 makes version
            // part of Tier B's correctness condition.
            "declared_version": detected.version,
            "covered": detected.covered,
            "evidence": detected.evidence,
        })).collect::<Vec<Value>>(),
        "manifests_read": set.manifests_read().iter()
            .map(|path| path.display().to_string()).collect::<Vec<String>>(),
        "files_scanned": set.files_scanned(),
        "declarations": set.declarations(),
        "decides_nothing": "This is what was DECLARED, not what is reachable. §1.2: you cannot \
                            infer the closed world, you can only have it declared.",
    });
    match serde_json::to_string_pretty(&document) {
        Ok(text) => format!("{text}\n"),
        Err(error) => format!("{{\"error\":\"{error}\"}}\n"),
    }
}

/// One root, with everything a reader needs to check it against the repository.
///
/// `tier` leads, because it is the load-bearing field: a root that does not say
/// which tier it came from invites a caller to trust a guessed convention as
/// though a manifest had declared it. `tier_caveat` rides beside it rather than
/// being left for the consumer to look up, so a machine-read root carries the
/// same warning a human-read one does.
fn root_json(root: &Root) -> Value {
    json!({
        "tier": root.tier().label(),
        "tier_caveat": root.tier().caveat(),
        "rule": root.rule(),
        "origin": root.origin(),
        "origin_file": root.origin_file().display().to_string(),
        "target": root.target(),
        "path": root.path(),
        "symbol": root.symbol(),
        "detail": root.detail(),
    })
}

/// A refusal, rendered for whoever asked.
///
/// Worded to avoid the strings a successful root dump is made of: a log scanner,
/// or a human skimming, must not be able to find "the root set" and a tier
/// heading in a report where nothing was read.
fn refusal(args: &ShowRootsArgs, headline: &str, detail: &str, remedy: &str) -> String {
    if args.json {
        let document = json!({
            "refused": true,
            "reason": headline,
            "detail": detail,
            "remedy": remedy,
        });
        // Note what is absent: `roots`, `gaps`, `root_count`. A consumer
        // reaching for them gets nothing rather than an empty list, because an
        // empty list here and an empty list from a repository with no roots are
        // the same bytes.
        return match serde_json::to_string_pretty(&document) {
            Ok(text) => format!("{text}\n"),
            Err(error) => format!("{{\"refused\":true,\"reason\":\"{error}\"}}\n"),
        };
    }
    format!("REFUSED — {headline} (exit 2)\n\n  {detail}\n  {remedy}\n")
}
