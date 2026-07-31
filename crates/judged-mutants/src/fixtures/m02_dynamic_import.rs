//! Class 2 — loaded via `importlib` / `require(variable)` / `Class.forName`.
//!
//! **The mechanism.** Two halves of the same failure, in one repository,
//! because §6.2's runtime-constructed shape is not a property of a language:
//!
//! - `app/main.py` calls `importlib.import_module(f"app.backends.{BACKEND}_backend")`.
//!   The live module is `app/backends/redis_backend.py`.
//! - `src/index.ts` calls `import()` on a template literal that interpolates a
//!   runtime `kind` into `./transports/<kind>Transport.js`. The live module is
//!   `src/transports/websocketTransport.ts`.
//!
//! Neither module name exists as a contiguous literal anywhere. §6.2 names
//! exactly this: the runtime-constructed variants Meta calls out, `"tbl_" +
//! region`, where the reference is "not a contiguous literal at all".
//!
//! **Why every other signal misses it.** Import resolution — `tsc`, a language
//! server, a bundler, `pyflakes`, any call-graph pass — resolves *static*
//! specifiers. A template literal and an f-string are expressions evaluated at
//! runtime; the resolver has nothing to resolve. `tsconfig.json` names only the
//! entry point, so even the build graph stops at `src/index.ts`. And a filename
//! search fails for the same reason as m01: the constructed name is assembled
//! from a prefix, a variable, and a suffix, none of which is the basename.
//!
//! **What is supposed to catch it.** §6.2's concatenation counter-signal —
//! "if the basename minus a common prefix/suffix appears as a literal ...
//! block" — plus §6.12's directory rule, since `backends` and `transports` both
//! appear as literals and a matched directory must be treated as rooted. Both
//! halves are asserted below.
//!
//! **Why both halves live in one fixture.** A tool that resolves Python
//! `importlib` and ignores template-literal `import()` has not handled the
//! class, and grading it as if it had would overstate its safety on every
//! polyglot repository — which is most of them.

use std::path::Path;

use judged_core::git::Repo;
use judged_core::{Error, Result};

use crate::mutant::{Ecosystem, GroundTruth, Mutant};

/// Polyglot on purpose: the Python half uses `importlib.import_module` with
/// a name built at runtime, the TypeScript half uses a template-literal
/// `import()`. Both defeat static import resolution, and a tool that handles
/// one and not the other should not score as if it handled the class.
pub struct DynamicImport;

/// The Python module reachable only through the f-string specifier.
const LIVE_PYTHON: &str = "app/backends/redis_backend.py";

/// The TypeScript module reachable only through the template-literal specifier.
const LIVE_TYPESCRIPT: &str = "src/transports/websocketTransport.ts";

/// The two files that assemble those specifiers at runtime.
const MECHANISM_PYTHON: &str = "app/main.py";
const MECHANISM_TYPESCRIPT: &str = "src/index.ts";

/// Files written into the mutant repository, as `(repo-relative path, body)`.
const FILES: &[(&str, &str)] = &[
    (
        "pyproject.toml",
        r#"[project]
name = "ledger-transport"
version = "0.1.0"
requires-python = ">=3.11"

[build-system]
requires = ["setuptools>=68"]
build-backend = "setuptools.build_meta"
"#,
    ),
    (
        "package.json",
        r#"{
  "name": "ledger-transport",
  "version": "0.1.0",
  "private": true,
  "type": "module",
  "scripts": {
    "build": "tsc -p tsconfig.json"
  },
  "devDependencies": {
    "typescript": "^5.4.0"
  }
}
"#,
    ),
    (
        // `files` and not `include`: the build graph names the entry point and
        // nothing else, which is the whole point. A glob here would root the
        // transport directory by accident and the mutant would test nothing.
        "tsconfig.json",
        r#"{
  "compilerOptions": {
    "target": "ES2022",
    "module": "ES2022",
    "moduleResolution": "bundler",
    "outDir": "dist",
    "strict": true
  },
  "files": ["src/index.ts"]
}
"#,
    ),
    ("app/__init__.py", "\"\"\"Ledger transport service.\"\"\"\n"),
    (
        MECHANISM_PYTHON,
        r#""""Entry point.

The backend module name is assembled at runtime, so no static import of any
backend exists anywhere in this repository. Note what a reader can still see
and a parser cannot use: the package prefix and the default value are both
plain literals, one character apart from the answer.
"""

import importlib
import os

BACKEND = os.environ.get("LEDGER_BACKEND", "redis")


def load_backend():
    module = importlib.import_module(f"app.backends.{BACKEND}_backend")
    return module.build()


if __name__ == "__main__":
    print(load_backend())
"#,
    ),
    ("app/backends/__init__.py", "\"\"\"Backend implementations.\"\"\"\n"),
    (
        LIVE_PYTHON,
        r#""""LIVE. Imported only by importlib.import_module() in app/main.py.

Nothing imports this module statically. Deleting it leaves the test suite
green and every import in the repository resolvable; the failure is an
ImportError in production the first time a connection is opened.
"""


class RedisBackend:
    def __init__(self, url: str = "redis://localhost:6379/0") -> None:
        self.url = url

    def ping(self) -> bool:
        return True


def build() -> RedisBackend:
    return RedisBackend()
"#,
    ),
    (
        MECHANISM_TYPESCRIPT,
        r#"// Entry point. The transport specifier is a template literal, so tsc,
// every language server, and every bundler resolve it to nothing.
const kind = process.env.TRANSPORT ?? "websocket";

export async function connect(url: string): Promise<unknown> {
  const mod = await import(`./transports/${kind}Transport.js`);
  return mod.create(url);
}
"#,
    ),
    (
        LIVE_TYPESCRIPT,
        r#"// LIVE. Reached only by the template-literal import() in src/index.ts.
export class WebsocketTransport {
  constructor(readonly url: string) {}

  send(frame: string): void {
    void frame;
  }
}

export function create(url: string): WebsocketTransport {
  return new WebsocketTransport(url);
}
"#,
    ),
    (
        "app/legacy_report_dump.py",
        r#""""DEAD DECOY. No import, no string, no config entry. It sits in the
same package as a module that only a runtime string keeps alive, which is
the discrimination the suite is actually measuring.
"""


def dump(rows: list[dict]) -> int:
    return len(rows)
"#,
    ),
    (
        "src/unusedAnalytics.ts",
        r#"// DEAD DECOY. Outside the tsconfig entry graph, named by no literal,
// and reachable by no runtime specifier this repository can construct.
export function trackPageView(page: string): void {
  void page;
}
"#,
    ),
];

impl DynamicImport {
    /// Repo-relative paths of the genuinely-dead files planted here.
    const DECOYS: [&'static str; 2] = ["app/legacy_report_dump.py", "src/unusedAnalytics.ts"];
}

impl Mutant for DynamicImport {
    fn id(&self) -> &str {
        "m02"
    }
    fn ecosystem(&self) -> Ecosystem {
        Ecosystem::Polyglot
    }
    fn mechanism(&self) -> &str {
        "module name computed at runtime and passed to importlib / require"
    }
    fn research_ref(&self) -> &str {
        "§10 E2 class 2"
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
        repo.commit("m02: transport service with two runtime-constructed imports")?;

        Ok(GroundTruth {
            live_paths: vec![
                Path::new(LIVE_PYTHON).to_path_buf(),
                Path::new(LIVE_TYPESCRIPT).to_path_buf(),
            ],
            live_symbols: vec!["RedisBackend".to_string(), "WebsocketTransport".to_string()],
            decoy_dead_paths: Self::DECOYS
                .iter()
                .map(Path::new)
                .map(Path::to_path_buf)
                .collect(),
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
    /// a plain textual search and not a smarter one.
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

    /// Files other than `artifact` itself that contain `needle`.
    fn references_outside(root: &Path, needle: &str, artifact: &str) -> Vec<String> {
        files_mentioning(root, needle)
            .into_iter()
            .filter(|hit| hit != artifact)
            .collect()
    }

    fn materialize_into_tempdir() -> (tempfile::TempDir, GroundTruth) {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let truth = DynamicImport
            .materialize(dir.path())
            .expect("m02 materializes");
        (dir, truth)
    }

    #[test]
    fn m02_is_a_real_git_repository_with_both_halves_committed() {
        let (dir, _truth) = materialize_into_tempdir();
        let repo = Repo::discover(dir.path()).expect("fixture is a git working tree");

        for live in [LIVE_PYTHON, LIVE_TYPESCRIPT] {
            assert!(
                repo.blob_sha(Path::new(live))
                    .expect("blob_sha query succeeds")
                    .is_some(),
                "{live} must be present in HEAD"
            );
        }
    }

    #[test]
    fn m02_ground_truth_names_files_that_are_really_there() {
        let (dir, truth) = materialize_into_tempdir();

        assert_eq!(
            truth.live_paths,
            vec![
                Path::new(LIVE_PYTHON).to_path_buf(),
                Path::new(LIVE_TYPESCRIPT).to_path_buf()
            ]
        );
        assert_eq!(
            truth.live_symbols,
            vec!["RedisBackend".to_string(), "WebsocketTransport".to_string()]
        );
        assert_eq!(truth.decoy_dead_paths.len(), DynamicImport::DECOYS.len());

        for path in truth.live_paths.iter().chain(&truth.decoy_dead_paths) {
            assert!(
                dir.path().join(path).is_file(),
                "ground truth names {} but it is not on disk",
                path.display()
            );
        }
    }

    #[test]
    fn m02_neither_constructed_module_name_exists_as_a_literal() {
        let (dir, _truth) = materialize_into_tempdir();

        for (live, basename) in [
            (LIVE_PYTHON, "redis_backend"),
            (LIVE_TYPESCRIPT, "websocketTransport"),
        ] {
            let elsewhere = references_outside(dir.path(), basename, live);
            assert!(
                elsewhere.is_empty(),
                "{live} is supposed to be unnamed outside itself; {basename} appears in {elsewhere:?}"
            );
        }
    }

    #[test]
    fn m02_both_halves_are_solvable_by_the_prefix_and_directory_counter_signals() {
        let (dir, _truth) = materialize_into_tempdir();

        // §6.2: the basename minus its suffix does appear as a literal, in the
        // very file that builds the specifier. §6.12: so does the containing
        // directory name, which roots the whole directory.
        for (mechanism, fragment, directory) in [
            (MECHANISM_PYTHON, "redis", "app.backends."),
            (MECHANISM_TYPESCRIPT, "websocket", "./transports/"),
        ] {
            for needle in [fragment, directory] {
                let hits = files_mentioning(dir.path(), needle);
                assert!(
                    hits.contains(&mechanism.to_string()),
                    "{mechanism} must contain the literal {needle:?}; hits were {hits:?}"
                );
            }
        }
    }

    #[test]
    fn m02_decoys_are_named_nowhere_at_all() {
        let (dir, truth) = materialize_into_tempdir();

        for decoy in &truth.decoy_dead_paths {
            let stem = decoy
                .file_stem()
                .and_then(|s| s.to_str())
                .expect("decoy has a UTF-8 stem");
            let own_path = decoy.to_string_lossy().to_string();
            let elsewhere = references_outside(dir.path(), stem, &own_path);
            assert!(
                elsewhere.is_empty(),
                "a decoy anything mentions is not a decoy; {own_path} is named by {elsewhere:?}"
            );
        }
    }
}
