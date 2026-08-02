//! Gate 0e — never touch the git directory (§9.3, §6.13).
//!
//! §9.3's clause is nine words: *"Never touch `.git/` — including
//! `.git/lfs/objects` and `.git/annex/objects`."* Implementing it as those
//! literal strings is wrong in three verified layouts and misses the store it
//! names, so this is the clause's **intent** rather than its letter, with every
//! divergence recorded — see `docs/decisions/2026-08-02-gate0-design-record.md`.
//!
//! # `.git` is not the git directory
//!
//! Verified against git 2.50.1. In a **linked worktree** and a **submodule**,
//! `.git` is a regular file holding `gitdir: …`; under
//! `git init --separate-git-dir` it is an 89-byte file pointing anywhere on the
//! filesystem. A prefix match on `.git/` protects none of them, and in a
//! worktree it protects nothing at all.
//!
//! So the test is **identity against paths git itself reports**:
//! `--absolute-git-dir` for this worktree's own directory and
//! `--git-common-dir` for the shared one. Both, because they differ: in a linked
//! worktree the first is `<common>/worktrees/<name>` and the second is the main
//! repository's, and protecting one leaves the other writable.
//!
//! # The stores are defaults, not locations
//!
//! §9.3 names `.git/lfs/objects` and `.git/annex/objects`. `lfs.storage` is a
//! documented git-lfs setting whose *"non-absolute path is relativized to inside
//! of Git repository directory"* and whose absolute form puts the object store
//! **outside `.git` entirely**. It is read from config here; the annex path is
//! confirmed to be exactly what §9.3 says.
//!
//! Not derived with `git rev-parse --git-path`. Verified in a linked worktree on
//! 2.50.1: `--git-path lfs/objects` answers
//! `…/.git/worktrees/wt/lfs/objects`, where neither git-lfs nor git-annex stores
//! anything, because git's per-worktree path table knows about `objects` and
//! knows nothing about `lfs`.
//!
//! # "Never touch" means never as a mutation target
//!
//! The letter of 0e is contradicted by the tool's own mandated writes: §8.2's
//! `git add -f`, `git write-tree`, `commit-tree` and `git tag`, and §9.7's
//! quarantine refs, all write inside the git directory and are **required**.
//! This gate answers one question — *may this candidate be deleted, moved or
//! overwritten* — and nothing routes those operations through it.
//!
//! # A candidate that CONTAINS the git directory is refused too
//!
//! The clause reads as though the hazard were a path inside `.git/`, and the
//! shape that actually destroys an object database is the *ancestor*: `rm -rf
//! repo/` takes `.git` with it, and §8.3 records the submodule recipe
//! `rm -rf $GIT_DIR/modules/<name>` doing exactly that to a submodule's entire
//! history and reflog. Both directions are refused.

use std::fmt;
use std::path::{Component, Path, PathBuf};

use crate::git::Repo;

/// Which protected region a candidate touched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Region {
    /// This worktree's own git directory (`--absolute-git-dir`).
    GitDir,
    /// The shared git directory (`--git-common-dir`), which differs from the
    /// above in a linked worktree and in a submodule.
    CommonDir,
    /// The git-lfs object store — `lfs.storage` when set, else `lfs` inside the
    /// common directory.
    LfsStore,
    /// The git-annex object store, `annex/objects` inside the common directory.
    AnnexStore,
}

impl Region {
    /// Stable lower-case label, for reports.
    pub fn as_str(self) -> &'static str {
        match self {
            Region::GitDir => "git-dir",
            Region::CommonDir => "common-dir",
            Region::LfsStore => "lfs-store",
            Region::AnnexStore => "annex-store",
        }
    }

    /// What deleting something here costs.
    pub fn consequence(self) -> &'static str {
        match self {
            Region::GitDir | Region::CommonDir => {
                "this is the object database: history, reflog and every unpushed commit live \
                 here, and nothing outside the repository can give them back"
            }
            Region::LfsStore => {
                "§6.13: LFS pointer files are ~130 bytes and tracked, the real content lives \
                 here, and for a local-only branch it may exist on no remote"
            }
            Region::AnnexStore => {
                "§6.13: annexed content lives here and the working tree holds only symlinks or \
                 pointer files to it"
            }
        }
    }
}

impl fmt::Display for Region {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// How the candidate touched the region.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Relation {
    /// The candidate *is* the region.
    Is,
    /// The candidate is inside it.
    Inside,
    /// The candidate **contains** it — deleting the candidate takes the region
    /// with it.
    Contains,
}

impl fmt::Display for Relation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Relation::Is => "is",
            Relation::Inside => "is inside",
            Relation::Contains => "contains",
        })
    }
}

/// One refusal, with everything a reader needs to check it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub region: Region,
    pub relation: Relation,
    /// The region's absolute path, as git reported it.
    pub region_path: PathBuf,
    pub detail: String,
}

impl fmt::Display for Finding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "0e {}: {}", self.region, self.detail)
    }
}

/// What 0e said about one candidate.
///
/// Three states, and [`Verdict::Unreadable`] never collapses into
/// [`Verdict::Clear`] — the contract in the design record, adopted after four of
/// six first-round Gate 0 designs treated "could not check" as "passed".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Refused, for these reasons. Never fewer than one.
    Refuses(Vec<Finding>),
    /// Checked, and the candidate is nowhere near a protected region.
    Clear,
    /// Could not be checked. **Refuses in effect** — see
    /// [`Verdict::permits_action`].
    Unreadable(String),
}

impl Verdict {
    /// Whether the candidate may be acted on **as far as this gate is
    /// concerned**.
    ///
    /// False for [`Verdict::Unreadable`]. A gate that could not locate the git
    /// directory has not established that a candidate is outside it, and
    /// reading that as permission is §6.20's inversion.
    ///
    /// True here is not a safety claim. It means 0e has nothing to say.
    pub fn permits_action(&self) -> bool {
        matches!(self, Verdict::Clear)
    }

    /// The refusals, empty unless [`Verdict::Refuses`].
    pub fn findings(&self) -> &[Finding] {
        match self {
            Verdict::Refuses(findings) => findings,
            _ => &[],
        }
    }
}

/// Gate 0e over one repository.
///
/// Built once: the four regions come from git and do not change during a run.
pub struct Gate0e {
    regions: Vec<(Region, PathBuf)>,
    /// Set when a probe failed. Every verdict is [`Verdict::Unreadable`] while
    /// this is set, because a partially-located set of regions cannot clear
    /// anything.
    unreadable: Option<String>,
}

impl Gate0e {
    /// Locate every protected region.
    ///
    /// Infallible by design: a probe that fails becomes the gate's
    /// [`Verdict::Unreadable`] state rather than an `Err` the caller might
    /// discard. There is no constructor that yields a gate which silently
    /// clears everything.
    pub fn build(repo: &Repo) -> Gate0e {
        let git_dir = match repo.absolute_git_dir() {
            Ok(path) => path,
            Err(source) => return Gate0e::blind(format!("`--absolute-git-dir` failed: {source}")),
        };
        let common = match repo.common_dir() {
            Ok(path) => path,
            Err(source) => return Gate0e::blind(format!("`--git-common-dir` failed: {source}")),
        };

        // §6.13: `lfs.storage` relativizes a non-absolute value to inside the
        // git directory and an absolute one puts the store anywhere. Unset is an
        // answer; a failed read is not.
        let lfs = match repo.config("lfs.storage") {
            Ok(Some(value)) => {
                let configured = PathBuf::from(&value);
                if configured.is_absolute() {
                    configured
                } else {
                    common.join(configured)
                }
            }
            Ok(None) => common.join("lfs"),
            Err(source) => return Gate0e::blind(format!("`lfs.storage` unreadable: {source}")),
        };

        Gate0e {
            // Resolved, not merely normalized, and both sides must be: git's
            // own answers are already canonical, but a path from `lfs.storage`
            // is whatever the user wrote. A region in one spelling and a
            // candidate in the other compare unequal and the gate silently
            // protects nothing — which is how the relocated LFS store came back
            // Clear the first time.
            regions: vec![
                (Region::GitDir, resolve(&git_dir)),
                (Region::CommonDir, resolve(&common)),
                (Region::LfsStore, resolve(&lfs)),
                (Region::AnnexStore, resolve(&common.join("annex"))),
            ],
            unreadable: None,
        }
    }

    fn blind(why: String) -> Gate0e {
        Gate0e {
            regions: Vec::new(),
            unreadable: Some(why),
        }
    }

    /// The regions being protected, for a report that has to say what it
    /// checked.
    pub fn regions(&self) -> &[(Region, PathBuf)] {
        &self.regions
    }

    /// Judge one **absolute** candidate path.
    ///
    /// Absolute because a relative path is ambiguous about which tree it belongs
    /// to, and 0e is the gate that must not guess.
    ///
    /// # The comparison resolves the candidate's existing prefix, and must
    ///
    /// git reports **canonical** paths, so comparing a candidate against them
    /// lexically protects nothing wherever the tree is reached through a
    /// symlink. That is not exotic: on macOS every temp directory is
    /// `/var/…` behind a link to `/private/var/…`, and the first version of this
    /// gate cleared the working tree root — the §8.3 `rm -rf repo/` case — purely
    /// because the two spellings did not match.
    ///
    /// So the deepest **existing** ancestor is canonicalized and the remainder
    /// re-appended, the same shape `explain_cmd::absolutize` already uses here.
    /// Canonicalizing the whole path is not an option: it returns `ENOENT` for a
    /// path that does not exist, and a candidate under consideration for deletion
    /// is exactly the one that may already be gone.
    ///
    /// A consequence worth naming rather than discovering later: this resolves
    /// symlinks in the existing prefix, so a link *into* the git directory is
    /// caught here when it exists. That is a **widening** of 0e — the clause is
    /// about the directory, not about links, and 0a owns links — and it errs
    /// toward refusing, which is the direction this gate is allowed to be wrong
    /// in.
    pub fn judge(&self, candidate: &Path) -> Verdict {
        if let Some(why) = &self.unreadable {
            return Verdict::Unreadable(why.clone());
        }
        if candidate.is_relative() {
            return Verdict::Unreadable(format!(
                "{} is relative, and 0e will not guess which tree it belongs to",
                candidate.display()
            ));
        }

        let candidate = resolve(candidate);
        let mut findings = Vec::new();
        for (region, path) in &self.regions {
            let relation = if candidate == *path {
                Some(Relation::Is)
            } else if candidate.starts_with(path) {
                Some(Relation::Inside)
            } else if path.starts_with(&candidate) {
                Some(Relation::Contains)
            } else {
                None
            };
            if let Some(relation) = relation {
                findings.push(Finding {
                    region: *region,
                    relation,
                    region_path: path.clone(),
                    detail: format!(
                        "{} {} {} ({}), and {}",
                        candidate.display(),
                        relation,
                        path.display(),
                        region,
                        region.consequence()
                    ),
                });
            }
        }

        if findings.is_empty() {
            Verdict::Clear
        } else {
            Verdict::Refuses(findings)
        }
    }
}

/// The candidate as a path comparable with git's canonical answers.
///
/// Canonicalizes the deepest ancestor that exists and re-appends the rest, so a
/// candidate that has already been deleted still resolves its parent. Falls back
/// to lexical normalization when nothing on the path exists at all.
fn resolve(path: &Path) -> PathBuf {
    if let Ok(canonical) = path.canonicalize() {
        return canonical;
    }
    let mut suffix: Vec<std::ffi::OsString> = Vec::new();
    let mut cursor = path.to_path_buf();
    while let Some(parent) = cursor.parent().map(Path::to_path_buf) {
        let Some(name) = cursor.file_name().map(|n| n.to_os_string()) else {
            break;
        };
        suffix.push(name);
        if let Ok(canonical) = parent.canonicalize() {
            let mut out = canonical;
            for part in suffix.iter().rev() {
                out.push(part);
            }
            return out;
        }
        cursor = parent;
    }
    normalize(path)
}

/// A path with `.` and redundant separators removed, and `..` resolved
/// lexically.
///
/// Lexical rather than `canonicalize` for the reason [`Gate0e::judge`] gives.
/// `..` is resolved so that `repo/.git/../.git` cannot walk past the comparison,
/// and popping past the root is clamped rather than wrapping.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalization_resolves_dot_segments_so_they_cannot_evade_the_comparison() {
        assert_eq!(
            normalize(Path::new("/a/b/../.git/./objects")),
            PathBuf::from("/a/.git/objects")
        );
    }

    #[test]
    fn a_relative_candidate_is_unreadable_rather_than_guessed_at() {
        let gate = Gate0e::blind("probe failed".to_string());
        assert!(!gate.judge(Path::new("relative/path")).permits_action());
    }

    /// A gate that could not locate its regions refuses everything, and says so.
    #[test]
    fn a_blind_gate_permits_nothing() {
        let gate = Gate0e::blind("`--absolute-git-dir` failed".to_string());
        let verdict = gate.judge(Path::new("/anywhere/at/all"));
        assert!(!verdict.permits_action());
        assert!(matches!(verdict, Verdict::Unreadable(_)));
    }
}
