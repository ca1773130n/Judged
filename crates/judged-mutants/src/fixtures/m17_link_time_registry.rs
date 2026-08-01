//! Class 17 — a symbol reachable only through a link-time registry *(§6.1)*.
//!
//! **Mechanism.** `backfill_missing_avatars` is submitted to an `inventory`
//! registry. `inventory::submit!` expands to an anonymous `const` block holding
//! a `#[used]` static in the platform's static-initializer section —
//! `.init_array` on ELF, `__DATA,__mod_init_func` on Mach-O, `.CRT$XCU` on PE —
//! which the loader walks before `main` and which links the submission into the
//! collection. `linkme`'s `#[distributed_slice]`, a C++ namespace-scope
//! `static Registrar`, and `__attribute__((constructor))` differ only in which
//! section they name. §6.1 groups them because of the property they share, and
//! states it exactly: **the item is placed by the linker, nothing in the source
//! ever names the section, so the call graph is genuinely empty and the item
//! still runs.**
//!
//! Substituting a hand-written `#[link_section]`/`#[used]` static for the macro
//! would present the same shape; the macro is used here because it is what
//! §6.1 names first and because the desugaring is written out at the submission
//! site, so nothing about the shape is hidden behind a dependency the fixture
//! never builds.
//!
//! **Why every other signal misses it.** §6.1's defeats list is short and this
//! mutant covers all of it: static reachability, compiler index, build graph.
//! `apply_all` iterates `inventory::iter::<Migration>`, so the edge from the
//! consumer to this function exists only after linking — no call-graph builder
//! can draw it, because no source file expresses it. That `--gc-sections` needs
//! `KEEP()` and `#[used(linker)]` escape hatches to exist at all is §6.1's own
//! evidence that not even the linker infers this without being told.
//!
//! **What this class does *not* defeat, stated plainly**, because a mutant that
//! overclaims is worse than no mutant: the grep veto survives here. The
//! submission names the function two lines below its definition, and a textual
//! scan sees that. §6.1 lists the signals this shape defeats and the grep veto
//! is not among them — it is listed as defeated only for *structural*
//! reflection, "because there is no identifier string anywhere to match". The
//! test below therefore pins the honest property: the name occurs in exactly
//! one file, twice, and both occurrences are the registration, never a call.

use std::path::Path;

use judged_core::git::Repo;
use judged_core::Result;

use crate::fixtures::write;
use crate::mutant::{Ecosystem, GroundTruth, Mutant};

/// `inventory::submit!` / `linkme` / a self-registering `static Registrar` /
/// `__attribute__((constructor))`. §10 E2 puts it exactly: the call graph is
/// genuinely empty and the code genuinely runs.
pub struct LinkTimeRegistry;

/// The file holding the submission. Deliberately named for the migration
/// number rather than for the function, so that the `mod` line that compiles it
/// cannot be mistaken for a reference to the live symbol.
const LIVE_FILE: &str = "src/migrations/m0007.rs";

/// The live symbol: no callers, and one link-time registration.
const LIVE_SYMBOL: &str = "backfill_missing_avatars";

/// The generic consumer. It names the registry and never an entry in it.
const CONSUMER: &str = "src/lib.rs";

/// An orphan left by a refactor: no `mod` declaration anywhere names it, so
/// cargo never compiles it and the linker never sees it. Dead in the way the
/// migration above only looks dead.
const DECOY: &str = "src/checksum_v1.rs";

/// The decoy's only definition, so a symbol-level analyzer — which never claims
/// a path — is asked a question it can answer (see
/// `GroundTruth::decoy_dead_symbols`).
const DECOY_SYMBOL: &str = "crc16";

impl Mutant for LinkTimeRegistry {
    fn id(&self) -> &str {
        "m17"
    }
    fn ecosystem(&self) -> Ecosystem {
        Ecosystem::Rust
    }
    fn mechanism(&self) -> &str {
        "registered at link time via inventory::submit!, with an empty call graph"
    }
    fn research_ref(&self) -> &str {
        "§10 E2 class 17"
    }
    fn materialize(&self, dir: &Path) -> Result<GroundTruth> {
        let repo = Repo::init(dir)?;
        let root = repo.root().to_path_buf();

        write(
            &root,
            "Cargo.toml",
            r#"[package]
name = "schema-migrator"
version = "0.4.2"
edition = "2021"

[dependencies]
# The mechanism, declared like any other dependency. Nothing about a registry
# looks unusual in a manifest, which is part of why it is missed.
inventory = "0.3"
"#,
        )?;

        // The consumer. Generic over the registry, and that is the whole class:
        // the edge from here to any one migration is created by the linker.
        write(
            &root,
            CONSUMER,
            r#"//! Migrations register themselves. This file never names one.

/// One migration, as the registry sees it.
pub struct Migration {
    pub id: &'static str,
    pub apply: fn() -> String,
}

// Declares the type submissions target. It does not enumerate them and it
// cannot: the set is assembled by the linker, after this file stops existing.
inventory::collect!(Migration);

/// Runs every registered migration.
///
/// §6.1: "the call graph is genuinely empty and the item still runs". This
/// function is the proof of the second half and the reason for the first.
pub fn apply_all() -> Vec<String> {
    let mut applied: Vec<String> = inventory::iter::<Migration>
        .into_iter()
        .map(|migration| (migration.apply)())
        .collect();
    applied.sort();
    applied
}

// Present only so the compiler emits the object files: a submission that is
// never compiled is never linked, and a migration that is never linked
// silently does not run. This is the Rust spelling of the `KEEP()` and
// `#[used(linker)]` escape hatches §6.1 points at.
mod migrations;
"#,
        )?;

        write(
            &root,
            "src/main.rs",
            r#"fn main() {
    for line in schema_migrator::apply_all() {
        println!("{line}");
    }
}
"#,
        )?;

        write(
            &root,
            "src/migrations/mod.rs",
            "// One file per migration. Nothing here calls anything.\nmod m0007;\n",
        )?;

        // THE LIVE ARTIFACT.
        write(
            &root,
            LIVE_FILE,
            r#"use crate::Migration;

/// Applied once, at deploy time, when the migrator walks the registry.
///
/// It has no callers. Not "none we found" — none exist and none can, because
/// the only thing that reaches it is the submission below, which the linker
/// resolves through a section no source file names.
pub fn backfill_missing_avatars() -> String {
    "0007: backfilled 4210 avatars".to_string()
}

// THE MECHANISM. `inventory::submit!` expands to roughly
//
//     const _: () = {
//         #[used]
//         #[link_section = "..."]  // .init_array | __DATA,__mod_init_func | .CRT$XCU
//         static REGISTER: extern "C" fn() = register;
//         extern "C" fn register() { /* splice this entry into the collection */ }
//     };
//
// `linkme`'s `#[distributed_slice]`, C++'s namespace-scope `static Registrar`,
// and `__attribute__((constructor))` differ only in which section they name.
// §6.1 groups all four for the property they share.
inventory::submit! {
    Migration { id: "0007", apply: backfill_missing_avatars }
}
"#,
        )?;

        // THE DECOY. An orphan: cargo never compiles it, so the linker never
        // sees it, so no registry can contain it.
        write(
            &root,
            DECOY,
            r#"// Left behind by the move to blake3. No `mod` declaration names this file.
pub fn crc16(bytes: &[u8]) -> u16 {
    bytes.iter().fold(0u16, |acc, b| acc.rotate_left(3) ^ u16::from(*b))
}
"#,
        )?;

        repo.add_all()?;
        repo.commit("m17: migration reachable only through its link-time registration")?;

        Ok(GroundTruth {
            live_paths: vec![LIVE_FILE.into()],
            live_symbols: vec![LIVE_SYMBOL.to_string()],
            decoy_dead_paths: vec![DECOY.into()],
            decoy_dead_symbols: vec![DECOY_SYMBOL.to_string()],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::support;

    #[test]
    fn m17_materializes_a_real_git_repo_with_one_commit() {
        let (_dir, repo, _truth) = support::materialize(&LinkTimeRegistry);
        support::assert_committed(&repo, &["Cargo.toml", LIVE_FILE, CONSUMER, DECOY]);
    }

    #[test]
    fn m17_ground_truth_paths_all_exist_on_disk() {
        let (_dir, repo, truth) = support::materialize(&LinkTimeRegistry);
        assert_eq!(truth.live_paths, vec![Path::new(LIVE_FILE).to_path_buf()]);
        assert_eq!(truth.live_symbols, vec![LIVE_SYMBOL.to_string()]);
        support::assert_ground_truth_is_on_disk(&repo, &truth);
    }

    /// The hardness assertion. One file names the live symbol, and it is the
    /// file that submits it. Anything else naming it would be a second way to
    /// reach the function, and the mutant would stop testing the registry.
    #[test]
    fn m17_the_registered_function_is_named_only_where_it_is_submitted() {
        let (_dir, repo, _truth) = support::materialize(&LinkTimeRegistry);

        let naming: Vec<String> = support::tree(repo.root())
            .into_iter()
            .filter(|(_, bytes)| support::mentions(bytes, LIVE_SYMBOL))
            .map(|(path, _)| path)
            .collect();
        assert_eq!(
            naming,
            vec![LIVE_FILE.to_string()],
            "{LIVE_SYMBOL} must be named only in the file that registers it"
        );
    }

    /// §6.1's actual claim, pinned: the two occurrences are the definition and
    /// the submission. Neither is a call, and a call is what every reachability
    /// signal is looking for.
    #[test]
    fn m17_nothing_anywhere_calls_the_registered_function() {
        let (_dir, repo, _truth) = support::materialize(&LinkTimeRegistry);

        let call_site = format!("{LIVE_SYMBOL}(");
        for (path, bytes) in support::tree(repo.root()) {
            let calls = support::occurrences(&bytes, &call_site);
            let expected = usize::from(path == LIVE_FILE); // the `fn` line only
            assert_eq!(
                calls, expected,
                "{path} contains {calls} occurrence(s) of `{call_site}`; the \
                 call graph for a link-time registry must be empty"
            );
        }

        let live = std::fs::read(repo.root().join(LIVE_FILE)).expect("read the migration");
        assert_eq!(
            support::occurrences(&live, LIVE_SYMBOL),
            2,
            "exactly two occurrences: the definition and the inventory::submit!"
        );
        assert!(
            support::mentions(&live, "inventory::submit!"),
            "the submission is the mechanism; without it the fixture is just a dead fn"
        );
    }

    /// A Rust source file is always named by the `mod` line that compiles it —
    /// there is no way to check one in without that. Pinning it here keeps the
    /// claim honest rather than silent: being compiled is not being called, and
    /// a tool that read `mod` as a liveness reference would call every file in
    /// every Rust crate alive.
    #[test]
    fn m17_the_migration_file_is_named_only_by_the_mod_line_that_compiles_it() {
        let (_dir, repo, _truth) = support::materialize(&LinkTimeRegistry);
        let stem = Path::new(LIVE_FILE)
            .file_stem()
            .and_then(|s| s.to_str())
            .expect("the live file has a UTF-8 stem");

        let naming: Vec<String> = support::tree(repo.root())
            .into_iter()
            .filter(|(path, bytes)| path != LIVE_FILE && support::mentions(bytes, stem))
            .map(|(path, _)| path)
            .collect();
        assert_eq!(naming, vec!["src/migrations/mod.rs".to_string()]);

        let declaring =
            std::fs::read(repo.root().join("src/migrations/mod.rs")).expect("read mod.rs");
        assert!(
            support::mentions(&declaring, &format!("mod {stem};")),
            "the only reference must be the module declaration itself"
        );
    }

    /// The consumer exists and is generic over the registry. If it named an
    /// entry, that entry would be reachable by an ordinary call and this would
    /// be a different mutant.
    #[test]
    fn m17_the_consumer_iterates_the_registry_without_naming_an_entry() {
        let (_dir, repo, _truth) = support::materialize(&LinkTimeRegistry);
        let consumer = std::fs::read(repo.root().join(CONSUMER)).expect("read lib.rs");

        assert!(
            support::mentions(&consumer, "inventory::collect!")
                && support::mentions(&consumer, "inventory::iter"),
            "the registry must be declared and iterated, or nothing runs the migration"
        );
        assert!(
            !support::mentions(&consumer, LIVE_SYMBOL),
            "the consumer must not name the entry it will end up calling"
        );

        let manifest = std::fs::read(repo.root().join("Cargo.toml")).expect("read Cargo.toml");
        assert!(
            support::mentions(&manifest, "inventory"),
            "a real Cargo.toml has to declare the crate the mechanism depends on"
        );
    }

    #[test]
    fn m17_the_decoy_is_named_by_nothing() {
        let (_dir, repo, truth) = support::materialize(&LinkTimeRegistry);
        support::assert_decoys_are_unreferenced(&repo, &truth);
    }
}
