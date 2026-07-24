//! Fetch a URL and extract its readable text (read-only, logged). Lets the agent
//! read full page content — not just search snippets — for deep research.

use super::{activity, Tool};
use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use tauri::AppHandle;

const MAX_TEXT: usize = 14_000;

pub struct WebFetchTool;

#[async_trait::async_trait]
impl Tool for WebFetchTool {
    fn name(&self) -> &'static str {
        "web_fetch"
    }
    fn description(&self) -> &'static str {
        "Fetch a web page (or raw URL) and return its readable text content."
    }
    fn args_hint(&self) -> Value {
        json!({ "url": "http(s) URL to fetch" })
    }
    fn mutating(&self) -> bool {
        false
    }
    async fn execute(&self, args: &Value, app: &AppHandle) -> Result<String> {
        let url = args["url"].as_str().ok_or_else(|| anyhow!("missing 'url'"))?;
        if !url.starts_with("http") {
            return Err(anyhow!("url must start with http(s)"));
        }
        activity(app, "fetch", url.to_string());

        let client = reqwest::Client::builder()
            .user_agent("Mozilla/5.0 (SleepersResearcher)")
            .build()?;
        let resp = client.get(url).send().await?;
        let status = resp.status();
        let ctype = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        let body = resp.text().await?;
        if !status.is_success() {
            return Err(anyhow!("fetch {} returned {}", url, status));
        }

        let text = if ctype.contains("html") || body.trim_start().starts_with('<') {
            html_to_text(&body)
        } else {
            body
        };
        let text = text.trim();
        if text.len() > MAX_TEXT {
            Ok(format!("{}\n\n[truncated at {MAX_TEXT} chars]", &text[..MAX_TEXT]))
        } else {
            Ok(text.to_string())
        }
    }
}

/// Very small HTML → text: drop script/style blocks, strip tags, decode common
/// entities, collapse whitespace. Good enough to read article content.
fn html_to_text(html: &str) -> String {
    let without_blocks = strip_block(&strip_block(html, "script"), "style");
    let mut out = String::with_capacity(without_blocks.len());
    let mut in_tag = false;
    for c in without_blocks.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    let decoded = out
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&rsquo;", "'")
        .replace("&mdash;", "—");
    decoded.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Remove `<tag ...>...</tag>` blocks (case-insensitive) entirely.
fn strip_block(s: &str, tag: &str) -> String {
    // ASCII-only lowercasing preserves byte length, so indices stay aligned with `s`.
    let lower = s.to_ascii_lowercase();
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let mut result = String::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        if let Some(rel) = lower[i..].find(&open) {
            let start = i + rel;
            result.push_str(&s[i..start]);
            if let Some(erel) = lower[start..].find(&close) {
                i = start + erel + close.len();
            } else {
                break;
            }
        } else {
            result.push_str(&s[i..]);
            break;
        }
    }
    result
}
