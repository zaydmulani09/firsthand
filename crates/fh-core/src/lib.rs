//! Core types for Firsthand: ids, hashes, timestamps, canonical JSON, the frozen
//! data model (packets, ledgers, claims, probes), policy, and path rules.
//!
//! Everything that is hashed or signed goes through [`canon::canonical_bytes`],
//! which implements RFC 8785 (JCS) and rejects floating point numbers.

pub mod canon;
pub mod error;
pub mod hash;
pub mod id;
pub mod model;
pub mod paths;
pub mod policy;
pub mod store;
pub mod time;

pub use error::{Error, ExitCode, Result};
pub use hash::{B3, GitOid};
pub use id::{Att, Clm, Dec, Id, IdGen, IdKind, Pkt, Snp, Tsk};
pub use model::*;
pub use time::{Clock, SystemClock, Ts};

/// Version of the firsthand tool, as recorded in packets.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// Packet format identifier.
pub const PACKET_FORMAT: &str = "firsthand.packet/1";
/// Ledger format identifier, used to derive the genesis `prev` hash.
pub const LEDGER_FORMAT: &str = "firsthand.ledger/1";
/// Schema version of every `--json` output.
pub const JSON_SCHEMA_VERSION: u32 = 1;

/// Detects secret-looking content. Implemented by `fh-redact`; used by snapshots
/// to exclude files before they enter a git object.
pub trait SecretScanner: Send + Sync {
    /// Returns the id of the first rule that matches, or `None`.
    fn scan(&self, bytes: &[u8]) -> Option<String>;
}
