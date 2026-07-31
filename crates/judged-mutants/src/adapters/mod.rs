//! Adapters: real analyzers, translated into the verdict the suite grades.
//!
//! Until now E2 has only ever graded SUTs written in this repository
//! ([`crate::sut::NaiveSut`], [`crate::sut::RefusingSut`]), which bounds the
//! harness and nothing else. §11 R1 makes the existence of an auto-act tier the
//! highest-risk open question in the design and names E2 as what resolves it —
//! and E2 can only resolve it against tools people actually run.
//!
//! Two rules hold for everything in this module, both from §9.2:
//!
//! 1. **Adapters are read-only.** They translate; they never invoke a `--fix`
//!    mode and never touch the repository. The orchestrator owns 100% of
//!    mutations.
//! 2. **Every adapter declares a capability envelope** — the finding classes the
//!    tool structurally *cannot* emit — so the orchestrator knows when the
//!    tool's silence means anything. Until the envelope lives on
//!    [`crate::sut::Sut`], each adapter exposes it as a public constant.
//!
//! A third rule is this module's own, and it is what makes the resulting number
//! worth anything: **an adapter must not be more careful than the tool it
//! adapts, and must not be less careful either.** Cleaning up a tool's claims
//! before grading them measures the adapter; inventing claims the tool never
//! made measures the adapter too. Where the translation is not exact — and for
//! Vulture it is not — the gap is stated in the adapter's own documentation and
//! surfaced in the report, never quietly closed.

pub mod vulture;
