//! Class 18 — an entry point declared only in a platform-side manifest *(§5.2)*.
//!
//! **The mechanism.** Two platforms, one shape: a file the *platform* reads
//! names an entry point that the *program* never mentions.
//!
//! - `vendor/site-packages/zzz_ledger_bootstrap.pth` contains one line,
//!   `import ledger_startup_hook`. CPython's `site` module globs `*.pth` out of
//!   every site directory at interpreter start and executes any line beginning
//!   with `import`. §5.2 states the consequence plainly: **a `.pth` file is an
//!   entry point with no caller anywhere.** Nothing names the `.pth` — not the
//!   packaging metadata, not a script, nothing. Its name is not even meaningful
//!   beyond sorting order.
//! - `android/app/src/main/AndroidManifest.xml` declares
//!   `<receiver android:name=".ota.OtaUpdateReceiver">`. Android instantiates
//!   the class by name when a matching broadcast arrives. §5.2 calls the
//!   Android manifest root set "**the single largest string-referenced root set
//!   in the JVM world**".
//!
//! **Why every other signal misses it.** These are §10 E2's structurally
//! different half: the caller does not exist in the repository at all. There is
//! no import edge to follow, because the importer is `site.py` inside the
//! interpreter and `ActivityThread` inside the Android runtime. Filename search
//! fails too, in the way that matters: the `.pth` line names a *module*
//! (`ledger_startup_hook`, no extension) and the manifest names a *class*
//! relative to the package (`.ota.OtaUpdateReceiver`, no path, no `.kt`), so
//! neither basename occurs anywhere in the repository.
//!
//! **What is supposed to catch it.** §5.2's root checklist, used as a checklist:
//! a `.pth` under a site directory and every `android:name` in an
//! `AndroidManifest.xml` are *roots*, enumerated before any reachability pass
//! runs, not conclusions reached by one. Failing that, §6.20's whole-repo
//! literal veto on the module name and the class name — which the tests below
//! confirm is available, so the mutant is hard but not unfair. The `.pth` file
//! itself has no such rescue: nothing names it, by construction. Only the
//! checklist saves it.

use std::path::Path;

use judged_core::git::Repo;
use judged_core::{Error, Result};

use crate::mutant::{Declaration, Ecosystem, GroundTruth, Mutant};

/// The site-directory manifest. §5.2: an entry point with no caller anywhere.
const LIVE_PTH: &str = "vendor/site-packages/zzz_ledger_bootstrap.pth";

/// The module that `.pth` line imports, named nowhere else.
const LIVE_PYTHON_HOOK: &str = "vendor/site-packages/ledger_startup_hook.py";

/// The broadcast receiver named only by the Android manifest.
const LIVE_RECEIVER: &str = "android/app/src/main/java/com/example/ledger/ota/OtaUpdateReceiver.kt";

/// The manifest that names it.
const MECHANISM_ANDROID: &str = "android/app/src/main/AndroidManifest.xml";

/// An Android `<receiver>`, a `.pth` file, an `NSExtensionPrincipalClass`, a
/// `META-INF/…AutoConfiguration.imports` line, a `[ModuleInitializer]`. The
/// platform, not the program, does the calling.
pub struct PlatformSideManifest;

/// Files written into the mutant repository, as `(repo-relative path, body)`.
const FILES: &[(&str, &str)] = &[
    (
        "pyproject.toml",
        r#"[project]
name = "ledger-agent"
version = "0.1.0"
requires-python = ">=3.11"

[build-system]
requires = ["setuptools>=68"]
build-backend = "setuptools.build_meta"
"#,
    ),
    ("app/__init__.py", "\"\"\"Ledger agent.\"\"\"\n"),
    (
        "app/main.py",
        r#""""The application, as far as any import graph can see.

It does not install the telemetry hook, does not import it, and does not know
it exists: by the time this module runs, the hook has already run.
"""


def main() -> int:
    print("ledger agent")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
"#,
    ),
    (
        // A `.pth` is a manifest, not a script: CPython's site module reads
        // every line, and executes the ones that begin with `import`. The
        // leading `zzz_` is the only control anyone has over ordering, which is
        // also why the filename carries no information a searcher could use.
        LIVE_PTH,
        "import ledger_startup_hook\n",
    ),
    (
        LIVE_PYTHON_HOOK,
        r#""""LIVE. Executed during interpreter startup, before any application
module is imported, because a .pth file in this directory says so.

Nothing imports this module. No console_scripts entry point names it, no test
touches it, and removing it leaves every import in the repository resolvable.
The failure is that telemetry silently stops.
"""

import sys


def install() -> None:
    sys.__ledger_telemetry_installed__ = True


install()
"#,
    ),
    (
        "android/settings.gradle.kts",
        r#"rootProject.name = "ledger-android"
include(":app")
"#,
    ),
    (
        "android/app/build.gradle.kts",
        r#"plugins {
    id("com.android.application")
    kotlin("android")
}

android {
    namespace = "com.example.ledger"
    compileSdk = 34

    defaultConfig {
        applicationId = "com.example.ledger"
        minSdk = 26
    }
}
"#,
    ),
    (
        MECHANISM_ANDROID,
        r#"<?xml version="1.0" encoding="utf-8"?>
<manifest xmlns:android="http://schemas.android.com/apk/res/android">
    <application android:label="Ledger">
        <!--
          The class below is instantiated by the Android runtime when a matching
          broadcast arrives. This attribute is the only reference to it in the
          repository, and it is relative to the package: no path, no extension.
        -->
        <receiver
            android:name=".ota.OtaUpdateReceiver"
            android:exported="true">
            <intent-filter>
                <action android:name="android.intent.action.MY_PACKAGE_REPLACED" />
            </intent-filter>
        </receiver>
    </application>
</manifest>
"#,
    ),
    (
        LIVE_RECEIVER,
        r#"package com.example.ledger.ota

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent

// LIVE. Constructed by the Android runtime from the android:name attribute in
// AndroidManifest.xml. No Kotlin or Java source in this project references it,
// and R8 would strip it too without the manifest-derived keep rules.
class OtaUpdateReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action == Intent.ACTION_MY_PACKAGE_REPLACED) {
            context.getSharedPreferences("ota", Context.MODE_PRIVATE)
                .edit()
                .putBoolean("updated", true)
                .apply()
        }
    }
}
"#,
    ),
    (
        "app/legacy_metrics_export.py",
        r#""""DEAD DECOY. No manifest names it, no .pth imports it, no module
imports it. It is what a working tool is supposed to find.
"""


def export(rows: list[dict]) -> int:
    return len(rows)
"#,
    ),
    (
        "android/app/src/main/java/com/example/ledger/ota/UnusedBackoffTable.kt",
        r#"package com.example.ledger.ota

// DEAD DECOY. Same package and same directory as a class only the manifest
// keeps alive. Distinguishing the two is the whole measurement.
object UnusedBackoffTable {
    val delaysSeconds = listOf(1, 2, 4, 8)
}
"#,
    ),
];

impl PlatformSideManifest {
    /// Repo-relative paths of the genuinely-dead files planted here.
    const DECOYS: [&'static str; 2] = [
        "app/legacy_metrics_export.py",
        "android/app/src/main/java/com/example/ledger/ota/UnusedBackoffTable.kt",
    ];

    /// The symbol each decoy defines, index-aligned with [`Self::DECOYS`].
    ///
    /// `export` is the Python module's only definition, and it is also a
    /// substring of `android:exported` in the manifest next door. That costs
    /// nothing here — grading matches whole symbol segments, not substrings —
    /// but it is why the suite does not assert that a decoy symbol appears in
    /// no other file: a byte search cannot tell a symbol from a prefix of an
    /// XML attribute name.
    const DECOY_SYMBOLS: [&'static str; 2] = ["export", "UnusedBackoffTable"];
}

impl Mutant for PlatformSideManifest {
    fn id(&self) -> &str {
        "m18"
    }
    fn ecosystem(&self) -> Ecosystem {
        Ecosystem::Polyglot
    }
    /// A Python package plus a Kotlin/Gradle Android half. Kotlin has no
    /// analyzer in this build, and there is no `package.json` — measured
    /// 2026-08-01, knip 6.31.0 exits 2 here — so Python is the whole of what
    /// can be read.
    fn languages(&self) -> &'static [Ecosystem] {
        &[Ecosystem::Python]
    }
    fn mechanism(&self) -> &str {
        "entry point declared only in a platform manifest the platform reads"
    }
    fn research_ref(&self) -> &str {
        "§10 E2 class 18"
    }
    /// Both manifests are read by the **platform** — CPython's `site` module at
    /// interpreter start, Android when a broadcast arrives. Neither happens inside
    /// the repository's test process.
    fn coverage_declaration(&self) -> Declaration {
        Declaration::nothing()
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
        repo.commit("m18: agent whose two entry points exist only in platform manifests")?;

        Ok(GroundTruth {
            live_paths: vec![
                Path::new(LIVE_PTH).to_path_buf(),
                Path::new(LIVE_PYTHON_HOOK).to_path_buf(),
                Path::new(LIVE_RECEIVER).to_path_buf(),
            ],
            live_symbols: vec!["OtaUpdateReceiver".to_string()],
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

    #[test]
    fn m18_is_a_real_git_repository_with_every_entry_point_committed() {
        let (_dir, repo, _truth) = support::materialize(&PlatformSideManifest);
        support::assert_committed(&repo, &[LIVE_PTH, LIVE_PYTHON_HOOK, LIVE_RECEIVER]);
    }

    #[test]
    fn m18_ground_truth_names_files_that_are_really_there() {
        let (_dir, repo, truth) = support::materialize(&PlatformSideManifest);

        assert_eq!(
            truth.live_paths,
            vec![
                Path::new(LIVE_PTH).to_path_buf(),
                Path::new(LIVE_PYTHON_HOOK).to_path_buf(),
                Path::new(LIVE_RECEIVER).to_path_buf(),
            ]
        );
        assert_eq!(truth.live_symbols, vec!["OtaUpdateReceiver".to_string()]);
        assert_eq!(
            truth.decoy_dead_paths.len(),
            PlatformSideManifest::DECOYS.len()
        );

        support::assert_ground_truth_is_on_disk(&repo, &truth);
    }

    #[test]
    fn m18_the_pth_file_is_named_by_absolutely_nothing() {
        let (_dir, repo, _truth) = support::materialize(&PlatformSideManifest);

        // §5.2's exact claim, asserted rather than assumed. This is the one
        // artifact in the mutant with no textual rescue at all: only an
        // enumerated root checklist saves it.
        assert!(
            support::references_outside(repo.root(), "zzz_ledger_bootstrap", LIVE_PTH).is_empty(),
            "the .pth file must have no caller and no mention anywhere"
        );
    }

    #[test]
    fn m18_manifest_named_entry_points_are_invisible_to_a_basename_search() {
        let (_dir, repo, _truth) = support::materialize(&PlatformSideManifest);

        for (live, basename) in [
            (LIVE_PYTHON_HOOK, "ledger_startup_hook.py"),
            (LIVE_RECEIVER, "OtaUpdateReceiver.kt"),
        ] {
            let elsewhere = support::references_outside(repo.root(), basename, live);
            assert!(
                elsewhere.is_empty(),
                "{live} must not be named by filename anywhere; found {elsewhere:?}"
            );
        }

        // The rescue signals, so the mutant is hard rather than impossible: the
        // module name is in the .pth line and the class name is in the manifest.
        assert!(
            support::files_mentioning(repo.root(), "ledger_startup_hook")
                .contains(&LIVE_PTH.to_string()),
            "the .pth line is the module's one rescue signal"
        );
        assert!(
            support::files_mentioning(repo.root(), "OtaUpdateReceiver")
                .contains(&MECHANISM_ANDROID.to_string()),
            "the android:name attribute is the receiver's one rescue signal"
        );
    }

    #[test]
    fn m18_decoys_are_named_nowhere_at_all() {
        let (_dir, repo, truth) = support::materialize(&PlatformSideManifest);
        support::assert_decoys_are_unreferenced(&repo, &truth);
    }
}
