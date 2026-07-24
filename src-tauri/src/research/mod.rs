//! Shared research infrastructure: a web-search provider with a keyless
//! fallback (Tavily if configured, otherwise DuckDuckGo HTML) and parallel
//! page fetching with readable-text extraction. Used by the web_search /
//! web_fetch tools and the deep_research orchestrator.

use anyhow::{anyhow, Result};
use futures_util::future::join_all;
use serde::Serialize;
use serde_json::{json, Value};

#[derive(Clone, Serialize)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) SleepersResearcher/0.1")
        .build()
        .unwrap_or_default()
}

/// Which search backend is active (for logging / UI).
pub fn provider_name() -> &'static str {
    match std::env::var("TAVILY_API_KEY") {
        Ok(k) if !k.is_empty() => "tavily",
        _ => "duckduckgo",
    }
}

/// Search the web. Prefers Tavily (better ranking) if a key is set, otherwise
/// falls back to keyless DuckDuckGo HTML — so search works out of the box.
pub async fn search(query: &str, max: usize) -> Result<Vec<SearchResult>> {
    if let Ok(key) = std::env::var("TAVILY_API_KEY") {
        if !key.is_empty() {
            return tavily(query, max, &key).await;
        }
    }
    duckduckgo(query, max).await
}

async fn tavily(query: &str, max: usize, key: &str) -> Result<Vec<SearchResult>> {
    let resp = client()
        .post("https://api.tavily.com/search")
        .json(&json!({
            "api_key": key, "query": query,
            "max_results": max, "search_depth": "advanced", "include_answer": false
        }))
        .send()
        .await?;
    let status = resp.status();
    let v: Value = resp.json().await?;
    if !status.is_success() {
        return Err(anyhow!("Tavily error {}: {}", status, v));
    }
    let out = v["results"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|r| SearchResult {
                    title: r["title"].as_str().unwrap_or("").to_string(),
                    url: r["url"].as_str().unwrap_or("").to_string(),
                    snippet: r["content"].as_str().unwrap_or("").chars().take(400).collect(),
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(out)
}

async fn duckduckgo(query: &str, max: usize) -> Result<Vec<SearchResult>> {
    let url = format!("https://html.duckduckgo.com/html/?q={}", urlencode(query));
    let html = client().get(&url).send().await?.text().await?;
    Ok(parse_ddg(&html, max))
}

fn parse_ddg(html: &str, max: usize) -> Vec<SearchResult> {
    let mut out = Vec::new();
    for seg in html.split("result__a").skip(1) {
        let href = between(seg, "href=\"", "\"");
        let url = decode_ddg_href(&href);
        if url.is_empty() {
            continue;
        }
        let title = collapse(&strip_tags(&between(seg, ">", "</a>")));
        let snippet = match seg.find("result__snippet") {
            Some(p) => collapse(&strip_tags(&between(&seg[p..], ">", "</a>"))),
            None => String::new(),
        };
        out.push(SearchResult { title, url, snippet });
        if out.len() >= max {
            break;
        }
    }
    out
}

/// DuckDuckGo wraps results as `//duckduckgo.com/l/?uddg=<encoded>&...`.
fn decode_ddg_href(href: &str) -> String {
    let h = html_unescape(href);
    if let Some(idx) = h.find("uddg=") {
        let rest = &h[idx + 5..];
        let enc = rest.split('&').next().unwrap_or("");
        return percent_decode(enc);
    }
    if h.starts_with("//") {
        format!("https:{h}")
    } else if h.starts_with("http") {
        h
    } else {
        String::new()
    }
}

/// Fetch a URL and return readable text (drops scripts/styles/tags).
pub async fn fetch_readable(url: &str, max_chars: usize) -> Result<String> {
    if !url.starts_with("http") {
        return Err(anyhow!("url must start with http(s)"));
    }
    let resp = client().get(url).send().await?;
    let status = resp.status();
    let ctype = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let body = resp.text().await?;
    if !status.is_success() {
        return Err(anyhow!("{} returned {}", url, status));
    }
    let text = if ctype.contains("html") || body.trim_start().starts_with('<') {
        html_to_text(&body)
    } else {
        body
    };
    let text = text.trim();
    Ok(if text.len() > max_chars {
        format!("{}…[truncated]", &text[..max_chars])
    } else {
        text.to_string()
    })
}

/// Fetch many URLs concurrently. Returns (url, Ok(text) | Err(msg)) in order.
pub async fn fetch_many(urls: Vec<String>, max_chars: usize) -> Vec<(String, Result<String, String>)> {
    let futs = urls.into_iter().map(|u| async move {
        let r = fetch_readable(&u, max_chars).await.map_err(|e| e.to_string());
        (u, r)
    });
    join_all(futs).await
}

// ----------------------------- helpers -----------------------------

pub fn html_to_text(html: &str) -> String {
    let cleaned = strip_block(&strip_block(html, "script"), "style");
    let mut out = String::with_capacity(cleaned.len());
    let mut in_tag = false;
    for c in cleaned.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    collapse(&html_unescape(&out))
}

fn strip_block(s: &str, tag: &str) -> String {
    let lower = s.to_ascii_lowercase();
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let mut result = String::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        if let Some(rel) = lower[i..].find(&open) {
            let start = i + rel;
            result.push_str(&s[i..start]);
            match lower[start..].find(&close) {
                Some(erel) => i = start + erel + close.len(),
                None => break,
            }
        } else {
            result.push_str(&s[i..]);
            break;
        }
    }
    result
}

fn strip_tags(s: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out
}

fn between(s: &str, a: &str, b: &str) -> String {
    if let Some(i) = s.find(a) {
        let start = i + a.len();
        if let Some(j) = s[start..].find(b) {
            return s[start..start + j].to_string();
        }
    }
    String::new()
}

fn collapse(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn html_unescape(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
        .replace("&nbsp;", " ")
}

fn urlencode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            b' ' => "+".to_string(),
            _ => format!("%{:02X}", b),
        })
        .collect()
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                    out.push(v);
                    i += 3;
                } else {
                    out.push(bytes[i]);
                    i += 1;
                }
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).to_string()
}
