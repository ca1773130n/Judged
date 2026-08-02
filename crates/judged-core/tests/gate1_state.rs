//! Behavioural tests for Gate 1 classes 1a–1f — the state whose destruction
//! reaches outside the repository.
//!
//! Every tree here is built on disk, because every hazard in these six classes
//! is a property of the filesystem rather than of our beliefs about it: a
//! broken symlink that is git-annex's *normal steady state*, a SQLite database
//! wearing a `.tmp` extension, a 130-byte pointer standing in for a 4 GB model,
//! a directory the process cannot read. None of those can be faked by a stub
//! without first assuming the answer the gate exists to find.
//!
//! Four tests are the point of the file:
//!
//! - [`terraform_marker_makes_an_unrelated_file_ineligible`] — the §6.10 shape.
//!   Nothing references the file, and it is still ineligible, tree-wide.
//! - [`an_unreadable_directory_is_a_hit_not_an_absence`] — a scan that did not
//!   finish has not proved there is no effector.
//! - [`a_broken_annex_symlink_is_steady_state_not_garbage`] — the rule every
//!   naive cleaner implements ("report dangling symlinks") deletes the pointer
//!   to every un-fetched annexed file.
//! - [`magic_bytes_cannot_see_plain_text_irreplaceables`] — the documented
//!   blind spot in §2.1's sniff, executed rather than asserted in prose. It is
//!   why 1b and 1c exist as separate name-driven classes instead of being
//!   folded into 1d.

use std::fs;
use std::path::{Path, PathBuf};

use judged_core::gate1::state::{
    sniff, DataStore, Ecosystem, Evidence, StateClass, StateFinding, StateGate, StateVerdict,
    HEAD_BYTES,
};
use judged_core::git::Repo;

// ---------------------------------------------------------------------------
// scaffolding
// ---------------------------------------------------------------------------

/// A temporary tree whose root has been canonicalized once.
///
/// macOS hands out temp roots under `/var/folders/…`, a symlink to
/// `/private/var/folders/…`. Canonicalizing here keeps fixture paths and the
/// paths the gate reports comparable on the platform these tests run on.
struct Tree {
    /// Held for its `Drop`: removing the tree is tempfile's job.
    _guard: tempfile::TempDir,
    root: PathBuf,
}

impl Tree {
    fn new(label: &str) -> Tree {
        let guard = tempfile::Builder::new()
            .prefix(&format!("judged-gate1-state-{label}-"))
            .tempdir()
            .expect("create temp dir");
        let root = fs::canonicalize(guard.path()).expect("canonicalize temp dir");
        Tree {
            _guard: guard,
            root,
        }
    }

    fn root(&self) -> &Path {
        &self.root
    }

    /// Write `contents` to `rel`, creating parent directories.
    fn file(&self, rel: &str, contents: &[u8]) -> PathBuf {
        let path = self.root.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parents");
        }
        fs::write(&path, contents).expect("write fixture file");
        path
    }

    fn dir(&self, rel: &str) -> PathBuf {
        let path = self.root.join(rel);
        fs::create_dir_all(&path).expect("create fixture dir");
        path
    }

    #[cfg(unix)]
    fn symlink(&self, rel: &str, target: &str) -> PathBuf {
        let path = self.root.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parents");
        }
        std::os::unix::fs::symlink(target, &path).expect("create symlink");
        path
    }
}

/// The class codes a verdict reports, in the order it reports them.
fn codes(verdict: &StateVerdict) -> Vec<&'static str> {
    verdict.findings().iter().map(|f| f.class.code()).collect()
}

/// The one finding of class `code`, or a failure naming what was found instead.
fn finding<'a>(verdict: &'a StateVerdict, code: &str) -> &'a StateFinding {
    verdict
        .findings()
        .iter()
        .find(|f| f.class.code() == code)
        .unwrap_or_else(|| panic!("expected a {code} finding, got {:?}", codes(verdict)))
}

/// Assert `haystack` contains `needle`, printing the whole text when it does not.
fn assert_says(haystack: &str, needle: &str) {
    assert!(
        haystack.contains(needle),
        "expected the explanation to contain {needle:?}\n--- actual ---\n{haystack}\n---"
    );
}

/// A tree with no effector, no store and no interesting file — the control.
fn plain_tree(label: &str) -> Tree {
    let tree = Tree::new(label);
    tree.file("src/main.rs", b"fn main() {}\n");
    tree.file("README.md", b"# a repository\n");
    tree
}

// ---------------------------------------------------------------------------
// 1a — external effectors (§6.10)
// ---------------------------------------------------------------------------

#[test]
fn terraform_marker_makes_an_unrelated_file_ineligible() {
    // The §6.10 shape: a file nothing references, in a tree that happens to
    // provision infrastructure.
    let tree = Tree::new("tf-tree");
    tree.file(
        "infra/main.tf",
        b"resource \"aws_db_instance\" \"main\" {\n  lifecycle { prevent_destroy = true }\n}\n",
    );
    let orphan = tree.file(
        "k8s/prod/postgres-pvc.yaml",
        b"kind: PersistentVolumeClaim\n",
    );

    let gate = StateGate::survey(tree.root());
    assert!(
        gate.has_external_effector(),
        "markers: {:?}",
        gate.effectors()
    );

    let verdict = gate.judge(&orphan);
    assert!(verdict.is_ineligible());
    let f = finding(&verdict, "1a");
    assert_eq!(f.class, StateClass::ExternalEffector);
    let why = f.why();
    assert_says(&why, "infra/main.tf");
    // The whole reason this class outranks every other signal, quoted:
    assert_says(
        &why,
        "doesn't prevent Terraform from destroying the resource if you remove the resource \
         configuration",
    );
    assert_says(&why, "report-only");
}

#[test]
fn argocd_is_detected_by_its_api_group_not_by_a_filename() {
    // An ArgoCD directory-recursive Application has no manifest list at all,
    // and the marker file's name carries no signal — only its API group does.
    let tree = Tree::new("argo");
    tree.file(
        "clusters/prod/whatever.yaml",
        b"apiVersion: argoproj.io/v1alpha1\nkind: Application\nspec:\n  syncPolicy:\n    automated:\n      prune: true\n",
    );
    let orphan = tree.file(
        "clusters/prod/postgres-pvc.yaml",
        b"kind: PersistentVolumeClaim\n",
    );

    let gate = StateGate::survey(tree.root());
    let verdict = gate.judge(&orphan);
    let f = finding(&verdict, "1a");
    match &f.evidence {
        Evidence::TreeMarker { system, marker, .. } => {
            assert_eq!(*system, Ecosystem::ArgoCd);
            assert!(
                marker.ends_with("clusters/prod/whatever.yaml"),
                "{marker:?}"
            );
        }
        other => panic!("expected a TreeMarker, got {other:?}"),
    }
    assert_says(&f.why(), "prune");
}

#[test]
fn every_effector_ecosystem_in_6_10_is_detected() {
    // §6.10 names nine. Each gets its own tree so one marker cannot mask another.
    let cases: &[(&str, &str, &[u8], Ecosystem)] = &[
        (
            "tf",
            "main.tf",
            b"resource \"null_resource\" \"x\" {}\n",
            Ecosystem::Terraform,
        ),
        (
            "tflock",
            ".terraform.lock.hcl",
            b"provider \"registry.terraform.io/hashicorp/aws\" {}\n",
            Ecosystem::Terraform,
        ),
        (
            "pulumi",
            "Pulumi.yaml",
            b"name: infra\nruntime: nodejs\n",
            Ecosystem::Pulumi,
        ),
        (
            "cdk",
            "cdk.json",
            b"{\"app\": \"npx ts-node bin/app.ts\"}\n",
            Ecosystem::Cdk,
        ),
        (
            "helm",
            "chart/Chart.yaml",
            b"apiVersion: v2\nname: api\nversion: 0.1.0\n",
            Ecosystem::Helm,
        ),
        (
            "kustomize",
            "overlays/prod/kustomization.yaml",
            b"resources:\n  - ../../base\n",
            Ecosystem::Kustomize,
        ),
        (
            "ansible-cfg",
            "ansible.cfg",
            b"[defaults]\ninventory = hosts\n",
            Ecosystem::Ansible,
        ),
        (
            "ansible-role",
            "roles/db/tasks/main.yml",
            b"- name: install\n  ansible.builtin.apt:\n    name: postgresql\n",
            Ecosystem::Ansible,
        ),
        (
            "flux",
            "clusters/prod/gitrepo.yaml",
            b"apiVersion: source.toolkit.fluxcd.io/v1\nkind: GitRepository\n",
            Ecosystem::Flux,
        ),
        (
            "crossplane",
            "xrd.yaml",
            b"apiVersion: apiextensions.crossplane.io/v1\nkind: Composition\n",
            Ecosystem::Crossplane,
        ),
    ];

    for (label, rel, contents, expected) in cases {
        let tree = Tree::new(label);
        tree.file(rel, contents);
        let unrelated = tree.file("notes/scratch.md", b"nothing\n");

        let gate = StateGate::survey(tree.root());
        let found: Vec<Ecosystem> = gate.effectors().iter().map(|m| m.system).collect();
        assert!(
            found.contains(expected),
            "{rel}: expected {expected:?} among {found:?}"
        );
        assert!(
            gate.judge(&unrelated).is_ineligible(),
            "{rel}: the whole tree must carry 1a"
        );
    }
}

#[test]
fn a_tree_with_no_effector_marker_does_not_carry_1a() {
    // The class has to be able to *not* fire, or it is a refusal to run rather
    // than a gate.
    let tree = plain_tree("no-effector");
    let gate = StateGate::survey(tree.root());

    assert!(!gate.has_external_effector(), "{:?}", gate.effectors());
    let verdict = gate.judge(&tree.root().join("src/main.rs"));
    assert_eq!(verdict, StateVerdict::Abstain);
    assert!(!verdict.is_ineligible());
    assert!(verdict.findings().is_empty());
}

#[test]
fn an_effector_needle_in_prose_does_not_fire() {
    // The API-group probe reads YAML only. A README that mentions argoproj.io
    // must not put an entire repository into report-only.
    let tree = plain_tree("prose");
    tree.file(
        "docs/deploy.md",
        b"We sync with argoproj.io Applications and fluxcd.io GitRepositories.\n",
    );
    let gate = StateGate::survey(tree.root());
    assert!(!gate.has_external_effector(), "{:?}", gate.effectors());
}

#[cfg(unix)]
#[test]
fn a_directory_symlink_leaving_the_tree_is_a_gap_not_a_silence() {
    // A symlink is the one way content can sit at a repository path without
    // being under the repository root. Not descending it is defensible — it is
    // an unbounded walk, and a link to `/` is a denial of service — but
    // *silently* not descending it is a false negative in the one class where
    // a false negative pages someone.
    let elsewhere = Tree::new("symlink-target");
    elsewhere.file("main.tf", b"resource \"aws_s3_bucket\" \"data\" {}\n");

    let tree = plain_tree("symlink-out");
    tree.symlink("infra", elsewhere.root().to_str().expect("utf-8 temp path"));

    let gate = StateGate::survey(tree.root());
    assert!(
        gate.scan_gaps().iter().any(|gap| gap.contains("infra")),
        "the link out of the tree must be recorded: {:?}",
        gate.scan_gaps()
    );
    assert!(gate.has_external_effector());
    let verdict = gate.judge(&tree.root().join("src/main.rs"));
    assert!(matches!(
        finding(&verdict, "1a").evidence,
        Evidence::ScanIncomplete { .. }
    ));
}

#[cfg(unix)]
#[test]
fn a_directory_symlink_inside_the_tree_is_not_a_gap() {
    // pnpm, yarn workspaces and Bazel output trees are full of these. Their
    // targets are walked directly, so descending them adds nothing — and
    // reporting them as gaps would put most JavaScript repositories into
    // permanent report-only for no evidence at all.
    let tree = plain_tree("symlink-in");
    tree.symlink("mirror", "src");

    let gate = StateGate::survey(tree.root());
    assert!(gate.scan_gaps().is_empty(), "{:?}", gate.scan_gaps());
    assert!(!gate.has_external_effector());
}

#[cfg(unix)]
#[test]
fn a_broken_symlink_does_not_become_a_scan_gap() {
    // The other half of the annex rule. If an un-fetched annexed file counted
    // as a hole in the survey, every git-annex and every DVC repository would
    // be report-only forever, and the tool would be useless exactly where
    // §6.13 says care is most needed.
    let tree = plain_tree("broken-link");
    tree.dir(".git/annex/objects");
    tree.symlink(
        "data/genome.fa",
        "../.git/annex/objects/Wk/9F/SHA256E-s42--deadbeef/genome.fa",
    );

    let gate = StateGate::survey(tree.root());
    assert!(gate.scan_gaps().is_empty(), "{:?}", gate.scan_gaps());
    assert!(!gate.has_external_effector());
}

#[cfg(unix)]
#[test]
fn an_unreadable_directory_is_a_hit_not_an_absence() {
    use std::os::unix::fs::PermissionsExt;

    // §6.20: a search that did not finish has found nothing *because it did not
    // look*. An unreadable subtree could hold every .tf file in the repository.
    let tree = plain_tree("unreadable-dir");
    let locked = tree.dir("infra");
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).expect("chmod");

    let gate = StateGate::survey(tree.root());
    let gaps = gate.scan_gaps().to_vec();
    let verdict = gate.judge(&tree.root().join("src/main.rs"));

    // Restore before any assertion can unwind past the temp dir's cleanup.
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).expect("restore chmod");

    assert!(!gaps.is_empty(), "an unreadable directory must be recorded");
    assert!(
        gate.has_external_effector(),
        "an incomplete survey cannot clear a tree"
    );
    let f = finding(&verdict, "1a");
    match &f.evidence {
        Evidence::ScanIncomplete { detail } => assert!(detail.contains("infra"), "{detail}"),
        other => panic!("expected ScanIncomplete, got {other:?}"),
    }
    assert_says(&f.why(), "never an absence");
}

// ---------------------------------------------------------------------------
// 1b — secrets and identity (§6.22)
// ---------------------------------------------------------------------------

#[test]
fn a_secret_is_escalated_for_rotation_and_never_deleted() {
    let tree = plain_tree("secret");
    let env = tree.file(".env", b"DATABASE_URL=postgres://u:p@h/db\n");

    let gate = StateGate::survey(tree.root());
    let verdict = gate.judge(&env);
    let f = finding(&verdict, "1b");
    assert_eq!(f.class, StateClass::SecretOrIdentity);

    let why = f.why();
    // All three failures §6.22 names have to be in front of the human.
    assert_says(&why, "rotate");
    assert_says(&why, "history");
    assert_says(&why, "audit trail");
    assert_says(StateClass::SecretOrIdentity.remediation(), "rotate");
}

#[test]
fn env_templates_are_not_secrets() {
    // .env.example holds no credential, and calling it one sends a human on a
    // rotation hunt for nothing — which is how a class stops being believed.
    let tree = plain_tree("env-template");
    for name in [".env.example", ".env.sample", ".env.template", ".env.dist"] {
        let path = tree.file(name, b"DATABASE_URL=\n");
        let gate = StateGate::survey(tree.root());
        let verdict = gate.judge(&path);
        assert!(
            !codes(&verdict).contains(&"1b"),
            "{name} must not be classified as a secret: {verdict:?}"
        );
    }
    // …while the real ones are.
    for name in [".env.local", ".env.production", ".env.prod.local"] {
        let path = tree.file(name, b"TOKEN=abc\n");
        let gate = StateGate::survey(tree.root());
        assert!(codes(&gate.judge(&path)).contains(&"1b"), "{name}");
    }
}

#[test]
fn secret_and_identity_filenames_are_caught() {
    let tree = plain_tree("secret-names");
    let names = [
        "id_rsa",
        "id_ed25519",
        "deploy.pem",
        "server.key",
        "bundle.p12",
        "release.jks",
        ".netrc",
        ".npmrc",
        ".pypirc",
        ".git-credentials",
        ".htpasswd",
        "secrets.auto.tfvars",
        "vault.kdbx",
    ];
    for name in names {
        let path = tree.file(name, b"x\n");
        let gate = StateGate::survey(tree.root());
        assert!(
            codes(&gate.judge(&path)).contains(&"1b"),
            "{name} must be 1b, got {:?}",
            codes(&gate.judge(&path))
        );
    }
}

#[test]
fn a_private_key_with_no_extension_is_caught_by_its_pem_header() {
    // §2.1's sniff pays for itself here: the name says nothing.
    let tree = plain_tree("pem");
    let key = tree.file(
        "ops/bastion",
        b"-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAA\n",
    );
    let cert = tree.file("ops/bastion-cert", b"-----BEGIN CERTIFICATE-----\nMIIB\n");

    let gate = StateGate::survey(tree.root());
    assert!(
        codes(&gate.judge(&key)).contains(&"1b"),
        "{:?}",
        gate.judge(&key)
    );
    // A certificate is public by construction. It is somebody else's class
    // (1l, platform contracts) and must not be reported as a credential.
    assert!(
        !codes(&gate.judge(&cert)).contains(&"1b"),
        "a certificate is not a secret: {:?}",
        gate.judge(&cert)
    );
}

#[test]
fn the_longest_pem_label_fits_in_the_head_window() {
    // "-----BEGIN OPENSSH PRIVATE KEY-----" is 35 bytes: a 32-byte window, the
    // figure §2.1 costs the sniff at, would read it as unknown.
    assert!(
        HEAD_BYTES >= b"-----BEGIN OPENSSH PRIVATE KEY-----".len(),
        "HEAD_BYTES = {HEAD_BYTES}"
    );
}

// ---------------------------------------------------------------------------
// 1c — infrastructure state (§6.17)
// ---------------------------------------------------------------------------

#[test]
fn tfstate_names_the_recovery_cost_it_imposes() {
    let tree = plain_tree("tfstate");
    let state = tree.file("terraform.tfstate", b"{\"version\": 4}\n");

    let gate = StateGate::survey(tree.root());
    let verdict = gate.judge(&state);
    let f = finding(&verdict, "1c");
    assert_eq!(f.class, StateClass::InfrastructureState);
    let why = f.why();
    // §6.17: Terraform.gitignore ignores *.tfstate, so git is not the backstop,
    // and recovery is manual, per resource.
    assert_says(&why, "terraform import");
    assert_says(&why, "gitignore");
}

#[test]
fn infrastructure_state_variants_are_caught() {
    let tree = plain_tree("infra-state");
    let names = [
        "terraform.tfstate",
        "terraform.tfstate.backup",
        "terraform.tfstate.1690000000.backup",
        "env/prod.tfstate",
        "prod.tfvars",
        "prod.auto.tfvars.json",
        "Pulumi.prod.yaml",
        ".pulumi/stacks/prod.json",
        "cdk.out/manifest.json",
        ".terraform/terraform.tfstate",
    ];
    for name in names {
        let path = tree.file(name, b"{}\n");
        let gate = StateGate::survey(tree.root());
        assert!(
            codes(&gate.judge(&path)).contains(&"1c"),
            "{name} must be 1c, got {:?}",
            codes(&gate.judge(&path))
        );
    }
}

#[test]
fn an_ansible_vault_is_caught_by_its_header_whatever_it_is_called() {
    let tree = plain_tree("vault");
    let vault = tree.file(
        "group_vars/all",
        b"$ANSIBLE_VAULT;1.1;AES256\n33633161626...\n",
    );
    let gate = StateGate::survey(tree.root());
    let verdict = gate.judge(&vault);
    assert!(codes(&verdict).contains(&"1c"), "{verdict:?}");
    match &finding(&verdict, "1c").evidence {
        Evidence::Magic { label, .. } => assert!(label.contains("Ansible"), "{label}"),
        other => panic!("expected Magic, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// 1d — local databases and persistence (§2.1)
// ---------------------------------------------------------------------------

#[test]
fn a_database_wearing_a_disposable_extension_is_caught_by_magic_bytes() {
    // Magic bytes first, extensions second: the name here is a lie in the most
    // dangerous direction.
    let tree = plain_tree("sqlite-tmp");
    let db = tree.file(
        "var/cache.tmp",
        b"SQLite format 3\0\x10\x00\x01\x01\x00@  \x00\x00\x00\x01",
    );

    let gate = StateGate::survey(tree.root());
    let verdict = gate.judge(&db);
    let f = finding(&verdict, "1d");
    assert_eq!(f.class, StateClass::LocalPersistence);
    match &f.evidence {
        Evidence::Magic { label, offset } => {
            assert!(label.contains("SQLite"), "{label}");
            assert_eq!(*offset, 0);
        }
        other => panic!("expected Magic, got {other:?}"),
    }
}

#[test]
fn known_database_magics_are_recognised_without_touching_the_disk() {
    let cases: &[(&[u8], &str)] = &[
        (b"SQLite format 3\0", "SQLite"),
        (b"REDIS0011\xfa\x09redis-ver", "Redis"),
        (
            b"\x00\x00\x00\x00\x00\x00\x00\x00DUCK\x00\x00\x00\x00",
            "DuckDB",
        ),
        (
            b"\x00\x01\x00\x00Standard Jet DB\x00\x01\x00\x00\x00",
            "Access",
        ),
    ];
    for (head, label) in cases {
        let hit = sniff(head).unwrap_or_else(|| panic!("no magic for {label}"));
        assert_eq!(hit.class, StateClass::LocalPersistence, "{label}");
        assert!(hit.label.contains(label), "{} vs {label}", hit.label);
    }
}

#[test]
fn an_empty_database_is_still_caught_by_its_extension() {
    // A freshly created, zero-byte store has no magic to read. The extension
    // list is the second line, and it has to hold on its own.
    let tree = plain_tree("empty-db");
    for name in [
        "app.sqlite3",
        "app.db",
        "sessions.mdb",
        "store.realm",
        "dump.rdb",
    ] {
        let path = tree.file(name, b"");
        let gate = StateGate::survey(tree.root());
        assert!(
            codes(&gate.judge(&path)).contains(&"1d"),
            "{name}: {:?}",
            gate.judge(&path)
        );
    }
}

#[test]
fn magic_bytes_cannot_see_plain_text_irreplaceables() {
    // The sniff's documented blind spot, executed. Every one of these is
    // unrecoverable, and none of them has a signature to find — which is why
    // 1b and 1c are separate, name-driven classes rather than folded into 1d.
    let blind: &[(&str, &[u8])] = &[
        (".env", b"DATABASE_URL=postgres://u:p@h/db\n"),
        ("terraform.tfstate", b"{\"version\": 4, \"serial\": 17}\n"),
        (".npmrc", b"//registry.npmjs.org/:_authToken=npm_x\n"),
        (".Rhistory", b"library(dplyr)\nsummary(fit)\n"),
        ("customers.csv", b"id,email\n1,a@b.c\n"),
    ];
    for (name, head) in blind {
        assert!(
            sniff(head).is_none(),
            "{name} must be invisible to the magic sniff, got {:?}",
            sniff(head)
        );
    }

    // …and the ones 1a–1f own are still refused, by name.
    let tree = plain_tree("blind-spot");
    for (name, head) in blind {
        if *name == ".Rhistory" {
            continue;
        }
        let path = tree.file(name, head);
        let gate = StateGate::survey(tree.root());
        assert!(
            gate.judge(&path).is_ineligible(),
            "{name} must still be refused by a name-driven class"
        );
    }

    // .Rhistory is the exception, and pinning it here is the point rather than
    // an omission: it is §9.3 class **1h**, session and scratch state, which
    // this module does not own. It is blind to the sniff *and* invisible to
    // 1a–1f, so it is refused only if 1h is implemented — and this assertion
    // fails loudly if some later edit quietly annexes 1h into this file, or if
    // anyone reads Abstain here as "safe to delete".
    let rhistory = tree.file(".Rhistory", b"library(dplyr)\nsummary(fit)\n");
    assert_eq!(
        StateGate::survey(tree.root()).judge(&rhistory),
        StateVerdict::Abstain,
        ".Rhistory belongs to 1h; 1a-1f must neither claim it nor clear it"
    );
}

#[cfg(unix)]
#[test]
fn a_file_whose_head_cannot_be_read_is_refused() {
    use std::os::unix::fs::PermissionsExt;

    let tree = plain_tree("unreadable-file");
    let path = tree.file("notes.txt", b"plain\n");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).expect("chmod");

    let gate = StateGate::survey(tree.root());
    let verdict = gate.judge(&path);

    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).expect("restore chmod");

    assert!(verdict.is_ineligible(), "{verdict:?}");
    let f = finding(&verdict, "1d");
    match &f.evidence {
        Evidence::Unreadable { detail } => assert!(!detail.is_empty()),
        other => panic!("expected Unreadable, got {other:?}"),
    }
    assert_says(&f.why(), "never an absence");
}

// ---------------------------------------------------------------------------
// 1e — models, weights, checkpoints (§9.13)
// ---------------------------------------------------------------------------

#[test]
fn a_checkpoint_is_refused_and_its_size_is_named_as_the_trap() {
    let tree = plain_tree("weights");
    let ckpt = tree.file(
        "models/epoch_12.safetensors",
        b"\x40\x00\x00\x00\x00\x00\x00\x00{}",
    );

    let gate = StateGate::survey(tree.root());
    let verdict = gate.judge(&ckpt);
    let f = finding(&verdict, "1e");
    assert_eq!(f.class, StateClass::ModelOrCheckpoint);
    // §9.13: never sort by bytes reclaimed. The largest object on the machine
    // is the one a size-ranked report puts at the top.
    assert_says(&f.why(), "bytes reclaimed");
}

#[test]
fn model_and_checkpoint_formats_are_caught() {
    let tree = plain_tree("model-exts");
    for name in [
        "m.pt",
        "m.pth",
        "m.onnx",
        "m.ckpt",
        "m.safetensors",
        "m.tflite",
        "m.gguf",
        "m.joblib",
        "m.mlmodel",
        "m.caffemodel",
        "pytorch_model.bin",
    ] {
        let path = tree.file(name, b"x");
        let gate = StateGate::survey(tree.root());
        assert!(
            codes(&gate.judge(&path)).contains(&"1e"),
            "{name}: {:?}",
            gate.judge(&path)
        );
    }
    // Magic, for the formats that have one and the names that do not say so.
    for (head, label) in [
        (&b"GGUF\x03\x00\x00\x00"[..], "GGUF"),
        (&b"\x89HDF\r\n\x1a\n"[..], "HDF5"),
    ] {
        let hit = sniff(head).unwrap_or_else(|| panic!("no magic for {label}"));
        assert_eq!(hit.class, StateClass::ModelOrCheckpoint, "{label}");
    }
}

// ---------------------------------------------------------------------------
// 1f — downloaded / acquired data (§6.13)
// ---------------------------------------------------------------------------

#[cfg(unix)]
#[test]
fn a_broken_annex_symlink_is_steady_state_not_garbage() {
    // git-annex, verbatim: after a drop "the file will still appear in your
    // work tree as a broken symlink". A "report dangling symlinks" rule — which
    // czkawka, rmlint and every naive Gate 0 implement — deletes the pointer to
    // every un-fetched annexed file.
    let tree = Tree::new("annex");
    tree.dir(".git/annex/objects");
    let link = tree.symlink(
        "data/genome.fa",
        "../.git/annex/objects/Wk/9F/SHA256E-s42--deadbeef/genome.fa",
    );
    assert!(
        fs::metadata(&link).is_err(),
        "the fixture must be a *broken* symlink, which is the normal state"
    );

    let gate = StateGate::survey(tree.root());
    let verdict = gate.judge(&link);
    let f = finding(&verdict, "1f");
    assert_eq!(f.class, StateClass::AcquiredData);
    match &f.evidence {
        Evidence::ContentPointer { store, .. } => assert!(store.contains("annex"), "{store}"),
        other => panic!("expected ContentPointer, got {other:?}"),
    }
    let why = f.why();
    assert_says(&why, "git annex get");
    assert_says(&why, "broken symlink");
}

#[test]
fn the_dvc_cache_holds_the_only_copy_and_its_config_holds_the_credentials() {
    let tree = Tree::new("dvc");
    tree.file(
        "dvc.yaml",
        b"stages:\n  prepare:\n    cmd: python prep.py\n",
    );
    let cached = tree.file(".dvc/cache/ab/cdef0123456789", b"\x00\x01binary payload");
    let local = tree.file(
        ".dvc/config.local",
        b"['remote \"s3\"']\n    secret_access_key = AKIA\n",
    );
    let pointer = tree.file(
        "data/raw.csv.dvc",
        b"outs:\n- md5: abcdef\n  path: raw.csv\n",
    );

    let gate = StateGate::survey(tree.root());

    let cache_verdict = gate.judge(&cached);
    assert!(codes(&cache_verdict).contains(&"1f"), "{cache_verdict:?}");
    assert_says(&finding(&cache_verdict, "1f").why(), "only copy");

    // config.local is two hazards at once, and a score would hide one of them.
    let local_verdict = gate.judge(&local);
    assert_eq!(codes(&local_verdict), vec!["1b", "1f"], "{local_verdict:?}");

    assert!(codes(&gate.judge(&pointer)).contains(&"1f"));
}

#[test]
fn an_lfs_pointer_is_130_bytes_and_still_ineligible() {
    // Size-based scanners see nothing here, and the content may exist on no
    // remote. Two classes fire: the extension says model, the content says
    // pointer. §9.13 wants both, as a list.
    let tree = plain_tree("lfs");
    tree.file(
        ".gitattributes",
        b"*.pt filter=lfs diff=lfs merge=lfs -text\n",
    );
    let pointer = tree.file(
        "models/resnet.pt",
        b"version https://git-lfs.github.com/spec/v1\noid sha256:4d7a\nsize 4194304\n",
    );

    let gate = StateGate::survey(tree.root());
    let verdict = gate.judge(&pointer);
    assert_eq!(codes(&verdict), vec!["1e", "1f"], "{verdict:?}");
    assert_says(&finding(&verdict, "1f").why(), "pointer");
}

#[test]
fn the_three_content_stores_of_6_13_are_detected() {
    let tree = Tree::new("stores");
    tree.dir(".dvc/cache");
    tree.file(
        ".gitattributes",
        b"*.psd filter=lfs diff=lfs merge=lfs -text\n",
    );

    // A real repository, because the annex store is located through git now
    // rather than by joining `.git` onto the root. This test previously wrote
    // `.git/annex/objects` into a plain directory and passed — which it could
    // only do because the probe was wrong in the way the worktree test below
    // demonstrates.
    let repo = Repo::init(tree.root()).expect("init");
    let git_dir = repo.common_dir().expect("common dir");
    fs::create_dir_all(git_dir.join("annex/objects")).expect("annex");

    let gate = StateGate::survey_in(tree.root(), Some(&repo));
    let mut stores = gate.data_stores().to_vec();
    stores.sort();
    assert_eq!(
        stores,
        vec![DataStore::GitAnnex, DataStore::Dvc, DataStore::GitLfs]
    );

    // A repository using none of them must say so, or the signal is noise.
    let plain = plain_tree("no-stores");
    assert!(StateGate::survey(plain.root()).data_stores().is_empty());
}

#[test]
fn acquired_data_formats_are_caught() {
    let tree = plain_tree("data-exts");
    for name in [
        "export.csv",
        "part-0.parquet",
        "events.ndjson",
        "t.avro",
        "t.feather",
    ] {
        let path = tree.file(name, b"id\n1\n");
        let gate = StateGate::survey(tree.root());
        assert!(
            codes(&gate.judge(&path)).contains(&"1f"),
            "{name}: {:?}",
            gate.judge(&path)
        );
    }
    let hit = sniff(b"PAR1\x00\x00\x00\x00").expect("parquet magic");
    assert_eq!(hit.class, StateClass::AcquiredData);
}

// ---------------------------------------------------------------------------
// cross-cutting: the output is a conflict list, not a score (§9.13)
// ---------------------------------------------------------------------------

#[test]
fn every_class_states_what_it_is_and_what_to_do_instead() {
    for class in StateClass::ALL {
        assert!(!class.code().is_empty());
        assert!(!class.title().is_empty(), "{:?}", class);
        let remediation = class.remediation();
        assert!(
            remediation.len() > 20,
            "{:?} remediation is not actionable: {remediation}",
            class
        );
        // Nothing in Gate 1 may propose deletion as the remedy.
        assert!(
            !remediation.to_lowercase().contains("delete it"),
            "{:?} proposes deletion: {remediation}",
            class
        );
    }
    let codes: Vec<&str> = StateClass::ALL.iter().map(|c| c.code()).collect();
    assert_eq!(codes, vec!["1a", "1b", "1c", "1d", "1e", "1f"]);
}

#[test]
fn findings_are_ordered_deduplicated_and_stable() {
    let tree = Tree::new("stable");
    tree.file("main.tf", b"resource \"x\" \"y\" {}\n");
    // One path that trips 1a (tree), 1c (tfvars) and 1b (secret name).
    let path = tree.file("secrets.auto.tfvars", b"db_password = \"hunter2\"\n");

    let gate = StateGate::survey(tree.root());
    let first = gate.judge(&path);
    let second = gate.judge(&path);
    assert_eq!(first, second, "judging must be deterministic");
    assert_eq!(codes(&first), vec!["1a", "1b", "1c"], "{first:?}");

    let mut seen: Vec<(&str, &Path)> = first
        .findings()
        .iter()
        .map(|f| (f.class.code(), f.path.as_path()))
        .collect();
    let before = seen.len();
    seen.dedup();
    assert_eq!(seen.len(), before, "duplicate findings: {first:?}");

    // The display form has to name the path, or a report cannot be acted on.
    let rendered = format!("{}", finding(&first, "1c"));
    assert_says(&rendered, "secrets.auto.tfvars");
    assert_says(&rendered, "1c");
}

#[test]
fn a_relative_path_and_an_absolute_path_get_the_same_verdict() {
    let tree = plain_tree("relative");
    tree.file("terraform.tfstate", b"{}\n");
    let gate = StateGate::survey(tree.root());
    assert_eq!(
        gate.judge(&tree.root().join("terraform.tfstate")),
        gate.judge(Path::new("terraform.tfstate"))
    );
}

#[test]
fn a_missing_path_is_refused_rather_than_cleared() {
    // A candidate that vanished between the walk and the judgement is not a
    // candidate that was proved harmless.
    let tree = plain_tree("missing");
    let gate = StateGate::survey(tree.root());
    let verdict = gate.judge(&tree.root().join("gone.txt"));
    assert!(verdict.is_ineligible(), "{verdict:?}");
}

/// §6.13's stores live inside the git directory, and `<root>/.git` is not it in
/// three ordinary layouts.
///
/// The probe used to read `root.join(".git/annex")`. In a **linked worktree**
/// `.git` is a regular file holding `gitdir: …`, so an annexed repository
/// checked out as a worktree reported no annex and §6.13's whole class of
/// hazard went unseen. Found while designing Gate 0; verified here.
#[test]
fn the_annex_store_is_found_through_the_common_git_dir_not_through_root_dot_git() {
    let main = tempfile::Builder::new()
        .prefix("judged-store-main-")
        .tempdir()
        .expect("scratch");
    let repo = Repo::init(main.path()).expect("init");
    std::fs::write(main.path().join("README.md"), "x\n").expect("write");
    repo.add_all().expect("add");
    repo.commit("initial").expect("commit");

    // The store, where git-annex actually keeps it.
    let git_dir = repo.common_dir().expect("common dir");
    std::fs::create_dir_all(git_dir.join("annex/objects")).expect("annex");

    assert!(
        StateGate::survey_in(main.path(), Some(&repo))
            .data_stores()
            .contains(&DataStore::GitAnnex),
        "the main working tree must see its own annex"
    );

    // And the case the old probe was blind to.
    let linked = main
        .path()
        .parent()
        .expect("parent")
        .join("judged-store-wt");
    let added = std::process::Command::new("git")
        .args(["worktree", "add", "-q"])
        .arg(&linked)
        .arg("HEAD")
        .current_dir(main.path())
        .output()
        .expect("spawn git");
    if !added.status.success() {
        // Never a silent skip: a probe that could not be set up is not a probe
        // that passed.
        panic!(
            "could not create a linked worktree, so the regression this test exists for was \
             never exercised: {}",
            String::from_utf8_lossy(&added.stderr)
        );
    }

    assert!(
        linked.join(".git").is_file(),
        "a linked worktree's .git is a FILE — that is the whole defect"
    );
    let worktree = Repo::discover(&linked).expect("the worktree is a repository");
    assert!(
        StateGate::survey_in(&linked, Some(&worktree))
            .data_stores()
            .contains(&DataStore::GitAnnex),
        "the annex is in the COMMON git dir, and the worktree must still see it"
    );

    std::process::Command::new("git")
        .args(["worktree", "remove", "--force"])
        .arg(&linked)
        .current_dir(main.path())
        .output()
        .expect("cleanup");
}
