//! Context Collector — gathers environmental info (cwd, git status, recent
//! commands) so the Grok sidebar always has rich session awareness.

use serde::{Deserialize, Serialize};
use std::process::Command;
use std::sync::Mutex;

/// Snapshot of the current terminal context sent to Grok.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionContext {
    pub cwd: String,
    pub git_branch: Option<String>,
    pub git_status: Option<String>,
    pub shell: String,
    pub os: String,
    pub recent_commands: Vec<String>,
}

/// Collects and caches session context for Grok AI prompts.
pub struct ContextCollector {
    recent_commands: Mutex<Vec<String>>,
}

impl ContextCollector {
    pub fn new() -> Self {
        Self {
            recent_commands: Mutex::new(Vec::new()),
        }
    }

    /// Record a command that was just executed.
    pub fn record_command(&self, cmd: &str) {
        let mut cmds = self.recent_commands.lock().unwrap();
        cmds.push(cmd.to_string());
        // Keep at most the last 20 commands.
        if cmds.len() > 20 {
            cmds.remove(0);
        }
    }

    /// Build a full context snapshot.
    pub fn collect(&self) -> SessionContext {
        let cwd = std::env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| "unknown".to_string());

        let git_branch = run_cmd("git", &["rev-parse", "--abbrev-ref", "HEAD"]);
        let git_status = run_cmd("git", &["status", "--short"]);

        let shell = std::env::var("SHELL").unwrap_or_else(|_| "unknown".to_string());

        let os = format!("{} {}", std::env::consts::OS, std::env::consts::ARCH);

        let recent_commands = self.recent_commands.lock().unwrap().clone();

        SessionContext {
            cwd,
            git_branch,
            git_status,
            shell,
            os,
            recent_commands,
        }
    }

    /// Format the context as a system-prompt fragment for Grok.
    pub fn as_system_prompt(&self) -> String {
        let ctx = self.collect();
        let mut prompt = format!(
            "Terminal context:\n- CWD: {}\n- Shell: {}\n- OS: {}",
            ctx.cwd, ctx.shell, ctx.os
        );
        if let Some(branch) = &ctx.git_branch {
            prompt.push_str(&format!("\n- Git branch: {branch}"));
        }
        if let Some(status) = &ctx.git_status {
            if !status.is_empty() {
                prompt.push_str(&format!("\n- Git status:\n{status}"));
            }
        }
        if !ctx.recent_commands.is_empty() {
            let last = ctx
                .recent_commands
                .iter()
                .rev()
                .take(5)
                .cloned()
                .collect::<Vec<_>>()
                .join("\n  ");
            prompt.push_str(&format!("\n- Recent commands:\n  {last}"));
        }
        prompt
    }
}

/// Run a command and return stdout trimmed, or None on failure.
fn run_cmd(program: &str, args: &[&str]) -> Option<String> {
    Command::new(program)
        .args(args)
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
            } else {
                None
            }
        })
}
