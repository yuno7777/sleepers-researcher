//! Code execution sandbox. Runs small Python or Rust snippets in a subprocess
//! with a wall-clock timeout and output cap. No arbitrary shell access — only
//! the two interpreters/compilers, and only after explicit confirmation.

use super::{activity, request_permission, Tool};
use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::process::Stdio;
use std::time::Duration;
use tauri::AppHandle;
use tokio::process::Command;

const TIMEOUT_SECS: u64 = 20;
const MAX_OUTPUT: usize = 8_000;

pub struct CodeExecTool;

#[async_trait::async_trait]
impl Tool for CodeExecTool {
    fn name(&self) -> &'static str {
        "code_exec"
    }
    fn description(&self) -> &'static str {
        "Execute a small Python or Rust snippet and return its output. Requires confirmation."
    }
    fn args_hint(&self) -> Value {
        json!({ "language": "python | rust", "code": "source code to run" })
    }
    fn mutating(&self) -> bool {
        true
    }
    async fn execute(&self, args: &Value, app: &AppHandle) -> Result<String> {
        let lang = args["language"].as_str().unwrap_or("python").to_lowercase();
        let code = args["code"].as_str().ok_or_else(|| anyhow!("missing 'code'"))?;

        let preview = format!("language: {lang}\ntimeout: {TIMEOUT_SECS}s\n\n{code}");
        let approved =
            request_permission(app, "code_exec", "Run code", &format!("{lang} snippet"), &preview)
                .await;
        if !approved {
            return Ok("DENIED: user declined to run the code.".into());
        }

        activity(app, "exec", format!("{lang} snippet ({} chars)", code.len()));

        let dir = std::env::temp_dir().join(format!("sr_exec_{}", std::process::id()));
        std::fs::create_dir_all(&dir).ok();

        let output = match lang.as_str() {
            "python" => {
                let file = dir.join("snippet.py");
                std::fs::write(&file, code)?;
                run_with_timeout(Command::new("python").arg(&file)).await
            }
            "rust" => {
                let src = dir.join("snippet.rs");
                let exe = dir.join(if cfg!(windows) { "snippet.exe" } else { "snippet" });
                std::fs::write(&src, code)?;
                let compile = run_with_timeout(
                    Command::new("rustc").arg("-O").arg(&src).arg("-o").arg(&exe),
                )
                .await?;
                if !exe.exists() {
                    return Ok(format!("Compilation failed:\n{compile}"));
                }
                run_with_timeout(&mut Command::new(&exe)).await
            }
            other => return Err(anyhow!("unsupported language: {other} (use python or rust)")),
        }?;

        let _ = std::fs::remove_dir_all(&dir);
        Ok(cap(&output))
    }
}

pub(crate) async fn run_with_timeout(cmd: &mut Command) -> Result<String> {
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let child = cmd.spawn().map_err(|e| anyhow!("spawn failed: {e}"))?;
    match tokio::time::timeout(Duration::from_secs(TIMEOUT_SECS), child.wait_with_output()).await {
        Ok(Ok(out)) => {
            let mut s = String::new();
            s.push_str(&String::from_utf8_lossy(&out.stdout));
            let err = String::from_utf8_lossy(&out.stderr);
            if !err.trim().is_empty() {
                s.push_str("\n[stderr]\n");
                s.push_str(&err);
            }
            if s.trim().is_empty() {
                s = "(no output)".into();
            }
            Ok(s)
        }
        Ok(Err(e)) => Err(anyhow!("process error: {e}")),
        Err(_) => Err(anyhow!("timed out after {TIMEOUT_SECS}s")),
    }
}

pub(crate) fn cap(s: &str) -> String {
    if s.len() > MAX_OUTPUT {
        format!("{}\n\n[output truncated at {MAX_OUTPUT} chars]", &s[..MAX_OUTPUT])
    } else {
        s.to_string()
    }
}
