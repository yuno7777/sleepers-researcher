use crate::backends::{ChatMessage, Router};
use crate::memory::MemoryStore;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::Mutex;
use tokio::sync::oneshot;

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Permissions {
    pub yolo: bool,
    pub auto_file_write: bool,
    pub auto_code_exec: bool,
}

impl Default for Permissions {
    fn default() -> Self {
        Self { yolo: false, auto_file_write: false, auto_code_exec: false }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub backend: String,
    pub model: String,
    pub mem_chunks: usize,
    pub context_tokens: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfo {
    pub id: i64,
    pub preview: String,
    pub ts: i64,
    pub active: bool,
}

/// Shared application state, accessed from Tauri command handlers.
pub struct AppState {
    pub backend: Mutex<String>,
    pub perms: Mutex<Permissions>,
    pub context_tokens: Mutex<usize>,
    /// Set by the Stop button; the ReAct loop checks it between steps.
    pub cancel: AtomicBool,
    /// Short-term memory: conversation history for the active session.
    pub conversation: Mutex<Vec<ChatMessage>>,
    /// Active chat session id (persisted across restarts).
    pub session: Mutex<i64>,
    /// LLM backend router (Gemini / Groq).
    pub router: Router,
    /// Long-term memory: RAG vector store + persistent chat history + soul.
    pub memory: MemoryStore,
    /// Pending permission requests awaiting a user decision from the UI modal.
    pub pending: Mutex<HashMap<u64, oneshot::Sender<bool>>>,
    pub next_perm_id: Mutex<u64>,
}

impl AppState {
    pub fn new() -> Self {
        let default_backend =
            std::env::var("DEFAULT_BACKEND").unwrap_or_else(|_| "gemini".to_string());
        let memory = MemoryStore::new();
        // Continue the most recent session; load its messages so context and the
        // chat panel survive restarts.
        let session = memory.latest_or_new_session();
        let history: Vec<ChatMessage> = memory
            .session_messages(session)
            .into_iter()
            .map(|(role, content)| ChatMessage { role, content })
            .collect();
        Self {
            backend: Mutex::new(default_backend),
            perms: Mutex::new(Permissions::default()),
            context_tokens: Mutex::new(0),
            cancel: AtomicBool::new(false),
            conversation: Mutex::new(history),
            session: Mutex::new(session),
            router: Router::from_env(),
            memory,
            pending: Mutex::new(HashMap::new()),
            next_perm_id: Mutex::new(1),
        }
    }
}
