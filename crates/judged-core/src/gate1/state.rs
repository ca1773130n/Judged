//! Gate 1 classes 1a–1f — the state whose destruction reaches outside the
//! repository.
//!
//! [`crate::gate1`] explains why Gate 1 exists at all: it is the only layer that
//! reasons about the **cost of being wrong** rather than about usefulness. This
//! module holds the six classes where that cost is not paid inside the working
//! tree. Everything here can be provably unreferenced, provably uncovered,
//! provably unimported — and deleting it still reaches a cloud account, a
//! credential issuer, a database, or a dataset that exists nowhere else.
//!
//! # 1a is the one that changes the shape of the whole run
//!
//! §6.10 calls external effectors *"the largest single gap in the whole
//! corpus"*, and the mechanism is worth stating precisely rather than
//! gesturing at. In a GitOps or IaC repository **a file is not a description of
//! desired state that a cleaner may prune; it is the desired state**. Deleting
//! it is an imperative `destroy`, executed later, by a controller, on a
//! schedule nobody is watching.
//!
//! HashiCorp's own resource-block reference, on the strongest anti-destruction
//! annotation the language offers: *"Terraform rejects operations to destroy
//! the resource and returns an error. **This rule doesn't prevent Terraform
//! from destroying the resource if you remove the resource configuration.**"*
//! `prevent_destroy` is bypassed by exactly the operation a repo cleaner
//! performs. `git revert` restores the HCL; the RDS instance is gone. Argo CD
//! is the same shape from the other end — pruning is off by default, but
//! `syncPolicy.automated.prune: true` is the documented way to turn it on, and
//! v1.8 added `allowEmpty` as a *second* guard against pruning to zero
//! resources, which is direct evidence that repo-side deletion destroying live
//! resources is a recurring production failure rather than a hypothetical.
//!
//! So 1a is not a per-file class. When any marker is present **the whole tree**
//! carries [`StateGate::has_external_effector`] and is ineligible above
//! report-only, regardless of every other signal. The canonical false negative
//! (§6.10) is an ArgoCD directory-recursive Application: `k8s/prod/postgres-pvc.yaml`
//! is referenced by nothing, imported by nothing, covered by nothing, and its
//! filename appears nowhere in the repository. Every signal in a naive design
//! returns UNUSED at maximum confidence. The next sync deletes the live
//! PersistentVolumeClaim.
//!
//! # Magic bytes first, extensions second — and the blind spot that follows
//!
//! §2.1 rates the magic-byte sniff a perfect-portability veto at the cost of one
//! short `pread` per file, and [`sniff`] is that check. It is genuinely strong:
//! it catches a SQLite database named `cache.tmp`, which no extension list ever
//! will.
//!
//! Its blind spot is equally real and is the reason 1b and 1c exist as separate
//! name-driven classes instead of being folded into 1d: **plain text has no
//! magic**. `.env`, `*.tfstate`, `.npmrc`, `.Rhistory` and a CSV of customers
//! are all unrecoverable and all invisible to a content sniff. A design that
//! trusted the sniff alone would clear every one of them. The test suite pins
//! that blind spot as an executable fact rather than a comment.
//!
//! # Why the classes overlap on purpose
//!
//! `secrets.auto.tfvars` in a Terraform repository is 1a, 1b and 1c at once; a
//! `.pt` file containing a git-lfs pointer is 1e and 1f. §9.13 asks for **a
//! conflict list, not a score**, and that is what [`StateVerdict::Ineligible`]
//! carries: one finding per objecting class, each with its own evidence and its
//! own remediation, because "rotate this credential" and "this is the mapping
//! to every provisioned resource" are different actions for the same file and
//! collapsing them into a number destroys both.
//!
//! # Nothing here proposes a deletion, and abstention is not consent
//!
//! Every [`StateClass::remediation`] names something to do *other than*
//! deleting — escalate, rotate, `terraform state rm`, `git annex get`,
//! `dvc push`. [`StateVerdict::Abstain`] means only that these six classes
//! recognised nothing; it is never a claim that a candidate is safe, and the
//! remaining Gate 1 classes, Gate 2 and Gate 0g all still have to run.
//!
//! Two boundaries are worth naming, because both look like bugs and are not:
//!
//! - **`.Rhistory` and friends abstain here.** They are §9.3's 1h, session and
//!   scratch state, which this module does not own. They are also invisible to
//!   [`sniff`], so 1a–1f neither claim them nor clear them, and the test suite
//!   pins that so a later edit cannot quietly annex 1h into this file — or read
//!   the abstention as safety.
//! - **An unreadable candidate is reported under 1d.** It is not literally a
//!   database. The sniff is 1d's mechanism, five of the six classes depend on
//!   it, and a file whose head cannot be read has been cleared by none of them
//!   — so it is refused under the class that owns the check that failed, with
//!   the failure quoted, rather than invented into a seventh class or silently
//!   dropped.
//!
//! # Cost, and what is deliberately not skipped
//!
//! [`StateGate::survey`] walks the tree once, reads no file except `.gitattributes`
//! and YAML (bounded, for Kubernetes API groups), and skips only `.git`. It does
//! **not** skip `node_modules/`, `vendor/` or `.venv/`: a skip list is a silent
//! false negative, and 1a is the class where a false negative pages someone. A
//! marker found under a vendored path is reported *with its path*, so a human
//! can see what it is. When the walk cannot finish — an unreadable directory, a
//! budget exhausted, a directory symlink pointing out of the tree — the tree is
//! treated as carrying an effector, because §6.20's rule holds here too: a
//! search that did not finish has found nothing *because it did not look*.
//!
//! Symlinks get three separate treatments, and the difference matters: a link
//! to a directory **inside** the tree is skipped in silence (its target is
//! walked anyway, and pnpm and yarn workspaces are built entirely from these);
//! a link to a directory **outside** the tree becomes a scan gap naming the
//! link, because descending it is unbounded — a link to `/` walks the machine —
//! and *silently* not descending it is the false negative this class exists to
//! prevent; and a **broken** link is skipped in silence, because that is
//! git-annex's documented normal state and treating it as a gap would put every
//! annex and DVC repository into permanent report-only.

use std::fmt;
use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use aho_corasick::AhoCorasick;

use crate::git::Repo;

/// Bytes read from the head of a candidate file.
///
/// §2.1 costs the sniff at one 32-byte `pread`. This reads 64, for one measured
/// reason: the longest signature that has to be told apart from its neighbours
/// is `-----BEGIN OPENSSH PRIVATE KEY-----`, which is 35 bytes. A 32-byte
/// window classifies an SSH private key as unknown, and `-----BEGIN ` alone
/// cannot distinguish a private key (1b, escalate for rotation) from a
/// certificate (public by construction, and somebody else's class).
pub const HEAD_BYTES: usize = 64;

/// Bytes read from a YAML file when looking for a Kubernetes API group.
const PROBE_BYTES: u64 = 64 * 1024;

/// Directory entries the survey will visit before giving up.
///
/// Exhausting it is recorded as a scan gap, never as an absence of markers.
const MAX_ENTRIES: usize = 1_000_000;

/// YAML files the survey will read before giving up.
const MAX_PROBES: usize = 50_000;

/// Markers retained per ecosystem. [`StateGate::effector_total`] keeps the
/// true count; the list is capped so a repository with ten thousand `.tf`
/// files does not carry ten thousand `PathBuf`s. Capping the *evidence* is
/// safe in a way that capping the *scan* would not be: one marker and ten
/// thousand markers produce the same verdict.
const MARKERS_PER_SYSTEM: usize = 8;

// ---------------------------------------------------------------------------
// classes
// ---------------------------------------------------------------------------

/// The six Gate 1 classes this module owns (§9.3, 1a–1f).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum StateClass {
    /// **1a** — the tree drives infrastructure. Deleting a file is a `destroy`.
    ExternalEffector,
    /// **1b** — credentials and identity. Escalate for rotation, never delete.
    SecretOrIdentity,
    /// **1c** — Terraform/Pulumi/Ansible state and variables.
    InfrastructureState,
    /// **1d** — local databases and on-disk persistence.
    LocalPersistence,
    /// **1e** — models, weights, checkpoints.
    ModelOrCheckpoint,
    /// **1f** — downloaded or acquired data, and the tools that manage it.
    AcquiredData,
}

impl StateClass {
    /// Every class this module owns, in §9.3's order.
    pub const ALL: [StateClass; 6] = [
        StateClass::ExternalEffector,
        StateClass::SecretOrIdentity,
        StateClass::InfrastructureState,
        StateClass::LocalPersistence,
        StateClass::ModelOrCheckpoint,
        StateClass::AcquiredData,
    ];

    /// §9.3's identifier for the class, e.g. `"1a"`.
    pub fn code(&self) -> &'static str {
        match self {
            StateClass::ExternalEffector => "1a",
            StateClass::SecretOrIdentity => "1b",
            StateClass::InfrastructureState => "1c",
            StateClass::LocalPersistence => "1d",
            StateClass::ModelOrCheckpoint => "1e",
            StateClass::AcquiredData => "1f",
        }
    }

    /// §9.3's name for the class.
    pub fn title(&self) -> &'static str {
        match self {
            StateClass::ExternalEffector => "external effectors",
            StateClass::SecretOrIdentity => "secrets and identity",
            StateClass::InfrastructureState => "infrastructure state",
            StateClass::LocalPersistence => "local databases and persistence",
            StateClass::ModelOrCheckpoint => "models, weights and checkpoints",
            StateClass::AcquiredData => "downloaded or acquired data",
        }
    }

    /// Why destroying this class of state is not recoverable from the
    /// repository, stated once per class so every finding carries it.
    pub fn rationale(&self) -> &'static str {
        match self {
            StateClass::ExternalEffector => {
                "in a GitOps or IaC tree a manifest is not a description of desired state that a \
                 cleaner may prune, it IS the desired state: removing the file is an imperative \
                 destroy, executed later by a controller, and `git revert` restores the text \
                 while the provisioned resource stays gone (§6.10)"
            }
            StateClass::SecretOrIdentity => {
                "deleting a credential is wrong in three separate ways (§6.22): it does not \
                 remove the secret from git history, it destroys the audit trail needed to know \
                 what to rotate and what it reached, and it reports success — so the run looks \
                 clean while a live credential stays valid in someone else's system"
            }
            StateClass::InfrastructureState => {
                "this is the mapping from configuration to every resource actually provisioned. \
                 GitHub's Terraform.gitignore ignores *.tfstate, so the class is overwhelmingly \
                 untracked and git is not the backstop (§6.17); losing it does not destroy the \
                 infrastructure, it destroys the tool's knowledge of it, and recovery is a \
                 manual `terraform import` per resource"
            }
            StateClass::LocalPersistence => {
                "a live store on disk, whose contents were never in git and are not restorable \
                 by any git operation (§8.1 rung R9). The magic-byte sniff is what makes this \
                 class portable across every ecosystem (§2.1) — the name is not evidence, and a \
                 database wearing a disposable extension is the case that pays for the read"
            }
            StateClass::ModelOrCheckpoint => {
                "weights and checkpoints are frequently LFS-tracked, frequently the largest \
                 objects on the machine, and frequently reproducible only by re-running the \
                 training that produced them. That size is exactly the trap: §9.13 says never \
                 sort by bytes reclaimed, because a report ranked that way puts the most \
                 expensive object in the repository at the top of the delete list"
            }
            StateClass::AcquiredData => {
                "acquired data hides behind three tools that make it look like junk (§6.13). \
                 git-annex leaves a broken symlink as its NORMAL steady state for content not \
                 fetched locally, and `git annex get` is how it comes back — so a \"report \
                 dangling symlinks\" rule deletes the pointer to every un-fetched annexed file. \
                 DVC keeps the only copy of un-pushed data in a gitignored .dvc/cache, and \
                 .dvc/config.local holds the credentials needed to re-fetch it. git-lfs pointer \
                 files are about 130 bytes, so size-based scanners see nothing, and the real \
                 content may exist on no remote at all"
            }
        }
    }

    /// What to do instead. Every variant names an action that is not deletion.
    pub fn remediation(&self) -> &'static str {
        match self {
            StateClass::ExternalEffector => {
                "hold the whole tree at report-only. If a resource really is unwanted, retire it \
                 through the effector — `terraform destroy` / a `removed` block / \
                 `terraform state rm`, an Argo CD sync with pruning reviewed — and let the \
                 manifest removal follow the resource, never lead it."
            }
            StateClass::SecretOrIdentity => {
                "escalate for rotation, never removal: rotate the credential at its issuer \
                 first, confirm the new one works, and only then decide what to do about the \
                 file and about history (TruffleHog --only-verified proves a credential is \
                 currently valid)."
            }
            StateClass::InfrastructureState => {
                "leave it in place and back it up outside the repository. To stop tracking a \
                 resource use `terraform state rm` or a `removed` block, which are the \
                 operations that keep state and reality in agreement."
            }
            StateClass::LocalPersistence => {
                "leave it in place. If the store really is disposable, let the tool that owns it \
                 drop it (its own reset/migrate/flush command), so the application's expectation \
                 and the disk stay in agreement."
            }
            StateClass::ModelOrCheckpoint => {
                "leave it in place and rank by nothing. If it is genuinely superseded, push it \
                 to the model registry or object store that owns it first, and record the URI \
                 next to the code that loads it."
            }
            StateClass::AcquiredData => {
                "leave it in place. Confirm the content exists elsewhere first — `git annex \
                 whereis`, `dvc push` then `dvc status --cloud`, `git lfs push --all` — and \
                 treat a broken symlink under an annex as a healthy pointer, not as garbage."
            }
        }
    }
}

// ---------------------------------------------------------------------------
// ecosystems (1a)
// ---------------------------------------------------------------------------

/// An infrastructure ecosystem whose manifests are imperative destroys (§6.10).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Ecosystem {
    /// Argo CD `Application` / `ApplicationSet` / `AppProject`.
    ArgoCd,
    /// Flux `*.toolkit.fluxcd.io` controllers.
    Flux,
    /// Kustomize overlays.
    Kustomize,
    /// Helm charts.
    Helm,
    /// Terraform / OpenTofu.
    Terraform,
    /// Pulumi.
    Pulumi,
    /// Crossplane compositions and claims.
    Crossplane,
    /// Ansible playbooks and roles.
    Ansible,
    /// AWS CDK.
    Cdk,
}

impl Ecosystem {
    /// The ecosystem's name, as its users write it.
    pub fn name(&self) -> &'static str {
        match self {
            Ecosystem::ArgoCd => "Argo CD",
            Ecosystem::Flux => "Flux",
            Ecosystem::Kustomize => "Kustomize",
            Ecosystem::Helm => "Helm",
            Ecosystem::Terraform => "Terraform",
            Ecosystem::Pulumi => "Pulumi",
            Ecosystem::Crossplane => "Crossplane",
            Ecosystem::Ansible => "Ansible",
            Ecosystem::Cdk => "AWS CDK",
        }
    }

    /// The documented destruction path for this ecosystem, quoted where the
    /// quote is the argument.
    pub fn destroy_note(&self) -> &'static str {
        match self {
            Ecosystem::Terraform => {
                "HashiCorp's resource-block reference, on the strongest anti-destruction \
                 annotation the language has: \"Terraform rejects operations to destroy the \
                 resource and returns an error. This rule doesn't prevent Terraform from \
                 destroying the resource if you remove the resource configuration.\" \
                 prevent_destroy is bypassed by exactly the operation a cleaner performs"
            }
            Ecosystem::ArgoCd => {
                "Argo CD will not prune by default, but `syncPolicy.automated.prune: true` and \
                 `argocd app set --auto-prune` are the documented ways to enable it and manual \
                 sync with pruning is always available; v1.8 added `allowEmpty` as a second \
                 guard specifically against pruning to zero resources, which is evidence that \
                 repo-side deletion destroying live resources is a recurring failure"
            }
            Ecosystem::Flux => {
                "Flux's Kustomization defaults to `prune: true` in most bootstrap layouts, so a \
                 manifest removed from git is garbage-collected from the cluster on the next \
                 reconcile"
            }
            Ecosystem::Kustomize => {
                "a Kustomize overlay is applied by something — Argo CD, Flux, or `kubectl apply \
                 -k` in a pipeline — and removing a resource from `resources:` or removing the \
                 file it names is how that applier is told to stop managing it"
            }
            Ecosystem::Helm => {
                "`helm upgrade` deletes any resource no longer rendered by the chart's \
                 templates, so removing a template file destroys the live object it produced"
            }
            Ecosystem::Pulumi => {
                "Pulumi diffs the program against the stack's state; a resource no longer \
                 constructed by the program is destroyed on the next `pulumi up`"
            }
            Ecosystem::Crossplane => {
                "Crossplane claims and compositions are Kubernetes objects that own real cloud \
                 resources, and the default `deletionPolicy: Delete` destroys the external \
                 resource when the object goes away"
            }
            Ecosystem::Ansible => {
                "an Ansible role or playbook removed from the tree stops enforcing the state it \
                 declared, and roles routinely carry `state: absent` tasks and vaulted \
                 credentials that nothing else holds"
            }
            Ecosystem::Cdk => {
                "CDK synthesizes CloudFormation, and a construct no longer synthesized is \
                 removed from the stack on the next deploy — with the default RemovalPolicy \
                 destroying the underlying resource"
            }
        }
    }
}

/// How an effector marker was recognised.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkerKind {
    /// The file's name, path or extension.
    Filename,
    /// A Kubernetes API group found inside the file.
    ApiGroup,
}

impl MarkerKind {
    fn describe(&self) -> &'static str {
        match self {
            MarkerKind::Filename => "by filename",
            MarkerKind::ApiGroup => "by the API group inside it",
        }
    }
}

/// One effector marker found in the tree, with the path that proves it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectorMarker {
    /// The ecosystem the marker belongs to.
    pub system: Ecosystem,
    /// Where it was found, relative to the surveyed root.
    pub path: PathBuf,
    /// What made it recognisable.
    pub kind: MarkerKind,
}

/// A content-addressed data store in use in the tree (§6.13).
///
/// Detected so a caller can see *why* broken symlinks, tiny pointer files and
/// gitignored cache directories are refused. The per-file rules do not depend
/// on this flag — a symlink into `.git/annex/objects` and a git-lfs pointer
/// header are self-evident — so a store that goes undetected cannot turn a
/// refusal into an approval.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DataStore {
    /// `.git/annex/`, or `filter=annex` in `.gitattributes`.
    GitAnnex,
    /// `.dvc/`, `dvc.yaml`, `dvc.lock` or `*.dvc`.
    Dvc,
    /// `.git/lfs/`, or `filter=lfs` in `.gitattributes`.
    GitLfs,
}

impl DataStore {
    /// The tool's name.
    pub fn name(&self) -> &'static str {
        match self {
            DataStore::GitAnnex => STORE_ANNEX,
            DataStore::Dvc => STORE_DVC,
            DataStore::GitLfs => STORE_LFS,
        }
    }
}

const STORE_ANNEX: &str = "git-annex";
const STORE_DVC: &str = "DVC";
const STORE_LFS: &str = "git-lfs";

// ---------------------------------------------------------------------------
// evidence and findings
// ---------------------------------------------------------------------------

/// What was actually observed. Never a score, always a fact with a location.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Evidence {
    /// An effector marker somewhere in the tree. Applies to every candidate.
    TreeMarker {
        /// The ecosystem it belongs to.
        system: Ecosystem,
        /// The marker's path, relative to the surveyed root.
        marker: PathBuf,
        /// How it was recognised.
        kind: MarkerKind,
    },

    /// The survey could not finish, so absence of markers was never proved.
    ScanIncomplete {
        /// What could not be read, quoted so it can be fixed.
        detail: String,
    },

    /// The candidate's own name, extension or path.
    Name {
        /// The rule that matched, in a form a human can check.
        matched: String,
    },

    /// The candidate's leading bytes.
    Magic {
        /// The signature's name.
        label: &'static str,
        /// Where the signature starts.
        offset: usize,
    },

    /// A symlink into a content-addressed object store.
    ContentPointer {
        /// The tool that owns the store.
        store: &'static str,
        /// The link's target, whether or not it currently resolves.
        target: PathBuf,
    },

    /// The candidate lies inside a tool-managed store directory.
    InStore {
        /// The tool that owns the store.
        store: &'static str,
        /// The store directory, relative to the surveyed root.
        root: PathBuf,
    },

    /// The head of the file could not be read.
    ///
    /// §6.20: a search that did not finish has found nothing *because it did
    /// not look*. This is a hit, never an absence.
    Unreadable {
        /// The failure, quoted.
        detail: String,
    },
}

impl Evidence {
    /// How strongly this evidence pins the class, so that one candidate carries
    /// one finding per class and it is the best-founded one available.
    fn specificity(&self) -> u8 {
        match self {
            Evidence::Magic { .. } => 5,
            Evidence::ContentPointer { .. } => 4,
            Evidence::InStore { .. } => 3,
            Evidence::Name { .. } => 2,
            Evidence::TreeMarker { .. } => 1,
            Evidence::ScanIncomplete { .. } | Evidence::Unreadable { .. } => 0,
        }
    }

    /// What was observed, quoted closely enough to re-check by hand.
    ///
    /// Public because [`StateFinding::why`] is: the sentence is already part of
    /// this module's output, and a caller assembling all sixteen §9.3 classes
    /// into one conflict list needs the evidence clause on its own rather than
    /// wrapped in a four-part block it then has to take apart.
    pub fn describe(&self) -> String {
        match self {
            Evidence::TreeMarker {
                system,
                marker,
                kind,
            } => format!(
                "this tree drives infrastructure: a {} marker at {} ({}) — {}",
                system.name(),
                marker.display(),
                kind.describe(),
                system.destroy_note()
            ),
            Evidence::ScanIncomplete { detail } => format!(
                "the survey for infrastructure markers did not finish ({detail}), so the absence \
                 of an effector was never established; a truncated or errored search is a hit, \
                 never an absence (§6.20)"
            ),
            Evidence::Name { matched } => format!("the path matches {matched}"),
            Evidence::Magic { label, offset } => {
                format!("the bytes at offset {offset} are a {label}")
            }
            Evidence::ContentPointer { store, target } => format!(
                "this is a {store} pointer: a symlink to {}, which is content-addressed storage \
                 and is a broken symlink whenever the content is not fetched locally",
                target.display()
            ),
            Evidence::InStore { store, root } => format!(
                "this path is inside {}, a {store} store directory",
                root.display()
            ),
            Evidence::Unreadable { detail } => format!(
                "the head of this file could not be read ({detail}), so the content sniff that \
                 1b, 1c, 1d, 1e and 1f all rely on could not run and none of them can clear it; \
                 a search that did not finish is a hit, never an absence (§6.20)"
            ),
        }
    }
}

/// One class's refusal, with the evidence behind it and the action to take.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateFinding {
    /// Which of 1a–1f objected.
    pub class: StateClass,
    /// The candidate, relative to the surveyed root where that is possible.
    pub path: PathBuf,
    /// What was observed.
    pub evidence: Evidence,
}

impl StateFinding {
    /// The whole argument, in a form a human can act on.
    ///
    /// Four parts, always: what the file is, what was seen, why destroying it
    /// is not recoverable, and what to do instead. §9.13 asks for a conflict
    /// list rather than a score, and a conflict a reader cannot act on is a
    /// score with extra words.
    pub fn why(&self) -> String {
        format!(
            "{} {} — {}\n  evidence: {}\n  why: {}\n  instead: {}",
            self.class.code(),
            self.class.title(),
            self.path.display(),
            self.evidence.describe(),
            self.class.rationale(),
            self.class.remediation()
        )
    }
}

impl fmt::Display for StateFinding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} {} [{}]",
            self.class.code(),
            self.path.display(),
            self.class.title()
        )
    }
}

/// What classes 1a–1f have to say about one candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StateVerdict {
    /// At least one class refused. The conflict list, ordered by class, at most
    /// one finding per class — never a score.
    Ineligible(Vec<StateFinding>),

    /// These six classes recognised nothing.
    ///
    /// **This is not a safety claim.** 1g–1p, Gate 2 and Gate 0g have not run,
    /// and §9.3's 1p rule — the unknown defaults to keep — still governs.
    Abstain,
}

impl StateVerdict {
    /// Whether any of 1a–1f refused.
    pub fn is_ineligible(&self) -> bool {
        matches!(self, StateVerdict::Ineligible(_))
    }

    /// The conflict list, empty when abstaining.
    pub fn findings(&self) -> &[StateFinding] {
        match self {
            StateVerdict::Ineligible(findings) => findings,
            StateVerdict::Abstain => &[],
        }
    }

    /// The objecting classes, in §9.3's order.
    pub fn classes(&self) -> Vec<StateClass> {
        self.findings().iter().map(|f| f.class).collect()
    }
}

// ---------------------------------------------------------------------------
// magic bytes (§2.1)
// ---------------------------------------------------------------------------

/// A magic-byte hit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Magic {
    /// The class the signature proves.
    pub class: StateClass,
    /// The signature's name, for the report.
    pub label: &'static str,
    /// Where the signature starts.
    pub offset: usize,
}

/// One entry in the signature table.
struct MagicRule {
    pattern: &'static [u8],
    offset: usize,
    class: StateClass,
    label: &'static str,
}

/// The signature table.
///
/// Deliberately small and deliberately unambiguous: every entry is a documented
/// file-format signature, not a heuristic. §2.1's claim is that this is a
/// *perfect-portability* veto — it works identically in every ecosystem because
/// it reads the file rather than believing its name.
const MAGICS: &[MagicRule] = &[
    // 1d — local persistence.
    MagicRule {
        pattern: b"SQLite format 3\0",
        offset: 0,
        class: StateClass::LocalPersistence,
        label: "SQLite 3 database header",
    },
    MagicRule {
        pattern: b"REDIS",
        offset: 0,
        class: StateClass::LocalPersistence,
        label: "Redis RDB snapshot header",
    },
    MagicRule {
        pattern: b"DUCK",
        offset: 8,
        class: StateClass::LocalPersistence,
        label: "DuckDB database header",
    },
    MagicRule {
        pattern: b"Standard Jet DB",
        offset: 4,
        class: StateClass::LocalPersistence,
        label: "Microsoft Access (Jet) database header",
    },
    MagicRule {
        pattern: b"Standard ACE DB",
        offset: 4,
        class: StateClass::LocalPersistence,
        label: "Microsoft Access (ACE) database header",
    },
    MagicRule {
        pattern: b"\x00\x06\x15\x61",
        offset: 12,
        class: StateClass::LocalPersistence,
        label: "Berkeley DB btree header",
    },
    // 1b — secrets. PEM labels only; a certificate is public and is not here.
    MagicRule {
        pattern: b"-----BEGIN OPENSSH PRIVATE KEY-----",
        offset: 0,
        class: StateClass::SecretOrIdentity,
        label: "OpenSSH private key (PEM)",
    },
    MagicRule {
        pattern: b"-----BEGIN RSA PRIVATE KEY-----",
        offset: 0,
        class: StateClass::SecretOrIdentity,
        label: "RSA private key (PEM)",
    },
    MagicRule {
        pattern: b"-----BEGIN EC PRIVATE KEY-----",
        offset: 0,
        class: StateClass::SecretOrIdentity,
        label: "EC private key (PEM)",
    },
    MagicRule {
        pattern: b"-----BEGIN DSA PRIVATE KEY-----",
        offset: 0,
        class: StateClass::SecretOrIdentity,
        label: "DSA private key (PEM)",
    },
    MagicRule {
        pattern: b"-----BEGIN ENCRYPTED PRIVATE KEY-----",
        offset: 0,
        class: StateClass::SecretOrIdentity,
        label: "encrypted PKCS#8 private key (PEM)",
    },
    MagicRule {
        pattern: b"-----BEGIN PRIVATE KEY-----",
        offset: 0,
        class: StateClass::SecretOrIdentity,
        label: "PKCS#8 private key (PEM)",
    },
    MagicRule {
        pattern: b"-----BEGIN PGP PRIVATE KEY BLOCK-----",
        offset: 0,
        class: StateClass::SecretOrIdentity,
        label: "PGP private key block",
    },
    MagicRule {
        pattern: b"PuTTY-User-Key-File-",
        offset: 0,
        class: StateClass::SecretOrIdentity,
        label: "PuTTY private key",
    },
    // 1c — infrastructure state.
    MagicRule {
        pattern: b"$ANSIBLE_VAULT;",
        offset: 0,
        class: StateClass::InfrastructureState,
        label: "Ansible Vault ciphertext header",
    },
    // 1e — models and checkpoints.
    MagicRule {
        pattern: b"GGUF",
        offset: 0,
        class: StateClass::ModelOrCheckpoint,
        label: "GGUF model container",
    },
    MagicRule {
        pattern: b"\x89HDF\r\n\x1a\n",
        offset: 0,
        class: StateClass::ModelOrCheckpoint,
        label: "HDF5 container (Keras/h5 weights)",
    },
    // 1f — acquired data.
    MagicRule {
        pattern: b"version https://git-lfs.github.com/spec/",
        offset: 0,
        class: StateClass::AcquiredData,
        label: "git-lfs pointer file",
    },
    MagicRule {
        pattern: b"PAR1",
        offset: 0,
        class: StateClass::AcquiredData,
        label: "Apache Parquet file",
    },
    MagicRule {
        pattern: b"ARROW1",
        offset: 0,
        class: StateClass::AcquiredData,
        label: "Apache Arrow / Feather v2 file",
    },
    MagicRule {
        pattern: b"Obj\x01",
        offset: 0,
        class: StateClass::AcquiredData,
        label: "Apache Avro container",
    },
];

/// Classify the head of a file by its leading bytes (§2.1).
///
/// Costs nothing but the bytes already read, works identically in every
/// ecosystem, and is the only check here that a misleading filename cannot
/// defeat. It is also **blind to plain text**: `.env`, `*.tfstate`, `.npmrc`,
/// `.Rhistory` and a CSV of customers all return `None` and are all
/// unrecoverable, which is why 1b, 1c and 1f also carry name-driven rules.
pub fn sniff(head: &[u8]) -> Option<Magic> {
    for rule in MAGICS {
        let end = rule.offset.checked_add(rule.pattern.len())?;
        if head.len() >= end && &head[rule.offset..end] == rule.pattern {
            return Some(Magic {
                class: rule.class,
                label: rule.label,
                offset: rule.offset,
            });
        }
    }
    None
}

// ---------------------------------------------------------------------------
// the gate
// ---------------------------------------------------------------------------

/// Gate 1 classes 1a–1f, surveyed over one tree.
///
/// Built once per run with [`StateGate::survey`], then asked about candidates
/// with [`StateGate::judge`]. The split exists because 1a is a property of the
/// tree, not of the file: a per-file check would have to re-walk the repository
/// for every candidate to answer it.
#[derive(Debug, Clone)]
pub struct StateGate {
    root: PathBuf,
    effectors: Vec<EffectorMarker>,
    effector_total: usize,
    stores: Vec<DataStore>,
    gaps: Vec<String>,
}

impl StateGate {
    /// Walk `root` once, recording every effector marker, every content store,
    /// and everything the walk could not read.
    ///
    /// Infallible by design, exactly as [`crate::veto::recency`] is: there is no
    /// error channel a caller could mistake for "no effectors here". A failure
    /// becomes a scan gap, and a scan gap makes the tree ineligible.
    pub fn survey(root: &Path) -> StateGate {
        // Discovers rather than passing `None`, because `None` disables the
        // §6.13 store probe outright and a caller holding a perfectly good
        // working tree would get "no annex here" without one ever being looked
        // for. Review found that footgun immediately after the probe itself was
        // fixed — the same shape, one layer up.
        //
        // A path that is not in a working tree yields `None` honestly: there is
        // no git directory, so nothing can be inside one.
        let repo = Repo::discover(root).ok();
        StateGate::survey_in(root, repo.as_ref())
    }

    /// [`StateGate::survey`], told which repository it is surveying.
    ///
    /// **The entry point production must use.** §6.13's git-annex and git-lfs
    /// stores live inside the git directory, and locating that directory needs
    /// git — `<root>/.git` is a regular *file* in a linked worktree and in a
    /// submodule, and is a file pointing elsewhere under
    /// `--separate-git-dir`.
    ///
    /// `None` means the caller has no working tree, which is a definite answer
    /// about a directory that is not a repository rather than a failed probe:
    /// there is no git directory, so nothing can be inside one. A caller that
    /// *has* a repository and passes `None` would silently downgrade a real
    /// probe into that answer, which is why the production call site passes it
    /// and this shim is documented as being for callers that genuinely have
    /// none.
    pub fn survey_in(root: &Path, repo: Option<&Repo>) -> StateGate {
        let mut gate = StateGate {
            root: root.to_path_buf(),
            effectors: Vec::new(),
            effector_total: 0,
            stores: Vec::new(),
            gaps: Vec::new(),
        };

        // Probed through git, because the walk deliberately does not descend into
        // the git directory — and these two live inside it.
        //
        // Through `git rev-parse --git-common-dir`, never `<root>/.git`. Three
        // ordinary layouts make the literal path wrong and they all fail the
        // same way: `.git` is a regular FILE holding `gitdir: …`, not a
        // directory — a linked worktree (63 bytes), a submodule, and
        // `git init --separate-git-dir` (89 bytes). In an annexed repository
        // checked out as a worktree, §6.13's store went undetected and the tree
        // was judged as though no annex existed.
        //
        // A failure to locate it is a GAP, never an absence. "There is no annex
        // here" and "I could not find out" are the §6.20 pair, and only the
        // first licenses anything.
        gate.probe_git_stores(repo);
        if root.join(".dvc").exists() {
            gate.note_store(DataStore::Dvc);
        }

        let probe = match AhoCorasick::new(API_GROUP_NEEDLES.iter().map(|(needle, _)| *needle)) {
            Ok(probe) => probe,
            Err(source) => {
                // Cannot happen for a static pattern set, and is recorded
                // rather than ignored precisely because "cannot happen" is how
                // a silent false negative gets shipped.
                gate.gaps.push(format!(
                    "could not build the API-group probe, so no YAML was searched for Argo CD, \
                     Flux or Crossplane markers: {source}"
                ));
                gate.finish();
                return gate;
            }
        };

        // Resolved once, so that "does this symlink leave the tree" is asked
        // against the same form of the path the filesystem answers with.
        let canonical_root = fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
        let mut stack = vec![root.to_path_buf()];
        let mut entries = 0usize;
        let mut probes = 0usize;

        while let Some(dir) = stack.pop() {
            let listing = match fs::read_dir(&dir) {
                Ok(listing) => listing,
                Err(source) => {
                    gate.gaps.push(format!(
                        "could not read the directory {}: {source}",
                        gate.display_path(&dir)
                    ));
                    continue;
                }
            };

            // read_dir order is filesystem order; sorting makes every report,
            // every marker choice and every test deterministic.
            let mut children: Vec<PathBuf> = Vec::new();
            for entry in listing {
                match entry {
                    Ok(entry) => children.push(entry.path()),
                    Err(source) => gate.gaps.push(format!(
                        "could not read an entry of {}: {source}",
                        gate.display_path(&dir)
                    )),
                }
            }
            children.sort();

            for child in children {
                entries += 1;
                if entries > MAX_ENTRIES {
                    gate.gaps.push(format!(
                        "the survey stopped after {MAX_ENTRIES} directory entries, so part of \
                         the tree was never examined"
                    ));
                    stack.clear();
                    break;
                }

                let file_type = match fs::symlink_metadata(&child) {
                    Ok(metadata) => metadata.file_type(),
                    Err(source) => {
                        gate.gaps.push(format!(
                            "could not stat {}: {source}",
                            gate.display_path(&child)
                        ));
                        continue;
                    }
                };

                if file_type.is_symlink() {
                    // A symlink is the one way content sits at a repository
                    // path without being under the repository root, so it is
                    // the one way the walk can miss an effector. Three cases,
                    // and each has to be got right separately:
                    match fs::metadata(&child) {
                        // A directory. If it resolves back inside the tree the
                        // walk already covers it, and descending would only
                        // duplicate markers (pnpm and yarn workspaces are built
                        // entirely from these). If it leaves the tree, the
                        // content is not covered by anything — and descending
                        // is not the answer either, because a link to `/` is an
                        // unbounded walk. It is recorded as a gap instead: the
                        // tree goes to report-only and the message names the
                        // link, which is honest about not having looked.
                        Ok(metadata) if metadata.is_dir() => {
                            let inside = fs::canonicalize(&child)
                                .map(|target| target.starts_with(&canonical_root))
                                .unwrap_or(false);
                            if !inside {
                                gate.gaps.push(format!(
                                    "{} is a symlink to a directory outside the tree, which was \
                                     not walked, so no effector marker inside it could be seen",
                                    gate.display_path(&child)
                                ));
                            }
                            continue;
                        }
                        // A file. Reading through the link is bounded and cheap,
                        // so it is treated exactly like a regular file below.
                        Ok(_) => {}
                        // Broken. This is git-annex's *documented normal state*
                        // for un-fetched content, so it must not become a scan
                        // gap: if it did, every annex and DVC repository would
                        // be in report-only forever. It holds no marker to
                        // miss, and it is judged as 1f per candidate.
                        Err(_) => continue,
                    }
                }

                if file_type.is_dir() {
                    // §9.3 0b, through the shared classifier. `name == ".git"`
                    // misses a linked worktree and a submodule (.git is a FILE)
                    // and a bare `vendor/foo.git/` (no .git at all). This gate
                    // keeps a gap list, so an unreadable probe is recorded as
                    // well as stopping the walk.
                    let boundary = crate::boundary::classify(&child);
                    if let crate::boundary::Boundary::Unreadable(why) = &boundary {
                        gate.gaps.push(format!(
                            "did not descend into {}: {why}. Not evidence that nothing is \
                             there (§6.20).",
                            gate.display_path(&child)
                        ));
                    }
                    if boundary.stops_the_walk() {
                        continue;
                    }
                    stack.push(child);
                    continue;
                }

                let rel = gate.relative(&child);
                if let Some((system, kind)) = filename_marker(&rel) {
                    gate.note_marker(system, rel.clone(), kind);
                }
                if let Some(store) = store_marker(&rel) {
                    gate.note_store(store);
                }
                if rel.file_name().is_some_and(|name| name == ".gitattributes") {
                    for store in gate.gitattributes_stores(&child) {
                        gate.note_store(store);
                    }
                }

                if is_yaml(&rel) {
                    if probes >= MAX_PROBES {
                        continue;
                    }
                    probes += 1;
                    match read_capped(&child, PROBE_BYTES) {
                        Ok(bytes) => {
                            for hit in probe.find_iter(&bytes) {
                                let (_, system) = API_GROUP_NEEDLES[hit.pattern().as_usize()];
                                gate.note_marker(system, rel.clone(), MarkerKind::ApiGroup);
                            }
                        }
                        Err(source) => gate.gaps.push(format!(
                            "could not read {} while looking for infrastructure API groups: \
                             {source}",
                            rel.display()
                        )),
                    }
                }
            }

            if entries > MAX_ENTRIES {
                break;
            }
        }

        if probes >= MAX_PROBES {
            gate.gaps.push(format!(
                "the survey stopped reading YAML after {MAX_PROBES} files, so some manifests \
                 were never searched for Argo CD, Flux or Crossplane API groups"
            ));
        }

        gate.finish();
        gate
    }

    /// The tree root this survey covers.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Every effector marker found, sorted, capped at eight per ecosystem.
    ///
    /// The cap bounds the report, not the search: [`StateGate::effector_total`]
    /// is the true count, and one marker is as disqualifying as a thousand.
    pub fn effectors(&self) -> &[EffectorMarker] {
        &self.effectors
    }

    /// How many markers were found in total, including those not retained.
    pub fn effector_total(&self) -> usize {
        self.effector_total
    }

    /// The content-addressed stores in use (§6.13).
    pub fn data_stores(&self) -> &[DataStore] {
        &self.stores
    }

    /// Everything the survey could not read, quoted so it can be fixed.
    pub fn scan_gaps(&self) -> &[String] {
        &self.gaps
    }

    /// Whether the whole tree is ineligible above report-only (1a).
    ///
    /// True when any marker was found **or** when the survey could not finish.
    /// The second half is the point: not having looked is not the same as
    /// having found nothing (§6.20).
    pub fn has_external_effector(&self) -> bool {
        !self.effectors.is_empty() || !self.gaps.is_empty()
    }

    /// Judge one candidate against 1a–1f.
    ///
    /// `path` may be absolute or relative to the surveyed root. Returns at most
    /// one finding per class, ordered by class, each carrying the best-founded
    /// evidence available for it — content beats a name, a name beats a
    /// tree-wide property.
    ///
    /// Infallible: a candidate that cannot be read, or that no longer exists,
    /// is **refused**, not cleared.
    pub fn judge(&self, path: &Path) -> StateVerdict {
        let rel = self.relative(path);
        let absolute = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.root.join(path)
        };

        let mut findings: Vec<StateFinding> = Vec::new();
        let mut push = |class: StateClass, evidence: Evidence| {
            findings.push(StateFinding {
                class,
                path: rel.clone(),
                evidence,
            });
        };

        // 1a first: it is true of this candidate before anything about the
        // candidate itself is known.
        if let Some(evidence) = self.tree_evidence() {
            push(StateClass::ExternalEffector, evidence);
        }

        for (class, matched) in name_rules(&rel) {
            push(class, Evidence::Name { matched });
        }
        if let Some((store, root)) = store_directory(&rel) {
            push(StateClass::AcquiredData, Evidence::InStore { store, root });
        }

        match fs::symlink_metadata(&absolute) {
            Ok(metadata) if metadata.file_type().is_symlink() => match fs::read_link(&absolute) {
                Ok(target) => {
                    // A broken annex symlink is the *normal* state, so the link
                    // is judged by its target text and never by resolving it.
                    if let Some(store) = pointer_store(&target) {
                        push(
                            StateClass::AcquiredData,
                            Evidence::ContentPointer { store, target },
                        );
                    }
                }
                Err(source) => push(
                    StateClass::LocalPersistence,
                    Evidence::Unreadable {
                        detail: format!("could not read the symlink: {source}"),
                    },
                ),
            },
            Ok(metadata) if metadata.is_dir() => {}
            Ok(_) => match read_capped(&absolute, HEAD_BYTES as u64) {
                Ok(head) => {
                    if let Some(magic) = sniff(&head) {
                        push(
                            magic.class,
                            Evidence::Magic {
                                label: magic.label,
                                offset: magic.offset,
                            },
                        );
                    }
                }
                Err(source) => push(
                    StateClass::LocalPersistence,
                    Evidence::Unreadable {
                        detail: source.to_string(),
                    },
                ),
            },
            Err(source) => push(
                StateClass::LocalPersistence,
                Evidence::Unreadable {
                    detail: source.to_string(),
                },
            ),
        }

        if findings.is_empty() {
            return StateVerdict::Abstain;
        }

        // One finding per class, keeping the best-founded evidence, ordered by
        // §9.3's class order. A conflict list is only readable if the same
        // objection appears once.
        findings.sort_by(|a, b| {
            a.class
                .cmp(&b.class)
                .then(b.evidence.specificity().cmp(&a.evidence.specificity()))
        });
        findings.dedup_by(|a, b| a.class == b.class);
        StateVerdict::Ineligible(findings)
    }

    /// The 1a evidence that applies to every candidate in this tree, if any.
    fn tree_evidence(&self) -> Option<Evidence> {
        if let Some(marker) = self.effectors.first() {
            return Some(Evidence::TreeMarker {
                system: marker.system,
                marker: marker.path.clone(),
                kind: marker.kind,
            });
        }
        if !self.gaps.is_empty() {
            return Some(Evidence::ScanIncomplete {
                detail: self.gaps.join("; "),
            });
        }
        None
    }

    fn note_marker(&mut self, system: Ecosystem, path: PathBuf, kind: MarkerKind) {
        self.effector_total += 1;
        if self
            .effectors
            .iter()
            .filter(|marker| marker.system == system)
            .count()
            < MARKERS_PER_SYSTEM
        {
            self.effectors.push(EffectorMarker { system, path, kind });
        }
    }

    /// §6.13's stores, located through git rather than through `<root>/.git`.
    ///
    /// This probe used to read `root.join(".git/annex")` and was **silently
    /// false in three ordinary layouts**: a linked worktree and a submodule
    /// both write `.git` as a regular FILE holding `gitdir: …` (verified — `git
    /// worktree add` produces 63 bytes), and `git init --separate-git-dir`
    /// writes an 89-byte file pointing anywhere on the filesystem. An annexed repository checked out
    /// as a worktree reported no annex, and §6.13's whole class of hazard went
    /// unseen.
    ///
    /// A failure to locate the directory is a **gap**, never an absence. "There
    /// is no annex here" and "I could not find out" are §6.20's pair, and only
    /// the first licenses anything.
    fn probe_git_stores(&mut self, repo: Option<&Repo>) {
        let Some(repo) = repo else {
            // No working tree, so no git directory, so nothing inside one. A
            // definite answer rather than an unexamined one.
            return;
        };
        match repo.common_dir() {
            Ok(git_dir) => {
                if git_dir.join("annex").exists() {
                    self.note_store(DataStore::GitAnnex);
                }
                if git_dir.join("lfs").exists() {
                    self.note_store(DataStore::GitLfs);
                }
            }
            Err(source) => self.gaps.push(format!(
                "could not locate the git directory, so no git-annex or git-lfs store was \
                 looked for: {source}. That is not evidence that neither is present (§6.13)."
            )),
        }
    }

    fn note_store(&mut self, store: DataStore) {
        if !self.stores.contains(&store) {
            self.stores.push(store);
        }
    }

    fn gitattributes_stores(&self, path: &Path) -> Vec<DataStore> {
        let mut found = Vec::new();
        let Ok(bytes) = read_capped(path, PROBE_BYTES) else {
            return found;
        };
        let text = String::from_utf8_lossy(&bytes);
        if text.contains("filter=lfs") {
            found.push(DataStore::GitLfs);
        }
        if text.contains("filter=annex") {
            found.push(DataStore::GitAnnex);
        }
        found
    }

    /// Sort everything the survey collected, so two runs over the same tree
    /// produce byte-identical reports.
    fn finish(&mut self) {
        self.effectors
            .sort_by(|a, b| a.system.cmp(&b.system).then(a.path.cmp(&b.path)));
        self.stores.sort();
        self.gaps.sort();
        self.gaps.dedup();
    }

    /// `path` relative to the surveyed root where possible, unchanged where not.
    fn relative(&self, path: &Path) -> PathBuf {
        if path.is_relative() {
            return path.to_path_buf();
        }
        path.strip_prefix(&self.root)
            .map(Path::to_path_buf)
            .unwrap_or_else(|_| path.to_path_buf())
    }

    fn display_path(&self, path: &Path) -> String {
        self.relative(path).display().to_string()
    }
}

// ---------------------------------------------------------------------------
// 1a — marker recognition
// ---------------------------------------------------------------------------

/// Kubernetes API groups that identify a controller which prunes.
///
/// Searched in YAML only. A README that mentions `argoproj.io` must not put an
/// entire repository into report-only; a manifest that declares it must.
const API_GROUP_NEEDLES: &[(&str, Ecosystem)] = &[
    ("argoproj.io", Ecosystem::ArgoCd),
    ("fluxcd.io", Ecosystem::Flux),
    ("crossplane.io", Ecosystem::Crossplane),
    ("kustomize.config.k8s.io", Ecosystem::Kustomize),
];

fn filename_marker(rel: &Path) -> Option<(Ecosystem, MarkerKind)> {
    let name = file_name(rel)?;
    let lower = name.to_ascii_lowercase();
    let extension = extension(rel);

    let system = if extension.as_deref() == Some("tf")
        || lower.ends_with(".tf.json")
        || lower == ".terraform.lock.hcl"
        || lower == "terragrunt.hcl"
    {
        Ecosystem::Terraform
    } else if name.starts_with("Pulumi.") && matches!(extension.as_deref(), Some("yaml" | "yml")) {
        Ecosystem::Pulumi
    } else if lower == "cdk.json" {
        Ecosystem::Cdk
    } else if lower == "chart.yaml" || lower == "chart.yml" {
        Ecosystem::Helm
    } else if lower == "kustomization.yaml"
        || lower == "kustomization.yml"
        || name == "Kustomization"
    {
        Ecosystem::Kustomize
    } else if lower == "ansible.cfg" || is_ansible_role_task(rel) {
        Ecosystem::Ansible
    } else {
        return None;
    };
    Some((system, MarkerKind::Filename))
}

/// `roles/<role>/tasks/main.yml` — §6.10's `roles/` marker, made specific
/// enough that a directory called `roles` in an unrelated project does not
/// put the whole tree into report-only.
fn is_ansible_role_task(rel: &Path) -> bool {
    let parts = components(rel);
    let name = parts.last().map(String::as_str).unwrap_or_default();
    if name != "main.yml" && name != "main.yaml" {
        return false;
    }
    parts.len() >= 4
        && parts[parts.len() - 2] == "tasks"
        && parts[..parts.len() - 3].contains(&"roles".to_string())
}

fn is_yaml(rel: &Path) -> bool {
    matches!(extension(rel).as_deref(), Some("yaml" | "yml"))
}

// ---------------------------------------------------------------------------
// 1b–1f — name and path rules
// ---------------------------------------------------------------------------

/// Extensions that are, on their own, sufficient to place a file in a class.
const EXTENSION_RULES: &[(&str, StateClass)] = &[
    // 1b — key material.
    ("pem", StateClass::SecretOrIdentity),
    ("key", StateClass::SecretOrIdentity),
    ("p8", StateClass::SecretOrIdentity),
    ("p12", StateClass::SecretOrIdentity),
    ("pfx", StateClass::SecretOrIdentity),
    ("pkcs12", StateClass::SecretOrIdentity),
    ("jks", StateClass::SecretOrIdentity),
    ("jceks", StateClass::SecretOrIdentity),
    ("keystore", StateClass::SecretOrIdentity),
    ("kdbx", StateClass::SecretOrIdentity),
    ("ppk", StateClass::SecretOrIdentity),
    ("asc", StateClass::SecretOrIdentity),
    ("gpg", StateClass::SecretOrIdentity),
    ("agekey", StateClass::SecretOrIdentity),
    // 1c — infrastructure state.
    ("tfstate", StateClass::InfrastructureState),
    ("tfvars", StateClass::InfrastructureState),
    ("tfplan", StateClass::InfrastructureState),
    // 1d — local persistence.
    ("db", StateClass::LocalPersistence),
    ("db3", StateClass::LocalPersistence),
    ("sqlite", StateClass::LocalPersistence),
    ("sqlite3", StateClass::LocalPersistence),
    ("sqlitedb", StateClass::LocalPersistence),
    ("mdb", StateClass::LocalPersistence),
    ("accdb", StateClass::LocalPersistence),
    ("realm", StateClass::LocalPersistence),
    ("rdb", StateClass::LocalPersistence),
    ("aof", StateClass::LocalPersistence),
    ("ldb", StateClass::LocalPersistence),
    ("sst", StateClass::LocalPersistence),
    ("frm", StateClass::LocalPersistence),
    ("ibd", StateClass::LocalPersistence),
    ("myd", StateClass::LocalPersistence),
    ("myi", StateClass::LocalPersistence),
    ("dbf", StateClass::LocalPersistence),
    ("fdb", StateClass::LocalPersistence),
    // 1e — models, weights, checkpoints.
    ("pt", StateClass::ModelOrCheckpoint),
    ("pth", StateClass::ModelOrCheckpoint),
    ("ckpt", StateClass::ModelOrCheckpoint),
    ("safetensors", StateClass::ModelOrCheckpoint),
    ("onnx", StateClass::ModelOrCheckpoint),
    ("tflite", StateClass::ModelOrCheckpoint),
    ("gguf", StateClass::ModelOrCheckpoint),
    ("ggml", StateClass::ModelOrCheckpoint),
    ("joblib", StateClass::ModelOrCheckpoint),
    ("mlmodel", StateClass::ModelOrCheckpoint),
    ("mlpackage", StateClass::ModelOrCheckpoint),
    ("caffemodel", StateClass::ModelOrCheckpoint),
    ("pdparams", StateClass::ModelOrCheckpoint),
    ("engine", StateClass::ModelOrCheckpoint),
    ("h5", StateClass::ModelOrCheckpoint),
    ("hdf5", StateClass::ModelOrCheckpoint),
    ("pkl", StateClass::ModelOrCheckpoint),
    ("pickle", StateClass::ModelOrCheckpoint),
    // 1f — acquired data.
    ("csv", StateClass::AcquiredData),
    ("tsv", StateClass::AcquiredData),
    ("parquet", StateClass::AcquiredData),
    ("avro", StateClass::AcquiredData),
    ("orc", StateClass::AcquiredData),
    ("feather", StateClass::AcquiredData),
    ("arrow", StateClass::AcquiredData),
    ("ndjson", StateClass::AcquiredData),
    ("jsonl", StateClass::AcquiredData),
    ("npy", StateClass::AcquiredData),
    ("npz", StateClass::AcquiredData),
    ("dvc", StateClass::AcquiredData),
];

/// Exact filenames that place a file in a class.
const NAME_RULES: &[(&str, StateClass)] = &[
    // 1b — identity and credential files, by their conventional names.
    ("id_rsa", StateClass::SecretOrIdentity),
    ("id_dsa", StateClass::SecretOrIdentity),
    ("id_ecdsa", StateClass::SecretOrIdentity),
    ("id_ecdsa_sk", StateClass::SecretOrIdentity),
    ("id_ed25519", StateClass::SecretOrIdentity),
    ("id_ed25519_sk", StateClass::SecretOrIdentity),
    (".netrc", StateClass::SecretOrIdentity),
    ("_netrc", StateClass::SecretOrIdentity),
    (".npmrc", StateClass::SecretOrIdentity),
    (".pypirc", StateClass::SecretOrIdentity),
    (".git-credentials", StateClass::SecretOrIdentity),
    (".htpasswd", StateClass::SecretOrIdentity),
    (".dockercfg", StateClass::SecretOrIdentity),
    (".pgpass", StateClass::SecretOrIdentity),
    (".my.cnf", StateClass::SecretOrIdentity),
    (".s3cfg", StateClass::SecretOrIdentity),
    ("credentials", StateClass::SecretOrIdentity),
    ("credentials.json", StateClass::SecretOrIdentity),
    // 1d — stores whose name is the whole signal.
    ("PG_VERSION", StateClass::LocalPersistence),
    ("dump.rdb", StateClass::LocalPersistence),
    ("appendonly.aof", StateClass::LocalPersistence),
    ("data.mdb", StateClass::LocalPersistence),
    ("lock.mdb", StateClass::LocalPersistence),
    // 1e — the ecosystem's fixed names, where the extension is uninformative.
    ("pytorch_model.bin", StateClass::ModelOrCheckpoint),
    ("model.bin", StateClass::ModelOrCheckpoint),
    ("tf_model.h5", StateClass::ModelOrCheckpoint),
    ("flax_model.msgpack", StateClass::ModelOrCheckpoint),
    ("saved_model.pb", StateClass::ModelOrCheckpoint),
    // 1f — the data tools' own files.
    ("dvc.lock", StateClass::AcquiredData),
];

/// `.env` segments that mark a template rather than a credential.
///
/// A template holds no secret, and reporting one as a secret sends a human on
/// a rotation hunt for nothing — which is how a never-delete class stops being
/// believed and starts being overridden.
const ENV_TEMPLATE_TOKENS: &[&str] = &[
    "example", "sample", "template", "dist", "defaults", "schema", "tpl", "tmpl",
];

/// Extensions on which a `secret`-bearing filename is treated as a credential
/// store. Deliberately excludes source extensions: `secrets.py` is code.
const SECRET_CONFIG_EXTENSIONS: &[&str] = &[
    "yaml",
    "yml",
    "json",
    "toml",
    "ini",
    "cfg",
    "conf",
    "properties",
    "env",
    "enc",
    "tfvars",
];

/// Path components that make everything beneath them credential material.
const SECRET_DIRECTORIES: &[&str] = &[".ssh", ".gnupg", ".aws", ".azure", ".kube"];

fn name_rules(rel: &Path) -> Vec<(StateClass, String)> {
    let mut hits: Vec<(StateClass, String)> = Vec::new();
    let Some(name) = file_name(rel) else {
        return hits;
    };
    let lower = name.to_ascii_lowercase();
    let parts = components(rel);
    let extension = extension(rel);

    // --- 1b -------------------------------------------------------------
    if lower == ".env" || lower.starts_with(".env.") || lower.ends_with(".env") {
        let is_template = lower
            .split('.')
            .any(|segment| ENV_TEMPLATE_TOKENS.contains(&segment));
        if !is_template {
            hits.push((
                StateClass::SecretOrIdentity,
                format!(
                    "the dotenv name `{name}`, which is not one of the template forms \
                         (.env.example, .env.sample, .env.template, .env.dist)"
                ),
            ));
        }
    }
    if lower.contains("secret")
        && extension
            .as_deref()
            .is_some_and(|ext| SECRET_CONFIG_EXTENSIONS.contains(&ext))
    {
        hits.push((
            StateClass::SecretOrIdentity,
            format!("the name `{name}`: a credential store, not source"),
        ));
    }
    if let Some(directory) = parts
        .iter()
        .find(|part| SECRET_DIRECTORIES.contains(&part.as_str()))
    {
        hits.push((
            StateClass::SecretOrIdentity,
            format!("its location inside `{directory}/`, which holds identity material"),
        ));
    }
    if parts.len() >= 2 && parts[parts.len() - 2] == ".dvc" && lower == "config.local" {
        hits.push((
            StateClass::SecretOrIdentity,
            "`.dvc/config.local`, which DVC documents as the place for \"sensitive values \
             (secrets) which should not reach the Git repo (credentials, private locations)\""
                .to_string(),
        ));
    }

    // --- 1c -------------------------------------------------------------
    if lower.contains(".tfstate") {
        hits.push((
            StateClass::InfrastructureState,
            format!("the Terraform state name `{name}` (*.tfstate and its .backup rotations)"),
        ));
    }
    if lower.ends_with(".tfvars.json") {
        hits.push((
            StateClass::InfrastructureState,
            format!("the Terraform variables name `{name}`"),
        ));
    }
    if name.starts_with("Pulumi.")
        && matches!(extension.as_deref(), Some("yaml" | "yml"))
        && lower != "pulumi.yaml"
        && lower != "pulumi.yml"
    {
        hits.push((
            StateClass::InfrastructureState,
            format!(
                "the Pulumi stack settings name `{name}`, which carries the stack's \
                     configuration and its encrypted secrets"
            ),
        ));
    }
    if parts.first().is_some_and(|first| first == ".pulumi") {
        hits.push((
            StateClass::InfrastructureState,
            "its location inside `.pulumi/`, the local-backend state directory".to_string(),
        ));
    }
    if parts.iter().any(|part| part == "cdk.out") {
        hits.push((
            StateClass::InfrastructureState,
            "its location inside `cdk.out/`, the synthesized CloudFormation and asset staging \
             directory"
                .to_string(),
        ));
    }

    // --- 1d/1e/1f by extension -----------------------------------------
    if let Some(ext) = extension.as_deref() {
        for (candidate, class) in EXTENSION_RULES {
            if *candidate == ext {
                hits.push((*class, format!("the `.{ext}` extension")));
            }
        }
    }

    // --- exact names ----------------------------------------------------
    for (candidate, class) in NAME_RULES {
        if *candidate == name || candidate.to_ascii_lowercase() == lower {
            hits.push((*class, format!("the filename `{candidate}`")));
        }
    }
    if lower.starts_with("ib_logfile") || lower.starts_with("ibdata") {
        hits.push((
            StateClass::LocalPersistence,
            format!("the InnoDB filename `{name}`"),
        ));
    }

    hits
}

/// The store directory a path lies inside, if any.
fn store_directory(rel: &Path) -> Option<(&'static str, PathBuf)> {
    let parts = components(rel);
    if parts.len() > 1 && parts[0] == ".dvc" {
        return Some((STORE_DVC, PathBuf::from(".dvc")));
    }
    None
}

/// The store a symlink points into, judged from the target text alone.
///
/// Never resolved: an annexed file whose content is not fetched is a **broken**
/// symlink, and that is git-annex's documented normal state — *"the file will
/// still appear in your work tree as a broken symlink. You can use `git annex
/// get` to as usual to get this file back."*
fn pointer_store(target: &Path) -> Option<&'static str> {
    let text = target.to_string_lossy().replace('\\', "/");
    if text.contains(".git/annex/objects") || text.contains("/annex/objects/") {
        return Some(STORE_ANNEX);
    }
    if text.contains(".dvc/cache") {
        return Some(STORE_DVC);
    }
    if text.contains(".git/lfs/objects") {
        return Some(STORE_LFS);
    }
    None
}

/// A data store implied by a single file's name.
fn store_marker(rel: &Path) -> Option<DataStore> {
    let name = file_name(rel)?;
    if name == "dvc.yaml" || name == "dvc.lock" || extension(rel).as_deref() == Some("dvc") {
        return Some(DataStore::Dvc);
    }
    None
}

// ---------------------------------------------------------------------------
// small helpers
// ---------------------------------------------------------------------------

fn file_name(path: &Path) -> Option<String> {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
}

/// The final extension, lowercased. `raw.csv.dvc` is a `.dvc` file.
fn extension(path: &Path) -> Option<String> {
    path.extension()
        .map(|ext| ext.to_string_lossy().to_ascii_lowercase())
}

fn components(path: &Path) -> Vec<String> {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(part) => Some(part.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect()
}

/// Read at most `limit` bytes from the head of a file.
fn read_capped(path: &Path, limit: u64) -> std::io::Result<Vec<u8>> {
    let file = fs::File::open(path)?;
    let mut head = Vec::new();
    file.take(limit).read_to_end(&mut head)?;
    Ok(head)
}
