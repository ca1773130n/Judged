//! Gate 2b and 2c — the reachability veto (§9.3).
//!
//! Two questions, both answered over the raw bytes of every tracked file:
//!
//! - **2b, manifest paths.** Does a CI artifact path, a Dockerfile `COPY`/`ADD`
//!   source, a `.dockerignore` negation, `MANIFEST.in`, `package.json#files`, a
//!   pyproject `include`, or a `.gitattributes` `filter=lfs` line name this
//!   path? §5.2's root checklist lists every one of these as a place a build or
//!   a deploy target reads a path that no source file mentions.
//! - **2c, glob reachability.** Does anything in the repository *enumerate a
//!   directory at runtime*? If so the whole directory is rooted — not just the
//!   files the pattern obviously names. §6.12 is explicit about that, and
//!   records the precedent: GameMaker shipped "automatically remove unused
//!   assets" and it deletes assets referenced only from rooms and timelines,
//!   because the reference lived in a data file the analysis did not parse.
//!
//! # This layer can only rescue
//!
//! [`Verdict::Vetoed`] means *keep it*. [`Verdict::Clear`] means this module
//! found no reachability evidence, and that is **not** a claim the candidate is
//! dead — nothing here may ever be read as one, because Gate 2 exists to bound
//! accusers, never to make an accusation (§9.1). A veto is absorbing: no later
//! evidence withdraws it.
//!
//! # An incomplete read is a hit, never an absence
//!
//! The rule that outranks everything else here. A directory we could not list,
//! a file we could not read, a workflow whose YAML does not load, a
//! `package.json` that is not JSON — each of those has told us *nothing* about
//! what it names. Treating "I could not look" as "there is nothing there" is
//! the inversion §6.20 records Meta hitting in production, where a truncated
//! BigGrep read as "no references" turned the safety net into the deletion
//! trigger. So any such failure becomes [`VetoReason::IncompleteRead`] and
//! vetoes **every** candidate until it is fixed. That is deliberately loud: an
//! unusable scan must be impossible to mistake for a clean one.
//!
//! # Deliberately over-broad
//!
//! Detection is textual and cheap — presence of an enumeration construct, not a
//! parser for five languages — because it runs over every tracked file, and
//! because the two error directions are not comparable (§1.3). A false veto
//! costs recall. A missed one costs an incident. Three consequences are worth
//! stating up front, since each has a test:
//!
//! - The directory *containing* an enumerating file is rooted, because the
//!   enumeration target is usually a runtime value. In a repository-root file
//!   that roots the whole repository.
//! - A file that enumerates a directory rescues itself. Deleting a loader is a
//!   real hazard, so that is the safe direction, but it does mean a genuinely
//!   dead loader will never be reported by this gate.
//! - Rooting a directory rescues the dead files sitting in it too.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;

use aho_corasick::AhoCorasick;
use saphyr_parser::{Event, Parser, ScanError};

/// Constructs that enumerate a directory at runtime, from §6.12's list plus the
/// ecosystem-specific spellings §5.2 names.
///
/// Matched case-insensitively as raw substrings, which is why each construct
/// appears once in a canonical spelling: `readdir` also covers `readdirSync`
/// and `ReadDir`, `walkdir` also covers `WalkDir`. Over-broad on purpose — the
/// question is whether a directory *might* be enumerated, not whether this
/// particular call site definitely does.
pub const ENUMERATION_CONSTRUCTS: &[&str] = &[
    // A recursive glob pattern, in any language or config file. Spelled with a
    // slash so that markdown bold (`**like this**`) does not root every
    // directory that contains a README.
    "**/",
    "/**",
    // Shell, Python, PHP, Node, Rust.
    "glob(",
    "iglob(",
    "globsync(",
    "globby(",
    "fast-glob",
    "import.meta.glob",
    "readdir",
    "read_dir",
    "scandir",
    "listdir",
    "opendir",
    "walk(",
    "os.walk",
    "filepath.walk",
    "walkdir",
    "walk_dir",
    // Ruby.
    "dir[",
    "dir.glob",
    "dir.entries",
    "dir.children",
    "dir.foreach",
    "dir.each_child",
    // Bundlers.
    "require.context",
    // Python packaging and resource loading.
    "importlib.resources",
    "pkgutil.iter_modules",
    "pkg_resources",
    // Go.
    "go:embed",
    "embed.fs",
    // Rust.
    "include_str!",
    "include_bytes!",
    "include_dir!",
    // Swift / Objective-C: `Bundle.module` is how a SwiftPM resource is read,
    // and a resource is never reached by an import (§5.2, Swift).
    "bundle.module",
    "bundle.main",
    "nsbundle",
    "contentsofdirectory",
    // JVM.
    "listfiles(",
    "files.walk",
    "files.list",
    "getresources(",
    // .NET.
    "getfiles(",
    "enumeratefiles",
    "getdirectories(",
];

/// Never descended into: git's object store is not tracked content, and its
/// packfiles are not references.
const SKIPPED_DIRECTORY: &str = ".git";

/// Characters that make a path component a pattern rather than a name.
const GLOB_METACHARACTERS: [char; 4] = ['*', '?', '[', '{'];

fn automaton() -> &'static AhoCorasick {
    static AUTOMATON: OnceLock<AhoCorasick> = OnceLock::new();
    AUTOMATON.get_or_init(|| {
        AhoCorasick::builder()
            .ascii_case_insensitive(true)
            .build(ENUMERATION_CONSTRUCTS)
            .expect("the construct list is a compile-time constant")
    })
}

/// Why a candidate was rescued. Every variant names the file the evidence came
/// from, because a veto nobody can check is a veto nobody will trust.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VetoReason {
    /// 2c — something enumerates `rooted` at runtime, so everything under it is
    /// reachable without being named anywhere.
    EnumeratedDirectory {
        /// The construct that fired, as it appears in [`ENUMERATION_CONSTRUCTS`].
        construct: String,
        /// The file the construct was found in, repo-relative.
        found_in: PathBuf,
        /// The rooted path. An empty path is the repository root.
        rooted: PathBuf,
    },
    /// 2b — a manifest names `rooted`, either exactly or as a directory the
    /// candidate sits under.
    ManifestPath {
        /// The manifest that named it, repo-relative.
        manifest: PathBuf,
        /// The rooted path. An empty path is the repository root.
        rooted: PathBuf,
    },
    /// The scan did not complete over `path`. This is a hit, never an absence:
    /// a search that did not finish found nothing *because it did not look*
    /// (§6.20). It vetoes every candidate.
    IncompleteRead {
        /// What could not be read, repo-relative where it is inside the tree.
        path: PathBuf,
        /// What went wrong, in enough detail to fix it.
        detail: String,
    },
}

impl fmt::Display for VetoReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VetoReason::EnumeratedDirectory {
                construct,
                found_in,
                rooted,
            } => write!(
                f,
                "{} enumerates {} at runtime ({construct}), so the whole directory is rooted",
                found_in.display(),
                describe(rooted)
            ),
            VetoReason::ManifestPath { manifest, rooted } => {
                write!(f, "{} names {}", manifest.display(), describe(rooted))
            }
            VetoReason::IncompleteRead { path, detail } => write!(
                f,
                "the scan did not complete over {}: {detail} — an incomplete search \
                 is a hit, never an absence (§6.20)",
                path.display()
            ),
        }
    }
}

fn describe(path: &Path) -> String {
    if path.as_os_str().is_empty() {
        "the repository root".to_string()
    } else {
        path.display().to_string()
    }
}

/// The answer for one candidate.
///
/// There is deliberately no variant meaning "dead". [`Verdict::Clear`] says
/// only that *this* gate found no reachability evidence; what happens next is
/// still whatever the accusing analyzer said, bounded by the other gates.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use]
pub enum Verdict {
    /// Keep it. Absorbing — no later evidence overrides this.
    Vetoed {
        /// The evidence that rescued the candidate.
        reason: VetoReason,
    },
    /// No reachability evidence here. **Not** a claim of deadness.
    Clear,
}

impl Verdict {
    /// Was the candidate rescued?
    #[must_use]
    pub fn is_veto(&self) -> bool {
        matches!(self, Verdict::Vetoed { .. })
    }

    /// The evidence, when there is any. `None` carries no claim either way.
    #[must_use]
    pub fn reason(&self) -> Option<&VetoReason> {
        match self {
            Verdict::Vetoed { reason } => Some(reason),
            Verdict::Clear => None,
        }
    }
}

/// One completed pass over a repository, queryable per candidate.
///
/// Build it once with [`Reachability::scan`] and ask it about as many
/// candidates as you like: the expensive part is the single pass over the
/// bytes, and each query is a prefix lookup.
#[derive(Debug, Clone)]
pub struct Reachability {
    root: PathBuf,
    /// Rooted paths. An empty key is the repository root and rescues
    /// everything.
    rooted: BTreeMap<PathBuf, VetoReason>,
    /// Suffix rules, from manifest patterns whose first component is a glob
    /// (`*.onnx filter=lfs`). Keyed by the literal tail, e.g. `.onnx`.
    suffixes: BTreeMap<String, VetoReason>,
    /// Every place the scan did not complete. Non-empty means every candidate
    /// is vetoed.
    incomplete: Vec<VetoReason>,
}

impl Reachability {
    /// Walk `root` and record everything 2b and 2c can see.
    ///
    /// Infallible by construction: an I/O failure is not an error to propagate
    /// but evidence of an incomplete search, and it is recorded as a veto that
    /// applies to every candidate. There is no size cap and no timeout for the
    /// same reason — a truncated read would have to be treated as a hit, so a
    /// cap would blanket-veto the repository rather than speed anything up.
    pub fn scan(root: &Path) -> Reachability {
        let mut scan = Scan {
            root: root.to_path_buf(),
            directories: BTreeSet::new(),
            files: BTreeSet::new(),
            rooted: BTreeMap::new(),
            suffixes: BTreeMap::new(),
            incomplete: Vec::new(),
        };
        scan.walk();
        scan.analyze();

        Reachability {
            root: scan.root,
            rooted: scan.rooted,
            suffixes: scan.suffixes,
            incomplete: scan.incomplete,
        }
    }

    /// Is `candidate` rescued?
    ///
    /// `candidate` is repo-relative, or absolute inside the scanned tree. One
    /// that is neither is vetoed: the scan never looked at it, and an absence
    /// of looking is not an absence of references.
    pub fn verdict(&self, candidate: &Path) -> Verdict {
        if let Some(reason) = self.incomplete.first() {
            return Verdict::Vetoed {
                reason: reason.clone(),
            };
        }

        let Some(relative) = self.relative(candidate) else {
            return Verdict::Vetoed {
                reason: VetoReason::IncompleteRead {
                    path: candidate.to_path_buf(),
                    detail: "the candidate is outside the scanned tree, so this scan \
                             never looked at it"
                        .to_string(),
                },
            };
        };

        for ancestor in relative.ancestors() {
            if let Some(reason) = self.rooted.get(ancestor) {
                return Verdict::Vetoed {
                    reason: reason.clone(),
                };
            }
        }

        if let Some(name) = relative.file_name().and_then(|name| name.to_str()) {
            for (suffix, reason) in &self.suffixes {
                if name.ends_with(suffix.as_str()) {
                    return Verdict::Vetoed {
                        reason: reason.clone(),
                    };
                }
            }
        }

        Verdict::Clear
    }

    /// Every rooted path and the evidence that rooted it, for reporting.
    pub fn roots(&self) -> impl Iterator<Item = (&Path, &VetoReason)> {
        self.rooted
            .iter()
            .map(|(path, reason)| (path.as_path(), reason))
    }

    /// Everywhere the scan did not complete. Non-empty means every verdict is a
    /// veto and the repository's real reachability is unknown until these are
    /// resolved.
    #[must_use]
    pub fn incomplete(&self) -> &[VetoReason] {
        &self.incomplete
    }

    fn relative(&self, candidate: &Path) -> Option<PathBuf> {
        let stripped = if candidate.is_absolute() {
            candidate.strip_prefix(&self.root).ok()?
        } else {
            candidate
        };
        normalize(stripped)
    }
}

/// Normalize a relative path, resolving `.` and `..` lexically. `None` when the
/// path escapes its base or is absolute — both mean "not inside the tree we
/// scanned".
fn normalize(path: &Path) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    return None;
                }
            }
            Component::Normal(part) => out.push(part),
            Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    Some(out)
}

// ---------------------------------------------------------------------------
// The pass itself.
// ---------------------------------------------------------------------------

struct Scan {
    root: PathBuf,
    directories: BTreeSet<PathBuf>,
    files: BTreeSet<PathBuf>,
    rooted: BTreeMap<PathBuf, VetoReason>,
    suffixes: BTreeMap<String, VetoReason>,
    incomplete: Vec<VetoReason>,
}

impl Scan {
    /// Enumerate the tree. Symlinks are neither read nor descended: a symlink's
    /// content is its target path, and a *broken* one is the normal steady
    /// state for git-annex (§6.13), so treating it as an unreadable file would
    /// blanket-veto every annexed repository.
    fn walk(&mut self) {
        self.directories.insert(PathBuf::new());
        let mut pending = vec![PathBuf::new()];

        while let Some(relative) = pending.pop() {
            let entries = match std::fs::read_dir(self.root.join(&relative)) {
                Ok(entries) => entries,
                Err(error) => {
                    self.incomplete_read(&relative, format!("directory listing failed: {error}"));
                    continue;
                }
            };

            for entry in entries {
                let entry = match entry {
                    Ok(entry) => entry,
                    Err(error) => {
                        self.incomplete_read(
                            &relative,
                            format!("a directory entry could not be read: {error}"),
                        );
                        continue;
                    }
                };

                let name = entry.file_name();
                if name == SKIPPED_DIRECTORY {
                    continue;
                }
                let child = relative.join(&name);

                let file_type = match entry.file_type() {
                    Ok(file_type) => file_type,
                    Err(error) => {
                        self.incomplete_read(&child, format!("file type unreadable: {error}"));
                        continue;
                    }
                };

                if file_type.is_symlink() {
                    continue;
                }
                if file_type.is_dir() {
                    self.directories.insert(child.clone());
                    pending.push(child);
                } else if file_type.is_file() {
                    self.files.insert(child);
                }
            }
        }
    }

    /// Read every file once and hand its bytes to both halves of the gate.
    /// Files are visited in sorted order so that two constructs rooting the
    /// same directory always produce the same recorded evidence.
    fn analyze(&mut self) {
        let files: Vec<PathBuf> = self.files.iter().cloned().collect();
        for relative in &files {
            let bytes = match std::fs::read(self.root.join(relative)) {
                Ok(bytes) => bytes,
                Err(error) => {
                    self.incomplete_read(relative, format!("file unreadable: {error}"));
                    continue;
                }
            };
            self.enumeration(relative, &bytes);
            self.manifest(relative, &bytes);
        }
    }

    // -- 2c -----------------------------------------------------------------

    fn enumeration(&mut self, relative: &Path, bytes: &[u8]) {
        let Some(found) = automaton().find(bytes) else {
            return;
        };
        let construct = ENUMERATION_CONSTRUCTS[found.pattern()];

        // The enumeration target is usually a runtime value, so the directory
        // the construct lives in is rooted whether or not a literal resolves.
        let containing = relative.parent().unwrap_or(Path::new("")).to_path_buf();
        self.root_enumerated(containing, construct, relative);

        // Then every directory a string literal in the same file can name.
        // Lossy decoding is fine here: a mangled literal resolves to nothing,
        // and the containing directory is already rooted.
        let text = String::from_utf8_lossy(bytes);
        for literal in string_literals(&text) {
            for directory in self.directories_for(relative, &literal) {
                self.root_enumerated(directory, construct, relative);
            }
        }
    }

    /// Directories a literal inside `source` might name, resolved both relative
    /// to `source` and to the repository root. A literal never roots the
    /// repository root itself — `glob("*.py")` says nothing about *where*, and
    /// the containing directory already carries that uncertainty.
    fn directories_for(&self, source: &Path, literal: &str) -> Vec<PathBuf> {
        let literal = literal.trim();
        if literal.is_empty() || literal.contains("://") || literal.starts_with('/') {
            return Vec::new();
        }
        let Some(prefix) = literal_prefix(literal) else {
            return Vec::new();
        };

        let base = source.parent().unwrap_or(Path::new(""));
        let mut out = Vec::new();
        for resolved in [normalize(&base.join(&prefix)), normalize(&prefix)] {
            let Some(resolved) = resolved else { continue };
            if resolved.as_os_str().is_empty() {
                continue;
            }
            if self.directories.contains(&resolved) {
                out.push(resolved);
            } else if self.files.contains(&resolved) {
                // A named file — root the directory holding it, which is what
                // `include_str!("templates/page.html")` really tells us.
                let parent = resolved.parent().unwrap_or(Path::new(""));
                if !parent.as_os_str().is_empty() {
                    out.push(parent.to_path_buf());
                }
            }
        }
        out
    }

    // -- 2b -----------------------------------------------------------------

    fn manifest(&mut self, relative: &Path, bytes: &[u8]) {
        let Some(kind) = ManifestKind::of(relative) else {
            return;
        };
        let text = match std::str::from_utf8(bytes) {
            Ok(text) => text,
            Err(error) => {
                self.incomplete_read(relative, format!("{kind} is not valid UTF-8: {error}"));
                return;
            }
        };

        match kind {
            ManifestKind::Yaml => self.yaml(relative, text),
            ManifestKind::Dockerfile => self.dockerfile(relative, text),
            ManifestKind::DockerIgnore => self.docker_ignore(relative, text),
            ManifestKind::ManifestIn => self.manifest_in(relative, text),
            ManifestKind::PackageJson => self.package_json(relative, text),
            ManifestKind::Include => self.include_lines(relative, text),
            ManifestKind::GitAttributes => self.git_attributes(relative, text),
        }
    }

    /// CI manifests: `artifacts.paths`, `cache: paths`, `upload-artifact
    /// with: path`, and the `run:`/`script:`/`entry:` bodies §5.2 lists as
    /// roots. A YAML file that does not load has told us nothing about what it
    /// references, so it is an incomplete read, not an empty one.
    fn yaml(&mut self, relative: &Path, text: &str) {
        let values = match yaml_values(text) {
            Ok(values) => values,
            Err(defect) => {
                self.incomplete_read(relative, format!("YAML did not parse: {defect}"));
                return;
            }
        };

        for (key, value) in values {
            match key {
                YamlKey::Path => {
                    for scalar in scalars(&value) {
                        self.root_pattern(scalar, relative);
                    }
                }
                YamlKey::Command => {
                    for token in path_tokens(&value) {
                        self.root_pattern(&token, relative);
                    }
                }
            }
        }
    }

    /// `COPY`/`ADD` source paths — §5.2 puts the exclamation mark on that line
    /// itself. The destination (the last argument) is a path inside the image,
    /// not in the repository, so it is dropped.
    fn dockerfile(&mut self, relative: &Path, text: &str) {
        for instruction in dockerfile_instructions(text) {
            let Some(verb) = instruction.split_whitespace().next() else {
                continue;
            };
            if !verb.eq_ignore_ascii_case("COPY") && !verb.eq_ignore_ascii_case("ADD") {
                continue;
            }

            let rest = instruction[verb.len()..].trim();
            let mut arguments: Vec<String> = if rest.starts_with('[') {
                rest.trim_matches(|c| c == '[' || c == ']')
                    .split(',')
                    .map(|argument| argument.trim().trim_matches('"').to_string())
                    .collect()
            } else {
                rest.split_whitespace()
                    .map(std::string::ToString::to_string)
                    .collect()
            };
            arguments.retain(|argument| !argument.starts_with("--"));
            arguments.pop();

            for source in &arguments {
                self.root_pattern(source, relative);
            }
        }
    }

    /// A `.dockerignore` negation re-includes a path the ignore list just
    /// excluded, which is a statement that the build needs it.
    fn docker_ignore(&mut self, relative: &Path, text: &str) {
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with('#') {
                continue;
            }
            if let Some(pattern) = line.strip_prefix('!') {
                self.root_pattern(pattern, relative);
            }
        }
    }

    fn manifest_in(&mut self, relative: &Path, text: &str) {
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut words = line.split_whitespace();
            let Some(command) = words.next() else {
                continue;
            };
            match command {
                // Both root a directory outright.
                "graft" | "recursive-include" => {
                    if let Some(directory) = words.next() {
                        self.root_pattern(directory, relative);
                    }
                }
                "include" | "global-include" => {
                    for pattern in words {
                        self.root_pattern(pattern, relative);
                    }
                }
                _ => {}
            }
        }
    }

    fn package_json(&mut self, relative: &Path, text: &str) {
        let document: serde_json::Value = match serde_json::from_str(text) {
            Ok(document) => document,
            Err(error) => {
                self.incomplete_read(relative, format!("not valid JSON: {error}"));
                return;
            }
        };
        let Some(files) = document.get("files").and_then(serde_json::Value::as_array) else {
            return;
        };
        for entry in files {
            if let Some(pattern) = entry.as_str() {
                self.root_pattern(pattern, relative);
            }
        }
    }

    /// `pyproject.toml` / `setup.cfg` `include` keys, read textually rather
    /// than with a TOML parser: the question is only which paths are named, and
    /// a line-oriented read cannot be *wrong* about a line it does not
    /// understand — it simply names less.
    fn include_lines(&mut self, relative: &Path, text: &str) {
        let mut inside_array = false;
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with('#') {
                continue;
            }
            if inside_array {
                for literal in string_literals(line) {
                    self.root_pattern(&literal, relative);
                }
                if line.contains(']') {
                    inside_array = false;
                }
                continue;
            }
            if !line.to_ascii_lowercase().contains("include") {
                continue;
            }
            for literal in string_literals(line) {
                self.root_pattern(&literal, relative);
            }
            if line.contains('[') && !line.contains(']') {
                inside_array = true;
            }
        }
    }

    /// LFS-tracked files are a §6.12 counter-signal in their own right: the
    /// content lives outside the repository and a pointer file is all a scanner
    /// ever sees.
    fn git_attributes(&mut self, relative: &Path, text: &str) {
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') || !line.contains("filter=lfs") {
                continue;
            }
            if let Some(pattern) = line.split_whitespace().next() {
                self.root_pattern(pattern, relative);
            }
        }
    }

    // -- recording ----------------------------------------------------------

    fn root_enumerated(&mut self, rooted: PathBuf, construct: &str, found_in: &Path) {
        let reason = VetoReason::EnumeratedDirectory {
            construct: construct.to_string(),
            found_in: found_in.to_path_buf(),
            rooted: rooted.clone(),
        };
        self.rooted.entry(rooted).or_insert(reason);
    }

    fn root_manifest(&mut self, rooted: PathBuf, manifest: &Path) {
        let reason = VetoReason::ManifestPath {
            manifest: manifest.to_path_buf(),
            rooted: rooted.clone(),
        };
        self.rooted.entry(rooted).or_insert(reason);
    }

    /// Root what a manifest pattern names.
    ///
    /// The literal prefix of the pattern is rooted, so `models/*.pt` roots
    /// `models/` entirely — §6.12's "treat the entire matched directory as
    /// rooted", applied to the manifest half. A pattern whose *first* component
    /// is already a glob names no directory at all: `*.onnx` becomes a suffix
    /// rule, and anything less tractable than that (`**/*`) roots the
    /// repository, because that is what it matches.
    fn root_pattern(&mut self, pattern: &str, manifest: &Path) {
        let pattern = pattern.trim().trim_matches('"').trim_matches('\'');
        if pattern.is_empty() || pattern.contains("://") || pattern.starts_with('/') {
            return;
        }
        // `~/.cargo/registry` and `$RUNNER_TEMP/cache` name locations outside
        // the repository, so no candidate can sit under them. Rooting them
        // rescues nothing and only adds noise to the evidence.
        if pattern.starts_with('~') || pattern.contains('$') {
            return;
        }
        if pattern == "." {
            self.root_manifest(PathBuf::new(), manifest);
            return;
        }

        let mut prefix = PathBuf::new();
        let mut truncated = false;
        for component in pattern.split('/') {
            if component.is_empty() || component == "." {
                continue;
            }
            if component == ".." {
                // Escapes the repository; there is nothing here to root.
                return;
            }
            if has_glob_metacharacter(component) {
                truncated = true;
                break;
            }
            prefix.push(component);
        }

        if !prefix.as_os_str().is_empty() {
            self.root_manifest(prefix, manifest);
            return;
        }
        if !truncated {
            return;
        }

        let last = pattern.rsplit('/').next().unwrap_or_default();
        if let Some(suffix) = last.strip_prefix('*') {
            if !suffix.is_empty() && !has_glob_metacharacter(suffix) {
                let reason = VetoReason::ManifestPath {
                    manifest: manifest.to_path_buf(),
                    rooted: PathBuf::from(last),
                };
                self.suffixes.entry(suffix.to_string()).or_insert(reason);
                return;
            }
        }
        self.root_manifest(PathBuf::new(), manifest);
    }

    fn incomplete_read(&mut self, path: &Path, detail: String) {
        self.incomplete.push(VetoReason::IncompleteRead {
            path: path.to_path_buf(),
            detail,
        });
    }
}

// ---------------------------------------------------------------------------
// Manifest recognition.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ManifestKind {
    Yaml,
    Dockerfile,
    DockerIgnore,
    ManifestIn,
    PackageJson,
    Include,
    GitAttributes,
}

impl ManifestKind {
    fn of(relative: &Path) -> Option<ManifestKind> {
        let name = relative.file_name()?.to_str()?;
        let extension = relative
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or_default();

        if extension.eq_ignore_ascii_case("yml") || extension.eq_ignore_ascii_case("yaml") {
            return Some(ManifestKind::Yaml);
        }
        if name == "Dockerfile"
            || name.starts_with("Dockerfile.")
            || name.ends_with(".Dockerfile")
            || name == "Containerfile"
        {
            return Some(ManifestKind::Dockerfile);
        }
        match name {
            ".dockerignore" => Some(ManifestKind::DockerIgnore),
            "MANIFEST.in" => Some(ManifestKind::ManifestIn),
            "package.json" => Some(ManifestKind::PackageJson),
            "pyproject.toml" | "setup.cfg" => Some(ManifestKind::Include),
            ".gitattributes" => Some(ManifestKind::GitAttributes),
            _ => None,
        }
    }
}

impl fmt::Display for ManifestKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            ManifestKind::Yaml => "the YAML manifest",
            ManifestKind::Dockerfile => "the Dockerfile",
            ManifestKind::DockerIgnore => "the .dockerignore",
            ManifestKind::ManifestIn => "the MANIFEST.in",
            ManifestKind::PackageJson => "the package.json",
            ManifestKind::Include => "the packaging manifest",
            ManifestKind::GitAttributes => "the .gitattributes",
        };
        f.write_str(name)
    }
}

// ---------------------------------------------------------------------------
// Textual helpers.
// ---------------------------------------------------------------------------

fn has_glob_metacharacter(component: &str) -> bool {
    component.contains(GLOB_METACHARACTERS)
}

/// The literal leading portion of a pattern, up to its first glob component.
/// `None` when there is nothing literal to root.
fn literal_prefix(pattern: &str) -> Option<PathBuf> {
    let mut prefix = PathBuf::new();
    for component in pattern.split('/') {
        if component.is_empty() || component == "." {
            continue;
        }
        if has_glob_metacharacter(component) {
            break;
        }
        prefix.push(component);
    }
    if prefix.as_os_str().is_empty() {
        None
    } else {
        Some(prefix)
    }
}

/// Every quoted run in `text`. Deliberately language-agnostic: this is used to
/// find the directory an enumeration construct might be pointed at, and the
/// cost of a wrong guess is a directory that stays.
fn string_literals(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let characters: Vec<char> = text.chars().collect();
    let mut index = 0;

    while index < characters.len() {
        let opener = characters[index];
        index += 1;
        if opener != '\'' && opener != '"' && opener != '`' {
            continue;
        }

        let mut literal = String::new();
        let mut closed = false;
        while index < characters.len() {
            let current = characters[index];
            if current == '\n' {
                break;
            }
            index += 1;
            if current == '\\' {
                index += 1;
                continue;
            }
            if current == opener {
                closed = true;
                break;
            }
            literal.push(current);
        }
        if closed && !literal.is_empty() {
            out.push(literal);
        }
    }
    out
}

/// Whitespace-separated tokens from a command body that look like repository
/// paths. Requiring a `/` keeps this from rooting every bare word in a shell
/// script; `bash scripts/verify_release.sh` is exactly the shape it is for.
fn path_tokens(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for token in text.split_whitespace() {
        let token = token.trim_matches(|c: char| {
            c == '"' || c == '\'' || c == '(' || c == ')' || c == ';' || c == ','
        });
        if !token.contains('/') || token.contains("://") {
            continue;
        }
        if token.starts_with('-') || token.starts_with('/') || token.starts_with('$') {
            continue;
        }
        out.push(token.to_string());
    }
    out
}

/// The paths inside one collected `path:` value.
///
/// Usually exactly one, because the parser has already split a block or flow
/// sequence into its elements. The exception is a block scalar — `path: |`
/// followed by one path per line is what every cache step in the wild writes —
/// which reaches us as a single scalar with newlines still in it.
fn scalars(value: &str) -> impl Iterator<Item = &str> {
    value.lines().map(str::trim).filter(|line| !line.is_empty())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum YamlKey {
    /// A key whose value *is* a path: `path`, `paths`.
    Path,
    /// A key whose value is a command that may name paths.
    Command,
}

/// Keys whose value is a path. `artifacts.paths`, `cache: paths` and
/// `upload-artifact with: path` all land here (§5.2, CI).
const PATH_KEYS: [&str; 2] = ["path", "paths"];

/// Keys whose value is a command line. §5.2 lists `run:` bodies, GitLab
/// `script`/`before_script`, and `.pre-commit-config.yaml` `entry:`.
const COMMAND_KEYS: [&str; 9] = [
    "run",
    "script",
    "before_script",
    "after_script",
    "command",
    "entrypoint",
    "entry",
    "args",
    "cmd",
];

// The reading is done by `saphyr-parser`, the same YAML 1.2 parser §5's root
// set uses: pure Rust, no `unsafe`, and the maintained line of descent from
// `yaml-rust`. It is consumed as an event stream rather than through a value
// tree because nothing here wants YAML's type resolution — every value this
// module looks at is a path or a command line, transcribed as written.
//
// What stood here was a second hand-written scanner for the subset of YAML that
// CI manifests were assumed to use: a line-oriented `key: value` split with its
// own structural check standing in for a parser. It read a trailing comment as
// part of the path it followed, so `- dist/  # the tarball` rooted a directory
// that does not exist and left the real one unrooted; it could not see into a
// flow mapping at all; and it rooted comment *lines* inside a `paths:` block as
// though they were directories. The first copy of that defect rejected valid
// manifests in seven of the nine repositories in the out-of-sample corpus,
// which is why there is not a third.
//
// A YAML file that does not load is still an incomplete read and still vetoes
// everything (§6.20) — that rule is unchanged, and the swap only widens the set
// of defects that trip it, because a real parser rejects more than a structural
// guess could.

/// One open collection, and what it expects next.
enum Frame {
    /// A sequence. Every element is read under the key that opened it, which is
    /// how `paths:` followed by a block sequence names one path per element.
    Sequence(Option<YamlKey>),
    /// A mapping: the key kind it sits under, which its own values inherit when
    /// their key is not one this module knows, and the slot to fill next.
    Mapping {
        inherited: Option<YamlKey>,
        expects: Expects,
    },
}

/// What the node about to arrive is. Doubles as the answer to "what does the
/// node I am holding mean", since a key names nothing and only a value does.
#[derive(Clone, Copy)]
enum Expects {
    /// A mapping key.
    Key,
    /// A value, to be collected under this key kind when there is one.
    Value(Option<YamlKey>),
}

/// Every interesting key's value, in document order.
///
/// `Err` is a document that did not load, which is an incomplete read: a file
/// we could not parse has told us nothing about what it references, and reading
/// that as "references nothing" is the §6.20 inversion this module exists to
/// refuse.
fn yaml_values(text: &str) -> Result<Vec<(YamlKey, String)>, String> {
    let mut out = Vec::new();
    let mut stack: Vec<Frame> = Vec::new();

    for event in Parser::new_from_str(text) {
        let (event, span) = event.map_err(|error| yaml_defect(&error))?;
        let line = span.start.line();

        match event {
            // A stream may hold several documents; each is read the same way,
            // because a Kubernetes manifest separated by `---` names paths in
            // every one of them.
            Event::Nothing
            | Event::StreamStart
            | Event::StreamEnd
            | Event::DocumentStart(_)
            | Event::DocumentEnd => {}

            Event::Scalar(value, ..) => match expected(&stack) {
                Expects::Key => {
                    let kind = key_kind(&value.to_ascii_lowercase()).or(inherited(&stack));
                    consumed(&mut stack, Expects::Value(kind));
                }
                Expects::Value(kind) => {
                    if let Some(kind) = kind {
                        out.push((kind, value.into_owned()));
                    }
                    consumed(&mut stack, Expects::Key);
                }
            },

            Event::SequenceStart(..) | Event::MappingStart(..) => {
                let Expects::Value(kind) = expected(&stack) else {
                    return Err(format!(
                        "line {line}: a mapping key that is not a scalar is not modelled"
                    ));
                };
                consumed(&mut stack, Expects::Key);
                stack.push(match event {
                    Event::SequenceStart(..) => Frame::Sequence(kind),
                    _ => Frame::Mapping {
                        inherited: kind,
                        expects: Expects::Key,
                    },
                });
            }

            // A well-formed event stream never closes a collection that is not
            // open, or closes a sequence with a mapping's end. Checking anyway
            // costs nothing and is the difference between a wrong answer and a
            // refusal if the parser ever surprises us — and in this module a
            // wrong answer is silent, where a refusal vetoes.
            Event::SequenceEnd | Event::MappingEnd => {
                let closed = matches!(
                    (stack.pop(), &event),
                    (Some(Frame::Sequence(_)), Event::SequenceEnd)
                        | (Some(Frame::Mapping { .. }), Event::MappingEnd)
                );
                if !closed {
                    return Err(format!(
                        "line {line}: a collection ended where none like it was open"
                    ));
                }
            }

            // An alias is a reference to content written elsewhere in the same
            // document, and that elsewhere is visited in its own right — so an
            // alias in a position this module ignores costs nothing. One
            // standing where a path or a command belongs is different: the
            // value is real and we did not read it, which is exactly an
            // incomplete read.
            Event::Alias(_) => match expected(&stack) {
                Expects::Value(None) => consumed(&mut stack, Expects::Key),
                Expects::Value(Some(_)) | Expects::Key => {
                    return Err(format!(
                        "line {line}: an alias stands where a path or a command was expected, \
                         and this module does not resolve anchors"
                    ))
                }
            },
        }
    }

    Ok(out)
}

/// What the next node to arrive will be. A node at the top of the stream is a
/// value — the document itself — under no key at all.
fn expected(stack: &[Frame]) -> Expects {
    match stack.last() {
        None => Expects::Value(None),
        Some(Frame::Sequence(kind)) => Expects::Value(*kind),
        Some(Frame::Mapping { expects, .. }) => *expects,
    }
}

/// The key kind a mapping's values inherit when their own key is not one this
/// module knows, so everything under `paths:` is read as a path however deeply
/// it is nested.
fn inherited(stack: &[Frame]) -> Option<YamlKey> {
    match stack.last() {
        Some(Frame::Mapping { inherited, .. }) => *inherited,
        _ => None,
    }
}

/// Record that the node the innermost mapping was waiting for has arrived, and
/// say what it expects instead. A sequence expects the same thing forever, so
/// for one this is a no-op.
fn consumed(stack: &mut [Frame], next: Expects) {
    if let Some(Frame::Mapping { expects, .. }) = stack.last_mut() {
        *expects = next;
    }
}

fn key_kind(key: &str) -> Option<YamlKey> {
    if PATH_KEYS.contains(&key) {
        Some(YamlKey::Path)
    } else if COMMAND_KEYS.contains(&key) {
        Some(YamlKey::Command)
    } else {
        None
    }
}

/// What the parser refused, with the line it refused on. §6.20 asks for a
/// message a human can act on, and the scanner's own wording is already that:
/// "tabs disallowed within this context", "while parsing a flow sequence,
/// expected ',' or ']'".
fn yaml_defect(error: &ScanError) -> String {
    format!("line {}: {}", error.marker().line(), error.info())
}

/// Dockerfile instructions with line continuations joined and comments
/// dropped, so a `COPY` split over three lines is read as one.
fn dockerfile_instructions(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut pending = String::new();

    for line in text.lines() {
        let line = line.trim_end();
        if line.trim_start().starts_with('#') {
            continue;
        }
        if let Some(head) = line.strip_suffix('\\') {
            pending.push_str(head);
            pending.push(' ');
            continue;
        }
        pending.push_str(line);
        let instruction = pending.trim().to_string();
        if !instruction.is_empty() {
            out.push(instruction);
        }
        pending.clear();
    }
    let instruction = pending.trim().to_string();
    if !instruction.is_empty() {
        out.push(instruction);
    }
    out
}
