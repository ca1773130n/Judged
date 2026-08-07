//! Shared vocabulary for Judged.
//!
//! Judged exists because there is no sound, general way to prove a file or a
//! symbol is unused (§0 item 1, argued in §1.1: Rice's theorem plus an open
//! world — the root set is unknowable, not merely unknown). Every analyzer
//! answers "unreachable from root set R under resolver X"; none answers "is
//! deleting this safe". So this crate deliberately contains no analyzer. It
//! contains the three things every part of the system has to agree on:
//!
//! - [`sarif`] — the integration contract adapters are held to (§9.2).
//! - [`fingerprint`] — content-derived finding identity (§9.2, §9.4).
//! - [`git`] — recoverability classification, i.e. Gate 0g (defined in §9.3,
//!   proved in §8.1): "the single most consequential finding in the document"
//!   per §0 item 10.
//!
//! [`coverage`] is the one module that is not shared vocabulary in that sense.
//! It reads an artifact somebody else produced, which makes it an adapter — but
//! it is the workspace's only Family X signal (§9.5), and every layer that wants
//! to ask "was this executed" has to agree on the same answer, so it lives here
//! rather than beside one consumer.
//!
//! # Unix only, and it says so at compile time
//!
//! The boundary gates reason about the filesystem in bytes, not in `Path`
//! components, because component iteration normalizes away the very hazards they
//! exist to detect. That reasoning is written against POSIX semantics
//! throughout: [`gate0a`] treats `/` as the separator and everything else as an
//! ordinary filename byte, which is correct on Unix — where `\` really is a
//! legal character in a name — and wrong on Windows, where `\` separates and
//! `C:\` is a root that must survive stripping while `C:` is a *drive-relative*
//! path resolved against a per-drive working directory.
//!
//! This crate is a library, so the platform it is *verified* on is not the
//! platform it can be *built* for. CI runs `ubuntu-latest` and nothing else; a
//! consumer compiling this for Windows would get a symlink gate that inspects
//! the target where it believes it inspected the link. §6.20's rule is that an
//! unanswerable question must not resolve in the tool's favour, and a safety
//! gate that is silently wrong on a platform nobody tested is that failure in
//! its purest form.
//!
//! So the restriction is stated where it cannot be missed rather than left to be
//! discovered. Adding Windows support means adding Windows CI and
//! prefix-preserving separator handling first; it is not a matter of widening a
//! byte comparison.

#[cfg(not(unix))]
compile_error!(
    "judged-core is Unix-only. Its boundary gates (gate0a in particular) compare raw path bytes \
     against POSIX separator rules: `/` separates and `\\` is an ordinary filename character. On \
     Windows both of those are wrong, and the failure is silent — `symlink_metadata(\"link\\\\\")` \
     follows the link, so the gate reports on the target while believing it reported on the link, \
     and a drive root stripped to `C:` becomes a drive-relative path. Nothing here is tested on \
     Windows: CI is ubuntu-latest only. Supporting it requires Windows CI and prefix-preserving \
     separator handling, not a wider byte comparison, so the build stops here rather than \
     producing a safety gate that is quietly wrong."
);

pub mod boundary;
pub mod coverage;
pub mod error;
pub mod fingerprint;
pub mod gate0a;
pub mod gate0e;
pub mod gate1;
pub mod gate3f;
pub mod git;
pub mod ledger;
pub mod roots;
pub mod sarif;
pub mod veto;

pub use error::{Error, Result};
