//! One error type for the whole workspace.
//!
//! Every failure carries the thing it failed on. §12 of the research is blunt
//! about silent failure being the dominant catastrophic-deletion mechanism: a
//! tool that cannot tell "scanned everything, found nothing" apart from "failed
//! to start" deletes the repository. The same discipline applies to our own
//! plumbing, so there is no stringly-typed catch-all variant and no `Other`.

use std::path::PathBuf;

/// Everything that can go wrong inside Judged.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Filesystem failure. The path is part of the error because "No such file
    /// or directory" without one is unactionable.
    #[error("i/o error on {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// A JSON or JSONL document did not parse. `context` names the document.
    #[error("malformed JSON in {context}: {source}")]
    Json {
        context: String,
        #[source]
        source: serde_json::Error,
    },

    /// A git operation failed or returned something unparseable.
    #[error("git: {0}")]
    Git(String),

    /// Well-formed JSON that is not a SARIF log we can use — a missing
    /// invocation, an absent `analysisTarget` set. Distinct from [`Error::Json`]
    /// because it is a contract violation by the adapter (§9.2), not a parse
    /// error, and the two get different remediation.
    #[error("SARIF contract violation: {0}")]
    Sarif(String),

    /// A mutant could not be materialized. A suite that silently skips mutants
    /// reports a pass it did not earn, so this must never be swallowed (§10 E2).
    #[error("mutant {mutant_id}: {message}")]
    Fixture { mutant_id: String, message: String },

    /// The system under test failed to run or produced output we cannot read.
    #[error("system under test {sut}: {message}")]
    Sut { sut: String, message: String },

    /// The ratchet baseline is unusable.
    #[error("baseline: {0}")]
    Baseline(String),
}

/// Workspace-wide result alias.
pub type Result<T> = std::result::Result<T, Error>;
