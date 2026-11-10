//! Applying plans: unified diffs, timestamped backups, atomic writes, and the
//! shared builders adapters use to compute config edits.

use crate::jsonedit::{self, Node, Seg};
use crate::util::read_opt;
use crate::{
    ClaudeCode, Codex, FileEdit, GeminiCli, HookPlan, HookScope, HostEnv, OpenCode, RepoCtx,
};
use fh_core::{AgentId, Clock, Error, Result, SystemClock};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// Unified diff of two texts with 3 lines of context. `before` empty renders as a
/// new file (`--- /dev/null`), `after` empty as a deletion.
pub fn unified_diff(path: &str, before: &[u8], after: &[u8]) -> String {
    let a_text = String::from_utf8_lossy(before);
    let b_text = String::from_utf8_lossy(after);
    let a: Vec<&str> = a_text.split_inclusive('\n').collect();
    let b: Vec<&str> = b_text.split_inclusive('\n').collect();
    if a == b {
        return String::new();
    }
    let from = if before.is_empty() {
        "/dev/null".to_owned()
    } else {
        format!("a/{path}")
    };
    let to = if after.is_empty() {
        "/dev/null".to_owned()
    } else {
        format!("b/{path}")
    };
    let mut out = format!("--- {from}\n+++ {to}\n");
    let ops = diff_ops(&a, &b);
    // Group ops into hunks with 3 lines of context.
    let changed: Vec<usize> = ops
        .iter()
        .enumerate()
        .filter(|(_, o)| !matches!(o, Op::Same(..)))
        .map(|(i, _)| i)
        .collect();
    let mut i = 0;
    while i < changed.len() {
        let start = changed[i].saturating_sub(3);
        let mut end = changed[i];
        while i < changed.len() && changed[i] <= end + 6 {
            end = changed[i];
            i += 1;
        }
        let end = (end + 4).min(ops.len());
        let (mut a0, mut b0) = (0, 0);
        for o in &ops[..start] {
            match o {
                Op::Same(..) => {
                    a0 += 1;
                    b0 += 1;
                }
                Op::Del(_) => a0 += 1,
                Op::Add(_) => b0 += 1,
            }
        }
        let (mut an, mut bn) = (0, 0);
        let mut body = String::new();
        for o in &ops[start..end] {
            let (sign, line) = match o {
                Op::Same(l) => {
                    an += 1;
                    bn += 1;
                    (' ', *l)
                }
                Op::Del(l) => {
                    an += 1;
                    ('-', *l)
                }
                Op::Add(l) => {
                    bn += 1;
                    ('+', *l)
                }
            };
            body.push(sign);
            body.push_str(line.trim_end_matches(['\n', '\r']));
            body.push('\n');
            if !line.ends_with('\n') {
                body.push_str("\\ No newline at end of file\n");
            }
        }
        let start_of = |n0: usize, n: usize| if n == 0 { n0 } else { n0 + 1 };
        out.push_str(&format!(
            "@@ -{},{an} +{},{bn} @@\n{body}",
            start_of(a0, an),
            start_of(b0, bn)
        ));
    }
    out
}

enum Op<'a> {
    Same(&'a str),
    Del(&'a str),
    Add(&'a str),
}

/// Line diff by longest common subsequence. Config files are small; very large
/// inputs fall back to delete-all/add-all.
fn diff_ops<'a>(a: &[&'a str], b: &[&'a str]) -> Vec<Op<'a>> {
    // ponytail: O(n*m) table, capped at 4M cells; Myers if huge configs ever matter.
    if a.len().saturating_mul(b.len()) > 4_000_000 {
        return a
            .iter()
            .map(|l| Op::Del(l))
            .chain(b.iter().map(|l| Op::Add(l)))
            .collect();
    }
    let (n, m) = (a.len(), b.len());
    let mut t = vec![0u32; (n + 1) * (m + 1)];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            t[i * (m + 1) + j] = if a[i] == b[j] {
                t[(i + 1) * (m + 1) + j + 1] + 1
            } else {
                t[(i + 1) * (m + 1) + j].max(t[i * (m + 1) + j + 1])
            };
        }
    }
    let (mut i, mut j, mut ops) = (0, 0, Vec::new());
    while i < n || j < m {
        if i < n && j < m && a[i] == b[j] {
            ops.push(Op::Same(a[i]));
            i += 1;
            j += 1;
        } else if i < n && (j == m || t[(i + 1) * (m + 1) + j] >= t[i * (m + 1) + j + 1]) {
            ops.push(Op::Del(a[i]));
            i += 1;
        } else {
            ops.push(Op::Add(b[j]));
            j += 1;
        }
    }
    ops
}

/// Apply a plan: for every edit, check the file still has the content the plan was
/// computed from, back it up to `<file>.firsthand-<timestamp>.bak`, then write the new
/// content atomically (temp file in the same directory, then rename). Returns the
/// backups created.
pub fn apply_plan(plan: &HookPlan) -> Result<Vec<PathBuf>> {
    for e in &plan.edits {
        if read_opt(&e.path)? != e.before {
            return Err(Error::Other(format!(
                "config changed since the plan was made: {}; run the command again to compute a fresh plan",
                e.path.display()
            )));
        }
    }
    let stamp: String = SystemClock
        .now()
        .as_str()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();
    let mut backups = Vec::new();
    for e in &plan.edits {
        if e.before.as_deref() == Some(&e.after[..]) {
            continue;
        }
        if let Some(before) = &e.before {
            backups.push(write_backup(&e.path, before, &stamp)?);
        }
        if e.is_removal() {
            retry(|| std::fs::remove_file(&e.path)).map_err(|err| Error::io(&e.path, err))?;
        } else {
            atomic_write(&e.path, &e.after)?;
        }
    }
    Ok(backups)
}

fn write_backup(path: &Path, bytes: &[u8], stamp: &str) -> Result<PathBuf> {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    for n in 0..100 {
        let suffix = if n == 0 {
            String::new()
        } else {
            format!("-{n}")
        };
        let bak = path.with_file_name(format!("{name}.firsthand-{stamp}{suffix}.bak"));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&bak)
        {
            Ok(mut f) => {
                use std::io::Write;
                f.write_all(bytes)
                    .and_then(|()| f.sync_all())
                    .map_err(|e| Error::io(&bak, e))?;
                return Ok(bak);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(Error::io(&bak, e)),
        }
    }
    Err(Error::Other(format!(
        "could not pick a backup name for {}; remove old .bak files next to it",
        path.display()
    )))
}

/// Write via a temp file in the same directory and rename over the target.
pub(crate) fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir).map_err(|e| Error::io(dir, e))?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = dir.join(format!(".{name}.firsthand-tmp-{}", std::process::id()));
    let mut f = std::fs::File::create(&tmp).map_err(|e| Error::io(&tmp, e))?;
    f.write_all(bytes)
        .and_then(|()| f.sync_all())
        .map_err(|e| Error::io(&tmp, e))?;
    drop(f);
    retry(|| std::fs::rename(&tmp, path)).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        Error::io(path, e)
    })
}

/// Retry briefly on `PermissionDenied` (antivirus and indexers hold files on Windows).
fn retry(mut f: impl FnMut() -> std::io::Result<()>) -> std::io::Result<()> {
    let mut wait = 10;
    loop {
        match f() {
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied && wait <= 640 => {
                std::thread::sleep(std::time::Duration::from_millis(wait));
                wait *= 2;
            }
            r => return r,
        }
    }
}

/// Plans that remove every Firsthand entry from every agent's config files in the
/// given scope, using the current process environment for home directories.
pub fn uninstall_plan(repo: &RepoCtx, scope: HookScope) -> Result<Vec<HookPlan>> {
    uninstall_plan_with(repo, scope, &HostEnv::from_process())
}

/// [`uninstall_plan`] with an explicit host.
pub fn uninstall_plan_with(
    repo: &RepoCtx,
    scope: HookScope,
    host: &HostEnv,
) -> Result<Vec<HookPlan>> {
    let plans = vec![
        ClaudeCode::new(host.clone()).uninstall(repo, scope)?,
        Codex::new(host.clone()).uninstall(repo, scope)?,
        GeminiCli::new(host.clone()).uninstall(repo, scope)?,
        OpenCode::new(host.clone()).uninstall(repo, scope)?,
    ];
    Ok(plans.into_iter().filter(|p| !p.edits.is_empty()).collect())
}

// ------------------------------------------------------------ edit builders

/// Compute one JSON config edit. Missing files start as `{}`. A result equal to the
/// current content yields no edit; a result that is an empty object for an existing
/// file yields a removal.
pub(crate) fn json_edit(
    path: PathBuf,
    scope: HookScope,
    description: String,
    f: impl FnOnce(&str) -> std::result::Result<String, String>,
) -> Result<Option<FileEdit>> {
    let before = read_opt(&path)?;
    let text = match &before {
        Some(b) => String::from_utf8(b.clone()).map_err(|_| Error::Config {
            path: path.clone(),
            reason: "not valid UTF-8; fix or remove the file".into(),
        })?,
        None => String::new(),
    };
    let base = if text.trim().is_empty() {
        "{}\n"
    } else {
        &text
    };
    let after = f(base).map_err(|reason| Error::Config {
        path: path.clone(),
        reason: format!("{reason}; fix the file by hand and run again"),
    })?;
    if after == text || (before.is_none() && jsonedit::is_empty_object(&after)) {
        return Ok(None);
    }
    let after = if jsonedit::is_empty_object(&after) && before.is_some() {
        Vec::new()
    } else {
        after.into_bytes()
    };
    Ok(Some(FileEdit {
        path,
        before,
        after,
        scope,
        description,
    }))
}

/// A hook entry: event name, optional matcher, command.
pub(crate) struct HookEntry {
    pub event: &'static str,
    pub matcher: Option<&'static str>,
    pub command: String,
}

/// Commands of the shape `<firsthand_cmd> hook <agent> <Event>` for each event.
pub(crate) fn hook_entries(
    repo: &RepoCtx,
    agent: AgentId,
    events: &[(&'static str, Option<&'static str>)],
) -> Vec<HookEntry> {
    events
        .iter()
        .map(|(event, matcher)| HookEntry {
            event,
            matcher: *matcher,
            command: format!("{} hook {} {event}", repo.firsthand_cmd, agent.as_str()),
        })
        .collect()
}

/// Node predicate: a hook handler object (under top-level `hooks`) whose command
/// carries the `firsthand hook` marker.
/// A matcher group whose handlers are all ours counts as ours too.
pub(crate) fn is_our_hook(path: &[Seg], n: &Node) -> bool {
    let ours = |n: &Node| {
        n.get("command")
            .and_then(Node::str)
            .is_some_and(crate::is_firsthand_hook_command)
    };
    let group = n
        .get("hooks")
        .map(Node::items)
        .is_some_and(|h| !h.is_empty() && h.iter().all(ours));
    matches!(path.first(), Some(Seg::Key(k)) if k == "hooks") && (ours(n) || group)
}

/// Firsthand commands currently configured in a Claude-style `hooks` object.
fn our_commands(text: &str) -> Vec<(String, String)> {
    let Ok(root) = jsonedit::parse(text) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    if let Some(jsonedit::Kind::Obj(events)) = root.get("hooks").map(|h| &h.kind) {
        for ev in events {
            for group in ev.value.items() {
                for h in group.get("hooks").map(Node::items).unwrap_or(&[]) {
                    if let Some(c) = h.get("command").and_then(Node::str)
                        && crate::is_firsthand_hook_command(c)
                    {
                        out.push((ev.key.clone(), c.to_owned()));
                    }
                }
            }
        }
    }
    out
}

/// Install Claude-style hook entries (`hooks.<Event>[].{matcher, hooks[]}`).
/// Already-correct entries change nothing; stale Firsthand entries are replaced.
pub(crate) fn install_hooks(
    text: &str,
    entries: &[HookEntry],
    extra: &Value,
) -> std::result::Result<String, String> {
    let mut have = our_commands(text);
    let mut want: Vec<(String, String)> = entries
        .iter()
        .map(|e| (e.event.to_owned(), e.command.clone()))
        .collect();
    have.sort();
    want.sort();
    if have == want {
        return Ok(text.to_owned());
    }
    let mut out = if have.is_empty() {
        text.to_owned()
    } else {
        jsonedit::remove_where(text, &is_our_hook)?
    };
    for e in entries {
        let mut handler = json!({"type": "command", "command": e.command});
        if let (Some(h), Some(x)) = (handler.as_object_mut(), extra.as_object()) {
            h.extend(x.clone());
        }
        let mut group = serde_json::Map::new();
        if let Some(m) = e.matcher {
            group.insert("matcher".into(), json!(m));
        }
        group.insert("hooks".into(), json!([handler]));
        out = jsonedit::append_item(&out, &["hooks", e.event], &Value::Object(group))?;
    }
    Ok(out)
}

/// Node predicate for an MCP server entry named `firsthand` under `container`
/// whose command line ends in `mcp serve`.
pub(crate) fn is_our_mcp(container: &'static str) -> impl Fn(&[Seg], &Node) -> bool {
    move |path, n| {
        let at = matches!(path, [Seg::Key(c), Seg::Key(k)] if c == container && k == "firsthand");
        if !at {
            return false;
        }
        let mut words: Vec<String> = Vec::new();
        for key in ["command", "args"] {
            match n.get(key).map(|v| &v.kind) {
                Some(jsonedit::Kind::Str(s)) => words.push(s.clone()),
                Some(jsonedit::Kind::Arr(a)) => {
                    words.extend(a.iter().filter_map(Node::str).map(str::to_owned))
                }
                _ => {}
            }
        }
        let line = words.join(" ").to_ascii_lowercase();
        line.contains("firsthand") && line.trim_end().ends_with("mcp serve")
    }
}

/// argv of `<firsthand_cmd> mcp serve`.
pub(crate) fn mcp_argv(repo: &RepoCtx) -> Vec<String> {
    let mut v = crate::util::split_cmd(&repo.firsthand_cmd);
    if v.is_empty() {
        v.push("firsthand".into());
    }
    v.extend(["mcp".into(), "serve".into()]);
    v
}

/// Edit that removes our hook handlers and MCP entry from a JSON config file.
pub(crate) fn json_uninstall(
    path: PathBuf,
    scope: HookScope,
    mcp_container: Option<&'static str>,
) -> Result<Option<FileEdit>> {
    if !path.exists() {
        return Ok(None);
    }
    let desc = format!("remove firsthand entries from {}", path.display());
    json_edit(path, scope, desc, |t| {
        let mcp = mcp_container.map(is_our_mcp);
        jsonedit::remove_where(t, &|p: &[Seg], n: &Node| {
            is_our_hook(p, n) || mcp.as_ref().is_some_and(|f| f(p, n))
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diff_shapes() {
        let d = unified_diff(
            "a.json",
            b"{\n  \"x\": 1\n}\n",
            b"{\n  \"x\": 1,\n  \"y\": 2\n}\n",
        );
        assert_eq!(
            d,
            "--- a/a.json\n+++ b/a.json\n@@ -1,3 +1,4 @@\n {\n-  \"x\": 1\n+  \"x\": 1,\n+  \"y\": 2\n }\n"
        );
        let new = unified_diff("n.json", b"", b"{}\n");
        assert!(new.starts_with("--- /dev/null\n+++ b/n.json\n@@ -0,0 +1,1 @@\n+{}\n"));
        assert_eq!(unified_diff("s", b"x", b"x"), "");
        assert!(unified_diff("s", b"x", b"y").contains("\\ No newline at end of file"));
    }

    #[test]
    fn apply_backs_up_and_detects_staleness() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("cfg").join("s.json");
        let plan = HookPlan {
            agent: AgentId::ClaudeCode,
            edits: vec![FileEdit {
                path: p.clone(),
                before: None,
                after: b"{}\n".to_vec(),
                scope: HookScope::Repo,
                description: "create".into(),
            }],
            manual_steps: vec![],
        };
        assert!(apply_plan(&plan).unwrap().is_empty());
        assert_eq!(std::fs::read(&p).unwrap(), b"{}\n");
        // Same plan again: the file no longer matches `before`.
        assert!(
            apply_plan(&plan)
                .unwrap_err()
                .to_string()
                .contains("changed since")
        );
        let rm = HookPlan {
            edits: vec![FileEdit {
                before: Some(b"{}\n".to_vec()),
                after: Vec::new(),
                ..plan.edits[0].clone()
            }],
            ..plan
        };
        let baks = apply_plan(&rm).unwrap();
        assert!(!p.exists());
        assert_eq!(std::fs::read(&baks[0]).unwrap(), b"{}\n");
        assert!(baks[0].to_string_lossy().ends_with(".bak"));
    }
}
