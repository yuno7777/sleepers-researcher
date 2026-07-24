//! Local embedded vector store + chat persistence, backed by SQLite (rusqlite,
//! bundled — no external service, no Docker). Embeddings are stored as f32 BLOBs
//! and searched by brute-force cosine similarity in Rust. For a local research
//! tool with thousands of chunks this is instant and dependency-free, which is
//! far more reliable to ship on Windows than a native vector extension or a
//! Docker-hosted Qdrant.

use anyhow::Result;
use rusqlite::{params, Connection};

pub struct Db {
    conn: Connection,
}

impl Db {
    pub fn open(path: &std::path::Path) -> Result<Self> {
        let db = Self { conn: Connection::open(path)? };
        db.migrate()?;
        Ok(db)
    }

    pub fn open_in_memory() -> Result<Self> {
        let db = Self { conn: Connection::open_in_memory()? };
        db.migrate()?;
        Ok(db)
    }

    fn migrate(&self) -> Result<()> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS sessions(
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                title TEXT,
                created_ts INTEGER
             );
             CREATE TABLE IF NOT EXISTS messages(
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id INTEGER,
                role TEXT,
                content TEXT,
                ts INTEGER
             );
             CREATE TABLE IF NOT EXISTS memories(
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                kind TEXT,        -- 'soul' | 'document' | 'image' | 'conversation'
                content TEXT,
                source TEXT,
                embedding BLOB,
                ts INTEGER
             );",
        )?;
        Ok(())
    }

    // ---- sessions / messages (persistent chat history) ----

    pub fn new_session(&self, title: &str) -> Result<i64> {
        self.conn
            .execute("INSERT INTO sessions(title, created_ts) VALUES(?1, ?2)", params![title, now()])?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn latest_session(&self) -> Option<i64> {
        self.conn
            .query_row("SELECT id FROM sessions ORDER BY id DESC LIMIT 1", [], |r| r.get(0))
            .ok()
    }

    /// Recent sessions with a preview taken from their first user message.
    pub fn list_sessions(&self, limit: usize) -> Vec<(i64, i64, String)> {
        let mut out = Vec::new();
        if let Ok(mut stmt) = self.conn.prepare(
            "SELECT s.id, s.created_ts,
                COALESCE((SELECT m.content FROM messages m
                          WHERE m.session_id = s.id AND m.role = 'user'
                          ORDER BY m.id ASC LIMIT 1), '')
             FROM sessions s ORDER BY s.id DESC LIMIT ?1",
        ) {
            if let Ok(rows) = stmt.query_map([limit as i64], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?))
            }) {
                out = rows.filter_map(|x| x.ok()).collect();
            }
        }
        out
    }

    pub fn add_message(&self, session: i64, role: &str, content: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO messages(session_id, role, content, ts) VALUES(?1,?2,?3,?4)",
            params![session, role, content, now()],
        )?;
        Ok(())
    }

    pub fn session_messages(&self, session: i64) -> Result<Vec<(String, String)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT role, content FROM messages WHERE session_id=?1 ORDER BY id ASC")?;
        let rows = stmt.query_map([session], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?;
        Ok(rows.filter_map(|x| x.ok()).collect())
    }

    // ---- vector memories ----

    pub fn add_memory(&self, kind: &str, content: &str, source: &str, emb: &[f32]) -> Result<()> {
        self.conn.execute(
            "INSERT INTO memories(kind, content, source, embedding, ts) VALUES(?1,?2,?3,?4,?5)",
            params![kind, content, source, f32_to_bytes(emb), now()],
        )?;
        Ok(())
    }

    pub fn soul_facts(&self) -> Vec<String> {
        let mut out = Vec::new();
        if let Ok(mut stmt) = self
            .conn
            .prepare("SELECT content FROM memories WHERE kind='soul' ORDER BY id DESC LIMIT 100")
        {
            if let Ok(rows) = stmt.query_map([], |r| r.get::<_, String>(0)) {
                out = rows.filter_map(|x| x.ok()).collect();
            }
        }
        out
    }

    pub fn count(&self) -> usize {
        self.conn
            .query_row("SELECT COUNT(*) FROM memories", [], |r| r.get::<_, i64>(0))
            .unwrap_or(0) as usize
    }

    /// Brute-force top-k by cosine similarity. Returns (kind, content, source, score).
    pub fn search(&self, query: &[f32], k: usize) -> Vec<(String, String, String, f32)> {
        let mut results = Vec::new();
        if let Ok(mut stmt) = self
            .conn
            .prepare("SELECT kind, content, source, embedding FROM memories")
        {
            if let Ok(rows) = stmt.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Vec<u8>>(3)?,
                ))
            }) {
                for (kind, content, source, blob) in rows.flatten() {
                    let score = cosine(query, &bytes_to_f32(&blob));
                    results.push((kind, content, source, score));
                }
            }
        }
        results.sort_by(|a, b| b.3.partial_cmp(&a.3).unwrap_or(std::cmp::Ordering::Equal));
        results.truncate(k);
        results
    }
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

fn f32_to_bytes(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

fn bytes_to_f32(b: &[u8]) -> Vec<f32> {
    b.chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return -1.0;
    }
    let (mut dot, mut na, mut nb) = (0.0f32, 0.0f32, 0.0f32);
    for i in 0..a.len() {
        dot += a[i] * b[i];
        na += a[i] * a[i];
        nb += b[i] * b[i];
    }
    if na == 0.0 || nb == 0.0 {
        -1.0
    } else {
        dot / (na.sqrt() * nb.sqrt())
    }
}
