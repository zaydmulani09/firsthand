//! Typed identifiers: a prefix plus a UUIDv7 in lowercase hex (`pkt_0193...`).
//!
//! With `FIRSTHAND_TEST_SEED` set, ids are derived from the seed, the frozen
//! clock, a per-process counter and an optional caller salt, so runs repeat
//! byte for byte. Code that creates ids of the same kind in separate processes
//! should pass a salt (for example the ledger position) to avoid collisions.

use crate::error::Result;
use crate::hash::invalid;
use crate::time::Ts;
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::borrow::Cow;
use std::fmt;
use std::marker::PhantomData;
use std::sync::atomic::{AtomicU64, Ordering};

/// A kind of identifier, defining its prefix.
pub trait IdKind {
    /// Prefix including the trailing `_`, e.g. `"pkt_"`.
    const PREFIX: &'static str;
}

macro_rules! kinds {
    ($($(#[$m:meta])* $name:ident => $p:literal),* $(,)?) => {$(
        $(#[$m])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum $name {}
        impl IdKind for $name { const PREFIX: &'static str = $p; }
    )*};
}

kinds! {
    /// Packet id marker.
    Pkt => "pkt_",
    /// Claim id marker.
    Clm => "clm_",
    /// Task id marker.
    Tsk => "tsk_",
    /// Decision id marker.
    Dec => "dec_",
    /// Attempt id marker.
    Att => "att_",
    /// Snapshot id marker.
    Snp => "snp_",
}

impl IdKind for crate::model::Leg {
    const PREFIX: &'static str = "leg_";
}

/// A typed identifier.
pub struct Id<T>(String, PhantomData<fn() -> T>);

impl<T: IdKind> Id<T> {
    /// Parse and validate an id string with the right prefix.
    pub fn parse(s: &str) -> Result<Self> {
        let rest = s
            .strip_prefix(T::PREFIX)
            .ok_or_else(|| invalid("id", s, &format!("expected prefix {}", T::PREFIX)))?;
        if rest.len() != 32
            || !rest
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(invalid(
                "id",
                s,
                "expected 32 lowercase hex digits after the prefix",
            ));
        }
        Ok(Id(s.to_owned(), PhantomData))
    }
    /// Wrap a UUID.
    pub fn from_uuid(u: uuid::Uuid) -> Self {
        Id(format!("{}{}", T::PREFIX, u.simple()), PhantomData)
    }
    /// Full string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
    /// Short display form: prefix plus the last 8 characters.
    pub fn short(&self) -> String {
        format!("{}{}", T::PREFIX, &self.0[self.0.len() - 8..])
    }
}

impl<T> Clone for Id<T> {
    fn clone(&self) -> Self {
        Id(self.0.clone(), PhantomData)
    }
}
impl<T> PartialEq for Id<T> {
    fn eq(&self, o: &Self) -> bool {
        self.0 == o.0
    }
}
impl<T> Eq for Id<T> {}
impl<T> PartialOrd for Id<T> {
    fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(o))
    }
}
impl<T> Ord for Id<T> {
    fn cmp(&self, o: &Self) -> std::cmp::Ordering {
        self.0.cmp(&o.0)
    }
}
impl<T> std::hash::Hash for Id<T> {
    fn hash<H: std::hash::Hasher>(&self, h: &mut H) {
        self.0.hash(h)
    }
}
impl<T> fmt::Debug for Id<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl<T> fmt::Display for Id<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl<T> Serialize for Id<T> {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&self.0)
    }
}
impl<'de, T: IdKind> Deserialize<'de> for Id<T> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Id::parse(&s).map_err(serde::de::Error::custom)
    }
}
impl<T: IdKind> JsonSchema for Id<T> {
    fn schema_name() -> Cow<'static, str> {
        format!("Id_{}", T::PREFIX.trim_end_matches('_')).into()
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({"type": "string", "pattern": format!("^{}[0-9a-f]{{32}}$", T::PREFIX)})
    }
}

/// Generates ids. Random in normal use, deterministic under `FIRSTHAND_TEST_SEED`.
#[derive(Debug)]
pub struct IdGen {
    seed: Option<String>,
    counter: AtomicU64,
}

impl Default for IdGen {
    fn default() -> Self {
        Self::from_env()
    }
}

impl IdGen {
    /// Reads `FIRSTHAND_TEST_SEED`.
    pub fn from_env() -> Self {
        IdGen {
            seed: std::env::var("FIRSTHAND_TEST_SEED").ok(),
            counter: AtomicU64::new(0),
        }
    }
    /// A deterministic generator.
    pub fn seeded(seed: &str) -> Self {
        IdGen {
            seed: Some(seed.to_owned()),
            counter: AtomicU64::new(0),
        }
    }
    /// Whether ids are deterministic.
    pub fn is_seeded(&self) -> bool {
        self.seed.is_some()
    }
    /// New id stamped with `now`.
    pub fn new_id<T: IdKind>(&self, now: &Ts) -> Id<T> {
        self.new_id_salted(now, b"")
    }
    /// New id with a salt mixed into deterministic mode.
    pub fn new_id_salted<T: IdKind>(&self, now: &Ts, salt: &[u8]) -> Id<T> {
        let ms = u64::try_from(now.unix_ms()).unwrap_or(0);
        let mut rnd = [0u8; 10];
        match &self.seed {
            Some(seed) => {
                let n = self.counter.fetch_add(1, Ordering::Relaxed);
                let h = blake3::Hasher::new()
                    .update(seed.as_bytes())
                    .update(T::PREFIX.as_bytes())
                    .update(&n.to_le_bytes())
                    .update(salt)
                    .finalize();
                rnd.copy_from_slice(&h.as_bytes()[..10]);
            }
            None => {
                if getrandom::fill(&mut rnd).is_err() {
                    // Fall back to hashing time and a counter; still unique per process.
                    let n = self.counter.fetch_add(1, Ordering::Relaxed);
                    let h = blake3::hash(&[ms.to_le_bytes(), n.to_le_bytes()].concat());
                    rnd.copy_from_slice(&h.as_bytes()[..10]);
                }
            }
        }
        Id::from_uuid(uuid::Builder::from_unix_timestamp_millis(ms, &rnd).into_uuid())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeded_is_deterministic_and_typed() {
        let t = Ts::parse("2026-11-09T14:02:11.123Z").unwrap();
        let a: Id<Pkt> = IdGen::seeded("s").new_id(&t);
        let b: Id<Pkt> = IdGen::seeded("s").new_id(&t);
        assert_eq!(a, b);
        assert!(a.as_str().starts_with("pkt_"));
        assert_eq!(a.short().len(), 12);
        assert!(Id::<Clm>::parse(a.as_str()).is_err());
        let g = IdGen::seeded("s");
        let c: Id<Pkt> = g.new_id(&t);
        let d: Id<Pkt> = g.new_id(&t);
        assert_ne!(c, d);
    }

    #[test]
    fn random_ids_differ() {
        let t = Ts::parse("2026-11-09T14:02:11.123Z").unwrap();
        let g = IdGen {
            seed: None,
            counter: AtomicU64::new(0),
        };
        let a: Id<Clm> = g.new_id(&t);
        let b: Id<Clm> = g.new_id(&t);
        assert_ne!(a, b);
        assert_eq!(Id::<Clm>::parse(a.as_str()).unwrap(), a);
    }
}
