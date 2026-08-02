//! Gate 3f against the shapes §6.24 names (§9.3).
//!
//! Two properties carry this file, and the second is the one that makes the
//! first worth having:
//!
//! 1. **Each condition fires on the marker §6.24 lists for it**, with evidence a
//!    reader can check — the file, the line, the literal.
//! 2. **It is not a constant function.** A gate that refuses everything measures
//!    exactly as much as one that refuses nothing, and 3f is absorbing — no ban
//!    count overrides it — so a refuse-everything bug would be invisible in
//!    every downstream number while silently disabling the whole pipeline. Every
//!    test that asserts a refusal is paired with something the same gate leaves
//!    alone.

use std::path::{Path, PathBuf};

use judged_core::gate3f::{Condition, Gate3f};

struct Repo {
    _dir: tempfile::TempDir,
    root: PathBuf,
}

impl Repo {
    fn of(files: &[(&str, &str)]) -> Repo {
        let dir = tempfile::Builder::new()
            .prefix("judged-gate3f-")
            .tempdir()
            .expect("scratch");
        for (name, body) in files {
            let path = dir.path().join(name);
            std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
            std::fs::write(&path, body).expect("write");
        }
        let root = dir.path().to_path_buf();
        Repo { _dir: dir, root }
    }

    fn gate(&self) -> Gate3f {
        Gate3f::build(&self.root).expect("gate builds")
    }
}

fn conditions(verdict: &judged_core::gate3f::Gate3fVerdict) -> Vec<Condition> {
    let mut out: Vec<Condition> = verdict.findings().iter().map(|f| f.condition).collect();
    out.dedup();
    out
}

/// §6.24: *"the class definition is the schema for data already written to
/// disk."* Both halves — the marker fires, and a neighbouring plain module does
/// not.
#[test]
fn serialization_markers_refuse_and_a_plain_module_is_left_alone() {
    let repo = Repo::of(&[
        (
            "pricing/legacy_rates.py",
            "class RateSnapshot:\n    def __getstate__(self):\n        return self.__dict__\n",
        ),
        ("pricing/plain.py", "def add(a, b):\n    return a + b\n"),
        (
            "src/wire.rs",
            "#[derive(Debug, Clone, Serialize, Deserialize)]\npub struct Envelope;\n",
        ),
    ]);
    let gate = repo.gate();

    let pickled = gate
        .judge_symbol("RateSnapshot", Some(Path::new("pricing/legacy_rates.py")))
        .expect("judges");
    assert_eq!(conditions(&pickled), vec![Condition::Serializable]);
    assert!(pickled.findings()[0].detail.contains("schema for data"));

    // The serde form that actually occurs. A literal `#[derive(Serialize` would
    // miss this, which is the whole reason the rule is two steps.
    let serde = gate
        .judge_symbol("Envelope", Some(Path::new("src/wire.rs")))
        .expect("judges");
    assert_eq!(conditions(&serde), vec![Condition::Serializable]);

    let plain = gate
        .judge_symbol("add", Some(Path::new("pricing/plain.py")))
        .expect("judges");
    assert!(
        !plain.is_ineligible(),
        "a module with none of §6.24's markers is not refused — the gate has to be \
         able to say nothing"
    );
}

/// §6.24: *"already-linked consumers that were never rebuilt."* Judged by
/// proximity, because the condition is about the symbol and not the file.
#[test]
fn an_abi_export_refuses_its_own_symbol_and_not_its_neighbour() {
    let repo = Repo::of(&[(
        "src/ffi.rs",
        "#[no_mangle]\npub extern \"C\" fn ledger_amortize(x: i64) -> i64 {\n    x\n}\n\
         \nfn helper_not_exported() {}\n",
    )]);
    let gate = repo.gate();

    let exported = gate
        .judge_symbol("ledger_amortize", Some(Path::new("src/ffi.rs")))
        .expect("judges");
    assert!(exported
        .findings()
        .iter()
        .any(|f| f.condition == Condition::AbiExport));

    let neighbour = gate
        .judge_symbol("helper_not_exported", Some(Path::new("src/ffi.rs")))
        .expect("judges");
    assert!(
        !neighbour.is_ineligible(),
        "one #[no_mangle] must not refuse every other symbol in the module — that is \
         how an absorbing gate becomes a constant function"
    );
}

/// Go's `//export` names the symbol on the marker line itself.
#[test]
fn a_go_export_directive_refuses_the_symbol_it_names() {
    let repo = Repo::of(&[(
        "cmd/libtelemetry/abi.go",
        "package main\n\n//export TelemetryFlush\nfunc TelemetryFlush() {}\n\n\
         func internalOnly() {}\n",
    )]);
    let gate = repo.gate();

    assert!(gate
        .judge_symbol("TelemetryFlush", Some(Path::new("cmd/libtelemetry/abi.go")))
        .expect("judges")
        .is_ineligible());
    assert!(!gate
        .judge_symbol("internalOnly", Some(Path::new("cmd/libtelemetry/abi.go")))
        .expect("judges")
        .is_ineligible());
}

/// A claim that a whole file is dead is a claim that everything in it is dead,
/// so any export anywhere in it refuses the claim.
#[test]
fn a_path_claim_is_refused_by_any_export_in_the_file() {
    let repo = Repo::of(&[(
        "src/ffi.rs",
        "fn a() {}\nfn b() {}\n#[no_mangle]\npub extern \"C\" fn shipped() {}\n",
    )]);

    assert!(repo
        .gate()
        .judge_path(Path::new("src/ffi.rs"))
        .expect("judges")
        .is_ineligible());
}

/// §6.24 lists `rq` and `bull` among the job frameworks. A substring search for
/// those matches most of an English dictionary, and a queue condition that fires
/// on every file would refuse every symbol in the repository.
#[test]
fn short_framework_names_do_not_fire_on_ordinary_words() {
    let repo = Repo::of(&[
        (
            "docs/notes.md",
            "The torque curve and the bulletin board are unrelated to any queue.\n",
        ),
        ("app/service.py", "def torque_bulletin():\n    return 1\n"),
    ]);
    let gate = repo.gate();

    assert!(
        gate.frameworks().is_empty(),
        "no framework is declared here, so none may be detected: {:?}",
        gate.frameworks()
    );
    assert!(!gate
        .judge_symbol("torque_bulletin", Some(Path::new("app/service.py")))
        .expect("judges")
        .is_ineligible());
}

/// The queue condition, both halves: a declared framework plus a file that binds
/// to it, and a file in the same repository that does not.
#[test]
fn a_declared_job_framework_refuses_only_the_files_that_bind_to_it() {
    let repo = Repo::of(&[
        ("requirements.txt", "celery==5.3.0\nrequests\n"),
        (
            "worker/tasks.py",
            "from celery import Task\n\nclass RebuildInvoiceIndex(Task):\n    def run(self):\n        pass\n",
        ),
        ("util/text.py", "def wrap(s):\n    return s.strip()\n"),
    ]);
    let gate = repo.gate();

    assert_eq!(gate.frameworks().len(), 1);
    assert_eq!(gate.frameworks()[0].name, "celery");
    assert_eq!(
        gate.frameworks()[0].found_in,
        PathBuf::from("requirements.txt")
    );

    let worker = gate
        .judge_symbol("RebuildInvoiceIndex", Some(Path::new("worker/tasks.py")))
        .expect("judges");
    assert!(worker
        .findings()
        .iter()
        .any(|f| f.condition == Condition::QueuePayload));
    assert!(worker.findings()[0].detail.contains("poison-pill"));

    let unrelated = gate
        .judge_symbol("wrap", Some(Path::new("util/text.py")))
        .expect("judges");
    assert!(
        !unrelated.is_ineligible(),
        "a repository containing Celery does not make every symbol in it a job — \
         that reading would refuse everything and measure nothing"
    );
}

/// A framework named only in prose is not a declaration.
#[test]
fn a_framework_mentioned_outside_a_declaration_context_is_not_detected() {
    let repo = Repo::of(&[(
        "README.md",
        "We used to run sidekiq here, but the workers were retired in 2019.\n",
    )]);

    assert!(repo.gate().frameworks().is_empty());
}

/// All of them, not the first. A Celery task that is also serde-derived is
/// refused twice, for two different reasons.
#[test]
fn every_condition_that_fires_is_reported() {
    let repo = Repo::of(&[
        ("Gemfile", "gem 'sidekiq'\n"),
        (
            "app/jobs/backfill.rb",
            "require 'sidekiq'\n#[derive(Serialize)]\nclass Backfill\n  serialVersionUID = 1\nend\n",
        ),
    ]);

    let verdict = repo
        .gate()
        .judge_symbol("Backfill", Some(Path::new("app/jobs/backfill.rb")))
        .expect("judges");

    let fired = conditions(&verdict);
    assert!(fired.contains(&Condition::Serializable));
    assert!(fired.contains(&Condition::QueuePayload));
}

/// With no declaring file there is nothing to read, and an empty verdict is the
/// honest answer rather than a clearance.
#[test]
fn an_unattributed_symbol_yields_an_empty_verdict_which_is_not_a_clearance() {
    let repo = Repo::of(&[(
        "src/ffi.rs",
        "#[no_mangle]\npub extern \"C\" fn shipped() {}\n",
    )]);

    let verdict = repo.gate().judge_symbol("shipped", None).expect("judges");
    assert!(verdict.findings().is_empty());
    assert!(!verdict.is_ineligible());
}

/// Evidence a reader can act on: which condition, which literal, which file and
/// line, and what deleting the candidate would break.
#[test]
fn a_finding_carries_the_evidence_and_the_consequence() {
    let repo = Repo::of(&[(
        "src/ffi.rs",
        "// a comment\n#[no_mangle]\npub extern \"C\" fn shipped() {}\n",
    )]);

    let verdict = repo
        .gate()
        .judge_symbol("shipped", Some(Path::new("src/ffi.rs")))
        .expect("judges");
    let finding = verdict
        .findings()
        .iter()
        .find(|f| f.condition == Condition::AbiExport)
        .expect("the export fired");

    assert_eq!(finding.found_in, PathBuf::from("src/ffi.rs"));
    assert_eq!(finding.line, 2, "the marker's own line, 1-based");
    assert!(finding.marker.contains("no_mangle"));
    assert!(finding.detail.contains("already-linked consumers"));
}
