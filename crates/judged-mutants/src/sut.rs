//! The system under test, and the two controls the suite needs to be meaningful.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use judged_core::{Error, Result};

/// What a cleaner claims is dead after looking at a repository.
///
/// There is no field for "confidence" and no field for "score". §9.2 records
/// that the SARIF spec itself warns rank values from different tools "are in
/// general not commensurable"; the suite grades on claims, not on how sure the
/// tool felt.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SutVerdict {
    /// Repo-relative paths the tool says can be removed.
    pub claimed_dead_paths: Vec<PathBuf>,
    /// Symbols the tool says can be removed.
    pub claimed_dead_symbols: Vec<String>,
}

/// A cleaner the suite can grade.
pub trait Sut {
    /// Name used in [`crate::runner::SuiteReport`].
    fn name(&self) -> &str;

    /// Analyze `repo` and return what it would remove. Implementations must not
    /// mutate `repo` — §9.2: adapters are read-only, the orchestrator owns 100%
    /// of mutations.
    fn run(&self, repo: &Path) -> Result<SutVerdict>;
}

/// A deliberately bad cleaner: reachability from obvious entry points, nothing
/// else. No grep veto, no config parsing, no framework conventions.
///
/// **This is the suite's own positive control.** §3.7 and §9.8 establish the
/// principle for evidence artifacts — if known-live symbols do not appear,
/// discard the artifact loudly — and the suite needs the same guarantee about
/// itself. `NaiveSut` must FAIL, loudly and on many mutants. **If a naive
/// cleaner ever passes the suite, the suite is theatre** and its green results
/// on a real tool mean nothing.
pub struct NaiveSut;

impl Sut for NaiveSut {
    fn name(&self) -> &str {
        "naive"
    }

    fn run(&self, repo: &Path) -> Result<SutVerdict> {
        let candidates = walk(repo)?;

        // The reference corpus, and the whole flaw. §7.5: grahama1970's
        // `SKIP_DIRS` excludes build config from the reference scan, and
        // NickCrew's entire cross-file check is `grep -r "from './FILE'"`. Every
        // surveyed tool decides what counts as a reference by looking only at
        // files it recognizes as code, so a reference from a YAML task list, a
        // CI step, a Dockerfile `COPY`, or an executed README block does not
        // exist as far as the tool is concerned.
        let mut corpus: Vec<(String, String)> = Vec::new();
        for rel in &candidates {
            if !is_parsed_source(rel) {
                continue;
            }
            let path = repo.join(rel);
            let bytes = fs::read(&path).map_err(|source| Error::Io {
                path: path.clone(),
                source,
            })?;
            corpus.push((rel.clone(), String::from_utf8_lossy(&bytes).into_owned()));
        }

        let mut claimed_dead_paths = BTreeSet::new();
        for rel in &candidates {
            if is_entry_point(rel) {
                continue;
            }
            let basename = basename(rel);
            let stem = stem(rel);
            let referenced = corpus.iter().any(|(owner, text)| {
                owner != rel && (text.contains(&basename) || text.contains(&stem))
            });
            if !referenced {
                claimed_dead_paths.insert(PathBuf::from(rel));
            }
        }

        // The same heuristic one level down, which is how `ts-prune` and friends
        // report unused exports: a declared name whose only textual occurrence
        // in the corpus is its own declaration is called dead. This is what
        // makes the control fail the classes whose live artifact is a symbol
        // rather than a file — reflection, link-time registries, ABI exports.
        let mut declared: BTreeSet<String> = BTreeSet::new();
        for (_, text) in &corpus {
            declared.extend(declarations(text));
        }
        let claimed_dead_symbols = declared
            .into_iter()
            .filter(|name| {
                corpus
                    .iter()
                    .map(|(_, text)| text.matches(name.as_str()).count())
                    .sum::<usize>()
                    <= 1
            })
            .collect();

        Ok(SutVerdict {
            claimed_dead_paths: claimed_dead_paths.into_iter().collect(),
            claimed_dead_symbols,
        })
    }
}

/// Extensions the naive tool recognizes as code. Everything else — YAML, JSON,
/// TOML, Dockerfile, CI workflows, markdown — is invisible to it, both as a
/// reference and, deliberately, not at all as a candidate: §7.5's `rm -rf lib/`
/// and "`package-lock.json` (if regenerable)" show these tools happily removing
/// files they never parse.
const PARSED_EXTENSIONS: &[&str] = &[
    "py", "pyi", "ts", "tsx", "js", "jsx", "mjs", "cjs", "rs", "go",
];

/// Stems every shipped cleaner treats as a root. Without these the control
/// would be a strawman rather than a faithful reproduction of §7.5 — Knip's
/// documented failure mode is *missing* entry points, not having none.
const ENTRY_STEMS: &[&str] = &["main", "index", "lib", "mod", "__init__", "__main__"];

/// Declaration keywords, in the order they are tried. Line-oriented and
/// language-agnostic on purpose: this is the level of rigour the surveyed tools
/// apply, not an accident of implementation.
const DECLARATION_KEYWORDS: &[&str] = &[
    "def ",
    "class ",
    "fn ",
    "func ",
    "function ",
    "struct ",
    "trait ",
    "interface ",
    "enum ",
];

/// Repo-relative, forward-slashed paths of every file under `root`, sorted.
///
/// `.git` is skipped. Packed objects and loose refs contain the names of files
/// that really are dead, and counting history as a live reference would make the
/// control accidentally safe — which would destroy its value as a control.
fn walk(root: &Path) -> Result<Vec<String>> {
    let mut out = Vec::new();
    walk_into(root, "", &mut out)?;
    out.sort();
    Ok(out)
}

fn walk_into(dir: &Path, prefix: &str, out: &mut Vec<String>) -> Result<()> {
    let entries = fs::read_dir(dir).map_err(|source| Error::Io {
        path: dir.to_path_buf(),
        source,
    })?;
    for entry in entries {
        let entry = entry.map_err(|source| Error::Io {
            path: dir.to_path_buf(),
            source,
        })?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == ".git" {
            continue;
        }
        let rel = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        let file_type = entry.file_type().map_err(|source| Error::Io {
            path: entry.path(),
            source,
        })?;
        if file_type.is_dir() {
            walk_into(&entry.path(), &rel, out)?;
        } else {
            out.push(rel);
        }
    }
    Ok(())
}

fn basename(rel: &str) -> String {
    rel.rsplit('/').next().unwrap_or(rel).to_string()
}

fn stem(rel: &str) -> String {
    let base = basename(rel);
    match base.rsplit_once('.') {
        // A leading dot is the whole name, not an extension: `.gitignore` has no
        // stem to strip.
        Some((head, _)) if !head.is_empty() => head.to_string(),
        _ => base,
    }
}

fn is_parsed_source(rel: &str) -> bool {
    match basename(rel).rsplit_once('.') {
        Some((head, ext)) if !head.is_empty() => PARSED_EXTENSIONS.contains(&ext),
        _ => false,
    }
}

fn is_entry_point(rel: &str) -> bool {
    ENTRY_STEMS.contains(&stem(rel).as_str())
}

/// Names declared in `text`, by a line scan that takes the first identifier
/// following the first declaration keyword on each line.
fn declarations(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        for keyword in DECLARATION_KEYWORDS {
            let Some(rest) = after_keyword(line, keyword) else {
                continue;
            };
            if let Some(name) = leading_identifier(rest) {
                out.push(name);
            }
            break;
        }
    }
    out
}

/// The text after the first occurrence of `keyword` that starts on an
/// identifier boundary, so `pub extern "C" fn f` is found but `defer` is not
/// mistaken for `def`.
fn after_keyword<'a>(line: &'a str, keyword: &str) -> Option<&'a str> {
    let mut from = 0;
    while let Some(offset) = line[from..].find(keyword) {
        let at = from + offset;
        let preceded_by_identifier = line[..at]
            .chars()
            .next_back()
            .is_some_and(|c| c.is_alphanumeric() || c == '_');
        if !preceded_by_identifier {
            return Some(&line[at + keyword.len()..]);
        }
        from = at + keyword.len();
    }
    None
}

fn leading_identifier(text: &str) -> Option<String> {
    let mut chars = text.chars();
    let first = chars.next()?;
    if !(first.is_alphabetic() || first == '_') {
        return None;
    }
    let mut name = String::from(first);
    for c in chars {
        if c.is_alphanumeric() || c == '_' {
            name.push(c);
        } else {
            break;
        }
    }
    Some(name)
}

/// A cleaner that claims nothing is ever dead.
///
/// The negative control, and the reason [`crate::mutant::GroundTruth`] carries
/// decoys. This SUT has a perfect false-removal record and is completely
/// useless; a suite that cannot tell it apart from a working tool is measuring
/// nothing. It must fail on decoy recall while passing on false removals.
pub struct RefusingSut;

impl Sut for RefusingSut {
    fn name(&self) -> &str {
        "refusing"
    }

    fn run(&self, _repo: &Path) -> Result<SutVerdict> {
        Ok(SutVerdict::default())
    }
}
