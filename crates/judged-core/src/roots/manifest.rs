//! Tier A — the manifests a build system or deploy target already reads (§5.2).
//!
//! This is the only one of the three tiers that can be trusted without a
//! qualifier, and the reason is narrow: nothing here is inferred. A
//! `package.json` `bin` map is not evidence that a file is an entry point, it is
//! a *declaration* that npm will install it as one. §5.1 rates the tier "high
//! confidence; auto-discoverable" for exactly that reason — we are reading the
//! same bytes the build system reads, and agreeing with it.
//!
//! So the job is transcription, not analysis. Every root this module emits
//! carries [`Tier::A`], the file it came from, and **the exact key inside that
//! file**, because §9.13 asks for `-printseeds` output a human can audit:
//! *"package.json declares this"* is not auditable, `package.json#exports./client`
//! is.
//!
//! # A manifest that would not parse is an error
//!
//! The one rule that outranks everything else here. A malformed `package.json`
//! has told us *nothing* about the roots of that project, and reporting it as
//! "declares no roots" would make every entry point in that workspace a
//! deletion candidate. That is §6.20 in a new costume — the failure mode where
//! a tool's own breakage "presents as clean output" — and it is why
//! [`ManifestError`] exists and why no parser in this file has a lenient path.
//! "Parsed, and it declares nothing" ([`ManifestRoots::is_empty`]) and "could
//! not parse" (an `Err`) are different answers and must stay different.
//!
//! # Two declarations that are not roots
//!
//! §5.2 flags two manifest keys that change what a downstream tier may
//! *conclude* rather than adding a root, and both are recorded as
//! [`Declaration`]s:
//!
//! - `package.json` `sideEffects: false` is the inverse of a root — an explicit
//!   statement that a bundler may drop modules nothing imports.
//! - `Cargo.toml` `crate-type = ["cdylib"]` or `["staticlib"]` means the
//!   consumer is outside the crate graph entirely, so "no callers in this
//!   workspace" stops being evidence of anything.
//!
//! # Scope
//!
//! §5.2 lists on the order of two hundred sources, and §11 R2 warns that the
//! registry is either a moat or an unbounded liability. This module covers the
//! six families the corpora actually contain — npm, Python packaging, Cargo,
//! Go, Docker and GitHub Actions — and stops. Anything not listed here is not
//! silently mis-parsed; it is simply not read, and [`scan`] reports which files
//! it did read.

use std::collections::BTreeSet;
use std::fmt;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// provenance
// ---------------------------------------------------------------------------

/// Which of the three §5.1 tiers a root came from.
///
/// Carried on every [`Root`] rather than implied by the module that produced
/// it. A root that does not say which tier it came from is worse than no root:
/// it invites a caller to trust a guessed framework convention as though a
/// manifest had declared it.
///
/// Everything in this module is [`Tier::A`] by construction — it is all read
/// out of a file some other tool already reads for the same purpose. The other
/// two variants exist so that a root set assembled from all three tiers is one
/// homogeneous list a human can read top to bottom.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Tier {
    /// A build system or deploy target already reads this file to find roots.
    A,
    /// A framework's file layout or annotations make a file an entry point with
    /// no source reference anywhere. Correct only if the framework *and its
    /// version* were detected correctly.
    B,
    /// The live set is determined by data or intent outside the repository.
    /// Must be solicited from a human; no static analysis produces it.
    C,
}

impl Tier {
    /// The single-letter tag used in `-printseeds` output.
    pub fn as_str(self) -> &'static str {
        match self {
            Tier::A => "A",
            Tier::B => "B",
            Tier::C => "C",
        }
    }
}

impl fmt::Display for Tier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What kind of root this is — enough for a human scanning `-printseeds` output
/// to see the shape of a repository's entry surface without reading every key.
///
/// The precise provenance is always in the [`Origin`]; this is the grouping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RootKind {
    /// A program something executes: `bin`, `[[bin]]`, `console_scripts`,
    /// `src/main.rs`, a Go `package main`, `manage.py`.
    Executable,
    /// A surface consumed from outside this package: `main`/`module`/`exports`,
    /// `[lib]`, `src/lib.rs`, `wsgi.py`.
    LibraryEntry,
    /// A test, example or benchmark target: `[[test]]`, `[[example]]`,
    /// `[[bench]]`, `conftest.py`.
    DevTarget,
    /// Code that runs as part of building: `build.rs`.
    BuildHook,
    /// A command line a build, CI or deploy step runs verbatim.
    Command,
    /// A file or directory a packaging or image step ships: `files`, `COPY`.
    PackagedFile,
    /// Another package in this repository the workspace declares.
    WorkspaceMember,
    /// A registration an external framework resolves at runtime:
    /// `[project.entry-points.<group>]`.
    PluginEntryPoint,
    /// A container image's `CMD` or `ENTRYPOINT`.
    ContainerEntry,
    /// A CI action a workflow step invokes with `uses:`.
    CiAction,
}

impl RootKind {
    /// The lowercase tag used in `-printseeds` output.
    pub fn as_str(self) -> &'static str {
        match self {
            RootKind::Executable => "executable",
            RootKind::LibraryEntry => "library_entry",
            RootKind::DevTarget => "dev_target",
            RootKind::BuildHook => "build_hook",
            RootKind::Command => "command",
            RootKind::PackagedFile => "packaged_file",
            RootKind::WorkspaceMember => "workspace_member",
            RootKind::PluginEntryPoint => "plugin_entry_point",
            RootKind::ContainerEntry => "container_entry",
            RootKind::CiAction => "ci_action",
        }
    }
}

impl fmt::Display for RootKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What a root actually points at.
///
/// The variants are kept apart because they license different follow-up
/// questions. A [`RootTarget::Path`] can be stat-ed; a [`RootTarget::Glob`]
/// cannot, and a pattern matching nothing today is still a declaration. A
/// [`RootTarget::Command`] is a shell string that may name a binary on `PATH`
/// rather than a file in this repository, and treating it as a path is how a
/// cleaner ends up "resolving" `npm run build` to a file called `npm`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RootTarget {
    /// A repo-relative path the manifest names outright, normalized lexically.
    Path(PathBuf),
    /// A repo-relative pattern. Expanding it is a filesystem question this
    /// module deliberately does not answer.
    Glob(String),
    /// A command line, run verbatim by a build, CI or deploy step.
    Command(String),
    /// A name something other than a path lookup resolves: a Python object
    /// reference (`pkg.mod:main`), a Go module path, an action ref
    /// (`owner/repo@v4`), or a build-system target Cargo locates by
    /// auto-discovery. Recording the name rather than inventing the file it
    /// probably resolves to keeps a declaration a declaration.
    Reference(String),
}

impl fmt::Display for RootTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RootTarget::Path(p) => write!(f, "{}", p.display()),
            RootTarget::Glob(s) | RootTarget::Command(s) | RootTarget::Reference(s) => {
                f.write_str(s)
            }
        }
    }
}

/// Where a root was declared: the exact file, and the exact key inside it.
///
/// Renders as `<file>#<key>` — `package.json#exports./client`,
/// `Cargo.toml#bin[1].path`, `.github/workflows/ci.yml#jobs.build.steps[2].run`.
/// §9.13 wants show-roots output a human can check against the repository, and
/// a key that only names the file is not checkable.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Origin {
    file: PathBuf,
    key: String,
}

impl Origin {
    /// Build an origin from a repo-relative file and a key path within it.
    pub fn new(file: impl Into<PathBuf>, key: impl Into<String>) -> Origin {
        Origin {
            file: file.into(),
            key: key.into(),
        }
    }

    /// The manifest, repo-relative.
    pub fn file(&self) -> &Path {
        &self.file
    }

    /// The key path inside the manifest.
    pub fn key(&self) -> &str {
        &self.key
    }
}

impl fmt::Display for Origin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}#{}", self.file.display(), self.key)
    }
}

/// One declared root: what it is, where it points, and who said so.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Root {
    tier: Tier,
    kind: RootKind,
    origin: Origin,
    target: RootTarget,
}

impl Root {
    /// Build a root. Public so that the other two tiers can produce the same
    /// shape; the tier is an explicit argument precisely so nothing can emit a
    /// root without stating where its confidence comes from.
    pub fn new(tier: Tier, kind: RootKind, origin: Origin, target: RootTarget) -> Root {
        Root {
            tier,
            kind,
            origin,
            target,
        }
    }

    /// Which §5.1 tier this root's confidence comes from.
    pub fn tier(&self) -> Tier {
        self.tier
    }

    /// The grouping this root falls into.
    pub fn kind(&self) -> RootKind {
        self.kind
    }

    /// The file and key that declared it.
    pub fn origin(&self) -> &Origin {
        &self.origin
    }

    /// What it points at.
    pub fn target(&self) -> &RootTarget {
        &self.target
    }
}

/// A manifest statement that changes what a downstream tier may conclude,
/// without itself naming a root.
///
/// §5.2 flags both of these, and they matter in opposite directions: one widens
/// what may be deleted, the other says the evidence for deleting is missing.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Declaration {
    /// `package.json` `"sideEffects": false` — the package declares that a
    /// bundler may drop any module nothing imports. The inverse of a root.
    TreeShakable { origin: Origin },
    /// `package.json` `"sideEffects": [...]` — only the listed modules have
    /// side effects (each is also emitted as a root, per §5.2); everything else
    /// is declared droppable.
    TreeShakableExcept { origin: Origin, globs: Vec<String> },
    /// `Cargo.toml` `crate-type` naming `cdylib` or `staticlib`: the consumer
    /// is outside the crate graph entirely, so "nothing in this workspace calls
    /// it" is not evidence about this target.
    ConsumerOutsideBuildGraph { origin: Origin, crate_type: String },
}

impl Declaration {
    /// The file and key that declared it.
    pub fn origin(&self) -> &Origin {
        match self {
            Declaration::TreeShakable { origin }
            | Declaration::TreeShakableExcept { origin, .. }
            | Declaration::ConsumerOutsideBuildGraph { origin, .. } => origin,
        }
    }
}

// ---------------------------------------------------------------------------
// the materialized set
// ---------------------------------------------------------------------------

/// Everything one or more manifests declared.
///
/// Fields are private and there is no public struct literal, so a root set
/// cannot be fabricated without a parser having produced it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ManifestRoots {
    roots: Vec<Root>,
    declarations: Vec<Declaration>,
    sources: Vec<PathBuf>,
}

impl ManifestRoots {
    /// The roots, in the order the parsers emitted them.
    pub fn roots(&self) -> &[Root] {
        &self.roots
    }

    /// The non-root declarations (§5.2's `sideEffects`, `crate-type`).
    pub fn declarations(&self) -> &[Declaration] {
        &self.declarations
    }

    /// Every manifest that was successfully read to build this set.
    ///
    /// A caller that wants to know whether the answer is thin because the
    /// repository is thin, or because nothing was read, asks this — the same
    /// "no data is not zero" distinction §6.20 turns on.
    pub fn sources(&self) -> &[PathBuf] {
        &self.sources
    }

    /// True when nothing was declared. Note that this is a *parsed* answer: a
    /// manifest that failed to parse never produces a `ManifestRoots` at all.
    pub fn is_empty(&self) -> bool {
        self.roots.is_empty() && self.declarations.is_empty()
    }

    /// `-printseeds` (§9.13): one tab-separated line per root, in order —
    /// tier, kind, origin, target.
    ///
    /// Targets are escaped, because some of them are not one line. A CI `run:`
    /// body is a shell script, and letting its newlines through would break the
    /// one invariant a line-oriented report has: the reader could no longer
    /// tell a second root from the second line of the first one.
    pub fn printseeds(&self) -> String {
        let mut out = String::new();
        for root in &self.roots {
            out.push_str(root.tier.as_str());
            out.push('\t');
            out.push_str(root.kind.as_str());
            out.push('\t');
            push_escaped(&mut out, &root.origin.to_string());
            out.push('\t');
            push_escaped(&mut out, &root.target.to_string());
            out.push('\n');
        }
        out
    }

    /// Fold another manifest's roots into this set.
    pub fn merge(&mut self, other: ManifestRoots) {
        self.roots.extend(other.roots);
        self.declarations.extend(other.declarations);
        self.sources.extend(other.sources);
    }

    fn from_source(file: &Path) -> ManifestRoots {
        ManifestRoots {
            roots: Vec::new(),
            declarations: Vec::new(),
            sources: vec![file.to_path_buf()],
        }
    }

    /// Everything a manifest declares is [`Tier::A`] by construction — that is
    /// what makes it a manifest rather than a convention.
    fn push(&mut self, kind: RootKind, file: &Path, key: String, target: RootTarget) {
        self.roots
            .push(Root::new(Tier::A, kind, Origin::new(file, key), target));
    }
}

/// Write `text` with the characters that would break a tab-separated line
/// spelled out, so every root stays on exactly one row.
fn push_escaped(out: &mut String, text: &str) {
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(c),
        }
    }
}

// ---------------------------------------------------------------------------
// failure
// ---------------------------------------------------------------------------

/// A manifest could not be read, or could not be understood.
///
/// Deliberately not a variant of [`crate::Error`]: to be actionable a manifest
/// failure has to carry the file, the key path that was being read, and the
/// line — a caller who is told only "parse error" cannot fix the manifest, and
/// a caller who is told nothing at all deletes the repository (§6.20).
#[derive(Debug)]
pub enum ManifestError {
    /// The manifest exists but could not be read off disk.
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    /// The manifest was read but not understood. `key` and `line` are filled in
    /// whenever the parser knows where it was.
    Parse {
        path: PathBuf,
        key: Option<String>,
        line: Option<usize>,
        detail: String,
    },
}

impl ManifestError {
    /// The manifest that failed.
    pub fn path(&self) -> &Path {
        match self {
            ManifestError::Read { path, .. } | ManifestError::Parse { path, .. } => path,
        }
    }

    fn parse(path: &Path, detail: impl Into<String>) -> ManifestError {
        ManifestError::Parse {
            path: path.to_path_buf(),
            key: None,
            line: None,
            detail: detail.into(),
        }
    }

    fn at_key(path: &Path, key: impl Into<String>, detail: impl Into<String>) -> ManifestError {
        ManifestError::Parse {
            path: path.to_path_buf(),
            key: Some(key.into()),
            line: None,
            detail: detail.into(),
        }
    }

    fn at_line(path: &Path, line: usize, detail: impl Into<String>) -> ManifestError {
        ManifestError::Parse {
            path: path.to_path_buf(),
            key: None,
            line: Some(line),
            detail: detail.into(),
        }
    }
}

impl fmt::Display for ManifestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ManifestError::Read { path, source } => {
                write!(f, "could not read manifest {}: {source}", path.display())
            }
            ManifestError::Parse {
                path,
                key,
                line,
                detail,
            } => {
                write!(f, "{}", path.display())?;
                if let Some(key) = key {
                    write!(f, "#{key}")?;
                }
                if let Some(line) = line {
                    write!(f, ":{line}")?;
                }
                write!(f, ": {detail}")
            }
        }
    }
}

impl std::error::Error for ManifestError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ManifestError::Read { source, .. } => Some(source),
            ManifestError::Parse { .. } => None,
        }
    }
}

/// Result of reading a manifest.
pub type ManifestResult<T> = std::result::Result<T, ManifestError>;

// ---------------------------------------------------------------------------
// key paths and path resolution
// ---------------------------------------------------------------------------

/// Append one segment to a key path.
///
/// Segments are joined with `.`, and the separator is elided when the segment
/// already begins with `.` or `[` — so an npm subpath export renders as
/// `exports./client` and an array element as `workspaces[0]`, which is what a
/// human checking the manifest expects to see.
fn key_join(prefix: &str, segment: &str) -> String {
    if prefix.is_empty() {
        segment.to_string()
    } else if segment.starts_with('.') || segment.starts_with('[') {
        format!("{prefix}{segment}")
    } else {
        format!("{prefix}.{segment}")
    }
}

fn key_index(prefix: &str, index: usize) -> String {
    key_join(prefix, &format!("[{index}]"))
}

/// The directory a manifest sits in, repo-relative (`""` at the repo root).
fn manifest_dir(manifest: &Path) -> &Path {
    manifest.parent().unwrap_or_else(|| Path::new(""))
}

/// Resolve a manifest-relative reference to a repo-relative path, lexically.
///
/// No filesystem access: a declared root that does not exist yet is still a
/// declaration, and resolving symlinks here would silently rewrite what the
/// manifest said. `..` is popped when there is something to pop and otherwise
/// kept, so a path escaping the repo root stays visibly escaped.
fn join_rel(dir: &Path, value: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    let dir_str = dir.to_string_lossy().into_owned();
    for segment in dir_str.split('/').chain(value.split('/')) {
        match segment {
            "" | "." => {}
            ".." => {
                if matches!(parts.last(), Some(&last) if last != "..") {
                    parts.pop();
                } else {
                    parts.push("..");
                }
            }
            other => parts.push(other),
        }
    }
    parts.join("/")
}

fn rel_path(dir: &Path, value: &str) -> RootTarget {
    RootTarget::Path(PathBuf::from(join_rel(dir, value)))
}

fn rel_glob(dir: &Path, value: &str) -> RootTarget {
    RootTarget::Glob(join_rel(dir, value))
}

// ---------------------------------------------------------------------------
// package.json (§5.2, JS/TS)
// ---------------------------------------------------------------------------

/// Parse a `package.json` into the roots npm, bundlers and Node already read
/// out of it.
///
/// Covers every key §5.2 lists: `main`, `module`, `browser`, `types`, `bin`
/// (string or map), every leaf of the nested conditional `exports` map,
/// `imports`, `workspaces`, `files`, `scripts`, and `sideEffects`.
///
/// `manifest` is the repo-relative path of the file, and every path it declares
/// is resolved against that file's directory — a `"main": "./src/index.js"` in
/// `packages/ui/package.json` is the root `packages/ui/src/index.js`.
///
/// A key whose value has the wrong JSON type is an error, not a skipped key:
/// `"main": 42` means we do not know what this package's entry point is, and
/// saying nothing about it is the §6.20 failure.
///
/// One deliberate omission: in a `browser` *map*, only the replacement (the
/// value) is emitted as a root. The map's key names a module this package's own
/// code imports — that is what makes the replacement meaningful — so it has an
/// in-source reference by construction and does not need declaring.
pub fn parse_package_json(manifest: &Path, content: &str) -> ManifestResult<ManifestRoots> {
    let json: serde_json::Value =
        serde_json::from_str(content).map_err(|e| ManifestError::Parse {
            path: manifest.to_path_buf(),
            key: None,
            line: Some(e.line()),
            detail: format!("not valid JSON: {e}"),
        })?;
    let object = json.as_object().ok_or_else(|| {
        ManifestError::parse(
            manifest,
            "the top level of a package.json must be an object",
        )
    })?;

    let dir = manifest_dir(manifest);
    let mut out = ManifestRoots::from_source(manifest);

    for key in ["main", "module", "types"] {
        if let Some(value) = object.get(key) {
            let text = expect_str(manifest, key, value)?;
            out.push(
                RootKind::LibraryEntry,
                manifest,
                key.to_string(),
                rel_path(dir, text),
            );
        }
    }

    if let Some(value) = object.get("browser") {
        match value {
            serde_json::Value::String(s) => {
                out.push(
                    RootKind::LibraryEntry,
                    manifest,
                    "browser".into(),
                    rel_path(dir, s),
                );
            }
            serde_json::Value::Object(map) => {
                for (spec, replacement) in map {
                    let key = key_join("browser", spec);
                    match replacement {
                        // `"fs": false` stubs a module out. It names no file.
                        serde_json::Value::Bool(false) => {}
                        serde_json::Value::String(s) => {
                            out.push(RootKind::LibraryEntry, manifest, key, rel_path(dir, s));
                        }
                        _ => {
                            return Err(ManifestError::at_key(
                                manifest,
                                key,
                                "a browser replacement must be a path or false",
                            ))
                        }
                    }
                }
            }
            _ => {
                return Err(ManifestError::at_key(
                    manifest,
                    "browser",
                    "must be a path or a replacement map",
                ))
            }
        }
    }

    if let Some(value) = object.get("bin") {
        match value {
            serde_json::Value::String(s) => {
                out.push(
                    RootKind::Executable,
                    manifest,
                    "bin".into(),
                    rel_path(dir, s),
                );
            }
            serde_json::Value::Object(map) => {
                for (name, path) in map {
                    let key = key_join("bin", name);
                    let text = expect_str(manifest, &key, path)?;
                    out.push(RootKind::Executable, manifest, key, rel_path(dir, text));
                }
            }
            _ => {
                return Err(ManifestError::at_key(
                    manifest,
                    "bin",
                    "must be a path or a map of command name to path",
                ))
            }
        }
    }

    for key in ["exports", "imports"] {
        if let Some(value) = object.get(key) {
            walk_export_map(manifest, dir, key, value, &mut out)?;
        }
    }

    if let Some(value) = object.get("workspaces") {
        match value {
            serde_json::Value::Array(items) => {
                push_glob_list(
                    manifest,
                    dir,
                    "workspaces",
                    items,
                    RootKind::WorkspaceMember,
                    &mut out,
                )?;
            }
            serde_json::Value::Object(map) => {
                let packages = map.get("packages").ok_or_else(|| {
                    ManifestError::at_key(
                        manifest,
                        "workspaces",
                        "object form must have a `packages` array",
                    )
                })?;
                let items = packages.as_array().ok_or_else(|| {
                    ManifestError::at_key(manifest, "workspaces.packages", "must be an array")
                })?;
                push_glob_list(
                    manifest,
                    dir,
                    "workspaces.packages",
                    items,
                    RootKind::WorkspaceMember,
                    &mut out,
                )?;
            }
            _ => {
                return Err(ManifestError::at_key(
                    manifest,
                    "workspaces",
                    "must be an array of patterns or an object with `packages`",
                ))
            }
        }
    }

    if let Some(value) = object.get("files") {
        let items = value.as_array().ok_or_else(|| {
            ManifestError::at_key(manifest, "files", "must be an array of patterns")
        })?;
        push_glob_list(
            manifest,
            dir,
            "files",
            items,
            RootKind::PackagedFile,
            &mut out,
        )?;
    }

    if let Some(value) = object.get("scripts") {
        let map = value
            .as_object()
            .ok_or_else(|| ManifestError::at_key(manifest, "scripts", "must be an object"))?;
        for (name, command) in map {
            let key = key_join("scripts", name);
            let text = expect_str(manifest, &key, command)?;
            out.push(
                RootKind::Command,
                manifest,
                key,
                RootTarget::Command(text.to_string()),
            );
        }
    }

    if let Some(value) = object.get("sideEffects") {
        let origin = Origin::new(manifest, "sideEffects");
        match value {
            // The inverse of a root (§5.2).
            serde_json::Value::Bool(false) => {
                out.declarations.push(Declaration::TreeShakable { origin });
            }
            // `true` is the default: nothing is declared either way.
            serde_json::Value::Bool(true) => {}
            serde_json::Value::Array(items) => {
                let mut globs = Vec::with_capacity(items.len());
                for (index, item) in items.iter().enumerate() {
                    let key = key_index("sideEffects", index);
                    let text = expect_str(manifest, &key, item)?;
                    let glob = join_rel(dir, text);
                    out.push(
                        RootKind::LibraryEntry,
                        manifest,
                        key,
                        RootTarget::Glob(glob.clone()),
                    );
                    globs.push(glob);
                }
                out.declarations
                    .push(Declaration::TreeShakableExcept { origin, globs });
            }
            _ => {
                return Err(ManifestError::at_key(
                    manifest,
                    "sideEffects",
                    "must be a boolean or an array of patterns",
                ))
            }
        }
    }

    Ok(out)
}

/// Walk a conditional `exports`/`imports` map to its leaves.
///
/// The map nests arbitrarily — subpath, then condition, then possibly more
/// conditions — and every leaf is a separate entry point, so the key path has
/// to record the whole route to it. A `null` leaf blocks a subpath: it declares
/// the *absence* of an export and names no file.
fn walk_export_map(
    manifest: &Path,
    dir: &Path,
    key: &str,
    value: &serde_json::Value,
    out: &mut ManifestRoots,
) -> ManifestResult<()> {
    match value {
        serde_json::Value::String(s) => {
            out.push(
                RootKind::LibraryEntry,
                manifest,
                key.to_string(),
                rel_path(dir, s),
            );
        }
        serde_json::Value::Null => {}
        serde_json::Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                walk_export_map(manifest, dir, &key_index(key, index), item, out)?;
            }
        }
        serde_json::Value::Object(map) => {
            for (segment, item) in map {
                walk_export_map(manifest, dir, &key_join(key, segment), item, out)?;
            }
        }
        _ => {
            return Err(ManifestError::at_key(
                manifest,
                key,
                "an exports leaf must be a path, a condition map, a fallback array, or null",
            ))
        }
    }
    Ok(())
}

fn push_glob_list(
    manifest: &Path,
    dir: &Path,
    key: &str,
    items: &[serde_json::Value],
    kind: RootKind,
    out: &mut ManifestRoots,
) -> ManifestResult<()> {
    for (index, item) in items.iter().enumerate() {
        let item_key = key_index(key, index);
        let text = expect_str(manifest, &item_key, item)?;
        out.push(kind, manifest, item_key, rel_glob(dir, text));
    }
    Ok(())
}

fn expect_str<'a>(
    manifest: &Path,
    key: &str,
    value: &'a serde_json::Value,
) -> ManifestResult<&'a str> {
    value.as_str().ok_or_else(|| {
        ManifestError::at_key(
            manifest,
            key,
            format!("must be a string, found {}", json_type_name(value)),
        )
    })
}

fn json_type_name(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "a boolean",
        serde_json::Value::Number(_) => "a number",
        serde_json::Value::String(_) => "a string",
        serde_json::Value::Array(_) => "an array",
        serde_json::Value::Object(_) => "an object",
    }
}

// ---------------------------------------------------------------------------
// TOML
// ---------------------------------------------------------------------------
//
// `pyproject.toml` and `Cargo.toml` are the two manifests §5.2 puts in the Tier
// A checklist that are written in TOML, and both are read here for a handful of
// tables. What follows is a parser for the subset that reaches: tables, arrays
// of tables, dotted and quoted keys, the four string forms, arrays, inline
// tables, booleans, and comments.
//
// It is deliberately *strict* rather than lenient. A lenient parser that
// shrugged at a construct it did not know would return a partial table, and a
// partial table is indistinguishable from a manifest that declares nothing —
// the §6.20 failure this whole module is built to avoid. Anything it cannot
// account for is an error carrying the line it gave up on. Numbers and
// datetimes are kept verbatim as [`Toml::Bare`] and never interpreted, because
// no key this module reads is a number.

/// A TOML value, in the subset the Tier A manifests need.
#[derive(Debug, Clone, PartialEq)]
enum Toml {
    Str(String),
    Bool(bool),
    /// An integer, float or datetime, unparsed. Kept so that a manifest
    /// containing one is not an error, and never read as anything else.
    Bare(String),
    Array(Vec<Toml>),
    /// Ordered so that `-printseeds` output follows the manifest, not the
    /// alphabet — a human checking a root against the file reads top to bottom.
    Table(Vec<(String, Toml)>),
}

impl Toml {
    fn as_str(&self) -> Option<&str> {
        match self {
            Toml::Str(s) => Some(s),
            _ => None,
        }
    }

    fn as_array(&self) -> Option<&[Toml]> {
        match self {
            Toml::Array(items) => Some(items),
            _ => None,
        }
    }

    fn as_table(&self) -> Option<&[(String, Toml)]> {
        match self {
            Toml::Table(entries) => Some(entries),
            _ => None,
        }
    }

    fn get(&self, key: &str) -> Option<&Toml> {
        self.as_table()?
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v)
    }

    fn get_path(&self, path: &[&str]) -> Option<&Toml> {
        let mut node = self;
        for key in path {
            node = node.get(key)?;
        }
        Some(node)
    }

    fn type_name(&self) -> &'static str {
        match self {
            Toml::Str(_) => "a string",
            Toml::Bool(_) => "a boolean",
            Toml::Bare(_) => "a number or datetime",
            Toml::Array(_) => "an array",
            Toml::Table(_) => "a table",
        }
    }
}

/// One step of the "current table" path: a key, or an index into an array of
/// tables produced by a `[[header]]`.
enum Step {
    Key(String),
    Index(usize),
}

struct TomlParser<'a> {
    path: &'a Path,
    bytes: &'a [u8],
    pos: usize,
    line: usize,
}

impl<'a> TomlParser<'a> {
    fn new(path: &'a Path, content: &'a str) -> TomlParser<'a> {
        TomlParser {
            path,
            bytes: content.as_bytes(),
            pos: 0,
            line: 1,
        }
    }

    fn err<T>(&self, detail: impl Into<String>) -> ManifestResult<T> {
        Err(ManifestError::at_line(self.path, self.line, detail))
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn bump(&mut self) -> Option<u8> {
        let byte = self.peek()?;
        self.pos += 1;
        if byte == b'\n' {
            self.line += 1;
        }
        Some(byte)
    }

    fn eat(&mut self, byte: u8) -> bool {
        if self.peek() == Some(byte) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn starts_with(&self, s: &str) -> bool {
        self.bytes[self.pos..].starts_with(s.as_bytes())
    }

    /// Spaces and tabs only. Newlines are structural at the top level.
    fn skip_inline_space(&mut self) {
        while matches!(self.peek(), Some(b' ') | Some(b'\t') | Some(b'\r')) {
            self.bump();
        }
    }

    fn skip_comment(&mut self) {
        if self.peek() == Some(b'#') {
            while !matches!(self.peek(), None | Some(b'\n')) {
                self.bump();
            }
        }
    }

    /// Everything that may appear between two structural tokens at the top
    /// level: blank lines, indentation, comments.
    fn skip_trivia(&mut self) {
        loop {
            self.skip_inline_space();
            self.skip_comment();
            if self.peek() == Some(b'\n') {
                self.bump();
            } else {
                return;
            }
        }
    }

    /// Nothing but whitespace and a comment may follow a value on its line.
    fn expect_line_end(&mut self) -> ManifestResult<()> {
        self.skip_inline_space();
        self.skip_comment();
        match self.peek() {
            None => Ok(()),
            Some(b'\n') => {
                self.bump();
                Ok(())
            }
            Some(byte) => self.err(format!("unexpected {:?} after a value", byte as char)),
        }
    }

    fn parse_document(&mut self) -> ManifestResult<Toml> {
        let mut root = Toml::Table(Vec::new());
        let mut current: Vec<Step> = Vec::new();

        loop {
            self.skip_trivia();
            match self.peek() {
                None => return Ok(root),
                Some(b'[') => current = self.parse_header(&mut root)?,
                _ => {
                    let key = self.parse_dotted_key()?;
                    self.skip_inline_space();
                    if !self.eat(b'=') {
                        return self.err("expected `=` after a key");
                    }
                    self.skip_inline_space();
                    let value = self.parse_value()?;
                    self.expect_line_end()?;
                    let table = resolve_step_path(&mut root, &current).ok_or_else(|| {
                        ManifestError::at_line(
                            self.path,
                            self.line,
                            "internal: lost the current table",
                        )
                    })?;
                    self.insert(table, &key, value)?;
                }
            }
        }
    }

    /// `[a.b]` or `[[a.b]]`, returning the new current-table path.
    fn parse_header(&mut self, root: &mut Toml) -> ManifestResult<Vec<Step>> {
        let array = self.starts_with("[[");
        self.bump();
        if array {
            self.bump();
        }
        self.skip_inline_space();
        let key = self.parse_dotted_key()?;
        self.skip_inline_space();
        if !self.eat(b']') || (array && !self.eat(b']')) {
            return self.err("unterminated table header");
        }
        self.expect_line_end()?;

        let mut steps: Vec<Step> = Vec::new();
        for (index, segment) in key.iter().enumerate() {
            let last = index + 1 == key.len();
            let node = resolve_step_path(root, &steps).ok_or_else(|| {
                ManifestError::at_line(self.path, self.line, "internal: lost the current table")
            })?;
            let entries = match node {
                Toml::Table(entries) => entries,
                other => {
                    return self.err(format!(
                        "`{segment}` is declared inside {}, which is not a table",
                        other.type_name()
                    ))
                }
            };
            if !entries.iter().any(|(k, _)| k == segment) {
                let empty = if last && array {
                    Toml::Array(Vec::new())
                } else {
                    Toml::Table(Vec::new())
                };
                entries.push((segment.clone(), empty));
            }
            steps.push(Step::Key(segment.clone()));

            if last && array {
                let node = resolve_step_path(root, &steps).ok_or_else(|| {
                    ManifestError::at_line(self.path, self.line, "internal: lost the current table")
                })?;
                let Toml::Array(items) = node else {
                    return self.err(format!("`{segment}` was already declared as a table, so `[[{segment}]]` cannot extend it"));
                };
                items.push(Toml::Table(Vec::new()));
                steps.push(Step::Index(items.len() - 1));
            } else {
                // A `[[a]]` earlier in the file means later `[a.b]` headers
                // address the *last* element of the array (TOML 1.0).
                let node = resolve_step_path(root, &steps).ok_or_else(|| {
                    ManifestError::at_line(self.path, self.line, "internal: lost the current table")
                })?;
                if let Toml::Array(items) = node {
                    if items.is_empty() {
                        return self.err(format!("`{segment}` is an array with no tables in it"));
                    }
                    steps.push(Step::Index(items.len() - 1));
                }
            }
        }
        Ok(steps)
    }

    /// `a`, `a.b`, `"a.b"`, `'a'` — returns the segments with quoting removed.
    fn parse_dotted_key(&mut self) -> ManifestResult<Vec<String>> {
        let mut segments = Vec::new();
        loop {
            self.skip_inline_space();
            let segment = match self.peek() {
                Some(b'"') => self.parse_basic_string()?,
                Some(b'\'') => self.parse_literal_string()?,
                _ => {
                    let start = self.pos;
                    while matches!(self.peek(), Some(b) if b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
                    {
                        self.bump();
                    }
                    if self.pos == start {
                        return self.err("expected a key");
                    }
                    String::from_utf8_lossy(&self.bytes[start..self.pos]).into_owned()
                }
            };
            segments.push(segment);
            self.skip_inline_space();
            if !self.eat(b'.') {
                return Ok(segments);
            }
        }
    }

    fn parse_value(&mut self) -> ManifestResult<Toml> {
        match self.peek() {
            Some(b'"') => {
                if self.starts_with(r#"""""#) {
                    self.parse_multiline_string(r#"""""#).map(Toml::Str)
                } else {
                    self.parse_basic_string().map(Toml::Str)
                }
            }
            Some(b'\'') => {
                if self.starts_with("'''") {
                    self.parse_multiline_string("'''").map(Toml::Str)
                } else {
                    self.parse_literal_string().map(Toml::Str)
                }
            }
            Some(b'[') => self.parse_array(),
            Some(b'{') => self.parse_inline_table(),
            Some(_) => self.parse_bare_value(),
            None => self.err("expected a value"),
        }
    }

    fn parse_basic_string(&mut self) -> ManifestResult<String> {
        self.bump(); // opening quote
        let mut out = String::new();
        loop {
            match self.bump() {
                None | Some(b'\n') => return self.err("unterminated string"),
                Some(b'"') => return Ok(out),
                Some(b'\\') => out.push(self.parse_escape()?),
                Some(byte) => self.push_utf8(&mut out, byte),
            }
        }
    }

    fn parse_literal_string(&mut self) -> ManifestResult<String> {
        self.bump(); // opening quote
        let mut out = String::new();
        loop {
            match self.bump() {
                None | Some(b'\n') => return self.err("unterminated literal string"),
                Some(b'\'') => return Ok(out),
                Some(byte) => self.push_utf8(&mut out, byte),
            }
        }
    }

    fn parse_multiline_string(&mut self, fence: &str) -> ManifestResult<String> {
        for _ in 0..3 {
            self.bump();
        }
        // A newline immediately after the opening fence is trimmed (TOML 1.0).
        if self.peek() == Some(b'\n') {
            self.bump();
        }
        let basic = fence.starts_with('"');
        let mut out = String::new();
        loop {
            if self.starts_with(fence) {
                for _ in 0..3 {
                    self.bump();
                }
                return Ok(out);
            }
            match self.bump() {
                None => return self.err("unterminated multi-line string"),
                Some(b'\\') if basic => out.push(self.parse_escape()?),
                Some(byte) => self.push_utf8(&mut out, byte),
            }
        }
    }

    fn parse_escape(&mut self) -> ManifestResult<char> {
        match self.bump() {
            Some(b'n') => Ok('\n'),
            Some(b't') => Ok('\t'),
            Some(b'r') => Ok('\r'),
            Some(b'"') => Ok('"'),
            Some(b'\\') => Ok('\\'),
            Some(b'b') => Ok('\u{8}'),
            Some(b'f') => Ok('\u{c}'),
            Some(b'u') => self.parse_unicode_escape(4),
            Some(b'U') => self.parse_unicode_escape(8),
            Some(byte) => self.err(format!("unknown escape `\\{}`", byte as char)),
            None => self.err("string ends in a backslash"),
        }
    }

    fn parse_unicode_escape(&mut self, digits: usize) -> ManifestResult<char> {
        let mut value: u32 = 0;
        for _ in 0..digits {
            let byte = match self.bump() {
                Some(b) => b,
                None => return self.err("truncated unicode escape"),
            };
            match (byte as char).to_digit(16) {
                Some(digit) => value = value * 16 + digit,
                None => return self.err("non-hex digit in a unicode escape"),
            }
        }
        match char::from_u32(value) {
            Some(c) => Ok(c),
            None => self.err("unicode escape is not a scalar value"),
        }
    }

    /// Bytes arrive one at a time, so a multi-byte UTF-8 sequence is
    /// reassembled here rather than lost.
    fn push_utf8(&mut self, out: &mut String, first: u8) {
        if first.is_ascii() {
            out.push(first as char);
            return;
        }
        let extra = match first {
            0xC0..=0xDF => 1,
            0xE0..=0xEF => 2,
            _ => 3,
        };
        let start = self.pos - 1;
        for _ in 0..extra {
            self.bump();
        }
        out.push_str(&String::from_utf8_lossy(&self.bytes[start..self.pos]));
    }

    fn parse_array(&mut self) -> ManifestResult<Toml> {
        self.bump(); // '['
        let mut items = Vec::new();
        loop {
            self.skip_trivia();
            if self.peek().is_none() {
                return self.err("unterminated array");
            }
            if self.eat(b']') {
                return Ok(Toml::Array(items));
            }
            items.push(self.parse_value()?);
            self.skip_trivia();
            if self.eat(b',') {
                continue;
            }
            self.skip_trivia();
            if self.eat(b']') {
                return Ok(Toml::Array(items));
            }
            return self.err("expected `,` or `]` in an array");
        }
    }

    fn parse_inline_table(&mut self) -> ManifestResult<Toml> {
        self.bump(); // '{'
        let mut table = Toml::Table(Vec::new());
        loop {
            self.skip_inline_space();
            if self.peek().is_none() {
                return self.err("unterminated inline table");
            }
            if self.eat(b'}') {
                return Ok(table);
            }
            let key = self.parse_dotted_key()?;
            self.skip_inline_space();
            if !self.eat(b'=') {
                return self.err("expected `=` in an inline table");
            }
            self.skip_inline_space();
            let value = self.parse_value()?;
            self.insert(&mut table, &key, value)?;
            self.skip_inline_space();
            if self.eat(b',') {
                continue;
            }
            if self.eat(b'}') {
                return Ok(table);
            }
            return self.err("expected `,` or `}` in an inline table");
        }
    }

    /// `true`, `false`, or a number/datetime kept verbatim.
    ///
    /// The charset check is what stops a lenient read of garbage: `name =
    /// widget` and `name = @@@` are both rejected here rather than silently
    /// becoming strings.
    fn parse_bare_value(&mut self) -> ManifestResult<Toml> {
        let start = self.pos;
        while matches!(self.peek(), Some(b) if !b" \t\r\n,]}#".contains(&b)) {
            self.bump();
        }
        let token = String::from_utf8_lossy(&self.bytes[start..self.pos]).into_owned();
        match token.as_str() {
            "true" => return Ok(Toml::Bool(true)),
            "false" => return Ok(Toml::Bool(false)),
            "" => return self.err("expected a value"),
            _ => {}
        }
        let numeric = token.starts_with(|c: char| c.is_ascii_digit() || c == '+' || c == '-')
            && token
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '+' | '-' | '.' | ':'));
        if numeric {
            Ok(Toml::Bare(token))
        } else {
            self.err(format!(
                "`{token}` is not a value: strings must be quoted, and this is not a number, a datetime, or a boolean"
            ))
        }
    }

    fn insert(&self, table: &mut Toml, key: &[String], value: Toml) -> ManifestResult<()> {
        let mut node = table;
        for segment in &key[..key.len() - 1] {
            let Toml::Table(entries) = node else {
                return Err(ManifestError::at_line(
                    self.path,
                    self.line,
                    format!("`{segment}` is not inside a table"),
                ));
            };
            if !entries.iter().any(|(k, _)| k == segment) {
                entries.push((segment.clone(), Toml::Table(Vec::new())));
            }
            node = entries
                .iter_mut()
                .find(|(k, _)| k == segment)
                .map(|(_, v)| v)
                .expect("just inserted");
        }
        let last = &key[key.len() - 1];
        let Toml::Table(entries) = node else {
            return Err(ManifestError::at_line(
                self.path,
                self.line,
                format!("`{last}` is not inside a table"),
            ));
        };
        if entries.iter().any(|(k, _)| k == last) {
            return Err(ManifestError::at_line(
                self.path,
                self.line,
                format!("`{last}` is defined twice"),
            ));
        }
        entries.push((last.clone(), value));
        Ok(())
    }
}

fn resolve_step_path<'a>(root: &'a mut Toml, steps: &[Step]) -> Option<&'a mut Toml> {
    let mut node = root;
    for step in steps {
        node = match (step, node) {
            (Step::Key(key), Toml::Table(entries)) => {
                entries.iter_mut().find(|(k, _)| k == key).map(|(_, v)| v)?
            }
            (Step::Index(index), Toml::Array(items)) => items.get_mut(*index)?,
            _ => return None,
        };
    }
    Some(node)
}

fn parse_toml(path: &Path, content: &str) -> ManifestResult<Toml> {
    TomlParser::new(path, content).parse_document()
}

/// Render one key-path segment for an [`Origin`], quoting it when it contains a
/// separator — `project.entry-points."flake8.extension".ACME` reads back
/// against the file, `project.entry-points.flake8.extension.ACME` does not.
fn toml_key_segment(segment: &str) -> String {
    if segment.is_empty()
        || segment
            .chars()
            .any(|c| !(c.is_ascii_alphanumeric() || c == '_' || c == '-'))
    {
        format!("{segment:?}")
    } else {
        segment.to_string()
    }
}

fn toml_str<'a>(path: &Path, key: &str, value: &'a Toml) -> ManifestResult<&'a str> {
    value.as_str().ok_or_else(|| {
        ManifestError::at_key(
            path,
            key,
            format!("must be a string, found {}", value.type_name()),
        )
    })
}

// ---------------------------------------------------------------------------
// pyproject.toml (§5.2, Python)
// ---------------------------------------------------------------------------

/// Parse a `pyproject.toml` into the entry points packaging tools install.
///
/// Covers `[project.scripts]`, `[project.gui-scripts]` and every
/// `[project.entry-points.<group>]` table. The last of these is the one §4.1
/// singles out: *a package whose only consumer is an entry-point group is
/// structurally invisible* to a dependency checker, because nothing imports it
/// — the installer wires it up from this table at install time.
///
/// Values stay [`RootTarget::Reference`]s. `acme.cli:main` is an object
/// reference, and turning it into `acme/cli.py` requires knowing the package
/// layout, `src/` or not, and namespace packages. That resolution belongs to
/// whoever knows the interpreter's view of the tree, not to a transcriber.
pub fn parse_pyproject_toml(manifest: &Path, content: &str) -> ManifestResult<ManifestRoots> {
    let doc = parse_toml(manifest, content)?;
    let mut out = ManifestRoots::from_source(manifest);

    for (table, kind) in [
        ("scripts", RootKind::Executable),
        ("gui-scripts", RootKind::Executable),
    ] {
        if let Some(node) = doc.get_path(&["project", table]) {
            let entries = expect_toml_table(manifest, &format!("project.{table}"), node)?;
            for (name, value) in entries {
                let key = format!("project.{table}.{}", toml_key_segment(name));
                let text = toml_str(manifest, &key, value)?;
                out.push(kind, manifest, key, RootTarget::Reference(text.to_string()));
            }
        }
    }

    if let Some(node) = doc.get_path(&["project", "entry-points"]) {
        let groups = expect_toml_table(manifest, "project.entry-points", node)?;
        for (group, members) in groups {
            let group_key = format!("project.entry-points.{}", toml_key_segment(group));
            let entries = expect_toml_table(manifest, &group_key, members)?;
            for (name, value) in entries {
                let key = format!("{group_key}.{}", toml_key_segment(name));
                let text = toml_str(manifest, &key, value)?;
                out.push(
                    RootKind::PluginEntryPoint,
                    manifest,
                    key,
                    RootTarget::Reference(text.to_string()),
                );
            }
        }
    }

    Ok(out)
}

fn expect_toml_table<'a>(
    manifest: &Path,
    key: &str,
    value: &'a Toml,
) -> ManifestResult<&'a [(String, Toml)]> {
    value.as_table().ok_or_else(|| {
        ManifestError::at_key(
            manifest,
            key,
            format!("must be a table, found {}", value.type_name()),
        )
    })
}

// ---------------------------------------------------------------------------
// Cargo.toml (§5.2, Rust)
// ---------------------------------------------------------------------------

/// Parse a `Cargo.toml` into its declared targets.
///
/// `[lib]`, `[[bin]]`, `[[example]]`, `[[bench]]` and `[[test]]`, plus the
/// `crate-type` declaration §5.2 flags: `cdylib` or `staticlib` means the
/// consumer is outside the crate graph entirely, recorded as
/// [`Declaration::ConsumerOutsideBuildGraph`] rather than as a root.
///
/// A target with no `path` is recorded by *name*, as a
/// [`RootTarget::Reference`]. Cargo locates it by target auto-discovery, and
/// writing down `src/bin/<name>.rs` here would be a guess wearing a
/// declaration's clothes — the file itself is found by [`scan`]'s implicit
/// sweep, which is where a filesystem question belongs.
pub fn parse_cargo_toml(manifest: &Path, content: &str) -> ManifestResult<ManifestRoots> {
    let doc = parse_toml(manifest, content)?;
    let dir = manifest_dir(manifest);
    let mut out = ManifestRoots::from_source(manifest);

    if let Some(node) = doc.get("lib") {
        let entries = expect_toml_table(manifest, "lib", node)?;
        push_cargo_target(
            manifest,
            dir,
            "lib",
            entries,
            RootKind::LibraryEntry,
            &mut out,
        )?;
        push_crate_type_declaration(manifest, "lib", entries, &mut out)?;
    }

    for (table, kind) in [
        ("bin", RootKind::Executable),
        ("example", RootKind::DevTarget),
        ("bench", RootKind::DevTarget),
        ("test", RootKind::DevTarget),
    ] {
        let Some(node) = doc.get(table) else { continue };
        let items = node.as_array().ok_or_else(|| {
            ManifestError::at_key(
                manifest,
                table,
                format!("must be an array of tables, found {}", node.type_name()),
            )
        })?;
        for (index, item) in items.iter().enumerate() {
            let key = format!("{table}[{index}]");
            let entries = expect_toml_table(manifest, &key, item)?;
            push_cargo_target(manifest, dir, &key, entries, kind, &mut out)?;
        }
    }

    Ok(out)
}

fn push_cargo_target(
    manifest: &Path,
    dir: &Path,
    key: &str,
    entries: &[(String, Toml)],
    kind: RootKind,
    out: &mut ManifestRoots,
) -> ManifestResult<()> {
    let lookup = |name: &str| entries.iter().find(|(k, _)| k == name).map(|(_, v)| v);

    if let Some(path) = lookup("path") {
        let key = key_join(key, "path");
        let text = toml_str(manifest, &key, path)?;
        out.push(kind, manifest, key, rel_path(dir, text));
    } else if let Some(name) = lookup("name") {
        let key = key_join(key, "name");
        let text = toml_str(manifest, &key, name)?;
        out.push(kind, manifest, key, RootTarget::Reference(text.to_string()));
    }
    Ok(())
}

fn push_crate_type_declaration(
    manifest: &Path,
    key: &str,
    entries: &[(String, Toml)],
    out: &mut ManifestRoots,
) -> ManifestResult<()> {
    // Cargo accepts both spellings; the key recorded is the one written.
    for spelling in ["crate-type", "crate_type"] {
        let Some(value) = entries.iter().find(|(k, _)| k == spelling).map(|(_, v)| v) else {
            continue;
        };
        let key = key_join(key, spelling);
        let items = value.as_array().ok_or_else(|| {
            ManifestError::at_key(
                manifest,
                &key,
                format!("must be an array, found {}", value.type_name()),
            )
        })?;
        for (index, item) in items.iter().enumerate() {
            let item_key = key_index(&key, index);
            let text = toml_str(manifest, &item_key, item)?;
            if matches!(text, "cdylib" | "staticlib") {
                out.declarations
                    .push(Declaration::ConsumerOutsideBuildGraph {
                        origin: Origin::new(manifest, item_key),
                        crate_type: text.to_string(),
                    });
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// setup.cfg (§5.2, Python)
// ---------------------------------------------------------------------------

/// Parse a `setup.cfg` for its `[options.entry_points]` groups.
///
/// setuptools' INI dialect nests one level without saying so: the value of
/// `console_scripts` is itself a block of `name = target` lines, indented under
/// the key. `console_scripts` and `gui_scripts` install executables; every
/// other group is a registration some framework resolves at runtime.
pub fn parse_setup_cfg(manifest: &Path, content: &str) -> ManifestResult<ManifestRoots> {
    let mut out = ManifestRoots::from_source(manifest);
    let mut section: Option<String> = None;
    // The key whose indented block we are currently inside, and its lines.
    let mut open_key: Option<(String, Vec<(usize, String)>)> = None;

    let flush = |out: &mut ManifestRoots,
                 section: &Option<String>,
                 open: Option<(String, Vec<(usize, String)>)>|
     -> ManifestResult<()> {
        let (Some(section), Some((group, lines))) = (section.as_deref(), open) else {
            return Ok(());
        };
        if section != "options.entry_points" {
            return Ok(());
        }
        let kind = match group.as_str() {
            "console_scripts" | "gui_scripts" => RootKind::Executable,
            _ => RootKind::PluginEntryPoint,
        };
        for (line_no, line) in lines {
            let Some((name, target)) = line.split_once('=') else {
                return Err(ManifestError::at_line(
                    manifest,
                    line_no,
                    format!("entry point `{}` is not `name = target`", line.trim()),
                ));
            };
            let key = format!("{section}.{group}.{}", name.trim());
            out.push(
                kind,
                manifest,
                key,
                RootTarget::Reference(target.trim().to_string()),
            );
        }
        Ok(())
    };

    for (index, raw) in content.lines().enumerate() {
        let line_no = index + 1;
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with(';') {
            continue;
        }
        let indented = raw.starts_with(' ') || raw.starts_with('\t');

        if indented {
            match open_key.as_mut() {
                Some((_, lines)) => lines.push((line_no, trimmed.to_string())),
                None => {
                    return Err(ManifestError::at_line(
                        manifest,
                        line_no,
                        "indented continuation with no key above it",
                    ))
                }
            }
            continue;
        }

        flush(&mut out, &section, open_key.take())?;

        if let Some(rest) = trimmed.strip_prefix('[') {
            let name = rest.strip_suffix(']').ok_or_else(|| {
                ManifestError::at_line(manifest, line_no, "unterminated section header")
            })?;
            section = Some(name.trim().to_string());
            continue;
        }

        let Some((key, value)) = trimmed.split_once('=') else {
            return Err(ManifestError::at_line(
                manifest,
                line_no,
                format!("`{trimmed}` is neither a section header nor `key = value`"),
            ));
        };
        if section.is_none() {
            return Err(ManifestError::at_line(
                manifest,
                line_no,
                format!("`{}` appears before any section header", key.trim()),
            ));
        }
        // `console_scripts =` opens an indented block; `packages = find:` does
        // not, and nothing this module reads lives in a single-line value.
        if value.trim().is_empty() {
            open_key = Some((key.trim().to_string(), Vec::new()));
        }
    }
    flush(&mut out, &section, open_key.take())?;

    Ok(out)
}

// ---------------------------------------------------------------------------
// Go (§5.2)
// ---------------------------------------------------------------------------

/// Every directive `go.mod` may contain. An unknown one means this is not a
/// `go.mod` we understand, which is not the same as one declaring no roots.
const GO_MOD_DIRECTIVES: [&str; 7] = [
    "module",
    "go",
    "toolchain",
    "require",
    "replace",
    "exclude",
    "retract",
];

/// Parse a `go.mod` for the module path.
///
/// The module path is the name every importable package in this tree hangs off,
/// so a `go.mod` without one has not told us what anything here is called.
pub fn parse_go_mod(manifest: &Path, content: &str) -> ManifestResult<ManifestRoots> {
    let mut out = ManifestRoots::from_source(manifest);
    let mut module: Option<String> = None;
    let mut in_block = false;

    for (index, raw) in content.lines().enumerate() {
        let line_no = index + 1;
        let line = match raw.split_once("//") {
            Some((before, _)) => before.trim(),
            None => raw.trim(),
        };
        if line.is_empty() {
            continue;
        }
        if in_block {
            if line == ")" {
                in_block = false;
            }
            continue;
        }
        let (directive, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
        if !GO_MOD_DIRECTIVES.contains(&directive) {
            return Err(ManifestError::at_line(
                manifest,
                line_no,
                format!("`{directive}` is not a go.mod directive"),
            ));
        }
        if rest.trim() == "(" {
            in_block = true;
            continue;
        }
        if directive == "module" {
            let path = rest.trim().trim_matches('"');
            if path.is_empty() {
                return Err(ManifestError::at_line(
                    manifest,
                    line_no,
                    "`module` has no path",
                ));
            }
            if module.is_some() {
                return Err(ManifestError::at_line(
                    manifest,
                    line_no,
                    "`module` appears twice",
                ));
            }
            module = Some(path.to_string());
        }
    }

    match module {
        Some(path) => {
            out.push(
                RootKind::LibraryEntry,
                manifest,
                "module".into(),
                RootTarget::Reference(path),
            );
            Ok(out)
        }
        None => Err(ManifestError::parse(manifest, "no `module` directive")),
    }
}

/// Read a `.go` file's package clause; a `package main` makes the file an
/// executable root.
///
/// §5.2 lists "`go.mod` + every `package main`" together for a reason: the
/// module file says what the tree is called, and only the package clause says
/// which files are programs.
pub fn parse_go_source(file: &Path, content: &str) -> ManifestResult<ManifestRoots> {
    let mut out = ManifestRoots::from_source(file);
    let package = go_package_clause(file, content)?;
    if package == "main" {
        out.push(
            RootKind::Executable,
            file,
            "package".into(),
            RootTarget::Path(file.to_path_buf()),
        );
    }
    Ok(out)
}

/// The package name, skipping comments. Written by hand rather than by regex
/// because `// package main` in a doc comment and `package main` in code must
/// not be confused.
fn go_package_clause(file: &Path, content: &str) -> ManifestResult<String> {
    let bytes = content.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b' ' | b'\t' | b'\r' | b'\n' => index += 1,
            b'/' if bytes.get(index + 1) == Some(&b'/') => {
                index += 2;
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
            }
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                index += 2;
                loop {
                    if index + 1 >= bytes.len() {
                        return Err(ManifestError::parse(file, "unterminated block comment"));
                    }
                    if bytes[index] == b'*' && bytes[index + 1] == b'/' {
                        index += 2;
                        break;
                    }
                    index += 1;
                }
            }
            _ => break,
        }
    }
    let rest = &content[index..];
    let after = rest
        .strip_prefix("package")
        .filter(|r| r.starts_with(|c: char| c.is_whitespace()));
    let Some(after) = after else {
        return Err(ManifestError::parse(file, "no `package` clause"));
    };
    let name = after
        .trim_start()
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .next()
        .unwrap_or("");
    if name.is_empty() {
        return Err(ManifestError::parse(file, "`package` clause has no name"));
    }
    Ok(name.to_string())
}

// ---------------------------------------------------------------------------
// Dockerfile (§5.2, containers)
// ---------------------------------------------------------------------------

/// Every instruction a Dockerfile may contain. An unrecognized one means this
/// is not a file we parsed correctly — most likely a heredoc body or a
/// continuation we failed to join — and guessing past it would drop whatever
/// `COPY` lines came after.
const DOCKER_INSTRUCTIONS: [&str; 18] = [
    "FROM",
    "RUN",
    "CMD",
    "LABEL",
    "MAINTAINER",
    "EXPOSE",
    "ENV",
    "ADD",
    "COPY",
    "ENTRYPOINT",
    "VOLUME",
    "USER",
    "WORKDIR",
    "ARG",
    "ONBUILD",
    "STOPSIGNAL",
    "HEALTHCHECK",
    "SHELL",
];

/// Parse a `Dockerfile` for its entry point and the repository files it copies
/// into the image.
///
/// Both `CMD`/`ENTRYPOINT` forms §5.2 names are handled: the exec form
/// (`["node", "server.js"]`, parsed as a JSON array) and the shell form. Both
/// are recorded as a [`RootTarget::Command`]; the exec form's argv is joined
/// with spaces, because what makes it a root is that the image runs it, and the
/// shell-versus-exec distinction changes how it is run, not which files matter.
///
/// `COPY`/`ADD` sources are the roots §5.2 flags with an exclamation mark, and
/// they are recorded as globs because Docker matches them as patterns. A
/// `COPY --from=<stage>` is deliberately skipped: it reads out of an earlier
/// build stage's filesystem, so its source path names nothing in this
/// repository and recording it would manufacture a root for a file that is not
/// here.
///
/// Keys are `<instruction>@<line>`, optionally with a source index —
/// `Dockerfile#copy@4[1]`. A Dockerfile has no other addressable structure, and
/// the line is what a human checks against the file.
pub fn parse_dockerfile(manifest: &Path, content: &str) -> ManifestResult<ManifestRoots> {
    let mut out = ManifestRoots::from_source(manifest);
    let lines: Vec<&str> = content.lines().collect();
    let mut index = 0;
    let mut saw_from = false;

    while index < lines.len() {
        let start_line = index + 1;
        let trimmed = lines[index].trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            index += 1;
            continue;
        }

        // Join continuation lines into one logical instruction.
        let mut text = String::new();
        loop {
            let line = lines[index].trim();
            index += 1;
            let (body, continued) = match line.strip_suffix('\\') {
                Some(body) => (body, true),
                None => (line, false),
            };
            if !text.is_empty() {
                text.push(' ');
            }
            text.push_str(body.trim());
            if !continued || index >= lines.len() {
                break;
            }
            // A comment between continuation lines is not part of the command.
            while index < lines.len() && lines[index].trim().starts_with('#') {
                index += 1;
            }
        }

        // A heredoc body is data, not instructions.
        for terminator in heredoc_terminators(&text) {
            while index < lines.len() && lines[index].trim() != terminator {
                index += 1;
            }
            index += 1; // the terminator line itself
        }

        let mut text = text.trim();
        loop {
            let (instruction, rest) = text.split_once(char::is_whitespace).unwrap_or((text, ""));
            let upper = instruction.to_ascii_uppercase();
            if !DOCKER_INSTRUCTIONS.contains(&upper.as_str()) {
                return Err(ManifestError::at_line(
                    manifest,
                    start_line,
                    format!("`{instruction}` is not a Dockerfile instruction"),
                ));
            }
            // `ONBUILD COPY . /app` is a deferred instruction; unwrap and retry.
            if upper == "ONBUILD" {
                text = rest.trim();
                continue;
            }
            let rest = rest.trim();
            if upper == "FROM" {
                saw_from = true;
            }
            if rest.is_empty() && matches!(upper.as_str(), "COPY" | "ADD" | "CMD" | "ENTRYPOINT") {
                return Err(ManifestError::at_line(
                    manifest,
                    start_line,
                    format!("`{upper}` has no arguments"),
                ));
            }
            match upper.as_str() {
                "COPY" | "ADD" => {
                    push_docker_copy(manifest, start_line, &upper, rest, &mut out)?;
                }
                "CMD" | "ENTRYPOINT" => {
                    let command = docker_command(manifest, start_line, rest)?;
                    out.push(
                        RootKind::ContainerEntry,
                        manifest,
                        format!("{}@{start_line}", upper.to_ascii_lowercase()),
                        RootTarget::Command(command),
                    );
                }
                _ => {}
            }
            break;
        }
    }

    if !saw_from {
        return Err(ManifestError::parse(
            manifest,
            "no `FROM` instruction: this is not a complete Dockerfile",
        ));
    }
    Ok(out)
}

/// Terminators opened by `<<EOF`, `<<-EOF`, `<<"EOF"` on one logical line.
fn heredoc_terminators(text: &str) -> Vec<String> {
    let mut terminators = Vec::new();
    let bytes = text.as_bytes();
    let mut index = 0;
    while index + 1 < bytes.len() {
        if bytes[index] == b'<' && bytes[index + 1] == b'<' {
            let mut cursor = index + 2;
            if bytes.get(cursor) == Some(&b'-') {
                cursor += 1;
            }
            let quote = matches!(bytes.get(cursor), Some(b'"') | Some(b'\''));
            if quote {
                cursor += 1;
            }
            let start = cursor;
            while matches!(bytes.get(cursor), Some(b) if b.is_ascii_alphanumeric() || *b == b'_') {
                cursor += 1;
            }
            if cursor > start {
                terminators.push(text[start..cursor].to_string());
            }
            index = cursor;
        } else {
            index += 1;
        }
    }
    terminators
}

fn push_docker_copy(
    manifest: &Path,
    line: usize,
    instruction: &str,
    rest: &str,
    out: &mut ManifestRoots,
) -> ManifestResult<()> {
    let dir = manifest_dir(manifest);
    let args = if rest.starts_with('[') {
        docker_exec_form(manifest, line, rest)?
    } else {
        shell_split(rest)
    };
    let mut from_stage = false;
    let mut operands: Vec<String> = Vec::new();
    for arg in args {
        if let Some(flag) = arg.strip_prefix("--") {
            if flag.starts_with("from=") {
                from_stage = true;
            }
            continue;
        }
        operands.push(arg);
    }
    if operands.len() < 2 {
        return Err(ManifestError::at_line(
            manifest,
            line,
            format!("`{instruction}` needs a source and a destination"),
        ));
    }
    if from_stage {
        return Ok(());
    }
    let key = format!("{}@{line}", instruction.to_ascii_lowercase());
    // The last operand is the destination inside the image.
    for (index, source) in operands[..operands.len() - 1].iter().enumerate() {
        out.push(
            RootKind::PackagedFile,
            manifest,
            key_index(&key, index),
            rel_glob(dir, source),
        );
    }
    Ok(())
}

fn docker_command(manifest: &Path, line: usize, rest: &str) -> ManifestResult<String> {
    if rest.starts_with('[') {
        Ok(docker_exec_form(manifest, line, rest)?.join(" "))
    } else {
        Ok(rest.to_string())
    }
}

fn docker_exec_form(manifest: &Path, line: usize, rest: &str) -> ManifestResult<Vec<String>> {
    let parsed: serde_json::Value = serde_json::from_str(rest).map_err(|e| {
        ManifestError::at_line(
            manifest,
            line,
            format!("exec form is not a JSON array: {e}"),
        )
    })?;
    let items = parsed
        .as_array()
        .ok_or_else(|| ManifestError::at_line(manifest, line, "exec form must be a JSON array"))?;
    items
        .iter()
        .map(|item| {
            item.as_str().map(str::to_string).ok_or_else(|| {
                ManifestError::at_line(manifest, line, "every exec-form argument must be a string")
            })
        })
        .collect()
}

/// Split on whitespace, respecting single and double quotes. Enough for
/// `COPY`/`ADD` operands, which is all it is used for.
fn shell_split(text: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    for c in text.chars() {
        match (quote, c) {
            (Some(q), _) if c == q => quote = None,
            (Some(_), _) => current.push(c),
            (None, '"') | (None, '\'') => quote = Some(c),
            (None, c) if c.is_whitespace() => {
                if !current.is_empty() {
                    args.push(std::mem::take(&mut current));
                }
            }
            (None, c) => current.push(c),
        }
    }
    if !current.is_empty() {
        args.push(current);
    }
    args
}

// ---------------------------------------------------------------------------
// YAML
// ---------------------------------------------------------------------------
//
// §5.2 asks for `.github/workflows/*.yml` `run:` bodies and `uses:` paths. A
// line scanner would find most of them, and would be the wrong tool for exactly
// one reason: it can never fail. A scanner that shrugs at a file it does not
// understand reports "this workflow declares no roots", which is the §6.20
// failure the whole module exists to avoid. So this is a real, if small,
// parser: block mappings and sequences, the four scalar styles, block scalars
// with their indicators and chomping, and flow collections. Anchors, aliases,
// tags and multi-document streams are *rejected*, not ignored — a construct we
// do not model is an error with a line number, never silence.

#[derive(Debug, Clone, PartialEq)]
enum Yaml {
    Scalar(String),
    Seq(Vec<Yaml>),
    Map(Vec<(String, Yaml)>),
    Empty,
}

struct YamlParser<'a> {
    path: &'a Path,
    lines: Vec<&'a str>,
    pos: usize,
    /// A mapping that began on a sequence line (`- uses: x`) is re-presented to
    /// the mapping parser as a line of its own, indented to the column the key
    /// actually starts at.
    pending: Option<(usize, usize, String)>,
    started: bool,
}

/// One significant line: 1-based number, indent column, content.
type YamlLine = (usize, usize, String);

impl<'a> YamlParser<'a> {
    fn new(path: &'a Path, content: &'a str) -> YamlParser<'a> {
        YamlParser {
            path,
            lines: content.lines().collect(),
            pos: 0,
            pending: None,
            started: false,
        }
    }

    fn err<T>(&self, line: usize, detail: impl Into<String>) -> ManifestResult<T> {
        Err(ManifestError::at_line(self.path, line, detail))
    }

    /// The next line that carries structure, without consuming it.
    fn peek(&mut self) -> ManifestResult<Option<YamlLine>> {
        if let Some(pending) = &self.pending {
            return Ok(Some(pending.clone()));
        }
        while self.pos < self.lines.len() {
            let raw = self.lines[self.pos];
            let indent = raw.len() - raw.trim_start_matches(' ').len();
            let content = raw[indent..].trim_end();
            if content.is_empty() || content.starts_with('#') {
                self.pos += 1;
                continue;
            }
            if content.starts_with('\t') || raw[..indent].contains('\t') {
                return self.err(self.pos + 1, "a tab may not indent a YAML line");
            }
            if indent == 0 && (content == "---" || content == "...") {
                if self.started {
                    return self.err(
                        self.pos + 1,
                        "more than one document in a workflow file is not modelled",
                    );
                }
                self.pos += 1;
                continue;
            }
            self.started = true;
            return Ok(Some((self.pos + 1, indent, content.to_string())));
        }
        Ok(None)
    }

    fn consume(&mut self) {
        if self.pending.take().is_none() {
            self.pos += 1;
        }
    }

    fn parse_document(&mut self) -> ManifestResult<Yaml> {
        let Some((_, indent, _)) = self.peek()? else {
            return Ok(Yaml::Empty);
        };
        let node = self.parse_node(indent)?;
        if let Some((line, _, _)) = self.peek()? {
            return self.err(line, "content after the end of the document");
        }
        Ok(node)
    }

    fn parse_node(&mut self, indent: usize) -> ManifestResult<Yaml> {
        let Some((line, actual, content)) = self.peek()? else {
            return Ok(Yaml::Empty);
        };
        if actual < indent {
            return Ok(Yaml::Empty);
        }
        if actual > indent {
            return self.err(line, "unexpected indentation");
        }
        if is_sequence_entry(&content) {
            self.parse_seq(indent)
        } else {
            self.parse_map(indent)
        }
    }

    fn parse_map(&mut self, indent: usize) -> ManifestResult<Yaml> {
        let mut entries: Vec<(String, Yaml)> = Vec::new();
        loop {
            let Some((line, actual, content)) = self.peek()? else {
                break;
            };
            if actual < indent {
                break;
            }
            if actual > indent {
                return self.err(line, "unexpected indentation");
            }
            if is_sequence_entry(&content) {
                return self.err(line, "a sequence entry where a mapping key was expected");
            }
            let Some((key, rest)) = split_key(&content) else {
                return self.err(line, format!("`{content}` is not `key: value`"));
            };
            self.consume();
            let value = self.parse_value(line, indent, rest)?;
            if entries.iter().any(|(k, _)| *k == key) {
                return self.err(line, format!("`{key}` is defined twice"));
            }
            entries.push((key, value));
        }
        Ok(Yaml::Map(entries))
    }

    fn parse_seq(&mut self, indent: usize) -> ManifestResult<Yaml> {
        let mut items = Vec::new();
        loop {
            let Some((line, actual, content)) = self.peek()? else {
                break;
            };
            if actual < indent || !is_sequence_entry(&content) {
                break;
            }
            if actual > indent {
                return self.err(line, "unexpected indentation");
            }
            let after_dash = &content[1..];
            let rest = after_dash.trim_start();
            let column = indent + 1 + (after_dash.len() - rest.len());
            self.consume();
            if rest.is_empty() {
                items.push(self.parse_child(indent)?);
            } else if split_key(rest).is_some() {
                // `- uses: x` opens a mapping at the column the key sits in.
                self.pending = Some((line, column, rest.to_string()));
                items.push(self.parse_map(column)?);
            } else {
                items.push(self.parse_value(line, indent, rest)?);
            }
        }
        Ok(Yaml::Seq(items))
    }

    /// The value of `key:` — either on the same line, or the block under it.
    fn parse_value(&mut self, line: usize, indent: usize, rest: &str) -> ManifestResult<Yaml> {
        if rest.is_empty() {
            return self.parse_child(indent);
        }
        if let Some((style, chomp)) = block_scalar_header(rest) {
            return self.read_block_scalar(indent, style, chomp);
        }
        parse_flow(self.path, line, rest)
    }

    /// The block nested under a key or a bare `-`.
    ///
    /// A sequence is allowed to sit at the *same* column as its key — both
    /// styles are ubiquitous in workflow files — but a mapping must be deeper.
    fn parse_child(&mut self, indent: usize) -> ManifestResult<Yaml> {
        let Some((_, actual, content)) = self.peek()? else {
            return Ok(Yaml::Empty);
        };
        if actual > indent {
            return self.parse_node(actual);
        }
        if actual == indent && is_sequence_entry(&content) {
            return self.parse_seq(indent);
        }
        Ok(Yaml::Empty)
    }

    fn read_block_scalar(&mut self, indent: usize, style: u8, chomp: u8) -> ManifestResult<Yaml> {
        let mut raw: Vec<&str> = Vec::new();
        while self.pos < self.lines.len() {
            let line = self.lines[self.pos];
            let line_indent = line.len() - line.trim_start_matches(' ').len();
            if line.trim().is_empty() {
                raw.push("");
                self.pos += 1;
                continue;
            }
            if line_indent <= indent {
                break;
            }
            raw.push(line);
            self.pos += 1;
        }
        while raw.last().is_some_and(|l| l.is_empty()) {
            raw.pop();
        }
        let content_indent = raw
            .iter()
            .find(|l| !l.is_empty())
            .map(|l| l.len() - l.trim_start_matches(' ').len())
            .unwrap_or(0);
        let dedented: Vec<&str> = raw
            .iter()
            .map(|l| {
                if l.len() > content_indent {
                    &l[content_indent..]
                } else {
                    ""
                }
            })
            .collect();

        let mut body = if style == b'|' {
            dedented.join("\n")
        } else {
            // Folded: a line break between two non-empty lines becomes a space.
            let mut folded = String::new();
            for (index, line) in dedented.iter().enumerate() {
                if index > 0 {
                    if line.is_empty() || dedented[index - 1].is_empty() {
                        folded.push('\n');
                    } else {
                        folded.push(' ');
                    }
                }
                folded.push_str(line);
            }
            folded
        };
        match chomp {
            b'-' => {}
            b'+' => body.push('\n'),
            _ if !body.is_empty() => body.push('\n'),
            _ => {}
        }
        Ok(Yaml::Scalar(body))
    }
}

fn is_sequence_entry(content: &str) -> bool {
    content == "-" || content.starts_with("- ")
}

/// `|`, `>`, with an optional chomping indicator. Returns (style, chomp).
fn block_scalar_header(rest: &str) -> Option<(u8, u8)> {
    let bytes = rest.as_bytes();
    let style = *bytes.first()?;
    if style != b'|' && style != b'>' {
        return None;
    }
    let mut chomp = b' ';
    for &byte in &bytes[1..] {
        match byte {
            b'+' | b'-' => chomp = byte,
            b'0'..=b'9' => {}
            _ => return None,
        }
    }
    Some((style, chomp))
}

/// Split `key: value`, honouring quoted keys. `None` when the line is not a
/// mapping entry at all.
fn split_key(content: &str) -> Option<(String, &str)> {
    let bytes = content.as_bytes();
    let (key, after) = match bytes.first()? {
        quote @ (b'"' | b'\'') => {
            let end = content[1..].find(*quote as char)? + 1;
            (content[1..end].to_string(), &content[end + 1..])
        }
        _ => {
            let mut index = 0;
            loop {
                let offset = content[index..].find(':')? + index;
                let next = content.as_bytes().get(offset + 1);
                if next.is_none() || next == Some(&b' ') {
                    break (content[..offset].trim().to_string(), &content[offset + 1..]);
                }
                index = offset + 1;
            }
        }
    };
    let after = after.strip_prefix(':').unwrap_or(after);
    if key.is_empty() || key.contains('#') {
        return None;
    }
    Some((key, after.trim()))
}

/// A scalar or flow collection written on one line.
fn parse_flow(path: &Path, line: usize, text: &str) -> ManifestResult<Yaml> {
    let text = strip_trailing_comment(text);
    match text.as_bytes().first() {
        None => Ok(Yaml::Empty),
        Some(b'&' | b'*' | b'!') => Err(ManifestError::at_line(
            path,
            line,
            "anchors, aliases and tags are not modelled",
        )),
        Some(b'"' | b'\'') => {
            let (value, rest) = read_quoted(path, line, text)?;
            if !rest.trim().is_empty() {
                return Err(ManifestError::at_line(
                    path,
                    line,
                    "trailing text after a quoted scalar",
                ));
            }
            Ok(Yaml::Scalar(value))
        }
        Some(b'[') | Some(b'{') => {
            let (value, rest) = read_flow_collection(path, line, text)?;
            if !rest.trim().is_empty() {
                return Err(ManifestError::at_line(
                    path,
                    line,
                    "trailing text after a flow collection",
                ));
            }
            Ok(value)
        }
        _ => Ok(Yaml::Scalar(text.to_string())),
    }
}

fn strip_trailing_comment(text: &str) -> &str {
    let bytes = text.as_bytes();
    let mut quote: Option<u8> = None;
    for (index, &byte) in bytes.iter().enumerate() {
        match (quote, byte) {
            (Some(q), b) if b == q => quote = None,
            (Some(_), _) => {}
            (None, b'"' | b'\'') => quote = Some(byte),
            (None, b'#') if index == 0 || bytes[index - 1] == b' ' => {
                return text[..index].trim_end();
            }
            _ => {}
        }
    }
    text.trim_end()
}

fn read_quoted<'a>(path: &Path, line: usize, text: &'a str) -> ManifestResult<(String, &'a str)> {
    let quote = text.as_bytes()[0];
    let mut out = String::new();
    let mut chars = text.char_indices().skip(1);
    while let Some((index, c)) = chars.next() {
        if c as u8 == quote && quote == b'\'' {
            // `''` is an escaped quote inside a single-quoted scalar.
            if text[index + 1..].starts_with('\'') {
                out.push('\'');
                chars.next();
                continue;
            }
            return Ok((out, &text[index + 1..]));
        }
        if c == '\\' && quote == b'"' {
            match chars.next() {
                Some((_, 'n')) => out.push('\n'),
                Some((_, 't')) => out.push('\t'),
                Some((_, other)) => out.push(other),
                None => break,
            }
            continue;
        }
        if c as u8 == quote {
            return Ok((out, &text[index + 1..]));
        }
        out.push(c);
    }
    Err(ManifestError::at_line(
        path,
        line,
        "unterminated quoted scalar",
    ))
}

fn read_flow_collection<'a>(
    path: &Path,
    line: usize,
    text: &'a str,
) -> ManifestResult<(Yaml, &'a str)> {
    let (open, close) = match text.as_bytes()[0] {
        b'[' => (b'[', b']'),
        _ => (b'{', b'}'),
    };
    let mut rest = &text[1..];
    let mut items: Vec<Yaml> = Vec::new();
    let mut pairs: Vec<(String, Yaml)> = Vec::new();
    loop {
        rest = rest.trim_start();
        if rest.is_empty() {
            return Err(ManifestError::at_line(
                path,
                line,
                "unterminated flow collection",
            ));
        }
        if rest.as_bytes()[0] == close {
            let node = if open == b'[' {
                Yaml::Seq(items)
            } else {
                Yaml::Map(pairs)
            };
            return Ok((node, &rest[1..]));
        }
        let (element, after) = read_flow_element(path, line, rest)?;
        rest = after;
        if open == b'[' {
            items.push(element);
        } else {
            let Yaml::Scalar(key) = element else {
                return Err(ManifestError::at_line(
                    path,
                    line,
                    "a flow mapping key must be a scalar",
                ));
            };
            let (key, value_text) = match key.split_once(':') {
                Some((k, v)) => (k.trim().to_string(), v.trim().to_string()),
                None => {
                    return Err(ManifestError::at_line(
                        path,
                        line,
                        "a flow mapping needs `key: value`",
                    ))
                }
            };
            pairs.push((key, Yaml::Scalar(value_text)));
        }
        rest = rest.trim_start();
        if rest.starts_with(',') {
            rest = &rest[1..];
        }
    }
}

fn read_flow_element<'a>(
    path: &Path,
    line: usize,
    text: &'a str,
) -> ManifestResult<(Yaml, &'a str)> {
    match text.as_bytes().first() {
        Some(b'"' | b'\'') => {
            let (value, rest) = read_quoted(path, line, text)?;
            Ok((Yaml::Scalar(value), rest))
        }
        Some(b'[' | b'{') => read_flow_collection(path, line, text),
        Some(b'&' | b'*' | b'!') => Err(ManifestError::at_line(
            path,
            line,
            "anchors, aliases and tags are not modelled",
        )),
        _ => {
            let end = text.find([',', ']', '}']).unwrap_or(text.len());
            Ok((Yaml::Scalar(text[..end].trim().to_string()), &text[end..]))
        }
    }
}

// ---------------------------------------------------------------------------
// .github/workflows/*.yml (§5.2, CI)
// ---------------------------------------------------------------------------

/// Parse a GitHub Actions workflow for its `run:` bodies and `uses:` paths.
///
/// Both are collected wherever they appear, at any depth, and keyed by their
/// full position — `.github/workflows/ci.yml#jobs.build.steps[2].run` — so a
/// human can open the file at that step.
///
/// A `uses:` beginning with `./` is a local composite action, and GitHub
/// resolves it against the **repository root**, not against the workflow's own
/// directory; anything else (`owner/repo@ref`, `docker://image`) resolves
/// outside this repository and stays a [`RootTarget::Reference`].
pub fn parse_github_workflow(manifest: &Path, content: &str) -> ManifestResult<ManifestRoots> {
    let document = YamlParser::new(manifest, content).parse_document()?;
    let mut out = ManifestRoots::from_source(manifest);
    collect_workflow_roots(manifest, &document, "", &mut out)?;
    Ok(out)
}

fn collect_workflow_roots(
    manifest: &Path,
    node: &Yaml,
    key: &str,
    out: &mut ManifestRoots,
) -> ManifestResult<()> {
    match node {
        Yaml::Map(entries) => {
            for (name, value) in entries {
                let child = key_join(key, name);
                match name.as_str() {
                    "run" | "uses" => {
                        let Yaml::Scalar(text) = value else {
                            return Err(ManifestError::at_key(manifest, child, "must be a scalar"));
                        };
                        let (kind, target) = if name == "run" {
                            (RootKind::Command, RootTarget::Command(text.clone()))
                        } else if text.starts_with("./") {
                            (RootKind::CiAction, rel_path(Path::new(""), text))
                        } else {
                            (RootKind::CiAction, RootTarget::Reference(text.clone()))
                        };
                        out.push(kind, manifest, child, target);
                    }
                    _ => collect_workflow_roots(manifest, value, &child, out)?,
                }
            }
        }
        Yaml::Seq(items) => {
            for (index, item) in items.iter().enumerate() {
                collect_workflow_roots(manifest, item, &key_index(key, index), out)?;
            }
        }
        Yaml::Scalar(_) | Yaml::Empty => {}
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// scanning a repository
// ---------------------------------------------------------------------------

/// Directories [`scan`] does not descend into.
///
/// Public because a caller is entitled to know what was *not* looked at. Every
/// one of these holds code that belongs to somebody else's repository or to a
/// build, and a root found inside one is a root for a project we are not
/// cleaning. The cost of the list is real and stated here rather than hidden:
/// a source directory that happens to be called `vendor` or `target` is not
/// read, and its manifests will not appear in [`ManifestRoots::sources`].
pub const SKIPPED_DIRECTORIES: [&str; 10] = [
    ".git",
    "node_modules",
    "target",
    "vendor",
    "__pycache__",
    ".venv",
    "venv",
    ".tox",
    ".mypy_cache",
    ".pytest_cache",
];

/// Files that are entry points because of what they are called, with the key
/// recorded for each, the kind of root it is, and — the load-bearing column —
/// the tier that name actually earns.
///
/// The split is §5.1 applied literally rather than by which list a file appears
/// on. `__main__.py` is Tier A because `python -m pkg` runs it by a fixed rule
/// with no framework involved, and `conftest.py` is Tier A because pytest
/// always reads it. `wsgi.py`, `asgi.py`, `manage.py` and `celery.py` are Tier
/// B: each is an entry point only because something *outside* the repository —
/// a gunicorn command line, `DJANGO_SETTINGS_MODULE`, a Celery worker
/// invocation — is configured to import that name. Calling those Tier A would
/// hand a caller a guess about a framework wearing a manifest's confidence,
/// which mod.rs names as worse than emitting no root at all.
const IMPLICIT_PYTHON_ROOTS: [(&str, &str, RootKind, Tier); 6] = [
    (
        "__main__.py",
        "python:dash-m",
        RootKind::Executable,
        Tier::A,
    ),
    (
        "conftest.py",
        "pytest:conftest",
        RootKind::DevTarget,
        Tier::A,
    ),
    ("wsgi.py", "wsgi:callable", RootKind::LibraryEntry, Tier::B),
    ("asgi.py", "asgi:callable", RootKind::LibraryEntry, Tier::B),
    ("manage.py", "django:manage", RootKind::Executable, Tier::B),
    ("celery.py", "celery:app", RootKind::LibraryEntry, Tier::B),
];

/// Materialize every Tier A root declared anywhere in a repository.
///
/// Walks the tree once, parses every manifest of a family this module knows,
/// and adds the implicit files no manifest key names — Cargo's default targets,
/// `build.rs`, and the Python files listed in [`IMPLICIT_PYTHON_ROOTS`]. Paths,
/// origins and sources are all repo-relative.
///
/// **A single manifest that will not parse fails the whole scan.** Returning
/// the other roots and quietly dropping the broken package's would produce a
/// root set missing exactly the entry points of the one package we could not
/// read — the most dangerous possible answer, and the one §6.20 warns presents
/// as clean output.
pub fn scan(repo_root: &Path) -> ManifestResult<ManifestRoots> {
    let mut files = Vec::new();
    walk(repo_root, Path::new(""), &mut files)?;
    files.sort();

    let crate_dirs: BTreeSet<PathBuf> = files
        .iter()
        .filter(|rel| rel.file_name() == Some(std::ffi::OsStr::new("Cargo.toml")))
        .map(|rel| manifest_dir(rel).to_path_buf())
        .collect();

    let mut out = ManifestRoots::default();
    for rel in &files {
        let absolute = repo_root.join(rel);
        let name = rel.file_name().and_then(|n| n.to_str()).unwrap_or_default();

        let parsed = match name {
            "package.json" => Some(parse_package_json as fn(&Path, &str) -> _),
            "pyproject.toml" => Some(parse_pyproject_toml as fn(&Path, &str) -> _),
            "setup.cfg" => Some(parse_setup_cfg as fn(&Path, &str) -> _),
            "Cargo.toml" => Some(parse_cargo_toml as fn(&Path, &str) -> _),
            "go.mod" => Some(parse_go_mod as fn(&Path, &str) -> _),
            _ if is_dockerfile(name) => Some(parse_dockerfile as fn(&Path, &str) -> _),
            _ if is_workflow(rel) => Some(parse_github_workflow as fn(&Path, &str) -> _),
            _ if name.ends_with(".go") => Some(parse_go_source as fn(&Path, &str) -> _),
            _ => None,
        };
        if let Some(parse) = parsed {
            let content =
                std::fs::read_to_string(&absolute).map_err(|source| ManifestError::Read {
                    path: rel.clone(),
                    source,
                })?;
            out.merge(parse(rel, &content)?);
        }

        if let Some((_, key, kind, tier)) = IMPLICIT_PYTHON_ROOTS
            .iter()
            .find(|(file, ..)| *file == name)
        {
            out.roots.push(Root::new(
                *tier,
                *kind,
                Origin::new(rel, *key),
                RootTarget::Path(rel.clone()),
            ));
        }

        if let Some(key) = cargo_implicit_key(rel, &crate_dirs) {
            let kind = match key {
                "cargo:default-lib" => RootKind::LibraryEntry,
                "cargo:build-script" => RootKind::BuildHook,
                _ => RootKind::Executable,
            };
            out.push(kind, rel, key.to_string(), RootTarget::Path(rel.clone()));
        }
    }
    Ok(out)
}

fn walk(repo_root: &Path, rel_dir: &Path, files: &mut Vec<PathBuf>) -> ManifestResult<()> {
    let dir = repo_root.join(rel_dir);
    let entries = std::fs::read_dir(&dir).map_err(|source| ManifestError::Read {
        path: rel_dir.to_path_buf(),
        source,
    })?;
    let mut children: Vec<PathBuf> = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| ManifestError::Read {
            path: rel_dir.to_path_buf(),
            source,
        })?;
        children.push(entry.path());
    }
    children.sort();

    for child in children {
        let Some(name) = child.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let rel = if rel_dir.as_os_str().is_empty() {
            PathBuf::from(name)
        } else {
            rel_dir.join(name)
        };
        // Never follow a symlink: it can leave the repository, and it can cycle.
        let meta = std::fs::symlink_metadata(&child).map_err(|source| ManifestError::Read {
            path: rel.clone(),
            source,
        })?;
        if meta.file_type().is_symlink() {
            continue;
        }
        if meta.is_dir() {
            if !SKIPPED_DIRECTORIES.contains(&name) {
                walk(repo_root, &rel, files)?;
            }
        } else {
            files.push(rel);
        }
    }
    Ok(())
}

fn is_dockerfile(name: &str) -> bool {
    name == "Dockerfile" || name.starts_with("Dockerfile.") || name.ends_with(".Dockerfile")
}

fn is_workflow(rel: &Path) -> bool {
    let Some(name) = rel.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    (name.ends_with(".yml") || name.ends_with(".yaml"))
        && manifest_dir(rel) == Path::new(".github/workflows")
}

/// Which Cargo auto-discovered target, if any, this file is.
///
/// Anchored on the directory of a `Cargo.toml`, because `src/main.rs` is only
/// Cargo's default binary when there is a crate around it.
fn cargo_implicit_key(rel: &Path, crate_dirs: &BTreeSet<PathBuf>) -> Option<&'static str> {
    for crate_dir in crate_dirs {
        let Ok(inner) = rel.strip_prefix(crate_dir) else {
            continue;
        };
        let inner = inner.to_string_lossy();
        let key = match inner.as_ref() {
            "src/main.rs" => "cargo:default-bin",
            "src/lib.rs" => "cargo:default-lib",
            "build.rs" => "cargo:build-script",
            // Cargo's two `src/bin` layouts, and only those: a module inside a
            // multi-file binary is part of a target, not a target.
            other
                if other.starts_with("src/bin/")
                    && ((other.ends_with(".rs") && other.matches('/').count() == 2)
                        || (other.ends_with("/main.rs") && other.matches('/').count() == 3)) =>
            {
                "cargo:src-bin"
            }
            _ => continue,
        };
        return Some(key);
    }
    None
}
