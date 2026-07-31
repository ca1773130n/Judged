//! Class 19 — an exported symbol with no in-repo caller but a live ABI consumer
//! *(§6.24, §6.9)*.
//!
//! **Mechanism.** `ledger_amortize` is a `#[no_mangle] extern "C"` function in a
//! crate whose only artifact is a `cdylib`. Its callers link against
//! `libledger.so` and were built years ago. §6.24 is blunt about what that
//! means: removing an exported symbol "breaks **already-linked consumers that
//! were never rebuilt**. There is no in-repo evidence of them at all." So this
//! fixture plants none — not a header, not a binding, not an integration test.
//! Planting one would be planting a second mechanism, and the mutant would stop
//! testing the thing it exists to test.
//!
//! **Why every other signal misses it.** All of them, and for once that is the
//! design rather than a limitation. Static reachability is correct that nothing
//! calls it. The grep veto is correct that the name occurs once. Runtime
//! coverage never sees it, because the process that calls it is not this
//! process. §10 E2: "This one is unfalsifiable from inside the repo *by
//! construction*, which makes it the right test of whether the tool refuses
//! rather than guesses." There is no analysis to improve here. There is only a
//! decision about what to do when evidence is unobtainable, and §6.24's rule
//! settles it: **no auto-act tier may include a candidate whose symbol is
//! exported across an ABI boundary — regardless of ban count.**
//!
//! **The signal a correct tool detects instead.** §6.9 names the failure mode
//! it must avoid — "a library's exports have no in-repo callers *by
//! definition*; this is not a bug in the tools, it is a category error the tool
//! must refuse to make" — and §6.24 lists the markers: `#[no_mangle]`,
//! `.map` version scripts, soname/visibility machinery. This repository carries
//! all three shapes: a `cdylib` crate-type, a version script wired in by
//! `build.rs`, and the attribute itself. §6.9's inverted rule is what makes
//! them decisive rather than advisory — **absence of a distribution manifest is
//! itself grounds for refusal, not for proceeding** — so a repository that does
//! advertise distribution leaves a tool no room at all.
//!
//! The version script matches `ledger_*` rather than listing symbols one by
//! one. That is both the common way to write one and the thing that keeps the
//! mutant hard: a manifest enumerating the export would hand a scanner the
//! answer, and §6.24's whole point is that the answer is not in the repository.

use std::path::Path;

use judged_core::git::Repo;
use judged_core::Result;

use crate::fixtures::write;
use crate::mutant::{Ecosystem, GroundTruth, Mutant};

/// Unfalsifiable from inside the repository **by construction**, which is
/// what makes it the right test: the only correct behaviour is to refuse,
/// and a tool that guesses here is guessing everywhere.
pub struct AbiConsumerExport;

/// The exported surface. One function, no callers, no header.
const LIVE_FILE: &str = "src/ffi.rs";

/// The live symbol. It occurs exactly once in the whole repository.
const LIVE_SYMBOL: &str = "ledger_amortize";

/// The version script — §6.24's `.map` marker, wired in by `build.rs` so it is
/// load-bearing rather than decorative.
const VERSION_SCRIPT: &str = "exports.map";

/// A rounding mode from a previous ABI revision. Orphaned: no `mod` names it,
/// so cargo never compiles it and no consumer can ever have linked it.
const DECOY: &str = "src/deprecated_rounding.rs";

impl Mutant for AbiConsumerExport {
    fn id(&self) -> &str {
        "m19"
    }
    fn ecosystem(&self) -> Ecosystem {
        Ecosystem::Rust
    }
    fn mechanism(&self) -> &str {
        "#[no_mangle] export whose only consumer is outside the repository"
    }
    fn research_ref(&self) -> &str {
        "§10 E2 class 19"
    }
    fn materialize(&self, dir: &Path) -> Result<GroundTruth> {
        let repo = Repo::init(dir)?;
        let root = repo.root().to_path_buf();

        write(
            &root,
            "Cargo.toml",
            r#"[package]
name = "ledger-abi"
version = "3.1.0"
edition = "2021"

[lib]
# §6.9's counter-signal, and §6.24's ABI marker. A cdylib exists to be loaded
# by something that is not in this repository; nothing else explains building
# one. Its absence is what §6.9 calls grounds for refusal in the first place.
crate-type = ["cdylib", "staticlib"]
"#,
        )?;

        write(
            &root,
            VERSION_SCRIPT,
            r#"# Handed to the linker by build.rs. Pins the ABI: every ledger_* symbol is
# global and versioned, everything else is local. Matching by pattern is both
# the ordinary way to write this and the reason it is not a manifest a scanner
# can read back -- §6.24: there is no in-repo evidence of the consumers.
LEDGER_3 {
    global:
        ledger_*;
    local:
        *;
};
"#,
        )?;

        write(
            &root,
            "build.rs",
            r#"fn main() {
    println!("cargo:rerun-if-changed=exports.map");
    println!("cargo:rustc-link-arg-cdylib=-Wl,--version-script=exports.map");
}
"#,
        )?;

        write(
            &root,
            "src/lib.rs",
            r#"//! Amortisation maths, shipped as libledger.so.

mod ffi;

/// Internal, and genuinely reachable: the export calls it, the tests cover it.
pub(crate) fn monthly_payment(principal: f64, annual_rate: f64, months: u32) -> f64 {
    let monthly = annual_rate / 12.0;
    if monthly == 0.0 {
        return principal / f64::from(months);
    }
    let growth = (1.0 + monthly).powi(months as i32);
    principal * monthly * growth / (growth - 1.0)
}

#[cfg(test)]
mod tests {
    #[test]
    fn zero_rate_amortises_evenly() {
        assert!((super::monthly_payment(1200.0, 0.0, 12) - 100.0).abs() < 1e-9);
    }
}
"#,
        )?;

        // THE LIVE ARTIFACT. Called by binaries linked years ago against
        // libledger.so.2 and never rebuilt. Nothing in this repository calls
        // it, declares it, or admits it has consumers -- §6.24: "There is no
        // in-repo evidence of them at all."
        write(
            &root,
            LIVE_FILE,
            r#"use std::os::raw::c_double;

/// Monthly payment on an amortising loan, in the caller's currency units.
///
/// # Safety
/// Plain scalars in, one scalar out. Kept `extern "C"` and unmangled because
/// the callers are C, and because their build predates this source tree.
#[no_mangle]
pub extern "C" fn ledger_amortize(
    principal: c_double,
    annual_rate: c_double,
    months: u32,
) -> c_double {
    crate::monthly_payment(principal, annual_rate, months)
}
"#,
        )?;

        // THE DECOY. From ABI revision 2, which nothing links any more. No
        // `mod` names it, so cargo never compiled it into revision 3 at all.
        write(
            &root,
            DECOY,
            r#"// Banker's rounding, replaced by the caller-side rounding in revision 3.
pub fn half_to_even(value: f64) -> f64 {
    let floor = value.floor();
    match value - floor {
        d if d > 0.5 => floor + 1.0,
        d if d < 0.5 => floor,
        _ if floor % 2.0 == 0.0 => floor,
        _ => floor + 1.0,
    }
}
"#,
        )?;

        repo.add_all()?;
        repo.commit("m19: exported ABI whose only callers were linked elsewhere")?;

        Ok(GroundTruth {
            live_paths: vec![LIVE_FILE.into()],
            live_symbols: vec![LIVE_SYMBOL.to_string()],
            decoy_dead_paths: vec![DECOY.into()],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::support;

    #[test]
    fn m19_materializes_a_real_git_repo_with_one_commit() {
        let (_dir, repo, _truth) = support::materialize(&AbiConsumerExport);
        support::assert_committed(&repo, &["Cargo.toml", VERSION_SCRIPT, LIVE_FILE, DECOY]);
    }

    #[test]
    fn m19_ground_truth_paths_all_exist_on_disk() {
        let (_dir, repo, truth) = support::materialize(&AbiConsumerExport);
        assert_eq!(truth.live_paths, vec![Path::new(LIVE_FILE).to_path_buf()]);
        assert_eq!(truth.live_symbols, vec![LIVE_SYMBOL.to_string()]);
        support::assert_ground_truth_is_on_disk(&repo, &truth);
    }

    /// The hardness assertion, in its strongest available form: the symbol
    /// occurs **once**, at its definition. Not once per file — once in the
    /// repository. Any second occurrence would be in-repo evidence of a
    /// consumer, which §6.24 says does not exist for this class.
    #[test]
    fn m19_the_exported_symbol_occurs_exactly_once_in_the_whole_repository() {
        let (_dir, repo, _truth) = support::materialize(&AbiConsumerExport);

        let occurrences: Vec<(String, usize)> = support::tree(repo.root())
            .into_iter()
            .map(|(path, bytes)| (path, support::occurrences(&bytes, LIVE_SYMBOL)))
            .filter(|(_, count)| *count > 0)
            .collect();
        assert_eq!(
            occurrences,
            vec![(LIVE_FILE.to_string(), 1)],
            "{LIVE_SYMBOL} must appear once, at its definition, and nowhere else"
        );
    }

    /// §6.24: "There is no in-repo evidence of them at all." A header, a
    /// `.def`, a ctypes binding, or a cgo preamble would each be such evidence
    /// and would each rescue the symbol by an entirely different mechanism.
    #[test]
    fn m19_the_repository_contains_no_evidence_of_any_consumer() {
        let (_dir, repo, _truth) = support::materialize(&AbiConsumerExport);

        for (path, bytes) in support::tree(repo.root()) {
            assert!(
                !path.ends_with(".h") && !path.ends_with(".hpp") && !path.ends_with(".def"),
                "{path} is a consumer-facing declaration; m19 must not ship one"
            );
            for binding in ["dlsym", "CDLL", "ffi.cdef", "extern crate ledger", "#cgo"] {
                assert!(
                    !support::mentions(&bytes, binding),
                    "{path} binds the library via {binding}; that is a second mechanism"
                );
            }
        }
    }

    /// The counter-signals §6.9 and §6.24 say a tool must key on, all present,
    /// so refusing here is a decision the tool could actually have reached.
    #[test]
    fn m19_the_repository_advertises_that_it_ships_an_abi() {
        let (_dir, repo, _truth) = support::materialize(&AbiConsumerExport);

        let manifest = std::fs::read(repo.root().join("Cargo.toml")).expect("read Cargo.toml");
        assert!(
            support::mentions(&manifest, "cdylib"),
            "§6.9's counter-signal is a distribution manifest; declare the crate-type"
        );
        let live = std::fs::read(repo.root().join(LIVE_FILE)).expect("read the export");
        assert!(
            support::mentions(&live, "#[no_mangle]") && support::mentions(&live, "extern \"C\""),
            "§6.24 lists #[no_mangle] among its ABI markers"
        );
        let build = std::fs::read(repo.root().join("build.rs")).expect("read build.rs");
        assert!(
            support::mentions(&build, VERSION_SCRIPT),
            "an unwired version script is decoration; hand it to the linker"
        );
    }

    /// The version script proves distributability without naming what is
    /// distributed. A `global:` list enumerating the symbol would be a manifest
    /// a scanner could read, and §6.24's point is that no such manifest exists.
    #[test]
    fn m19_the_version_script_pins_the_abi_without_naming_the_symbol() {
        let (_dir, repo, _truth) = support::materialize(&AbiConsumerExport);
        let script =
            std::fs::read(repo.root().join(VERSION_SCRIPT)).expect("read the version script");

        assert!(support::mentions(&script, "global:") && support::mentions(&script, "local:"));
        assert!(
            support::mentions(&script, "ledger_*;"),
            "the export is matched by pattern, the way version scripts are written"
        );
        assert!(
            !support::mentions(&script, LIVE_SYMBOL),
            "listing the symbol would hand a scanner the evidence §6.24 says is absent"
        );
    }

    #[test]
    fn m19_the_decoy_is_named_by_nothing() {
        let (_dir, repo, truth) = support::materialize(&AbiConsumerExport);
        support::assert_decoys_are_unreferenced(&repo, &truth);
    }
}
