//! Class 1 — referenced only by a string in a YAML/JSON config.
//!
//! **The mechanism.** `ledger/dunning.py` defines `DunningConfig`. The only
//! thing in the repository that names it is the string
//! `"ledger.dunning.DunningConfig"` in `ledger/apps.yaml`, which
//! `settings.py` loads at import time to build `INSTALLED_APPS` — §6.2's first
//! named shape (Django `INSTALLED_APPS = ['myapp.SomeConfig']`) in its
//! externalised-config form. Django imports the module and instantiates the
//! class at startup; nothing in the repository imports it.
//!
//! **Why the list is in YAML and not in `settings.py`.** An earlier revision
//! put the dotted string directly in `settings.py` and argued that was harder,
//! because a tool parsing every file as code would still only see a string
//! literal. That reasoning inverts for the signal §6.2 marks *mandatory*: a
//! whole-repo literal veto reads `.py` files, finds the stem, and vetoes — so
//! the mutant was passed by every grep-based cleaner without their having
//! implemented anything. §10 E2 class 1 says *YAML/JSON config*, and
//! `fixtures/mod.rs` says classes 1–14 share the shape "a reference in a place
//! you didn't parse". `settings.py` is not such a place. `apps.yaml` is.
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

use crate::mutant::{Declaration, Ecosystem, GroundTruth, Mutant};

/// A Django `AppConfig` named only as a dotted string in a YAML app list. No
/// import, no call site: the reference exists, but only as data, and only in a
/// file no Python analyzer parses.
pub struct YamlStringRef;

/// Repo-relative path of the artifact that is alive and looks dead.
const LIVE: &str = "ledger/dunning.py";

/// The one file in the repository that names [`LIVE`], and how.
const MECHANISM: &str = "ledger/apps.yaml";

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
        r#"# The only reference in this repository to the dunning app.
#
# Django imports the module and instantiates the class at startup. To every
# Python analyzer this file does not exist: it is not code, it is not on the
# import graph, and nothing here is a symbol.
installed_apps:
  - django.contrib.contenttypes
  - django.contrib.auth
  - ledger.dunning.DunningConfig

middleware:
  - django.middleware.common.CommonMiddleware
"#,
    ),
    (
        "ledger/settings.py",
        r#""""Django settings, loaded from apps.yaml at import time.

Note what is NOT here: no app is named in this file. The list is data, read
from a file the parser does not read, so the import graph reaches settings and
stops -- and a whole-repo literal veto only helps if it searches YAML too.
"""

import pathlib
import yaml

_CONFIG = yaml.safe_load((pathlib.Path(__file__).parent / "apps.yaml").read_text())

INSTALLED_APPS = _CONFIG["installed_apps"]
MIDDLEWARE = _CONFIG["middleware"]
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

    /// The symbol each decoy defines, index-aligned with [`Self::DECOYS`], so a
    /// symbol-level analyzer is asked a question it can answer. Without these a
    /// tool that only ever names symbols scores zero decoys and reads as having
    /// found nothing (see `GroundTruth::decoy_dead_symbols`).
    const DECOY_SYMBOLS: [&'static str; 2] = ["dump_invoices", "RATES"];
}

impl Mutant for YamlStringRef {
    fn id(&self) -> &str {
        "m01"
    }
    fn ecosystem(&self) -> Ecosystem {
        Ecosystem::Python
    }
    fn mechanism(&self) -> &str {
        "dotted class path appearing only as a string in a YAML app list"
    }
    fn research_ref(&self) -> &str {
        "§10 E2 class 1"
    }
    /// `settings.py` reads `apps.yaml` at import time and Django imports the module
    /// to instantiate the class, so a test process that boots the app loads the
    /// file. `DunningConfig` is a **class**, and a class has no `FNDA` record —
    /// only functions do — so the symbol claim gets no coverage evidence at all.
    fn coverage_declaration(&self) -> Declaration {
        Declaration::loaded(["ledger/dunning.py"])
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
            decoy_dead_paths: Self::DECOYS
                .iter()
                .map(Path::new)
                .map(Path::to_path_buf)
                .collect(),
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
    fn m01_is_a_real_git_repository_whose_live_artifact_is_committed() {
        let (_dir, repo, _truth) = support::materialize(&YamlStringRef);

        // Recoverability is part of what the suite exercises (Gate 0g), so it
        // is not incidental that the fixture is committed rather than merely
        // initialised.
        support::assert_committed(&repo, &[LIVE]);
    }

    #[test]
    fn m01_ground_truth_names_files_that_are_really_there() {
        let (_dir, repo, truth) = support::materialize(&YamlStringRef);

        assert_eq!(truth.live_paths, vec![Path::new(LIVE).to_path_buf()]);
        assert_eq!(truth.live_symbols, vec!["DunningConfig".to_string()]);
        assert_eq!(truth.decoy_dead_paths.len(), YamlStringRef::DECOYS.len());

        support::assert_ground_truth_is_on_disk(&repo, &truth);
    }

    #[test]
    fn m01_live_module_is_invisible_to_a_basename_search_but_visible_to_a_stem_veto() {
        let (_dir, repo, _truth) = support::materialize(&YamlStringRef);

        // The mutant is only hard if this holds: no file — not even the one
        // config that keeps the module alive — spells its filename. The
        // reference is dotted, so the basename genuinely occurs nowhere.
        let elsewhere = support::references_outside(repo.root(), "dunning.py", LIVE);
        assert!(
            elsewhere.is_empty(),
            "nothing outside {LIVE} may contain its basename; found {elsewhere:?}"
        );

        // And it is only *fair* if this holds: §6.2's mandatory whole-repo
        // literal veto, applied to the stem, does find it. A mutant nothing can
        // solve measures nothing.
        let stem_hits = support::files_mentioning(repo.root(), "ledger.dunning");
        assert!(
            stem_hits.contains(&MECHANISM.to_string()),
            "the settings list is the one rescue signal; hits were {stem_hits:?}"
        );
    }

    #[test]
    fn m01_decoys_are_named_nowhere_at_all() {
        let (_dir, repo, truth) = support::materialize(&YamlStringRef);
        support::assert_decoys_are_unreferenced(&repo, &truth);
    }
}
