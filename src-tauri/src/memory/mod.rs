//! Memory layer: long-term RAG vector store + persistent chat history + the
//! "soul" (durable facts about the user). Embeddings come from Gemini Embedding 2
//! (`gemini-embedding-2`), which is natively multimodal — text and images map
//! into the same 3072-dim space, so a text query can match an ingested image.

pub mod vector_store;

use anyhow::{anyhow, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use vector_store::Db;

/// App data directory (`%APPDATA%\com.sleepers.researcher`) — holds the DB and
/// the installed app's `.env`.
pub fn data_dir() -> PathBuf {
    let base = std::env::var("APPDATA").unwrap_or_else(|_| ".".into());
    let dir = PathBuf::from(base).join("com.sleepers.researcher");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

enum Modality {
    Text(String),
    Image { mime: String, b64: String },
}

pub struct MemoryStore {
    db: Mutex<Db>,
    client: reqwest::Client,
    api_key: String,
    model: String,
}

impl MemoryStore {
    pub fn new() -> Self {
        let path = data_dir().join("sleepers.db");
        let db = Db::open(&path)
            .or_else(|_| Db::open_in_memory())
            .expect("failed to open memory database");
        Self {
            db: Mutex::new(db),
            client: reqwest::Client::new(),
            api_key: std::env::var("GEMINI_API_KEY").unwrap_or_default(),
            model: std::env::var("EMBEDDING_MODEL").unwrap_or_else(|_| "gemini-embedding-2".into()),
        }
    }

    pub fn has_key(&self) -> bool {
        !self.api_key.is_empty()
    }

    async fn embed(&self, input: &Modality) -> Result<Vec<f32>> {
        if self.api_key.is_empty() {
            return Err(anyhow!("GEMINI_API_KEY not set (needed for embeddings)"));
        }
        let part = match input {
            Modality::Text(t) => json!({ "text": t }),
            Modality::Image { mime, b64 } => json!({ "inlineData": { "mimeType": mime, "data": b64 } }),
        };
        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:embedContent",
            self.model
        );
        let resp = self
            .client
            .post(&url)
            .header("x-goog-api-key", &self.api_key)
            .json(&json!({ "content": { "parts": [part] } }))
            .send()
            .await?;
        let status = resp.status();
        let v: Value = resp.json().await?;
        if !status.is_success() {
            return Err(anyhow!("embed error {}: {}", status, v));
        }
        let vals = v["embedding"]["values"]
            .as_array()
            .ok_or_else(|| anyhow!("no embedding in response"))?;
        Ok(vals.iter().filter_map(|x| x.as_f64().map(|f| f as f32)).collect())
    }

    pub async fn embed_text(&self, t: &str) -> Result<Vec<f32>> {
        self.embed(&Modality::Text(t.to_string())).await
    }

    /// Store a text memory (soul fact, document chunk, conversation).
    pub async fn remember(&self, kind: &str, content: &str, source: &str) -> Result<()> {
        let emb = self.embed_text(content).await?;
        self.db.lock().unwrap().add_memory(kind, content, source, &emb)?;
        Ok(())
    }

    /// Store an image memory (embedded directly into the multimodal space).
    pub async fn add_image(&self, mime: &str, b64: &str, label: &str, source: &str) -> Result<()> {
        let emb = self.embed(&Modality::Image { mime: mime.into(), b64: b64.into() }).await?;
        self.db.lock().unwrap().add_memory("image", label, source, &emb)?;
        Ok(())
    }

    /// Retrieve top-k relevant memories for a text query.
    pub async fn retrieve(&self, query: &str, k: usize) -> Vec<(String, String, String, f32)> {
        match self.embed_text(query).await {
            Ok(q) => self.db.lock().unwrap().search(&q, k),
            Err(_) => Vec::new(),
        }
    }

    // ---- chat persistence passthroughs ----
    pub fn new_session(&self, title: &str) -> i64 {
        self.db.lock().unwrap().new_session(title).unwrap_or(0)
    }
    pub fn latest_or_new_session(&self) -> i64 {
        let db = self.db.lock().unwrap();
        db.latest_session()
            .unwrap_or_else(|| db.new_session("Session").unwrap_or(0))
    }
    pub fn add_message(&self, session: i64, role: &str, content: &str) {
        let _ = self.db.lock().unwrap().add_message(session, role, content);
    }
    pub fn list_sessions(&self, limit: usize) -> Vec<(i64, i64, String)> {
        self.db.lock().unwrap().list_sessions(limit)
    }
    pub fn session_messages(&self, session: i64) -> Vec<(String, String)> {
        self.db.lock().unwrap().session_messages(session).unwrap_or_default()
    }
    pub fn soul_facts(&self) -> Vec<String> {
        self.db.lock().unwrap().soul_facts()
    }
    pub fn count(&self) -> usize {
        self.db.lock().unwrap().count()
    }
}

// ----------------------------- ingestion -----------------------------

/// Classify a file by extension. Videos and unknown binaries are skipped.
enum FileKind {
    Text,
    Image(&'static str), // mime
    Pdf,
    Skip,
}

fn classify(path: &Path) -> FileKind {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    match ext.as_str() {
        "md" | "markdown" | "txt" | "rs" | "py" | "js" | "ts" | "tsx" | "jsx" | "json" | "csv"
        | "html" | "htm" | "css" | "toml" | "yaml" | "yml" | "xml" | "c" | "cpp" | "h" | "hpp"
        | "java" | "go" | "rb" | "sh" | "sql" | "log" | "tex" | "ini" | "cfg" => FileKind::Text,
        "png" => FileKind::Image("image/png"),
        "jpg" | "jpeg" => FileKind::Image("image/jpeg"),
        "webp" => FileKind::Image("image/webp"),
        "gif" => FileKind::Image("image/gif"),
        "bmp" => FileKind::Image("image/bmp"),
        "pdf" => FileKind::Pdf,
        _ => FileKind::Skip,
    }
}

fn collect_files(dir: &Path, out: &mut Vec<PathBuf>, cap: usize) {
    if out.len() >= cap {
        return;
    }
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                // skip noisy/huge dirs
                let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if matches!(name, "node_modules" | "target" | ".git" | "dist" | ".venv") {
                    continue;
                }
                collect_files(&p, out, cap);
            } else if p.is_file() {
                out.push(p);
            }
            if out.len() >= cap {
                return;
            }
        }
    }
}

fn chunk_text(text: &str, size: usize) -> Vec<String> {
    let words: Vec<&str> = text.split_whitespace().collect();
    let mut chunks = Vec::new();
    let mut cur = String::new();
    for w in words {
        if cur.len() + w.len() + 1 > size && !cur.is_empty() {
            chunks.push(std::mem::take(&mut cur));
        }
        cur.push_str(w);
        cur.push(' ');
    }
    if !cur.trim().is_empty() {
        chunks.push(cur);
    }
    chunks
}

fn extract_pdf(path: &Path) -> Option<String> {
    let p = path.to_path_buf();
    // pdf-extract can panic on malformed files — isolate it.
    std::panic::catch_unwind(move || pdf_extract::extract_text(&p).ok())
        .ok()
        .flatten()
}

pub struct IngestReport {
    pub files: usize,
    pub chunks: usize,
    pub skipped: usize,
    pub errors: usize,
}

/// Ingest a directory tree into the vector store. `progress` is called with a
/// human-readable status per file (used to drive the activity log).
pub async fn ingest_dir<F: Fn(&str)>(
    store: &MemoryStore,
    root: &Path,
    progress: F,
) -> IngestReport {
    let mut files = Vec::new();
    collect_files(root, &mut files, 1000);

    let mut report = IngestReport { files: 0, chunks: 0, skipped: 0, errors: 0 };
    for path in files {
        let name = path.display().to_string();
        match classify(&path) {
            FileKind::Text => {
                if let Ok(text) = std::fs::read_to_string(&path) {
                    let chunks = chunk_text(&text, 1500);
                    let mut ok = 0;
                    for c in &chunks {
                        match store.remember("document", c, &name).await {
                            Ok(_) => ok += 1,
                            Err(_) => report.errors += 1,
                        }
                    }
                    if ok > 0 {
                        report.files += 1;
                        report.chunks += ok;
                        progress(&format!("text: {name} (+{ok})"));
                    }
                } else {
                    report.errors += 1;
                }
            }
            FileKind::Pdf => match extract_pdf(&path) {
                Some(text) if !text.trim().is_empty() => {
                    let chunks = chunk_text(&text, 1500);
                    let mut ok = 0;
                    for c in &chunks {
                        if store.remember("document", c, &name).await.is_ok() {
                            ok += 1;
                        } else {
                            report.errors += 1;
                        }
                    }
                    if ok > 0 {
                        report.files += 1;
                        report.chunks += ok;
                        progress(&format!("pdf: {name} (+{ok})"));
                    }
                }
                _ => {
                    report.errors += 1;
                    progress(&format!("pdf failed: {name}"));
                }
            },
            FileKind::Image(mime) => match std::fs::read(&path) {
                Ok(bytes) => {
                    let b64 = STANDARD.encode(&bytes);
                    let label = format!("[image] {name}");
                    match store.add_image(mime, &b64, &label, &name).await {
                        Ok(_) => {
                            report.files += 1;
                            report.chunks += 1;
                            progress(&format!("image: {name}"));
                        }
                        Err(_) => report.errors += 1,
                    }
                }
                Err(_) => report.errors += 1,
            },
            FileKind::Skip => report.skipped += 1,
        }
    }
    report
}
