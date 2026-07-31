//! Class 15 — a worker class named only in an already-enqueued job payload *(§6.24)*.
//!
//! **Mechanism.** `RebuildInvoiceIndex` is a registered Celery task whose last
//! call site was removed when the feature was retired. One job naming it is
//! still sitting in the queue. The fixture commits that queue as
//! `var/broker/celery-default.jsonl`, a Kombu envelope with a **protocol 1**
//! body — which is base64. Celery puts the task name in the message body under
//! protocol 1 and in the headers under protocol 2; protocol 1 is used here
//! precisely because it reproduces the case where the name is not legible to
//! anything that reads the repository as text.
//!
//! **Why every other signal misses it.** §6.24: "Deleting `BackfillUserAvatars`
//! does not break the build, does not break any test, and does not break the
//! deploy — it breaks the *worker*, hours later, on jobs enqueued before the
//! deploy." Every safety property a cleaner can own is satisfied at the moment
//! of deletion. Static reachability finds no caller and is right. The grep veto
//! finds no occurrence and is right. Runtime coverage never observed the task
//! and is right. The test suite is green, and it is green *honestly*. The
//! evidence that would refute deadness is a message in another process's
//! queue, and §6.24 is blunt that no amount of scanning any repository at any
//! time can find it.
//!
//! Committing the queue file is a **concession to the harness**, not a
//! weakening of the class: in production the payload is a row in Redis on
//! another host. Even with that concession the name is unreachable textually,
//! which is why the test decodes the body rather than grepping for it.
//!
//! §6.24's rule is that no auto-act tier may contain a candidate whose name can
//! appear in a queue payload, and that the finding must carry a
//! *drain-the-queue-first* precondition rather than a delete recommendation.
//! The fixture supplies the counter-signal that rule keys off — a declared
//! `celery` dependency — so a tool that refuses here refuses for a reason it
//! could actually have found.

use std::path::Path;

use judged_core::git::Repo;
use judged_core::{Error, Result};

use crate::mutant::{Ecosystem, GroundTruth, Mutant};

/// The job Celery would hand a worker, protocol 1, before base64.
///
/// Kept as one constant so that the name written into the queue and the name
/// declared as ground truth cannot drift apart.
const ENQUEUED_JOB: &str = concat!(
    r#"{"id": "0f6a2c1e-8f4d-4b17-9a0e-2f5b7c3d1a44", "#,
    r#""task": "worker.tasks.RebuildInvoiceIndex", "#,
    r#""args": ["acme"], "kwargs": {}, "retries": 0, "eta": null}"#
);

/// Constructed by enqueuing, then deleting the class, then draining. §10 E2
/// is specific about the bar: the test suite must stay green **and** the
/// tool must still refuse. Green tests are not evidence here.
pub struct EnqueuedJobPayload;

impl Mutant for EnqueuedJobPayload {
    fn id(&self) -> &str {
        "m15"
    }
    fn ecosystem(&self) -> Ecosystem {
        Ecosystem::Python
    }
    fn mechanism(&self) -> &str {
        "worker class named only inside a job payload already sitting in the queue"
    }
    fn research_ref(&self) -> &str {
        "§10 E2 class 15"
    }
    fn materialize(&self, dir: &Path) -> Result<GroundTruth> {
        let repo = Repo::init(dir)?;
        let root = repo.root().to_path_buf();

        write(
            &root,
            "pyproject.toml",
            "[project]\nname = \"invoicing\"\nversion = \"0.1.0\"\n\
             requires-python = \">=3.11\"\ndependencies = [\"celery>=5.3\"]\n",
        )?;
        write(&root, "worker/__init__.py", "")?;

        write(
            &root,
            "worker/celery_app.py",
            r#"from celery import Celery

app = Celery("worker", broker="redis://localhost:6379/0", include=["worker.tasks"])

# Protocol 1 carries the task name inside the message body; protocol 2 moves it
# into the AMQP headers. The queue file checked in under var/broker/ is a
# protocol 1 message, so its body is base64 and the task name is not legible to
# anything reading the repository as text.
app.conf.task_protocol = 1
"#,
        )?;

        // THE LIVE ARTIFACT. Registered so the worker can resolve it by name,
        // and called from nowhere: the enqueue site was deleted with the
        // feature. Registration-by-instantiation is exactly the shape a "no
        // callers" analysis reads as dead.
        write(
            &root,
            "worker/tasks.py",
            r#"from .celery_app import app


class RebuildInvoiceIndex(app.Task):
    name = "worker.tasks.RebuildInvoiceIndex"

    def run(self, tenant_slug):
        return f"reindexed {tenant_slug}"


app.register_task(RebuildInvoiceIndex())
"#,
        )?;

        // The green suite. It exercises the app and never the task, which is
        // what makes "green build, green tests, green deploy" true here.
        write(
            &root,
            "tests/test_worker.py",
            r#"from worker.celery_app import app


def test_app_is_configured():
    assert app.main == "worker"
    assert app.conf.task_protocol == 1
"#,
        )?;

        // The broker, stood in for by a file. One undelivered job.
        write(
            &root,
            "var/broker/celery-default.jsonl",
            &format!(
                "{{\"body\": \"{}\", \"content-encoding\": \"utf-8\", \
                 \"content-type\": \"application/json\", \"headers\": {{}}, \
                 \"properties\": {{\"body_encoding\": \"base64\", \
                 \"correlation_id\": \"0f6a2c1e-8f4d-4b17-9a0e-2f5b7c3d1a44\", \
                 \"delivery_tag\": \"7c1d9e2a-5b30-4f8c-8d61-3ae4f0b9c722\", \
                 \"delivery_info\": {{\"exchange\": \"\", \"routing_key\": \"celery\"}}}}}}\n",
                base64_encode(ENQUEUED_JOB.as_bytes())
            ),
        )?;

        // THE DECOY. Nothing imports it and no payload names it.
        write(
            &root,
            "worker/textwrap_helper.py",
            "def hang_indent(text, width=72):\n    return text\n",
        )?;

        repo.add_all()?;
        repo.commit("m15: retired task with one job still in the queue")?;

        Ok(GroundTruth {
            live_paths: vec!["worker/tasks.py".into()],
            live_symbols: vec!["RebuildInvoiceIndex".to_string()],
            decoy_dead_paths: vec!["worker/textwrap_helper.py".into()],
        })
    }
}

/// Standard base64, because Kombu writes a protocol 1 body that way and no
/// base64 crate is available to this crate.
///
/// Hand-rolling an encoder is the smaller evil: the alternative is a fixture
/// whose "encoded" payload no real worker could decode, which would make the
/// mutant assert a liveness that does not exist. The unit test decodes it with
/// a separate implementation for exactly that reason.
fn base64_encode(input: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let bytes: [u32; 3] = [
            chunk.first().copied().unwrap_or(0) as u32,
            chunk.get(1).copied().unwrap_or(0) as u32,
            chunk.get(2).copied().unwrap_or(0) as u32,
        ];
        let triple = (bytes[0] << 16) | (bytes[1] << 8) | bytes[2];
        out.push(ALPHABET[(triple >> 18) as usize & 0x3f] as char);
        out.push(ALPHABET[(triple >> 12) as usize & 0x3f] as char);
        out.push(match chunk.len() {
            1 => '=',
            _ => ALPHABET[(triple >> 6) as usize & 0x3f] as char,
        });
        out.push(match chunk.len() {
            3 => ALPHABET[triple as usize & 0x3f] as char,
            _ => '=',
        });
    }
    out
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
        let truth = EnqueuedJobPayload
            .materialize(dir.path())
            .expect("m15 materializes");
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

    /// Independent of the encoder under test on purpose: if both directions
    /// shared code, a broken alphabet would round-trip and the test would
    /// certify a payload no Celery worker could read.
    fn base64_decode(input: &str) -> Vec<u8> {
        const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut bits = 0u32;
        let mut held = 0u32;
        let mut out = Vec::new();
        for byte in input.bytes().filter(|b| *b != b'=') {
            let value = ALPHABET
                .iter()
                .position(|candidate| *candidate == byte)
                .expect("queue payload is standard base64") as u32;
            bits = (bits << 6) | value;
            held += 6;
            if held >= 8 {
                held -= 8;
                out.push((bits >> held) as u8);
            }
        }
        out
    }

    fn envelope_body(line: &str) -> &str {
        const KEY: &str = "\"body\": \"";
        let start = line.find(KEY).expect("kombu envelope has a body") + KEY.len();
        let rest = &line[start..];
        let end = rest.find('"').expect("the body is a quoted string");
        &rest[..end]
    }

    #[test]
    fn materializes_a_real_git_repo_with_one_commit() {
        let (_dir, repo, _truth) = materialize();
        assert!(
            repo.root().join(".git").is_dir(),
            "expected a git directory"
        );
        assert!(
            repo.is_tracked(Path::new("var/broker/celery-default.jsonl"))
                .expect("query the index"),
            "the queue file stands in for the broker and must be committed"
        );
    }

    #[test]
    fn ground_truth_paths_all_exist_on_disk() {
        let (_dir, repo, truth) = materialize();
        assert!(
            !truth.live_paths.is_empty(),
            "m15's live artifact is a file"
        );
        assert!(
            !truth.decoy_dead_paths.is_empty(),
            "without a decoy, a tool that claims nothing passes m15 for free"
        );
        for path in truth.live_paths.iter().chain(&truth.decoy_dead_paths) {
            assert!(path.is_relative(), "{path:?} must be repo-relative");
            assert!(repo.root().join(path).is_file(), "{path:?} is missing");
        }
    }

    /// The whole point of §6.24: the payload names the class, and no textual
    /// scan of the repository — including of the committed queue file — can
    /// find it, because Celery's protocol 1 body is base64.
    #[test]
    fn the_class_name_survives_only_inside_the_encoded_payload() {
        let (_dir, repo, truth) = materialize();
        let symbol = truth
            .live_symbols
            .first()
            .expect("m15 declares the worker class as a live symbol");

        let naming: Vec<String> = tree(repo.root())
            .into_iter()
            .filter(|(_, bytes)| mentions(bytes, symbol))
            .map(|(path, _)| path)
            .collect();
        assert_eq!(
            naming,
            vec!["worker/tasks.py".to_string()],
            "{symbol} must be greppable only where it is defined"
        );

        let queue = std::fs::read_to_string(repo.root().join("var/broker/celery-default.jsonl"))
            .expect("read the queue file");
        let line = queue.lines().next().expect("at least one enqueued job");
        let decoded = base64_decode(envelope_body(line));
        assert!(
            mentions(&decoded, symbol),
            "the enqueued payload must actually name {symbol}, or the mutant \
             asserts a liveness that does not exist"
        );
    }

    /// A job framework is present, which is §6.24's implementable
    /// counter-signal ("detect a job framework ... → ineligible above
    /// report-only"). Without it the mutant would be unfair rather than hard.
    #[test]
    fn the_repository_admits_it_runs_a_job_framework() {
        let (_dir, repo, _truth) = materialize();
        let manifest = std::fs::read(repo.root().join("pyproject.toml")).expect("read pyproject");
        assert!(
            mentions(&manifest, "celery"),
            "§6.24's counter-signal is framework detection; declare the dependency"
        );
    }

    /// "Deleting it does not break the build, does not break any test, and does
    /// not break the deploy" (§6.24). That is only true if nothing enqueues it.
    #[test]
    fn nothing_in_the_repository_enqueues_the_task() {
        let (_dir, repo, _truth) = materialize();
        for (path, bytes) in tree(repo.root()) {
            for enqueue in [".delay(", "apply_async", "send_task"] {
                assert!(
                    !mentions(&bytes, enqueue),
                    "{path} still enqueues via {enqueue}; m15 requires the last \
                     call site to be gone"
                );
            }
        }
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
