//! Path rules for packets and probes: UTF-8, `/` separators, NFC, relative,
//! no `..`, no Windows reserved names, no trailing dots or spaces.

use crate::error::{Error, Result};
use std::collections::HashMap;
use unicode_normalization::UnicodeNormalization;

const RESERVED: &[&str] = &["CON", "PRN", "AUX", "NUL"];

/// Whether a single path component is a reserved Windows device name
/// (`CON`, `PRN`, `AUX`, `NUL`, `COM1..9`, `LPT1..9`), with or without extension.
pub fn is_reserved_windows_name(component: &str) -> bool {
    let stem = component
        .split('.')
        .next()
        .unwrap_or(component)
        .trim_end_matches(' ');
    let up = stem.to_ascii_uppercase();
    if RESERVED.contains(&up.as_str()) {
        return true;
    }
    for p in ["COM", "LPT"] {
        if let Some(rest) = up.strip_prefix(p) {
            let mut c = rest.chars();
            if let (Some(d), None) = (c.next(), c.next()) {
                // COM1..COM9, plus superscript digits Windows also treats as reserved.
                if ('1'..='9').contains(&d) || matches!(d, '\u{b9}' | '\u{b2}' | '\u{b3}') {
                    return true;
                }
            }
        }
    }
    false
}

fn unsafe_path(path: &str, reason: &str) -> Error {
    Error::UnsafePath {
        path: path.chars().take(200).collect(),
        reason: reason.to_owned(),
    }
}

/// Normalize a repo-relative path for storage in packets. Converts `\` to `/`,
/// applies NFC, and rejects anything that is absolute, escapes the root,
/// contains empty components, control characters, reserved Windows names, or
/// components ending in a dot or space.
pub fn normalize_rel(path: &str) -> Result<String> {
    if path.is_empty() {
        return Err(unsafe_path(path, "empty path"));
    }
    let p: String = path.replace('\\', "/").nfc().collect();
    if p.starts_with('/') {
        return Err(unsafe_path(path, "absolute path"));
    }
    let b = p.as_bytes();
    if b.len() >= 2 && b[1] == b':' && b[0].is_ascii_alphabetic() {
        return Err(unsafe_path(path, "drive letter"));
    }
    if p.chars().any(|c| c.is_control()) {
        return Err(unsafe_path(path, "control character"));
    }
    let mut out = Vec::new();
    for comp in p.split('/') {
        match comp {
            "" => return Err(unsafe_path(path, "empty component")),
            "." => continue,
            ".." => return Err(unsafe_path(path, "parent component (..)")),
            c if c.ends_with('.') || c.ends_with(' ') => {
                return Err(unsafe_path(path, "component ends with a dot or space"));
            }
            c if is_reserved_windows_name(c) => {
                return Err(unsafe_path(path, "reserved Windows device name"));
            }
            c => out.push(c),
        }
    }
    if out.is_empty() {
        return Err(unsafe_path(path, "path resolves to the root"));
    }
    Ok(out.join("/"))
}

/// Find paths that collide on a case-insensitive file system. Returns pairs.
pub fn case_collisions<'a>(paths: impl IntoIterator<Item = &'a str>) -> Vec<(String, String)> {
    let mut seen: HashMap<String, &'a str> = HashMap::new();
    let mut out = Vec::new();
    for p in paths {
        let key: String = p.nfc().collect::<String>().to_lowercase();
        if let Some(prev) = seen.get(&key) {
            if *prev != p {
                out.push(((*prev).to_owned(), p.to_owned()));
            }
        } else {
            seen.insert(key, p);
        }
    }
    out
}

/// Replace the home directory with `~` and the username with `<user>` in text.
/// Handles both `\` and `/` separators.
pub fn scrub_identity(text: &str, home: Option<&str>, user: Option<&str>) -> String {
    let mut s = text.to_owned();
    if let Some(h) = home.filter(|h| h.len() > 3) {
        let fwd = h.replace('\\', "/");
        let back = h.replace('/', "\\");
        for v in [h.to_owned(), fwd, back] {
            s = s.replace(&v, "~");
        }
    }
    if let Some(u) = user.filter(|u| u.len() >= 3) {
        s = replace_word(&s, u, "<user>");
    }
    s
}

fn replace_word(s: &str, word: &str, with: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find(word) {
        let before_ok = rest[..i]
            .chars()
            .last()
            .is_none_or(|c| !c.is_alphanumeric());
        let after = &rest[i + word.len()..];
        let after_ok = after.chars().next().is_none_or(|c| !c.is_alphanumeric());
        out.push_str(&rest[..i]);
        out.push_str(if before_ok && after_ok { with } else { word });
        rest = after;
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn rules() {
        assert_eq!(normalize_rel("src\\a b\\c.rs").unwrap(), "src/a b/c.rs");
        assert_eq!(normalize_rel("./x/./y").unwrap(), "x/y");
        assert_eq!(normalize_rel("e\u{301}.txt").unwrap(), "\u{e9}.txt");
        for bad in [
            "",
            "/etc/passwd",
            "C:/x",
            "c:\\x",
            "a/../b",
            "..",
            "a//b",
            "con",
            "src/NUL.txt",
            "COM1",
            "lpt9.log",
            "a./b",
            "a /b",
            "x\ny",
            ".",
        ] {
            assert!(normalize_rel(bad).is_err(), "{bad:?} should be rejected");
        }
        assert!(normalize_rel("COM10").is_ok());
        assert!(normalize_rel("console.rs").is_ok());
    }

    #[test]
    fn collisions() {
        let c = case_collisions(["a/B.rs", "a/b.rs", "c.rs"]);
        assert_eq!(c, vec![("a/B.rs".to_owned(), "a/b.rs".to_owned())]);
    }

    #[test]
    fn scrub() {
        let s = scrub_identity(
            "C:\\Users\\alice\\x and C:/Users/alice/y by alice, not alicex",
            Some("C:\\Users\\alice"),
            Some("alice"),
        );
        assert_eq!(s, "~\\x and ~/y by <user>, not alicex");
    }

    proptest! {
        #[test]
        fn normalized_paths_are_safe(s in "[a-zA-Z0-9 ._/\\\\-]{1,40}") {
            if let Ok(n) = normalize_rel(&s) {
                prop_assert!(!n.starts_with('/'));
                prop_assert!(!n.contains('\\'));
                prop_assert!(n.split('/').all(|c| c != ".." && !c.is_empty()));
                prop_assert_eq!(normalize_rel(&n).unwrap(), n);
            }
        }
    }
}
