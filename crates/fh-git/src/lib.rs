//! Git CLI plumbing for Firsthand. Every call goes through [`GitRunner`], which
//! hardens the environment and reads NUL-delimited output as bytes.
//!
//! No libgit2: behavior matches what users run, and Windows builds stay simple.

pub mod advanced;
pub mod repo;
pub mod runner;
pub mod snapshot;

pub use repo::{Repo, TreeEntry};
pub use runner::GitRunner;
pub use snapshot::TreeOnly;
