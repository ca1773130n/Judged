//! `judged explain <path>` — the gate trace §9.13 asks for by name.
//!
//! §9.13's second invariant lists three commands together: `--why-alive`,
//! `show-roots`, and `--explain <path>` — *"the full gate trace: which gate
//! vetoed, which `.gitignore` line matched, which magic bytes, which reference
//! hits, whether the toolchain was present"*. Gate 1 has existed as sixteen
//! classes and Gate 0g since the first commit, and until this command neither
//! was reachable by a human: they were internal predicates whose output nothing
//! printed.
//!
//! # Gate 0g leads, and that ordering is the content
//!
//! §9.3 puts recoverability before every usefulness question and says why in
//! the same breath: *"usefulness is irrelevant until recoverability is known,
//! because the cost of being wrong is set by the rung, not the tier."* §8.1 is
//! the proof, and it is the single most consequential finding in the research —
//! git protects the object database and not the working tree, so the intuitive
//! risk ordering is exactly backwards. A tracked file is dangerous to
//! *misclassify* and recoverable to delete; an untracked or ignored one is safe
//! to classify and **unrecoverable** to delete. A trace that printed the gate
//! verdict and omitted the rung would have published the cheap half of the
//! answer, and would teach the wrong model to whoever read it.
//!
//! # What it does not say
//!
//! It never says a file is safe to delete. Gate 1 refuses; it does not accuse
//! (§9.1), and NO OBJECTION means sixteen classes had nothing to say — not that
//! anything established the candidate is dead. The trace therefore ends by
//! naming the gates it did **not** run, because §6.20's rule applies to this
//! command's own output as much as to an analyzer's: a trace that silently omits
//! a gate is indistinguishable from one in which that gate abstained.

use std::path::{Path, PathBuf};

use judged_core::git::RecoverabilityClass;
use judged_mutants::gate1::{Gate1, Gate1Trace, IgnoreRule};
use serde_json::{json, Value};

use crate::args::ExplainArgs;

/// Trace one path through Gate 0g and Gate 1, and render it.
pub fn run(args: &ExplainArgs) -> (String, i32) {
    // Resolved against the working directory FIRST, and everything downstream
    // uses the absolute form. Gate 1's three modules all interpret a relative
    // path as relative to the *working tree root*, so handing them the string
    // the user typed joins it onto the root a second time whenever the two
    // differ — `judged explain repo/src/thing.py` from the directory above
    // becomes `<root>/repo/src/thing.py`, which does not exist.
    //
    // The consequence was not a visible failure, which is why it survived a
    // green test suite: a path that does not exist is not tracked, so Gate 0g
    // answered UNTRACKED (rung R9, the most alarming answer available) about a
    // committed and pushed file, and 1d refused it for unreadable content. Two
    // confident lines about a file the report was not asked about.
    let path = absolutize(&args.path);

    // The working tree is discovered from the path's own directory rather than
    // from the process's, so `judged explain ../other-repo/src/thing.py` traces
    // the repository that actually holds the file. Answering from the wrong
    // repository would produce a plausible trace about the wrong ignore rules.
    let anchor = anchor_for(&path);
    let gate = match Gate1::build(&anchor) {
        Ok(gate) => gate,
        Err(error) => {
            return (
                refusal(
                    args,
                    &format!("`{}` could not be traced", args.path.display()),
                    &error.to_string(),
                    "`judged explain` needs a path inside a git working tree: Gate 0g is \
                     computed from the index and the remote refs, and there is no honest \
                     answer about recoverability without them.",
                ),
                2,
            )
        }
    };

    let trace = match gate.trace(&path) {
        Ok(trace) => trace,
        Err(error) => {
            return (
                refusal(
                    args,
                    &format!("`{}` could not be classified", args.path.display()),
                    &error.to_string(),
                    "Give a path inside the working tree. Classifying something outside it \
                     would answer a question nobody asked with data we do not have.",
                ),
                2,
            )
        }
    };

    let report = if args.json {
        render_json(&trace, gate.root())
    } else {
        render_text(&trace, gate.root())
    };
    (report, 0)
}

/// The path as an absolute one, resolved the same way the shell resolved it.
///
/// Canonicalized where possible, because the working tree root git reports is
/// canonical and the two have to be comparable: on macOS a scratch directory is
/// reached as `/var/...` and reported as `/private/var/...`, and a candidate
/// keyed one way against a root keyed the other strips no prefix and is judged
/// as though it were outside the tree.
///
/// A path that does not exist cannot be canonicalized, and that case is not an
/// error — `judged explain` on a file somebody is deciding whether to restore is
/// exactly the case where it is missing. Its *parent* is canonicalized instead,
/// so the resolution is right even when the leaf is not there.
fn absolutize(path: &Path) -> PathBuf {
    if let Ok(canonical) = path.canonicalize() {
        return canonical;
    }
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().map_or_else(|_| path.to_path_buf(), |cwd| cwd.join(path))
    };
    match (absolute.parent(), absolute.file_name()) {
        (Some(parent), Some(name)) => parent
            .canonicalize()
            .map_or_else(|_| absolute.clone(), |parent| parent.join(name)),
        _ => absolute,
    }
}

/// The directory to discover the working tree from.
///
/// A file's parent, or a directory itself. `path` is already absolute, so there
/// is no bare-name case to fall back on.
fn anchor_for(path: &Path) -> PathBuf {
    if path.is_dir() {
        return path.to_path_buf();
    }
    path.parent()
        .map_or_else(|| path.to_path_buf(), Path::to_path_buf)
}

// ---------------------------------------------------------------------------
// Text
// ---------------------------------------------------------------------------

fn render_text(trace: &Gate1Trace, root: &Path) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "{}\n  in {}\n\n",
        trace.path.display(),
        root.display()
    ));

    if !trace.exists {
        out.push_str(
            "  NOTE — nothing is at this path right now. Every line below is about the path \
             rather than about content that exists, and Gate 1's classes that read bytes could \
             not read any.\n\n",
        );
    }

    // -- Gate 0g -----------------------------------------------------------
    out.push_str("RECOVERABILITY (Gate 0g, §8.1) — what git could give back\n");
    out.push_str(&format!(
        "  class    {}\n  rung     {}\n  meaning  {}\n",
        class_label(trace.recoverability),
        trace.rung,
        wrap(trace.rung_meaning, 11)
    ));
    if let Some(rule) = &trace.ignore_rule {
        out.push_str(&format!("  ignored  {}\n", ignore_line(rule)));
    }
    out.push('\n');

    // -- Gate 1 ------------------------------------------------------------
    out.push_str("GATE 1 — the never-touch inventory (§9.3)\n");
    if trace.is_ineligible() {
        out.push_str(&format!(
            "  INELIGIBLE — {} of the sixteen classes refuse this path.\n",
            trace.findings.len()
        ));
        out.push_str(
            "  A Gate 1 refusal is absorbing: it is justified by IRREVERSIBILITY, not by\n  \
             uselessness, so no later evidence of uselessness moves it.\n\n",
        );
        for finding in &trace.findings {
            out.push_str(&format!(
                "  {} {}\n      {}\n",
                finding.class,
                finding.title,
                wrap(&finding.evidence, 6)
            ));
        }
    } else {
        out.push_str(
            "  NO OBJECTION — none of the sixteen classes recognised this path.\n\n  \
             This is not permission to delete anything. Gate 1 refuses; it does not accuse\n  \
             (§9.1). An absence of objection from these sixteen classes says the path is not\n  \
             in the never-touch inventory, and says nothing whatever about whether it is used.\n",
        );
    }
    out.push('\n');

    // -- The evidence Gate 1 read ------------------------------------------
    out.push_str("EVIDENCE READ\n");
    out.push_str(&format!(
        "  magic bytes   {}\n",
        match &trace.magic {
            Some(magic) => format!(
                "{} at offset {} (§2.1 — read from the file, not from its name)",
                magic.label, magic.offset
            ),
            None => "none matched the signature table".to_string(),
        }
    ));
    out.push_str(&format!(
        "  file type     {}\n",
        match trace.type_signal {
            Some(signal) => format!("{} ({})", signal.label(), signal_kind(signal)),
            None => "UNDETERMINED — no extension, magic signature or path convention named it, \
                 which is class 1p above"
                .to_string(),
        }
    ));
    out.push_str(&format!(
        "  ignore rule   {}\n",
        match &trace.ignore_rule {
            Some(rule) => ignore_line(rule),
            None => "no ignore rule matched this path".to_string(),
        }
    ));
    out.push('\n');

    // -- What did not run --------------------------------------------------
    if !trace.gaps.is_empty() {
        out.push_str("SCAN GAPS — what the survey could not read (§6.20)\n");
        for gap in &trace.gaps {
            out.push_str(&format!("  - {gap}\n"));
        }
        out.push_str(
            "  While any of these stand, class 1a refuses the whole tree: absence of an\n  \
             effector was never proved, it was merely not observed.\n\n",
        );
    }

    out.push_str("NOT RUN by this command\n");
    out.push_str(
        "  Gate 0a–0f  the boundary refusals: symlink traversal, nested repositories,\n              \
         in-progress rebases, dirty trees, build locks.\n  \
         Gate 2      the reference veto — whether anything in the repository NAMES this\n              \
         path, and whether it was modified recently. `judged mutants --veto`\n              \
         measures it; nothing here has asked it.\n  \
         Gate 3      artifact and deadness promotion.\n  \
         A gate this command did not run is not a gate that abstained. Nothing above is\n  \
         evidence that this path is unused (§6.20).\n",
    );
    out
}

/// `RecoverabilityClass` as §9.3 spells it in the pipeline listing.
fn class_label(class: RecoverabilityClass) -> &'static str {
    match class {
        RecoverabilityClass::TrackedPushed => "TRACKED_PUSHED",
        RecoverabilityClass::TrackedUnpushed => "TRACKED_UNPUSHED",
        RecoverabilityClass::Untracked => "UNTRACKED",
        RecoverabilityClass::Ignored => "IGNORED",
    }
}

/// Which of the three determinations named the file's type.
fn signal_kind(signal: judged_core::gate1::contracts::TypeSignal) -> &'static str {
    use judged_core::gate1::contracts::TypeSignal;
    match signal {
        TypeSignal::Extension(_) => "a recognised extension",
        TypeSignal::Magic(_) => "recognised leading bytes",
        TypeSignal::PathName(_) => "a name the ecosystem gives a fixed meaning",
    }
}

/// One ignore rule, with the line a reader would go and edit.
fn ignore_line(rule: &IgnoreRule) -> String {
    format!(
        "`{}` at {}:{}{}",
        rule.pattern,
        rule.source.display(),
        rule.line,
        if rule.is_negation() {
            " — a `!` negation, so the repository is explicitly asking to keep this (class 1m)"
        } else {
            ""
        }
    )
}

/// Wrap a sentence to a readable width, indenting continuation lines.
///
/// The rung's meaning is a paragraph and the classes' evidence can be one too;
/// printed unwrapped they run off the side of a terminal, which is how the most
/// important line in the report becomes the one nobody reads.
fn wrap(text: &str, indent: usize) -> String {
    const WIDTH: usize = 76;
    let pad = " ".repeat(indent);
    let mut out = String::new();
    let mut column = indent;
    for word in text.split_whitespace() {
        if column > indent && column + 1 + word.len() > WIDTH {
            out.push('\n');
            out.push_str(&pad);
            column = indent;
        } else if column > indent {
            out.push(' ');
            column += 1;
        }
        out.push_str(word);
        column += word.len();
    }
    out
}

// ---------------------------------------------------------------------------
// JSON
// ---------------------------------------------------------------------------

fn render_json(trace: &Gate1Trace, root: &Path) -> String {
    let document = json!({
        "path": trace.path.display().to_string(),
        "repository": root.display().to_string(),
        "exists": trace.exists,
        // Gate 0g first here too. The key order in a JSON object is not a
        // contract, but the document is read by humans as often as by machines
        // and it should not have to be reordered to be understood.
        "recoverability": {
            "class": class_label(trace.recoverability),
            "rung": trace.rung,
            "meaning": trace.rung_meaning,
        },
        "gate1": {
            "disposition": if trace.is_ineligible() { "INELIGIBLE" } else { "NO_OBJECTION" },
            "findings": trace.findings.iter().map(|finding| json!({
                "class": finding.class,
                "title": finding.title,
                "evidence": finding.evidence,
            })).collect::<Vec<Value>>(),
        },
        "evidence": {
            "magic": trace.magic.as_ref().map(|magic| json!({
                "label": magic.label,
                "offset": magic.offset,
                "class": magic.class.code(),
            })),
            "type_signal": trace.type_signal.map(|signal| json!({
                "label": signal.label(),
                "kind": signal_kind(signal),
            })),
            "ignore_rule": trace.ignore_rule.as_ref().map(|rule| json!({
                "source": rule.source.display().to_string(),
                "line": rule.line,
                "pattern": rule.pattern,
                "negation": rule.is_negation(),
            })),
        },
        "scan_gaps": trace.gaps,
        // Named rather than omitted, for the same reason the text report names
        // them: a consumer that cannot see which gates ran will read a Gate 1
        // NO_OBJECTION as a deletion verdict (§6.20).
        "gates_not_run": ["0a-0f", "2", "3"],
    });
    match serde_json::to_string_pretty(&document) {
        Ok(text) => format!("{text}\n"),
        Err(error) => format!("{{\"refused\":true,\"reason\":\"{error}\"}}\n"),
    }
}

/// A refusal, in whichever form was asked for.
///
/// Note what is absent from the JSON: `recoverability`, `gate1`. A consumer
/// reaching for them gets nothing rather than a null, because a null class and a
/// class of `UNTRACKED` are both answers and only one of them was computed.
fn refusal(args: &ExplainArgs, headline: &str, detail: &str, remedy: &str) -> String {
    if args.json {
        let document = json!({
            "refused": true,
            "reason": headline,
            "detail": detail,
            "remedy": remedy,
        });
        return match serde_json::to_string_pretty(&document) {
            Ok(text) => format!("{text}\n"),
            Err(error) => format!("{{\"refused\":true,\"reason\":\"{error}\"}}\n"),
        };
    }
    format!("REFUSED — {headline} (exit 2)\n\n  {detail}\n  {remedy}\n")
}
