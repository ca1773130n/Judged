//! The materialized root set (§5).
//!
//! Reachability analysis is only meaningful under a closed-world assumption,
//! and real repositories are open-world in at least five independent
//! directions (§1.2): humans invoke scripts, other repositories consume
//! artifacts, runtime data names classes, deploy-time config supplies strings,
//! and published artifacts outlive the code that produced them.
//!
//! **You cannot infer the closed world. You can only have it declared.** GraalVM
//! is the industrial proof — an entire ecosystem of reachability metadata exists
//! because "determining dynamically-accessed elements via static analysis is
//! infeasible as reachability depends on data available only at run time". Nix
//! is the other: roots must be registered, and `--print-roots` lets you audit
//! the classification before anything is collected.
//!
//! So this module does not decide what is reachable. It **materializes what was
//! declared**, records where each root came from, and shows it to a human — the
//! job ProGuard's `-printseeds` does, and which §9.13 asks for by name.
//!
//! # Provenance is the load-bearing field
//!
//! §5.1 splits roots into three tiers, and the difference is not cosmetic:
//!
//! - **A, machine-declared** — a build system or deploy target already reads
//!   this file to find roots. Auto-discoverable, high confidence.
//! - **B, convention-inferable** — a framework's file layout or annotations make
//!   a file an entry point with no source reference anywhere. Correct only if
//!   the framework *and its version* were detected correctly.
//! - **C, undiscoverable** — the live set is determined by data or intent
//!   outside the repository entirely. No amount of static cleverness moves a
//!   Tier C root into A or B; it has to be solicited from a human and recorded.
//!
//! A root that does not say which tier it came from is worse than no root: it
//! invites a caller to trust a guessed convention as though a manifest had
//! declared it.

pub mod convention;
pub mod declared;
pub mod insource;
pub mod manifest;
