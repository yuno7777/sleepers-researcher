//! Web search via the Tavily API (read-only, logged). Requires TAVILY_API_KEY.

use super::{activity, Tool};
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
        "Search the web for current information."
    }
    fn args_hint(&self) -> Value {
        json!({ "query": "search query" })
    }
    fn mutating(&self) -> bool {
        false
    }
    async fn execute(&self, args: &Value, app: &AppHandle) -> Result<String> {
        let query = args["query"].as_str().ok_or_else(|| anyhow!("missing 'query'"))?;
        let key = std::env::var("TAVILY_API_KEY").unwrap_or_default();
        if key.is_empty() {
            return Ok("web_search unavailable: TAVILY_API_KEY is not set in .env.".into());
        }
        activity(app, "search", query.to_string());

        let client = reqwest::Client::new();
        let resp = client
            .post("https://api.tavily.com/search")
            .json(&json!({
                "api_key": key,
                "query": query,
                "max_results": 8,
                "search_depth": "advanced",
                "include_answer": true,
                "include_raw_content": false
            }))
            .send()
            .await?;
        let status = resp.status();
        let v: Value = resp.json().await?;
        if !status.is_success() {
            return Err(anyhow!("Tavily error {}: {}", status, v));
        }

        let mut out = String::new();
        if let Some(ans) = v["answer"].as_str() {
            if !ans.is_empty() {
                out.push_str(&format!("Summary: {ans}\n\n"));
            }
        }
        if let Some(results) = v["results"].as_array() {
            for (i, r) in results.iter().enumerate() {
                out.push_str(&format!(
                    "[{}] {}\n{}\n{}\n\n",
                    i + 1,
                    r["title"].as_str().unwrap_or(""),
                    r["url"].as_str().unwrap_or(""),
                    truncate(r["content"].as_str().unwrap_or(""), 500)
                ));
            }
        }
        if out.is_empty() {
            out = "No results.".into();
        }
        Ok(out)
    }
}

fn truncate(s: &str, n: usize) -> String {
    if s.len() > n {
        format!("{}…", &s[..n])
    } else {
        s.to_string()
    }
}
