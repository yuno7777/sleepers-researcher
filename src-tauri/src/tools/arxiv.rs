//! arXiv fetcher (read-only, logged). Search by query, or fetch by arXiv id.
//! Uses the public arXiv Atom API — no key required.

use super::{activity, Tool};
use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use tauri::AppHandle;

pub struct ArxivTool;

#[async_trait::async_trait]
impl Tool for ArxivTool {
    fn name(&self) -> &'static str {
        "arxiv"
    }
    fn description(&self) -> &'static str {
        "Fetch/summarize arXiv papers by search query or by paper id."
    }
    fn args_hint(&self) -> Value {
        json!({ "query": "search terms (optional)", "id": "arXiv id e.g. 2403.01234 (optional)" })
    }
    fn mutating(&self) -> bool {
        false
    }
    async fn execute(&self, args: &Value, app: &AppHandle) -> Result<String> {
        let url = if let Some(id) = args["id"].as_str().filter(|s| !s.is_empty()) {
            activity(app, "arxiv", format!("id {id}"));
            format!("http://export.arxiv.org/api/query?id_list={id}")
        } else if let Some(q) = args["query"].as_str().filter(|s| !s.is_empty()) {
            activity(app, "arxiv", format!("search {q}"));
            format!(
                "http://export.arxiv.org/api/query?search_query=all:{}&start=0&max_results=5",
                urlencoding(q)
            )
        } else {
            return Err(anyhow!("provide 'query' or 'id'"));
        };

        let client = reqwest::Client::new();
        let xml = client.get(&url).send().await?.text().await?;
        let entries = parse_entries(&xml);
        if entries.is_empty() {
            return Ok("No papers found.".into());
        }
        Ok(entries.join("\n\n"))
    }
}

/// Minimal Atom parsing — extract title / id / summary from each <entry>.
fn parse_entries(xml: &str) -> Vec<String> {
    let mut out = Vec::new();
    for chunk in xml.split("<entry>").skip(1) {
        let entry = chunk.split("</entry>").next().unwrap_or("");
        let title = tag(entry, "title");
        let id = tag(entry, "id");
        let summary = tag(entry, "summary");
        if title.is_empty() {
            continue;
        }
        out.push(format!(
            "Title: {}\nLink: {}\nAbstract: {}",
            collapse(&title),
            id.trim(),
            collapse(&truncate(&summary, 700))
        ));
    }
    out
}

fn tag(s: &str, name: &str) -> String {
    let open = format!("<{name}>");
    let close = format!("</{name}>");
    if let (Some(a), Some(b)) = (s.find(&open), s.find(&close)) {
        s[a + open.len()..b].to_string()
    } else {
        String::new()
    }
}

fn collapse(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn truncate(s: &str, n: usize) -> String {
    if s.len() > n {
        format!("{}…", &s[..n])
    } else {
        s.to_string()
    }
}

fn urlencoding(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            b' ' => "+".to_string(),
            _ => format!("%{:02X}", b),
        })
        .collect()
}
