//! Shared accumulator that turns parsed records into an [`IngestedSession`] with
//! caps applied. Everything here is hearsay; nothing is executed.

use crate::IngestOpts;
use crate::util::cap;
use fh_core::{AgentId, B3, IngestedMessage, IngestedSession, IngestedToolEvent, Role, Ts};
use std::collections::HashMap;

/// Per-message text cap.
pub(crate) const MSG_CAP: usize = 16 * 1024;
/// Per-tool-output cap.
pub(crate) const OUT_CAP: usize = 4 * 1024;

#[derive(Default)]
pub(crate) struct Acc {
    pub session_id: Option<String>,
    pub cwd: Option<String>,
    pub git_branch: Option<String>,
    started: Option<Ts>,
    ended: Option<Ts>,
    messages: Vec<IngestedMessage>,
    tools: Vec<IngestedToolEvent>,
    by_call: HashMap<String, usize>,
    pub skipped: u32,
}

impl Acc {
    /// Record a timestamp for start and end bounds.
    pub fn saw(&mut self, ts: Option<&Ts>) {
        if let Some(t) = ts {
            if self.started.as_ref().is_none_or(|s| t < s) {
                self.started = Some(t.clone());
            }
            if self.ended.as_ref().is_none_or(|e| t > e) {
                self.ended = Some(t.clone());
            }
        }
    }

    pub fn message(&mut self, role: Role, text: &str, ts: Option<Ts>, is_summary: bool) {
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        self.saw(ts.as_ref());
        self.messages.push(IngestedMessage {
            role,
            text: cap(text, MSG_CAP),
            ts,
            is_summary,
        });
    }

    pub fn tool_use(&mut self, call_id: Option<&str>, tool: &str, summary: String, ts: Option<Ts>) {
        self.saw(ts.as_ref());
        if let Some(id) = call_id {
            self.by_call.insert(id.to_owned(), self.tools.len());
        }
        self.tools.push(IngestedToolEvent {
            tool: cap(tool, 200),
            summary,
            output: None,
            ts,
            is_error: false,
        });
    }

    /// Attach a result to its call; unmatched results become their own event.
    pub fn tool_result(&mut self, call_id: Option<&str>, output: &str, is_error: bool) {
        let out = Some(cap(output.trim(), OUT_CAP)).filter(|o| !o.is_empty());
        match call_id.and_then(|id| self.by_call.get(id)).copied() {
            Some(i) => {
                self.tools[i].output = out;
                self.tools[i].is_error = is_error;
            }
            None => self.tools.push(IngestedToolEvent {
                tool: "unknown".into(),
                summary: String::new(),
                output: out,
                ts: None,
                is_error,
            }),
        }
    }

    pub fn finish(
        mut self,
        agent: AgentId,
        fallback_id: &str,
        hash: B3,
        opts: &IngestOpts,
    ) -> IngestedSession {
        let keep = opts.max_messages as usize;
        if self.messages.len() > keep {
            self.messages.drain(..self.messages.len() - keep);
        }
        if self.tools.len() > keep {
            self.tools.drain(..self.tools.len() - keep);
        }
        let final_assistant_text = self
            .messages
            .iter()
            .rev()
            .find(|m| m.role == Role::Assistant)
            .map(|m| m.text.clone())
            .unwrap_or_default();
        IngestedSession {
            agent,
            session_id: self.session_id.unwrap_or_else(|| fallback_id.to_owned()),
            cwd: self.cwd,
            git_branch: self.git_branch,
            started: self.started,
            ended: self.ended,
            messages: self.messages,
            tool_events: self.tools,
            final_assistant_text,
            source_hash: hash,
            skipped_records: self.skipped,
        }
    }
}
