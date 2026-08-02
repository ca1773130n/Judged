//! Class 12 — a symbol aliased via `//go:linkname` / `extern "C"` / `#[no_mangle]`.
//!
//! **The mechanism.** `internal/sampler/drain.go` defines an *unexported*
//! `drain`. Nothing in the module calls it — nothing outside its own package
//! could, and its own package does not. The linker connects it to
//! `collector.drainSamples`, a body-less declaration carrying
//! `//go:linkname drainSamples …sampler.drain`. To the Go type checker that
//! directive is a comment, so the call graph has no edge at all.
//!
//! `cmd/libtelemetry/abi.go` carries the second half of the class: a cgo
//! `//export TelemetryFlush` in a `main` package built with
//! `-buildmode=c-shared`. Same shape, other end of the same wire — a name
//! bound at link time by something that is not an import.
//!
//! **Why every other signal misses it.** §4.1 records both failures by name.
//! `x/tools/cmd/deadcode`: `//go:linkname` aliasing → *"spuriously reported as
//! dead"*, and it lists assembly/cgo callers as the same blind spot.
//! `staticcheck` U1000 says it in its own source: *"we cannot observe function
//! calls in assembly files"* — and this package contains an assembly file,
//! because a body-less Go declaration is legal only alongside one. Deleting
//! `drain` leaves every package type-checking and `go vet` clean; it fails at
//! **link** time, in the build that ships rather than the build that tests.
//!
//! **What is supposed to catch it.** For `drain`, the whole-repo literal veto
//! of §6.20 reading directives as text: the qualified target name is spelled
//! out in the directive, so a literal search finds it even though no parser
//! does. For `TelemetryFlush` nothing in the repository corroborates the
//! `//export` line — §10 E2 class 19's point, arriving early: that shape is
//! unfalsifiable from inside the repo by construction, so the only correct
//! behaviour is to refuse.

use std::path::{Path, PathBuf};

use judged_core::git::Repo;
use judged_core::{Error, Result};

use crate::mutant::{Declaration, Ecosystem, GroundTruth, Mutant};

/// `//go:linkname` binds a name across package boundaries at link time. The
/// Go call graph shows nothing; the program depends on it.
pub struct LinknameAlias;

/// Repo-relative path of the artifact that is alive and looks dead.
const LIVE: &str = "internal/sampler/drain.go";

/// The symbol inside [`LIVE`] that only the linker connects to a caller.
const LIVE_SYMBOL: &str = "drain";

/// The one file that names [`LIVE_SYMBOL`] — inside a comment, at that.
const MECHANISM: &str = "internal/collector/collector.go";

/// The directive in [`MECHANISM`], verbatim. It is the sole rescue signal for
/// [`LIVE`], and it is legible only to a literal search.
///
/// `cfg(test)`, because it names an invariant rather than any file's contents.
#[cfg(test)]
const LINKNAME_DIRECTIVE: &str =
    "//go:linkname drainSamples example.com/m12/telemetry/internal/sampler.drain";

/// The empty assembly file that makes the body-less declaration legal — and
/// that staticcheck says, in its own source, it cannot read.
const ASSEMBLY_STUB: &str = "internal/collector/asm_stub.s";

/// The second live artifact: an entry point the C agent calls by symbol name.
const LIVE_ABI: &str = "cmd/libtelemetry/abi.go";

/// The symbol inside [`LIVE_ABI`]. Its only in-repo evidence is the directive
/// sitting directly above it, in the same file.
const ABI_SYMBOL: &str = "TelemetryFlush";

/// Files written into the mutant repository, as `(repo-relative path, body)`.
const FILES: &[(&str, &str)] = &[
    ("go.mod", "module example.com/m12/telemetry\n\ngo 1.22\n"),
    (
        "cmd/telemetryd/main.go",
        r#"// Command telemetryd records latencies and prints what the collector has.
//
// Everything reachable in this module is reachable from here -- except the one
// function the linker attaches to the collector.
package main

import (
	"fmt"

	"example.com/m12/telemetry/internal/collector"
	"example.com/m12/telemetry/internal/sampler"
)

func main() {
	sampler.Record(1200)
	sampler.Record(830)
	fmt.Println(collector.Snapshot())
}
"#,
    ),
    (
        "internal/sampler/reservoir.go",
        r#"// Package sampler buffers latency observations for the collector.
package sampler

// buffer holds every observation taken since the process started, in
// microseconds.
var buffer []int

// Record adds one observation. The command calls this on every request, so the
// package is unambiguously alive and only one file inside it is at stake.
func Record(us int) {
	buffer = append(buffer, us)
}
"#,
    ),
    // THE LIVE ARTIFACT.
    (
        LIVE,
        r#"package sampler

// LIVE. Unexported, so nothing outside this package could call it even in
// principle, and nothing inside it does. The collector binds to it at link
// time through the //go:linkname directive in internal/collector.
//
// Removing this file leaves every package type-checking and `go vet` clean.
// It fails at link time, which is to say in the build that ships.
func drain() []int {
	drained := buffer
	buffer = nil
	return drained
}
"#,
    ),
    // THE MECHANISM. One directive, in one comment, in one file.
    (
        MECHANISM,
        r#"// Package collector renders what the sampler has buffered.
package collector

import (
	"fmt"
	_ "unsafe" // for go:linkname

	// Blank import: the linkname target has to be part of the build for the
	// linker to find it. It binds the package, never the function.
	_ "example.com/m12/telemetry/internal/sampler"
)

// drainSamples is bound by the linker to an unexported function in another
// package. There is no import edge and no call edge here: to the type checker
// the line below is a comment, which is exactly why x/tools deadcode reports
// its target as dead.
//
//go:linkname drainSamples example.com/m12/telemetry/internal/sampler.drain
func drainSamples() []int

// Snapshot renders the buffered observations.
func Snapshot() string {
	return fmt.Sprint(drainSamples())
}
"#,
    ),
    (
        ASSEMBLY_STUB,
        r#"// Intentionally empty.
//
// A Go package may declare a function without a body only if it also contains
// an assembly file. staticcheck's own source says "we cannot observe function
// calls in assembly files", so the presence of this file is one more reason
// its verdict on this package cannot be taken as evidence.
"#,
    ),
    // THE SECOND LIVE ARTIFACT, the extern "C" half of the class.
    (
        LIVE_ABI,
        r#"// Command libtelemetry is built with `go build -buildmode=c-shared`: cgo
// requires exported functions to live in a main package, and main never runs.
package main

/*
#include <stdlib.h>
*/
import "C"

import "example.com/m12/telemetry/internal/collector"

// TelemetryFlush is called by the C agent that dlopens the shared library. The
// //export directive is the whole of its in-repo evidence, and it sits in this
// same file -- nothing else in the repository corroborates it, and nothing
// could.
//
//export TelemetryFlush
func TelemetryFlush() *C.char {
	return C.CString(collector.Snapshot())
}

func main() {}
"#,
    ),
    (
        "internal/sampler/legacy_histogram.go",
        r#"package sampler

// DEAD DECOY. Superseded by the reservoir. No caller, no directive, no build
// tag: a cleaner that never says this is dead has told us nothing, however
// clean its false-removal record looks.
func legacyHistogram(buckets []int) map[int]int {
	counts := make(map[int]int)
	for _, bucket := range buckets {
		counts[bucket]++
	}
	return counts
}
"#,
    ),
    (
        "internal/collector/unused_percentile.go",
        r#"package collector

// DEAD DECOY. A second one on purpose: decoy recall is a rate, and one decoy
// cannot tell a tool that reasoned from a tool that guessed once.
func unusedPercentile(sorted []int, p int) int {
	if len(sorted) == 0 {
		return 0
	}
	return sorted[(len(sorted)-1)*p/100]
}
"#,
    ),
];

impl LinknameAlias {
    /// Repo-relative paths of the genuinely-dead files planted here. Both are
    /// compiled — Go does not complain about an uncalled function — and
    /// neither is named by anything.
    const DECOYS: [&'static str; 2] = [
        "internal/sampler/legacy_histogram.go",
        "internal/collector/unused_percentile.go",
    ];

    /// The symbol each decoy defines, index-aligned with [`Self::DECOYS`].
    /// Without these a symbol-level analyzer scores zero decoys here and reads
    /// as having found nothing (see `GroundTruth::decoy_dead_symbols`).
    const DECOY_SYMBOLS: [&'static str; 2] = ["legacyHistogram", "unusedPercentile"];
}

impl Mutant for LinknameAlias {
    fn id(&self) -> &str {
        "m12"
    }
    fn ecosystem(&self) -> Ecosystem {
        Ecosystem::Go
    }
    fn mechanism(&self) -> &str {
        "symbol bound through a //go:linkname alias rather than an import"
    }
    fn research_ref(&self) -> &str {
        "§10 E2 class 12"
    }
    /// `drain` is called through the `//go:linkname` alias at runtime, so a test
    /// process enters it and Go's coverage records the call.
    ///
    /// `TelemetryFlush` in `cmd/libtelemetry/abi.go` is not declared: it is an ABI
    /// export whose consumer is outside the repository, which is m19's situation
    /// and gets m19's answer.
    fn coverage_declaration(&self) -> Declaration {
        Declaration::default().calling("internal/sampler/drain.go", "drain")
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
        repo.commit("m12: telemetry module whose sampler is bound only by the linker")?;

        Ok(GroundTruth {
            // Repo-relative, because the runner keys ground truth and SUT
            // claims on the same repo-relative rendering and the fixture's own
            // canonicalized root is not the path the runner holds.
            //
            // Two live files, one class: §10 E2 class 12 names `//go:linkname`
            // and `extern "C"` together because they are the same failure —
            // a name the linker resolves and no parser does.
            live_paths: vec![PathBuf::from(LIVE), PathBuf::from(LIVE_ABI)],
            live_symbols: vec![LIVE_SYMBOL.to_string(), ABI_SYMBOL.to_string()],
            decoy_dead_paths: Self::DECOYS.iter().copied().map(PathBuf::from).collect(),
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
    fn m12_is_a_real_git_repository_whose_live_artifacts_are_committed() {
        let (_dir, repo, _truth) = support::materialize(&LinknameAlias);
        support::assert_committed(&repo, &[LIVE, LIVE_ABI]);
    }

    #[test]
    fn m12_ground_truth_names_files_that_are_really_there() {
        let (_dir, repo, truth) = support::materialize(&LinknameAlias);

        assert_eq!(
            truth.live_paths,
            vec![PathBuf::from(LIVE), PathBuf::from(LIVE_ABI)]
        );
        assert_eq!(
            truth.live_symbols,
            vec![LIVE_SYMBOL.to_string(), ABI_SYMBOL.to_string()]
        );
        assert_eq!(truth.decoy_dead_paths.len(), LinknameAlias::DECOYS.len());

        support::assert_ground_truth_is_on_disk(&repo, &truth);
    }

    #[test]
    fn m12_the_aliased_function_has_no_caller_outside_the_directive() {
        let (_dir, repo, _truth) = support::materialize(&LinknameAlias);

        // The definition and the one directive, and nothing else. In
        // particular not `cmd/telemetryd/main.go`: a Go call site would give
        // the type checker an edge and the mutant would be testing nothing.
        assert_eq!(
            support::files_mentioning(repo.root(), LIVE_SYMBOL),
            vec![MECHANISM.to_string(), LIVE.to_string()],
            "only the linkname directive and the definition may name the target"
        );

        // And it is only *fair* if this holds: §6.20's whole-repo literal veto,
        // applied to the qualified name, does find it. A mutant nothing can
        // solve measures nothing.
        assert_eq!(
            support::files_mentioning(repo.root(), LINKNAME_DIRECTIVE),
            vec![MECHANISM.to_string()],
            "the directive is the one rescue signal and must be spelled in full"
        );
    }

    #[test]
    fn m12_the_c_export_is_corroborated_by_nothing_at_all() {
        let (_dir, repo, _truth) = support::materialize(&LinknameAlias);

        // The `//export` line and the definition it decorates are the same
        // file. This is the class at its most honest: there is no second
        // opinion available inside the repository, so a tool that answers
        // "dead" here is guessing, not measuring.
        assert_eq!(
            support::files_mentioning(repo.root(), ABI_SYMBOL),
            vec![LIVE_ABI.to_string()],
            "nothing outside the exported file may name the exported symbol"
        );
    }

    #[test]
    fn m12_neither_live_file_is_named_by_its_filename() {
        let (_dir, repo, _truth) = support::materialize(&LinknameAlias);

        // Go resolves packages by directory and symbols by directive; neither
        // spells a filename. So a cleaner that greps for `drain.go` or
        // `abi.go` before deleting either finds nothing to stop it, which is
        // what makes these mutants hard rather than merely unusual.
        for live in [LIVE, LIVE_ABI] {
            let basename = Path::new(live)
                .file_name()
                .and_then(|name| name.to_str())
                .expect("live path has a UTF-8 basename");
            assert!(
                support::files_mentioning(repo.root(), basename).is_empty(),
                "{basename} must be spelled nowhere; nothing links Go files by name"
            );
        }
    }

    #[test]
    fn m12_decoys_are_named_nowhere_at_all() {
        let (_dir, repo, truth) = support::materialize(&LinknameAlias);
        support::assert_decoys_are_unreferenced(&repo, &truth);
    }
}
