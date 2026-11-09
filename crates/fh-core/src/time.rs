//! Timestamps and the clock. `FIRSTHAND_FAKE_TIME` (RFC 3339) freezes the clock.

use crate::error::Result;
use crate::hash::invalid;
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Deserializer, Serialize};
use std::borrow::Cow;
use std::fmt;

/// RFC 3339 UTC timestamp with millisecond precision: `2026-11-09T14:02:11.123Z`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct Ts(String);

impl Ts {
    /// Parse any RFC 3339 timestamp and normalize it to UTC milliseconds.
    pub fn parse(s: &str) -> Result<Self> {
        let t: jiff::Timestamp = s
            .parse()
            .map_err(|e: jiff::Error| invalid("timestamp", s, &e.to_string()))?;
        Ok(Self::from_jiff(t))
    }
    /// Convert from a jiff timestamp, truncating to milliseconds.
    pub fn from_jiff(t: jiff::Timestamp) -> Self {
        let z = t.to_zoned(jiff::tz::TimeZone::UTC);
        Ts(format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
            z.year(),
            z.month(),
            z.day(),
            z.hour(),
            z.minute(),
            z.second(),
            z.millisecond()
        ))
    }
    /// Build from milliseconds since the Unix epoch.
    pub fn from_unix_ms(ms: i64) -> Self {
        Self::from_jiff(
            jiff::Timestamp::from_millisecond(ms).unwrap_or(jiff::Timestamp::UNIX_EPOCH),
        )
    }
    /// Milliseconds since the Unix epoch.
    pub fn unix_ms(&self) -> i64 {
        self.0
            .parse::<jiff::Timestamp>()
            .map(|t| t.as_millisecond())
            .unwrap_or(0)
    }
    /// The string form.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Ts {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for Ts {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        let ts = Ts::parse(&s).map_err(serde::de::Error::custom)?;
        if ts.0 != s {
            return Err(serde::de::Error::custom(format!(
                "timestamp {s:?} is not in canonical form {:?}",
                ts.0
            )));
        }
        Ok(ts)
    }
}

impl JsonSchema for Ts {
    fn schema_name() -> Cow<'static, str> {
        "Ts".into()
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "string",
            "pattern": "^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}\\.[0-9]{3}Z$"
        })
    }
}

/// Source of the current time.
pub trait Clock: Send + Sync {
    /// The current time.
    fn now(&self) -> Ts;
}

/// Wall clock, frozen when `FIRSTHAND_FAKE_TIME` is set.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Ts {
        if let Ok(v) = std::env::var("FIRSTHAND_FAKE_TIME")
            && let Ok(ts) = Ts::parse(&v)
        {
            return ts;
        }
        Ts::from_jiff(jiff::Timestamp::now())
    }
}

/// A clock fixed at one instant, for tests.
#[derive(Debug, Clone)]
pub struct FixedClock(pub Ts);

impl Clock for FixedClock {
    fn now(&self) -> Ts {
        self.0.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes() {
        assert_eq!(
            Ts::parse("2026-11-09T14:02:11Z").unwrap().as_str(),
            "2026-11-09T14:02:11.000Z"
        );
        assert_eq!(
            Ts::parse("2026-11-09T15:02:11.1234+01:00")
                .unwrap()
                .as_str(),
            "2026-11-09T14:02:11.123Z"
        );
        assert!(serde_json::from_str::<Ts>("\"2026-11-09T14:02:11Z\"").is_err());
        assert!(serde_json::from_str::<Ts>("\"2026-11-09T14:02:11.000Z\"").is_ok());
        let t = Ts::parse("2026-11-09T14:02:11.123Z").unwrap();
        assert_eq!(Ts::from_unix_ms(t.unix_ms()), t);
    }
}
