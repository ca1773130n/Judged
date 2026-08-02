//! Class 13 — a file un-ignored by a `!` gitignore negation.
//!
//! **The mechanism.** The real Magento shape, four lines that have to be read
//! together:
//!
//! ```text
//! /media/*
//! !/media/customer
//! /media/customer/*
//! !/media/customer/.htaccess
//! ```
//!
//! Git cannot re-include a file whose parent directory is excluded, so
//! `/media/*` has to be undone for the directory before the carve-out inside it
//! can apply, and then re-narrowed. The last matching pattern wins, so
//! `media/customer/.htaccess` is **not ignored** while everything around it is.
//! `.vscode/settings.json` gets the same treatment from the second stanza.
//!
//! §6.17 measured this directly over the canonical github/gitignore corpus: 246
//! negation patterns across 41 of 312 templates (Prestashop 73, Magento 17).
//! This is not an exotic configuration, and the negated paths are things like
//! `.vscode/settings.json`, `var/logs/.gitkeep`, `/media/**/.htaccess` — "files
//! whose entire purpose is to exist".
//!
//! **Why every other signal misses it.** No language server, compiler, or
//! import graph has an opinion about `.htaccess` or `settings.json`; neither is
//! code. The one signal a cleaner *does* reach for here is ignore status, and
//! §6.17 is unambiguous that reading it wrong is the most seductive available
//! mistake: **ignore-status is per-FILE, never per-directory**. Answer
//! "is `media/customer/` ignored?" instead of "is this file ignored?" and both
//! live artifacts are deleted. §6.17's verdict on the shortcut that produces
//! that answer: "A gitignore-derived junk classifier that drops the `!` lines
//! deletes checked-in editor configuration and directory placeholders."
//!
//! **What is supposed to catch it.** `git check-ignore -v` on the file itself,
//! reading the *pattern* rather than the exit code — which is what
//! [`judged_core::git::Repo::recoverability`] does, and what the test below
//! pins. The ignored siblings planted next to each live file exist so that a
//! per-directory answer and a per-file answer cannot produce the same result.

use std::path::Path;

use judged_core::git::Repo;
use judged_core::{Error, Result};

use crate::mutant::{Declaration, Ecosystem, GroundTruth, Mutant};

/// `.vscode/settings.json` and `media/customer/.htaccess`: deliberately rescued
/// from a broad ignore rule, which makes the negation itself the statement
/// of intent. §6.17 and Gate 0g both live here.
pub struct GitignoreNegation;

/// Re-included by the Magento `/media/*` carve-out.
const LIVE_HTACCESS: &str = "media/customer/.htaccess";

/// Re-included by the `.vscode/*` carve-out.
const LIVE_EDITOR_CONFIG: &str = ".vscode/settings.json";

/// The one file that decides the fate of both.
const MECHANISM: &str = ".gitignore";

/// Ignored files planted beside the live ones, so that a per-directory answer
/// and a per-file answer cannot agree.
///
/// Kept separate from [`FILES`] rather than mixed in, because they are **not**
/// decoys and must never be listed as such: they are not dead, they are merely
/// untracked, and §8.1 makes untracked-or-ignored the class with *zero* recovery
/// path — the least safe thing in the repository to delete, not the safest.
const IGNORED_SIBLINGS: [(&str, &str); 3] = [
    (
        "media/customer/thumb_00042.png",
        "IGNORED SIBLING. A regenerated thumbnail, in the same directory as a\ntracked file. Per-directory ignore logic cannot tell them apart.\n",
    ),
    (
        "media/catalog/product/placeholder.jpg",
        "IGNORED SIBLING. Under a directory git never descends into, because\n/media/* excludes it and no negation brings it back.\n",
    ),
    (
        ".vscode/ipch.db",
        "IGNORED SIBLING. Editor cache, genuinely disposable, one directory\nentry away from configuration that is not.\n",
    ),
];

/// Files written into the mutant repository, as `(repo-relative path, body)`.
const FILES: &[(&str, &str)] = &[
    (
        MECHANISM,
        r#"# Magento's real shape (§6.17: /media/* with 17 negation carve-outs).
# Git cannot re-include a file whose parent directory is excluded, so the
# directory has to be un-excluded first and then re-narrowed. Delete any one
# of these four lines and a checked-in file changes status.
/media/*
!/media/customer
/media/customer/*
!/media/customer/.htaccess

# .vscode/settings.json is one of the 246 negations §6.17 counted across the
# canonical templates: checked-in editor configuration whose entire purpose is
# to exist. The cache database beside it really is disposable.
.vscode/*
!.vscode/settings.json
"#,
    ),
    (
        "composer.json",
        r#"{
  "name": "ledger/storefront",
  "type": "project",
  "license": "proprietary",
  "require": {
    "php": ">=8.2"
  }
}
"#,
    ),
    (
        "pub/index.php",
        r#"<?php
// Front controller. Served by the web server; called by nothing in this repo.
declare(strict_types=1);

echo json_encode(['ok' => true]);
"#,
    ),
    (
        LIVE_HTACCESS,
        r#"# LIVE. Tracked, because the last pattern that matches this path in the
# ignore file is a negation. It is what stops the customer upload directory
# from serving uploaded PHP, so its absence is a security incident rather
# than a crash -- nothing fails, and nothing tells you.
Options -Indexes
<FilesMatch "\.(php|phtml)$">
    Require all denied
</FilesMatch>
"#,
    ),
    (
        LIVE_EDITOR_CONFIG,
        r#"{
  "editor.formatOnSave": true,
  "files.trimTrailingWhitespace": true,
  "php.validate.executablePath": "/usr/bin/php"
}
"#,
    ),
    (
        "lib/OldShippingCalculator.php",
        r#"<?php
// DEAD DECOY. Tracked, referenced by no include, no autoload rule, no config
// key, and no negation. A tool that never says this is dead is safe and
// useless, which is the state the decoys exist to expose.
declare(strict_types=1);

final class OldShippingCalculator
{
    public function rate(float $weight): float
    {
        return $weight * 1.5;
    }
}
"#,
    ),
    (
        "pub/legacy_dispatch.php",
        r#"<?php
// DEAD DECOY. Second front controller from a routing scheme that is gone.
declare(strict_types=1);

echo 'gone';
"#,
    ),
];

impl GitignoreNegation {
    /// Repo-relative paths of the genuinely-dead files planted here.
    const DECOYS: [&'static str; 2] = ["lib/OldShippingCalculator.php", "pub/legacy_dispatch.php"];

    /// The symbol each decoy defines, index-aligned with [`Self::DECOYS`].
    ///
    /// The second is `""` on purpose: the dead front controller is a file that
    /// `echo`s and declares nothing, so it has no symbol route at all. Adding
    /// one to make the class score better would invent a route no tool can
    /// take.
    const DECOY_SYMBOLS: [&'static str; 2] = ["OldShippingCalculator", ""];
}

impl Mutant for GitignoreNegation {
    fn id(&self) -> &str {
        "m13"
    }
    fn ecosystem(&self) -> Ecosystem {
        Ecosystem::Polyglot
    }
    /// PHP, `composer.json`, and checked-in media. None of the four analyzers
    /// Judged adapts reads PHP, so every one of them skips this class — which
    /// is the honest answer rather than a gap in the catalogue. A tool that
    /// cannot parse the language cannot have an opinion about it, and grading
    /// its silence here would be §6.20's error exactly. The class still does
    /// its job against a language-agnostic cleaner: it is one of the five that
    /// must always catch [`crate::sut::NaiveSut`].
    fn languages(&self) -> &'static [Ecosystem] {
        &[]
    }
    fn mechanism(&self) -> &str {
        "file rescued from a broad ignore rule by an explicit ! negation"
    }
    fn research_ref(&self) -> &str {
        "§10 E2 class 13"
    }
    /// An `.htaccess` and an editor config, rescued from a broad ignore rule. Neither
    /// is code, and nothing executes them in a test process.
    fn coverage_declaration(&self) -> Declaration {
        Declaration::nothing()
    }

    fn materialize(&self, dir: &Path) -> Result<GroundTruth> {
        let repo = Repo::init(dir)?;
        for (relative, body) in FILES.iter().chain(IGNORED_SIBLINGS.iter()) {
            let path = repo.root().join(relative);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(|source| Error::Io {
                    path: parent.to_path_buf(),
                    source,
                })?;
            }
            std::fs::write(&path, body).map_err(|source| Error::Io { path, source })?;
        }
        // `add_all` is `git add --all`, deliberately not `-f` (§8.2). That is
        // load-bearing here and not a detail: the ignored siblings must stay out
        // of the index for the two live files to differ from them at all.
        repo.add_all()?;
        repo.commit("m13: storefront whose ignore file re-includes two tracked artifacts")?;

        Ok(GroundTruth {
            live_paths: vec![
                Path::new(LIVE_HTACCESS).to_path_buf(),
                Path::new(LIVE_EDITOR_CONFIG).to_path_buf(),
            ],
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
    use judged_core::git::RecoverabilityClass;

    #[test]
    fn m13_is_a_real_git_repository_and_the_negated_files_are_committed() {
        let (_dir, repo, _truth) = support::materialize(&GitignoreNegation);

        // If either is missing from HEAD, the negation did not take.
        support::assert_committed(&repo, &[LIVE_HTACCESS, LIVE_EDITOR_CONFIG]);
    }

    #[test]
    fn m13_ground_truth_names_files_that_are_really_there() {
        let (_dir, repo, truth) = support::materialize(&GitignoreNegation);

        assert_eq!(
            truth.live_paths,
            vec![
                Path::new(LIVE_HTACCESS).to_path_buf(),
                Path::new(LIVE_EDITOR_CONFIG).to_path_buf()
            ]
        );
        assert!(truth.live_symbols.is_empty());
        assert_eq!(
            truth.decoy_dead_paths.len(),
            GitignoreNegation::DECOYS.len()
        );

        support::assert_ground_truth_is_on_disk(&repo, &truth);

        // The ignored siblings are not ground truth — they are the other half
        // of the per-file/per-directory contrast — but they still have to be
        // on disk for the contrast to mean anything.
        for (sibling, _) in IGNORED_SIBLINGS {
            assert!(
                repo.root().join(sibling).is_file(),
                "{sibling} is not on disk"
            );
        }
    }

    #[test]
    fn m13_ignore_status_differs_per_file_inside_one_directory() {
        let (_dir, repo, _truth) = support::materialize(&GitignoreNegation);

        // The whole class in two assertions: same directory, opposite verdicts.
        // Any implementation that answers per-directory gets one of these wrong,
        // and §6.17 says which one it will be.
        for live in [LIVE_HTACCESS, LIVE_EDITOR_CONFIG] {
            assert_eq!(
                repo.recoverability(Path::new(live))
                    .expect("recoverability query succeeds"),
                RecoverabilityClass::TrackedUnpushed,
                "{live} is re-included by a ! negation and must classify as tracked"
            );
        }
        for (sibling, _) in IGNORED_SIBLINGS {
            assert_eq!(
                repo.recoverability(Path::new(sibling))
                    .expect("recoverability query succeeds"),
                RecoverabilityClass::Ignored,
                "{sibling} must classify as ignored"
            );
        }
    }

    #[test]
    fn m13_live_files_are_named_only_by_the_ignore_file() {
        let (_dir, repo, _truth) = support::materialize(&GitignoreNegation);

        for (live, basename) in [
            (LIVE_HTACCESS, ".htaccess"),
            (LIVE_EDITOR_CONFIG, "settings.json"),
        ] {
            assert_eq!(
                support::references_outside(repo.root(), basename, live),
                vec![MECHANISM.to_string()],
                "{live} must be named by {MECHANISM} and by nothing else"
            );
        }
    }

    #[test]
    fn m13_decoys_are_named_nowhere_at_all() {
        let (_dir, repo, truth) = support::materialize(&GitignoreNegation);
        support::assert_decoys_are_unreferenced(&repo, &truth);
    }
}
