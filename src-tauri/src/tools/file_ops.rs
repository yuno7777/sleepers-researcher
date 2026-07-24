//! File read (read-only, logged) and file write (mutating, confirmed with a
//! before/after preview).

use super::{activity, request_permission, Tool};
use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use tauri::AppHandle;

const MAX_READ: usize = 20_000;

pub struct ReadFileTool;

#[async_trait::async_trait]
impl Tool for ReadFileTool {
    fn name(&self) -> &'static str {
        "read_file"
    }
    fn description(&self) -> &'static str {
        "Read a local text/code file (read-only)."
    }
    fn args_hint(&self) -> Value {
        json!({ "path": "absolute or relative file path" })
    }
    fn mutating(&self) -> bool {
        false
    }
    async fn execute(&self, args: &Value, app: &AppHandle) -> Result<String> {
        let path = args["path"].as_str().ok_or_else(|| anyhow!("missing 'path'"))?;
        activity(app, "read", path.to_string());
        let content = std::fs::read_to_string(path)
            .map_err(|e| anyhow!("could not read {path}: {e}"))?;
        if content.len() > MAX_READ {
            Ok(format!(
                "{}\n\n[truncated at {MAX_READ} chars of {}]",
                &content[..MAX_READ],
                content.len()
            ))
        } else {
            Ok(content)
        }
    }
}

pub struct WriteFileTool;

#[async_trait::async_trait]
impl Tool for WriteFileTool {
    fn name(&self) -> &'static str {
        "write_file"
    }
    fn description(&self) -> &'static str {
        "Write/overwrite a local file. Requires user confirmation."
    }
    fn args_hint(&self) -> Value {
        json!({ "path": "file path", "content": "full file contents to write" })
    }
    fn mutating(&self) -> bool {
        true
    }
    async fn execute(&self, args: &Value, app: &AppHandle) -> Result<String> {
        let path = args["path"].as_str().ok_or_else(|| anyhow!("missing 'path'"))?;
        let content = args["content"].as_str().ok_or_else(|| anyhow!("missing 'content'"))?;

        let existing = std::fs::read_to_string(path).ok();
        let preview = match &existing {
            Some(old) => format!(
                "OVERWRITE {path}\n\n--- current ({} bytes) ---\n{}\n\n+++ new ({} bytes) +++\n{}",
                old.len(),
                truncate(old, 1500),
                content.len(),
                truncate(content, 3000),
            ),
            None => format!(
                "CREATE {path}\n\n+++ new ({} bytes) +++\n{}",
                content.len(),
                truncate(content, 3000)
            ),
        };

        let approved = request_permission(
            app,
            "file_write",
            "Write file",
            path,
            &preview,
        )
        .await;

        if !approved {
            return Ok(format!("DENIED: user declined to write {path}."));
        }

        if let Some(parent) = std::path::Path::new(path).parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        std::fs::write(path, content).map_err(|e| anyhow!("write failed: {e}"))?;
        activity(app, "write", format!("{} ({} bytes)", path, content.len()));
        Ok(format!("Wrote {} bytes to {path}.", content.len()))
    }
}

fn truncate(s: &str, n: usize) -> String {
    if s.len() > n {
        format!("{}…[+{} chars]", &s[..n], s.len() - n)
    } else {
        s.to_string()
    }
}
