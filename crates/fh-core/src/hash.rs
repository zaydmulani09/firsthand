//! Content hashes: BLAKE3 (`b3:<hex>`) and git object ids.

use crate::error::{Error, Result};
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Deserializer, Serialize};
use std::borrow::Cow;
use std::fmt;

/// A BLAKE3-256 digest rendered as `b3:` followed by 64 lowercase hex digits.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct B3(String);

impl B3 {
    /// Hash raw bytes.
    pub fn of(bytes: &[u8]) -> Self {
        B3(format!("b3:{}", blake3::hash(bytes).to_hex()))
    }
    /// Hash the concatenation of several byte slices.
    pub fn of_parts(parts: &[&[u8]]) -> Self {
        let mut h = blake3::Hasher::new();
        for p in parts {
            h.update(p);
        }
        B3(format!("b3:{}", h.finalize().to_hex()))
    }
    /// Parse and validate a `b3:` string.
    pub fn parse(s: &str) -> Result<Self> {
        let hex = s
            .strip_prefix("b3:")
            .ok_or_else(|| invalid("b3 hash", s, "missing b3: prefix"))?;
        if hex.len() != 64 || !is_lower_hex(hex) {
            return Err(invalid("b3 hash", s, "expected 64 lowercase hex digits"));
        }
        Ok(B3(s.to_owned()))
    }
    /// The full `b3:<hex>` string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
    /// The hex part without the prefix.
    pub fn hex(&self) -> &str {
        &self.0[3..]
    }
}

fn is_lower_hex(s: &str) -> bool {
    s.bytes()
        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

impl fmt::Display for B3 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for B3 {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        B3::parse(&s).map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for B3 {
    fn schema_name() -> Cow<'static, str> {
        "B3".into()
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({"type": "string", "pattern": "^b3:[0-9a-f]{64}$"})
    }
}

/// A git object id: 40 (sha1) or 64 (sha256) lowercase hex digits.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct GitOid(String);

impl GitOid {
    /// Parse and validate an object id.
    pub fn parse(s: &str) -> Result<Self> {
        if !(s.len() == 40 || s.len() == 64) || !is_lower_hex(s) {
            return Err(invalid(
                "git oid",
                s,
                "expected 40 or 64 lowercase hex digits",
            ));
        }
        Ok(GitOid(s.to_owned()))
    }
    /// The hex string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
    /// First 7 characters, for display.
    pub fn short(&self) -> &str {
        &self.0[..7]
    }
}

impl fmt::Display for GitOid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for GitOid {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        GitOid::parse(&s).map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for GitOid {
    fn schema_name() -> Cow<'static, str> {
        "GitOid".into()
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({"type": "string", "pattern": "^([0-9a-f]{40}|[0-9a-f]{64})$"})
    }
}

pub(crate) fn invalid(what: &'static str, value: &str, reason: &str) -> Error {
    let v: String = value.chars().take(80).collect();
    Error::Invalid {
        what,
        value: v,
        reason: reason.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn b3_roundtrip_and_rejects() {
        let h = B3::of(b"abc");
        assert_eq!(B3::parse(h.as_str()).unwrap(), h);
        assert!(B3::parse("b3:XYZ").is_err());
        assert!(B3::parse(&h.as_str().to_uppercase()).is_err());
        assert_eq!(B3::of_parts(&[b"a", b"bc"]), h);
    }

    #[test]
    fn oid_lengths() {
        assert!(GitOid::parse(&"a".repeat(40)).is_ok());
        assert!(GitOid::parse(&"a".repeat(64)).is_ok());
        assert!(GitOid::parse(&"a".repeat(41)).is_err());
    }
}
