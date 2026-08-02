//! The §10 E2 catalogue: 19 known-live artifacts, one reachability mechanism each.
//!
//! Classes 1–14 are the "minimum catalogue", each derived from one documented
//! real failure. They share a shape: *a reference in a place you didn't parse*.
//!
//! Classes 15–19 were added to cover §6.24 and the under-served ecosystems, and
//! they are structurally different: *a reference in a place that does not exist
//! in the repository at all*. None of the first fourteen exercises that, which
//! is why they are not optional extras.
//!
//! §10 E2 is explicit that 19 is a floor, not a ceiling — each class encodes one
//! documented incident, so the catalogue grows every time a new one is
//! documented. Add classes; never remove one to make a run green.

pub mod coverage;

use std::path::Path;

use judged_core::{Error, Result};

use crate::mutant::Mutant;

/// Write one fixture file, creating parents, attaching the path to any failure.
///
/// Lives here because eight class modules had a byte-identical private copy,
/// each carrying a doc comment explaining that it was duplicated "because
/// `fixtures/mod.rs` is complete and there is nowhere to put a shared helper".
/// That constraint was an instruction to the agents writing those modules, not
/// a property of the code, and it is not a reason to keep eight copies of a
/// nine-line function.
pub(crate) fn write(root: &Path, rel: &str, contents: &str) -> Result<()> {
    let path = root.join(rel);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| Error::Io {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    std::fs::write(&path, contents).map_err(|source| Error::Io { path, source })
}

/// The catalogue, in class order. `all()[n - 1]` is §10 E2 class `n`.
pub fn all() -> Vec<Box<dyn Mutant>> {
    vec![
        Box::new(m01_yaml_string_ref::YamlStringRef),
        Box::new(m02_dynamic_import::DynamicImport),
        Box::new(m03_plugin_dir_scan::PluginDirScan),
        Box::new(m04_human_cli_subcommand::HumanCliSubcommand),
        Box::new(m05_error_path_only::ErrorPathOnly),
        Box::new(m06_concurrency_helper::ConcurrencyHelper),
        Box::new(m07_guard_clause::GuardClause),
        Box::new(m08_ci_manifest_ref::CiManifestRef),
        Box::new(m09_readme_executed_block::ReadmeExecutedBlock),
        Box::new(m10_framework_convention::FrameworkConvention),
        Box::new(m11_reflective_field::ReflectiveField),
        Box::new(m12_linkname_alias::LinknameAlias),
        Box::new(m13_gitignore_negation::GitignoreNegation),
        Box::new(m14_checked_in_generated_asset::CheckedInGeneratedAsset),
        Box::new(m15_enqueued_job_payload::EnqueuedJobPayload),
        Box::new(m16_persisted_serialized_blob::PersistedSerializedBlob),
        Box::new(m17_link_time_registry::LinkTimeRegistry),
        Box::new(m18_platform_side_manifest::PlatformSideManifest),
        Box::new(m19_abi_consumer_export::AbiConsumerExport),
    ]
}

/// Class 1 — referenced only by a string in a YAML/JSON config.
pub mod m01_yaml_string_ref;

/// Class 2 — loaded via `importlib` / `require(variable)` / `Class.forName`.
pub mod m02_dynamic_import;

/// Class 3 — registered by a directory-scanning plugin loader.
pub mod m03_plugin_dir_scan;

/// Class 4 — a CLI subcommand invoked only by humans.
pub mod m04_human_cli_subcommand;

/// Class 5 — an error-handling module reached only on failure *(debloat Issue 5)*.
pub mod m05_error_path_only;

/// Class 6 — a synchronization helper used only under concurrency *(debloat Issue 4)*.
pub mod m06_concurrency_helper;

/// Class 7 — a guard clause with no observable effect *(debloat Issue 3)*.
pub mod m07_guard_clause;

/// Class 8 — referenced only from a Dockerfile / CI workflow / k8s manifest.
pub mod m08_ci_manifest_ref;

/// Class 9 — referenced only from a README code block that CI executes.
pub mod m09_readme_executed_block;

/// Class 10 — loaded by framework convention.
pub mod m10_framework_convention;

/// Class 11 — an ORM/serializer field touched only via reflection
/// *(Periphery's Codable case)*.
pub mod m11_reflective_field;

/// Class 12 — a symbol aliased via `//go:linkname` / `extern "C"` / `#[no_mangle]`.
pub mod m12_linkname_alias;

/// Class 13 — a file un-ignored by a `!` gitignore negation.
pub mod m13_gitignore_negation;

/// Class 14 — a checked-in generated artifact served directly by a CDN.
pub mod m14_checked_in_generated_asset;

/// Class 15 — a worker class named only in an already-enqueued job payload *(§6.24)*.
pub mod m15_enqueued_job_payload;

/// Class 16 — a type whose only remaining consumer is a persisted serialized
/// blob *(§6.24; exactly what OpenRewrite's `serialVersionUID` bail-out protects)*.
pub mod m16_persisted_serialized_blob;

/// Class 17 — a symbol reachable only through a link-time registry *(§6.1)*.
pub mod m17_link_time_registry;

/// Class 18 — an entry point declared only in a platform-side manifest *(§5.2)*.
pub mod m18_platform_side_manifest;

/// Class 19 — an exported symbol with no in-repo caller but a live ABI consumer
/// *(§6.24, §6.9)*.
pub mod m19_abi_consumer_export;

/// Test scaffolding every class module shares: one materializer, one whole-repo
/// byte search, one plain-text search, and the three assertions each fixture
/// makes about the repository it built.
#[cfg(test)]
mod support;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mutant::Ecosystem;

    #[test]
    fn the_catalogue_declares_all_nineteen_e2_classes_in_order() {
        let mutants = all();

        assert_eq!(mutants.len(), 19, "§10 E2 defines 19 classes");

        let ids: Vec<String> = mutants.iter().map(|m| m.id().to_string()).collect();
        let expected_ids: Vec<String> = (1..=19).map(|n| format!("m{n:02}")).collect();
        assert_eq!(ids, expected_ids);

        let ecosystems: Vec<Ecosystem> = mutants.iter().map(|m| m.ecosystem()).collect();
        assert_eq!(
            ecosystems,
            vec![
                Ecosystem::Python,     // m01 yaml_string_ref
                Ecosystem::Polyglot,   // m02 dynamic_import (Python + TS)
                Ecosystem::Python,     // m03 plugin_dir_scan
                Ecosystem::Rust,       // m04 human_cli_subcommand
                Ecosystem::Python,     // m05 error_path_only
                Ecosystem::Rust,       // m06 concurrency_helper
                Ecosystem::Rust,       // m07 guard_clause
                Ecosystem::Polyglot,   // m08 ci_manifest_ref
                Ecosystem::Rust,       // m09 readme_executed_block
                Ecosystem::Polyglot,   // m10 framework_convention (Django + Jest)
                Ecosystem::Python,     // m11 reflective_field
                Ecosystem::Go,         // m12 linkname_alias
                Ecosystem::Polyglot,   // m13 gitignore_negation
                Ecosystem::TypeScript, // m14 checked_in_generated_asset
                Ecosystem::Python,     // m15 enqueued_job_payload
                Ecosystem::Python,     // m16 persisted_serialized_blob
                Ecosystem::Rust,       // m17 link_time_registry
                Ecosystem::Polyglot,   // m18 platform_side_manifest
                Ecosystem::Rust,       // m19 abi_consumer_export
            ]
        );
    }

    #[test]
    fn every_mutant_traces_back_to_its_research_class() {
        for (index, mutant) in all().iter().enumerate() {
            let class = index + 1;
            assert_eq!(
                mutant.research_ref(),
                format!("§10 E2 class {class}"),
                "{} must cite the class it encodes",
                mutant.id()
            );
            assert!(
                !mutant.mechanism().is_empty(),
                "{} must name its single liveness mechanism",
                mutant.id()
            );
        }
    }
}
