//! deep_research: the Perplexity-style engine. Decomposes a question into
//! sub-queries, searches each, fetches the top unique sources in parallel, and
//! returns numbered evidence for the agent to synthesize with [n] citations.
//! Emits `research:sources` so the UI can show a sources list.

use super::{activity, Tool};
use crate::backends::ChatMessage;
use crate::research::{self, SearchResult};
use crate::state::AppState;
use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::collections::HashSet;
use tauri::{AppHandle, Emitter, Manager};

const MAX_SUBQUERIES: usize = 5;
const MAX_SOURCES: usize = 8;
const PER_SOURCE_CHARS: usize = 2600;

pub struct DeepResearchTool;

#[async_trait::async_trait]
impl Tool for DeepResearchTool {
    fn name(&self) -> &'static str {
        "deep_research"
    }
    fn description(&self) -> &'static str {
        "Deep multi-source web research: plans sub-queries, searches, reads several \
pages in parallel, and returns numbered evidence to cite as [n]."
    }
    fn args_hint(&self) -> Value {
        json!({ "query": "the research question" })
    }
    fn mutating(&self) -> bool {
        false
    }
    async fn execute(&self, args: &Value, app: &AppHandle) -> Result<String> {
        let query = args["query"].as_str().ok_or_else(|| anyhow!("missing 'query'"))?;
        let state = app.state::<AppState>();
        let backend_name = state.backend.lock().unwrap().clone();

        // 1. Plan sub-queries with the current backend.
        activity(app, "research", format!("planning: {query}"));
        let sub_queries = plan_subqueries(&state, &backend_name, query).await;
        activity(app, "research", format!("{} sub-queries", sub_queries.len()));

        // 2. Search each, collecting unique sources.
        let mut seen = HashSet::new();
        let mut sources: Vec<SearchResult> = Vec::new();
        for sq in sub_queries.iter().take(MAX_SUBQUERIES) {
            activity(app, "search", format!("{} ({})", sq, research::provider_name()));
            if let Ok(results) = research::search(sq, 4).await {
                for r in results {
                    if !r.url.is_empty() && seen.insert(r.url.clone()) {
                        sources.push(r);
                    }
                }
            }
            if sources.len() >= MAX_SOURCES {
                break;
            }
        }
        sources.truncate(MAX_SOURCES);
        if sources.is_empty() {
            return Ok("deep_research found no sources (search may be unavailable).".into());
        }

        // 3. Fetch the pages in parallel.
        activity(app, "fetch", format!("{} sources in parallel", sources.len()));
        let urls: Vec<String> = sources.iter().map(|s| s.url.clone()).collect();
        let fetched = research::fetch_many(urls, PER_SOURCE_CHARS).await;

        // 4. Build numbered evidence + emit the source list for the UI.
        let mut evidence = String::new();
        let mut cite_list = Vec::new();
        for (i, ((url, res), src)) in fetched.iter().zip(sources.iter()).enumerate() {
            let n = i + 1;
            let body = match res {
                Ok(t) if !t.trim().is_empty() => t.clone(),
                _ => format!("(page unavailable) {}", src.snippet),
            };
            evidence.push_str(&format!("[{n}] {} — {}\n{}\n\n", src.title, url, body));
            cite_list.push(json!({ "n": n, "title": src.title, "url": url }));
        }
        let _ = app.emit("research:sources", json!({ "query": query, "sources": cite_list }));

        Ok(format!(
            "Evidence gathered from {} sources. Write a thorough, well-structured answer to \
\"{}\" and cite sources inline as [n] matching the numbers below. End with a \"Sources\" list.\n\n{}",
            sources.len(),
            query,
            evidence
        ))
    }
}

async fn plan_subqueries(state: &AppState, backend_name: &str, query: &str) -> Vec<String> {
    let prompt = vec![
        ChatMessage::system(
            "You are a research planner. Given a question, output ONLY a JSON array of 3-5 \
specific web-search queries that together thoroughly answer it. No prose, no markdown.",
        ),
        ChatMessage::user(format!("Question: {query}")),
    ];
    if let Ok(resp) = state.router.backend(backend_name).complete(&prompt).await {
        if let Some(list) = parse_string_array(&resp.content) {
            if !list.is_empty() {
                return list;
            }
        }
    }
    vec![query.to_string()]
}

fn parse_string_array(s: &str) -> Option<Vec<String>> {
    let start = s.find('[')?;
    let end = s.rfind(']')?;
    if end <= start {
        return None;
    }
    serde_json::from_str::<Vec<String>>(&s[start..=end]).ok()
}
