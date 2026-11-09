//! Canonical JSON per RFC 8785 (JCS), restricted to integers.
//!
//! Object keys are sorted by UTF-16 code units, there is no whitespace, strings
//! use the JCS escape set, and numbers must be integers within +/-(2^53 - 1).
//! Floats are rejected with [`Error::FloatInCanonical`].

use crate::error::{Error, Result};
use crate::hash::B3;
use serde::Serialize;
use serde_json::Value;

/// Largest integer that survives a round trip through an IEEE double.
pub const MAX_SAFE_INT: u64 = (1 << 53) - 1;

/// Serialize `value` to canonical bytes.
pub fn canonical_bytes<T: Serialize + ?Sized>(value: &T) -> Result<Vec<u8>> {
    let v = serde_json::to_value(value)?;
    canonical_value_bytes(&v)
}

/// Canonical bytes of an already-built JSON value.
pub fn canonical_value_bytes(v: &Value) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(256);
    write_value(v, &mut out, &mut String::new())?;
    Ok(out)
}

/// BLAKE3 of the canonical bytes.
pub fn hash_canonical<T: Serialize + ?Sized>(value: &T) -> Result<B3> {
    Ok(B3::of(&canonical_bytes(value)?))
}

/// Check that a value contains no floats or out-of-range integers.
pub fn check_integers(v: &Value) -> Result<()> {
    canonical_value_bytes(v).map(|_| ())
}

fn write_value(v: &Value, out: &mut Vec<u8>, path: &mut String) -> Result<()> {
    match v {
        Value::Null => out.extend_from_slice(b"null"),
        Value::Bool(b) => out.extend_from_slice(if *b { b"true" } else { b"false" }),
        Value::Number(n) => {
            if let Some(u) = n.as_u64() {
                if u > MAX_SAFE_INT {
                    return Err(Error::IntegerRange {
                        value: u.to_string(),
                        path: ptr(path),
                    });
                }
                out.extend_from_slice(u.to_string().as_bytes());
            } else if let Some(i) = n.as_i64() {
                if i.unsigned_abs() > MAX_SAFE_INT {
                    return Err(Error::IntegerRange {
                        value: i.to_string(),
                        path: ptr(path),
                    });
                }
                out.extend_from_slice(i.to_string().as_bytes());
            } else {
                return Err(Error::FloatInCanonical { path: ptr(path) });
            }
        }
        Value::String(s) => write_string(s, out),
        Value::Array(a) => {
            out.push(b'[');
            for (i, item) in a.iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                let len = path.len();
                path.push('/');
                path.push_str(&i.to_string());
                write_value(item, out, path)?;
                path.truncate(len);
            }
            out.push(b']');
        }
        Value::Object(m) => {
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
            out.push(b'{');
            for (i, k) in keys.iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                write_string(k, out);
                out.push(b':');
                let len = path.len();
                path.push('/');
                path.push_str(&k.replace('~', "~0").replace('/', "~1"));
                write_value(&m[k.as_str()], out, path)?;
                path.truncate(len);
            }
            out.push(b'}');
        }
    }
    Ok(())
}

fn ptr(p: &str) -> String {
    if p.is_empty() {
        "/".to_owned()
    } else {
        p.to_owned()
    }
}

fn write_string(s: &str, out: &mut Vec<u8>) {
    out.push(b'"');
    for c in s.chars() {
        match c {
            '"' => out.extend_from_slice(b"\\\""),
            '\\' => out.extend_from_slice(b"\\\\"),
            '\u{8}' => out.extend_from_slice(b"\\b"),
            '\u{c}' => out.extend_from_slice(b"\\f"),
            '\n' => out.extend_from_slice(b"\\n"),
            '\r' => out.extend_from_slice(b"\\r"),
            '\t' => out.extend_from_slice(b"\\t"),
            c if (c as u32) < 0x20 => {
                out.extend_from_slice(format!("\\u{:04x}", c as u32).as_bytes());
            }
            c => {
                let mut buf = [0u8; 4];
                out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            }
        }
    }
    out.push(b'"');
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use serde_json::json;

    fn c(v: Value) -> String {
        String::from_utf8(canonical_value_bytes(&v).unwrap()).unwrap()
    }

    #[test]
    fn sorts_by_utf16_and_escapes() {
        // U+1F600 encodes as D83D DE00, which sorts before U+FB33 (FB33) in UTF-16
        // even though its scalar value is larger.
        let v = json!({"\u{fb33}": 1, "\u{1f600}": 2, "a": 3, "\u{80}": 4});
        assert_eq!(
            c(v),
            "{\"a\":3,\"\u{80}\":4,\"\u{1f600}\":2,\"\u{fb33}\":1}"
        );
        assert_eq!(
            c(json!("a\"b\\c\u{1}\n\u{7f}")),
            "\"a\\\"b\\\\c\\u0001\\n\u{7f}\""
        );
        assert_eq!(c(json!({"b": [], "a": {}})), "{\"a\":{},\"b\":[]}");
    }

    #[test]
    fn integers_only() {
        assert_eq!(c(json!(MAX_SAFE_INT)), "9007199254740991");
        assert_eq!(c(json!(-(MAX_SAFE_INT as i64))), "-9007199254740991");
        assert!(matches!(
            canonical_value_bytes(&json!({"x": [1.5]})),
            Err(Error::FloatInCanonical { path }) if path == "/x/0"
        ));
        assert!(matches!(
            canonical_value_bytes(&json!(MAX_SAFE_INT + 1)),
            Err(Error::IntegerRange { .. })
        ));
    }

    fn arb_json() -> impl Strategy<Value = Value> {
        let leaf = prop_oneof![
            Just(Value::Null),
            any::<bool>().prop_map(Value::Bool),
            (-(MAX_SAFE_INT as i64)..=(MAX_SAFE_INT as i64)).prop_map(|i| json!(i)),
            ".*".prop_map(Value::String),
        ];
        leaf.prop_recursive(4, 64, 8, |inner| {
            prop_oneof![
                prop::collection::vec(inner.clone(), 0..6).prop_map(Value::Array),
                prop::collection::btree_map(".*", inner, 0..6)
                    .prop_map(|m| Value::Object(m.into_iter().collect())),
            ]
        })
    }

    proptest! {
        #[test]
        fn roundtrip_is_stable(v in arb_json()) {
            let a = canonical_value_bytes(&v).unwrap();
            let back: Value = serde_json::from_slice(&a).unwrap();
            prop_assert_eq!(&back, &v);
            prop_assert_eq!(canonical_value_bytes(&back).unwrap(), a);
        }
    }
}
