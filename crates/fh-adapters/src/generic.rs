//! Generic: any agent that can run a shell command or speak MCP. No hooks and no
//! session logs; delivery goes through the AGENTS.md managed block.

use crate::plan::mcp_argv;
use crate::{
    AgentAdapter, BriefRef, Capabilities, ConfigPlan, Detection, HookPlan, HookScope, HostEnv,
    IngestOpts, Inject, LaunchSpec, RepoCtx, SessionRef, Witness,
};
use fh_core::{AgentId, Error, HookEvent, IngestedSession, Result};

/// Generic adapter.
#[derive(Debug, Clone, Copy, Default)]
pub struct Generic;

impl AgentAdapter for Generic {
    fn id(&self) -> AgentId {
        AgentId::Generic
    }

    fn detect(&self, _host: &HostEnv) -> Detection {
        Detection {
            installed: true,
            version: None,
            config_paths: Vec::new(),
            notes: vec![
                "generic needs no install: use firsthand run, note, claim, or the MCP server"
                    .into(),
            ],
        }
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            witness_bash: Witness::None,
            witness_edits: Witness::None,
            pre_tool_events: false,
            session_start_inject: Inject::File,
            stop_event: false,
            compact_event: false,
            subagent_events: false,
            ingest: false,
            mcp: true,
        }
    }

    fn discover_sessions(&self, _repo: &RepoCtx) -> Result<Vec<SessionRef>> {
        Ok(Vec::new())
    }

    fn ingest(&self, _s: &SessionRef, _opts: &IngestOpts) -> Result<IngestedSession> {
        Err(Error::Unsupported(
            "generic agents have no session log; use `firsthand import md <file>` for a markdown handoff".into(),
        ))
    }

    fn hook_plan(&self, repo: &RepoCtx, _scope: HookScope) -> Result<HookPlan> {
        Ok(HookPlan {
            agent: AgentId::Generic,
            edits: Vec::new(),
            manual_steps: vec![format!(
                "no hooks for generic agents: run commands through `{} run -- <command>` to witness them",
                repo.firsthand_cmd
            )],
        })
    }

    fn parse_hook_event(&self, _raw: &[u8], _argv_tail: &[String]) -> Result<HookEvent> {
        Err(Error::Unsupported(
            "generic agents send no hook events; use `firsthand run -- <command>`".into(),
        ))
    }

    fn mcp_install_plan(&self, repo: &RepoCtx, _scope: HookScope) -> Result<ConfigPlan> {
        Ok(HookPlan {
            agent: AgentId::Generic,
            edits: Vec::new(),
            manual_steps: vec![format!(
                "add a stdio MCP server to your agent's config with the command line: {}",
                crate::util::join_argv(
                    &mcp_argv(repo)
                        .into_iter()
                        .map(serde_json::Value::String)
                        .collect::<Vec<_>>()
                )
            )],
        })
    }

    fn launch_spec(&self, _brief: &BriefRef, _repo: &RepoCtx) -> Result<LaunchSpec> {
        Err(Error::Unsupported(
            "generic agents have no launch command; paste the bootstrap prompt into your agent"
                .into(),
        ))
    }
}
