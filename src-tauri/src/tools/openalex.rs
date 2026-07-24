//! OpenAlex scholarly search (read-only, logged, no key). Searches 240M+ works
//! and returns titles, authors, year, citation count, venue, and a link — plus
//! an open-access PDF URL when one exists. Great for academic deep research.

use super::{activity, Tool};
use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use tauri::AppHandle;

pub struct OpenAlexTool;

#[async_trait::async_trait]
impl Tool for OpenAlexTool {
    fn name(&self) -> &'static str {
        "openalex"
    }
    fn description(&self) -> &'static str {
        "Search scholarly papers (OpenAlex): titles, authors, year, citations, open-access PDFs."
    }
    fn args_hint(&self) -> Value {
        json!({ "query": "topic or paper search" })
    }
    fn mutating(&self) -> bool {
        false
    }
    async fn execute(&self, args: &Value, app: &AppHandle) -> Result<String> {
        let query = args["query"].as_str().ok_or_else(|| anyhow!("missing 'query'"))?;
        activity(app, "openalex", query.to_string());

        // `mailto` is the polite-pool convention; no key required.
        let url = format!(
            "https://api.openalex.org/works?search={}&per_page=6&sort=relevance_score:desc\
&mailto=research@sleepers.local",
            encode(query)
        );
        let client = reqwest::Client::builder()
            .user_agent("SleepersResearcher/0.1 (mailto:research@sleepers.local)")
            .build()?;
        let v: Value = client.get(&url).send().await?.json().await?;

        let works = match v["results"].as_array() {
            Some(w) if !w.is_empty() => w,
            _ => return Ok("No scholarly results.".into()),
        };

        let mut out = String::new();
        for w in works {
            let title = w["title"].as_str().unwrap_or("(untitled)");
            let year = w["publication_year"].as_i64().unwrap_or(0);
            let cites = w["cited_by_count"].as_i64().unwrap_or(0);
            let venue = w["primary_location"]["source"]["display_name"]
                .as_str()
                .unwrap_or("");
            let doi = w["doi"].as_str().unwrap_or("");
            let landing = w["primary_location"]["landing_page_url"].as_str().unwrap_or("");
            let oa_pdf = w["open_access"]["oa_url"].as_str().unwrap_or("");
            let authors: Vec<&str> = w["authorships"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|au| au["author"]["display_name"].as_str())
                        .take(3)
                        .collect()
                })
                .unwrap_or_default();

            let link = if !doi.is_empty() { doi } else { landing };
            out.push_str(&format!(
                "- {title} ({year}) — {} | {cites} citations{}\n  {link}{}\n",
                authors.join(", "),
                if venue.is_empty() { String::new() } else { format!(" | {venue}") },
                if oa_pdf.is_empty() { String::new() } else { format!("\n  open-access PDF: {oa_pdf}") },
            ));
        }
        Ok(out)
    }
}

fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            b' ' => "%20".to_string(),
            _ => format!("%{:02X}", b),
        })
        .collect()
}
