//! Class 1 — referenced only by a string in a config.
//!
//! **The mechanism.** `ledger/dunning.py` defines `DunningConfig`. The only
//! thing in the repository that names it is the string
//! `"ledger.dunning.DunningConfig"` inside `INSTALLED_APPS` in
//! `ledger/settings.py` — §6.2's very first named shape, Django
//! `INSTALLED_APPS = ['myapp.SomeConfig']`. Django imports the module and
//! instantiates the class at startup; nothing in the repository imports it.
//!
//! **Why every other signal misses it.** The import graph stops at
//! `manage.py -> ledger.settings`, because a list of strings is data, not an
//! import — so `dunning` has no in-edges and reads as a leaf. Nor does a
//! filename search help: the reference is dotted (`ledger.dunning`), so the
//! basename `dunning.py` appears nowhere in the repository at all. §6.2 is
//! blunt about the consequence — this defeats *all* pure-code reachability,
//! because "the name exists, but not in a file the parser reads".
//!
//! **What is supposed to catch it.** The whole-repo literal veto of §6.20,
//! matching on the *stem* rather than the basename. `dunning` does appear, as
//! a substring of a string literal. The mutant is therefore hard but fair: it
//! is solvable by exactly the counter-signal §6.2 marks mandatory, and by
//! nothing weaker. The test below asserts both halves.

use std::path::Path;

use judged_core::git::Repo;
use judged_core::{Error, Result};

use crate::mutant::{Ecosystem, GroundTruth, Mutant};

/// A Django `AppConfig` named only as a dotted string in a settings list. No
/// import, no call site: the reference exists, but only as data.
///
/// The scaffold originally sketched this as a Celery task in `celery.yaml`.
/// The Django `INSTALLED_APPS` shape is the one §6.2 names first and is
/// strictly harder, because `settings.py` *is* Python — so the mutant survives
/// even a tool that parses every file in the repository as code, which the
/// YAML variant does not.
pub struct YamlStringRef;

/// Repo-relative path of the artifact that is alive and looks dead.
const LIVE: &str = "ledger/dunning.py";

/// The one file in the repository that names [`LIVE`], and how.
const MECHANISM: &str = "ledger/settings.py";

/// Files written into the mutant repository, as `(repo-relative path, body)`.
const FILES: &[(&str, &str)] = &[
    (
        "pyproject.toml",
        r#"[project]
name = "ledger"
version = "0.1.0"
requires-python = ">=3.11"
dependencies = ["django>=5.0"]

[build-system]
requires = ["setuptools>=68"]
build-backend = "setuptools.build_meta"
"#,
    ),
    (
        "manage.py",
        r#""""Entry point.

Imports the settings module the ordinary way, so the import graph reaches
settings.py -- and stops there. Everything reachable in this repository is
reachable from here, except the one app named only as a string.
"""

from ledger import settings


def main() -> None:
    for app in settings.INSTALLED_APPS:
        print(app)


if __name__ == "__main__":
    main()
"#,
    ),
    ("ledger/__init__.py", "\"\"\"Ledger service.\"\"\"\n"),
    (
        MECHANISM,
        r#""""Django settings.

The last entry of INSTALLED_APPS is the only reference in this repository to
the dunning app. Django imports the module and instantiates the class at
startup. To a reachability pass it is an unremarkable string in a list.
"""

INSTALLED_APPS = [
    "django.contrib.contenttypes",
    "django.contrib.auth",
    "ledger.dunning.DunningConfig",
]

MIDDLEWARE = [
    "django.middleware.common.CommonMiddleware",
]
"#,
    ),
    (
        LIVE,
        r#""""LIVE. Constructed by Django from the INSTALLED_APPS string.

Nothing in this repository imports this module. Deleting it does not break
any import, any test that does not boot Django, or any type check -- it
breaks production at startup with ModuleNotFoundError.
"""

from django.apps import AppConfig


class DunningConfig(AppConfig):
    name = "ledger.dunning"
    verbose_name = "Dunning"

    def ready(self) -> None:
        """Django calls this once per installed app during startup."""
"#,
    ),
    (
        "ledger/legacy_invoice_dump.py",
        r#""""DEAD DECOY. Superseded by the reporting service; nothing imports it,
no string names it, no config mentions it. A cleaner that never says this is
dead has told us nothing, however safe its false-removal record looks.
"""


def dump_invoices(rows: list[dict]) -> str:
    return "\n".join(str(row) for row in rows)
"#,
    ),
    (
        "ledger/unused_currency_table.py",
        r#""""DEAD DECOY. Second one on purpose: decoy recall is a rate, and a
single decoy cannot distinguish a tool that reasons from one that guessed.
"""

RATES = {"EUR": 1.0, "USD": 1.08}
"#,
    ),
];

impl YamlStringRef {
    /// Repo-relative paths of the genuinely-dead files planted here.
    const DECOYS: [&'static str; 2] = [
        "ledger/legacy_invoice_dump.py",
        "ledger/unused_currency_table.py",
    ];
}

impl Mutant for YamlStringRef {
    fn id(&self) -> &str {
        "m01"
    }
    fn ecosystem(&self) -> Ecosystem {
        Ecosystem::Python
    }
    fn mechanism(&self) -> &str {
        "dotted class path appearing only as a string literal in a settings list"
    }
    fn research_ref(&self) -> &str {
        "§10 E2 class 1"
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
        repo.commit("m01: ledger service with a string-referenced Django app")?;

        Ok(GroundTruth {
            // Repo-relative, because the runner keys ground truth and SUT
            // claims on the same repo-relative rendering and the fixture's own
            // canonicalized root is not the path the runner holds.
            live_paths: vec![Path::new(LIVE).to_path_buf()],
            live_symbols: vec!["DunningConfig".to_string()],
            decoy_dead_paths: Self::DECOYS.iter().map(Path::new).map(Path::to_path_buf).collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    /// Every file in `root` whose bytes contain `needle`, repo-relative.
    ///
    /// Deliberately `git grep --fixed-strings`: the claim under test is that
    /// the artifact survives *a plain textual search*, so the check has to be
    /// a plain textual search and not a smarter one. `git grep` also skips
    /// `.git/`, where the committed blobs would otherwise match everything.
    fn files_mentioning(root: &Path, needle: &str) -> Vec<String> {
        let output = Command::new("git")
            .args(["grep", "-I", "-l", "--untracked", "--fixed-strings", needle])
            .current_dir(root)
            .output()
            .expect("git grep should run inside a materialized fixture");
        String::from_utf8(output.stdout)
            .expect("fixture files are UTF-8")
            .lines()
            .map(str::to_string)
            .collect()
    }

    fn materialize_into_tempdir() -> (tempfile::TempDir, GroundTruth) {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let truth = YamlStringRef
            .materialize(dir.path())
            .expect("m01 materializes");
        (dir, truth)
    }

    #[test]
    fn m01_is_a_real_git_repository_whose_live_artifact_is_committed() {
        let (dir, _truth) = materialize_into_tempdir();
        let repo = Repo::discover(dir.path()).expect("fixture is a git working tree");

        // A blob SHA at HEAD exists only if there is a commit containing it, so
        // this asserts "real repo" and "one commit" together. Recoverability is
        // part of what the suite exercises (Gate 0g), so it is not incidental
        // that the fixture is committed rather than merely initialised.
        assert!(
            repo.blob_sha(Path::new(LIVE))
                .expect("blob_sha query succeeds")
                .is_some(),
            "{LIVE} must be present in HEAD"
        );
    }

    #[test]
    fn m01_ground_truth_names_files_that_are_really_there() {
        let (dir, truth) = materialize_into_tempdir();

        assert_eq!(truth.live_paths, vec![Path::new(LIVE).to_path_buf()]);
        assert_eq!(truth.live_symbols, vec!["DunningConfig".to_string()]);
        assert_eq!(truth.decoy_dead_paths.len(), YamlStringRef::DECOYS.len());

        for path in truth.live_paths.iter().chain(&truth.decoy_dead_paths) {
            assert!(
                dir.path().join(path).is_file(),
                "ground truth names {} but it is not on disk",
                path.display()
            );
        }
    }

    #[test]
    fn m01_live_module_is_invisible_to_a_basename_search_but_visible_to_a_stem_veto() {
        let (dir, _truth) = materialize_into_tempdir();

        // The mutant is only hard if this holds: no file — not even the one
        // config that keeps the module alive — spells its filename. The
        // reference is dotted, so the basename genuinely occurs nowhere.
        let elsewhere: Vec<String> = files_mentioning(dir.path(), "dunning.py")
            .into_iter()
            .filter(|hit| hit != LIVE)
            .collect();
        assert!(
            elsewhere.is_empty(),
            "nothing outside {LIVE} may contain its basename; found {elsewhere:?}"
        );

        // And it is only *fair* if this holds: §6.2's mandatory whole-repo
        // literal veto, applied to the stem, does find it. A mutant nothing can
        // solve measures nothing.
        let stem_hits = files_mentioning(dir.path(), "ledger.dunning");
        assert!(
            stem_hits.contains(&MECHANISM.to_string()),
            "the settings list is the one rescue signal; hits were {stem_hits:?}"
        );
    }

    #[test]
    fn m01_decoys_are_named_nowhere_at_all() {
        let (dir, truth) = materialize_into_tempdir();

        for decoy in &truth.decoy_dead_paths {
            let stem = decoy
                .file_stem()
                .and_then(|s| s.to_str())
                .expect("decoy has a UTF-8 stem");
            let own_path = decoy.to_string_lossy().to_string();
            let elsewhere: Vec<String> = files_mentioning(dir.path(), stem)
                .into_iter()
                .filter(|hit| *hit != own_path)
                .collect();
            assert!(
                elsewhere.is_empty(),
                "a decoy anything mentions is not a decoy; {own_path} is named by {elsewhere:?}"
            );
        }
    }
}
