//! Class 11 — an ORM/serializer field touched only via reflection
//! *(Periphery's Codable case)*.
//!
//! **Mechanism.** `app/serialize.py` walks `type(model).model_fields` and pulls
//! each value with `getattr`. Every field of `RetentionPolicy` therefore
//! reaches the wire, and not one of them is written down outside the class
//! body — not in the serializer, not in the caller, which constructs the model
//! with `**raw` from parsed JSON.
//!
//! **Why every other signal misses it.** §6.1 is explicit that *structural*
//! reflection "defeats even the grep veto — **there is no identifier string
//! anywhere to match**, because the reflection is over shape, not name." An
//! attribute-access index finds no reads; a compiler index finds no
//! references; the grep veto has nothing to grep. Periphery hits this with
//! Swift `Codable` and ships a dedicated retention rule for it.
//!
//! The mutant is deliberately built so that §6.1's own counter-signal fires:
//! "the candidate is a whole struct/class whose *fields are individually
//! unreferenced* — a strong tell for serialization or reflection." Here that
//! is true of every field at once, which is the strongest form of the tell a
//! tool will ever get. If a cleaner cannot use it here it cannot use it
//! anywhere.

use std::path::Path;

use judged_core::git::Repo;
use judged_core::{Error, Result};

use crate::mutant::{Ecosystem, GroundTruth, Mutant};

/// The field is never read by name anywhere. The serializer walks it
/// reflectively, and deleting it silently changes the wire format.
pub struct ReflectiveField;

impl Mutant for ReflectiveField {
    fn id(&self) -> &str {
        "m11"
    }
    fn ecosystem(&self) -> Ecosystem {
        Ecosystem::Python
    }
    fn mechanism(&self) -> &str {
        "model field enumerated reflectively by a serializer, never named"
    }
    fn research_ref(&self) -> &str {
        "§10 E2 class 11"
    }
    fn materialize(&self, dir: &Path) -> Result<GroundTruth> {
        let repo = Repo::init(dir)?;
        let root = repo.root().to_path_buf();

        write(
            &root,
            "pyproject.toml",
            "[project]\nname = \"retention\"\nversion = \"0.1.0\"\n\
             requires-python = \">=3.11\"\ndependencies = [\"pydantic>=2\"]\n",
        )?;
        write(&root, "app/__init__.py", "")?;

        // THE LIVE ARTIFACT. Three fields, none of them read by name anywhere,
        // including here — the class body declares them and that is the last
        // time any of these identifiers occurs in the repository.
        write(
            &root,
            "app/models.py",
            r#"from pydantic import BaseModel


class RetentionPolicy(BaseModel):
    tenant_slug: str
    retention_days: int
    legal_hold_until: str | None = None
"#,
        )?;

        // The mechanism. Shape, not names: nothing here can be grepped for.
        write(
            &root,
            "app/serialize.py",
            r#"# Emit any pydantic model as a wire payload without naming a field.


def to_wire(model):
    payload = {}
    for name in type(model).model_fields:
        payload[name] = getattr(model, name)
    return payload
"#,
        )?;

        // The caller builds the model reflectively too, so the constructor
        // does not reintroduce keyword arguments that would name the fields.
        write(
            &root,
            "app/main.py",
            r#"import json
import sys

from .models import RetentionPolicy
from .serialize import to_wire


def main():
    raw = json.loads(sys.stdin.read())
    json.dump(to_wire(RetentionPolicy(**raw)), sys.stdout)
"#,
        )?;

        // THE DECOY. Imported by nothing, reflected over by nothing.
        write(
            &root,
            "app/color_utils.py",
            "def to_hex(rgb):\n    return \"\".join(format(c, \"02x\") for c in rgb)\n",
        )?;

        repo.add_all()?;
        repo.commit("m11: retention policy serialized by reflection over its shape")?;

        Ok(GroundTruth {
            // Symbols only, deliberately. `app/models.py` is live too, but it
            // is live the ordinary way — `app/main.py` imports it — and listing
            // it would let a tool fail m11 for a reason that has nothing to do
            // with reflection. The only way to fail this mutant is to call a
            // field dead.
            live_paths: Vec::new(),
            live_symbols: vec![
                "tenant_slug".to_string(),
                "retention_days".to_string(),
                "legal_hold_until".to_string(),
            ],
            decoy_dead_paths: vec!["app/color_utils.py".into()],
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

    fn materialize() -> (TempDir, Repo, GroundTruth) {
        let dir = TempDir::new().expect("create tempdir");
        let truth = ReflectiveField
            .materialize(dir.path())
            .expect("m11 materializes");
        let repo = Repo::discover(dir.path()).expect("fixture is a git repo");
        (dir, repo, truth)
    }

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
        assert!(
            repo.root().join(".git").is_dir(),
            "expected a git directory"
        );
        assert!(
            repo.is_tracked(Path::new("pyproject.toml"))
                .expect("query the index"),
            "the fixture must be committed, not just written to disk"
        );
    }

    #[test]
    fn ground_truth_declares_fields_and_a_decoy() {
        let (_dir, repo, truth) = materialize();
        assert!(
            !truth.live_symbols.is_empty(),
            "m11's live artifact is a set of field names, not a file"
        );
        assert!(
            !truth.decoy_dead_paths.is_empty(),
            "without a decoy, a tool that claims nothing passes m11 for free"
        );
        for path in truth.live_paths.iter().chain(&truth.decoy_dead_paths) {
            assert!(path.is_relative(), "{path:?} must be repo-relative");
            assert!(repo.root().join(path).is_file(), "{path:?} is missing");
        }
    }

    /// §2.1's tell, made literal: *every* field of the model is individually
    /// unreferenced. If even one were read by name the model would stop looking
    /// like a serialization boundary and the mutant would test something else.
    #[test]
    fn no_field_is_named_outside_the_model_definition() {
        let (_dir, repo, truth) = materialize();
        for field in &truth.live_symbols {
            let naming: Vec<String> = tree(repo.root())
                .into_iter()
                .filter(|(_, bytes)| mentions(bytes, field))
                .map(|(path, _)| path)
                .collect();
            assert_eq!(
                naming,
                vec!["app/models.py".to_string()],
                "{field} must be reachable only by reflection over the class"
            );
        }
    }

    /// The serializer must walk the model's shape, not a hand-written list.
    /// §6.1: for structural reflection "there is no identifier string anywhere
    /// to match", which is what defeats even the grep veto.
    #[test]
    fn the_serializer_enumerates_fields_reflectively() {
        let (_dir, repo, _truth) = materialize();
        let serializer = std::fs::read(repo.root().join("app/serialize.py"))
            .expect("the fixture ships a serializer");
        assert!(
            mentions(&serializer, "model_fields"),
            "the serializer must enumerate Pydantic's field map, not names"
        );
        assert!(
            mentions(&serializer, "getattr"),
            "field access must go through reflection, not attribute syntax"
        );
    }

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
