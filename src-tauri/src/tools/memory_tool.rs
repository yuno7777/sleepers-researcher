//! Memory tools. `remember` stores durable facts (the user "soul") or notes
//! into long-term memory; `recall` retrieves relevant memories. Both are
//! local-only and non-mutating to the filesystem, so they run without a modal —
//! they are logged to the activity panel.

use super::{activity, Tool};
use crate::state::AppState;
use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};

pub struct RememberTool;

#[async_trait::async_trait]
impl Tool for RememberTool {
    fn name(&self) -> &'static str {
        "remember"
    }
    fn description(&self) -> &'static str {
        "Save a durable fact to long-term memory. Use kind 'soul' for facts about the user."
    }
    fn args_hint(&self) -> Value {
        json!({ "content": "the fact to remember", "kind": "soul | note (default soul)" })
    }
    fn mutating(&self) -> bool {
        false
    }
    async fn execute(&self, args: &Value, app: &AppHandle) -> Result<String> {
        let content = args["content"].as_str().ok_or_else(|| anyhow!("missing 'content'"))?;
        let kind = match args["kind"].as_str() {
            Some("note") => "note",
            _ => "soul",
        };
        let state = app.state::<AppState>();
        state.memory.remember(kind, content, "agent").await?;
        activity(app, "memory", format!("remembered ({kind}): {content}"));
        let _ = app.emit("memory:update", ());
        Ok(format!("Saved to long-term memory ({kind})."))
    }
}

pub struct RecallTool;

#[async_trait::async_trait]
impl Tool for RecallTool {
    fn name(&self) -> &'static str {
        "recall"
    }
    fn description(&self) -> &'static str {
        "Search long-term memory (past chats, documents, images, soul) for relevant info."
    }
    fn args_hint(&self) -> Value {
        json!({ "query": "what to look up" })
    }
    fn mutating(&self) -> bool {
        false
    }
    async fn execute(&self, args: &Value, app: &AppHandle) -> Result<String> {
        let query = args["query"].as_str().ok_or_else(|| anyhow!("missing 'query'"))?;
        activity(app, "recall", query.to_string());
        let state = app.state::<AppState>();
        let hits = state.memory.retrieve(query, 6).await;
        if hits.is_empty() {
            return Ok("No relevant memories found.".into());
        }
        let mut out = String::new();
        for (kind, content, source, score) in hits {
            if score < 0.3 {
                continue;
            }
            let c = if content.len() > 400 { &content[..400] } else { &content };
            out.push_str(&format!("[{kind} {score:.2}] (src: {source})\n{c}\n\n"));
        }
        if out.is_empty() {
            out = "No sufficiently relevant memories found.".into();
        }
        Ok(out)
    }
}
