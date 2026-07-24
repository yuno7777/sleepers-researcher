//! Fetch a URL and extract its readable text (read-only, logged). Lets the agent
//! read full page content — not just search snippets — for deep research.

use super::{activity, Tool};
use crate::research;
use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use tauri::AppHandle;

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
        activity(app, "fetch", url.to_string());
        research::fetch_readable(url, 14_000).await
    }
}
