//! Class 14 — a checked-in generated artifact served directly by a CDN.
//!
//! **The mechanism.** `dist/widget.7f3a91c.js` is committed build output. The
//! only thing in the repository that names it is a `<script src>` attribute in
//! `public/index.html`, which the CDN origin serves verbatim. The filename
//! carries a content hash, so it is stamped in by the bundler at build time and
//! pasted into the HTML by hand at release time — which is exactly why the
//! build configuration cannot name it either.
//!
//! **Why every other signal misses it.** Four independent reasons, and a
//! cleaner only has to believe one:
//!
//! - No TypeScript source imports it. It is not a module, it is the *result* of
//!   bundling the modules, so the import graph can never reach it.
//! - It lives under `dist/`, and `tsconfig.json` declares `outDir: "dist"` —
//!   the directory is, by every naming convention in the ecosystem, disposable
//!   build output.
//! - Its name is a hash. Nothing in `package.json`'s build script contains the
//!   string, because the build script does not know it either.
//! - The consumer is outside the repository entirely (§5.2's hosting/platform
//!   contracts, §6.11): the request comes from a browser, against a CDN.
//!
//! **The trap the decoy sets.** `dist/widget.0c9e142.js` is the previous
//! release's bundle, left behind, and it really is dead. Both files are
//! minified, hashed, and sitting in the same "obviously regenerable" directory.
//! Only the HTML tells them apart. A tool that roots all of `dist/` is safe and
//! scores zero decoy recall; a tool that treats `dist/` as junk deletes a live
//! production asset. §10 E2 requires the suite to be able to see the difference,
//! which is what having both files here is for.

use std::path::Path;

use judged_core::git::Repo;
use judged_core::{Error, Result};

use crate::mutant::{Ecosystem, GroundTruth, Mutant};

/// It looks like build output, which is what makes it dangerous: the whole
/// point is that the consumer is outside the repository.
pub struct CheckedInGeneratedAsset;

/// The committed bundle the CDN actually serves.
const LIVE: &str = "dist/widget.7f3a91c.js";

/// The one file that names it.
const MECHANISM: &str = "public/index.html";

/// Files written into the mutant repository, as `(repo-relative path, body)`.
const FILES: &[(&str, &str)] = &[
    (
        "package.json",
        // The build script cannot name the output file: the bundler stamps a
        // content hash into it. That is not incidental to the class, it is why
        // no build-graph analysis reaches the artifact.
        r#"{
  "name": "ledger-widget",
  "version": "0.1.0",
  "private": true,
  "type": "module",
  "scripts": {
    "build": "tsc -p tsconfig.json && esbuild src/main.ts --bundle --minify --entry-names=[name].[hash] --outdir=dist"
  },
  "devDependencies": {
    "esbuild": "^0.21.0",
    "typescript": "^5.4.0"
  }
}
"#,
    ),
    (
        "tsconfig.json",
        r#"{
  "compilerOptions": {
    "target": "ES2022",
    "module": "ES2022",
    "moduleResolution": "bundler",
    "outDir": "dist",
    "strict": true
  },
  "include": ["src/**/*.ts"]
}
"#,
    ),
    (
        ".gitignore",
        r#"node_modules/

# dist/ is deliberately NOT ignored. The release process commits the bundle so
# the CDN origin can serve it straight out of the repository, which is what
# makes a "gitignored implies regenerable" heuristic answer the wrong question
# here (§6.17).
"#,
    ),
    (
        "src/main.ts",
        r#"// Bundle entry point. Compiles into dist/, under a name it never learns.
export function mount(root: HTMLElement): void {
  root.textContent = "ledger";
}

mount(document.body);
"#,
    ),
    (
        MECHANISM,
        r#"<!doctype html>
<meta charset="utf-8">
<title>Ledger</title>
<!--
  The bundle below is committed under dist/ and served straight from the CDN
  origin that mirrors this directory. This tag is the only place in the
  repository where its filename appears; the hash is pasted in at release time.
-->
<script src="/dist/widget.7f3a91c.js" defer></script>
<div id="root"></div>
"#,
    ),
    (
        LIVE,
        r#"// LIVE. Committed build output, requested by every browser that loads the
// page, referenced by exactly one HTML attribute and by nothing else.
(()=>{function m(r){r.textContent="ledger"}m(document.body)})();
"#,
    ),
    (
        "dist/widget.0c9e142.js",
        r#"// DEAD DECOY. The previous release's bundle, superseded and left behind.
// Byte-for-byte as plausible as the live one; only the HTML tells them apart.
(()=>{function m(r){r.textContent="ledger (old)"}m(document.body)})();
"#,
    ),
    (
        "src/unusedFeatureFlags.ts",
        r#"// DEAD DECOY. Inside the tsconfig include glob, so it compiles -- and is
// imported by nothing, named by nothing, and shipped in no bundle.
export const FLAGS: Record<string, boolean> = { legacyCheckout: false };
"#,
    ),
];

impl CheckedInGeneratedAsset {
    /// Repo-relative paths of the genuinely-dead files planted here.
    const DECOYS: [&'static str; 2] = ["dist/widget.0c9e142.js", "src/unusedFeatureFlags.ts"];

    /// The symbol each decoy defines, index-aligned with [`Self::DECOYS`].
    ///
    /// The bundle's is `""`. Its only function is a single letter inside an
    /// IIFE — not a module-level name any analyzer reports, and declaring `m`
    /// would additionally make a claim of `anything.m` score as a decoy find.
    /// A minified artifact has no symbol route, and saying so is the honest
    /// declaration.
    const DECOY_SYMBOLS: [&'static str; 2] = ["", "FLAGS"];
}

impl Mutant for CheckedInGeneratedAsset {
    fn id(&self) -> &str {
        "m14"
    }
    fn ecosystem(&self) -> Ecosystem {
        Ecosystem::TypeScript
    }
    fn mechanism(&self) -> &str {
        "committed build output whose only consumer is a CDN path"
    }
    fn research_ref(&self) -> &str {
        "§10 E2 class 14"
    }
    fn materialize(&self, dir: &Path) -> Result<GroundTruth> {
        let repo = Repo::init(dir)?;
        for (relative, body) in FILES {
            let path = repo.root().join(relative);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(|source| Error::Io {
                    path: parent.to_path_buf(),
                    source,
                })?;
            }
            std::fs::write(&path, body).map_err(|source| Error::Io { path, source })?;
        }
        repo.add_all()?;
        repo.commit("m14: widget whose committed bundle is served straight from dist/")?;

        Ok(GroundTruth {
            live_paths: vec![Path::new(LIVE).to_path_buf()],
            live_symbols: Vec::new(),
            decoy_dead_paths: Self::DECOYS
                .iter()
                .map(Path::new)
                .map(Path::to_path_buf)
                .collect(),
            decoy_dead_symbols: Self::DECOY_SYMBOLS
                .iter()
                .map(|symbol| (*symbol).to_string())
                .collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::support;

    /// `widget.7f3a91c.js` — the hashed basename, derived from [`LIVE`] rather
    /// than transcribed, so that changing the fixture's hash cannot leave the
    /// assertions quietly testing a filename that no longer exists.
    fn bundle_basename() -> &'static str {
        Path::new(LIVE)
            .file_name()
            .and_then(|name| name.to_str())
            .expect("LIVE has a UTF-8 basename")
    }

    /// `widget.7f3a91c` — the same name without its extension, which is the
    /// form a build script would spell if it knew the name at all.
    fn bundle_stem() -> &'static str {
        Path::new(LIVE)
            .file_stem()
            .and_then(|stem| stem.to_str())
            .expect("LIVE has a UTF-8 stem")
    }

    #[test]
    fn m14_commits_the_bundle_despite_it_looking_like_build_output() {
        let (_dir, repo, _truth) = support::materialize(&CheckedInGeneratedAsset);

        // If a stray ignore rule kept dist/ out of the index, the mutant would
        // be testing the ignore path (m13) instead of the CDN path.
        support::assert_committed(&repo, &[LIVE]);
    }

    #[test]
    fn m14_ground_truth_names_files_that_are_really_there() {
        let (_dir, repo, truth) = support::materialize(&CheckedInGeneratedAsset);

        assert_eq!(truth.live_paths, vec![Path::new(LIVE).to_path_buf()]);
        assert!(truth.live_symbols.is_empty());
        assert_eq!(
            truth.decoy_dead_paths.len(),
            CheckedInGeneratedAsset::DECOYS.len()
        );

        support::assert_ground_truth_is_on_disk(&repo, &truth);
    }

    #[test]
    fn m14_the_bundle_is_named_by_one_html_attribute_and_by_no_build_config() {
        let (_dir, repo, _truth) = support::materialize(&CheckedInGeneratedAsset);

        assert_eq!(
            support::references_outside(repo.root(), bundle_basename(), LIVE),
            vec![MECHANISM.to_string()],
            "only the script tag may name the bundle"
        );

        // Specifically not the build script, which is where a reader would
        // expect to find it and where a build-graph analysis would look.
        let build_config_hits = support::files_mentioning(repo.root(), bundle_stem());
        assert!(
            !build_config_hits.contains(&"package.json".to_string())
                && !build_config_hits.contains(&"tsconfig.json".to_string()),
            "the hashed name must not appear in build configuration; hits were {build_config_hits:?}"
        );
    }

    #[test]
    fn m14_the_stale_bundle_beside_it_really_is_unreferenced() {
        let (_dir, repo, truth) = support::materialize(&CheckedInGeneratedAsset);
        support::assert_decoys_are_unreferenced(&repo, &truth);
    }
}
