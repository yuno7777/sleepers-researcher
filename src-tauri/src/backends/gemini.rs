//! Gemini backend — direct REST calls to the Google Generative Language API.
//! Auth: `x-goog-api-key` header. Streaming via `:streamGenerateContent?alt=sse`.

use super::{ChatMessage, LlmBackend, LlmResponse};
use anyhow::{anyhow, Result};
use futures_util::StreamExt;
use serde_json::{json, Value};
use tokio::sync::mpsc::UnboundedSender;

const BASE: &str = "https://generativelanguage.googleapis.com/v1beta/models";

pub struct GeminiBackend {
    client: reqwest::Client,
    api_key: String,
    model: String,
}

impl GeminiBackend {
    pub fn from_env() -> Self {
        Self {
            client: reqwest::Client::new(),
            api_key: std::env::var("GEMINI_API_KEY").unwrap_or_default(),
            model: std::env::var("GEMINI_MODEL")
                .unwrap_or_else(|_| "gemini-3.1-flash-lite-preview".into()),
        }
    }

    pub fn has_key(&self) -> bool {
        !self.api_key.is_empty()
    }

    /// Convert our messages into Gemini's `contents` + `systemInstruction` shape.
    fn build_body(&self, messages: &[ChatMessage]) -> Value {
        let mut contents = Vec::new();
        let mut system_parts = Vec::new();
        for m in messages {
            match m.role.as_str() {
                "system" => system_parts.push(json!({ "text": m.content })),
                "assistant" => contents.push(json!({
                    "role": "model",
                    "parts": [{ "text": m.content }]
                })),
                // user + tool results are fed back as user turns
                _ => contents.push(json!({
                    "role": "user",
                    "parts": [{ "text": m.content }]
                })),
            }
        }
        let mut body = json!({
            "contents": contents,
            "generationConfig": { "maxOutputTokens": 4096, "temperature": 0.7 }
        });
        if !system_parts.is_empty() {
            body["systemInstruction"] = json!({ "parts": system_parts });
        }
        body
    }
}

#[async_trait::async_trait]
impl LlmBackend for GeminiBackend {
    fn name(&self) -> &'static str {
        "gemini"
    }
    fn model(&self) -> &str {
        &self.model
    }

    async fn complete(&self, messages: &[ChatMessage]) -> Result<LlmResponse> {
        if !self.has_key() {
            return Err(anyhow!("GEMINI_API_KEY not set"));
        }
        let url = format!("{BASE}/{}:generateContent", self.model);
        let resp = self
            .client
            .post(&url)
            .header("x-goog-api-key", &self.api_key)
            .json(&self.build_body(messages))
            .send()
            .await?;
        let status = resp.status();
        let v: Value = resp.json().await?;
        if !status.is_success() {
            return Err(anyhow!("Gemini error {}: {}", status, v));
        }
        let content = v["candidates"][0]["content"]["parts"]
            .as_array()
            .map(|parts| {
                parts
                    .iter()
                    .filter_map(|p| p["text"].as_str())
                    .collect::<String>()
            })
            .unwrap_or_default();
        Ok(LlmResponse { content })
    }

    async fn complete_stream(
        &self,
        messages: &[ChatMessage],
        tx: UnboundedSender<String>,
    ) -> Result<LlmResponse> {
        if !self.has_key() {
            return Err(anyhow!("GEMINI_API_KEY not set"));
        }
        let url = format!("{BASE}/{}:streamGenerateContent?alt=sse", self.model);
        let resp = self
            .client
            .post(&url)
            .header("x-goog-api-key", &self.api_key)
            .json(&self.build_body(messages))
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(anyhow!("Gemini stream error {}: {}", status, text));
        }

        let mut full = String::new();
        let mut buf = String::new();
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            buf.push_str(&String::from_utf8_lossy(&chunk));
            // SSE lines are newline-delimited; process complete lines.
            while let Some(pos) = buf.find('\n') {
                let line = buf[..pos].trim().to_string();
                buf.drain(..=pos);
                if let Some(data) = line.strip_prefix("data:") {
                    let data = data.trim();
                    if data.is_empty() {
                        continue;
                    }
                    if let Ok(v) = serde_json::from_str::<Value>(data) {
                        if let Some(parts) = v["candidates"][0]["content"]["parts"].as_array() {
                            for p in parts {
                                if let Some(t) = p["text"].as_str() {
                                    full.push_str(t);
                                    let _ = tx.send(t.to_string());
                                }
                            }
                        }
                    }
                }
            }
        }
        Ok(LlmResponse { content: full })
    }
}
