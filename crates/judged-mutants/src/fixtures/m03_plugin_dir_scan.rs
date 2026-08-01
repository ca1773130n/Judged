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
use judged_core::Result;

use crate::fixtures::write;
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
            // Index-aligned with the decoy above: a symbol-level analyzer never
            // claims a path, so without this it is never asked a question it
            // can answer (see `GroundTruth::decoy_dead_symbols`).
            decoy_dead_symbols: vec!["hang_indent".to_string()],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::support;

    #[test]
    fn m03_materializes_a_real_git_repo_with_one_commit() {
        let (_dir, repo, _truth) = support::materialize(&PluginDirScan);
        support::assert_committed(&repo, &["pyproject.toml"]);
    }

    #[test]
    fn m03_ground_truth_paths_all_exist_on_disk() {
        let (_dir, repo, truth) = support::materialize(&PluginDirScan);
        assert!(
            !truth.live_paths.is_empty(),
            "m03's live artifact is a file"
        );
        support::assert_ground_truth_is_on_disk(&repo, &truth);
    }

    /// The claim m03 makes is that the plugin's *name exists only as a
    /// filename*. A directory-scanning loader derives the module name from
    /// `Path.stem`, so unlike class 2 there is no string to grep for — not even
    /// in the loader. Prove it rather than trust it.
    #[test]
    fn m03_the_plugin_name_appears_in_no_file_in_the_repository() {
        let (_dir, repo, truth) = support::materialize(&PluginDirScan);
        let live = truth
            .live_paths
            .first()
            .expect("m03 declares a live plugin file");
        let stem = live
            .file_stem()
            .expect("live plugin has a file name")
            .to_string_lossy()
            .into_owned();

        for (path, bytes) in support::tree(repo.root()) {
            assert!(
                !support::mentions(&bytes, &stem),
                "{path} names the plugin {stem:?}; m03 must be reachable only \
                 by directory scan"
            );
        }
    }

    #[test]
    fn m03_the_decoy_is_named_by_nothing() {
        let (_dir, repo, truth) = support::materialize(&PluginDirScan);
        support::assert_decoys_are_unreferenced(&repo, &truth);
    }
}
