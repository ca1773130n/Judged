//! Gate 0a — never traverse a symlink (§9.3, §6.13, §6.16).
//!
//! > *"lstat everything. NEVER traverse a symlink; never rm -rf a target.
//! > Report a link only if dangling — AND ONLY IF git-annex/DVC absent."*
//!
//! # A trailing separator defeats `lstat`, and the spec does not say so
//!
//! Measured here, not taken from the clause:
//!
//! | spelling | `lstat` answers |
//! | --- | --- |
//! | `link` | the **link** — `is_symlink = true` |
//! | `link/` | the **target** — `is_symlink = false`, mode `0o40755` |
//! | `dangling/` | `ENOENT` |
//! | `filelink/` | `ENOTDIR` |
//!
//! So "lstat everything" is only meaningful on a separator-free spelling, and a
//! gate that lstats what it was handed answers about the target while believing
//! it answered about the link. Every candidate is stripped lexically before any
//! syscall.
//!
//! That spelling is also the *whole* of §6.16's hazard: `rm -rf LINK/` deletes
//! the target's **contents** while `rm -rf LINK` removes only the link, and
//! `find LINK/ -type f` enumerates files outside the repository. Rust is not
//! exempt — `std::fs::remove_dir_all("LINK/")` destroys the target too.
//!
//! # A dangling link is the normal state of an annexed repository
//!
//! §6.13 is the reason the clause ends the way it does. After `git annex drop`,
//! *"the file will still appear in your work tree as a broken symlink"* — that
//! is the **steady state** for content not fetched locally, and the content is
//! recoverable with `git annex get`. A cleaner that reports dangling symlinks as
//! candidates deletes the pointer to every un-fetched annexed file.
//!
//! So a dangling link is reportable only when no store is present, and a store
//! this gate could not *check for* is treated as present — see
//! [`StorePresence::Unreadable`].
//!
//! # Where this is narrower than §6.13 implies, and it is stated rather than hidden
//!
//! §6.13's premise does not hold by default for either tool, both verified
//! against upstream documentation during design:
//!
//! - **DVC's default is not symlinks.** *"By default, DVC tries to use reflinks
//!   … falls back to the copying strategy."* Symlinks need an explicit
//!   `cache.type`. So a DVC repository's data is usually invisible to a link
//!   rule entirely.
//! - **git-annex content is not always a symlink.** *"Files added to the annex
//!   get a symlink **or pointer file**."* Unlocked and adjusted-branch
//!   repositories store pointer *files* — regular files this gate cannot see.
//!
//! The repository-level store probe is therefore the load-bearing half and the
//! per-link rule is the narrow one. Gate 1's content classes and §6.13's own
//! marker detection cover the pointer-file case; 0a does not claim to.
//!
//! # What it does not decide
//!
//! §9.3 says what to **report** and never what may be done to a link itself.
//! Whether a dangling pointer may be unlinked is left open, deliberately, and
//! needs a determination before any auto-act.

use std::ffi::OsStr;
use std::fmt;
use std::path::{Path, PathBuf};

use crate::git::Repo;

/// Which of 0a's rules fired.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Condition {
    /// The candidate is a symlink that resolves. Never traversed, never a
    /// candidate.
    ResolvingLink,
    /// The candidate is a dangling symlink in a repository that has — or may
    /// have — a git-annex or DVC store.
    DanglingLinkWithStore,
    /// An **ancestor** of the candidate is a symlink, so reaching the candidate
    /// at all means traversing one.
    LinkedAncestor,
    /// The candidate was spelled with a trailing separator **and** names a
    /// symlink: §6.16's `rm -rf LINK/`, which deletes the target's contents.
    TrailingSeparatorOnLink,
}

impl Condition {
    /// Stable lower-case label, for reports.
    pub fn as_str(self) -> &'static str {
        match self {
            Condition::ResolvingLink => "resolving-link",
            Condition::DanglingLinkWithStore => "dangling-link-with-store",
            Condition::LinkedAncestor => "linked-ancestor",
            Condition::TrailingSeparatorOnLink => "trailing-separator-on-link",
        }
    }

    /// What acting on it would cost.
    pub fn consequence(self) -> &'static str {
        match self {
            Condition::ResolvingLink => {
                "§6.16: the target is outside this repository as often as not — Bazel's \
                 `bazel-out` points into `~/.cache/bazel` — and acting on the link's path can \
                 reach it"
            }
            Condition::DanglingLinkWithStore => {
                "§6.13: after `git annex drop` a broken symlink is the NORMAL state for content \
                 not fetched locally, and `git annex get` restores it — deleting the pointer \
                 deletes the only reference to the content"
            }
            Condition::LinkedAncestor => {
                "reaching this candidate means traversing a link, which §9.3 0a forbids outright"
            }
            Condition::TrailingSeparatorOnLink => {
                "§6.16, measured: `rm -rf LINK/` deletes the TARGET's contents while `rm -rf \
                 LINK` removes only the link, and `std::fs::remove_dir_all` behaves the same way"
            }
        }
    }
}

impl fmt::Display for Condition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Whether a content-addressed store is present, as a three-state answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StorePresence {
    /// A git-annex or DVC store was found, named here.
    Present(&'static str),
    /// Neither was found, and both were genuinely looked for.
    Absent,
    /// The probe failed. **Treated as present.** "There is no annex here" and
    /// "I could not find out" are §6.20's pair, and only the first licenses
    /// reporting a broken symlink as dead.
    Unreadable(String),
}

impl StorePresence {
    /// Whether a dangling link may be reported. Only when a store is genuinely
    /// absent.
    pub fn permits_reporting_a_dangling_link(&self) -> bool {
        matches!(self, StorePresence::Absent)
    }
}

/// One refusal, with the evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub condition: Condition,
    /// The path the rule fired on, separator-stripped.
    pub subject: PathBuf,
    pub detail: String,
}

impl fmt::Display for Finding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "0a {}: {}", self.condition, self.detail)
    }
}

/// What 0a said about one candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Refuses(Vec<Finding>),
    /// 0a has nothing to say. **Not a safety claim** — it means no link was
    /// involved, or a dangling link is reportable because no store exists.
    Clear,
    /// Could not be checked, and therefore refuses in effect.
    Unreadable(String),
}

impl Verdict {
    /// Whether 0a permits the candidate to be acted on.
    pub fn permits_action(&self) -> bool {
        matches!(self, Verdict::Clear)
    }

    pub fn findings(&self) -> &[Finding] {
        match self {
            Verdict::Refuses(findings) => findings,
            _ => &[],
        }
    }
}

/// Gate 0a over one repository.
pub struct Gate0a {
    root: PathBuf,
    store: StorePresence,
}

impl Gate0a {
    /// Probe the repository for a content-addressed store.
    ///
    /// Infallible: a failed probe becomes [`StorePresence::Unreadable`], which
    /// is treated as present. There is no constructor that yields a gate
    /// permitting a dangling link to be reported because a probe crashed.
    pub fn build(repo: &Repo) -> Gate0a {
        let store = match repo.common_dir() {
            Ok(git_dir) => {
                // `Path::exists` is wrong twice over here, and review caught
                // both: it FOLLOWS symlinks — which this gate exists to forbid —
                // and it returns `false` for a permission error, so an
                // unreadable probe would have become `Absent` and licensed
                // reporting every broken symlink in an annexed repository. That
                // is §6.20's inversion inside §6.13's own safeguard.
                match (
                    marker(&git_dir.join("annex")),
                    marker(&repo.root().join(".dvc")),
                ) {
                    (Ok(true), _) => StorePresence::Present("git-annex"),
                    (_, Ok(true)) => StorePresence::Present("DVC"),
                    (Ok(false), Ok(false)) => StorePresence::Absent,
                    (Err(why), _) | (_, Err(why)) => StorePresence::Unreadable(why),
                }
            }
            Err(source) => StorePresence::Unreadable(format!(
                "could not locate the git directory, so no git-annex store was looked for: \
                 {source}"
            )),
        };
        Gate0a {
            root: repo.root().to_path_buf(),
            store,
        }
    }

    /// What the store probe found.
    pub fn store(&self) -> &StorePresence {
        &self.store
    }

    /// Judge one candidate, **absolute**.
    pub fn judge(&self, candidate: &Path) -> Verdict {
        if candidate.is_relative() {
            return Verdict::Unreadable(format!(
                "{} is relative, and 0a will not guess which tree it belongs to",
                candidate.display()
            ));
        }

        // Before any syscall. `lstat("link/")` answers about the TARGET, so a
        // gate that skips this believes it examined a link when it examined
        // whatever the link points at.
        let (stripped, had_separator) = strip_trailing_separators(candidate);
        let mut findings = Vec::new();

        // An ancestor being a link means the candidate is only reachable by
        // traversing one — checked first, because it is true regardless of what
        // the candidate itself turns out to be.
        match self.linked_ancestor(&stripped) {
            Ok(Some(ancestor)) => findings.push(Finding {
                condition: Condition::LinkedAncestor,
                subject: ancestor.clone(),
                detail: format!(
                    "{} is a symlink on the way to {}, and {}",
                    ancestor.display(),
                    stripped.display(),
                    Condition::LinkedAncestor.consequence()
                ),
            }),
            Ok(None) => {}
            Err(why) => return Verdict::Unreadable(why),
        }

        match std::fs::symlink_metadata(&stripped) {
            Ok(meta) if meta.file_type().is_symlink() => {
                if had_separator {
                    findings.push(Finding {
                        condition: Condition::TrailingSeparatorOnLink,
                        subject: stripped.clone(),
                        detail: format!(
                            "{} was spelled with a trailing separator and names a symlink; {}",
                            candidate.display(),
                            Condition::TrailingSeparatorOnLink.consequence()
                        ),
                    });
                }
                // Dangling or not: `metadata` follows the link, so `NotFound`
                // means the target is gone. Reading the link's own bytes is not
                // needed and would not answer the question.
                let dangling = match std::fs::metadata(&stripped) {
                    Ok(_) => false,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
                    Err(error) => {
                        return Verdict::Unreadable(format!(
                            "{} is a symlink whose target could not be resolved: {error}. \
                             Neither dangling nor resolving is a true statement about it.",
                            stripped.display()
                        ))
                    }
                };

                if !dangling {
                    findings.push(Finding {
                        condition: Condition::ResolvingLink,
                        subject: stripped.clone(),
                        detail: format!(
                            "{} is a symlink that resolves, and {}",
                            stripped.display(),
                            Condition::ResolvingLink.consequence()
                        ),
                    });
                } else if !self.store.permits_reporting_a_dangling_link() {
                    findings.push(Finding {
                        condition: Condition::DanglingLinkWithStore,
                        subject: stripped.clone(),
                        detail: format!(
                            "{} is a broken symlink and this repository {}. {}",
                            stripped.display(),
                            match &self.store {
                                StorePresence::Present(name) => format!("has a {name} store"),
                                StorePresence::Unreadable(why) =>
                                    format!("could not be checked for one ({why})"),
                                StorePresence::Absent => unreachable!(
                                    "Absent permits reporting, so this arm is not reached"
                                ),
                            },
                            Condition::DanglingLinkWithStore.consequence()
                        ),
                    });
                }
            }
            // Not a link, or nothing there. Neither is 0a's business: a path
            // that does not exist is not a link, and §9.3's other conjuncts own
            // the rest.
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Verdict::Unreadable(format!(
                    "{} could not be lstat'd: {error}",
                    stripped.display()
                ))
            }
        }

        if findings.is_empty() {
            Verdict::Clear
        } else {
            Verdict::Refuses(findings)
        }
    }

    /// The first ancestor of `candidate` below the repository root that is a
    /// symlink.
    ///
    /// Bounded by the root: the tree above it is not this repository's to judge,
    /// and on macOS it is reached through `/var -> /private/var`, which would
    /// otherwise make every candidate in a temp directory a linked-ancestor
    /// refusal.
    fn linked_ancestor(&self, candidate: &Path) -> Result<Option<PathBuf>, String> {
        let Ok(relative) = candidate.strip_prefix(&self.root) else {
            // NOT `Ok(None)`. Review found that returning "no linked ancestor"
            // here made `permits_action()` mean "0a did not check this tree" —
            // and `Repo::discover` canonicalizes the root, so a candidate
            // spelled through a symlinked root falls outside it and reached
            // exactly this arm. The ancestor walk is the part of 0a that catches
            // traversal, so skipping it silently is the one outcome this gate
            // must not have. 0c owns the containment question; 0a says it could
            // not answer.
            return Err(format!(
                "{} is not under {}, so 0a could not walk its ancestors. That is a question \
                 for §9.3 0c, and this gate has not established anything about the candidate.",
                candidate.display(),
                self.root.display()
            ));
        };
        let mut cursor = self.root.clone();
        for component in relative.components() {
            cursor.push(component);
            if cursor == candidate {
                break;
            }
            match std::fs::symlink_metadata(&cursor) {
                Ok(meta) if meta.file_type().is_symlink() => return Ok(Some(cursor)),
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
                Err(error) => {
                    return Err(format!(
                        "{} could not be lstat'd while walking to {}: {error}",
                        cursor.display(),
                        candidate.display()
                    ))
                }
            }
        }
        Ok(None)
    }
}

/// Whether a store marker is present, without following a link to find out.
///
/// `symlink_metadata` rather than `Path::exists` for two independent reasons,
/// both of which review had to point out. `exists` follows symlinks, and a gate
/// whose entire subject is "never traverse a symlink" must not traverse one to
/// answer its own question. And `exists` returns `false` for a permission error,
/// which would turn "I could not look" into "there is no annex here" — the one
/// answer that licenses deleting an annexed repository's pointers.
///
/// A dangling marker counts as **present**: `.git/annex` as a broken link is
/// still a repository that has an annex.
fn marker(path: &Path) -> Result<bool, String> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!(
            "{} could not be checked for a content store: {error}",
            path.display()
        )),
    }
}

/// A path with trailing separators removed, and whether any were.
///
/// Operates on the raw bytes rather than through `Path::components`, because
/// component iteration silently normalizes the very thing being detected:
/// `Path::new("link/").components()` yields the same sequence as
/// `Path::new("link")`, so the hazard would be invisible.
///
/// A lone `/` keeps its separator — it is the root, not a trailing separator.
fn strip_trailing_separators(path: &Path) -> (PathBuf, bool) {
    let bytes = path.as_os_str().as_encoded_bytes();
    let mut end = bytes.len();
    while end > 1 && bytes[end - 1] == b'/' {
        end -= 1;
    }
    if end == bytes.len() {
        return (path.to_path_buf(), false);
    }
    // SAFETY: the slice ends on a `/` boundary, which is ASCII, so the remaining
    // bytes are still a valid `OsStr` under the same encoding they came from.
    let trimmed = unsafe { OsStr::from_encoded_bytes_unchecked(&bytes[..end]) };
    (PathBuf::from(trimmed), true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trailing_separators_are_stripped_from_the_bytes_and_the_root_survives() {
        // The empty and multi-slash cases, which review noted were uncovered.
        assert_eq!(
            strip_trailing_separators(Path::new("")),
            (PathBuf::from(""), false),
            "an empty path has no trailing separator to strip"
        );
        assert_eq!(
            strip_trailing_separators(Path::new("///")),
            (PathBuf::from("/"), true),
            "every separator but the root's own is stripped"
        );
        assert_eq!(
            strip_trailing_separators(Path::new("/a/link//")),
            (PathBuf::from("/a/link"), true)
        );
        assert_eq!(
            strip_trailing_separators(Path::new("/a/link")),
            (PathBuf::from("/a/link"), false)
        );
        assert_eq!(
            strip_trailing_separators(Path::new("/")),
            (PathBuf::from("/"), false),
            "a lone separator is the root, not a trailing separator"
        );
    }

    /// An unreadable store probe is treated as present, so a broken symlink is
    /// not reportable. §6.13's whole point.
    #[test]
    fn an_unreadable_store_probe_does_not_permit_reporting_a_dangling_link() {
        assert!(!StorePresence::Unreadable("git failed".to_string())
            .permits_reporting_a_dangling_link());
        assert!(!StorePresence::Present("git-annex").permits_reporting_a_dangling_link());
        assert!(StorePresence::Absent.permits_reporting_a_dangling_link());
    }
}
