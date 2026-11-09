//! Small shared helpers: path comparison, capped reads, executable lookup.

use crate::{Detection, HostEnv};
use fh_core::{B3, Error, Result, Ts};
use serde_json::Value;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Path as a comparable string: no `\\?\` prefix, `/` separators, no trailing `/`.
pub(crate) fn norm_path(p: &str) -> String {
    let p = p
        .strip_prefix(r"\\?\UNC\")
        .map(|r| format!(r"\\{r}"))
        .unwrap_or_else(|| p.strip_prefix(r"\\?\").unwrap_or(p).to_owned());
    let mut s = p.replace('\\', "/");
    while s.len() > 1 && s.ends_with('/') && !s.ends_with(":/") {
        s.pop();
    }
    s
}

/// Plain display string of a path (no verbatim prefix).
pub(crate) fn plain(p: &Path) -> String {
    let s = p.to_string_lossy();
    s.strip_prefix(r"\\?\").unwrap_or(&s).to_owned()
}

/// Whether `inner` is `outer` or below it. Case-insensitive for drive-letter
/// (Windows) paths, where the file system is case-insensitive by default.
pub(crate) fn path_inside(inner: &str, outer: &str) -> bool {
    let (mut i, mut o) = (norm_path(inner), norm_path(outer));
    let windowsy = |s: &str| s.as_bytes().get(1) == Some(&b':') || s.starts_with("//");
    if windowsy(&i) || windowsy(&o) {
        i = i.to_lowercase();
        o = o.to_lowercase();
    }
    i == o || i.starts_with(&format!("{}/", o.trim_end_matches('/')))
}

/// Last modification time of a file.
pub(crate) fn mtime(p: &Path) -> Option<Ts> {
    let t = std::fs::metadata(p).ok()?.modified().ok()?;
    let ms = t.duration_since(std::time::UNIX_EPOCH).ok()?.as_millis();
    Some(Ts::from_unix_ms(i64::try_from(ms).ok()?))
}

/// Bytes of a session source, capped. `hash` covers the whole file.
pub(crate) struct Source {
    pub bytes: Vec<u8>,
    pub hash: B3,
}

/// Read a file, at most `max_bytes` from its end (starting at a line boundary).
pub(crate) fn read_source(path: &Path, max_bytes: u64) -> Result<Source> {
    let io = |e| Error::io(path, e);
    let mut f = std::fs::File::open(path).map_err(io)?;
    let mut h = blake3::Hasher::new();
    h.update_reader(&mut f).map_err(io)?;
    let len = f.metadata().map_err(io)?.len();
    let cap = if max_bytes == 0 { u64::MAX } else { max_bytes };
    let start = len.saturating_sub(cap);
    f.seek(SeekFrom::Start(start)).map_err(io)?;
    let mut bytes = Vec::new();
    f.take(cap).read_to_end(&mut bytes).map_err(io)?;
    if start > 0 {
        let cut = bytes
            .iter()
            .position(|b| *b == b'\n')
            .map_or(bytes.len(), |i| i + 1);
        bytes.drain(..cut);
    }
    Ok(Source {
        bytes,
        hash: B3::parse(&format!("b3:{}", h.finalize().to_hex()))?,
    })
}

/// The first `max_lines` lines of a file, each cut at `max_line` bytes.
pub(crate) fn head_lines(path: &Path, max_lines: usize, max_line: u64) -> Vec<String> {
    let Ok(f) = std::fs::File::open(path) else {
        return Vec::new();
    };
    let mut r = BufReader::new(f);
    let mut out = Vec::new();
    for _ in 0..max_lines {
        let mut buf = Vec::new();
        match r.by_ref().take(max_line).read_until(b'\n', &mut buf) {
            Ok(0) | Err(_) => break,
            Ok(_) => out.push(String::from_utf8_lossy(&buf).trim_end().to_owned()),
        }
        if !buf.ends_with(b"\n") {
            // Line longer than the cap: skip its remainder.
            let mut rest = Vec::new();
            if r.read_until(b'\n', &mut rest).is_err() {
                break;
            }
        }
    }
    out
}

/// Non-empty lines of a byte buffer, lossily decoded, `\r` stripped.
pub(crate) fn lines(bytes: &[u8]) -> impl Iterator<Item = String> + '_ {
    bytes
        .split(|b| *b == b'\n')
        .map(|l| String::from_utf8_lossy(l.strip_suffix(b"\r").unwrap_or(l)).into_owned())
        .filter(|l| !l.trim().is_empty())
}

/// String field of a JSON object.
pub(crate) fn s<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

/// Parse a timestamp field (RFC 3339 string or epoch milliseconds).
pub(crate) fn ts(v: Option<&Value>) -> Option<Ts> {
    match v? {
        Value::String(t) => Ts::parse(t).ok(),
        Value::Number(n) => n.as_i64().map(Ts::from_unix_ms),
        _ => None,
    }
}

/// Cut text at `max` bytes on a char boundary, saying how much was dropped.
pub(crate) fn cap(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_owned();
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!(
        "{}\n[truncated by firsthand: {} more bytes]",
        &text[..end],
        text.len() - end
    )
}

/// One-line summary of a tool input: the most telling string field, else compact JSON.
pub(crate) fn summarize_input(input: &Value) -> String {
    const KEYS: [&str; 11] = [
        "command",
        "cmd",
        "file_path",
        "filePath",
        "notebook_path",
        "path",
        "pattern",
        "url",
        "query",
        "description",
        "prompt",
    ];
    let text = KEYS
        .iter()
        .find_map(|k| match input.get(*k) {
            Some(Value::String(s)) => Some(s.clone()),
            Some(Value::Array(a)) if a.iter().all(Value::is_string) => Some(join_argv(a)),
            _ => None,
        })
        .unwrap_or_else(|| match input {
            Value::String(s) => s.clone(),
            Value::Null => String::new(),
            other => other.to_string(),
        });
    cap(&text, 300)
}

/// Join an argv array for display. Arguments with spaces are double-quoted.
pub(crate) fn join_argv(a: &[Value]) -> String {
    a.iter()
        .filter_map(Value::as_str)
        .map(|s| {
            if s.is_empty() || s.contains([' ', '\t', '"']) {
                format!("\"{}\"", s.replace('"', "\\\""))
            } else {
                s.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Split `firsthand_cmd` into argv: a leading double-quoted path, then whitespace words.
pub(crate) fn split_cmd(cmd: &str) -> Vec<String> {
    let cmd = cmd.trim();
    let mut out = Vec::new();
    let rest = if let Some(r) = cmd.strip_prefix('"') {
        let end = r.find('"').unwrap_or(r.len());
        out.push(r[..end].to_owned());
        r.get(end + 1..).unwrap_or("")
    } else {
        cmd
    };
    out.extend(rest.split_whitespace().map(str::to_owned));
    out
}

/// Text from a value that is a string, an array of text blocks, or an object with
/// a text-like field.
pub(crate) fn text_of(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Array(a) => a
            .iter()
            .map(text_of)
            .filter(|t| !t.is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
        Value::Object(o) => {
            if matches!(
                o.get("type").and_then(Value::as_str),
                Some("thinking" | "redacted_thinking" | "reasoning" | "image")
            ) {
                return String::new();
            }
            ["text", "content", "output", "llmContent"]
                .iter()
                .find_map(|k| o.get(*k).map(text_of))
                .unwrap_or_default()
        }
        _ => String::new(),
    }
}

/// Find `name` on `PATH` (with `PATHEXT` on Windows).
pub(crate) fn which(host: &HostEnv, name: &str) -> Option<PathBuf> {
    let path = host.path_var.as_ref()?;
    let exts: Vec<String> = if cfg!(windows) {
        host.var("PATHEXT")
            .unwrap_or(".COM;.EXE;.BAT;.CMD")
            .split(';')
            .filter(|e| !e.is_empty())
            .map(str::to_ascii_lowercase)
            .collect()
    } else {
        vec![String::new()]
    };
    std::env::split_paths(path).find_map(|dir| {
        exts.iter()
            .map(|e| dir.join(format!("{name}{e}")))
            .find(|p| p.is_file())
    })
}

/// Run `<exe> --version` with a 5 second limit; first non-empty output line.
pub(crate) fn run_version(exe: &Path) -> std::result::Result<String, String> {
    let mut child = Command::new(exe)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("could not run {} --version: {e}", exe.display()))?;
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if start.elapsed() > Duration::from_secs(5) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "{} --version did not exit within 5 s",
                    exe.display()
                ));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(e) => return Err(format!("waiting for {} --version: {e}", exe.display())),
        }
    }
    let out = child
        .wait_with_output()
        .map_err(|e| format!("reading {} --version: {e}", exe.display()))?;
    let text =
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    text.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| format!("{} --version printed nothing", exe.display()))
}

/// Shared `detect`: find the binary, ask its version, list existing config paths.
pub(crate) fn detect(host: &HostEnv, bin: &str, configs: Vec<PathBuf>) -> Detection {
    let mut d = Detection {
        config_paths: configs.into_iter().filter(|p| p.exists()).collect(),
        ..Detection::default()
    };
    match which(host, bin) {
        None => d.notes.push(format!("{bin} not found on PATH")),
        Some(exe) => match run_version(&exe) {
            Ok(v) => {
                d.installed = true;
                d.version = Some(v);
            }
            Err(e) => d.notes.push(e),
        },
    }
    d
}

/// Read a file if it exists.
pub(crate) fn read_opt(path: &Path) -> Result<Option<Vec<u8>>> {
    match std::fs::read(path) {
        Ok(b) => Ok(Some(b)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(Error::io(path, e)),
    }
}

/// Home directory or a specific error.
pub(crate) fn home(host: &HostEnv) -> Result<&Path> {
    host.home.as_deref().ok_or_else(|| {
        Error::Environment("no home directory found; set HOME (or USERPROFILE on Windows)".into())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths() {
        assert!(path_inside(r"C:\Users\dev\repo\sub", r"c:\users\dev\repo"));
        assert!(path_inside(r"\\?\C:\Users\dev\repo", r"C:/Users/dev/repo/"));
        assert!(!path_inside(r"C:\Users\dev\repo2", r"C:\Users\dev\repo"));
        assert!(path_inside("/home/a/r/x", "/home/a/r"));
        assert!(!path_inside("/home/a/R", "/home/a/r"));
        assert_eq!(norm_path(r"\\?\UNC\srv\share\x"), "//srv/share/x");
    }

    #[test]
    fn caps_on_char_boundary() {
        let t = cap("héllo", 2);
        assert!(t.starts_with('h') && t.contains("5 more bytes"));
        assert_eq!(cap("abc", 3), "abc");
    }

    #[test]
    fn split() {
        assert_eq!(
            split_cmd(r#""C:\Program Files\fh\firsthand.exe" --x"#),
            vec![r"C:\Program Files\fh\firsthand.exe", "--x"]
        );
        assert_eq!(split_cmd("firsthand"), vec!["firsthand"]);
    }
}
