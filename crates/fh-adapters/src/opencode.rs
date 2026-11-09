//! OpenCode: sessions in `opencode.db` (SQLite, opened read-only) or the legacy JSON
//! `storage/` tree; hooks through a TypeScript plugin; MCP in `opencode.json`.

use crate::hookparse::{self, base_event, other_tool, patch_files, tool};
use crate::ingest::Acc;
use crate::plan::{self, json_edit, json_uninstall};
use crate::util::{self, mtime, path_inside, plain, read_opt, s, summarize_input, ts};
use crate::{
    AgentAdapter, BriefRef, Capabilities, ConfigPlan, Detection, FileEdit, HookPlan, HookScope,
    HostEnv, IngestOpts, Inject, LaunchSpec, RepoCtx, SessionRef, Witness,
};
use fh_core::{
    AgentId, B3, Error, FileOp, HookEvent, HookEventKind, IngestedSession, Result, Role, ToolClass,
};
use rusqlite::{Connection, OpenFlags, params};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// Repo-relative path of the generated plugin.
pub const OPENCODE_PLUGIN_PATH: &str = ".opencode/plugins/firsthand.ts";

/// First line of the generated plugin; `uninstall` deletes only files that start with it.
const PLUGIN_MARKER: &str = "// firsthand hook plugin.";

/// OpenCode adapter.
#[derive(Debug, Clone)]
pub struct OpenCode {
    host: HostEnv,
}

/// TypeScript plugin that forwards OpenCode events to `<firsthand_cmd> hook opencode <event>`
/// with JSON on stdin. A failing or missing binary never breaks the session.
pub fn plugin_source(firsthand_cmd: &str) -> String {
    let mut argv = util::split_cmd(firsthand_cmd);
    if argv.is_empty() {
        argv.push("firsthand".into());
    }
    let argv = serde_json::to_string(&argv).unwrap_or_else(|_| "[\"firsthand\"]".into());
    format!(
        r#"{PLUGIN_MARKER} Managed by `firsthand init`; `firsthand uninstall` deletes this file.
// Each event is sent as JSON on stdin to: firsthand hook opencode <event>
const FIRSTHAND: string[] = {argv};
const EVENTS = ["session.created", "session.idle", "session.compacted", "file.edited"];
const pendingArgs = new Map<string, unknown>();

async function send(event: string, payload: Record<string, unknown>): Promise<void> {{
  try {{
    const proc = Bun.spawn([...FIRSTHAND, "hook", "opencode", event], {{
      stdin: new Blob([JSON.stringify({{ hook_event_name: event, ...payload }})]),
      stdout: "ignore",
      stderr: "ignore",
    }});
    await proc.exited;
  }} catch {{
    // Witnessing is best effort: never fail the tool call or the session.
  }}
}}

export const FirsthandPlugin = async ({{ directory }}: {{ directory: string }}) => ({{
  "tool.execute.before": async (input: any, output: any) => {{
    pendingArgs.set(input.callID, output.args);
    await send("tool.execute.before", {{
      cwd: directory,
      session_id: input.sessionID,
      tool_name: input.tool,
      tool_use_id: input.callID,
      tool_input: output.args,
    }});
  }},
  "tool.execute.after": async (input: any, output: any) => {{
    const args = pendingArgs.get(input.callID);
    pendingArgs.delete(input.callID);
    await send("tool.execute.after", {{
      cwd: directory,
      session_id: input.sessionID,
      tool_name: input.tool,
      tool_use_id: input.callID,
      tool_input: args,
      tool_output: output.output,
      tool_title: output.title,
      tool_metadata: output.metadata,
    }});
  }},
  event: async ({{ event }}: {{ event: any }}) => {{
    if (!EVENTS.includes(event.type)) return;
    const p = event.properties ?? {{}};
    await send(event.type, {{
      cwd: directory,
      session_id: p.sessionID ?? p.info?.id,
      file: p.file,
    }});
  }},
}});
"#
    )
}

impl OpenCode {
    /// Adapter looking at `host`.
    pub fn new(host: HostEnv) -> Self {
        OpenCode { host }
    }

    /// Candidate data directories, in probe order.
    fn data_dirs(&self) -> Vec<PathBuf> {
        let mut v = Vec::new();
        if let Some(x) = self.host.var("XDG_DATA_HOME") {
            v.push(PathBuf::from(x).join("opencode"));
        }
        if let Some(h) = &self.host.home {
            v.push(h.join(".local").join("share").join("opencode"));
        }
        for k in ["LOCALAPPDATA", "APPDATA"] {
            if let Some(x) = self.host.var(k) {
                v.push(PathBuf::from(x).join("opencode"));
            }
        }
        v.dedup();
        v
    }

    fn config_dir(&self) -> Result<PathBuf> {
        match self.host.var("XDG_CONFIG_HOME") {
            Some(x) => Ok(PathBuf::from(x).join("opencode")),
            None => Ok(util::home(&self.host)?.join(".config").join("opencode")),
        }
    }

    fn plugin_path(&self, repo: &RepoCtx, scope: HookScope) -> Result<PathBuf> {
        Ok(match scope {
            HookScope::Repo => repo
                .root
                .join(".opencode")
                .join("plugins")
                .join("firsthand.ts"),
            HookScope::User => self.config_dir()?.join("plugins").join("firsthand.ts"),
        })
    }

    fn config_file(&self, repo: &RepoCtx, scope: HookScope) -> Result<PathBuf> {
        let dir = match scope {
            HookScope::Repo => repo.root.clone(),
            HookScope::User => self.config_dir()?,
        };
        let jsonc = dir.join("opencode.jsonc");
        Ok(if !dir.join("opencode.json").exists() && jsonc.exists() {
            jsonc
        } else {
            dir.join("opencode.json")
        })
    }

    pub(crate) fn uninstall(&self, repo: &RepoCtx, scope: HookScope) -> Result<HookPlan> {
        let mut edits = Vec::new();
        let plugin = self.plugin_path(repo, scope)?;
        if let Some(before) = read_opt(&plugin)?
            && before.starts_with(PLUGIN_MARKER.as_bytes())
        {
            edits.push(FileEdit {
                description: format!("delete {}", plugin.display()),
                path: plugin,
                before: Some(before),
                after: Vec::new(),
                scope,
            });
        }
        edits.extend(json_uninstall(
            self.config_file(repo, scope)?,
            scope,
            Some("mcp"),
        )?);
        Ok(HookPlan {
            agent: AgentId::Opencode,
            edits,
            manual_steps: Vec::new(),
        })
    }
}

impl AgentAdapter for OpenCode {
    fn id(&self) -> AgentId {
        AgentId::Opencode
    }

    fn detect(&self, host: &HostEnv) -> Detection {
        let me = OpenCode::new(host.clone());
        let mut configs: Vec<PathBuf> = me
            .data_dirs()
            .into_iter()
            .flat_map(|d| [d.join("opencode.db"), d.join("storage")])
            .collect();
        configs.extend(me.config_dir().ok().map(|d| d.join("opencode.json")));
        util::detect(host, "opencode", configs)
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            witness_bash: Witness::Partial,
            witness_edits: Witness::Partial,
            pre_tool_events: true,
            session_start_inject: Inject::None,
            stop_event: true,
            compact_event: true,
            subagent_events: false,
            ingest: true,
            mcp: true,
        }
    }

    fn discover_sessions(&self, repo: &RepoCtx) -> Result<Vec<SessionRef>> {
        let root = plain(&repo.root);
        let mut out = Vec::new();
        for dir in self.data_dirs() {
            let db = dir.join("opencode.db");
            if db.is_file() {
                out.extend(db_sessions(&db, &root)?);
            }
            out.extend(storage_sessions(&dir.join("storage"), &root));
        }
        out.sort_by(|a, b| b.modified.cmp(&a.modified).then(a.id.cmp(&b.id)));
        Ok(out)
    }

    fn ingest(&self, sref: &SessionRef, opts: &IngestOpts) -> Result<IngestedSession> {
        if sref.path.extension().is_some_and(|e| e == "db") {
            db_ingest(sref, opts)
        } else {
            storage_ingest(sref, opts)
        }
    }

    fn hook_plan(&self, repo: &RepoCtx, scope: HookScope) -> Result<HookPlan> {
        let path = self.plugin_path(repo, scope)?;
        let after = plugin_source(&repo.firsthand_cmd).into_bytes();
        let before = read_opt(&path)?;
        if let Some(b) = &before
            && !b.starts_with(PLUGIN_MARKER.as_bytes())
        {
            return Err(Error::Config {
                path,
                reason: "exists and was not written by firsthand; move it away and run again"
                    .into(),
            });
        }
        let edits = if before.as_deref() == Some(&after[..]) {
            Vec::new()
        } else {
            vec![FileEdit {
                description: format!("write the firsthand plugin {}", path.display()),
                path,
                before,
                after,
                scope,
            }]
        };
        Ok(HookPlan {
            agent: AgentId::Opencode,
            edits,
            manual_steps: vec![
                "OpenCode loads plugins at startup: restart open OpenCode sessions in this repo"
                    .into(),
            ],
        })
    }

    fn parse_hook_event(&self, raw: &[u8], argv_tail: &[String]) -> Result<HookEvent> {
        let v = hookparse::payload(raw, argv_tail)?;
        let mut ev = base_event(AgentId::Opencode, &v, argv_tail);
        if ev.raw_event_name == "file.edited" {
            let mut t = tool("file.edited", ToolClass::FileEdit);
            t.file_paths = s(&v, "file").map(str::to_owned).into_iter().collect();
            t.file_op = Some(FileOp::Edit);
            ev.tool = Some(t);
        } else if let Some(name) = s(&v, "tool_name") {
            ev.tool = Some(opencode_tool(name, &v, ev.kind));
        }
        Ok(ev)
    }

    fn mcp_install_plan(&self, repo: &RepoCtx, scope: HookScope) -> Result<ConfigPlan> {
        let path = self.config_file(repo, scope)?;
        let entry = json!({"type": "local", "command": plan::mcp_argv(repo), "enabled": true});
        let desc = format!("add the firsthand MCP server to {}", path.display());
        let edit = json_edit(path, scope, desc, |t| {
            crate::jsonedit::insert_member(t, &["mcp"], "firsthand", &entry)
        })?;
        Ok(HookPlan {
            agent: AgentId::Opencode,
            edits: edit.into_iter().collect(),
            manual_steps: Vec::new(),
        })
    }

    fn launch_spec(&self, brief: &BriefRef, repo: &RepoCtx) -> Result<LaunchSpec> {
        Ok(LaunchSpec {
            argv: vec![
                "opencode".into(),
                "--prompt".into(),
                brief.bootstrap.clone(),
            ],
            env: Vec::new(),
            cwd: repo.root.clone(),
            verified: false,
        })
    }
}

fn opencode_tool(name: &str, v: &Value, kind: HookEventKind) -> fh_core::ToolCall {
    let input = v.get("tool_input").unwrap_or(&Value::Null);
    let post = kind == HookEventKind::PostToolUse;
    let meta = v.get("tool_metadata").unwrap_or(&Value::Null);
    match name {
        "bash" => {
            let mut t = tool(name, ToolClass::Shell);
            (t.command, t.argv) = hookparse::shell_command(input);
            if post {
                t.exit_code = ["exit", "exitCode", "exit_code"]
                    .iter()
                    .find_map(|k| meta.get(*k).and_then(Value::as_i64))
                    .and_then(|c| i32::try_from(c).ok());
                t.stdout = s(v, "tool_output")
                    .or_else(|| s(meta, "output"))
                    .map(str::to_owned);
                t.is_error = t.exit_code.map(|c| c != 0);
            }
            t
        }
        "write" | "edit" | "multiedit" | "patch" | "apply_patch" => {
            let mut t = tool(name, ToolClass::FileEdit);
            if let Some(p) = hookparse::find_string(input, "*** Begin Patch") {
                (t.file_paths, t.file_op) = patch_files(p);
            } else {
                t.file_paths = ["filePath", "file_path", "path"]
                    .iter()
                    .find_map(|k| s(input, k))
                    .map(str::to_owned)
                    .into_iter()
                    .collect();
                t.file_op = Some(if name == "write" {
                    FileOp::Write
                } else {
                    FileOp::Edit
                });
            }
            t
        }
        _ => other_tool(name, input),
    }
}

// ------------------------------------------------------------ SQLite

/// `file:` URI for a read-only open. Percent-encodes characters URIs reserve.
fn ro_uri(path: &Path) -> String {
    let p = plain(path).replace('\\', "/");
    let mut enc = String::new();
    for c in p.chars() {
        match c {
            '%' | '?' | '#' | ' ' => enc.push_str(&format!("%{:02X}", c as u32)),
            _ => enc.push(c),
        }
    }
    let lead = if enc.starts_with('/') { "" } else { "/" };
    format!("file:{lead}{enc}?mode=ro")
}

fn open_ro(db: &Path) -> Result<Connection> {
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY
        | OpenFlags::SQLITE_OPEN_URI
        | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    Connection::open_with_flags(ro_uri(db), flags).map_err(|e| sql_err(db, e))
}

fn sql_err(db: &Path, e: rusqlite::Error) -> Error {
    Error::Other(format!(
        "opencode database {}: {e} (opened read-only; close OpenCode if it is mid-migration and retry)",
        db.display()
    ))
}

fn columns(c: &Connection, table: &str) -> Vec<String> {
    let Ok(mut st) = c.prepare(&format!("PRAGMA table_info({table})")) else {
        return Vec::new();
    };
    st.query_map([], |r| r.get::<_, String>(1))
        .map(|rows| rows.flatten().collect())
        .unwrap_or_default()
}

/// Refuse schemas this reader does not know, naming what was found.
fn check_schema(c: &Connection, db: &Path) -> Result<bool> {
    let need: [(&str, &[&str]); 3] = [
        ("session", &["id", "directory"]),
        ("message", &["id", "session_id", "data"]),
        ("part", &["message_id", "data"]),
    ];
    for (t, cols) in need {
        let have = columns(c, t);
        if let Some(missing) = cols.iter().find(|col| !have.iter().any(|h| h == *col)) {
            return Err(Error::Unsupported(format!(
                "opencode database {} has an unknown schema (table {t} lacks column {missing}; found {:?}); update firsthand or report the opencode version",
                db.display(),
                have
            )));
        }
    }
    Ok(columns(c, "session").iter().any(|h| h == "time_updated"))
}

fn db_sessions(db: &Path, root: &str) -> Result<Vec<SessionRef>> {
    let c = open_ro(db)?;
    let has_time = check_schema(&c, db)?;
    let sql = if has_time {
        "SELECT id, directory, time_updated FROM session"
    } else {
        "SELECT id, directory, NULL FROM session"
    };
    let mut st = c.prepare(sql).map_err(|e| sql_err(db, e))?;
    let rows = st
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, Option<i64>>(2)?,
            ))
        })
        .map_err(|e| sql_err(db, e))?;
    let mut out = Vec::new();
    for row in rows.flatten() {
        let (id, dir, t) = row;
        if let Some(dir) = dir.filter(|d| path_inside(d, root)) {
            out.push(SessionRef {
                agent: AgentId::Opencode,
                id,
                path: db.to_path_buf(),
                modified: t.map(fh_core::Ts::from_unix_ms).or_else(|| mtime(db)),
                cwd: Some(dir),
            });
        }
    }
    Ok(out)
}

fn db_ingest(sref: &SessionRef, opts: &IngestOpts) -> Result<IngestedSession> {
    let db = &sref.path;
    let c = open_ro(db)?;
    check_schema(&c, db)?;
    let mut acc = Acc::default();
    acc.session_id = Some(sref.id.clone());
    acc.cwd = c
        .query_row(
            "SELECT directory FROM session WHERE id = ?1",
            params![sref.id],
            |r| r.get::<_, Option<String>>(0),
        )
        .map_err(|e| sql_err(db, e))?;
    let mut st = c
        .prepare("SELECT id, data FROM message WHERE session_id = ?1 ORDER BY id DESC LIMIT ?2")
        .map_err(|e| sql_err(db, e))?;
    let mut msgs: Vec<(String, String)> = st
        .query_map(params![sref.id, i64::from(opts.max_messages)], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .map_err(|e| sql_err(db, e))?
        .flatten()
        .collect();
    msgs.reverse();
    let mut pst = c
        .prepare("SELECT data FROM part WHERE message_id = ?1 ORDER BY id")
        .map_err(|e| sql_err(db, e))?;
    let mut hasher = blake3::Hasher::new();
    let mut budget = if opts.max_bytes == 0 {
        u64::MAX
    } else {
        opts.max_bytes
    };
    for (id, data) in msgs {
        hasher.update(data.as_bytes());
        let parts: Vec<String> = pst
            .query_map(params![id], |r| r.get(0))
            .map_err(|e| sql_err(db, e))?
            .flatten()
            .collect();
        let mut part_vals = Vec::new();
        for p in parts {
            hasher.update(p.as_bytes());
            budget = budget.saturating_sub(p.len() as u64);
            if budget == 0 {
                acc.skipped += 1;
                continue;
            }
            match serde_json::from_str::<Value>(&p) {
                Ok(v) => part_vals.push(v),
                Err(_) => acc.skipped += 1,
            }
        }
        match serde_json::from_str::<Value>(&data) {
            Ok(info) => message(&mut acc, &info, &part_vals),
            Err(_) => acc.skipped += 1,
        }
    }
    let hash = B3::parse(&format!("b3:{}", hasher.finalize().to_hex()))?;
    Ok(acc.finish(AgentId::Opencode, &sref.id, hash, opts))
}

// ------------------------------------------------------------ legacy JSON storage

fn json_files(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|e| e == "json"))
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

fn storage_sessions(storage: &Path, root: &str) -> Vec<SessionRef> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(storage.join("session")) else {
        return out;
    };
    for project in rd.flatten().filter(|e| e.path().is_dir()) {
        for f in json_files(&project.path()) {
            let Ok(b) = std::fs::read(&f) else { continue };
            let Ok(v) = serde_json::from_slice::<Value>(&b) else {
                continue;
            };
            let Some(dir) = s(&v, "directory").filter(|d| path_inside(d, root)) else {
                continue;
            };
            out.push(SessionRef {
                agent: AgentId::Opencode,
                id: s(&v, "id").map(str::to_owned).unwrap_or_else(|| {
                    f.file_stem()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_default()
                }),
                modified: v
                    .get("time")
                    .and_then(|t| ts(t.get("updated")))
                    .or_else(|| mtime(&f)),
                cwd: Some(dir.to_owned()),
                path: f,
            });
        }
    }
    out
}

fn storage_ingest(sref: &SessionRef, opts: &IngestOpts) -> Result<IngestedSession> {
    let storage = sref
        .path
        .ancestors()
        .find(|a| a.file_name().is_some_and(|n| n == "storage"))
        .ok_or_else(|| Error::Invalid {
            what: "opencode session path",
            value: sref.path.display().to_string(),
            reason: "not inside an opencode storage directory".into(),
        })?
        .to_path_buf();
    let mut hasher = blake3::Hasher::new();
    let info_bytes = std::fs::read(&sref.path).map_err(|e| Error::io(&sref.path, e))?;
    hasher.update(&info_bytes);
    let mut acc = Acc::default();
    if let Ok(info) = serde_json::from_slice::<Value>(&info_bytes) {
        acc.session_id = s(&info, "id").map(str::to_owned);
        acc.cwd = s(&info, "directory").map(str::to_owned);
    }
    let mut files = json_files(&storage.join("message").join(&sref.id));
    let keep = opts.max_messages as usize;
    if files.len() > keep {
        files.drain(..files.len() - keep);
    }
    let mut budget = if opts.max_bytes == 0 {
        u64::MAX
    } else {
        opts.max_bytes
    };
    for f in files {
        let Ok(b) = std::fs::read(&f) else {
            acc.skipped += 1;
            continue;
        };
        hasher.update(&b);
        let Ok(info) = serde_json::from_slice::<Value>(&b) else {
            acc.skipped += 1;
            continue;
        };
        let mid = s(&info, "id").map(str::to_owned).unwrap_or_else(|| {
            f.file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default()
        });
        let mut parts = Vec::new();
        for pf in json_files(&storage.join("part").join(&mid)) {
            let Ok(pb) = std::fs::read(&pf) else {
                acc.skipped += 1;
                continue;
            };
            hasher.update(&pb);
            budget = budget.saturating_sub(pb.len() as u64);
            match serde_json::from_slice::<Value>(&pb) {
                Ok(v) if budget > 0 => parts.push(v),
                _ => acc.skipped += 1,
            }
        }
        message(&mut acc, &info, &parts);
    }
    let hash = B3::parse(&format!("b3:{}", hasher.finalize().to_hex()))?;
    Ok(acc.finish(AgentId::Opencode, &sref.id, hash, opts))
}

/// Fold one message (info plus parts) into the accumulator.
fn message(acc: &mut Acc, info: &Value, parts: &[Value]) {
    let t = info.get("time").and_then(|t| ts(t.get("created")));
    let role = match s(info, "role") {
        Some("assistant") => Role::Assistant,
        Some("user") => Role::User,
        _ => Role::System,
    };
    let inline = info.get("parts").and_then(Value::as_array);
    let parts: Vec<&Value> = if parts.is_empty() {
        inline.into_iter().flatten().collect()
    } else {
        parts.iter().collect()
    };
    let mut text = Vec::new();
    let summary = info.get("summary") == Some(&Value::Bool(true));
    for p in parts {
        match s(p, "type") {
            Some("text") => text.push(s(p, "text").unwrap_or("").to_owned()),
            Some("tool") => {
                let state = p.get("state").unwrap_or(&Value::Null);
                let input = state.get("input").unwrap_or(&Value::Null);
                let started = state.get("time").and_then(|x| ts(x.get("start")));
                let call = s(p, "callID").or_else(|| s(p, "id"));
                acc.tool_use(
                    call,
                    s(p, "tool").unwrap_or("tool"),
                    summarize_input(input),
                    started.or(t.clone()),
                );
                let status = s(state, "status").unwrap_or("");
                if matches!(status, "completed" | "error") {
                    let out = s(state, "output")
                        .or_else(|| s(state, "error"))
                        .unwrap_or("");
                    acc.tool_result(call, out, status == "error");
                }
            }
            // reasoning parts are hidden reasoning: never ingested.
            _ => {}
        }
    }
    acc.message(
        if summary { Role::System } else { role },
        &text.join("\n"),
        t,
        summary,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uri_escapes() {
        assert_eq!(
            ro_uri(Path::new("/home/a b/x?#%.db")),
            "file:/home/a%20b/x%3F%23%25.db?mode=ro"
        );
        assert_eq!(
            ro_uri(Path::new(r"C:\Users\x\o.db")),
            "file:/C:/Users/x/o.db?mode=ro"
        );
    }

    #[test]
    fn plugin_quotes_paths() {
        let p = plugin_source(r#""C:\Program Files\fh\firsthand.exe""#);
        assert!(p.starts_with(PLUGIN_MARKER));
        assert!(
            p.contains(r#"const FIRSTHAND: string[] = ["C:\\Program Files\\fh\\firsthand.exe"];"#)
        );
        assert!(p.contains("firsthand hook opencode"));
    }
}
