//! Gate 1 — the never-touch inventory (§9.3).
//!
//! Every other layer in this crate reasons about **usefulness**: is this
//! referenced, is it an entry point, is it reachable, did it run. Gate 1 is the
//! only one that reasons about the **cost of being wrong**, and §9.3 states the
//! distinction exactly: its refusals are *"justified by IRREVERSIBILITY, not
//! uselessness"*. That is why it runs BEFORE the reference veto rather than
//! after — a file can be provably unreferenced and still be the last copy of
//! something.
//!
//! §1.3 is the argument in full. The two error directions are not comparable:
//! a missed deletion costs disk and a little cognitive load, and is recoverable
//! at any time. A wrong deletion ranges from a build break to a silent
//! behavioural change discovered eleven months later during the incident the
//! code existed for. Gate 1 is where that asymmetry is spent.
//!
//! # Gate 1 pairs with Gate 0g, and neither works alone
//!
//! [`crate::git::RecoverabilityClass`] answers *what could restore this* —
//! tracked-and-pushed is recoverable at rung R2–R4, untracked and ignored are
//! R9, unrecoverable, because git protects the object database and not the
//! working tree (§8.1). Gate 1 answers *what does destroying it cost*. A
//! candidate needs both: a `.env` is unrecoverable AND expensive; a stale build
//! artifact is unrecoverable and cheap; a committed migration is recoverable
//! and catastrophic to remove, because every deployed environment holds a row
//! naming it.
//!
//! # 1p is the rule the other fifteen exist to make affordable
//!
//! **The unknown defaults to KEEP.** A file whose type cannot be determined is
//! not a candidate. Fifteen enumerated classes are how a tool avoids refusing
//! everything while still refusing on ignorance — they buy back the recall that
//! rule costs, and none of them is licence to guess.

pub mod content;
pub mod contracts;
pub mod state;
