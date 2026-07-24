use crate::backends::ChatMessage;
use crate::state::{AppState, Permissions, SessionInfo, Status};
use serde_json::json;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::oneshot;

#[tauri::command]
pub fn get_status(state: State<'_, AppState>) -> Status {
    let requested = state.backend.lock().unwrap().clone();
    let resolved = state.router.backend(&requested);
    Status {
        backend: resolved.name().to_string(),
        model: resolved.model().to_string(),
        mem_chunks: state.memory.count(),
        context_tokens: *state.context_tokens.lock().unwrap(),
    }
}

/// Recent chat sessions for the sidebar switcher.
#[tauri::command]
pub fn list_sessions(state: State<'_, AppState>) -> Vec<SessionInfo> {
    let active = *state.session.lock().unwrap();
    state
        .memory
        .list_sessions(30)
        .into_iter()
        .map(|(id, ts, preview)| SessionInfo {
            id,
            ts,
            preview: if preview.trim().is_empty() {
                "New chat".to_string()
            } else {
                preview.chars().take(60).collect()
            },
            active: id == active,
        })
        .collect()
}

/// Switch to a past chat and return its messages for rendering.
#[tauri::command]
pub fn load_session(state: State<'_, AppState>, id: i64) -> Vec<ChatMessage> {
    let msgs: Vec<ChatMessage> = state
        .memory
        .session_messages(id)
        .into_iter()
        .map(|(role, content)| ChatMessage { role, content })
        .collect();
    *state.session.lock().unwrap() = id;
    *state.conversation.lock().unwrap() = msgs.clone();
    *state.context_tokens.lock().unwrap() =
        msgs.iter().map(|m| m.content.len() / 4 + 4).sum();
    msgs
}

/// Ask the running agent loop to stop after the current step.
#[tauri::command]
pub fn cancel_agent(state: State<'_, AppState>) {
    state.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// Messages for the active session — used to render history on app launch.
#[tauri::command]
pub fn get_history(state: State<'_, AppState>) -> Vec<ChatMessage> {
    state.conversation.lock().unwrap().clone()
}

/// Durable facts about the user (the "soul"), for the settings panel.
#[tauri::command]
pub fn get_soul(state: State<'_, AppState>) -> Vec<String> {
    state.memory.soul_facts()
}

/// Start a fresh chat session. Past chats remain in long-term memory.
#[tauri::command]
pub fn new_chat(state: State<'_, AppState>) {
    let id = state.memory.new_session("Session");
    *state.session.lock().unwrap() = id;
    state.conversation.lock().unwrap().clear();
    *state.context_tokens.lock().unwrap() = 0;
}

#[tauri::command]
pub fn set_backend(state: State<'_, AppState>, backend: String) {
    *state.backend.lock().unwrap() = backend;
}

#[tauri::command]
pub fn set_permissions(state: State<'_, AppState>, perms: Permissions) {
    *state.perms.lock().unwrap() = perms;
}

/// Resolve a pending permission request (called by the confirmation modal).
#[tauri::command]
pub fn permission_respond(state: State<'_, AppState>, id: u64, approved: bool) {
    if let Some(tx) = state.pending.lock().unwrap().remove(&id) {
        let _ = tx.send(approved);
    }
}

/// Register a pending permission request and return its id + receiver.
/// Used by the agent loop (next milestone) to gate mutating tools.
#[allow(dead_code)]
pub fn new_permission(state: &AppState) -> (u64, oneshot::Receiver<bool>) {
    let mut id_guard = state.next_perm_id.lock().unwrap();
    let id = *id_guard;
    *id_guard += 1;
    let (tx, rx) = oneshot::channel();
    state.pending.lock().unwrap().insert(id, tx);
    (id, rx)
}

#[tauri::command]
pub async fn chat_send(app: AppHandle, message: String, backend: String) -> Result<(), String> {
    {
        let state = app.state::<AppState>();
        state.cancel.store(false, std::sync::atomic::Ordering::Relaxed);
        *state.backend.lock().unwrap() = backend.clone();
        let session = *state.session.lock().unwrap();
        state
            .conversation
            .lock()
            .unwrap()
            .push(ChatMessage::user(message.clone()));
        state.memory.add_message(session, "user", &message);
    }
    let _ = app.emit("activity:log", json!({ "kind": "chat", "detail": format!("→ {backend}") }));

    tauri::async_runtime::spawn(async move {
        {
            let state = app.state::<AppState>();
            if !state.router.is_configured(&backend) {
                let _ = app.emit(
                    "agent:token",
                    format!("No API key configured for {backend}. Add it to .env and restart."),
                );
                let _ = app.emit("agent:done", ());
                return;
            }
        }
        // Hand off to the ReAct agent loop.
        crate::agent::react_loop::run(app.clone(), backend).await;
    });

    Ok(())
}

#[tauri::command]
pub async fn ingest_folder(app: AppHandle) -> Result<String, String> {
    use tauri_plugin_dialog::DialogExt;

    let folder = app.dialog().file().blocking_pick_folder();
    let Some(path) = folder else {
        return Err("cancelled".into());
    };
    let path = path.into_path().map_err(|e| e.to_string())?;

    let _ = app.emit(
        "activity:log",
        json!({ "kind": "ingest", "detail": format!("ingesting {}…", path.display()) }),
    );

    let app_cb = app.clone();
    let report = {
        let state = app.state::<AppState>();
        crate::memory::ingest_dir(&state.memory, &path, |s| {
            let _ = app_cb.emit("activity:log", json!({ "kind": "ingest", "detail": s }));
        })
        .await
    };

    let _ = app.emit("memory:update", ());
    Ok(format!(
        "Ingested {} files ({} chunks, {} skipped, {} errors)",
        report.files, report.chunks, report.skipped, report.errors
    ))
}
