//! Class 8 — referenced only from a Dockerfile / CI workflow / k8s manifest.
//!
//! **The mechanism.** Two artifacts, each named exactly once, in a file written
//! in a language that has nothing to do with the project's ecosystem:
//!
//! - `scripts/verify_release.sh` appears only in a `run:` body in
//!   `.github/workflows/ci.yml`.
//! - `deploy/uwsgi.ini` appears only in a `COPY` instruction in `Dockerfile`.
//!
//! §5.2's root checklist names both lines explicitly — "`.github/workflows/*.yml`
//! `run:` bodies" under CI, and "`Dockerfile` … `COPY`/`ADD` (source paths!)"
//! under Containers. The exclamation mark is in the research, not added here.
//!
//! **Why every other signal misses it.** The project is Python; every Python
//! tool reads `.py` files. A shell script invoked by CI has no importer, no
//! caller, and no entry in `pyproject.toml`. A `.ini` consumed by uWSGI inside a
//! container image is never opened by anything in the repository at all — the
//! process that reads it does not exist until the image runs. Both files are, to
//! a reachability pass, unreferenced leaves; both are load-bearing.
//!
//! Note the second-order trap in the workflow: `actions/checkout@v4` clones at
//! depth 1. §6.19 makes that the CI default, so this is also the shape of repo
//! in which history-derived evidence must abstain rather than accuse.
//!
//! **What is supposed to catch it.** §6.20's whole-repo literal veto *over every
//! file type* — the word "every" is what this mutant tests. A veto that reads
//! source files and skips YAML and Dockerfiles scores exactly zero here, and
//! that is the common implementation.

use std::path::Path;

use judged_core::git::Repo;
use judged_core::{Error, Result};

use crate::mutant::{Ecosystem, GroundTruth, Mutant};

/// The reference is real, executable, and written in a language no code
/// analyzer for the project's ecosystem reads.
pub struct CiManifestRef;

/// Referenced only from a CI `run:` body.
const LIVE_SCRIPT: &str = "scripts/verify_release.sh";

/// Referenced only from a Dockerfile `COPY` source path.
const LIVE_CONFIG: &str = "deploy/uwsgi.ini";

/// The two manifests that carry those references.
const MECHANISM_CI: &str = ".github/workflows/ci.yml";
const MECHANISM_DOCKER: &str = "Dockerfile";

/// Files written into the mutant repository, as `(repo-relative path, body)`.
const FILES: &[(&str, &str)] = &[
    (
        "pyproject.toml",
        r#"[project]
name = "svc"
version = "0.1.0"
requires-python = ">=3.11"

[build-system]
requires = ["setuptools>=68"]
build-backend = "setuptools.build_meta"
"#,
    ),
    ("svc/__init__.py", "\"\"\"The service.\"\"\"\n"),
    (
        "svc/main.py",
        r#""""The whole application, as far as any Python tool can see.

Nothing here opens a script or a uWSGI config: the release check runs in CI
and the config is read by a process inside the container image.
"""


def handle(request: dict) -> dict:
    return {"ok": True, "path": request.get("path", "/")}


if __name__ == "__main__":
    print(handle({"path": "/health"}))
"#,
    ),
    (
        MECHANISM_CI,
        r#"name: ci

on:
  push:
    branches: [main]

jobs:
  release:
    runs-on: ubuntu-latest
    steps:
      # Depth 1 by default (§6.19): the clone this workflow produces cannot
      # answer history questions at all, and does so silently.
      - uses: actions/checkout@v4
      - name: Verify the release artifacts
        run: bash scripts/verify_release.sh
"#,
    ),
    (
        LIVE_SCRIPT,
        r#"#!/usr/bin/env bash
# LIVE. Invoked only from the release job in .github/workflows/ci.yml.
# No Python module imports it, and pyproject.toml does not mention it.
set -euo pipefail

test -f pyproject.toml
python -c "import svc.main; print(svc.main.handle({}))"
echo "release artifacts verified"
"#,
    ),
    (
        MECHANISM_DOCKER,
        r#"FROM python:3.11-slim

WORKDIR /app
COPY pyproject.toml ./
COPY svc/ ./svc/

# The only reference in this repository to the uWSGI config. The process that
# reads it does not exist until this image runs.
COPY deploy/uwsgi.ini /etc/uwsgi/uwsgi.ini

CMD ["uwsgi", "--ini", "/etc/uwsgi/uwsgi.ini"]
"#,
    ),
    (
        LIVE_CONFIG,
        r#"; LIVE. Copied into the image by the Dockerfile and read by uWSGI at
; container start. Nothing in this repository opens it.
[uwsgi]
module = svc.main
processes = 4
threads = 2
http-socket = :8080
"#,
    ),
    (
        "scripts/old_benchmark.sh",
        r#"#!/usr/bin/env bash
# DEAD DECOY. Was wired into a CI job that no longer exists; no workflow, no
# Dockerfile, and no Python module names it now.
set -euo pipefail
python -c "print('benchmark')"
"#,
    ),
    (
        "deploy/unused_nginx.conf",
        r#"# DEAD DECOY. Sits beside a config that only a COPY line keeps alive,
# which is the discrimination the suite is measuring.
server {
    listen 8081;
    location / { return 404; }
}
"#,
    ),
];

impl CiManifestRef {
    /// Repo-relative paths of the genuinely-dead files planted here.
    const DECOYS: [&'static str; 2] = ["scripts/old_benchmark.sh", "deploy/unused_nginx.conf"];
}

impl Mutant for CiManifestRef {
    fn id(&self) -> &str {
        "m08"
    }
    fn ecosystem(&self) -> Ecosystem {
        Ecosystem::Polyglot
    }
    fn mechanism(&self) -> &str {
        "script invoked only from a CI workflow, Dockerfile or k8s manifest"
    }
    fn research_ref(&self) -> &str {
        "§10 E2 class 8"
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
        repo.commit("m08: service whose release script and uWSGI config live in manifests")?;

        Ok(GroundTruth {
            live_paths: vec![
                Path::new(LIVE_SCRIPT).to_path_buf(),
                Path::new(LIVE_CONFIG).to_path_buf(),
            ],
            live_symbols: Vec::new(),
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
        let truth = CiManifestRef
            .materialize(dir.path())
            .expect("m08 materializes");
        (dir, truth)
    }

    #[test]
    fn m08_is_a_real_git_repository_with_both_manifest_targets_committed() {
        let (dir, _truth) = materialize_into_tempdir();
        let repo = Repo::discover(dir.path()).expect("fixture is a git working tree");

        for live in [LIVE_SCRIPT, LIVE_CONFIG] {
            assert!(
                repo.blob_sha(Path::new(live))
                    .expect("blob_sha query succeeds")
                    .is_some(),
                "{live} must be present in HEAD"
            );
        }
    }

    #[test]
    fn m08_ground_truth_names_files_that_are_really_there() {
        let (dir, truth) = materialize_into_tempdir();

        assert_eq!(
            truth.live_paths,
            vec![
                Path::new(LIVE_SCRIPT).to_path_buf(),
                Path::new(LIVE_CONFIG).to_path_buf()
            ]
        );
        // No symbol-level claim: the artifacts are whole files, and the class is
        // about a path reference, not a name reference.
        assert!(truth.live_symbols.is_empty());
        assert_eq!(truth.decoy_dead_paths.len(), CiManifestRef::DECOYS.len());

        for path in truth.live_paths.iter().chain(&truth.decoy_dead_paths) {
            assert!(
                dir.path().join(path).is_file(),
                "ground truth names {} but it is not on disk",
                path.display()
            );
        }
    }

    #[test]
    fn m08_each_live_file_is_named_by_exactly_one_manifest_and_no_source_file() {
        let (dir, _truth) = materialize_into_tempdir();

        for (live, basename, mechanism) in [
            (LIVE_SCRIPT, "verify_release.sh", MECHANISM_CI),
            (LIVE_CONFIG, "uwsgi.ini", MECHANISM_DOCKER),
        ] {
            let elsewhere = references_outside(dir.path(), basename, live);
            assert_eq!(
                elsewhere,
                vec![mechanism.to_string()],
                "{live} must be named by {mechanism} and by nothing else"
            );
        }
    }

    #[test]
    fn m08_decoys_are_named_nowhere_at_all() {
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
