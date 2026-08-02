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

use saphyr_parser::{Event, Marker, Parser, ScalarStyle, ScanError};
use std::collections::{BTreeMap, BTreeSet};
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
    /// A path a manifest declared that this scan could not point at anything in
    /// the repository: it names nothing on disk, it escapes the repository, or
    /// — for a `COPY` source — the build context it is relative to is not
    /// declared anywhere this module reads.
    ///
    /// The declaration is kept verbatim, because it is still a declaration: a
    /// `"main": "dist/index.js"` is what npm will publish whether or not `dist/`
    /// has been built yet. What is *not* kept is the claim that it resolves.
    /// Every root a manifest declares is Tier A — machine-declared — and the
    /// out-of-sample corpus found 99 Tier A roots naming paths that did not
    /// exist, spelled exactly like the ones that did (§4.3). A reader could not
    /// tell them apart, and neither could a caller.
    Unresolved(String),
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
            RootTarget::Glob(s)
            | RootTarget::Command(s)
            | RootTarget::Reference(s)
            | RootTarget::Unresolved(s) => f.write_str(s),
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
// A checklist that are written in TOML, and both are read with the `toml`
// crate — the parser Cargo is itself built on, so a manifest Cargo accepts is a
// manifest this module accepts.
//
// What stood here was a hand-written parser for the subset of TOML those two
// files were assumed to need, and a subset of somebody else's file format is a
// list of valid files we reject. It rejected ripgrep's `Cargo.toml`, whose
// Debian `extended-description` opens with `"""\` — TOML 1.0 line continuation,
// which trims the newline and the next line's leading whitespace — and all 76
// of that repository's Tier A roots went with it.
//
// Strictness is unchanged and still the whole point (§6.20): `toml` refuses a
// malformed document rather than handing back a partial table, and a partial
// table is indistinguishable from a manifest that declares nothing.

/// Parse a TOML manifest, naming the file and the line when it will not parse.
fn parse_toml(path: &Path, content: &str) -> ManifestResult<toml::Value> {
    // `from_str`, not `Value::from_str`: the latter parses a single TOML
    // *value*, and a manifest is a document.
    toml::from_str::<toml::Value>(content).map_err(|err| match err.span() {
        Some(span) => ManifestError::at_line(path, line_at(content, span.start), err.message()),
        None => ManifestError::parse(path, err.message()),
    })
}

/// The 1-based line holding byte `offset`.
fn line_at(content: &str, offset: usize) -> usize {
    content[..offset.min(content.len())]
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count()
        + 1
}

/// What the manifest actually holds, for an error that says so rather than
/// saying only that a key was wrong.
fn toml_type_name(value: &toml::Value) -> &'static str {
    match value {
        toml::Value::String(_) => "a string",
        toml::Value::Integer(_) => "an integer",
        toml::Value::Float(_) => "a float",
        toml::Value::Boolean(_) => "a boolean",
        toml::Value::Datetime(_) => "a datetime",
        toml::Value::Array(_) => "an array",
        toml::Value::Table(_) => "a table",
    }
}

/// Follow a key path, stopping at the first segment the document does not have.
fn toml_path<'a>(value: &'a toml::Value, path: &[&str]) -> Option<&'a toml::Value> {
    let mut node = value;
    for key in path {
        node = node.get(key)?;
    }
    Some(node)
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

fn toml_str<'a>(path: &Path, key: &str, value: &'a toml::Value) -> ManifestResult<&'a str> {
    value.as_str().ok_or_else(|| {
        ManifestError::at_key(
            path,
            key,
            format!("must be a string, found {}", toml_type_name(value)),
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
        if let Some(node) = toml_path(&doc, &["project", table]) {
            let entries = expect_toml_table(manifest, &format!("project.{table}"), node)?;
            for (name, value) in entries {
                let key = format!("project.{table}.{}", toml_key_segment(name));
                let text = toml_str(manifest, &key, value)?;
                out.push(kind, manifest, key, RootTarget::Reference(text.to_string()));
            }
        }
    }

    if let Some(node) = toml_path(&doc, &["project", "entry-points"]) {
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
    value: &'a toml::Value,
) -> ManifestResult<&'a toml::Table> {
    value.as_table().ok_or_else(|| {
        ManifestError::at_key(
            manifest,
            key,
            format!("must be a table, found {}", toml_type_name(value)),
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
                format!("must be an array of tables, found {}", toml_type_name(node)),
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
    entries: &toml::Table,
    kind: RootKind,
    out: &mut ManifestRoots,
) -> ManifestResult<()> {
    if let Some(path) = entries.get("path") {
        let key = key_join(key, "path");
        let text = toml_str(manifest, &key, path)?;
        out.push(kind, manifest, key, rel_path(dir, text));
    } else if let Some(name) = entries.get("name") {
        let key = key_join(key, "name");
        let text = toml_str(manifest, &key, name)?;
        out.push(kind, manifest, key, RootTarget::Reference(text.to_string()));
    }
    Ok(())
}

fn push_crate_type_declaration(
    manifest: &Path,
    key: &str,
    entries: &toml::Table,
    out: &mut ManifestRoots,
) -> ManifestResult<()> {
    // Cargo accepts both spellings; the key recorded is the one written.
    for spelling in ["crate-type", "crate_type"] {
        let Some(value) = entries.get(spelling) else {
            continue;
        };
        let key = key_join(key, spelling);
        let items = value.as_array().ok_or_else(|| {
            ManifestError::at_key(
                manifest,
                &key,
                format!("must be an array, found {}", toml_type_name(value)),
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
///
/// The list is a moving target and being one release behind it is a defect, not
/// a limitation: `godebug` has existed since Go 1.23, and rejecting it cost
/// sample-controller — whose `go.mod` is generated, and carries one — its
/// entire Tier A root set. `tool` arrived in 1.24 and `ignore` in 1.25.
const GO_MOD_DIRECTIVES: [&str; 10] = [
    "module",
    "go",
    "toolchain",
    "godebug",
    "require",
    "replace",
    "exclude",
    "retract",
    "tool",
    "ignore",
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
/// they are recorded as globs because Docker matches them as patterns. Two
/// sources are deliberately skipped, for the same reason in both cases —
/// recording them would manufacture a root for a file that is not here:
///
/// - `COPY --from=<stage>` reads out of an earlier build stage's filesystem.
/// - `ADD https://…` fetches over the network. It used to be rebased like a
///   relative path, which produced roots spelled
///   `src/ad/https:/github.com/…/opentelemetry-javaagent.jar`.
///
/// # The build context is not in the Dockerfile
///
/// A `COPY` source is resolved by Docker against the **build context**, and
/// nothing in a Dockerfile says what the context is — it is an argument to
/// `docker build`, or `build.context` in a compose file. Assuming the context is
/// the Dockerfile's own directory is right for `docker build path/to/svc` and
/// wrong for a monorepo that builds every service from the root, which is how
/// 99 of otel-demo's 130 packaged-file roots came to name paths that do not
/// exist (`COPY ./src/ad/settings.gradle*` in `src/ad/Dockerfile` became
/// `src/ad/src/ad/settings.gradle*`).
///
/// So the context is *resolved rather than assumed*, by
/// [`docker_build_context`]: the two candidates are the Dockerfile's directory
/// and the repository root, and the one under which the sources actually name
/// something wins. When neither does, the source is recorded verbatim as a
/// [`RootTarget::Unresolved`] — the Dockerfile's own spelling, with no context
/// invented for it.
///
/// This function has no tree to ask, so it resolves nothing: a Dockerfile at
/// the repository root needs no resolution (both candidates are the same
/// directory), and for any other Dockerfile the sources come back unresolved.
/// [`scan`] passes the real tree and gets the real answer.
///
/// Keys are `<instruction>@<line>`, optionally with a source index —
/// `Dockerfile#copy@4[1]`. A Dockerfile has no other addressable structure, and
/// the line is what a human checks against the file.
pub fn parse_dockerfile(manifest: &Path, content: &str) -> ManifestResult<ManifestRoots> {
    parse_dockerfile_in(manifest, content, &|_| false)
}

/// One `COPY`/`ADD` source, or one `CMD`/`ENTRYPOINT`, held until the build
/// context is known.
///
/// The context cannot be decided until every source has been read, and the
/// roots must still come out in the order the file declares them, so the
/// instructions are collected first and turned into roots afterwards.
enum DockerRoot {
    Entry { key: String, command: String },
    Source { key: String, source: String },
}

/// [`parse_dockerfile`], given a way to ask whether a repo-relative path names
/// something in the repository. See that function for everything else.
fn parse_dockerfile_in(
    manifest: &Path,
    content: &str,
    exists: &dyn Fn(&str) -> bool,
) -> ManifestResult<ManifestRoots> {
    let mut out = ManifestRoots::from_source(manifest);
    let mut pending: Vec<DockerRoot> = Vec::new();
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
                    push_docker_copy(manifest, start_line, &upper, rest, &mut pending)?;
                }
                "CMD" | "ENTRYPOINT" => {
                    let command = docker_command(manifest, start_line, rest)?;
                    pending.push(DockerRoot::Entry {
                        key: format!("{}@{start_line}", upper.to_ascii_lowercase()),
                        command,
                    });
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

    let sources: Vec<&str> = pending
        .iter()
        .filter_map(|p| match p {
            DockerRoot::Source { source, .. } => Some(source.as_str()),
            DockerRoot::Entry { .. } => None,
        })
        .collect();
    let context = docker_build_context(manifest_dir(manifest), &sources, exists);

    for item in pending {
        match item {
            DockerRoot::Entry { key, command } => out.push(
                RootKind::ContainerEntry,
                manifest,
                key,
                RootTarget::Command(command),
            ),
            DockerRoot::Source { key, source } => {
                let target = match &context {
                    // `COPY . .` resolves to the context directory itself, which
                    // at the repository root is `.`. It used to resolve to the
                    // empty string, and an empty target is not a root.
                    Some(context) => match join_rel(Path::new(context), &source) {
                        empty if empty.is_empty() => RootTarget::Path(PathBuf::from(".")),
                        joined => RootTarget::Glob(joined),
                    },
                    None => RootTarget::Unresolved(source),
                };
                out.push(RootKind::PackagedFile, manifest, key, target);
            }
        }
    }
    Ok(out)
}

/// Which directory this Dockerfile's `COPY` sources are relative to, when the
/// tree can settle it.
///
/// There are two candidates and no declaration. `docker build path/to/svc`
/// makes the context the Dockerfile's own directory; `docker build .` with
/// `-f path/to/svc/Dockerfile`, or a compose file with `context: ./`, makes it
/// the repository root. Both are ordinary, and picking one by fiat is what
/// produced §4.3's 99 roots naming nothing.
///
/// So each candidate is scored by how many of the sources name something real
/// under it, and the winner is the one the repository agrees with. A tie goes
/// to the Dockerfile's directory: it is the narrower claim, and it is the
/// reading `docker build <dir>` gives. **A score of zero on both is not a tie
/// but an answer** — the sources name nothing either way, so which context was
/// meant is genuinely unknown and `None` says so.
///
/// A Dockerfile at the repository root needs none of this: the two candidates
/// are the same directory, so there is nothing to resolve and nothing that
/// could be doubled.
fn docker_build_context(
    dir: &Path,
    sources: &[&str],
    exists: &dyn Fn(&str) -> bool,
) -> Option<String> {
    let dir = dir.to_string_lossy().into_owned();
    if dir.is_empty() {
        return Some(String::new());
    }
    let score = |context: &str| -> usize {
        sources
            .iter()
            .filter(|source| exists(lookup_prefix(&join_rel(Path::new(context), source))))
            .count()
    };
    let (here, root) = (score(&dir), score(""));
    match (here, root) {
        (0, 0) => None,
        (here, root) if root > here => Some(String::new()),
        _ => Some(dir),
    }
}

/// The longest leading run of a target that can be looked up as a path.
///
/// A pattern cannot be stat-ed, but the directory it lives in can, and that is
/// enough to tell `src/ad/gradlew*` (which probes `src/ad`) from
/// `src/ad/src/ad/gradlew*` (which probes `src/ad/src`). A segment carrying a
/// glob metacharacter or an unexpanded build argument ends the run, so
/// `.build/${OS}-${ARCH}/node_exporter` probes `.build`. Returning `""` — the
/// repository root, which always exists — means the very first segment was
/// already a pattern, and there is nothing to check.
fn lookup_prefix(target: &str) -> &str {
    let mut end = 0;
    let mut cursor = 0;
    for segment in target.split('/') {
        if segment.contains(['*', '?', '[', '$']) {
            return &target[..end];
        }
        cursor += segment.len();
        end = cursor;
        cursor += 1; // the `/` that follows it
    }
    target
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
    out: &mut Vec<DockerRoot>,
) -> ManifestResult<()> {
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
        // A remote source is fetched over the network at build time. It is not
        // a file in this repository, and rebasing it produced targets spelled
        // `src/ad/https:/github.com/…`.
        if is_remote_source(source) {
            continue;
        }
        out.push(DockerRoot::Source {
            key: key_index(&key, index),
            source: source.clone(),
        });
    }
    Ok(())
}

/// Whether an `ADD` source is a URL rather than a path in the build context.
///
/// Docker accepts `http://`, `https://` and Git remotes here. The test is for
/// any `scheme://` prefix, because what matters is that it is not a local path,
/// not which protocol fetches it.
fn is_remote_source(source: &str) -> bool {
    let Some(scheme) = source.split_once("://").map(|(scheme, _)| scheme) else {
        return false;
    };
    !scheme.is_empty()
        && scheme.starts_with(|c: char| c.is_ascii_alphabetic())
        && scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
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
// §5.2 asks for `.github/workflows/*.yml` `run:` bodies and `uses:` paths, and
// the reading is done by `saphyr-parser`: a YAML 1.2 parser, pure Rust, no
// `unsafe`, and the maintained line of descent from `yaml-rust`.
//
// It is used as an event stream rather than through a value tree, for two
// reasons. Nothing here wants YAML's type resolution — `on:` is a key GitHub
// reads, not the boolean `true`, and every value this module records is
// transcribed as written. And a value tree resolves anchors and aliases
// silently, where this module wants to refuse them: GitHub does not expand
// anchors in a workflow, so a file using one does not mean here what it says.
//
// What stood here was a hand-written parser for the subset of YAML workflows
// were assumed to use. It read a trailing comment as a key's value and it could
// not read a flow mapping, and between them those two rejected five of the nine
// repositories in the out-of-sample corpus. A construct we do not model is
// still an error carrying a line number, never silence — but "do not model" now
// means anchors, aliases and tags, and no longer means "written on one line
// instead of three".

#[derive(Debug, Clone, PartialEq)]
enum Yaml {
    Scalar(String),
    Seq(Vec<Yaml>),
    Map(Vec<(String, Yaml)>),
    Empty,
}

/// A collection under construction: its children so far, and — for a mapping —
/// the key still waiting for its value.
enum Frame {
    Seq(Vec<Yaml>),
    Map(Vec<(String, Yaml)>, Option<String>),
}

/// Parse a YAML document into [`Yaml`].
///
/// Mappings keep the order they were written in, because `-printseeds` output
/// a human checks against the file has to read top to bottom.
fn parse_yaml(path: &Path, content: &str) -> ManifestResult<Yaml> {
    let mut stack: Vec<Frame> = Vec::new();
    let mut document = Yaml::Empty;
    let mut documents = 0usize;

    for event in Parser::new_from_str(content) {
        let (event, span) = event.map_err(|err| {
            ManifestError::at_line(path, err.marker().line(), yaml_detail(content, &err))
        })?;
        let line = span.start.line();

        let node = match event {
            Event::Nothing | Event::StreamStart | Event::StreamEnd | Event::DocumentEnd => continue,
            Event::DocumentStart(_) => {
                documents += 1;
                if documents > 1 {
                    return Err(ManifestError::at_line(
                        path,
                        line,
                        "more than one document in a workflow file is not modelled",
                    ));
                }
                continue;
            }
            // An alias is only ever a reference to an anchor, so both spellings
            // of the construct land on the same sentence.
            Event::Alias(_) => {
                return Err(ManifestError::at_line(
                    path,
                    line,
                    "anchors and aliases are not modelled in a workflow file",
                ))
            }
            Event::Scalar(value, style, anchor, tag) => {
                reject_anchor_or_tag(path, line, anchor, tag.is_some())?;
                if style == ScalarStyle::Plain && is_yaml_null(&value) {
                    Yaml::Empty
                } else {
                    Yaml::Scalar(value.into_owned())
                }
            }
            Event::SequenceStart(anchor, tag) => {
                reject_anchor_or_tag(path, line, anchor, tag.is_some())?;
                stack.push(Frame::Seq(Vec::new()));
                continue;
            }
            Event::MappingStart(anchor, tag) => {
                reject_anchor_or_tag(path, line, anchor, tag.is_some())?;
                stack.push(Frame::Map(Vec::new(), None));
                continue;
            }
            Event::SequenceEnd => match stack.pop() {
                Some(Frame::Seq(items)) => Yaml::Seq(items),
                _ => return Err(unbalanced(path, line)),
            },
            Event::MappingEnd => match stack.pop() {
                Some(Frame::Map(entries, None)) => Yaml::Map(entries),
                _ => return Err(unbalanced(path, line)),
            },
        };

        match stack.last_mut() {
            None => document = node,
            Some(Frame::Seq(items)) => items.push(node),
            Some(Frame::Map(entries, pending)) => match pending.take() {
                Some(key) => entries.push((key, node)),
                None => match node {
                    Yaml::Scalar(key) => *pending = Some(key),
                    _ => {
                        return Err(ManifestError::at_line(
                            path,
                            line,
                            "a mapping key that is not a scalar is not modelled",
                        ))
                    }
                },
            },
        }
    }

    Ok(document)
}

/// A collection that ended where none had begun, or a mapping that ended on a
/// key with no value. The parser does not produce either, and if it ever does
/// the answer is an error rather than a half-read document.
fn unbalanced(path: &Path, line: usize) -> ManifestError {
    ManifestError::at_line(path, line, "a collection ended where none was open")
}

fn reject_anchor_or_tag(
    path: &Path,
    line: usize,
    anchor: usize,
    tagged: bool,
) -> ManifestResult<()> {
    if anchor != 0 {
        return Err(ManifestError::at_line(
            path,
            line,
            "anchors and aliases are not modelled in a workflow file",
        ));
    }
    if tagged {
        return Err(ManifestError::at_line(
            path,
            line,
            "an explicit tag is not modelled in a workflow file",
        ));
    }
    Ok(())
}

/// YAML 1.2 core-schema null.
///
/// A key written with no value at all (`pull_request:`) reaches us as a plain
/// `~`, so this is also what keeps an empty `run:` from being read as the
/// command `~`: an absent value is [`Yaml::Empty`], and a `run` that is not a
/// scalar is an error rather than a root pointing at nothing.
fn is_yaml_null(value: &str) -> bool {
    matches!(value, "" | "~" | "null" | "Null" | "NULL")
}

/// Say what the scanner found in this module's vocabulary, keeping its own
/// words in parentheses.
///
/// The classification matters because a caller has to be able to tell a
/// construct we deliberately refuse from a file that is simply broken. Each
/// branch is a fact about the input rather than a guess: the last one asks
/// whether the failing line is indented to a column no earlier line opened, and
/// leaves the scanner's wording alone when it is not.
fn yaml_detail(content: &str, err: &ScanError) -> String {
    let info = err.info();
    if info.contains("anchor") || info.contains("alias") {
        return format!("anchors and aliases are not modelled in a workflow file ({info})");
    }
    if info.contains("tab") {
        return info.to_string();
    }
    if info.contains("quoted scalar") && info.contains("end of stream") {
        return format!("unterminated quoted scalar ({info})");
    }
    if is_unopened_column(content, err.marker()) {
        return format!("unexpected indentation ({info})");
    }
    info.to_string()
}

/// Whether the failing line sits at a column no earlier line indented to.
///
/// A dedent onto a column that was never opened is reported by the scanner as
/// "did not find expected key", which is true and useless: §6.20 wants a
/// message a human can act on, and the corpus quotes these messages back at
/// their authors. A line at a column that *is* open failed for some other
/// reason and keeps the scanner's own wording.
fn is_unopened_column(content: &str, marker: &Marker) -> bool {
    let mut opened = BTreeSet::new();
    for (index, raw) in content.lines().enumerate() {
        if index + 1 >= marker.line() {
            break;
        }
        let text = raw.trim_start_matches(' ');
        if text.is_empty() || text.starts_with('#') {
            continue;
        }
        opened.insert(raw.len() - text.len());
    }
    !opened.contains(&marker.col())
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
    let document = parse_yaml(manifest, content)?;
    let mut out = ManifestRoots::from_source(manifest);
    collect_workflow_roots(manifest, &document, "", &mut out)?;
    Ok(out)
}

/// Whether a `run` key at this position is a `defaults` block rather than a
/// step's command.
///
/// The Actions schema spells two different things `run`: a step's `run` is the
/// command, and `defaults.run` — at the workflow or at a job — is a mapping of
/// `shell` and `working-directory` that applies to every step under it. Reading
/// the second as a command rejected cobra's workflow, and with it every root in
/// the repository. The distinction is positional in the schema, so it is
/// positional here: `key` is the path of the *holder* of the `run` key.
fn is_defaults_block(key: &str) -> bool {
    key == "defaults" || key.ends_with(".defaults")
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
                    "run" if is_defaults_block(key) => {
                        collect_workflow_roots(manifest, value, &child, out)?;
                    }
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
///
/// **Every target is resolved against the tree before it is returned.** The
/// parsers say what a manifest declares; only a scan can say whether the
/// declaration names anything, and [`resolve_against_tree`] is where the two
/// meet.
pub fn scan(repo_root: &Path) -> ManifestResult<ManifestRoots> {
    let mut files = Vec::new();
    walk(repo_root, Path::new(""), &mut files)?;
    files.sort();

    let crates = cargo_crates(repo_root, &files)?;

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
            _ if is_workflow(rel) => Some(parse_github_workflow as fn(&Path, &str) -> _),
            _ if name.ends_with(".go") => Some(parse_go_source as fn(&Path, &str) -> _),
            _ => None,
        };
        let read = || {
            std::fs::read_to_string(&absolute).map_err(|source| ManifestError::Read {
                path: rel.clone(),
                source,
            })
        };
        if let Some(parse) = parsed {
            out.merge(parse(rel, &read()?)?);
        } else if is_dockerfile(name) {
            // The only parser that needs the tree while it parses: which
            // directory a `COPY` source is relative to is a question about the
            // repository, not about the Dockerfile.
            out.merge(parse_dockerfile_in(rel, &read()?, &|path| {
                present(repo_root, Path::new(path))
            })?);
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

        if let Some(key) = cargo_implicit_key(rel, &crates) {
            let kind = match key {
                "cargo:default-lib" => RootKind::LibraryEntry,
                "cargo:build-script" => RootKind::BuildHook,
                "cargo:test" | "cargo:bench" | "cargo:example" => RootKind::DevTarget,
                _ => RootKind::Executable,
            };
            out.push(kind, rel, key.to_string(), RootTarget::Path(rel.clone()));
        }
    }
    resolve_against_tree(repo_root, &mut out);
    Ok(out)
}

/// Whether a repo-relative path names something in this repository.
///
/// `symlink_metadata` rather than `exists`, so that a symlink counts as present
/// without being followed — following one can leave the repository, which is
/// the same reason [`walk`] refuses to descend through them.
///
/// A path made only of `.` (or of nothing) is the repository root, which is
/// present by construction. A path carrying `..`, a leading separator or a
/// Windows prefix has left the repository, and whatever sits outside it is not
/// this repository's root — the question is not even asked of the filesystem,
/// so that `repo_root.join(..)` can never be made to stat somewhere else.
fn present(repo_root: &Path, rel: &Path) -> bool {
    let mut names_something = false;
    for component in rel.components() {
        match component {
            std::path::Component::Normal(_) => names_something = true,
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir
            | std::path::Component::RootDir
            | std::path::Component::Prefix(_) => return false,
        }
    }
    if !names_something {
        return true;
    }
    std::fs::symlink_metadata(repo_root.join(rel)).is_ok()
}

/// Settle every target against the tree that was just walked.
///
/// A manifest declares; only the repository can confirm. The corpus found 99
/// Tier A roots naming paths that do not exist, spelled indistinguishably from
/// the ones that do (§4.3) — so this is where the three outcomes are separated,
/// and it is deliberately the *only* place, so that no parser has to be trusted
/// to have got it right:
///
/// - **[`RootTarget::Path`]** — the target names something in the tree. This is
///   the only shape that reaches a caller as a resolved path.
/// - **[`RootTarget::Glob`]** — the target is a pattern, and the directory it
///   would be matched in exists. It is still not expanded: which files a
///   pattern selects is a question with a different answer on every checkout.
/// - **[`RootTarget::Unresolved`]** — everything else. The declaration is kept
///   verbatim; the claim that it points at a file is dropped.
///
/// A pattern whose parent directory *is* present stays a pattern rather than
/// becoming a path, and a plain path that is present becomes one even if a
/// parser recorded it as a glob: `COPY ./pb/` is matched by Docker as a pattern
/// and names exactly one directory, and reporting `pb` as a path a caller can
/// open is more useful than reporting it as a pattern nobody will expand.
///
/// [`RootTarget::Command`] and [`RootTarget::Reference`] are left alone. A
/// command line is not a path and a module path is not a path, and stat-ing
/// either is how a cleaner "resolves" `npm run build` to a file called `npm`.
fn resolve_against_tree(repo_root: &Path, out: &mut ManifestRoots) {
    for root in &mut out.roots {
        match &root.target {
            // Checked as a path rather than as text, so that a filename this
            // platform allows but UTF-8 does not is answered about accurately
            // instead of being demoted by a lossy conversion.
            RootTarget::Path(path) => {
                if !present(repo_root, path) {
                    root.target = RootTarget::Unresolved(path.to_string_lossy().into_owned());
                }
            }
            RootTarget::Glob(glob) => {
                let glob = glob.clone();
                let prefix = lookup_prefix(&glob);
                if prefix == glob {
                    // Not a pattern at all, whatever the parser called it.
                    root.target = match present(repo_root, Path::new(&glob)) {
                        true => RootTarget::Path(PathBuf::from(glob)),
                        false => RootTarget::Unresolved(glob),
                    };
                } else if !present(repo_root, Path::new(prefix)) {
                    root.target = RootTarget::Unresolved(glob);
                }
            }
            RootTarget::Command(_) | RootTarget::Reference(_) | RootTarget::Unresolved(_) => {}
        }
    }
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
            // The name list is noise filtering (node_modules, target). §9.3 0b
            // is a different question: a linked worktree and a submodule carry
            // `.git` as a FILE and a bare `vendor/foo.git/` carries none, so a
            // name match alone let Tier A read another repository's manifests
            // and materialize roots from them.
            if !SKIPPED_DIRECTORIES.contains(&name)
                && !crate::boundary::classify(&repo_root.join(&rel)).stops_the_walk()
            {
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

/// Which of Cargo's four target auto-discovery sweeps a crate leaves switched
/// on.
///
/// Every one can be turned off in the manifest, and ripgrep — the first
/// repository in the out-of-sample corpus — turns `autotests` off and declares
/// the one test target it wants. Sweeping `tests/*.rs` regardless would invent
/// targets Cargo does not build, which is §9.5's fabricated root: a longer list
/// that hides a real gap rather than closing one.
#[derive(Debug, Clone, Copy)]
struct CargoAutoDiscovery {
    bins: bool,
    examples: bool,
    tests: bool,
    benches: bool,
}

impl CargoAutoDiscovery {
    /// Read the four `[package]` switches. A switch that is present but not a
    /// boolean is an error, like every other mistyped key in this module: it
    /// means we do not know which targets this crate has.
    fn read(manifest: &Path, doc: &toml::Value) -> ManifestResult<CargoAutoDiscovery> {
        let switch = |name: &str| match toml_path(doc, &["package", name]) {
            None => Ok(true),
            Some(toml::Value::Boolean(value)) => Ok(*value),
            Some(other) => Err(ManifestError::at_key(
                manifest,
                format!("package.{name}"),
                format!("must be a boolean, found {}", toml_type_name(other)),
            )),
        };
        Ok(CargoAutoDiscovery {
            bins: switch("autobins")?,
            examples: switch("autoexamples")?,
            tests: switch("autotests")?,
            benches: switch("autobenches")?,
        })
    }
}

/// Every crate directory in the tree, with the auto-discovery its manifest
/// leaves on.
///
/// A manifest with no `[package]` is a workspace file, and a workspace file has
/// no targets: Cargo discovers nothing beside it, so neither do we.
fn cargo_crates(
    repo_root: &Path,
    files: &[PathBuf],
) -> ManifestResult<BTreeMap<PathBuf, CargoAutoDiscovery>> {
    let mut crates = BTreeMap::new();
    for rel in files {
        if rel.file_name() != Some(std::ffi::OsStr::new("Cargo.toml")) {
            continue;
        }
        let content =
            std::fs::read_to_string(repo_root.join(rel)).map_err(|source| ManifestError::Read {
                path: rel.clone(),
                source,
            })?;
        let doc = parse_toml(rel, &content)?;
        if doc.get("package").is_none() {
            continue;
        }
        crates.insert(
            manifest_dir(rel).to_path_buf(),
            CargoAutoDiscovery::read(rel, &doc)?,
        );
    }
    Ok(crates)
}

/// Which Cargo auto-discovered target, if any, this file is.
///
/// Anchored on the directory of a `Cargo.toml`, because `src/main.rs` is only
/// Cargo's default binary when there is a crate around it.
///
/// §5.2 asks for `[[bin]]`, `[[example]]`, `[[bench]]` and `[[test]]`, and
/// Cargo finds all four on disk with no key naming them. A test binary nothing
/// declares is still a binary Cargo builds, and the file that holds it is still
/// an entry point with no caller.
fn cargo_implicit_key(
    rel: &Path,
    crates: &BTreeMap<PathBuf, CargoAutoDiscovery>,
) -> Option<&'static str> {
    for (crate_dir, auto) in crates {
        let Ok(inner) = rel.strip_prefix(crate_dir) else {
            continue;
        };
        let inner = inner.to_string_lossy();
        let key = match inner.as_ref() {
            "src/main.rs" if auto.bins => "cargo:default-bin",
            "src/lib.rs" => "cargo:default-lib",
            "build.rs" => "cargo:build-script",
            other if auto.bins && is_cargo_target(other, "src/bin/") => "cargo:src-bin",
            other if auto.tests && is_cargo_target(other, "tests/") => "cargo:test",
            other if auto.benches && is_cargo_target(other, "benches/") => "cargo:bench",
            other if auto.examples && is_cargo_target(other, "examples/") => "cargo:example",
            _ => continue,
        };
        return Some(key);
    }
    None
}

/// Cargo's two layouts for a target directory, and only those: `<dir>/name.rs`
/// and `<dir>/name/main.rs`.
///
/// Anything else under the directory is part of a target rather than a target —
/// a module beside a multi-file target's `main.rs`, or the `tests/common/mod.rs`
/// that test files share fixtures through.
fn is_cargo_target(inner: &str, dir: &str) -> bool {
    let Some(rest) = inner.strip_prefix(dir) else {
        return false;
    };
    (rest.ends_with(".rs") && !rest.contains('/'))
        || (rest.ends_with("/main.rs") && rest.matches('/').count() == 1)
}
