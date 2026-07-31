//! Shared vocabulary for Judged.
//!
//! Judged exists because there is no sound, general way to prove a file or a
//! symbol is unused (§0.1: Rice's theorem plus an open world). Every analyzer
//! answers "unreachable from root set R under resolver X"; none answers "is
//! deleting this safe". So this crate deliberately contains no analyzer. It
//! contains the three things every part of the system has to agree on:
//!
//! - [`sarif`] — the integration contract adapters are held to (§9.2).
//! - [`fingerprint`] — content-derived finding identity (§9.2, §9.4).
//! - [`git`] — recoverability classification, i.e. Gate 0g (§8.1), the single
//!   most consequential finding in the research.

pub mod error;
pub mod fingerprint;
pub mod git;
pub mod sarif;

pub use error::{Error, Result};
