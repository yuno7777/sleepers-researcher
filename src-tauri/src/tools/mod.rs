//! Tool layer. Each tool implements `Tool`. Read-only tools (web search, arXiv,
//! file read) run freely and are logged to the activity panel. Mutating tools
//! (file write, code execution) must pass through `request_permission`, which
//! drives the confirmation modal in the UI — unless YOLO / per-tool
//! auto-approve is enabled.

pub mod arxiv;
pub mod code_exec;
pub mod deep_research;
pub mod file_ops;
pub mod memory_tool;
pub mod openalex;
pub mod pdf_create;
pub mod shell;
pub mod web_fetch;
pub mod web_search;
pub mod wikipedia;

use crate::state::AppState;
use anyhow::Result;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};

/// A capability the agent can invoke.
#[async_trait::async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    /// JSON-schema-ish description of arguments, shown to the model.
    fn args_hint(&self) -> Value;
    /// Whether invoking this tool mutates state (requires confirmation).
    fn mutating(&self) -> bool;
    /// Run the tool. Returns an observation string fed back to the model.
    async fn execute(&self, args: &Value, app: &AppHandle) -> Result<String>;
}

/// Build the default tool registry.
pub fn registry() -> Vec<Box<dyn Tool>> {
    vec![
        Box::new(deep_research::DeepResearchTool),
        Box::new(file_ops::ReadFileTool),
        Box::new(file_ops::WriteFileTool),
        Box::new(code_exec::CodeExecTool),
        Box::new(shell::ShellTool),
        Box::new(web_search::WebSearchTool),
        Box::new(web_fetch::WebFetchTool),
        Box::new(wikipedia::WikipediaTool),
        Box::new(openalex::OpenAlexTool),
        Box::new(arxiv::ArxivTool),
        Box::new(pdf_create::CreatePdfTool),
        Box::new(memory_tool::RememberTool),
        Box::new(memory_tool::RecallTool),
    ]
}

/// A compact, model-facing description of all tools for the ReAct system prompt.
pub fn tools_prompt(tools: &[Box<dyn Tool>]) -> String {
    let mut s = String::new();
    for t in tools {
        s.push_str(&format!(
            "- {}: {} | args: {}{}\n",
            t.name(),
            t.description(),
            t.args_hint(),
            if t.mutating() { " [needs user approval]" } else { "" }
        ));
    }
    s
}

/// Log a read-only / informational tool action to the activity panel.
pub fn activity(app: &AppHandle, kind: &str, detail: impl Into<String>) {
    let _ = app.emit(
        "activity:log",
        json!({ "kind": kind, "detail": detail.into() }),
    );
}

/// Gate a mutating action behind the UI confirmation modal.
/// Returns true if the user approved (or approval was auto-granted).
pub async fn request_permission(
    app: &AppHandle,
    kind: &str,
    title: &str,
    sub: &str,
    body: &str,
) -> bool {
    // Check auto-approve / YOLO without holding the lock across await.
    let (id, rx) = {
        let state = app.state::<AppState>();
        let perms = state.perms.lock().unwrap().clone();
        if perms.yolo
            || (kind == "file_write" && perms.auto_file_write)
            || (kind == "code_exec" && perms.auto_code_exec)
        {
            activity(app, "permission", format!("{kind} auto-approved"));
            return true;
        }
        crate::commands::new_permission(&state)
    };

    let _ = app.emit(
        "permission:request",
        json!({
            "id": id,
            "kind": kind,
            "title": title,
            "sub": sub,
            "body": body,
        }),
    );

    // Default to deny on channel error / window close.
    rx.await.unwrap_or(false)
}
