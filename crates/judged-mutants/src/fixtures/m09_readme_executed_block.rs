//! Class 9 — referenced only from a README code block that CI executes.
//!
//! **The mechanism.** `src/lib.rs` carries
//! `#![doc = include_str!("../README.md")]`, so the ```` ```rust ```` block in
//! the README becomes a doctest. `cargo test --doc` — a step in the fixture's
//! CI workflow — compiles and runs it. That block is the only thing in the
//! repository that names `badge::render_badge`.
//!
//! **Why every other signal misses it.** The call site is inside a Markdown
//! file, so no Rust parser sees it as code; the binary does not call it, so the
//! call graph from `main` does not reach it; and §4.1 records the specific tool
//! failure — **cargo-udeps cannot see doctests at all**, which is why a crate
//! used only from a doc example is reported as an unused dependency. A tool
//! that models "what does `cargo build` need" gets the same answer, because
//! `cargo build` genuinely does not need it. Only `cargo test --doc` does, and
//! it is the job nobody models.
//!
//! **What is supposed to catch it.** §0.9's rule that documentation is not in
//! the deletion path, plus the whole-repo literal veto (§6.20) reading Markdown
//! as text worth matching. The crate is `publish = false`, so "it is `pub`, it
//! must be someone's API" is not available as an escape hatch here — the item
//! is public solely because a doctest compiles as an external crate.

use std::path::{Path, PathBuf};

use judged_core::git::Repo;
use judged_core::{Error, Result};

use crate::mutant::{Declaration, Ecosystem, GroundTruth, Mutant};

/// Documentation that is also a test. §0.9 keeps docs out of the deletion
/// path entirely; this mutant checks the tool honours that even when the
/// doc is the *only* thing keeping code alive.
pub struct ReadmeExecutedBlock;

/// Repo-relative path of the artifact that is alive and looks dead.
const LIVE: &str = "src/badge.rs";

/// The symbol inside [`LIVE`] that only the README example calls.
const LIVE_SYMBOL: &str = "render_badge";

/// The one file that names [`LIVE_SYMBOL`] — and it is not source.
const MECHANISM: &str = "README.md";

/// The crate root, which turns [`MECHANISM`] into executable documentation.
const DOC_INCLUDE_SITE: &str = "src/lib.rs";

/// The include that makes the README a doctest rather than prose.
///
/// `cfg(test)`, because it names an invariant rather than any file's contents.
#[cfg(test)]
const DOC_INCLUDE: &str = "include_str!(\"../README.md\")";

/// The CI step that runs it. Without this the block is prose and the mutant
/// would be claiming a liveness mechanism it does not have.
const CI_WORKFLOW: &str = ".github/workflows/ci.yml";

/// The command in [`CI_WORKFLOW`] that executes the README block.
///
/// `cfg(test)`, because it names an invariant rather than any file's contents.
#[cfg(test)]
const CI_DOCTEST_STEP: &str = "cargo test --doc";

/// Files written into the mutant repository, as `(repo-relative path, body)`.
const FILES: &[(&str, &str)] = &[
    (
        "Cargo.toml",
        r#"[package]
name = "m09-badgekit"
version = "0.1.0"
edition = "2021"
publish = false

[dependencies]
"#,
    ),
    (
        DOC_INCLUDE_SITE,
        r#"//! The crate root. The attribute below is what turns the README from
//! prose into a compiled, executed test -- and it is the only line in the
//! repository that connects the two files.
#![doc = include_str!("../README.md")]

pub mod badge;
pub mod coverage;
"#,
    ),
    (
        "src/main.rs",
        r#"//! The binary. It computes a percentage and prints it; it renders nothing.
//! `cargo build` therefore does not need the renderer, and neither does any
//! call graph rooted at `main` -- which is exactly the answer a reachability
//! pass gives, and it is wrong.

fn main() {
    println!("{}", m09_badgekit::coverage::percent(41, 50));
}
"#,
    ),
    (
        "src/coverage.rs",
        r#"//! The routinely-exercised sibling, with the only unit test in the crate.
//! Its presence is what makes the asymmetry legible: this module reads as hot,
//! the renderer reads as dead, and the difference between them lives in a
//! Markdown file.

/// Covered lines as a whole percentage.
pub fn percent(covered: usize, total: usize) -> u8 {
    if total == 0 {
        return 0;
    }
    ((covered * 100) / total) as u8
}

#[cfg(test)]
mod tests {
    #[test]
    fn rounds_down() {
        assert_eq!(super::percent(41, 50), 82);
    }
}
"#,
    ),
    // THE LIVE ARTIFACT.
    (
        LIVE,
        r#"//! LIVE. Called from one place in the repository, and that place is
//! Markdown. `pub` here is not an API claim: the package is `publish = false`,
//! and the item is public only because a doctest compiles as a separate crate
//! and can reach nothing else.

/// Render one `label: value` status badge.
pub fn render_badge(label: &str, value: &str) -> String {
    format!("<svg role=\"img\"><title>{label}: {value}</title></svg>")
}
"#,
    ),
    // THE MECHANISM. The fenced block below is a doctest, not an illustration.
    (
        MECHANISM,
        r#"# badgekit

Status badges for the dashboards.

```rust
use m09_badgekit::badge::render_badge;

let svg = render_badge("coverage", "82%");
assert!(svg.contains("82%"));
```

The example above is compiled and executed as a documentation test on every
push; see the workflow under `.github/workflows/`.
"#,
    ),
    (
        CI_WORKFLOW,
        r#"name: ci

on: [push]

jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      # The unit tests never touch the renderer. The second step is the only
      # job anywhere that executes it, and no dependency-level tool models it:
      # §4.1 records that cargo-udeps cannot see doc-tests at all.
      - run: cargo test --lib
      - run: cargo test --doc
"#,
    ),
    (
        "src/orphan_sparkline.rs",
        r#"//! DEAD DECOY. The sparkline was dropped when the dashboard moved to
//! server-rendered charts; no `mod` declares this file, so it is not even
//! compiled, and nothing names it.

pub fn spark(values: &[u8]) -> String {
    values.iter().map(|v| char::from(b'0' + v % 10)).collect()
}
"#,
    ),
    (
        "src/unused_palette.rs",
        // `r##` because the body contains `"#`, which would close an `r#`
        // string on the first colour literal.
        r##"//! DEAD DECOY. A second one on purpose: decoy recall is a rate, and one
//! decoy cannot tell a tool that reasoned from a tool that guessed once.

pub const BRAND: [&str; 3] = ["#4c1", "#dfb317", "#e05d44"];
"##,
    ),
];

impl ReadmeExecutedBlock {
    /// Repo-relative paths of the genuinely-dead files planted here. Neither
    /// is declared with `mod`, so neither is even compiled.
    const DECOYS: [&'static str; 2] = ["src/orphan_sparkline.rs", "src/unused_palette.rs"];

    /// The symbol each decoy defines, index-aligned with [`Self::DECOYS`].
    /// Without these a symbol-level analyzer scores zero decoys here and reads
    /// as having found nothing (see `GroundTruth::decoy_dead_symbols`).
    const DECOY_SYMBOLS: [&'static str; 2] = ["spark", "BRAND"];
}

impl Mutant for ReadmeExecutedBlock {
    fn id(&self) -> &str {
        "m09"
    }
    fn ecosystem(&self) -> Ecosystem {
        Ecosystem::Rust
    }
    fn mechanism(&self) -> &str {
        "API exercised only by a README example that CI runs"
    }
    fn research_ref(&self) -> &str {
        "§10 E2 class 9"
    }
    /// `#![doc = include_str!("../README.md")]` makes the README block a doctest, and
    /// `cargo test --doc` runs it — so a test suite really does enter it.
    ///
    /// Worth knowing that the tooling lags the mechanism: `cargo-llvm-cov` only
    /// instruments doctests behind `--doctests` on nightly, so a real artifact
    /// often would not carry this record. That is a limit of the instrumenter, not
    /// a fact about how the artifact is reached, and the declaration answers the
    /// second question.
    fn coverage_declaration(&self) -> Declaration {
        Declaration::default().calling("src/badge.rs", "render_badge")
    }

    fn materialize(&self, dir: &Path) -> Result<GroundTruth> {
        let repo = Repo::init(dir)?;
        for (relative, body) in FILES {
            let path = repo.root().join(relative);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(|source| Error::Io {
                    path: parent.to_path_buf(),
                    source,
                })?;
            }
            std::fs::write(&path, body).map_err(|source| Error::Io { path, source })?;
        }
        repo.add_all()?;
        repo.commit("m09: badgekit whose renderer is called only from the README")?;

        Ok(GroundTruth {
            // Repo-relative, because the runner keys ground truth and SUT
            // claims on the same repo-relative rendering and the fixture's own
            // canonicalized root is not the path the runner holds.
            live_paths: vec![PathBuf::from(LIVE)],
            live_symbols: vec![LIVE_SYMBOL.to_string()],
            decoy_dead_paths: Self::DECOYS.iter().copied().map(PathBuf::from).collect(),
            decoy_dead_symbols: Self::DECOY_SYMBOLS
                .iter()
                .map(|symbol| (*symbol).to_string())
                .collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::support;

    #[test]
    fn m09_is_a_real_git_repository_whose_live_artifact_is_committed() {
        let (_dir, repo, _truth) = support::materialize(&ReadmeExecutedBlock);
        support::assert_committed(&repo, &[LIVE]);
    }

    #[test]
    fn m09_ground_truth_names_files_that_are_really_there() {
        let (_dir, repo, truth) = support::materialize(&ReadmeExecutedBlock);

        assert_eq!(truth.live_paths, vec![Path::new(LIVE).to_path_buf()]);
        assert_eq!(truth.live_symbols, vec![LIVE_SYMBOL.to_string()]);
        assert_eq!(
            truth.decoy_dead_paths.len(),
            ReadmeExecutedBlock::DECOYS.len()
        );

        support::assert_ground_truth_is_on_disk(&repo, &truth);
    }

    #[test]
    fn m09_no_source_file_names_the_documented_api() {
        let (_dir, repo, _truth) = support::materialize(&ReadmeExecutedBlock);

        // The Markdown file and the definition, and nothing else. In
        // particular not `src/main.rs`: if the binary called it, the call graph
        // would rescue it and the mutant would be testing nothing.
        assert_eq!(
            support::files_mentioning(repo.root(), LIVE_SYMBOL),
            vec![MECHANISM.to_string(), LIVE.to_string()],
            "only the README example and the definition may name the API"
        );
    }

    #[test]
    fn m09_the_documented_module_is_never_named_by_its_filename() {
        let (_dir, repo, _truth) = support::materialize(&ReadmeExecutedBlock);
        let basename = Path::new(LIVE)
            .file_name()
            .and_then(|n| n.to_str())
            .expect("LIVE has a UTF-8 basename");

        // The README rescues the *symbol*, never the path: a Markdown example
        // spells `badge::render_badge`, not `src/badge.rs`. So a cleaner that
        // greps for the filename before deleting the file finds nothing, and
        // the mutant is hard for the reason it claims to be.
        assert!(
            support::files_mentioning(repo.root(), basename).is_empty(),
            "{basename} must be spelled nowhere; the README names the item, not the file"
        );
    }

    #[test]
    fn m09_the_readme_block_is_actually_executed() {
        let (_dir, repo, _truth) = support::materialize(&ReadmeExecutedBlock);

        // A README block that CI does not run is a comment, and this class
        // would then be indistinguishable from an ordinary dead function. Both
        // halves have to hold: the crate root includes the README as docs, and
        // CI runs the doctests.
        assert_eq!(
            support::files_mentioning(repo.root(), DOC_INCLUDE),
            vec![DOC_INCLUDE_SITE.to_string()],
            "the crate root must include the README as documentation"
        );
        assert_eq!(
            support::files_mentioning(repo.root(), CI_DOCTEST_STEP),
            vec![CI_WORKFLOW.to_string()],
            "CI must run the doctests, or the block is not executed"
        );
    }

    #[test]
    fn m09_decoys_are_named_nowhere_at_all() {
        let (_dir, repo, truth) = support::materialize(&ReadmeExecutedBlock);
        support::assert_decoys_are_unreferenced(&repo, &truth);
    }
}
