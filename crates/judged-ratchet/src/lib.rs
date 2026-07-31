//! The ratchet — build this first (§9.14).
//!
//! Baseline the current state; fail CI only on **new** dead code, new junk, new
//! unused dependencies. Zero deletion risk, zero configuration burden,
//! immediate value. §0.5: "a reaper that never stops the inflow is bailing a
//! boat."
//!
//! This crate cannot delete anything. That is a design property, not an
//! omission — see [`diff::RatchetOutcome`].

pub mod baseline;
pub mod diff;
pub mod rot;

pub use baseline::{Baseline, BaselineEntry};
pub use diff::{baseline_state, exit_code, ratchet, RatchetOutcome};
pub use rot::{detect_rot, has_expired, RotReason};
