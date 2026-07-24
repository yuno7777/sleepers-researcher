//! Shell command execution on the host PC. This is powerful, so every command
//! is shown in the confirmation modal and must be approved before it runs
//! (unless code_exec/YOLO auto-approve is on). Timeout + output cap apply.

use super::code_exec::{cap, run_with_timeout};
use super::{activity, request_permission, Tool};
use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use tauri::AppHandle;
use tokio::process::Command;

pub struct ShellTool;

#[async_trait::async_trait]
impl Tool for ShellTool {
    fn name(&self) -> &'static str {
        "shell"
    }
    fn description(&self) -> &'static str {
        "Run a shell command on the PC (scripts, tooling, file ops). Requires confirmation."
    }
    fn args_hint(&self) -> Value {
        json!({ "command": "the command line to execute" })
    }
    fn mutating(&self) -> bool {
        true
    }
    async fn execute(&self, args: &Value, app: &AppHandle) -> Result<String> {
        let command = args["command"].as_str().ok_or_else(|| anyhow!("missing 'command'"))?;

        let approved = request_permission(
            app,
            "code_exec",
            "Run shell command",
            "host shell",
            command,
        )
        .await;
        if !approved {
            return Ok("DENIED: user declined to run the command.".into());
        }
        activity(app, "shell", command.to_string());

        // Windows: run through cmd.exe. (A POSIX build would use `sh -c`.)
        let mut cmd = if cfg!(windows) {
            let mut c = Command::new("cmd");
            c.arg("/C").arg(command);
            c
        } else {
            let mut c = Command::new("sh");
            c.arg("-c").arg(command);
            c
        };
        let out = run_with_timeout(&mut cmd).await?;
        Ok(cap(&out))
    }
}
