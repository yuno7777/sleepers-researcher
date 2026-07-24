//! Wikipedia lookup (read-only, logged, no key). Returns intro extracts for the
//! top matching articles — a reliable, never-bot-blocked encyclopedic source.

use super::{activity, Tool};
use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use tauri::AppHandle;

pub struct WikipediaTool;

#[async_trait::async_trait]
impl Tool for WikipediaTool {
    fn name(&self) -> &'static str {
        "wikipedia"
    }
    fn description(&self) -> &'static str {
        "Look up a topic on Wikipedia (intro extracts + links)."
    }
    fn args_hint(&self) -> Value {
        json!({ "query": "topic to look up" })
    }
    fn mutating(&self) -> bool {
        false
    }
    async fn execute(&self, args: &Value, app: &AppHandle) -> Result<String> {
        let query = args["query"].as_str().ok_or_else(|| anyhow!("missing 'query'"))?;
        activity(app, "wikipedia", query.to_string());

        let url = format!(
            "https://en.wikipedia.org/w/api.php?action=query&format=json&prop=extracts\
&exintro=1&explaintext=1&redirects=1&generator=search&gsrsearch={}&gsrlimit=3",
            encode(query)
        );
        let client = reqwest::Client::builder()
            .user_agent("SleepersResearcher/0.1 (research tool)")
            .build()?;
        let v: Value = client.get(&url).send().await?.json().await?;

        let pages = match v["query"]["pages"].as_object() {
            Some(p) => p,
            None => return Ok("No Wikipedia results.".into()),
        };
        let mut items: Vec<(i64, String, String)> = Vec::new();
        for page in pages.values() {
            let title = page["title"].as_str().unwrap_or("").to_string();
            let extract = page["extract"].as_str().unwrap_or("").to_string();
            let index = page["index"].as_i64().unwrap_or(999);
            if !title.is_empty() && !extract.is_empty() {
                items.push((index, title, extract));
            }
        }
        if items.is_empty() {
            return Ok("No Wikipedia results.".into());
        }
        items.sort_by_key(|(i, _, _)| *i);

        let mut out = String::new();
        for (_, title, extract) in items {
            let link = format!("https://en.wikipedia.org/wiki/{}", title.replace(' ', "_"));
            let body: String = extract.chars().take(1200).collect();
            out.push_str(&format!("## {title}\n{link}\n{body}\n\n"));
        }
        Ok(out)
    }
}

fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{:02X}", b),
        })
        .collect()
}
