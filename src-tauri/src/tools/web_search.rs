//! Web search (read-only, logged). Uses Tavily if TAVILY_API_KEY is set,
//! otherwise a keyless DuckDuckGo fallback — so search works out of the box.

use super::{activity, Tool};
use crate::research;
use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use tauri::AppHandle;

pub struct WebSearchTool;

#[async_trait::async_trait]
impl Tool for WebSearchTool {
    fn name(&self) -> &'static str {
        "web_search"
    }
    fn description(&self) -> &'static str {
        "Search the web for current information (Tavily, or keyless DuckDuckGo)."
    }
    fn args_hint(&self) -> Value {
        json!({ "query": "search query" })
    }
    fn mutating(&self) -> bool {
        false
    }
    async fn execute(&self, args: &Value, app: &AppHandle) -> Result<String> {
        let query = args["query"].as_str().ok_or_else(|| anyhow!("missing 'query'"))?;
        activity(app, "search", format!("{} ({})", query, research::provider_name()));

        let results = research::search(query, 8).await?;
        if results.is_empty() {
            return Ok("No results.".into());
        }
        let mut out = String::new();
        for (i, r) in results.iter().enumerate() {
            out.push_str(&format!("[{}] {}\n{}\n{}\n\n", i + 1, r.title, r.url, r.snippet));
        }
        Ok(out)
    }
}
