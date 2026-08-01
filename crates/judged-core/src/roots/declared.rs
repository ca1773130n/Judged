//! Tier C — the roots nothing can discover, so a human has to say them out loud.
//!
//! §5.1 is blunt about it: **no amount of static cleverness moves a Tier C root
//! into A or B.** The live set is decided by data or intent that is not in the
//! repository at all — class names in a `type` column, a Sidekiq queue holding
//! serialized classes, a feature flag currently off but not retired, an ops
//! runbook, a downstream consumer of a published library, a customer-authored
//! plugin, a deployment environment telemetry never sees. The product move is
//! not a cleverer parser. It is to **ask**, and to record the answer somewhere
//! it gets reviewed.
//!
//! This module is that record. It reads [`ROOTS_FILE`], materializes what it
//! declares, and — the part §5.3 says nothing in the survey has — reports what
//! the ruleset says about *itself*.
//!
//! # Correction before mute
//!
//! §5.3's design principle, and the first question to ask about every entry
//! here: **can this be a CORRECTION instead of a MUTE?** cargo-machete's
//! `renamed` map is the exemplar — it teaches the tool the real name, so the
//! whole class of mismatch stops being wrong. A mute only stops *this* finding
//! being reported, and it stops it forever, including once the code underneath
//! has changed into something that really is dead. A correction improves
//! precision permanently; a mute creates a blind spot permanently. Reach for
//! `.judged/roots.toml` when there is genuinely nothing to teach — which for
//! Tier C, by definition, there is not.
//!
//! # Why this shape, and not a new DSL
//!
//! §5.3 compares five designs that already exist and recommends a hybrid rather
//! than a sixth. Four of its five points land here:
//!
//! - **Location** — manifest-colocated and committed, cargo-machete's insight.
//!   `.judged/roots.toml` is reviewed in the same pull request as the code it
//!   protects. A keep-list in `~/.config` is a keep-list nobody reads.
//! - **Matching** — gitignore pathspec semantics, so nobody has to learn
//!   anything: `*` stops at a separator, `**` does not, a pattern without a
//!   slash matches at any depth, a leading `/` anchors, naming a directory
//!   declares its contents, and a later `!` carves a hole in an earlier
//!   pattern.
//! - **Suppression semantics** — SARIF's `suppressions` object:
//!   [`SuppressionKind`] and [`SuppressionStatus`], with `reason` as its
//!   mandatory `justification`.
//! - **Self-linting** — [`DeclaredRoots::lint`]. *A suppression list without rot
//!   detection is the off switch.*
//!
//! The fifth, GraalVM's conditional `keep X when reachable(Y)`, is the one §5.3
//! calls the best idea in the survey, and is deliberately **not** here: it needs
//! a reachability model to evaluate the guard against, and this crate has none.
//! Expiry is the poor relation that works without one — a deadline instead of a
//! guard.
//!
//! # What `rejected` means
//!
//! SARIF §3.35: a *rejected* suppression is not in effect. So a rejected entry
//! protects nothing, and stays in the file as the durable record that somebody
//! looked at this candidate and decided it was not a root. That third state is
//! what the binary keep-lists in ProGuard, Periphery and Vulture cannot express
//! — in those, a turned-down exemption is simply deleted, and the next person to
//! meet the same finding re-litigates it from scratch.
//!
//! §5.3's prose glosses `rejected` as "a human examined this candidate and said
//! the tool was wrong". Read that way, `rejected` and `accepted` would both mean
//! keep, leaving two spellings for one behaviour. This module follows SARIF's
//! normative meaning instead, because §5.3's own instruction is to take the
//! object *verbatim*, and because three distinct states are the thing being
//! bought.
//!
//! # Expiry is asked for, never re-implemented
//!
//! [`DeclaredRoots::lint`] takes the expiry predicate as an argument. That is
//! not indirection for its own sake: `judged-ratchet` already owns this exact
//! rule in `judged_ratchet::rot::has_expired`, including the decision that a
//! date it cannot evaluate counts as expired — and `judged-ratchet` depends on
//! `judged-core`, so naming it back from here is a package cycle cargo refuses
//! outright (`error: cyclic package dependency: package judged-core depends on
//! itself`). A second copy could silently disagree with the first, which is the
//! failure `has_expired` was made public to end. So the definition stays in one
//! place and callers hand it over; `judged-cli` depends on both crates and is
//! where the two meet.

use std::fmt;
use std::path::Path;

/// Where declared roots live: committed, and next to the code they protect.
///
/// §5.3 point 4. The location is part of the design, not a default — an ignore
/// file outside version control is reviewed by nobody and expires by accident.
pub const ROOTS_FILE: &str = ".judged/roots.toml";

/// Which of §5.1's three tiers a root came from.
///
/// Recorded on every root because a root that does not say where it came from
/// is worse than no root: it invites a caller to trust a guessed framework
/// convention as though a manifest had declared it, and the three tiers do not
/// deserve equal trust. Everything this module produces is [`Tier::C`] by
/// construction.
///
/// This belongs in `roots::mod`, next to the prose defining the tiers, so that
/// [`manifest`](super::manifest) and [`convention`](super::convention) name the
/// same type. It is here only because that module is complete and not this
/// task's to edit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Tier {
    /// Machine-declared: a build system or deploy target already reads the file
    /// this root came from.
    A,
    /// Convention-inferable: a framework's layout or annotations make a file an
    /// entry point with no source reference. Correct only if the framework *and
    /// its version* were detected correctly.
    B,
    /// Undiscoverable: solicited from a human and recorded here. Confidence in
    /// the derivation is none — it is exactly as good as the person who wrote
    /// it down.
    C,
}

impl fmt::Display for Tier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Tier::A => "A",
            Tier::B => "B",
            Tier::C => "C",
        })
    }
}

/// SARIF `suppression.kind`, verbatim.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuppressionKind {
    /// The suppression is expressed in the code itself — a `// judged:ignore`
    /// style directive. An entry here carrying this kind is the manifest
    /// indexing something that lives in a source comment, which §5.3 notes is
    /// invisible in rendered output and rots silently.
    InSource,
    /// The suppression is expressed outside the code, which is what a committed
    /// `.judged/roots.toml` entry normally is.
    External,
}

impl fmt::Display for SuppressionKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            SuppressionKind::InSource => "inSource",
            SuppressionKind::External => "external",
        })
    }
}

/// SARIF `suppression.status`, verbatim. See the module docs on `rejected`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuppressionStatus {
    /// Reviewed and agreed: this is a root.
    Accepted,
    /// Proposed, not yet reviewed. Still protects, because the safe direction
    /// while a human decides is to keep — treating "undecided" as "not a root"
    /// deletes the thing the question was about.
    UnderReview,
    /// Reviewed and turned down: not a root, and the entry remains as the
    /// record of that decision.
    Rejected,
}

impl fmt::Display for SuppressionStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            SuppressionStatus::Accepted => "accepted",
            SuppressionStatus::UnderReview => "underReview",
            SuppressionStatus::Rejected => "rejected",
        })
    }
}

/// `.judged/roots.toml` did not parse, and where.
///
/// Every parse failure is fatal rather than skipped. A dropped entry is a
/// declared root that silently stops protecting anything, and the whole reason
/// this file exists is that nothing else can find what it names (AGENTS.md rule
/// 12, "Fail Loudly").
///
/// This is a module-local type rather than a [`crate::Error`] variant only
/// because `error.rs` is not this task's to edit; it belongs there, as that
/// module's own docs argue — one variant per fallible boundary, spelled out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MalformedRoots {
    /// 1-based line in [`ROOTS_FILE`], so the message can be clicked.
    pub line: usize,
    /// What is wrong, phrased as something to do about it.
    pub message: String,
}

impl fmt::Display for MalformedRoots {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}: {}", ROOTS_FILE, self.line, self.message)
    }
}

impl std::error::Error for MalformedRoots {}

fn bad(line: usize, message: impl Into<String>) -> MalformedRoots {
    MalformedRoots {
        line,
        message: message.into(),
    }
}

/// One `[[root]]` entry: a human's claim that something is live.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclaredRoot {
    /// A gitignore pathspec. A leading `!` negates it.
    pub pathspec: String,
    /// SARIF's `justification`, and §5.3's first addition: **mandatory**. An
    /// entry without one is unreviewable — the next person cannot tell a live
    /// constraint from an abandoned workaround, so nobody ever dares remove it.
    pub reason: String,
    /// SARIF `suppression.kind`.
    pub kind: SuppressionKind,
    /// SARIF `suppression.status`.
    pub status: SuppressionStatus,
    /// §5.3's optional `expires: YYYY-MM-DD`. Deliberately not validated here:
    /// the shape rule belongs to the one expiry definition this module borrows,
    /// which reads an unevaluable date as expired and carries the raw text into
    /// the report, rather than granting a longer amnesty than the author asked
    /// for.
    pub expires: Option<String>,
    /// The `[[root]]` header's line in [`ROOTS_FILE`] — where a reader goes to
    /// argue with this declaration.
    pub line: usize,
}

/// A root this file put into the root set, with its provenance attached.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seed {
    /// Repo-relative path, as it appeared in the candidate set.
    pub path: String,
    /// Always [`Tier::C`] from this module. See [`Tier`] for why it is carried
    /// rather than implied.
    pub tier: Tier,
    /// The pathspec that put it here.
    pub pathspec: String,
    /// The declaring entry's mandatory justification.
    pub reason: String,
    /// The declaring entry's SARIF kind.
    pub kind: SuppressionKind,
    /// The declaring entry's SARIF status — `accepted` or `underReview`, never
    /// `rejected`, since a rejected suppression is not in effect.
    pub status: SuppressionStatus,
    /// Line of the `[[root]]` header that declared it.
    pub declared_at_line: usize,
}

/// Why an entry has stopped earning its place in the file.
///
/// Every variant carries what a human needs in order to act without re-running
/// anything, for the same reason `judged_ratchet::rot::RotReason` does: the cost
/// of pruning is what decides whether pruning happens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RootRot {
    /// The pathspec names one exact path and there is no such file. The
    /// strongest statement available — it can never match again whatever the
    /// analyzers do. Vulture's executable-whitelist property, generalized: the
    /// list is checked against reality, not merely against this run's findings.
    ReferentGone {
        /// Line of the `[[root]]` header.
        line: usize,
        /// The pathspec as written.
        pathspec: String,
    },
    /// A deadline a human set has passed — or is a date nobody can evaluate,
    /// which the borrowed expiry rule treats the same way.
    Expired {
        /// Line of the `[[root]]` header.
        line: usize,
        /// The pathspec as written.
        pathspec: String,
        /// The `expires` value verbatim, so an author can see what they typed.
        expires: String,
    },
    /// Nothing in this run was decided by this entry — it matched no candidate,
    /// or every candidate it matched was decided by a later entry. Periphery's
    /// superfluous-ignore warning, generalized. A pattern protecting nothing is
    /// a blind spot nobody is watching.
    MatchedNothing {
        /// Line of the `[[root]]` header.
        line: usize,
        /// The pathspec as written.
        pathspec: String,
    },
}

impl RootRot {
    /// The line to open, shared by every variant.
    pub fn line(&self) -> usize {
        match self {
            RootRot::ReferentGone { line, .. }
            | RootRot::Expired { line, .. }
            | RootRot::MatchedNothing { line, .. } => *line,
        }
    }
}

impl fmt::Display for RootRot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}: ", ROOTS_FILE, self.line())?;
        match self {
            RootRot::ReferentGone { pathspec, .. } => write!(
                f,
                "`{pathspec}` names a path that does not exist; it can never match again"
            ),
            RootRot::Expired {
                pathspec, expires, ..
            } => write!(
                f,
                "`{pathspec}` expired at `{expires}`; renew it with a fresh reason or delete it"
            ),
            RootRot::MatchedNothing { pathspec, .. } => write!(
                f,
                "`{pathspec}` decided nothing in this run; it protects a blind spot"
            ),
        }
    }
}

/// The parsed contents of [`ROOTS_FILE`].
#[derive(Debug, Clone, Default)]
pub struct DeclaredRoots {
    entries: Vec<DeclaredRoot>,
    /// Compiled in step with `entries`; index `i` belongs to `entries[i]`.
    patterns: Vec<Pattern>,
}

impl DeclaredRoots {
    /// Parse the text of a `.judged/roots.toml`.
    ///
    /// Empty input is a valid, empty ruleset: a repository with no Tier C roots
    /// is a normal repository, so an absent file is `parse("")` and not an
    /// error. Reading the bytes is the caller's job — it already owns
    /// [`crate::Error::Io`], and a second variant here saying the same thing
    /// would be the sprawl this workspace's single error type exists to avoid.
    ///
    /// # The accepted subset
    ///
    /// A deliberately small slice of TOML: `[[root]]` headers, `key = "…"`
    /// pairs whose values are double-quoted basic strings, `#` comments, blank
    /// lines. Anything outside it — a bare value, a table that is not
    /// `[[root]]`, a repeated or misspelled key — is an error rather than a
    /// shrug, because each of those silently drops an entry or a mandatory
    /// field, and a dropped Tier C root is a deletion nobody vetoed.
    pub fn parse(text: &str) -> Result<DeclaredRoots, MalformedRoots> {
        let mut entries: Vec<DeclaredRoot> = Vec::new();
        let mut draft: Option<Draft> = None;

        for (index, raw) in text.lines().enumerate() {
            let line = index + 1;
            let content = strip_comment(raw).trim();
            if content.is_empty() {
                continue;
            }

            if content.starts_with('[') {
                if content != "[[root]]" {
                    return Err(bad(
                        line,
                        format!("only `[[root]]` tables belong here, found `{content}`"),
                    ));
                }
                if let Some(previous) = draft.take() {
                    entries.push(previous.finish()?);
                }
                draft = Some(Draft::new(line));
                continue;
            }

            let Some(open) = draft.as_mut() else {
                return Err(bad(
                    line,
                    "a key outside any table; every entry must begin with `[[root]]`",
                ));
            };
            let (key, value) = split_key_value(content, line)?;
            open.set(key, value, line)?;
        }

        if let Some(last) = draft.take() {
            entries.push(last.finish()?);
        }

        let patterns = entries
            .iter()
            .map(|entry| Pattern::compile(&entry.pathspec))
            .collect();
        Ok(DeclaredRoots { entries, patterns })
    }

    /// Every declaration in the file, in file order — including the rejected
    /// ones, which protect nothing but are the record of a decision.
    pub fn entries(&self) -> &[DeclaredRoot] {
        &self.entries
    }

    /// The index of the entry that *decides* `path`: the last one whose
    /// pathspec matches, which is gitignore's last-match-wins.
    fn deciding(&self, path: &str) -> Option<usize> {
        self.patterns
            .iter()
            .enumerate()
            .rev()
            .find(|(_, pattern)| pattern.matches(path))
            .map(|(index, _)| index)
    }

    /// Turn declarations into roots, against the paths this run is considering.
    ///
    /// A candidate becomes a [`Seed`] when the entry deciding it is a positive
    /// pattern that was not rejected. The two ways an entry can decide *not* to
    /// protect are different statements, and both are needed: a leading `!` says
    /// "the pattern above does not reach here", a correction to scope, while
    /// `status = "rejected"` says "this declaration was reviewed and turned
    /// down", a verdict on the claim. Where both apply the `!` wins, because
    /// coverage is settled before the verdict on the covering entry is
    /// consulted.
    ///
    /// Output is sorted by path and deduplicated, so a report diffed between two
    /// runs shows changes rather than reordering.
    pub fn materialize<S: AsRef<str>>(&self, candidates: &[S]) -> Vec<Seed> {
        let mut seeds: Vec<Seed> = Vec::new();
        for candidate in candidates {
            let path = candidate.as_ref();
            let Some(index) = self.deciding(path) else {
                continue;
            };
            if self.patterns[index].negated {
                continue;
            }
            let entry = &self.entries[index];
            if entry.status == SuppressionStatus::Rejected {
                continue;
            }
            seeds.push(Seed {
                path: path.to_string(),
                tier: Tier::C,
                pathspec: entry.pathspec.clone(),
                reason: entry.reason.clone(),
                kind: entry.kind,
                status: entry.status,
                declared_at_line: entry.line,
            });
        }
        seeds.sort_by(|a, b| a.path.cmp(&b.path));
        seeds.dedup_by(|a, b| a.path == b.path);
        seeds
    }

    /// Lint the ruleset against itself — §5.3's second addition, and the thing
    /// it says nothing in the survey has.
    ///
    /// *A suppression list without rot detection is the off switch.* Every entry
    /// here suppresses a finding forever; without this the file only ever grows,
    /// and a repository's dead code slowly migrates into it.
    ///
    /// `now` is supplied rather than read from the clock, so that a CI run and a
    /// local run over the same inputs agree. `has_expired` is supplied for the
    /// reason in the module docs — pass `judged_ratchet::rot::has_expired`.
    ///
    /// At most **one** reason per entry, because the remediation for all three
    /// is "delete the line", and three reasons for one line is how a rot report
    /// becomes something people skim past. Precedence, most-specific first:
    ///
    /// 1. [`RootRot::ReferentGone`] — the path is not there, so nothing about
    ///    this run can change the verdict, and it necessarily implies the empty
    ///    match as well.
    /// 2. [`RootRot::Expired`] — a deadline a human set has passed. A stronger
    ///    statement than a pattern that happens not to fire today.
    /// 3. [`RootRot::MatchedNothing`] — nothing in this run was decided by it.
    ///
    /// Deliberately the same ordering, and the same argument, as
    /// `judged_ratchet::rot::detect_rot`, so a reader who has met one meets no
    /// surprises in the other.
    ///
    /// `repo_root` is what the repo-relative pathspecs resolve against — the
    /// same directory the candidate paths are relative to. `judged_ratchet`
    /// takes a `Repo` for this and uses only its root; a `&Path` asks for
    /// exactly what is needed, and does not require the caller to have a git
    /// repository in hand.
    pub fn lint<S: AsRef<str>>(
        &self,
        candidates: &[S],
        repo_root: &Path,
        now: &str,
        has_expired: &dyn Fn(&str, &str) -> bool,
    ) -> Vec<RootRot> {
        let mut decided = vec![false; self.entries.len()];
        for candidate in candidates {
            if let Some(index) = self.deciding(candidate.as_ref()) {
                decided[index] = true;
            }
        }

        let mut rot = Vec::new();
        for (index, entry) in self.entries.iter().enumerate() {
            let pathspec = entry.pathspec.clone();
            if let Some(literal) = self.patterns[index].literal_path() {
                if !repo_root.join(literal).exists() {
                    rot.push(RootRot::ReferentGone {
                        line: entry.line,
                        pathspec,
                    });
                    continue;
                }
            }
            if let Some(expires) = entry.expires.as_deref().filter(|e| has_expired(e, now)) {
                rot.push(RootRot::Expired {
                    line: entry.line,
                    pathspec,
                    expires: expires.to_string(),
                });
            } else if !decided[index] {
                rot.push(RootRot::MatchedNothing {
                    line: entry.line,
                    pathspec,
                });
            }
        }
        rot
    }

    /// ProGuard `-printseeds`, which §9.13 asks for by name: show a human every
    /// root this file put into the set, and where it came from.
    ///
    /// This is the module's actual product. Nothing here decides what is
    /// reachable; it materializes what was declared and makes the declaration
    /// auditable *before* anything acts on it — the same reason Nix ships
    /// `--print-roots`.
    ///
    /// Tab-separated and sorted, so it diffs cleanly between runs.
    pub fn print_seeds<S: AsRef<str>>(&self, candidates: &[S]) -> String {
        use fmt::Write as _;

        let seeds = self.materialize(candidates);
        let mut out = String::new();
        out.push_str("# judged print-seeds — roots declared in ");
        out.push_str(ROOTS_FILE);
        out.push('\n');
        out.push_str("# tier\tstatus\tkind\tpath\tdeclared at\treason\n");

        if seeds.is_empty() {
            // §6.20: "no data" must be a distinct state from "zero executions".
            // A silently empty report reads as success — to a human skimming CI
            // output, and to whatever consumes it next.
            out.push_str("# no declared roots matched this run\n");
            return out;
        }

        for seed in &seeds {
            // Writing into a String cannot fail; the `Result` is an artefact of
            // the `Write` trait being shared with fallible sinks.
            let _ = writeln!(
                out,
                "{}\t{}\t{}\t{}\t{}:{}\t{}",
                seed.tier,
                seed.status,
                seed.kind,
                seed.path,
                ROOTS_FILE,
                seed.declared_at_line,
                seed.reason,
            );
        }
        out
    }
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

/// An entry under construction, so that a missing field is reported against the
/// `[[root]]` header rather than against whichever line happened to be last.
struct Draft {
    line: usize,
    pathspec: Option<String>,
    reason: Option<String>,
    kind: Option<SuppressionKind>,
    status: Option<SuppressionStatus>,
    expires: Option<String>,
}

impl Draft {
    fn new(line: usize) -> Draft {
        Draft {
            line,
            pathspec: None,
            reason: None,
            kind: None,
            status: None,
            expires: None,
        }
    }

    fn set(&mut self, key: String, value: String, line: usize) -> Result<(), MalformedRoots> {
        let occupied = match key.as_str() {
            "path" => self.pathspec.replace(value).is_some(),
            "reason" => self.reason.replace(value).is_some(),
            "expires" => self.expires.replace(value).is_some(),
            "kind" => {
                let kind = match value.as_str() {
                    "inSource" => SuppressionKind::InSource,
                    "external" => SuppressionKind::External,
                    other => {
                        return Err(bad(
                            line,
                            format!(
                                "`kind` is SARIF's, so it is `inSource` or `external`, \
                                 not `{other}`"
                            ),
                        ))
                    }
                };
                self.kind.replace(kind).is_some()
            }
            "status" => {
                let status = match value.as_str() {
                    "accepted" => SuppressionStatus::Accepted,
                    "underReview" => SuppressionStatus::UnderReview,
                    "rejected" => SuppressionStatus::Rejected,
                    other => {
                        return Err(bad(
                            line,
                            format!(
                                "`status` is SARIF's, so it is `accepted`, `underReview` or \
                                 `rejected`, not `{other}`"
                            ),
                        ))
                    }
                };
                self.status.replace(status).is_some()
            }
            other => {
                return Err(bad(
                    line,
                    format!(
                        "unknown key `{other}`; an entry has `path`, `reason`, `kind`, \
                         `status`, and optionally `expires`"
                    ),
                ))
            }
        };
        if occupied {
            return Err(bad(
                line,
                format!("`{key}` is set twice in one entry; which one is meant?"),
            ));
        }
        Ok(())
    }

    fn finish(self) -> Result<DeclaredRoot, MalformedRoots> {
        let line = self.line;
        let pathspec = self
            .pathspec
            .ok_or_else(|| bad(line, "this entry has no `path`, so it declares nothing"))?;
        validate_pathspec(&pathspec, line)?;
        let reason = self.reason.ok_or_else(|| {
            bad(
                line,
                "this entry has no `reason`; §5.3 makes it mandatory, because an entry \
                 nobody can justify is one nobody will ever dare remove",
            )
        })?;
        let kind = self
            .kind
            .ok_or_else(|| bad(line, "this entry has no `kind` (`inSource` or `external`)"))?;
        let status = self.status.ok_or_else(|| {
            bad(
                line,
                "this entry has no `status`; SARIF lets it default, but a defaulted status \
                 records no human decision, and a recorded decision is the only thing this \
                 file is for",
            )
        })?;
        Ok(DeclaredRoot {
            pathspec,
            reason,
            kind,
            status,
            expires: self.expires,
            line,
        })
    }
}

/// A pathspec that resolves outside the repository is refused.
///
/// `..` would make the referent check in [`DeclaredRoots::lint`] ask about a
/// file outside the working tree, and — worse — makes the file unreviewable, in
/// that a reader cannot tell what it protects. A leading `/` is *not* an
/// absolute path; it is gitignore's anchor, and resolves at the repo root.
fn validate_pathspec(pathspec: &str, line: usize) -> Result<(), MalformedRoots> {
    let body = pathspec.strip_prefix('!').unwrap_or(pathspec);
    if body.trim().is_empty() {
        return Err(bad(line, "`path` is empty, so it declares nothing"));
    }
    if body.split('/').any(|component| component == "..") {
        return Err(bad(
            line,
            format!("`{pathspec}` escapes the repository with `..`; pathspecs are repo-relative"),
        ));
    }
    if body.contains('\\') {
        return Err(bad(
            line,
            format!("`{pathspec}` contains `\\`; pathspecs use `/` on every platform"),
        ));
    }
    if let Err(message) = check_classes(body) {
        return Err(bad(line, format!("`{pathspec}`: {message}")));
    }
    Ok(())
}

/// Reject an unterminated `[`, rather than quietly treating it as a literal
/// bracket and matching something the author never asked for.
fn check_classes(pattern: &str) -> Result<(), String> {
    let chars: Vec<char> = pattern.chars().collect();
    let mut index = 0;
    while index < chars.len() {
        if chars[index] == '[' {
            match class_end(&chars[index..]) {
                Some(offset) => index += offset + 1,
                None => return Err("an unterminated `[` character class".to_string()),
            }
        } else {
            index += 1;
        }
    }
    Ok(())
}

/// The content of a line up to an unquoted `#`.
///
/// Quote-aware, because a `reason` is prose: `#215914` and embedded quotes turn
/// up constantly, and truncating one silently mangles the field §5.3 makes
/// mandatory. Indexing by byte is safe — `"`, `\` and `#` are ASCII, so `index`
/// always lands on a character boundary.
fn strip_comment(raw: &str) -> &str {
    let mut in_string = false;
    let mut escaped = false;
    for (index, byte) in raw.bytes().enumerate() {
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
        } else if byte == b'"' {
            in_string = true;
        } else if byte == b'#' {
            return &raw[..index];
        }
    }
    raw
}

/// Split `key = "value"`. The first `=` is the separator, since no key in the
/// accepted subset may contain one.
fn split_key_value(content: &str, line: usize) -> Result<(String, String), MalformedRoots> {
    let Some(equals) = content.find('=') else {
        return Err(bad(
            line,
            format!("expected `key = \"value\"`, found `{content}`"),
        ));
    };
    let key = content[..equals].trim().to_string();
    if key.is_empty() {
        return Err(bad(line, "a value with no key"));
    }
    let value = parse_basic_string(content[equals + 1..].trim(), line)?;
    Ok((key, value))
}

/// A TOML basic string: double quotes, with `\"`, `\\`, `\n` and `\t`.
///
/// Bare, literal and multi-line strings are refused rather than guessed at. A
/// parser that guesses is a parser that disagrees with the format it imitates,
/// and the disagreement surfaces as a silently dropped root.
fn parse_basic_string(source: &str, line: usize) -> Result<String, MalformedRoots> {
    let mut chars = source.chars();
    if chars.next() != Some('"') {
        return Err(bad(
            line,
            format!("values must be double-quoted basic strings, found `{source}`"),
        ));
    }
    let mut out = String::new();
    let mut closed = false;
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                closed = true;
                break;
            }
            '\\' => match chars.next() {
                Some('"') => out.push('"'),
                Some('\\') => out.push('\\'),
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some(other) => {
                    return Err(bad(
                        line,
                        format!(
                            "unknown escape `\\{other}`; this subset has `\\\"` `\\\\` \
                             `\\n` `\\t`"
                        ),
                    ))
                }
                None => return Err(bad(line, "a string ending in a lone backslash")),
            },
            _ => out.push(c),
        }
    }
    if !closed {
        return Err(bad(line, "an unterminated string"));
    }
    if chars.next().is_some() {
        return Err(bad(line, "trailing text after the closing quote"));
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// gitignore pathspec matching (§5.3 point 1)
// ---------------------------------------------------------------------------

/// One component of a compiled pathspec.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Segment {
    /// `**` — zero or more whole path components, except as the final segment,
    /// where git specifies "everything *inside*" and so requires at least one.
    AnyDepth,
    /// A single component, matched with `*`, `?` and `[…]`. None of those ever
    /// crosses a separator, which comes free here: matching is per component.
    Component(String),
}

/// A compiled pathspec.
#[derive(Debug, Clone, Default)]
struct Pattern {
    negated: bool,
    segments: Vec<Segment>,
    /// Set when the pathspec names one exact path, so [`DeclaredRoots::lint`]
    /// has something on disk to look for. `None` for globs and negations — a
    /// pattern has no single referent, and inventing one would invent findings.
    literal: Option<String>,
}

impl Pattern {
    fn compile(pathspec: &str) -> Pattern {
        let (negated, body) = match pathspec.strip_prefix('!') {
            Some(rest) => (true, rest),
            None => (false, pathspec),
        };
        // A trailing `/` is gitignore's directory marker. Candidates are files,
        // so `app/jobs/` and `app/jobs` mean the same thing here: everything
        // underneath.
        let body = body.trim_end_matches('/');
        // git's anchoring rule: a pattern containing a slash anywhere but the
        // end is relative to the ignore file's own directory — the repo root,
        // for us. One without a slash floats, and matches at any depth.
        let anchored = body.starts_with('/') || body.contains('/');
        let body = body.trim_start_matches('/');

        let mut segments = Vec::new();
        if !anchored {
            segments.push(Segment::AnyDepth);
        }
        for part in body.split('/') {
            if part == "**" {
                segments.push(Segment::AnyDepth);
            } else if !part.is_empty() {
                segments.push(Segment::Component(part.to_string()));
            }
        }

        let literal = (!negated && !body.is_empty() && !body.contains(['*', '?', '[']))
            .then(|| body.to_string());

        Pattern {
            negated,
            segments,
            literal,
        }
    }

    fn literal_path(&self) -> Option<&str> {
        self.literal.as_deref()
    }

    /// Whether this pathspec covers `path`.
    ///
    /// Every prefix of the path is tried, not only the whole thing, because
    /// git's rule is that matching a *directory* matches everything under it.
    /// Candidates are files, so `app/jobs` has to reach `app/jobs/mailer.rb` — a
    /// user who writes the directory name and gets nothing back would rightly
    /// call that a bug.
    fn matches(&self, path: &str) -> bool {
        let components: Vec<&str> = path.split('/').filter(|c| !c.is_empty()).collect();
        (1..=components.len()).any(|depth| match_segments(&self.segments, &components[..depth]))
    }
}

fn match_segments(segments: &[Segment], components: &[&str]) -> bool {
    let Some((head, rest)) = segments.split_first() else {
        return components.is_empty();
    };
    match head {
        Segment::AnyDepth => {
            if rest.is_empty() {
                // Trailing `**`: "everything inside", so never zero components.
                return !components.is_empty();
            }
            (0..=components.len()).any(|skip| match_segments(rest, &components[skip..]))
        }
        Segment::Component(pattern) => match components.split_first() {
            Some((first, tail)) if component_matches(pattern, first) => match_segments(rest, tail),
            _ => false,
        },
    }
}

/// Glob one path component: `*`, `?`, `[…]`, everything else literal.
fn component_matches(pattern: &str, text: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let text: Vec<char> = text.chars().collect();
    glob(&pattern, &text)
}

fn glob(pattern: &[char], text: &[char]) -> bool {
    let Some((&head, rest)) = pattern.split_first() else {
        return text.is_empty();
    };
    match head {
        '*' => (0..=text.len()).any(|skip| glob(rest, &text[skip..])),
        '?' => !text.is_empty() && glob(rest, &text[1..]),
        '[' => match class_end(pattern) {
            Some(end) => {
                !text.is_empty()
                    && class_matches(&pattern[1..end], text[0])
                    && glob(&pattern[end + 1..], &text[1..])
            }
            // Unreachable for anything that came through `validate_pathspec`,
            // which refuses an unterminated `[` outright.
            None => !text.is_empty() && text[0] == '[' && glob(rest, &text[1..]),
        },
        c => !text.is_empty() && text[0] == c && glob(rest, &text[1..]),
    }
}

/// Index of the `]` closing the class that starts at `chars[0]`.
///
/// A `!`/`^` immediately after the `[`, and a `]` immediately after that, are
/// part of the class rather than its terminator — POSIX's way of spelling a
/// literal `]`, which git inherits.
fn class_end(chars: &[char]) -> Option<usize> {
    let mut index = 1;
    if matches!(chars.get(index), Some('!') | Some('^')) {
        index += 1;
    }
    if matches!(chars.get(index), Some(']')) {
        index += 1;
    }
    while index < chars.len() {
        if chars[index] == ']' {
            return Some(index);
        }
        index += 1;
    }
    None
}

fn class_matches(body: &[char], candidate: char) -> bool {
    let (negated, body) = match body.first() {
        Some('!') | Some('^') => (true, &body[1..]),
        _ => (false, body),
    };
    let mut hit = false;
    let mut index = 0;
    while index < body.len() {
        if index + 2 < body.len() && body[index + 1] == '-' {
            if body[index] <= candidate && candidate <= body[index + 2] {
                hit = true;
            }
            index += 3;
        } else {
            if body[index] == candidate {
                hit = true;
            }
            index += 1;
        }
    }
    hit != negated
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_trailing_double_star_means_everything_inside_not_the_thing_itself() {
        // git's wording, and the difference matters: `app/**` declaring `app`
        // itself would make a file named `app` a root by accident.
        let pattern = Pattern::compile("app/**");
        assert!(pattern.matches("app/a.rb"));
        assert!(pattern.matches("app/deep/a.rb"));
        assert!(!pattern.matches("app"));
    }

    #[test]
    fn a_double_star_in_the_middle_matches_zero_directories() {
        // "A slash followed by two consecutive asterisks then a slash matches
        // zero or more directories" — so `a/**/b` has to reach `a/b`.
        let pattern = Pattern::compile("a/**/b.rb");
        assert!(pattern.matches("a/b.rb"));
        assert!(pattern.matches("a/x/b.rb"));
        assert!(pattern.matches("a/x/y/b.rb"));
        assert!(!pattern.matches("b.rb"));
    }

    #[test]
    fn only_an_unambiguous_pathspec_has_a_referent_to_look_for() {
        // The guard on `ReferentGone`. Asking the filesystem for a path spelled
        // `app/**` would answer "missing" on every healthy repository — the
        // loudest possible false positive, and one a reader learns to ignore.
        assert_eq!(Pattern::compile("a/b.rb").literal_path(), Some("a/b.rb"));
        assert_eq!(Pattern::compile("/a/b.rb").literal_path(), Some("a/b.rb"));
        assert_eq!(Pattern::compile("a/b/").literal_path(), Some("a/b"));
        assert_eq!(Pattern::compile("a/*.rb").literal_path(), None);
        assert_eq!(Pattern::compile("a?.rb").literal_path(), None);
        assert_eq!(Pattern::compile("a[0].rb").literal_path(), None);
        // A negation asserts where something *is not*; there is nothing on disk
        // whose absence would make it stale.
        assert_eq!(Pattern::compile("!a/b.rb").literal_path(), None);
    }

    #[test]
    fn a_star_never_crosses_a_separator_however_greedy_it_gets() {
        let pattern = Pattern::compile("app/*/*.rb");
        assert!(pattern.matches("app/x/a.rb"));
        assert!(!pattern.matches("app/x/y/a.rb"));
        assert!(!pattern.matches("app/a.rb"));
    }

    #[test]
    fn character_classes_close_where_posix_says_they_do() {
        // `[]]` and `[!]]` spell a literal `]`; getting this wrong turns the
        // rest of the pattern into class body and matches wildly.
        assert_eq!(class_end(&['[', ']', ']']), Some(2));
        assert_eq!(class_end(&['[', '!', ']', ']']), Some(3));
        assert_eq!(class_end(&['[', 'a', '-', 'z', ']']), Some(4));
        assert_eq!(class_end(&['[', 'a', 'b']), None);
        assert!(class_matches(&['a', '-', 'z'], 'm'));
        assert!(!class_matches(&['a', '-', 'z'], 'M'));
        assert!(class_matches(&['!', 'a'], 'b'));
        assert!(!class_matches(&['!', 'a'], 'a'));
        // A trailing `-` is a literal `-`, not a half-written range.
        assert!(class_matches(&['a', '-'], '-'));
    }

    #[test]
    fn a_hash_inside_a_string_is_not_a_comment() {
        assert_eq!(strip_comment("a = \"x # y\" # z"), "a = \"x # y\" ");
        assert_eq!(strip_comment("a = \"x \\\" # y\""), "a = \"x \\\" # y\"");
        assert_eq!(strip_comment("# all of it"), "");
    }
}
