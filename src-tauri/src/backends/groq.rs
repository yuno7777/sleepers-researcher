//! Groq backend — OpenAI-compatible REST endpoint. Auth: `Authorization: Bearer`.
//! Fast model, good for quick drafts and tool-routing decisions.

use super::{ChatMessage, LlmBackend, LlmResponse};
use anyhow::{anyhow, Result};
use futures_util::StreamExt;
use serde_json::{json, Value};
use tokio::sync::mpsc::UnboundedSender;

const URL: &str = "https://api.groq.com/openai/v1/chat/completions";

pub struct GroqBackend {
    client: reqwest::Client,
    api_key: String,
    model: String,
}

impl GroqBackend {
    pub fn from_env() -> Self {
        Self {
            client: reqwest::Client::new(),
            api_key: std::env::var("GROQ_API_KEY").unwrap_or_default(),
            model: std::env::var("GROQ_MODEL")
                .unwrap_or_else(|_| "llama-3.3-70b-versatile".into()),
        }
    }

    pub fn has_key(&self) -> bool {
        !self.api_key.is_empty()
    }

    fn messages_json(&self, messages: &[ChatMessage]) -> Vec<Value> {
        messages
            .iter()
            .map(|m| {
                let role = match m.role.as_str() {
                    "system" | "assistant" | "tool" => m.role.as_str(),
                    _ => "user",
                };
                json!({ "role": role, "content": m.content })
            })
            .collect()
    }
}

#[async_trait::async_trait]
impl LlmBackend for GroqBackend {
    fn name(&self) -> &'static str {
        "groq"
    }
    fn model(&self) -> &str {
        &self.model
    }

    async fn complete(&self, messages: &[ChatMessage]) -> Result<LlmResponse> {
        if !self.has_key() {
            return Err(anyhow!("GROQ_API_KEY not set"));
        }
        let body = json!({
            "model": self.model,
            "messages": self.messages_json(messages),
            "temperature": 0.7,
        });
        let resp = self
            .client
            .post(URL)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await?;
        let status = resp.status();
        let v: Value = resp.json().await?;
        if !status.is_success() {
            return Err(anyhow!("Groq error {}: {}", status, v));
        }
        let content = v["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        Ok(LlmResponse { content })
    }

    async fn complete_stream(
        &self,
        messages: &[ChatMessage],
        tx: UnboundedSender<String>,
    ) -> Result<LlmResponse> {
        if !self.has_key() {
            return Err(anyhow!("GROQ_API_KEY not set"));
        }
        let body = json!({
            "model": self.model,
            "messages": self.messages_json(messages),
            "temperature": 0.7,
            "stream": true,
        });
        let resp = self
            .client
            .post(URL)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(anyhow!("Groq stream error {}: {}", status, text));
        }

        let mut full = String::new();
        let mut buf = String::new();
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            buf.push_str(&String::from_utf8_lossy(&chunk));
            while let Some(pos) = buf.find('\n') {
                let line = buf[..pos].trim().to_string();
                buf.drain(..=pos);
                if let Some(data) = line.strip_prefix("data:") {
                    let data = data.trim();
                    if data.is_empty() || data == "[DONE]" {
                        continue;
                    }
                    if let Ok(v) = serde_json::from_str::<Value>(data) {
                        if let Some(t) = v["choices"][0]["delta"]["content"].as_str() {
                            full.push_str(t);
                            let _ = tx.send(t.to_string());
                        }
                    }
                }
            }
        }
        Ok(LlmResponse { content: full })
    }
}
