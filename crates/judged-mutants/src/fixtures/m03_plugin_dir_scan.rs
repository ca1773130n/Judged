//! Class 3 — registered by a directory-scanning plugin loader.
//!
//! **Mechanism.** `pluginhost/loader.py` lists `pluginhost/plugins/*.py` at
//! startup and imports every module it finds, deriving each module name from
//! `Path.stem`. The live plugin is therefore reachable *because of where it
//! sits on disk*, and for no other reason.
//!
//! **Why every other signal misses it.** Class 2 (`m02`) is the neighbouring
//! mechanism — a dynamic import — but there the module name exists somewhere as
//! a string, so the grep veto (§9.6) can still fire. Here it does not exist
//! anywhere: the loader interpolates a stem it read from the filesystem, so
//! there is no identifier to match, exactly the shape §6.1 calls out for Go's
//! structural reflection ("there is no identifier string anywhere to match").
//! A call graph rooted at `main` stops at `import_module`; the compiler index
//! never sees the plugin; the build graph does not mention it.
//!
//! The counter-signal a tool is *expected* to catch is the reflection primitive
//! in the loader (§6.1: "presence of any reflection primitive in the module or
//! its transitive importers → cap the tier for the whole directory"). This
//! fixture is fair, not impossible.

use std::path::Path;

use judged_core::git::Repo;
use judged_core::{Error, Result};

use crate::mutant::{Ecosystem, GroundTruth, Mutant};

/// The loader iterates `plugins/*.py` at startup. Nothing names the plugin;
/// its liveness is a property of where the file sits on disk, which is
/// invisible to a call-graph.
pub struct PluginDirScan;

impl Mutant for PluginDirScan {
    fn id(&self) -> &str {
        "m03"
    }
    fn ecosystem(&self) -> Ecosystem {
        Ecosystem::Python
    }
    fn mechanism(&self) -> &str {
        "plugin discovered by directory scan at startup, never named in code"
    }
    fn research_ref(&self) -> &str {
        "§10 E2 class 3"
    }
    fn materialize(&self, dir: &Path) -> Result<GroundTruth> {
        let repo = Repo::init(dir)?;
        let root = repo.root().to_path_buf();

        write(
            &root,
            "pyproject.toml",
            "[project]\nname = \"pluginhost\"\nversion = \"0.1.0\"\nrequires-python = \">=3.11\"\n",
        )?;
        write(&root, "pluginhost/__init__.py", "")?;

        // The mechanism, in full. Note that no plugin name occurs here: the
        // stem comes off the filesystem, so the loader cannot be grepped for
        // what it loads.
        write(
            &root,
            "pluginhost/loader.py",
            r#"# Import every module that happens to be sitting in plugins/.
import importlib
from pathlib import Path

PLUGIN_DIR = Path(__file__).with_name("plugins")


def load_all():
    for path in sorted(PLUGIN_DIR.glob("*.py")):
        if path.stem.startswith("_"):
            continue
        yield importlib.import_module(f"{__package__}.plugins.{path.stem}")
"#,
        )?;
        write(&root, "pluginhost/plugins/__init__.py", "")?;

        // THE LIVE ARTIFACT. Its module name is spelled nowhere in the tree.
        write(
            &root,
            "pluginhost/plugins/tsvwriter.py",
            r#"# Registered purely by being a *.py file inside plugins/.

EXTENSION = ".tsv"


def emit(rows):
    return "\n".join("\t".join(str(cell) for cell in row) for row in rows)
"#,
        )?;

        write(
            &root,
            "pluginhost/main.py",
            r#"from .loader import load_all


def main():
    for module in load_all():
        print(module.__name__)
"#,
        )?;

        // THE DECOY. Not under plugins/, so the scan never reaches it, and no
        // module imports it. Genuinely dead — a tool that finds nothing here
        // has told us nothing (see `GroundTruth::decoy_dead_paths`).
        write(
            &root,
            "pluginhost/textwrap_helper.py",
            r#"# Left behind when the report renderer moved to Jinja.


def hang_indent(text, width=72):
    return text
"#,
        )?;

        repo.add_all()?;
        repo.commit("m03: plugin host with a directory-scanning loader")?;

        Ok(GroundTruth {
            // Repo-relative, so that a report reads the same wherever the
            // fixture was materialized.
            live_paths: vec!["pluginhost/plugins/tsvwriter.py".into()],
            live_symbols: vec!["pluginhost.plugins.tsvwriter".to_string()],
            decoy_dead_paths: vec!["pluginhost/textwrap_helper.py".into()],
        })
    }
}

/// Write one fixture file, creating parents, attaching the path to any failure.
///
/// Duplicated in each mutant module rather than shared: `fixtures/mod.rs` is
/// complete and declares only the nineteen class modules, so there is nowhere
/// to put a shared helper without changing it.
fn write(root: &Path, rel: &str, contents: &str) -> Result<()> {
    let path = root.join(rel);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| Error::Io {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    std::fs::write(&path, contents).map_err(|source| Error::Io { path, source })
}

#[cfg(test)]
mod tests {
    use super::*;
    use judged_core::git::Repo;
    use tempfile::TempDir;

    /// Materialize into a throwaway directory and hand back the repo with it,
    /// so that the fixture's git state can be interrogated too.
    fn materialize() -> (TempDir, Repo, GroundTruth) {
        let dir = TempDir::new().expect("create tempdir");
        let truth = PluginDirScan
            .materialize(dir.path())
            .expect("m03 materializes");
        let repo = Repo::discover(dir.path()).expect("fixture is a git repo");
        (dir, repo, truth)
    }

    /// Every file in the working tree except `.git`, as (repo-relative path,
    /// bytes). Bytes rather than text because a later mutant's evidence is
    /// binary, and a shared shape keeps these tests comparable.
    fn tree(root: &Path) -> Vec<(String, Vec<u8>)> {
        let mut out = Vec::new();
        walk(root, root, &mut out);
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    fn walk(dir: &Path, root: &Path, out: &mut Vec<(String, Vec<u8>)>) {
        for entry in std::fs::read_dir(dir).expect("read fixture directory") {
            let path = entry.expect("read directory entry").path();
            if path.file_name().is_some_and(|n| n == ".git") {
                continue;
            }
            if path.is_dir() {
                walk(&path, root, out);
            } else {
                let rel = path
                    .strip_prefix(root)
                    .expect("path is under the fixture root")
                    .to_string_lossy()
                    .into_owned();
                out.push((rel, std::fs::read(&path).expect("read fixture file")));
            }
        }
    }

    fn mentions(haystack: &[u8], needle: &str) -> bool {
        let needle = needle.as_bytes();
        haystack.windows(needle.len()).any(|w| w == needle)
    }

    #[test]
    fn materializes_a_real_git_repo_with_one_commit() {
        let (_dir, repo, _truth) = materialize();
        assert!(repo.root().join(".git").is_dir(), "expected a git directory");
        assert!(
            repo.is_tracked(Path::new("pyproject.toml"))
                .expect("query the index"),
            "the fixture must be committed, not just written to disk"
        );
    }

    #[test]
    fn ground_truth_paths_all_exist_on_disk() {
        let (_dir, repo, truth) = materialize();
        assert!(!truth.live_paths.is_empty(), "m03's live artifact is a file");
        assert!(
            !truth.decoy_dead_paths.is_empty(),
            "without a decoy, a tool that claims nothing passes m03 for free"
        );
        for path in truth.live_paths.iter().chain(&truth.decoy_dead_paths) {
            assert!(path.is_relative(), "{path:?} must be repo-relative");
            assert!(repo.root().join(path).is_file(), "{path:?} is missing");
        }
    }

    /// The claim m03 makes is that the plugin's *name exists only as a
    /// filename*. A directory-scanning loader derives the module name from
    /// `Path.stem`, so unlike class 2 there is no string to grep for — not even
    /// in the loader. Prove it rather than trust it.
    #[test]
    fn the_plugin_name_appears_in_no_file_in_the_repository() {
        let (_dir, repo, truth) = materialize();
        let live = truth
            .live_paths
            .first()
            .expect("m03 declares a live plugin file");
        let stem = live
            .file_stem()
            .expect("live plugin has a file name")
            .to_string_lossy()
            .into_owned();

        for (path, bytes) in tree(repo.root()) {
            assert!(
                !mentions(&bytes, &stem),
                "{path} names the plugin {stem:?}; m03 must be reachable only \
                 by directory scan"
            );
        }
    }

    /// A decoy that some mechanism secretly reaches would make the suite grade
    /// a correct refusal as a miss.
    #[test]
    fn the_decoy_is_named_by_nothing() {
        let (_dir, repo, truth) = materialize();
        for decoy in &truth.decoy_dead_paths {
            let stem = decoy
                .file_stem()
                .expect("decoy has a file name")
                .to_string_lossy()
                .into_owned();
            for (path, bytes) in tree(repo.root()) {
                if Path::new(&path) == decoy.as_path() {
                    continue;
                }
                assert!(
                    !mentions(&bytes, &stem),
                    "{path} references the decoy {stem:?}, so it is not dead"
                );
            }
        }
    }
}
