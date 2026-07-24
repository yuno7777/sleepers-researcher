//! LLM backend abstraction. Gemini and Groq implement a common `LlmBackend`
//! trait so they are interchangeable behind a config-driven `Router`. A third
//! backend (e.g. a local Ollama-served model) can be added by implementing the
//! same trait and registering it in `Router`.

pub mod gemini;
pub mod groq;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::UnboundedSender;

use gemini::GeminiBackend;
use groq::GroqBackend;

/// A single message in a conversation. Roles: "system" | "user" | "assistant" | "tool".
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

impl ChatMessage {
    pub fn system(s: impl Into<String>) -> Self {
        Self { role: "system".into(), content: s.into() }
    }
    pub fn user(s: impl Into<String>) -> Self {
        Self { role: "user".into(), content: s.into() }
    }
    pub fn assistant(s: impl Into<String>) -> Self {
        Self { role: "assistant".into(), content: s.into() }
    }
}

#[derive(Clone, Debug, Default)]
pub struct LlmResponse {
    pub content: String,
}

/// Common interface every LLM provider implements.
#[async_trait::async_trait]
pub trait LlmBackend: Send + Sync {
    fn name(&self) -> &'static str;
    fn model(&self) -> &str;

    /// One-shot, non-streaming completion. Used for fast routing/tool decisions.
    async fn complete(&self, messages: &[ChatMessage]) -> Result<LlmResponse>;

    /// Streaming completion. Emits token deltas over `tx` as they arrive and
    /// returns the fully accumulated text.
    async fn complete_stream(
        &self,
        messages: &[ChatMessage],
        tx: UnboundedSender<String>,
    ) -> Result<LlmResponse>;
}

/// Config-driven switch between backends. Default backend comes from
/// `DEFAULT_BACKEND` but the caller can override per request.
pub struct Router {
    gemini: GeminiBackend,
    groq: GroqBackend,
    default: String,
}

impl Router {
    pub fn from_env() -> Self {
        Self {
            gemini: GeminiBackend::from_env(),
            groq: GroqBackend::from_env(),
            default: std::env::var("DEFAULT_BACKEND").unwrap_or_else(|_| "gemini".into()),
        }
    }

    /// Resolve a backend by name, falling back to the configured default.
    pub fn backend(&self, name: &str) -> &dyn LlmBackend {
        match name.to_lowercase().as_str() {
            "groq" => &self.groq,
            "gemini" => &self.gemini,
            _ => self.backend(&self.default.clone()),
        }
    }

    /// Whether the given backend has an API key configured.
    pub fn is_configured(&self, name: &str) -> bool {
        match name.to_lowercase().as_str() {
            "groq" => self.groq.has_key(),
            _ => self.gemini.has_key(),
        }
    }
}
