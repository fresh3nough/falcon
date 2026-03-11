//! Context Collector — gathers deep environmental info (cwd, git status,
//! recent commands, block history with output, selected text, env changes)
//! so the AI agent always has rich, Warp-grade session awareness.

use crate::block::BlockManager;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::process::Command;
use std::sync::Mutex;
use std::time::Instant;

/// Maximum characters of block output to include in a snapshot.
const BLOCK_OUTPUT_MAX: usize = 2000;

/// Maximum number of blocks to include in the full context.
const BLOCK_HISTORY_LIMIT: usize = 50;

/// Maximum number of recent commands to track.
const RECENT_COMMANDS_LIMIT: usize = 50;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Lightweight snapshot of a terminal block for prompt injection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockSnapshot {
    pub command: String,
    pub output: String,
    pub exit_code: Option<i32>,
    pub cwd: String,
}

/// Basic context snapshot (backward-compatible with sidebar).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionContext {
    pub cwd: String,
    pub git_branch: Option<String>,
    pub git_status: Option<String>,
    pub shell: String,
    pub os: String,
    pub recent_commands: Vec<String>,
}

/// Full-depth context snapshot used for agent system prompts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FullSessionContext {
    pub cwd: String,
    pub git_branch: Option<String>,
    pub git_status: Option<String>,
    pub git_diff_stat: Option<String>,
    pub shell: String,
    pub os: String,
    pub recent_commands: Vec<String>,
    pub block_history: Vec<BlockSnapshot>,
    pub selected_text: Option<String>,
    pub env_snapshot: Vec<(String, String)>,
    pub last_exit_code: Option<i32>,
    pub session_duration_secs: u64,
}

// ---------------------------------------------------------------------------
// ContextCollector
// ---------------------------------------------------------------------------

/// Collects and caches session context for AI prompts.
pub struct ContextCollector {
    recent_commands: Mutex<Vec<String>>,
    selected_text: Mutex<Option<String>>,
    last_exit_code: Mutex<Option<i32>>,
    session_start: Instant,
    /// Baseline env vars captured at startup for diff detection.
    baseline_env: HashMap<String, String>,
}

impl ContextCollector {
    pub fn new() -> Self {
        // Capture baseline environment at startup.
        let baseline_env: HashMap<String, String> = std::env::vars().collect();
        Self {
            recent_commands: Mutex::new(Vec::new()),
            selected_text: Mutex::new(None),
            last_exit_code: Mutex::new(None),
            session_start: Instant::now(),
            baseline_env,
        }
    }

    /// Record a command that was just executed.
    pub fn record_command(&self, cmd: &str) {
        let mut cmds = self.recent_commands.lock().unwrap();
        cmds.push(cmd.to_string());
        if cmds.len() > RECENT_COMMANDS_LIMIT {
            cmds.remove(0);
        }
    }

    /// Record the exit code of the most recent command.
    pub fn record_exit_code(&self, code: i32) {
        *self.last_exit_code.lock().unwrap() = Some(code);
    }

    /// Store the currently selected/highlighted text from the terminal.
    pub fn set_selected_text(&self, text: Option<String>) {
        *self.selected_text.lock().unwrap() = text;
    }

    /// Build a basic context snapshot (sidebar / lightweight calls).
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

    /// Build a full-depth context snapshot with block history, env diff,
    /// selected text, and session metadata.
    pub fn collect_full(&self, blocks: &BlockManager) -> FullSessionContext {
        let cwd = std::env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| "unknown".to_string());

        let git_branch = run_cmd("git", &["rev-parse", "--abbrev-ref", "HEAD"]);
        let git_status = run_cmd("git", &["status", "--short"]);
        let git_diff_stat = run_cmd("git", &["diff", "--stat"]);
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "unknown".to_string());
        let os = format!("{} {}", std::env::consts::OS, std::env::consts::ARCH);
        let recent_commands = self.recent_commands.lock().unwrap().clone();

        // Serialize recent blocks into lightweight snapshots.
        let recent_blocks = blocks.get_recent_blocks(BLOCK_HISTORY_LIMIT);
        let block_history: Vec<BlockSnapshot> = recent_blocks
            .into_iter()
            .map(|b| {
                let output = if b.output.len() > BLOCK_OUTPUT_MAX {
                    format!(
                        "{}\n...truncated ({} bytes total)",
                        &b.output[..BLOCK_OUTPUT_MAX],
                        b.output.len()
                    )
                } else {
                    b.output.clone()
                };
                BlockSnapshot {
                    command: b.command,
                    output,
                    exit_code: b.exit_code,
                    cwd: b.cwd,
                }
            })
            .collect();

        // Detect environment variable changes since session start.
        let current_env: HashMap<String, String> = std::env::vars().collect();
        let env_snapshot: Vec<(String, String)> = current_env
            .iter()
            .filter(|(k, v)| self.baseline_env.get(k.as_str()).map(|bv| bv != *v).unwrap_or(true))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();

        let selected_text = self.selected_text.lock().unwrap().clone();
        let last_exit_code = *self.last_exit_code.lock().unwrap();
        let session_duration_secs = self.session_start.elapsed().as_secs();

        FullSessionContext {
            cwd,
            git_branch,
            git_status,
            git_diff_stat,
            shell,
            os,
            recent_commands,
            block_history,
            selected_text,
            env_snapshot,
            last_exit_code,
            session_duration_secs,
        }
    }

    /// Format the basic context as a system-prompt fragment (sidebar chat).
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
                .take(10)
                .cloned()
                .collect::<Vec<_>>()
                .join("\n  ");
            prompt.push_str(&format!("\n- Recent commands:\n  {last}"));
        }
        prompt
    }

    /// Build a full-depth system-prompt fragment for the agent, including
    /// block history, selected text, env changes, and git diff stats.
    pub fn as_full_system_prompt(&self, blocks: &BlockManager) -> String {
        let ctx = self.collect_full(blocks);
        let mut prompt = String::with_capacity(8192);

        // -- Environment section --
        prompt.push_str("[ENVIRONMENT]\n");
        prompt.push_str(&format!("CWD: {}\n", ctx.cwd));
        prompt.push_str(&format!("Shell: {}\n", ctx.shell));
        prompt.push_str(&format!("OS: {}\n", ctx.os));
        prompt.push_str(&format!("Session uptime: {}s\n", ctx.session_duration_secs));
        if let Some(code) = ctx.last_exit_code {
            prompt.push_str(&format!("Last exit code: {code}\n"));
        }

        // -- Git section --
        if ctx.git_branch.is_some() || ctx.git_status.is_some() {
            prompt.push_str("\n[GIT]\n");
            if let Some(branch) = &ctx.git_branch {
                prompt.push_str(&format!("Branch: {branch}\n"));
            }
            if let Some(status) = &ctx.git_status {
                if !status.is_empty() {
                    prompt.push_str(&format!("Status:\n{status}\n"));
                }
            }
            if let Some(diff) = &ctx.git_diff_stat {
                if !diff.is_empty() {
                    prompt.push_str(&format!("Diff stat:\n{diff}\n"));
                }
            }
        }

        // -- Environment variable changes --
        if !ctx.env_snapshot.is_empty() {
            prompt.push_str("\n[ENV CHANGES]\n");
            for (k, v) in &ctx.env_snapshot {
                // Skip noisy vars.
                if k.starts_with('_') || k == "SHLVL" || k == "OLDPWD" {
                    continue;
                }
                let display_val = if v.len() > 200 {
                    format!("{}...", &v[..200])
                } else {
                    v.clone()
                };
                prompt.push_str(&format!("{k}={display_val}\n"));
            }
        }

        // -- Recent commands --
        if !ctx.recent_commands.is_empty() {
            prompt.push_str("\n[RECENT COMMANDS]\n");
            for cmd in ctx.recent_commands.iter().rev().take(20) {
                prompt.push_str(&format!("$ {cmd}\n"));
            }
        }

        // -- Block history (full output context) --
        if !ctx.block_history.is_empty() {
            prompt.push_str("\n[BLOCK HISTORY]\n");
            for (i, block) in ctx.block_history.iter().enumerate() {
                let code_str = block
                    .exit_code
                    .map(|c| format!(" [exit {c}]"))
                    .unwrap_or_default();
                prompt.push_str(&format!(
                    "--- Block {} (cwd: {}){}\n$ {}\n{}\n",
                    i + 1,
                    block.cwd,
                    code_str,
                    block.command,
                    block.output
                ));
            }
        }

        // -- Selected text --
        if let Some(text) = &ctx.selected_text {
            if !text.is_empty() {
                prompt.push_str("\n[SELECTED TEXT]\n");
                prompt.push_str(text);
                prompt.push('\n');
            }
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
