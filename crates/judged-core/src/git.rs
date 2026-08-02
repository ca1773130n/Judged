//! Git as a recoverability oracle, not as a source of truth about liveness.
//!
//! Implemented by shelling out to the `git` binary rather than linking
//! libgit2: the research verified its findings against git 2.50.1's actual
//! behaviour (§8.1), and `git` is already a hard requirement for anything this
//! tool would run against. One fewer native dependency, and the commands in the
//! tombstone index (§9.4) are literally the commands we run.

use std::ffi::{OsStr, OsString};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use crate::error::{Error, Result};

/// Environment variables that redirect git at a different repository.
///
/// Every git hook runs with `GIT_DIR` set, and so does anything invoked from a
/// `git` subprocess. If Judged inherits one of these it will answer questions
/// about path X using repository Y — a wrong recoverability class produced with
/// total confidence, which is precisely the failure mode §8.1 warns about. The
/// child never sees them.
const INHERITED_GIT_ENV: [&str; 6] = [
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_COMMON_DIR",
    "GIT_NAMESPACE",
];

/// Identity used for the commits Judged makes itself (E2 fixtures, §10). The
/// `.invalid` TLD is reserved by RFC 2606 and can never route mail.
const COMMIT_IDENTITY_NAME: &str = "Judged";
const COMMIT_IDENTITY_EMAIL: &str = "judged@judged.invalid";

/// A finished `git` invocation, kept together with the command that produced it
/// so that every error message can name the exact thing that failed (AGENTS.md
/// rule 12, "Fail Loudly": errors are actionable or they are noise).
struct GitRun {
    args: Vec<OsString>,
    dir: PathBuf,
    output: Output,
}

impl GitRun {
    /// Run `git <args>` in `dir`, optionally feeding `stdin`.
    ///
    /// A non-zero exit is **not** an error here. Half of git's plumbing answers
    /// questions through the exit code — `ls-files --error-unmatch` exits 1 for
    /// "not tracked", `check-ignore` exits 1 for "nothing was ignored" — so
    /// each caller decides which codes are answers and which are failures.
    fn new<I, S>(dir: &Path, args: I, stdin: Option<&[u8]>) -> Result<GitRun>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let args: Vec<OsString> = args
            .into_iter()
            .map(|a| a.as_ref().to_os_string())
            .collect();
        let mut cmd = Command::new("git");
        cmd.args(&args)
            .current_dir(dir)
            .stdin(if stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for var in INHERITED_GIT_ENV {
            cmd.env_remove(var);
        }
        // A cleaner that blocks on a credential prompt mid-scan is
        // indistinguishable from one that has crashed.
        cmd.env("GIT_TERMINAL_PROMPT", "0");

        let mut child = cmd.spawn().map_err(|source| {
            Error::Git(format!(
                "failed to run `git` in {}: {source}",
                dir.display()
            ))
        })?;
        if let Some(bytes) = stdin {
            let mut pipe = child
                .stdin
                .take()
                .ok_or_else(|| Error::Git("git stdin pipe was not created".to_string()))?;
            // Every payload written here is a single path, far below the pipe
            // buffer, so writing before waiting cannot deadlock.
            pipe.write_all(bytes)
                .map_err(|source| Error::Git(format!("failed to write to git stdin: {source}")))?;
            // Dropping closes the pipe; `--stdin` readers wait for EOF.
        }
        let output = child
            .wait_with_output()
            .map_err(|source| Error::Git(format!("failed to wait for git: {source}")))?;
        Ok(GitRun {
            args,
            dir: dir.to_path_buf(),
            output,
        })
    }

    fn success(&self) -> bool {
        self.output.status.success()
    }

    /// Exit status, or `None` when git was killed by a signal.
    fn code(&self) -> Option<i32> {
        self.output.status.code()
    }

    /// The failure, described well enough to reproduce by hand.
    fn failure(&self) -> Error {
        let rendered: Vec<String> = self
            .args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        Error::Git(format!(
            "`git {}` in {} exited with {}: {}",
            rendered.join(" "),
            self.dir.display(),
            match self.code() {
                Some(code) => code.to_string(),
                None => "a signal".to_string(),
            },
            String::from_utf8_lossy(&self.output.stderr).trim()
        ))
    }

    fn require_success(self) -> Result<GitRun> {
        if self.success() {
            Ok(self)
        } else {
            Err(self.failure())
        }
    }

    fn stdout_str(&self) -> Result<&str> {
        std::str::from_utf8(&self.output.stdout)
            .map_err(|source| Error::Git(format!("git printed non-UTF-8 output: {source}")))
    }

    fn stdout_trimmed(&self) -> Result<&str> {
        self.stdout_str().map(str::trim)
    }
}

/// How much of a path git could give back after we delete it.
///
/// **This enum is Gate 0g, and it encodes the single most consequential finding
/// in the research (§8.1, and the READ-THIS-FIRST box above §0).**
///
/// Git protects the object database, not the working tree. A file that was
/// never `git add`-ed leaves *nothing* behind when deleted — no blob, no reflog
/// entry, no `lost-found`. So the intuitive risk ordering is exactly backwards:
///
/// - `TrackedPushed` / `TrackedUnpushed` files are **source**, so misclassifying
///   them is a behavioural change — but deleting one is recoverable with
///   `git checkout <sha>^ -- <path>`.
/// - `Untracked` and `Ignored` files are safe to *classify* (they are not
///   source) and unsafe to *delete* (uncommitted human work, `.env`, dev
///   SQLite databases, `terraform.tfstate.backup`, patched `node_modules`).
///
/// The highest-volume, most tempting targets of any repo cleaner — build
/// output, caches, logs, scratch files — are precisely the files git cannot
/// restore. "Gitignored" is *positively* correlated with irrecoverability
/// (§6.17: only 5.9% of github/gitignore patterns are confidently regenerable).
///
/// §8.2 gives the escape hatch: `git add` is a one-command promotion from the
/// bottom rung to a recoverable one. **If an implementation ever auto-deletes an
/// `Untracked` or `Ignored` path without first promoting its rung, it has
/// reintroduced the exact defect this project exists to prevent.**
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoverabilityClass {
    /// In the index and present on a remote branch. Fully recoverable.
    TrackedPushed,
    /// Committed locally but not pushed. Recoverable until the local clone is.
    TrackedUnpushed,
    /// Not in the index and not ignored. **Zero recovery path.**
    Untracked,
    /// Matched by an ignore rule. **Zero recovery path**, and the class most
    /// likely to hold irreplaceable local state.
    Ignored,
}

/// A git working tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repo {
    root: PathBuf,
}

impl Repo {
    /// Walk up from `start` to the enclosing working tree.
    ///
    /// Runs `git rev-parse --show-toplevel`. Fails when `start` is not inside a
    /// working tree — including inside a bare repository, which prints nothing.
    /// "Not a repository" must never be mistaken for "a repository with nothing
    /// in it" — §6.20's rule that `"no data" must be a distinct state from
    /// "zero executions"`, applied to the substrate the classifier runs on.
    pub fn discover(start: &Path) -> Result<Repo> {
        let run = GitRun::new(start, ["rev-parse", "--show-toplevel"], None)?.require_success()?;
        let toplevel = run.stdout_trimmed()?;
        if toplevel.is_empty() {
            return Err(Error::Git(format!(
                "{} has no working tree (bare repository?)",
                start.display()
            )));
        }
        // Resolve symlinks once, here, so that every later `strip_prefix`
        // compares like with like: on macOS the same directory is reachable as
        // both /tmp/x and /private/tmp/x, and Gate 0c (§9.3) is a real-path
        // containment check.
        let root = std::fs::canonicalize(toplevel).map_err(|source| Error::Io {
            path: PathBuf::from(toplevel),
            source,
        })?;
        Ok(Repo { root })
    }

    /// Create a new repository at `dir`. Used to build E2 mutant fixtures
    /// (§10), which must be real repositories because recoverability class is
    /// part of what the suite exercises.
    ///
    /// `git init`, with `init.defaultBranch` pinned so that fixtures do not
    /// inherit the branch name (or the version-dependent hint) of whoever runs
    /// the suite. Creates `dir` if it does not exist.
    pub fn init(dir: &Path) -> Result<Repo> {
        std::fs::create_dir_all(dir).map_err(|source| Error::Io {
            path: dir.to_path_buf(),
            source,
        })?;
        GitRun::new(
            dir,
            ["-c", "init.defaultBranch=main", "init", "--quiet"],
            None,
        )?
        .require_success()?;
        Repo::discover(dir)
    }

    /// Absolute path to the working tree root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Blob SHA of `path` at `HEAD`, or `None` when the path is not in `HEAD`.
    ///
    /// This is the content identity that invalidates cached evidence (§9.4
    /// `subject_blob_sha`) and one of the inputs to a fingerprint.
    ///
    /// Runs `git ls-tree -z HEAD -- :(literal)<path>` rather than the shorter
    /// `git rev-parse HEAD:<path>`, because `rev-parse` happily returns the
    /// *tree* SHA of a directory and the *commit* SHA of a submodule gitlink.
    /// Either would hand a fingerprint an identity that can never match a blob,
    /// and neither can be told apart from a real blob SHA after the fact.
    /// `ls-tree` names the object type, so non-blobs become `None`.
    pub fn blob_sha(&self, path: &Path) -> Result<Option<String>> {
        let rel = self.relative(path)?;
        let run = GitRun::new(
            &self.root,
            [
                OsString::from("ls-tree"),
                OsString::from("-z"),
                OsString::from("HEAD"),
                OsString::from("--"),
                literal_pathspec(&rel),
            ],
            None,
        )?;
        if !run.success() {
            // A repository with no commits has no HEAD to read. That is a
            // normal state for a freshly initialised fixture (§10), not a
            // failure — but any *other* non-zero exit is.
            if self.head_exists()? {
                return Err(run.failure());
            }
            return Ok(None);
        }
        // One NUL-terminated record per matching entry, each
        // `<mode> SP <type> SP <object> TAB <path>`. No output at all means the
        // path is not in HEAD.
        let stdout = run.stdout_str()?;
        let Some(record) = stdout.split('\0').find(|r| !r.is_empty()) else {
            return Ok(None);
        };
        let Some((kind, oid)) = parse_ls_tree_record(record) else {
            return Err(Error::Git(format!(
                "unparseable `git ls-tree` record for {}: {record:?}",
                rel.display()
            )));
        };
        // A directory (`tree`) or a submodule gitlink (`commit`) is not content
        // we can fingerprint. §9.3 Gate 0b refuses to descend into nested repos
        // anyway; this makes the refusal impossible to bypass by accident.
        if kind != "blob" {
            return Ok(None);
        }
        Ok(Some(oid.to_string()))
    }

    /// Whether `HEAD` resolves to a commit (i.e. the repository has history).
    fn head_exists(&self) -> Result<bool> {
        Ok(GitRun::new(
            &self.root,
            ["rev-parse", "--verify", "--quiet", "HEAD"],
            None,
        )?
        .success())
    }

    /// Whether `path` is in the index.
    ///
    /// Runs `git ls-files --error-unmatch -- :(literal)<path>`: exit 0 means
    /// tracked, exit 1 means "no such path in the index", and anything else is
    /// a real failure that must not be silently read as "not tracked".
    pub fn is_tracked(&self, path: &Path) -> Result<bool> {
        self.is_tracked_rel(&self.relative(path)?)
    }

    fn is_tracked_rel(&self, rel: &Path) -> Result<bool> {
        let run = GitRun::new(
            &self.root,
            [
                OsString::from("ls-files"),
                OsString::from("--error-unmatch"),
                OsString::from("-z"),
                OsString::from("--"),
                literal_pathspec(rel),
            ],
            None,
        )?;
        match run.code() {
            Some(0) => Ok(true),
            Some(1) => Ok(false),
            _ => Err(run.failure()),
        }
    }

    /// Whether an ignore rule's *last* match on `rel` excludes it.
    ///
    /// Runs `git check-ignore -vz --stdin --non-matching` (§9.3 Gate 0g) and
    /// reads the reported pattern rather than the exit code, because a pattern
    /// re-included by a `!` negation is reported *as a match* while not being
    /// ignored. §6.16/§6.17: ignore status belongs to a file, never to the
    /// directory above it — Magento's `/media/*` plus `!/media/customer` plus
    /// `!/media/customer/.htaccess` is the canonical shape, and a
    /// directory-level answer there deletes a checked-in `.htaccess`.
    fn is_ignored_rel(&self, rel: &Path) -> Result<bool> {
        // `-z` makes both directions NUL-delimited, so no filename can be
        // misread no matter what it contains.
        let path = rel.to_str().ok_or_else(|| {
            Error::Git(format!(
                "path {} is not valid UTF-8; refusing to guess its ignore status",
                rel.display()
            ))
        })?;
        let mut stdin = path.as_bytes().to_vec();
        stdin.push(0);
        let run = GitRun::new(
            &self.root,
            ["check-ignore", "-vz", "--stdin", "--non-matching"],
            Some(&stdin),
        )?;
        // Exit 1 means "none of the given paths are ignored" — an answer, not a
        // failure. Anything above that (128: outside the repository) is real.
        match run.code() {
            Some(0) | Some(1) => {}
            _ => return Err(run.failure()),
        }
        // Records are four NUL-terminated fields: source, line, pattern, path.
        let mut fields = run.output.stdout.split(|b| *b == 0);
        let pattern = fields.nth(2).ok_or_else(|| {
            Error::Git(format!(
                "git check-ignore returned no verdict for {}",
                rel.display()
            ))
        })?;
        // No pattern matched at all => not ignored. A matched pattern starting
        // with `!` is a negation, which re-includes the file => not ignored.
        Ok(!pattern.is_empty() && pattern[0] != b'!')
    }

    /// Whether `HEAD` is contained by any remote-tracking ref.
    ///
    /// `git for-each-ref --contains HEAD refs/remotes/`. Note this consults the
    /// *local cache* of the remote's state: a stale `refs/remotes/**` can claim
    /// a commit is published after someone force-pushed it away. It is the
    /// definition §9.3 Gate 0g gives, and it errs toward the answer a `git
    /// fetch` would confirm.
    fn head_is_on_a_remote(&self) -> Result<bool> {
        // An unborn HEAD (fresh repo, nothing committed) makes `--contains`
        // fail outright, and is trivially not on any remote.
        if !self.head_exists()? {
            return Ok(false);
        }
        let run = GitRun::new(
            &self.root,
            [
                "for-each-ref",
                "--format=%(refname)",
                "--contains",
                "HEAD",
                "refs/remotes/",
            ],
            None,
        )?
        .require_success()?;
        Ok(!run.stdout_trimmed()?.is_empty())
    }

    /// Resolve `path` to a path relative to the working tree root.
    fn relative(&self, path: &Path) -> Result<PathBuf> {
        let absolute = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.root.join(path)
        };
        // Gate 0c (§9.3) is a *real*-path containment check, so resolve
        // symlinks when we can. `canonicalize` fails on paths that do not
        // exist, and "classify a file that is already gone" is a legitimate
        // question, so fall back to the unresolved path and let git refuse
        // anything that escapes the working tree.
        let resolved = std::fs::canonicalize(&absolute).unwrap_or(absolute);
        let rel = resolved.strip_prefix(&self.root).map_err(|_| {
            Error::Git(format!(
                "{} is outside the working tree {}",
                path.display(),
                self.root.display()
            ))
        })?;
        // The root reduces to the empty path, and `git ls-files -- :(literal)`
        // with an empty path matches *every tracked file* (verified on git
        // 2.50.1) — so an unguarded root would classify as tracked, i.e. as if
        // deleting the whole working tree were recoverable. Refuse instead.
        if rel.as_os_str().is_empty() {
            return Err(Error::Git(format!(
                "{} is the working tree root, not a candidate path",
                self.root.display()
            )));
        }
        Ok(rel.to_path_buf())
    }

    /// Whether this clone is shallow or has a partial object filter.
    ///
    /// A shallow clone cannot answer "was this ever used" over history, so
    /// history-derived evidence must abstain rather than accuse. §6.19: shallow
    /// is the CI default (`actions/checkout` fetches depth 1), which is exactly
    /// where an automated cleaner is most likely to run, and the missing history
    /// is silent — every query succeeds and simply reports less.
    ///
    /// `git rev-parse --is-shallow-repository` covers `.git/shallow`; a
    /// *partial* clone (`--filter=blob:none`) keeps the commits but not the
    /// blobs, so content-level history questions are equally unanswerable
    /// without refetching. Both trip the same abstention.
    pub fn is_shallow(&self) -> Result<bool> {
        let run = GitRun::new(&self.root, ["rev-parse", "--is-shallow-repository"], None)?
            .require_success()?;
        if run.stdout_trimmed()? == "true" {
            return Ok(true);
        }
        // `remote.<name>.promisor=true` is the marker git writes when a clone
        // is served by a promisor remote, i.e. when objects may be absent.
        let promisor = GitRun::new(
            &self.root,
            [
                "config",
                "--get-regexp",
                "--type=bool",
                r"^remote\..*\.promisor$",
            ],
            None,
        )?;
        match promisor.code() {
            // Exit 1 is "no such key" — an answer.
            Some(1) => Ok(false),
            Some(0) => Ok(promisor
                .stdout_trimmed()?
                .lines()
                .any(|line| line.split_whitespace().nth(1) == Some("true"))),
            _ => Err(promisor.failure()),
        }
    }

    /// The repository's **common** git directory, absolute.
    ///
    /// `git rev-parse --git-common-dir`, which is the only portable answer to
    /// "where does this repository actually keep its objects". Joining `.git`
    /// onto the working tree root is wrong in three ordinary layouts, and they
    /// all fail the same way: **`.git` is a regular file holding `gitdir: …`,
    /// not a directory.**
    ///
    /// - A **linked worktree** — verified, `git worktree add` writes a 63-byte
    ///   file — with the real directory under `<common>/worktrees/<name>`.
    /// - A **submodule**, with the real directory under
    ///   `<super>/.git/modules/<name>`.
    /// - **`git init --separate-git-dir`**, which writes an 89-byte file
    ///   pointing anywhere on the filesystem. (An earlier version of this doc
    ///   claimed there was no `.git` entry at all in this case. Verified false:
    ///   the file is there, and it is a file, which is exactly why the naive
    ///   join fails.)
    ///
    /// Relative output is joined onto the working tree root — a plain
    /// repository prints `.git` rather than an absolute path.
    ///
    /// Errors rather than guessing. A caller that cannot locate the git
    /// directory must record a gap, never conclude that what lives inside it is
    /// absent (§6.20).
    pub fn common_dir(&self) -> Result<PathBuf> {
        let run =
            GitRun::new(&self.root, ["rev-parse", "--git-common-dir"], None)?.require_success()?;
        let printed = run.stdout_trimmed()?;
        let path = Path::new(&printed);
        Ok(if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.root.join(path)
        })
    }

    /// Classify `path` for Gate 0g. See [`RecoverabilityClass`].
    ///
    /// Exactly one class, decided in this order: index membership first
    /// (`git ls-files`), then — only for untracked paths — ignore status
    /// (`git check-ignore`), then, for tracked paths, whether `HEAD` is
    /// published (`git for-each-ref --contains`).
    ///
    /// The class describes what git holds **at HEAD**, not what is on disk: a
    /// tracked file with uncommitted modifications is `TrackedPushed` while its
    /// working-tree content exists nowhere in the object database. That gap is
    /// Gate 0d's job (§9.3: refuse to auto-act while tracked files are dirty),
    /// and this function deliberately does not paper over it.
    ///
    /// Costs two or three `git` invocations per path, one of which walks
    /// history. That is deliberate for now — a wrong answer here is
    /// unrecoverable and a slow one is not — but a whole-tree scan should grow
    /// a batched entry point (`ls-files` and `check-ignore --stdin` both accept
    /// many paths at once) rather than calling this in a loop.
    pub fn recoverability(&self, path: &Path) -> Result<RecoverabilityClass> {
        let rel = self.relative(path)?;
        if self.is_tracked_rel(&rel)? {
            // §8.1: tracked content is restorable; the only question left is
            // whether it survives the loss of this clone.
            if self.head_is_on_a_remote()? {
                return Ok(RecoverabilityClass::TrackedPushed);
            }
            return Ok(RecoverabilityClass::TrackedUnpushed);
        }
        if self.is_ignored_rel(&rel)? {
            return Ok(RecoverabilityClass::Ignored);
        }
        Ok(RecoverabilityClass::Untracked)
    }

    /// Stage everything in the working tree. Fixture construction only.
    ///
    /// `git add --all`. Deliberately **not** `-f`: §8.2 makes `git add -f` a
    /// rung promotion (R9 to R6) that only a caller who means it may perform,
    /// and a fixture whose ignored files were silently staged would model a
    /// repository safer than the one it is meant to represent.
    pub fn add_all(&self) -> Result<()> {
        GitRun::new(&self.root, ["add", "--all"], None)?.require_success()?;
        Ok(())
    }

    /// Commit the index with `message`. Fixture construction only.
    ///
    /// Carries its own identity (`-c user.name`/`-c user.email`) because CI
    /// images configure none and `git commit` would otherwise die; skips hooks
    /// and signing because a fixture commit must not execute the ambient user's
    /// `core.hooksPath` scripts or block on a GPG passphrase prompt.
    pub fn commit(&self, message: &str) -> Result<()> {
        GitRun::new(
            &self.root,
            [
                "-c",
                &format!("user.name={COMMIT_IDENTITY_NAME}"),
                "-c",
                &format!("user.email={COMMIT_IDENTITY_EMAIL}"),
                "commit",
                "--quiet",
                "--no-gpg-sign",
                "--no-verify",
                "-m",
                message,
            ],
            None,
        )?
        .require_success()?;
        Ok(())
    }
}

/// Split one `git ls-tree` record into `(object type, object id)`.
///
/// The record is `<mode> SP <type> SP <object> TAB <path>`; the path is
/// discarded because we asked about exactly one path and already know it.
fn parse_ls_tree_record(record: &str) -> Option<(&str, &str)> {
    let (meta, _path) = record.split_once('\t')?;
    let mut fields = meta.split_whitespace();
    let _mode = fields.next()?;
    let kind = fields.next()?;
    let oid = fields.next()?;
    Some((kind, oid))
}

/// Wrap `rel` as a literal pathspec.
///
/// Without `:(literal)`, git reads `report[1].txt` as a glob and can match a
/// *different* file — which in this crate means answering a recoverability
/// question about the wrong path.
fn literal_pathspec(rel: &Path) -> OsString {
    let mut spec = OsString::from(":(literal)");
    spec.push(rel.as_os_str());
    spec
}
