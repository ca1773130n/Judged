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

pub mod boundary;
pub mod coverage;
pub mod error;
pub mod fingerprint;
pub mod gate1;
pub mod gate3f;
pub mod git;
pub mod ledger;
pub mod roots;
pub mod sarif;
pub mod veto;

pub use error::{Error, Result};
