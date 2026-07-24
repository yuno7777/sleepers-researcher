//! ReAct loop. Each step asks the model for a single JSON object — either a
//! tool call `{tool, args}` or a final `{answer}`. Tool observations feed back
//! until the model answers (or we hit the step cap). Before reasoning, relevant
//! long-term memory and the user "soul" are retrieved and injected; afterwards
//! the turn is persisted and embedded so it is recallable in future chats.

use crate::backends::ChatMessage;
use crate::state::AppState;
use crate::tools::{self, Tool};
use serde_json::{json, Value};
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Emitter, Manager};

const MAX_STEPS: usize = 6;
/// Most recent conversation turns sent to the model each request.
const MAX_HISTORY: usize = 24;

fn system_prompt(tools: &[Box<dyn Tool>], memory_ctx: &str) -> String {
    let mut p = format!(
        "You are Sleepers Researcher, a local-first AI research and dev agent using a ReAct loop.\n\
You may use these tools:\n{}\n\
On each turn respond with EXACTLY ONE JSON object and nothing else.\n\
To call a tool: {{\"thought\":\"why\",\"tool\":\"<name>\",\"args\":{{...}}}}\n\
When you can answer the user: {{\"thought\":\"why\",\"answer\":\"<final answer>\"}}\n\
Rules: for deep research, web_search to find sources, then web_fetch the most promising \
URLs to read full page content, and cross-reference several sources before answering — do \
not rely on snippets alone. Use arxiv for papers. Use the shell tool to run scripts/tooling \
on the PC and create_pdf to produce PDF reports. Read files before writing them. Use recall \
to look up past context, and remember to save durable facts the user shares about themselves, \
their projects, goals, or preferences (store those with kind \"soul\"). Be efficient: don't \
call tools you don't need. Keep answers precise and technical. \
Never wrap the JSON in markdown fences. Only one JSON object per turn.",
        tools::tools_prompt(tools)
    );
    if !memory_ctx.is_empty() {
        p.push_str("\n\n--- MEMORY ---\n");
        p.push_str(memory_ctx);
    }
    p
}

pub async fn run(app: AppHandle, backend_name: String) {
    let state = app.state::<AppState>();
    let toolset = tools::registry();

    // The user query just added to the conversation.
    let query = state
        .conversation
        .lock()
        .unwrap()
        .iter()
        .rev()
        .find(|m| m.role == "user")
        .map(|m| m.content.clone())
        .unwrap_or_default();

    // Retrieve long-term memory + soul and build the injected context.
    let memory_ctx = build_memory_context(&app, &query).await;

    let mut messages = vec![ChatMessage::system(system_prompt(&toolset, &memory_ctx))];
    {
        // Keep only the most recent turns so long chats never blow the context
        // window — older turns remain retrievable through long-term memory.
        let conv = state.conversation.lock().unwrap();
        let start = conv.len().saturating_sub(MAX_HISTORY);
        messages.extend(conv[start..].iter().cloned());
    }

    let backend = state.router.backend(&backend_name);
    let mut final_answer: Option<String> = None;

    for _ in 0..MAX_STEPS {
        if state.cancel.load(Ordering::Relaxed) {
            final_answer = Some("Stopped.".into());
            break;
        }
        let resp = match backend.complete(&messages).await {
            Ok(r) => r.content,
            Err(e) => {
                final_answer = Some(format!("[backend error] {e}"));
                break;
            }
        };

        match extract_json(&resp) {
            Some(v) if v.get("tool").is_some() => {
                let tool_name = v["tool"].as_str().unwrap_or("").to_string();
                let args = v.get("args").cloned().unwrap_or_else(|| json!({}));
                let _ = app.emit(
                    "agent:tool",
                    json!({ "text": format!("{}  {}", tool_name, compact(&args)) }),
                );

                let observation = match toolset.iter().find(|t| t.name() == tool_name) {
                    Some(t) => match t.execute(&args, &app).await {
                        Ok(o) => o,
                        Err(e) => format!("error: {e}"),
                    },
                    None => format!("unknown tool '{tool_name}'"),
                };

                messages.push(ChatMessage::assistant(resp));
                messages.push(ChatMessage::user(format!("Observation: {observation}")));
            }
            Some(v) if v.get("answer").is_some() => {
                final_answer = Some(v["answer"].as_str().unwrap_or("").to_string());
                break;
            }
            _ => {
                final_answer = Some(resp);
                break;
            }
        }
    }

    let answer =
        final_answer.unwrap_or_else(|| "I couldn't complete this within the step limit.".into());

    // Stream the final answer to the chat panel.
    for piece in answer.split_inclusive(' ') {
        if state.cancel.load(Ordering::Relaxed) {
            break;
        }
        let _ = app.emit("agent:token", piece);
        tokio::time::sleep(std::time::Duration::from_millis(7)).await;
    }

    // Persist + embed the turn so it is recallable across future chats.
    let session = *state.session.lock().unwrap();
    state.memory.add_message(session, "assistant", &answer);
    {
        let mut conv = state.conversation.lock().unwrap();
        conv.push(ChatMessage::assistant(answer.clone()));
        let used: usize = conv.iter().map(|m| m.content.len() / 4 + 4).sum();
        *state.context_tokens.lock().unwrap() = used;
    }
    let turn = format!("User: {}\nAssistant: {}", query, answer);
    let _ = state.memory.remember("conversation", &turn, "chat").await;

    let _ = app.emit("agent:done", ());
    let _ = app.emit("memory:update", ());
}

/// Embed the query, retrieve top memories, and format soul + hits for injection.
async fn build_memory_context(app: &AppHandle, query: &str) -> String {
    let state = app.state::<AppState>();
    let soul = state.memory.soul_facts();
    let hits = if query.is_empty() {
        Vec::new()
    } else {
        state.memory.retrieve(query, 6).await
    };

    let mut ctx = String::new();
    if !soul.is_empty() {
        ctx.push_str("What you know about the user (soul):\n");
        for f in &soul {
            ctx.push_str(&format!("- {f}\n"));
        }
    }
    let relevant: Vec<_> = hits.iter().filter(|(_, _, _, score)| *score > 0.35).collect();
    if !relevant.is_empty() {
        ctx.push_str("\nRelevant memory (retrieved):\n");
        for (kind, content, source, score) in relevant {
            ctx.push_str(&format!(
                "- [{kind} {:.2}] {} (src: {})\n",
                score,
                truncate(content, 280),
                truncate(source, 80)
            ));
        }
    }
    ctx
}

fn truncate(s: &str, n: usize) -> String {
    if s.len() > n {
        format!("{}…", &s[..n])
    } else {
        s.to_string()
    }
}

fn extract_json(s: &str) -> Option<Value> {
    let trimmed = s.trim();
    if let Ok(v) = serde_json::from_str::<Value>(trimmed) {
        return Some(v);
    }
    let start = trimmed.find('{')?;
    let end = trimmed.rfind('}')?;
    if end > start {
        serde_json::from_str::<Value>(&trimmed[start..=end]).ok()
    } else {
        None
    }
}

fn compact(v: &Value) -> String {
    let s = v.to_string();
    if s.len() > 120 {
        format!("{}…", &s[..120])
    } else {
        s
    }
}
