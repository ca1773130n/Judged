//! Roots declared **in source**, by a marker a toolchain reads (§5.2).
//!
//! Tier A of §5.1 is *"a build system or deploy target already reads this file
//! to find roots"*, and nothing in that sentence requires the file to be a
//! manifest. The Go toolchain reads `//export`. The linker keeps a
//! `#[no_mangle]` symbol. CPython's `site` module executes a `.pth` file's
//! `import` lines before any user code runs. Each of those is a machine-declared
//! entry point that [`super::manifest`] cannot see, because there is no manifest
//! involved.
//!
//! §5.2 lists every one of these and the R1 determination's §7 item 4 names the
//! set this module implements:
//!
//! > Root-set coverage for the sources §5.2 names and the implementation lacks:
//! > `//go:linkname` and `//export` (Go), `.pth` and `sitecustomize.py`
//! > (Python), `#[no_mangle]` / `#[used]` / `#[ctor]` (Rust) …
//!
//! # Why these are Tier A and not Tier B
//!
//! §5.1's Tier B is *convention-inferable* — "correct only if the framework and
//! its version were detected correctly", which is a guess and is labelled as
//! one. None of these is a guess. `#[no_mangle]` does not mean the symbol is
//! probably exported; it means the linker will emit it under that name. The
//! marker **is** the declaration, and reading it wrongly is a parsing bug rather
//! than a mis-detected framework.
//!
//! # This overlaps Gate 3f, and the overlap is the design
//!
//! [`crate::gate3f`] refuses a claim whose symbol is exported across an ABI
//! boundary, reading some of the same markers. It is not the same question.
//! 3f asks *what does it cost to be wrong about deleting this* and answers with
//! a refusal; the root set asks *was this declared an entry point* and answers
//! with a provenance-carrying root. §9.3 and §5 keep them apart deliberately,
//! and the report says which layer earned a rescue precisely so the two are not
//! read as one signal counted twice.
//!
//! # What it deliberately does not read
//!
//! `//go:embed`, `//go:wasmexport` and `#[wasm_bindgen]`/`#[pyo3::pymodule]` are
//! all in §5.2 and all absent here, because §7 item 4 does not name them and a
//! root source added without a fixture that exercises it is a rule nothing
//! measures. They are listed in the module's tests as unimplemented rather than
//! left to be rediscovered.

use std::fmt;
use std::path::{Path, PathBuf};

use crate::{Error, Result};

/// Which §5.2 marker declared this root.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Marker {
    /// `//go:linkname local importpath.remote` — §4.1 records that this is why
    /// `x/tools/cmd/deadcode` reports a symbol *"spuriously as dead"*: the alias
    /// is resolved by the linker and appears in no call graph.
    GoLinkname,
    /// `//export Name` — a cgo entry point. The consumer is outside the build.
    GoExport,
    /// `#[no_mangle]` — the symbol is emitted under exactly that name for a
    /// consumer that is not in this crate graph.
    RustNoMangle,
    /// `#[used]` — the linker is instructed to keep a static nothing references,
    /// which is how `inventory`, `linkme` and `.init_array` registration work.
    RustUsed,
    /// `#[export_name = "..."]` — as `no_mangle`, with the name given.
    RustExportName,
    /// `#[ctor]` — runs before `main`, referenced by nothing.
    RustCtor,
    /// A `.pth` file. §5.2: *"whose lines beginning with `import` are executed at
    /// interpreter start (`site` module semantics). A `.pth` file is an entry
    /// point with no caller anywhere."*
    PythonPth,
    /// `sitecustomize.py` / `usercustomize.py` — imported by `site` at
    /// interpreter start, named by nothing.
    PythonSiteCustomize,
}

impl Marker {
    /// Stable lower-case rule label, for reports.
    pub fn as_str(self) -> &'static str {
        match self {
            Marker::GoLinkname => "go/linkname",
            Marker::GoExport => "go/export",
            Marker::RustNoMangle => "rust/no-mangle",
            Marker::RustUsed => "rust/used",
            Marker::RustExportName => "rust/export-name",
            Marker::RustCtor => "rust/ctor",
            Marker::PythonPth => "python/pth",
            Marker::PythonSiteCustomize => "python/sitecustomize",
        }
    }

    /// Who reads this marker, in a phrase a report can put after "declared by".
    pub fn reader(self) -> &'static str {
        match self {
            Marker::GoLinkname => "the Go linker, which binds the alias with no call site",
            Marker::GoExport => "cgo, for a consumer outside this build",
            Marker::RustNoMangle | Marker::RustExportName => {
                "the linker, which emits the symbol under that exact name"
            }
            Marker::RustUsed => "the linker, which is told to keep a static nothing references",
            Marker::RustCtor => "the platform loader, before main",
            Marker::PythonPth => "CPython's `site` module, at interpreter start",
            Marker::PythonSiteCustomize => "CPython's `site` module, which imports it by name",
        }
    }
}

impl fmt::Display for Marker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One in-source root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InSourceRoot {
    marker: Marker,
    file: PathBuf,
    line: usize,
    symbol: Option<String>,
    target: String,
}

impl InSourceRoot {
    /// Which marker declared it.
    pub fn marker(&self) -> Marker {
        self.marker
    }

    /// The file carrying the marker, repo-relative.
    pub fn file(&self) -> &Path {
        &self.file
    }

    /// The 1-based line, so a reader can open the file at the declaration.
    pub fn line(&self) -> usize {
        self.line
    }

    /// The symbol declared a root, when the marker names or implies one.
    ///
    /// `None` for a marker whose root is the **file** — a `.pth` and a
    /// `sitecustomize.py` are entry points in themselves.
    pub fn symbol(&self) -> Option<&str> {
        self.symbol.as_deref()
    }

    /// What the declaration points at, as written.
    pub fn target(&self) -> &str {
        &self.target
    }

    /// The origin, spelled `file:line`, for a report a reader can check.
    pub fn origin(&self) -> String {
        format!("{}:{}", self.file.display(), self.line)
    }
}

/// Every in-source root under `root`.
///
/// Errors only on a directory that cannot be listed. A **file** that cannot be
/// read is skipped rather than fatal: unlike the Tier A manifest scan, where one
/// unreadable manifest means every machine-declared root is missing, a source
/// file that will not decode costs exactly the roots declared in that file. The
/// caller records the difference — see `judged-mutants`' `GapKind`.
pub fn scan(root: &Path) -> Result<Vec<InSourceRoot>> {
    let mut found = Vec::new();
    for absolute in walk(root)? {
        let relative = absolute
            .strip_prefix(root)
            .unwrap_or(&absolute)
            .to_path_buf();
        let Ok(text) = std::fs::read_to_string(&absolute) else {
            continue;
        };
        let name = relative
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        let extension = relative
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default();

        match extension {
            // Code scanners get the text with string-literal lines blanked. A
            // `.pth` is not code and has no literals to confuse.
            "go" => scan_go(
                &relative,
                &blank_string_literals(&text, Language::Go),
                &mut found,
            ),
            "rs" => scan_rust(
                &relative,
                &blank_string_literals(&text, Language::Rust),
                &mut found,
            ),
            "pth" => scan_pth(&relative, &text, &mut found),
            _ => {}
        }
        if name == "sitecustomize.py" || name == "usercustomize.py" {
            found.push(InSourceRoot {
                marker: Marker::PythonSiteCustomize,
                file: relative.clone(),
                line: 1,
                symbol: None,
                target: relative.display().to_string(),
            });
        }
    }
    found.sort_by(|a, b| {
        (a.marker, &a.file, a.line, &a.symbol).cmp(&(b.marker, &b.file, b.line, &b.symbol))
    });
    found.dedup();
    Ok(found)
}

/// Which language's string-literal syntax to blank out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Language {
    Rust,
    Go,
}

/// Replace every line that is string-literal content with a blank one, keeping
/// the line count so reported line numbers stay true.
///
/// # Why this exists, and the measurement that put it here
///
/// A line-oriented scanner cannot tell a `#[no_mangle]` that annotates an item
/// from one that is *text inside a string*, and the difference is not academic:
/// run against **this repository**, the first version of this module reported
/// five in-source roots and **all five were wrong**. Four came from a test
/// file's escaped string literals and one from a fixture's raw string; one
/// symbol was even reported as `ledger_v2_amortize\` — a line-continuation
/// backslash that had leaked out of the literal into the name. Judged has no FFI
/// and declares none of these roots.
///
/// That is 0% precision on the first real repository measured, and
/// `docs/evals/2026-08-02-out-of-sample-corpus.md` already records the standing
/// warning: *"a bigger root set that is less accurate is not straightforwardly
/// an improvement."* A rule at 0% precision is not shippable, so either this got
/// fixed or the code-scanning half did not ship.
///
/// # What it handles, and what it does not
///
/// Multi-line **raw** strings — Rust's `r"…"`/`r#"…"#` and Go's backticks — are
/// tracked across lines, because those are what put fixture source at column
/// zero. Escaped-string content is caught by the cheaper tell: a line carrying
/// `\"` or a literal backslash-n is string content, since neither can appear in
/// a real attribute or item line.
///
/// It is not a lexer and does not claim to be. A single-line `"…"` containing a
/// marker with no escapes at all would still slip through, and a raw string
/// opened and closed on one line is simply left alone. Both are safe: this only
/// ever *removes* candidate lines, so its failures cost roots rather than invent
/// them — the direction a root source is allowed to be wrong in, and the
/// opposite of the defect it was written for.
fn blank_string_literals(text: &str, language: Language) -> String {
    let mut out = String::with_capacity(text.len());
    let mut raw_close: Option<String> = None;

    for line in text.lines() {
        if let Some(close) = raw_close.clone() {
            // Inside a multi-line raw string: blank the line, and leave the
            // state only when its terminator appears.
            out.push('\n');
            if line.contains(&close) {
                raw_close = None;
            }
            continue;
        }

        // Escaped-string content. `\"` and a literal backslash-n cannot occur in
        // a Rust attribute or a Go directive, so a line carrying either is text.
        if line.contains("\\\"") || line.contains("\\n") {
            out.push('\n');
            continue;
        }

        if let Some(close) = raw_opener(line, language) {
            // `raw_opener` returns `Some` only for a string still open at the end
            // of this line, so there is nothing further to decide here. An
            // earlier version re-checked that with `line.find(&close)` and got it
            // backwards: the opener line contains `r#"`, never `"#`, so `find`
            // returned `None`, the guard read that as trailing content and
            // skipped setting the state — leaving every following line unblanked.
            // That single bug was the last surviving phantom root on this
            // repository.
            out.push_str(line);
            out.push('\n');
            raw_close = Some(close);
            continue;
        }

        out.push_str(line);
        out.push('\n');
    }
    out
}

/// The terminator of a multi-line raw string opened on this line, if one is.
fn raw_opener(line: &str, language: Language) -> Option<String> {
    match language {
        // Go: a backtick string runs to the next backtick. An odd count on the
        // line means it is still open.
        Language::Go => (line.matches('`').count() % 2 == 1).then(|| "`".to_string()),
        // Rust: `r"`, `r#"`, `r##"` … closed by `"` and the same number of `#`.
        Language::Rust => {
            let at = line.find("r\"").or_else(|| line.find("r#"))?;
            let rest = &line[at + 1..];
            let hashes = rest.chars().take_while(|c| *c == '#').count();
            if !rest[hashes..].starts_with('"') {
                return None;
            }
            let close = format!("\"{}", "#".repeat(hashes));
            (!rest[hashes + 1..].contains(&close)).then_some(close)
        }
    }
}

/// `//go:linkname` and `//export`.
///
/// Both name their symbol on the directive line, which is what makes Go the
/// easy case: there is no following item to associate.
fn scan_go(file: &Path, text: &str, out: &mut Vec<InSourceRoot>) {
    for (index, raw) in text.lines().enumerate() {
        let line = raw.trim();
        let number = index + 1;

        if let Some(rest) = line.strip_prefix("//go:linkname") {
            // `//go:linkname local` (pull) or `//go:linkname local remote`
            // (push). Both names are roots: the local one is the symbol the
            // call graph cannot reach, and the remote one is what it is bound
            // to. Declaring only the local name would miss the half §4.1 says
            // deadcode reports spuriously.
            let mut parts = rest.split_whitespace();
            let local = parts.next();
            let remote = parts.next();
            for name in [local, remote].into_iter().flatten() {
                out.push(InSourceRoot {
                    marker: Marker::GoLinkname,
                    file: file.to_path_buf(),
                    line: number,
                    symbol: Some(trailing_identifier(name)),
                    target: name.to_string(),
                });
            }
            continue;
        }

        if let Some(rest) = line.strip_prefix("//export ") {
            if let Some(name) = rest.split_whitespace().next() {
                out.push(InSourceRoot {
                    marker: Marker::GoExport,
                    file: file.to_path_buf(),
                    line: number,
                    symbol: Some(name.to_string()),
                    target: name.to_string(),
                });
            }
        }
    }
}

/// The Rust attributes, each associated with the item it annotates.
fn scan_rust(file: &Path, text: &str, out: &mut Vec<InSourceRoot>) {
    let lines: Vec<&str> = text.lines().collect();
    for (index, raw) in lines.iter().enumerate() {
        let line = raw.trim();
        let number = index + 1;

        let marker = if line.starts_with("#[no_mangle]") || line.starts_with("#[unsafe(no_mangle)]")
        {
            Marker::RustNoMangle
        } else if line.starts_with("#[used]") || line.starts_with("#[unsafe(used)]") {
            Marker::RustUsed
        } else if line.starts_with("#[ctor]") || line.starts_with("#[ctor::ctor]") {
            Marker::RustCtor
        } else if line.starts_with("#[export_name") || line.starts_with("#[unsafe(export_name") {
            Marker::RustExportName
        } else {
            continue;
        };

        // `#[export_name = "abi_name"]` states the exported spelling outright,
        // and that spelling is what an outside consumer links against — so it is
        // a root in its own right, beside the item's Rust name.
        if marker == Marker::RustExportName {
            if let Some(name) = quoted(line) {
                out.push(InSourceRoot {
                    marker,
                    file: file.to_path_buf(),
                    line: number,
                    symbol: Some(name.clone()),
                    target: name,
                });
            }
        }

        if let Some((item, item_line)) = item_after(&lines, index) {
            out.push(InSourceRoot {
                marker,
                file: file.to_path_buf(),
                line: item_line,
                symbol: Some(item.clone()),
                target: item,
            });
        }
    }
}

/// A `.pth` file: the file itself, plus every module its `import` lines name.
///
/// §5.2 is precise about the semantics and they are unusual enough to restate:
/// `site` executes any line **beginning with `import`** and treats every other
/// line as a path to add to `sys.path`. So an `import` line is running code with
/// no caller, and the module it names is an entry point.
fn scan_pth(file: &Path, text: &str, out: &mut Vec<InSourceRoot>) {
    out.push(InSourceRoot {
        marker: Marker::PythonPth,
        file: file.to_path_buf(),
        line: 1,
        symbol: None,
        target: file.display().to_string(),
    });

    for (index, raw) in text.lines().enumerate() {
        let line = raw.trim();
        // `import x` and `import x; y()` both run. Anything else on the line is
        // a path entry, not code.
        let Some(rest) = line.strip_prefix("import ") else {
            continue;
        };
        let Some(module) = rest
            .split(|c: char| c == ';' || c.is_whitespace())
            .find(|token| !token.is_empty())
        else {
            continue;
        };
        out.push(InSourceRoot {
            marker: Marker::PythonPth,
            file: file.to_path_buf(),
            line: index + 1,
            symbol: Some(module.to_string()),
            target: module.to_string(),
        });
    }
}

/// The name of the item an attribute annotates: the first `fn`, `static` or
/// `const` at or after `from`, skipping further attributes.
///
/// Stops at a blank line for the reason [`crate::gate3f`] does — an attribute
/// binds to the declaration that follows it, and running past a blank line
/// attributes a marker to an unrelated item.
fn item_after(lines: &[&str], from: usize) -> Option<(String, usize)> {
    for (offset, raw) in lines.iter().enumerate().skip(from + 1) {
        let line = raw.trim();
        if line.is_empty() {
            return None;
        }
        // Another attribute, a doc comment, or a visibility-only line: keep
        // looking. `#[no_mangle]` above `#[allow(...)]` above the `fn` is
        // ordinary.
        if line.starts_with("#[") || line.starts_with("//") {
            continue;
        }
        for keyword in ["fn ", "static ", "const "] {
            if let Some(at) = line.find(keyword) {
                let rest = &line[at + keyword.len()..];
                let name: String = rest
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                if !name.is_empty() {
                    return Some((name, offset + 1));
                }
            }
        }
        return None;
    }
    None
}

/// The contents of the first double-quoted string on a line.
fn quoted(line: &str) -> Option<String> {
    let start = line.find('"')? + 1;
    let rest = &line[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

/// The trailing identifier of a qualified Go name — `pkg.Name` is bound as
/// `Name`.
fn trailing_identifier(name: &str) -> String {
    name.rsplit(['.', '/']).next().unwrap_or(name).to_string()
}

/// Every file under `root`, skipping `.git`.
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
            if path.is_dir() {
                // §9.3 0b, through the shared classifier: `name == ".git"` misses a
                // linked worktree and a submodule (.git is a FILE) and a bare
                // `vendor/foo.git/` (no .git at all). An unreadable probe stops the
                // walk too — descending on a failed lstat would read "could not look"
                // as "nothing here" (§6.20).
                if crate::boundary::classify(&path).stops_the_walk() {
                    continue;
                }
                stack.push(path);
            } else {
                out.push(path);
            }
        }
    }
    out.sort();
    Ok(out)
}
