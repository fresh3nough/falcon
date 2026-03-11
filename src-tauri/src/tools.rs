//! Tool Registry & Execution Engine — defines every tool the agent can invoke,
//! classifies them by safety level, and routes execution.
//!
//! ## Safety levels
//! - **ReadOnly**: auto-executed without user approval.
//! - **Write**: requires user approval (can modify files/state).
//! - **Destructive**: requires approval + shows a warning badge.

use crate::grok::{FunctionDef, ToolDef};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::process::Command;
use std::time::Instant;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum output length before smart truncation kicks in.
const OUTPUT_MAX: usize = 20_000;

/// How much of the head/tail to keep when truncating.
const TRUNC_HEAD: usize = 8_000;
const TRUNC_TAIL: usize = 4_000;

/// Shell patterns considered destructive.
const DESTRUCTIVE_PATTERNS: &[&str] = &[
    "rm ", "rmdir", "sudo ", "su ", "mkfs", "dd if=", "chmod ", "chown ",
    "> /dev/", "docker system prune", "docker rm", "docker rmi",
    "kill ", "pkill", "killall", "shutdown", "reboot", "halt",
    "fdisk", "DROP ", "DELETE FROM", "truncate", "format ",
];

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Safety classification for a tool invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToolSafety {
    /// Auto-execute without asking the user.
    ReadOnly,
    /// Requires user approval before execution.
    Write,
    /// Requires approval + shows a destructive-action warning.
    Destructive,
}

/// Structured result returned by every tool execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub output: String,
    pub exit_code: Option<i32>,
    pub truncated: bool,
    pub duration_ms: u64,
}

// ---------------------------------------------------------------------------
// Tool definitions
// ---------------------------------------------------------------------------

/// Build the complete set of tools the agent can invoke.
pub fn build_tools() -> Vec<ToolDef> {
    vec![
        // -- Shell --
        tool("run_shell_command",
            "Run a shell command in the user's terminal. Returns stdout, stderr, and exit code.",
            json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string", "description": "The shell command to execute" }
                },
                "required": ["command"]
            }),
        ),
        tool("run_script",
            "Execute a multi-line script with the given interpreter (bash, python3, node, etc.). \
             Returns combined output and exit code.",
            json!({
                "type": "object",
                "properties": {
                    "interpreter": {
                        "type": "string",
                        "description": "The interpreter to use (e.g. bash, python3, node)"
                    },
                    "script": {
                        "type": "string",
                        "description": "The full script content to execute"
                    }
                },
                "required": ["interpreter", "script"]
            }),
        ),

        // -- File read --
        tool("read_file",
            "Read the contents of a file. Large files are automatically truncated.",
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path to the file to read" }
                },
                "required": ["path"]
            }),
        ),

        // -- File write --
        tool("write_file",
            "Create or overwrite a file with the given content. Parent directories are created \
             automatically.",
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path to the file to write" },
                    "content": { "type": "string", "description": "Full file content to write" }
                },
                "required": ["path", "content"]
            }),
        ),
        tool("edit_file",
            "Perform a surgical find-and-replace in a file. The search string must match exactly \
             (including whitespace). Only the first occurrence is replaced.",
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path to the file to edit" },
                    "search": { "type": "string", "description": "Exact text to find" },
                    "replace": { "type": "string", "description": "Replacement text" }
                },
                "required": ["path", "search", "replace"]
            }),
        ),

        // -- File search --
        tool("search_files",
            "Recursively grep for a pattern in files under a directory. Returns matching lines \
             with file paths and line numbers.",
            json!({
                "type": "object",
                "properties": {
                    "pattern": { "type": "string", "description": "Regex pattern to search for" },
                    "path": {
                        "type": "string",
                        "description": "Directory to search in (defaults to cwd)"
                    },
                    "file_glob": {
                        "type": "string",
                        "description": "Optional file glob filter, e.g. '*.rs' or '*.py'"
                    }
                },
                "required": ["pattern"]
            }),
        ),
        tool("find_files",
            "Find files by name pattern (glob) under a directory.",
            json!({
                "type": "object",
                "properties": {
                    "glob": {
                        "type": "string",
                        "description": "Glob pattern, e.g. '*.rs', 'Cargo.*', '**/*.test.ts'"
                    },
                    "path": {
                        "type": "string",
                        "description": "Directory to search in (defaults to cwd)"
                    }
                },
                "required": ["glob"]
            }),
        ),

        // -- Directory --
        tool("list_directory",
            "List files and directories at a given path.",
            json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Directory path to list (defaults to cwd)"
                    }
                },
                "required": []
            }),
        ),
        tool("get_working_directory",
            "Get the current working directory.",
            json!({ "type": "object", "properties": {}, "required": [] }),
        ),

        // -- Git --
        tool("get_git_status",
            "Get git branch, working-tree status, and recent log entries.",
            json!({ "type": "object", "properties": {}, "required": [] }),
        ),

        // -- System introspection --
        tool("get_environment",
            "Get relevant environment variables (PATH, HOME, EDITOR, LANG, and any custom ones).",
            json!({ "type": "object", "properties": {}, "required": [] }),
        ),
        tool("get_process_list",
            "List running processes, optionally filtered by a keyword.",
            json!({
                "type": "object",
                "properties": {
                    "filter": {
                        "type": "string",
                        "description": "Optional keyword to filter processes"
                    }
                },
                "required": []
            }),
        ),
        tool("get_system_info",
            "Get system information: OS, kernel, architecture, memory, disk usage.",
            json!({ "type": "object", "properties": {}, "required": [] }),
        ),

        // -- Git workflow --
        tool("git_commit",
            "Stage files and create a git commit with the given message.",
            json!({
                "type": "object",
                "properties": {
                    "message": { "type": "string", "description": "Commit message" },
                    "files": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Files to stage (defaults to all changed files if empty)"
                    }
                },
                "required": ["message"]
            }),
        ),
        tool("git_diff",
            "Show git diff. Use staged=true for staged changes, or provide a specific path.",
            json!({
                "type": "object",
                "properties": {
                    "staged": { "type": "boolean", "description": "Show staged diff instead of unstaged" },
                    "path": { "type": "string", "description": "Optional file path to diff" }
                },
                "required": []
            }),
        ),
        tool("git_branch",
            "List branches, create a new branch, or switch to a branch.",
            json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["list", "create", "switch"],
                        "description": "Action to perform"
                    },
                    "name": { "type": "string", "description": "Branch name (for create/switch)" }
                },
                "required": ["action"]
            }),
        ),
        tool("git_push",
            "Push commits to the remote repository.",
            json!({
                "type": "object",
                "properties": {
                    "remote": { "type": "string", "description": "Remote name (defaults to origin)" },
                    "branch": { "type": "string", "description": "Branch to push (defaults to current)" }
                },
                "required": []
            }),
        ),
        tool("git_pull",
            "Pull latest changes from the remote repository.",
            json!({
                "type": "object",
                "properties": {
                    "remote": { "type": "string", "description": "Remote name (defaults to origin)" },
                    "branch": { "type": "string", "description": "Branch to pull (defaults to current)" }
                },
                "required": []
            }),
        ),
        tool("git_log",
            "Show recent git commit history.",
            json!({
                "type": "object",
                "properties": {
                    "count": { "type": "integer", "description": "Number of commits to show (default 10)" },
                    "oneline": { "type": "boolean", "description": "Use one-line format (default true)" }
                },
                "required": []
            }),
        ),

        // -- MCP/Goose bridge --
        tool("call_mcp_tool",
            "Invoke a Goose/MCP tool by name. Requires the `goose` CLI to be installed. \
             Returns the tool's output.",
            json!({
                "type": "object",
                "properties": {
                    "tool_name": { "type": "string", "description": "Name of the MCP tool to invoke" },
                    "arguments": {
                        "type": "object",
                        "description": "Arguments to pass to the MCP tool"
                    }
                },
                "required": ["tool_name"]
            }),
        ),

        // -- Agent control --
        tool("final_answer",
            "Call this when the task is complete with a brief summary of what was accomplished.",
            json!({
                "type": "object",
                "properties": {
                    "summary": {
                        "type": "string",
                        "description": "Brief summary of what was done"
                    }
                },
                "required": ["summary"]
            }),
        ),
    ]
}

// ---------------------------------------------------------------------------
// Safety classification
// ---------------------------------------------------------------------------

/// Classify a tool call by safety level.
pub fn classify_safety(tool_name: &str, args: &serde_json::Value) -> ToolSafety {
    match tool_name {
        // Always safe — read-only.
        "read_file" | "list_directory" | "get_working_directory" | "get_git_status"
        | "search_files" | "find_files" | "get_environment" | "get_process_list"
        | "get_system_info" | "final_answer"
        | "git_diff" | "git_log" => ToolSafety::ReadOnly,

        // Write tools — need approval.
        "write_file" | "edit_file"
        | "git_commit" | "git_branch" | "git_push" | "git_pull"
        | "call_mcp_tool" => ToolSafety::Write,

        // Shell commands: check for destructive patterns.
        "run_shell_command" => {
            let cmd = args
                .get("command")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if is_destructive(cmd) {
                ToolSafety::Destructive
            } else {
                ToolSafety::Write
            }
        }

        // Scripts: check interpreter + content.
        "run_script" => {
            let script = args
                .get("script")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if is_destructive(script) {
                ToolSafety::Destructive
            } else {
                ToolSafety::Write
            }
        }

        _ => ToolSafety::Write,
    }
}

/// Returns `true` if the text matches a known destructive pattern.
pub fn is_destructive(text: &str) -> bool {
    let lower = text.to_lowercase();
    DESTRUCTIVE_PATTERNS
        .iter()
        .any(|p| lower.contains(&p.to_lowercase()))
}

// ---------------------------------------------------------------------------
// Tool execution
// ---------------------------------------------------------------------------

/// Execute a tool by name with the given arguments.
///
/// This handles all tools *except* those needing special flow (approval
/// gating is handled by the agent loop).
pub fn execute_tool(name: &str, args: &serde_json::Value) -> ToolResult {
    let start = Instant::now();
    let (output, exit_code) = match name {
        "read_file" => exec_read_file(args),
        "write_file" => exec_write_file(args),
        "edit_file" => exec_edit_file(args),
        "list_directory" => exec_list_directory(args),
        "get_working_directory" => exec_get_cwd(),
        "get_git_status" => exec_git_status(),
        "search_files" => exec_search_files(args),
        "find_files" => exec_find_files(args),
        "get_environment" => exec_get_environment(),
        "get_process_list" => exec_get_process_list(args),
        "get_system_info" => exec_get_system_info(),
        "run_shell_command" => exec_shell_command(args),
        "run_script" => exec_run_script(args),
        "git_commit" => exec_git_commit(args),
        "git_diff" => exec_git_diff(args),
        "git_branch" => exec_git_branch(args),
        "git_push" => exec_git_push(args),
        "git_pull" => exec_git_pull(args),
        "git_log" => exec_git_log(args),
        "call_mcp_tool" => exec_mcp_tool(args),
        _ => (format!("Unknown tool: {name}"), None),
    };
    let duration_ms = start.elapsed().as_millis() as u64;
    let (output, truncated) = smart_truncate(output);
    ToolResult {
        output,
        exit_code,
        truncated,
        duration_ms,
    }
}

// ---------------------------------------------------------------------------
// Individual tool handlers
// ---------------------------------------------------------------------------

fn exec_read_file(args: &serde_json::Value) -> (String, Option<i32>) {
    let path = args.get("path").and_then(|v| v.as_str()).unwrap_or("");
    match std::fs::read_to_string(path) {
        Ok(content) => (content, Some(0)),
        Err(e) => (format!("Error reading file: {e}"), Some(1)),
    }
}

fn exec_write_file(args: &serde_json::Value) -> (String, Option<i32>) {
    let path = args.get("path").and_then(|v| v.as_str()).unwrap_or("");
    let content = args.get("content").and_then(|v| v.as_str()).unwrap_or("");

    // Create parent directories if needed.
    if let Some(parent) = std::path::Path::new(path).parent() {
        if !parent.exists() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                return (format!("Error creating directories: {e}"), Some(1));
            }
        }
    }

    match std::fs::write(path, content) {
        Ok(()) => (format!("Wrote {} bytes to {path}", content.len()), Some(0)),
        Err(e) => (format!("Error writing file: {e}"), Some(1)),
    }
}

fn exec_edit_file(args: &serde_json::Value) -> (String, Option<i32>) {
    let path = args.get("path").and_then(|v| v.as_str()).unwrap_or("");
    let search = args.get("search").and_then(|v| v.as_str()).unwrap_or("");
    let replace = args.get("replace").and_then(|v| v.as_str()).unwrap_or("");

    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => return (format!("Error reading file: {e}"), Some(1)),
    };

    if !content.contains(search) {
        return (
            format!("Search string not found in {path}. No changes made."),
            Some(1),
        );
    }

    // Replace only the first occurrence.
    let new_content = content.replacen(search, replace, 1);
    match std::fs::write(path, &new_content) {
        Ok(()) => {
            let lines_removed = search.lines().count();
            let lines_added = replace.lines().count();
            (
                format!(
                    "Edited {path}: -{lines_removed} lines, +{lines_added} lines"
                ),
                Some(0),
            )
        }
        Err(e) => (format!("Error writing file: {e}"), Some(1)),
    }
}

fn exec_list_directory(args: &serde_json::Value) -> (String, Option<i32>) {
    let path = args.get("path").and_then(|v| v.as_str()).unwrap_or(".");
    match std::fs::read_dir(path) {
        Ok(entries) => {
            let mut items: Vec<String> = entries
                .filter_map(|e| e.ok())
                .map(|e| {
                    let name = e.file_name().to_string_lossy().to_string();
                    if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                        format!("{name}/")
                    } else {
                        name
                    }
                })
                .collect();
            items.sort();
            (items.join("\n"), Some(0))
        }
        Err(e) => (format!("Error listing directory: {e}"), Some(1)),
    }
}

fn exec_get_cwd() -> (String, Option<i32>) {
    let cwd = std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "unknown".to_string());
    (cwd, Some(0))
}

fn exec_git_status() -> (String, Option<i32>) {
    let branch = run_cmd("git", &["rev-parse", "--abbrev-ref", "HEAD"])
        .unwrap_or_else(|| "not a git repo".to_string());
    let status = run_cmd("git", &["status", "--short"])
        .unwrap_or_else(|| "N/A".to_string());
    let log = run_cmd("git", &["log", "--oneline", "-5"])
        .unwrap_or_default();

    let mut out = format!("Branch: {branch}\nStatus:\n{status}");
    if !log.is_empty() {
        out.push_str(&format!("\n\nRecent commits:\n{log}"));
    }
    (out, Some(0))
}

fn exec_search_files(args: &serde_json::Value) -> (String, Option<i32>) {
    let pattern = args.get("pattern").and_then(|v| v.as_str()).unwrap_or("");
    let path = args.get("path").and_then(|v| v.as_str()).unwrap_or(".");
    let file_glob = args.get("file_glob").and_then(|v| v.as_str());

    let mut cmd_args = vec![
        "-rn".to_string(),
        "--color=never".to_string(),
        "--max-count=100".to_string(),
    ];
    if let Some(glob) = file_glob {
        cmd_args.push(format!("--include={glob}"));
    }
    cmd_args.push(pattern.to_string());
    cmd_args.push(path.to_string());

    let args_str: Vec<&str> = cmd_args.iter().map(|s| s.as_str()).collect();
    match run_cmd_with_status("grep", &args_str) {
        (Some(output), code) => (output, Some(code)),
        (None, _) => ("No matches found.".to_string(), Some(1)),
    }
}

fn exec_find_files(args: &serde_json::Value) -> (String, Option<i32>) {
    let glob = args.get("glob").and_then(|v| v.as_str()).unwrap_or("*");
    let path = args.get("path").and_then(|v| v.as_str()).unwrap_or(".");

    match run_cmd_with_status("find", &[path, "-type", "f", "-name", glob, "-maxdepth", "8"]) {
        (Some(output), code) => (output, Some(code)),
        (None, _) => ("No files found.".to_string(), Some(1)),
    }
}

fn exec_get_environment() -> (String, Option<i32>) {
    let important_keys = [
        "PATH", "HOME", "USER", "SHELL", "EDITOR", "VISUAL", "LANG", "LC_ALL",
        "TERM", "DISPLAY", "WAYLAND_DISPLAY", "XDG_SESSION_TYPE",
        "VIRTUAL_ENV", "CONDA_DEFAULT_ENV", "NVM_DIR", "GOPATH",
        "CARGO_HOME", "RUSTUP_HOME", "NODE_PATH",
    ];

    let mut lines: Vec<String> = Vec::new();
    for key in &important_keys {
        if let Ok(val) = std::env::var(key) {
            let display_val = if val.len() > 300 {
                format!("{}...", &val[..300])
            } else {
                val
            };
            lines.push(format!("{key}={display_val}"));
        }
    }

    // Also include any XAI_ or FALCON_ prefixed vars (redacted).
    for (k, _) in std::env::vars() {
        if (k.starts_with("XAI_") || k.starts_with("FALCON_")) && !k.contains("KEY") {
            if let Ok(val) = std::env::var(&k) {
                lines.push(format!("{k}={val}"));
            }
        }
    }

    (lines.join("\n"), Some(0))
}

fn exec_get_process_list(args: &serde_json::Value) -> (String, Option<i32>) {
    let filter = args.get("filter").and_then(|v| v.as_str());

    let ps_output = run_cmd("ps", &["aux", "--sort=-%mem"])
        .unwrap_or_else(|| "Failed to run ps".to_string());

    let result = if let Some(keyword) = filter {
        let lower_kw = keyword.to_lowercase();
        ps_output
            .lines()
            .filter(|line| {
                line.to_lowercase().contains(&lower_kw)
                    || line.starts_with("USER") // Keep header
            })
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        // Return top 30 processes by memory.
        ps_output
            .lines()
            .take(31)
            .collect::<Vec<_>>()
            .join("\n")
    };

    (result, Some(0))
}

fn exec_get_system_info() -> (String, Option<i32>) {
    let mut info = String::new();

    if let Some(uname) = run_cmd("uname", &["-a"]) {
        info.push_str(&format!("System: {uname}\n"));
    }

    // Memory info.
    if let Some(mem) = run_cmd("free", &["-h"]) {
        info.push_str(&format!("\nMemory:\n{mem}\n"));
    }

    // Disk usage.
    if let Some(df) = run_cmd("df", &["-h", "."]) {
        info.push_str(&format!("\nDisk (cwd):\n{df}\n"));
    }

    // CPU info (brief).
    if let Ok(cpuinfo) = std::fs::read_to_string("/proc/cpuinfo") {
        let model = cpuinfo
            .lines()
            .find(|l| l.starts_with("model name"))
            .map(|l| l.split(':').nth(1).unwrap_or("").trim().to_string());
        let cores = cpuinfo
            .lines()
            .filter(|l| l.starts_with("processor"))
            .count();
        if let Some(model) = model {
            info.push_str(&format!("\nCPU: {model} ({cores} cores)\n"));
        }
    }

    (info, Some(0))
}

fn exec_shell_command(args: &serde_json::Value) -> (String, Option<i32>) {
    let command = args.get("command").and_then(|v| v.as_str()).unwrap_or("");
    run_shell(command)
}

fn exec_run_script(args: &serde_json::Value) -> (String, Option<i32>) {
    let interpreter = args
        .get("interpreter")
        .and_then(|v| v.as_str())
        .unwrap_or("bash");
    let script = args.get("script").and_then(|v| v.as_str()).unwrap_or("");

    // Write script to a temp file and execute.
    let tmp_dir = std::env::temp_dir();
    let script_path = tmp_dir.join(format!("falcon_script_{}", std::process::id()));
    if let Err(e) = std::fs::write(&script_path, script) {
        return (format!("Failed to write temp script: {e}"), Some(1));
    }

    let result = run_shell(&format!("{} {}", interpreter, script_path.display()));

    // Clean up.
    let _ = std::fs::remove_file(&script_path);

    result
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Execute a shell command via `bash -c` and return (output, exit_code).
fn run_shell(command: &str) -> (String, Option<i32>) {
    match Command::new("bash").args(["-c", command]).output() {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            let exit_code = output.status.code().unwrap_or(-1);

            let mut result = String::new();
            if !stdout.is_empty() {
                result.push_str(&stdout);
            }
            if !stderr.is_empty() {
                if !result.is_empty() {
                    result.push('\n');
                }
                result.push_str(&format!("[stderr] {stderr}"));
            }
            result.push_str(&format!("\n[exit code: {exit_code}]"));

            (result, Some(exit_code))
        }
        Err(e) => (format!("Failed to execute command: {e}"), Some(-1)),
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

/// Run a command and return (stdout_or_none, exit_code).
fn run_cmd_with_status(program: &str, args: &[&str]) -> (Option<String>, i32) {
    match Command::new(program).args(args).output() {
        Ok(o) => {
            let code = o.status.code().unwrap_or(-1);
            let stdout = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if stdout.is_empty() {
                (None, code)
            } else {
                (Some(stdout), code)
            }
        }
        Err(_) => (None, -1),
    }
}

/// Intelligently truncate output: keep the first TRUNC_HEAD chars and last
/// TRUNC_TAIL chars, with a marker in between.
fn smart_truncate(output: String) -> (String, bool) {
    if output.len() <= OUTPUT_MAX {
        return (output, false);
    }
    let head = &output[..TRUNC_HEAD];
    let tail = &output[output.len() - TRUNC_TAIL..];
    let truncated = format!(
        "{head}\n\n... [truncated: {total} bytes total, showing first {h} + last {t}] ...\n\n{tail}",
        total = output.len(),
        h = TRUNC_HEAD,
        t = TRUNC_TAIL,
    );
    (truncated, true)
}

// ---------------------------------------------------------------------------
// Git tool handlers
// ---------------------------------------------------------------------------

fn exec_git_commit(args: &serde_json::Value) -> (String, Option<i32>) {
    let message = args.get("message").and_then(|v| v.as_str()).unwrap_or("");
    let files: Vec<&str> = args
        .get("files")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();

    // Stage files (or all if none specified).
    let stage_result = if files.is_empty() {
        run_shell("git add -A")
    } else {
        let paths = files.join(" ");
        run_shell(&format!("git add {paths}"))
    };
    if stage_result.1.map(|c| c != 0).unwrap_or(true) {
        return stage_result;
    }

    // Commit.
    run_shell(&format!("git commit -m {}", shell_quote(message)))
}

fn exec_git_diff(args: &serde_json::Value) -> (String, Option<i32>) {
    let staged = args.get("staged").and_then(|v| v.as_bool()).unwrap_or(false);
    let path = args.get("path").and_then(|v| v.as_str());

    let mut cmd = String::from("git diff");
    if staged {
        cmd.push_str(" --cached");
    }
    if let Some(p) = path {
        cmd.push(' ');
        cmd.push_str(p);
    }
    run_shell(&cmd)
}

fn exec_git_branch(args: &serde_json::Value) -> (String, Option<i32>) {
    let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("list");
    let name = args.get("name").and_then(|v| v.as_str()).unwrap_or("");

    match action {
        "list" => run_shell("git branch -a"),
        "create" => run_shell(&format!("git checkout -b {name}")),
        "switch" => run_shell(&format!("git checkout {name}")),
        _ => (format!("Unknown branch action: {action}"), Some(1)),
    }
}

fn exec_git_push(args: &serde_json::Value) -> (String, Option<i32>) {
    let remote = args.get("remote").and_then(|v| v.as_str()).unwrap_or("origin");
    let branch = args.get("branch").and_then(|v| v.as_str());

    let cmd = if let Some(b) = branch {
        format!("git push {remote} {b}")
    } else {
        format!("git push {remote}")
    };
    run_shell(&cmd)
}

fn exec_git_pull(args: &serde_json::Value) -> (String, Option<i32>) {
    let remote = args.get("remote").and_then(|v| v.as_str()).unwrap_or("origin");
    let branch = args.get("branch").and_then(|v| v.as_str());

    let cmd = if let Some(b) = branch {
        format!("git pull {remote} {b}")
    } else {
        format!("git pull {remote}")
    };
    run_shell(&cmd)
}

fn exec_git_log(args: &serde_json::Value) -> (String, Option<i32>) {
    let count = args.get("count").and_then(|v| v.as_i64()).unwrap_or(10);
    let oneline = args.get("oneline").and_then(|v| v.as_bool()).unwrap_or(true);

    let fmt = if oneline { "--oneline" } else { "--format=medium" };
    run_shell(&format!("git log {fmt} -{count}"))
}

// ---------------------------------------------------------------------------
// MCP bridge stub
// ---------------------------------------------------------------------------

/// Invoke a Goose/MCP tool via the `goose` CLI subprocess.
/// Falls back gracefully if goose is not installed.
fn exec_mcp_tool(args: &serde_json::Value) -> (String, Option<i32>) {
    let tool_name = args.get("tool_name").and_then(|v| v.as_str()).unwrap_or("");
    let tool_args = args.get("arguments").cloned().unwrap_or(json!({}));

    // Check if goose CLI is available.
    if run_cmd("which", &["goose"]).is_none() {
        return (
            "Goose CLI is not installed. Install it with: pipx install goose-ai".to_string(),
            Some(1),
        );
    }

    // Invoke via goose CLI. This is a stub — real integration would use
    // MCP protocol over stdio/HTTP.
    let args_json = serde_json::to_string(&tool_args).unwrap_or_default();
    run_shell(&format!("goose run --tool {tool_name} --args '{args_json}'"))
}

// ---------------------------------------------------------------------------
// Quoting helper
// ---------------------------------------------------------------------------

/// Single-quote a string for safe shell embedding.
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Helper to build a `ToolDef` concisely.
fn tool(name: &str, description: &str, parameters: serde_json::Value) -> ToolDef {
    ToolDef {
        tool_type: "function".to_string(),
        function: FunctionDef {
            name: name.to_string(),
            description: description.to_string(),
            parameters,
        },
    }
}
