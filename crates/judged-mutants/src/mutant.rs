//! What a mutant is, and what it promises about the repository it builds.
//!
//! §10 E2 borrows the mutation-based soundness methodology from the Android
//! static-analysis literature (muSE / Bonett et al., ACM TOSEM 3439802):
//! systematically inject known-live artifacts reachable only through **one
//! mechanism each**. Any "dead" verdict on an injected artifact is a hard
//! failure — not a tuning opportunity.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use judged_core::{Error, Result};

/// The language ecosystem a mutant exercises.
///
/// `Polyglot` is not a catch-all: several §10 classes are only expressible
/// across a language boundary, e.g. a CI manifest referencing a script, or a
/// gitignore negation that no language server ever sees.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ecosystem {
    Python,
    TypeScript,
    Rust,
    Go,
    Polyglot,
}

/// What a materialized mutant guarantees about itself.
///
/// The decoys are the load-bearing part. §3.7 and §9.8 require a positive
/// control on every evidence artifact, and the same logic applies one level up
/// to the suite: without genuinely-dead files planted in the repository, a tool
/// that refuses to call anything dead scores a perfect zero false removals and
/// looks indistinguishable from a tool that works. The decoys are what make
/// [`crate::sut::RefusingSut`] fail.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GroundTruth {
    /// Files that are genuinely reachable. Claiming any of these is dead is a
    /// false removal and fails the suite outright.
    pub live_paths: Vec<PathBuf>,
    /// Symbols that are genuinely reachable, same rule.
    pub live_symbols: Vec<String>,
    /// Files that really are dead. A tool that finds none of these has told us
    /// nothing, however safe it looks.
    pub decoy_dead_paths: Vec<PathBuf>,
    /// The symbol each decoy defines, **index-aligned with
    /// [`decoy_dead_paths`](Self::decoy_dead_paths)**: entry `i` is a symbol
    /// defined by decoy `i`, and `""` means that decoy has no symbol route at
    /// all.
    ///
    /// # Why this exists
    ///
    /// Without it, decoy recall asks a question only a *file*-level tool can
    /// answer. A symbol-level analyzer never claims a path, so it scored zero of
    /// every decoy in the catalogue — which reads on the scoreboard as "found
    /// nothing" when the truth is "was never asked a question it could answer".
    /// That is §6.20's category error ("no data" must be a distinct state from
    /// "zero executions") committed by the suite's own positive control, and it
    /// would be repeated by every symbol-level analyzer added after it.
    ///
    /// # One symbol per decoy, and what that costs
    ///
    /// A decoy usually defines several names — a module with a constant and a
    /// function, a Kotlin `object` with a property. Exactly one is declared: the
    /// file's primary definition, chosen from what the file *is* and never from
    /// what some tool happens to print. A tool that names a different symbol in
    /// the same file therefore earns no credit, so the recorded recall is a
    /// **floor** on the tool's real recall.
    ///
    /// That asymmetry is deliberate and it points the same way as
    /// [`live_symbols`](Self::live_symbols) matching: understating recall makes
    /// a working tool look more like one that refuses to answer, which pushes
    /// §11 R1 toward *deleting* the auto-act tier. Overstating it would push
    /// toward shipping one. Only the first error is survivable.
    ///
    /// # `""` is a route that does not exist, not a missing declaration
    ///
    /// Four of the catalogue's thirty-one decoys define nothing a symbol-level
    /// analyzer could name: a bash script, an nginx config, a second PHP front
    /// controller that only `echo`s, and a minified bundle whose only function
    /// is a single letter inside an IIFE. Inventing a symbol for those would be
    /// inventing a route no tool can take — the adapter rule of §9.2 ("not more
    /// careful than the tool, and not less") applied to the fixtures. They are
    /// declared `""` and are reachable by path alone.
    pub decoy_dead_symbols: Vec<String>,
}

/// What a test suite exercising one fixture's documented entry point actually
/// enters.
///
/// Empty by default, which is the conservative reading — a class declares no
/// execution until somebody states the mechanism reaches it. `fixtures::all()`
/// is pinned by a table in `tests/coverage_declarations.rs` so that a class
/// which simply forgot to declare fails loudly instead of quietly contributing
/// nothing (the §6.20 shape: "did not run" and "nobody said" must not share a
/// row).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Declaration {
    /// Live paths a test run loads. **Import counts** — a module that is
    /// imported and never otherwise touched has executed, and a claim that the
    /// file is dead is therefore wrong.
    pub covered_paths: Vec<PathBuf>,
    /// Live symbols a test run calls, each with the live path that declares it.
    ///
    /// The declaring file is carried rather than inferred because most fixtures
    /// have several live paths, and writing a function record into the wrong
    /// one would make the artifact quietly false about where the code lives.
    pub called_symbols: Vec<(PathBuf, String)>,
}

impl Declaration {
    /// Nothing is entered: the class's live artifacts are all reached by a
    /// mechanism no test process takes.
    pub fn nothing() -> Declaration {
        Declaration::default()
    }

    /// Files loaded, nothing called.
    pub fn loaded(paths: impl IntoIterator<Item = &'static str>) -> Declaration {
        Declaration {
            covered_paths: paths.into_iter().map(PathBuf::from).collect(),
            called_symbols: Vec::new(),
        }
    }

    /// Add a called symbol, and the file that declares it. The file is covered
    /// by implication — you cannot call into a module without loading it — so
    /// this records that too rather than making every caller repeat it.
    pub fn calling(mut self, file: &'static str, symbol: &'static str) -> Declaration {
        let file = PathBuf::from(file);
        if !self.covered_paths.contains(&file) {
            self.covered_paths.push(file.clone());
        }
        self.called_symbols.push((file, symbol.to_string()));
        self
    }

    /// Whether this class declares any execution at all.
    pub fn is_empty(&self) -> bool {
        self.covered_paths.is_empty() && self.called_symbols.is_empty()
    }

    /// Reject a declaration that does not describe the fixture it belongs to.
    ///
    /// The three constraints from the module docs, checked rather than trusted.
    /// A generator whose only guard is the care of whoever writes the next
    /// fixture is a generator that will eventually plant coverage on a decoy,
    /// and the run that does it will look like a success.
    pub fn check(&self, mutant_id: &str, truth: &GroundTruth) -> Result<()> {
        let live_paths: BTreeSet<&Path> = truth.live_paths.iter().map(PathBuf::as_path).collect();
        let live_symbols: BTreeSet<&str> = truth.live_symbols.iter().map(String::as_str).collect();
        let decoys: BTreeSet<&Path> = truth
            .decoy_dead_paths
            .iter()
            .map(PathBuf::as_path)
            .collect();

        let refuse = |message: String| {
            Err(Error::Fixture {
                mutant_id: mutant_id.to_string(),
                message,
            })
        };

        for path in &self.covered_paths {
            // Checked before the live-path membership test, so the message names
            // the worse mistake when a path is somehow both.
            if decoys.contains(path.as_path()) {
                return refuse(format!(
                    "the coverage declaration marks the decoy {} as executed. A decoy is \
                     genuinely dead; an artifact showing one executed is a false statement \
                     about the fixture, and decoy recall would stop meaning anything.",
                    path.display()
                ));
            }
            if !live_paths.contains(path.as_path()) {
                return refuse(format!(
                    "the coverage declaration covers {}, which is not one of this class's \
                     live paths. A generated artifact may only ever describe what the \
                     fixture already declares.",
                    path.display()
                ));
            }
        }

        for (file, symbol) in &self.called_symbols {
            if !live_symbols.contains(symbol.as_str()) {
                return refuse(format!(
                    "the coverage declaration calls {symbol}, which is not one of this \
                     class's live symbols."
                ));
            }
            if !live_paths.contains(file.as_path()) {
                return refuse(format!(
                    "the coverage declaration says {} declares {symbol}, but that file is \
                     not one of this class's live paths.",
                    file.display()
                ));
            }
        }

        Ok(())
    }
}

/// One injected liveness mechanism.
pub trait Mutant {
    /// Stable identifier, `m01`..`m19`. Used in reports and in release gating.
    fn id(&self) -> &str;

    /// The ecosystem the mutant is written in.
    fn ecosystem(&self) -> Ecosystem;

    /// The language toolchains that can load this mutant's repository at all.
    ///
    /// Distinct from [`ecosystem`](Self::ecosystem), which names the class's
    /// *character* and may be `Polyglot`. This names what an analyzer needs to
    /// be able to parse in order to have any opinion, and it is what
    /// [`crate::runner::run_suite`] matches against [`crate::sut::Sut::reads`]
    /// to decide whether a class is graded or skipped.
    ///
    /// # Why the two cannot be the same method
    ///
    /// `Polyglot` is not a toolchain. Five classes in the catalogue carry it
    /// and they contain five different mixtures: m02 is Python plus
    /// TypeScript, m10 Python plus JavaScript, m08 Python plus shell and CI
    /// YAML, m18 Python plus Kotlin, m13 PHP plus checked-in media. Measured
    /// 2026-08-01, knip 6.31.0 loads m02, m10 and m14 and exits 2 with
    /// `Unable to find package.json` on m08, m13 and m18 — so a SUT that
    /// matched on `Polyglot` would be handed three repositories it cannot read
    /// in order to be handed the two it can. Collapsing the distinction in
    /// either direction costs something real: too wide aborts the run, too
    /// narrow drops a class the tool genuinely reads, and a dropped class is a
    /// false removal that never gets counted.
    ///
    /// # The default, and why `Polyglot` defaults to nothing
    ///
    /// A single-language class is loadable by its own toolchain and no other,
    /// so it does not repeat itself. A polyglot class has an answer that cannot
    /// be derived, and the default is therefore the empty set: read by no
    /// language-specific analyzer at all. That is the conservative reading —
    /// silence rather than a guess — and it is deliberately not silent about
    /// being wrong, because `runner_capability.rs` pins every fixture's answer
    /// as a matrix that a missing override fails.
    fn languages(&self) -> &'static [Ecosystem] {
        match self.ecosystem() {
            Ecosystem::Python => &[Ecosystem::Python],
            Ecosystem::TypeScript => &[Ecosystem::TypeScript],
            Ecosystem::Rust => &[Ecosystem::Rust],
            Ecosystem::Go => &[Ecosystem::Go],
            Ecosystem::Polyglot => &[],
        }
    }

    /// The single mechanism by which the live artifact is reachable. One
    /// mechanism per mutant is the whole methodology — a mutant reachable two
    /// ways cannot tell you which signal caught it.
    fn mechanism(&self) -> &str;

    /// Where this class comes from in the research document, so a failure can
    /// be traced back to the documented real-world incident it encodes.
    fn research_ref(&self) -> &str;

    /// Build the mutant repository under `dir` and declare its ground truth.
    fn materialize(&self, dir: &Path) -> Result<GroundTruth>;

    /// Of this class's live artifacts, which a test suite exercising its
    /// documented entry point actually enters (§9.5, Family X).
    ///
    /// Answered from the injected [`mechanism`](Self::mechanism) and from
    /// nothing else. It is a property of how the artifact is reached, not a
    /// threshold and not a knob: m12's `//go:linkname` alias is called through
    /// at runtime, so a test enters it; m05's recovery handler is entered by no
    /// test that does not inject a fault; m08's script runs in a pipeline and
    /// not in a test process. The rule and its pre-commitment are in
    /// `docs/decisions/2026-08-02-e2-coverage-artifacts.md`.
    ///
    /// # The default is nothing, and a missing override is a failure
    ///
    /// [`Declaration::nothing`], exactly as [`languages`](Self::languages)
    /// defaults `Polyglot` to the empty set: silence rather than a guess. And,
    /// like that one, silence is not allowed to pass unnoticed — every class's
    /// answer is pinned as a table in `tests/coverage_declarations.rs`, so a
    /// fixture that simply never declared fails rather than quietly
    /// contributing no coverage. A class that had nothing to say and a class
    /// nobody asked are the §6.20 pair, and they must not share a row.
    fn coverage_declaration(&self) -> Declaration {
        Declaration::nothing()
    }
}
