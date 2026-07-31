//! Class 10 — loaded by framework convention.
//!
//! **Mechanism.** Two conventions, no imports, in one repository:
//!
//! - Django ≥3.2 loads an app by scanning `<app>/apps.py` for the single
//!   `AppConfig` subclass it contains and instantiating it. `INSTALLED_APPS`
//!   names the *package*; the class name is written down nowhere.
//! - Jest substitutes a root `__mocks__/<package>.js` for a node_modules
//!   package automatically, with no `jest.mock()` call. The directory name is
//!   the whole registration.
//!
//! **Why every other signal misses it.** There is no reference to follow in
//! either case, so a call graph, a compiler index and a module resolver all
//! agree the two files are unreachable — correctly, because the edge is in the
//! framework's loader, not in this repository. It is not even a *dynamic*
//! reference of the §6.1 kind: no `importlib`, no `require(variable)`, nothing
//! to flag as a reflection primitive. Only knowing the framework's rule
//! rescues these files, which is why §10 E2 lists the convention itself as a
//! class rather than folding it into class 2.
//!
//! `Polyglot` because the convention, not the language, is what is under test:
//! the Python half and the JavaScript half fail identically.

use std::path::Path;

use judged_core::git::Repo;
use judged_core::Result;

use crate::fixtures::write;
use crate::mutant::{Ecosystem, GroundTruth, Mutant};

/// Django's `AppConfig` autoload and Jest's `__mocks__` directory in one
/// repository: two frameworks, two conventions, neither expressed as an
/// import. Polyglot because the convention, not the language, is the thing
/// under test.
pub struct FrameworkConvention;

impl Mutant for FrameworkConvention {
    fn id(&self) -> &str {
        "m10"
    }
    fn ecosystem(&self) -> Ecosystem {
        Ecosystem::Polyglot
    }
    fn mechanism(&self) -> &str {
        "loaded by framework convention: Django AppConfig autoload, Jest __mocks__"
    }
    fn research_ref(&self) -> &str {
        "§10 E2 class 10"
    }
    fn materialize(&self, dir: &Path) -> Result<GroundTruth> {
        let repo = Repo::init(dir)?;
        let root = repo.root().to_path_buf();

        // ---- Django half -------------------------------------------------
        write(
            &root,
            "pyproject.toml",
            "[project]\nname = \"billing\"\nversion = \"0.1.0\"\n\
             dependencies = [\"django>=4.2\"]\n",
        )?;
        write(
            &root,
            "billing/settings.py",
            r#"INSTALLED_APPS = [
    "django.contrib.contenttypes",
    "reporting",
]

SECRET_KEY = "fixture-only"
"#,
        )?;
        write(&root, "reporting/__init__.py", "")?;

        // THE LIVE ARTIFACT (Python half). Django instantiates this class at
        // startup because it is the one AppConfig subclass in apps.py. The
        // string "ReportingConfig" occurs in this file and in no other.
        write(
            &root,
            "reporting/apps.py",
            r#"from django.apps import AppConfig


class ReportingConfig(AppConfig):
    name = "reporting"
    verbose_name = "Reporting"

    def ready(self):
        # Runs once at startup, after the app registry is populated.
        from django.core.signals import request_started

        request_started.connect(lambda **_: None, weak=False)
"#,
        )?;

        // ---- Jest half ---------------------------------------------------
        write(
            &root,
            "package.json",
            r#"{
  "name": "billing-web",
  "version": "0.1.0",
  "private": true,
  "scripts": { "test": "jest" },
  "dependencies": { "redis": "^4.6.0" },
  "devDependencies": { "jest": "^29.7.0" }
}
"#,
        )?;
        write(
            &root,
            "src/cache.js",
            r#"const { createClient } = require("redis");

async function warm(keys) {
  const client = createClient();
  await client.connect();
  return Promise.all(keys.map((key) => client.get(key)));
}

module.exports = { warm };
"#,
        )?;
        write(
            &root,
            "tests/cache.test.js",
            r#"const { warm } = require("../src/cache");

test("warm resolves every key", async () => {
  await expect(warm(["a", "b"])).resolves.toEqual([null, null]);
});
"#,
        )?;

        // THE LIVE ARTIFACT (JavaScript half). Jest swaps this in for the real
        // package whenever a test transitively requires it. Deliberately no
        // comment naming the directory: the test asserts that the convention's
        // own name appears in no file, which is the property under test.
        write(
            &root,
            "__mocks__/redis.js",
            r#"// Stands in for the real client during tests.
const store = new Map();

function createClient() {
  return {
    connect: async () => {},
    get: async (key) => store.get(key) ?? null,
    set: async (key, value) => void store.set(key, value),
  };
}

module.exports = { createClient };
"#,
        )?;

        // ---- Decoys ------------------------------------------------------
        // One per ecosystem, so that a tool cannot pass the Jest half by
        // reporting only Python findings.
        write(
            &root,
            "reporting/textwrap_helper.py",
            "def hang_indent(text, width=72):\n    return text\n",
        )?;
        write(
            &root,
            "src/color_utils.js",
            "function toHex(rgb) {\n  return rgb.map((c) => c.toString(16)).join(\"\");\n}\n\n\
             module.exports = { toHex };\n",
        )?;

        repo.add_all()?;
        repo.commit("m10: Django AppConfig autoload and a Jest module mock")?;

        Ok(GroundTruth {
            live_paths: vec!["reporting/apps.py".into(), "__mocks__/redis.js".into()],
            live_symbols: vec!["ReportingConfig".to_string()],
            decoy_dead_paths: vec![
                "reporting/textwrap_helper.py".into(),
                "src/color_utils.js".into(),
            ],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::support;

    #[test]
    fn m10_materializes_a_real_git_repo_with_one_commit() {
        let (_dir, repo, _truth) = support::materialize(&FrameworkConvention);

        // m10 is a two-ecosystem fixture, so both manifests have to be there.
        support::assert_committed(&repo, &["pyproject.toml", "package.json"]);
    }

    #[test]
    fn m10_ground_truth_paths_all_exist_on_disk() {
        let (_dir, repo, truth) = support::materialize(&FrameworkConvention);
        assert_eq!(
            truth.live_paths.len(),
            2,
            "m10 injects one convention per framework: Django and Jest"
        );
        support::assert_ground_truth_is_on_disk(&repo, &truth);
    }

    /// Django ≥3.2 finds the single `AppConfig` subclass in `<app>/apps.py` by
    /// scanning the module. Nothing writes the class name down, so a tool that
    /// resolves `INSTALLED_APPS` strings still never arrives at this symbol.
    #[test]
    fn m10_the_app_config_class_is_named_only_by_its_own_definition() {
        let (_dir, repo, truth) = support::materialize(&FrameworkConvention);
        let symbol = truth
            .live_symbols
            .first()
            .expect("m10 declares the AppConfig class as a live symbol");
        let naming: Vec<String> = support::tree(repo.root())
            .into_iter()
            .filter(|(_, bytes)| support::mentions(bytes, symbol))
            .map(|(path, _)| path)
            .collect();
        assert_eq!(
            naming,
            vec!["reporting/apps.py".to_string()],
            "{symbol} must be discovered by convention, not by reference"
        );
    }

    /// Jest substitutes a root `__mocks__/<module>.js` for a node_modules
    /// package automatically — no `jest.mock()` call required. So the directory
    /// name is the entire registration, and it must appear in no file.
    #[test]
    fn m10_nothing_in_the_repository_references_the_mocks_directory() {
        let (_dir, repo, _truth) = support::materialize(&FrameworkConvention);
        for (path, bytes) in support::tree(repo.root()) {
            assert!(
                !support::mentions(&bytes, "__mocks__"),
                "{path} names __mocks__; Jest's convention must be the only \
                 thing that puts the mock in the module graph"
            );
        }
    }

    #[test]
    fn m10_the_decoys_are_named_by_nothing() {
        let (_dir, repo, truth) = support::materialize(&FrameworkConvention);
        support::assert_decoys_are_unreferenced(&repo, &truth);
    }
}
