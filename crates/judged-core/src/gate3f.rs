//! Gate 3f — persisted, in-flight, and already-shipped references (§9.3, §6.24).
//!
//! §9.3 states the conjunct in one sentence and ends it with the only clause of
//! its kind in the whole design:
//!
//! > **3f** NOT §6.24: the candidate's type is not serializable, its name cannot
//! > appear in a queue payload, and its symbol is not exported across an ABI
//! > boundary. **No ban count overrides this.**
//!
//! Every other gate weighs. This one refuses, and nothing outweighs it, because
//! the evidence that would refute deadness **is not in any observable system**.
//! §6.24: a Sidekiq payload enqueued yesterday, a row pickled last year, a
//! binary linked in 2023. Static reachability, the grep veto, runtime coverage,
//! tombstones and the build graph all read the *current* repository and the
//! *currently running* fleet. None of them can see any of those.
//!
//! That is why deleting one of these breaks nothing you can measure. The build
//! is green, the tests are green, the deploy is green, and the worker dies hours
//! later on a job enqueued before the deploy — with retry-and-backoff turning a
//! single error into a poison-pill loop.
//!
//! # It refuses; it never accuses
//!
//! Same invariant as Gate 1 and the three rescue layers: the only operation is
//! dropping a claim. A candidate with no 3f finding has not been shown to be
//! dead — it has been shown that *this* gate has nothing to say about it.
//!
//! # The markers are §6.24's own list, not a guess
//!
//! §6.24 enumerates its counter-signals explicitly, which is unusual in this
//! design and is why this gate could be built straight from the specification
//! rather than fitted to whatever the E2 catalogue happens to contain. The three
//! conditions below cite the phrases they implement.
//!
//! # Where this is narrower than §6.24, and why that is stated rather than hidden
//!
//! For the queue condition, §6.24 says *"every class reachable from a job /
//! serializable base type is ineligible"*. **Reachable** is a type-level
//! analysis this project does not have. What is implemented is the checkable
//! subset — the declaring file itself references a detected job framework — and
//! that is deliberately weaker than the specification, so a class that inherits
//! a job base three files away is missed. Being narrower than a safety rule is
//! the wrong direction to err in, so it is recorded as a gap rather than
//! described as the rule. See [`Condition::QueuePayload`].
//!
//! The alternative is worse and was rejected: refusing every symbol in any
//! repository that contains Sidekiq would make the gate a constant function, and
//! a gate that refuses everything measures exactly as much as one that refuses
//! nothing (§3.7 makes the same point about a positive control that always
//! passes).

use std::collections::BTreeSet;
use std::fmt;
use std::path::{Path, PathBuf};

use crate::{Error, Result};

/// Which of 3f's three conditions fired.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Condition {
    /// *"the candidate's type is not serializable"*.
    ///
    /// §6.24's counter-signal is explicit about scope: the marker counts *"on
    /// the candidate **or its declaring type**"*, so this is judged at file
    /// granularity rather than by proximity to the symbol. A class definition is
    /// the schema for data already written to disk; deleting a field of it is a
    /// silent read failure at some future date.
    Serializable,
    /// *"its name cannot appear in a queue payload"*.
    ///
    /// Narrower than §6.24 as implemented — see the module docs. A finding here
    /// means the declaring file references a detected job framework, not that a
    /// type-level reachability analysis was performed.
    QueuePayload,
    /// *"its symbol is not exported across an ABI boundary"*.
    ///
    /// Judged by proximity, because this condition is about the **symbol** and
    /// not the file: one `#[no_mangle]` export must not refuse every other
    /// symbol that happens to share its module.
    AbiExport,
}

impl Condition {
    /// Stable lower-case label, for reports.
    pub fn as_str(self) -> &'static str {
        match self {
            Condition::Serializable => "serializable",
            Condition::QueuePayload => "queue-payload",
            Condition::AbiExport => "abi-export",
        }
    }

    /// What deleting a candidate in this condition actually breaks, in a
    /// sentence somebody can act on. §9.13 wants a reason, not a score.
    pub fn consequence(self) -> &'static str {
        match self {
            Condition::Serializable => {
                "the type is the schema for data already written to disk, so removing it is a \
                 silent read failure whenever that data is next loaded"
            }
            Condition::QueuePayload => {
                "a job enqueued before the deploy names this by string, so removing it kills the \
                 worker hours later — and retry-with-backoff makes it a poison-pill loop rather \
                 than one error"
            }
            Condition::AbiExport => {
                "already-linked consumers that were never rebuilt resolve this symbol, and there \
                 is no in-repo evidence of them at all"
            }
        }
    }
}

impl fmt::Display for Condition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One condition firing, with the evidence that fired it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gate3fFinding {
    /// Which condition.
    pub condition: Condition,
    /// The marker literal that matched, quoted closely enough to re-check by
    /// hand.
    pub marker: String,
    /// The file it matched in, repo-relative.
    pub found_in: PathBuf,
    /// The 1-based line, so a reader can open the file at the right place.
    pub line: usize,
    /// The whole reason, including what deleting the candidate would break.
    pub detail: String,
}

impl fmt::Display for Gate3fFinding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "3f {}: {} ({}:{})",
            self.condition,
            self.detail,
            self.found_in.display(),
            self.line
        )
    }
}

/// What 3f had to say about one candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gate3fVerdict {
    candidate: String,
    findings: Vec<Gate3fFinding>,
}

impl Gate3fVerdict {
    /// The candidate, spelled as it was handed over.
    pub fn candidate(&self) -> &str {
        &self.candidate
    }

    /// Every condition that fired, in [`Condition`] order. Empty when none did.
    ///
    /// All of them, not the first: a Celery task class that is also
    /// `#[derive(Deserialize)]` is refused twice for two different reasons, and
    /// reporting only whichever the evaluation order reached first would make
    /// the verdict an artifact of this file's shape.
    pub fn findings(&self) -> &[Gate3fFinding] {
        &self.findings
    }

    /// Whether 3f refuses this candidate.
    ///
    /// **The complement is not a safety claim.** No condition firing means this
    /// gate has nothing to say, not that the candidate is dead.
    pub fn is_ineligible(&self) -> bool {
        !self.findings.is_empty()
    }
}

// ---------------------------------------------------------------------------
// The markers, quoted from §6.24
// ---------------------------------------------------------------------------

/// Serialization markers, from §6.24's counter-signal list verbatim:
/// *"`serialVersionUID`, `__reduce__`, `__getstate__`/`__setstate__`,
/// `readObject`/`writeObject`, `[Serializable]`, `#[derive(Serialize,
/// Deserialize)]` on the candidate or its declaring type → **VETO**, matching
/// OpenRewrite's shipped behaviour."*
///
/// serde is handled separately by [`derive_line_names_serde`], because a real
/// derive list is `#[derive(Debug, Clone, Serialize, Deserialize)]` and a
/// literal `#[derive(Serialize` would miss every one of them.
const SERIALIZATION_MARKERS: &[&str] = &[
    "serialVersionUID",
    "__reduce__",
    "__getstate__",
    "__setstate__",
    "readObject",
    "writeObject",
    "[Serializable]",
    "implements Serializable",
    "BinaryFormatter",
    "Marshal.load",
    "Marshal.dump",
    "pickle.load",
    "pickle.dump",
];

/// ABI-boundary markers. §6.24: *"Library-evolution / ABI markers (`soname`,
/// `@_spi`, `-fvisibility=default` exports, `.map` version scripts,
/// `#[no_mangle]`, JNI `native`)"*, plus the two spellings §7 item 4 of the R1
/// determination names for Go and the Rust 2024 form.
const ABI_MARKERS: &[&str] = &[
    "#[no_mangle]",
    "#[unsafe(no_mangle)]",
    "//export ",
    "extern \"C\"",
    "JNIEXPORT",
    "__declspec(dllexport)",
    "@_cdecl",
    "@_spi(",
    "-fvisibility=default",
    "soname",
];

/// Job frameworks, from §6.24 verbatim: *"`sidekiq`, `celery`, `activejob`,
/// `resque`, `bull`, `hangfire`, `rq`, `dramatiq`, `temporal`"*.
///
/// `rq` and `bull` are two and four characters and would match inside ordinary
/// words everywhere, so a framework is only ever detected in a **declaration
/// context** — a dependency manifest, or an import line. See
/// [`Gate3f::detect_frameworks`].
const JOB_FRAMEWORKS: &[&str] = &[
    "sidekiq",
    "celery",
    "activejob",
    "resque",
    "bull",
    "hangfire",
    "rq",
    "dramatiq",
    "temporal",
];

/// Files that declare dependencies, where a framework name is a declaration
/// rather than a coincidence.
const MANIFESTS: &[&str] = &[
    "package.json",
    "requirements.txt",
    "pyproject.toml",
    "Pipfile",
    "Gemfile",
    "go.mod",
    "Cargo.toml",
    "composer.json",
    "build.gradle",
    "pom.xml",
];

/// How many lines after a marker a symbol may appear and still be the thing the
/// marker exports.
///
/// Three, because the common shapes need it and nothing needs more:
/// `#[no_mangle]` sits above an optional `pub extern "C"` and then the `fn`
/// line, and Go's `//export Name` names the symbol on the marker line itself.
/// Widening this trades precision for refusals in a gate that is already
/// absorbing.
const ABI_PROXIMITY_LINES: usize = 3;

// ---------------------------------------------------------------------------
// The gate
// ---------------------------------------------------------------------------

/// One job framework found in a declaration context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectedFramework {
    /// The framework name as §6.24 spells it.
    pub name: &'static str,
    /// Where it was declared, repo-relative.
    pub found_in: PathBuf,
    /// The 1-based line.
    pub line: usize,
}

/// Gate 3f over one repository.
///
/// Built once per repository: the job-framework scan is repo-wide and its result
/// is queried per candidate, so a suite that judges many claims does not re-read
/// every manifest for each one.
pub struct Gate3f {
    root: PathBuf,
    frameworks: Vec<DetectedFramework>,
}

impl Gate3f {
    /// Scan `root` for job frameworks and prepare to judge candidates.
    pub fn build(root: &Path) -> Result<Gate3f> {
        let frameworks = Gate3f::detect_frameworks(root)?;
        Ok(Gate3f {
            root: root.to_path_buf(),
            frameworks,
        })
    }

    /// The repository this gate reads.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Job frameworks found anywhere in the repository, in discovery order.
    ///
    /// Exposed because a report that says "refused: queue payload" without
    /// saying *which framework was detected and where* is a verdict a reader
    /// cannot check.
    pub fn frameworks(&self) -> &[DetectedFramework] {
        &self.frameworks
    }

    /// Judge a claimed **file**.
    ///
    /// All three conditions are asked at file granularity here, including the
    /// ABI one: a claim that the whole file is dead is a claim that everything
    /// in it is dead, so a single exported symbol anywhere in it refuses the
    /// claim.
    pub fn judge_path(&self, relative: &Path) -> Result<Gate3fVerdict> {
        let findings = self.scan(relative, None)?;
        Ok(Gate3fVerdict {
            candidate: relative.display().to_string(),
            findings,
        })
    }

    /// Judge a claimed **symbol**, given the file the analyzer said declares it.
    ///
    /// With no declaring file there is nothing to read, so the verdict is empty
    /// — which is the honest answer and not a clearance. A gate that guessed a
    /// file here would refuse or clear on a file the analyzer never named.
    pub fn judge_symbol(&self, name: &str, declared_in: Option<&Path>) -> Result<Gate3fVerdict> {
        let findings = match declared_in {
            Some(file) => self.scan(file, Some(name))?,
            None => Vec::new(),
        };
        Ok(Gate3fVerdict {
            candidate: name.to_string(),
            findings,
        })
    }

    /// The three conditions over one file. `symbol` narrows the ABI condition to
    /// proximity; `None` means judge the file as a whole.
    fn scan(&self, relative: &Path, symbol: Option<&str>) -> Result<Vec<Gate3fFinding>> {
        let absolute = self.root.join(relative);
        // A file that is not there cannot be read, and cannot refuse. Distinct
        // from a file that is there and says nothing, but the verdict is the
        // same shape and neither is a clearance.
        let Ok(text) = std::fs::read_to_string(&absolute) else {
            return Ok(Vec::new());
        };

        let mut findings = Vec::new();
        let lines: Vec<&str> = text.lines().collect();

        for (index, line) in lines.iter().enumerate() {
            let number = index + 1;

            for marker in SERIALIZATION_MARKERS {
                if line.contains(marker) {
                    findings.push(self.finding(Condition::Serializable, marker, relative, number));
                }
            }
            if derive_line_names_serde(line) {
                findings.push(self.finding(
                    Condition::Serializable,
                    "#[derive(… Serialize/Deserialize …)]",
                    relative,
                    number,
                ));
            }

            for marker in ABI_MARKERS {
                if !line.contains(marker) {
                    continue;
                }
                // For a symbol claim the marker only counts if it is plausibly
                // exporting *that* symbol. For a path claim every marker counts.
                let exports_the_symbol = match symbol {
                    None => true,
                    Some(name) => names_symbol_nearby(&lines, index, name),
                };
                if exports_the_symbol {
                    findings.push(self.finding(Condition::AbiExport, marker, relative, number));
                }
            }
        }

        // The queue condition is a property of the file and the repository
        // together, so it is asked once rather than per line.
        if let Some(framework) = self.framework_referenced_by(&text) {
            findings.push(Gate3fFinding {
                condition: Condition::QueuePayload,
                marker: framework.name.to_string(),
                found_in: framework.found_in.clone(),
                line: framework.line,
                detail: format!(
                    "{} is declared in {}:{} and this file references it, so {}",
                    framework.name,
                    framework.found_in.display(),
                    framework.line,
                    Condition::QueuePayload.consequence()
                ),
            });
        }

        // Sorted and deduplicated so a report is diffable and a file that spells
        // the same marker twice refuses once with one reason.
        findings.sort_by(|a, b| {
            (a.condition, &a.marker, a.line).cmp(&(b.condition, &b.marker, b.line))
        });
        findings.dedup_by(|a, b| a.condition == b.condition && a.marker == b.marker);
        Ok(findings)
    }

    fn finding(
        &self,
        condition: Condition,
        marker: &str,
        relative: &Path,
        line: usize,
    ) -> Gate3fFinding {
        Gate3fFinding {
            condition,
            marker: marker.to_string(),
            found_in: relative.to_path_buf(),
            line,
            detail: format!(
                "{} names {marker:?} at {}:{line}, so {}",
                relative.display(),
                relative.display(),
                condition.consequence()
            ),
        }
    }

    /// The first detected framework this file references, if any.
    fn framework_referenced_by(&self, text: &str) -> Option<&DetectedFramework> {
        let lowered = text.to_lowercase();
        self.frameworks
            .iter()
            .find(|framework| references_framework(&lowered, framework.name))
    }

    /// Every job framework declared anywhere in the repository.
    ///
    /// A framework counts as declared when it appears in a dependency manifest
    /// or on an import line, never merely because the string occurs. §6.24 lists
    /// `rq` and `bull` among the frameworks, and a bare substring search for
    /// those matches `torque`, `bulletin` and most of an English dictionary —
    /// which would put every symbol in every repository into the queue
    /// condition and turn the gate into a constant function.
    fn detect_frameworks(root: &Path) -> Result<Vec<DetectedFramework>> {
        let mut found: Vec<DetectedFramework> = Vec::new();
        let mut seen: BTreeSet<&'static str> = BTreeSet::new();

        for entry in walk(root)? {
            let relative = entry.strip_prefix(root).unwrap_or(&entry).to_path_buf();
            let is_manifest = relative
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| MANIFESTS.contains(&name));

            let Ok(text) = std::fs::read_to_string(&entry) else {
                continue;
            };
            for (index, line) in text.lines().enumerate() {
                let lowered = line.to_lowercase();
                let declaring = is_manifest || is_import_line(&lowered);
                if !declaring {
                    continue;
                }
                for name in JOB_FRAMEWORKS {
                    if seen.contains(name) || !contains_word(&lowered, name) {
                        continue;
                    }
                    seen.insert(name);
                    found.push(DetectedFramework {
                        name,
                        found_in: relative.clone(),
                        line: index + 1,
                    });
                }
            }
        }
        Ok(found)
    }
}

/// Whether a line is a `#[derive(...)]` naming serde.
///
/// Two steps rather than one literal, because a real derive list is
/// `#[derive(Debug, Clone, Serialize, Deserialize)]` and any single literal that
/// matched it would either miss the common form or match the words `Serialize`
/// and `Deserialize` wherever they appear in prose.
fn derive_line_names_serde(line: &str) -> bool {
    line.contains("#[derive(") && (line.contains("Serialize") || line.contains("Deserialize"))
}

/// Whether `name` appears on the marker line or in the declaration that
/// immediately follows it.
///
/// A marker annotates the *next declaration*, and a blank line ends that
/// declaration — so the window stops at one rather than running the full
/// [`ABI_PROXIMITY_LINES`]. Without that stop this reads into whatever comes
/// after, and Go's shape is the case that proves it: `//export TelemetryFlush`
/// sits three lines above an unrelated `func internalOnly()`, so a fixed window
/// refuses a symbol nothing exports. Which is how an absorbing gate quietly
/// becomes a constant function.
fn names_symbol_nearby(lines: &[&str], marker_index: usize, name: &str) -> bool {
    let bare = name.rsplit(['.', ':', '/', '#']).next().unwrap_or(name);
    if bare.is_empty() {
        return false;
    }
    let end = (marker_index + ABI_PROXIMITY_LINES + 1).min(lines.len());
    for (offset, line) in lines[marker_index..end].iter().enumerate() {
        // The marker's own line always counts — Go writes the exported name on
        // it — and a blank line after it ends the declaration it annotates.
        if offset > 0 && line.trim().is_empty() {
            return false;
        }
        if contains_word(line, bare) {
            return true;
        }
    }
    false
}

/// Whether a file's text references `framework` in a way that binds this file to
/// it: an import, a decorator, or a base class.
fn references_framework(lowered_text: &str, framework: &str) -> bool {
    lowered_text.lines().any(|line| {
        (is_import_line(line) || is_binding_line(line)) && contains_word(line, framework)
    })
}

/// An import in any of the ecosystems this project reads.
fn is_import_line(lowered: &str) -> bool {
    let trimmed = lowered.trim_start();
    trimmed.starts_with("import ")
        || trimmed.starts_with("from ")
        || trimmed.starts_with("use ")
        || trimmed.starts_with("require ")
        || trimmed.contains("require(")
        || trimmed.starts_with("#include")
        || trimmed.starts_with("gem ")
}

/// A decorator or a superclass list — the two places a job framework binds a
/// class to itself without an import on the same line.
fn is_binding_line(lowered: &str) -> bool {
    lowered.trim_start().starts_with('@')
        || lowered.contains("class ")
        || lowered.contains("extends ")
        || lowered.contains("include ")
}

/// Whether `needle` occurs in `haystack` bounded by non-identifier characters.
///
/// The whole reason `rq` and `bull` are usable at all: a substring test would
/// match them inside ordinary words, and §6.24 lists both.
fn contains_word(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    let bytes = haystack.as_bytes();
    let mut from = 0;
    while let Some(offset) = haystack[from..].find(needle) {
        let start = from + offset;
        let end = start + needle.len();
        let before_ok = start == 0 || !is_identifier_byte(bytes[start - 1]);
        let after_ok = end == bytes.len() || !is_identifier_byte(bytes[end]);
        if before_ok && after_ok {
            return true;
        }
        from = start + 1;
        if from >= haystack.len() {
            break;
        }
    }
    false
}

fn is_identifier_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// Every file under `root`, skipping `.git`.
///
/// Deliberately not git-aware: 3f asks what is *on disk*, and an untracked
/// `requirements.txt` declaring Celery is exactly as much evidence that this
/// repository runs a worker as a tracked one.
fn walk(root: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = std::fs::read_dir(&dir).map_err(|source| Error::Io {
            path: dir.clone(),
            source,
        })?;
        for entry in entries {
            let entry = entry.map_err(|source| Error::Io {
                path: dir.clone(),
                source,
            })?;
            let path = entry.path();
            if path.file_name().is_some_and(|name| name == ".git") {
                continue;
            }
            if path.is_dir() {
                stack.push(path);
            } else {
                out.push(path);
            }
        }
    }
    out.sort();
    Ok(out)
}
