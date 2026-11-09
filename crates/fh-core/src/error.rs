//! Error taxonomy and stable process exit codes.

use std::path::PathBuf;

/// Stable exit codes documented in `docs/cli.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ExitCode {
    /// Success.
    Ok = 0,
    /// Internal or uncategorized error.
    Internal = 1,
    /// Bad command line usage.
    Usage = 2,
    /// Verification found stale or unverified claims under `--fail-on`.
    Stale = 10,
    /// Verification found refuted claims.
    Refuted = 11,
    /// Packet integrity, signature or ledger chain failure.
    Integrity = 12,
    /// Policy denied a command, or approval is required in non-interactive mode.
    Policy = 13,
    /// Environment problem: git missing or too old, not a git repository.
    Environment = 14,
    /// Feature unavailable for that agent or platform.
    Unsupported = 15,
}

/// Result alias using [`Error`].
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Every error Firsthand can report. Messages start lowercase, have no trailing
/// period, and say what to do where possible.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A float appeared in a structure that is hashed or signed.
    #[error(
        "canonical json: floating point number at {path} (signed structures allow integers only)"
    )]
    FloatInCanonical {
        /// JSON pointer of the offending value.
        path: String,
    },
    /// An integer outside the interoperable range.
    #[error("canonical json: integer {value} at {path} is outside +/-(2^53 - 1)")]
    IntegerRange {
        /// The value as text.
        value: String,
        /// JSON pointer of the offending value.
        path: String,
    },
    /// A malformed identifier, hash or timestamp.
    #[error("invalid {what}: {value:?} ({reason})")]
    Invalid {
        /// What kind of value was invalid.
        what: &'static str,
        /// The offending value (never a secret).
        value: String,
        /// Why it was rejected.
        reason: String,
    },
    /// A path failed normalization rules.
    #[error("unsafe path {path:?}: {reason}")]
    UnsafePath {
        /// The path as given.
        path: String,
        /// Why it was rejected.
        reason: String,
    },
    /// git is missing, too old, or the directory is not a repository.
    #[error("{0}")]
    Environment(String),
    /// A git command failed.
    #[error("git {args} failed (exit {code:?}): {stderr}")]
    Git {
        /// The git arguments, joined by spaces.
        args: String,
        /// Exit code, if the process ran.
        code: Option<i32>,
        /// Trimmed stderr.
        stderr: String,
    },
    /// Packet integrity, signature or chain failure.
    #[error("integrity: {0}")]
    Integrity(String),
    /// Policy refused something.
    #[error("policy: {0}")]
    Policy(String),
    /// Feature unsupported for this agent or platform.
    #[error("unsupported: {0}")]
    Unsupported(String),
    /// Bad configuration file.
    #[error("config {path}: {reason}")]
    Config {
        /// File that failed to parse.
        path: PathBuf,
        /// Parser message.
        reason: String,
    },
    /// Placeholder used only before the owning track merges. Fails `xtask lint-placeholders` after G1.
    #[error("not implemented yet (owned by track {0})")]
    NotYetImplemented(&'static str),
    /// I/O failure with the path involved.
    #[error("io {path}: {source}")]
    Io {
        /// Path being accessed.
        path: PathBuf,
        /// Underlying error.
        #[source]
        source: std::io::Error,
    },
    /// JSON (de)serialization failure.
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    /// Anything else, with context.
    #[error("{0}")]
    Other(String),
}

impl Error {
    /// Exit code this error maps to.
    pub fn exit_code(&self) -> ExitCode {
        match self {
            Error::Environment(_) => ExitCode::Environment,
            Error::Integrity(_) => ExitCode::Integrity,
            Error::Policy(_) => ExitCode::Policy,
            Error::Unsupported(_) => ExitCode::Unsupported,
            Error::Git { .. } => ExitCode::Internal,
            _ => ExitCode::Internal,
        }
    }

    /// Helper to wrap an I/O error with its path.
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Error::Io {
            path: path.into(),
            source,
        }
    }
}
