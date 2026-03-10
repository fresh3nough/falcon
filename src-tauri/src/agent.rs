//! Agent Orchestrator — multi-step agentic loop powered by Grok tool-calling.
//!
//! Provides a ReAct-style agent that can plan, gather context, and execute
//! shell commands with user approval.  Safe read-only tools (read_file,
//! list_directory, get_working_directory, get_git_status) auto-execute;
//! shell commands require explicit approval via a `tokio::sync::oneshot`
//! channel stored in [`crate::AppState`].

use crate::grok::{FunctionDef, GrokClient, ToolCall, ToolDef};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::process::Command;
use tauri::{AppHandle, Emitter, Manager};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Shell patterns considered destructive — these get a warning badge in the
/// command preview but ALL shell commands still require approval.
const DESTRUCTIVE_PATTERNS: &[&str] = &[
    "rm ", "rmdir", "sudo ", "su ", "mkfs", "dd if=", "chmod ", "chown ",
    "> /dev/", "docker system prune", "docker rm", "docker rmi",
    "kill ", "pkill", "killall", "shutdown", "reboot", "halt",
    "fdisk", "DROP ", "DELETE FROM", "truncate", "format ",
];

/// Maximum number of agent loop iterations to prevent runaway execution.
const MAX_ITERATIONS: usize = 15;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Approval signal sent from the `agent_approve` / `agent_cancel` commands.
pub enum AgentApproval {
    Approve,
    Cancel,
}

/// Payload emitted as `agent-step` events to the frontend.
#[derive(Debug, Clone, Serialize)]
pub struct AgentStepEvent {
    pub session_id: String,
    pub step: String,
    pub data: serde_json::Value,
}

/// A single command preview shown in the approval pane.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentCommandPreview {
    pub tool_call_id: String,
    pub command: String,
    pub is_destructive: bool,
}

// ---------------------------------------------------------------------------
// System prompt
// ---------------------------------------------------------------------------

/// Build the system prompt injected into every agent conversation.
fn agent_system_prompt(context_info: &str) -> String {
    format!(
        "You are Grok Agent in Grok Terminal, an AI-powered terminal assistant.\n\
         You have access to the user's shell and can run commands to help them.\n\n\
         RULES:\n\
         1. Think step-by-step about what the user needs\n\
         2. Use read-only tools (read_file, list_directory, get_working_directory, \
            get_git_status) freely to gather context\n\
         3. Run shell commands ONLY via the run_shell_command tool\n\
         4. Be concise in your reasoning\n\
         5. When the task is complete, call final_answer with a brief summary\n\
         6. If a command fails, analyze the error and try a fix\n\
         7. Never run destructive commands without good reason\n\n\
         {context_info}"
    )
}

// ---------------------------------------------------------------------------
// Tool registry
// ---------------------------------------------------------------------------

/// Build the set of tools the agent can invoke.
pub fn build_tools() -> Vec<ToolDef> {
    vec![
        ToolDef {
            tool_type: "function".to_string(),
            function: FunctionDef {
                name: "run_shell_command".to_string(),
                description: "Run a shell command in the user's terminal. \
                              Returns stdout, stderr, and exit code."
                    .to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "command": {
                            "type": "string",
                            "description": "The shell command to execute"
                        }
                    },
                    "required": ["command"]
                }),
            },
        },
        ToolDef {
            tool_type: "function".to_string(),
            function: FunctionDef {
                name: "read_file".to_string(),
                description: "Read the contents of a file.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Path to the file to read"
                        }
                    },
                    "required": ["path"]
                }),
            },
        },
        ToolDef {
            tool_type: "function".to_string(),
            function: FunctionDef {
                name: "list_directory".to_string(),
                description: "List files and directories at a given path.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Directory path to list (defaults to cwd)"
                        }
                    },
                    "required": []
                }),
            },
        },
        ToolDef {
            tool_type: "function".to_string(),
            function: FunctionDef {
                name: "get_working_directory".to_string(),
                description: "Get the current working directory.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {},
                    "required": []
                }),
            },
        },
        ToolDef {
            tool_type: "function".to_string(),
            function: FunctionDef {
                name: "get_git_status".to_string(),
                description: "Get git branch and working-tree status.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {},
                    "required": []
                }),
            },
        },
        ToolDef {
            tool_type: "function".to_string(),
            function: FunctionDef {
                name: "final_answer".to_string(),
                description: "Call this when the task is complete with a brief \
                              summary of what was accomplished."
                    .to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "summary": {
                            "type": "string",
                            "description": "Brief summary of what was done"
                        }
                    },
                    "required": ["summary"]
                }),
            },
        },
    ]
}

// ---------------------------------------------------------------------------
// Safety helpers
// ---------------------------------------------------------------------------

/// Returns `true` if the command matches a known destructive pattern.
fn is_destructive(command: &str) -> bool {
    let lower = command.to_lowercase();
    DESTRUCTIVE_PATTERNS
        .iter()
        .any(|p| lower.contains(&p.to_lowercase()))
}

// ---------------------------------------------------------------------------
// Tool execution
// ---------------------------------------------------------------------------

/// Execute a safe, read-only tool (everything except `run_shell_command`).
fn execute_safe_tool(name: &str, args: &serde_json::Value) -> String {
    match name {
        "read_file" => {
            let path = args
                .get("path")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            match std::fs::read_to_string(path) {
                Ok(content) => {
                    if content.len() > 10_000 {
                        format!(
                            "{}...\n[truncated, {} total bytes]",
                            &content[..10_000],
                            content.len()
                        )
                    } else {
                        content
                    }
                }
                Err(e) => format!("Error reading file: {e}"),
            }
        }
        "list_directory" => {
            let path = args
                .get("path")
                .and_then(|v| v.as_str())
                .unwrap_or(".");
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
                    items.join("\n")
                }
                Err(e) => format!("Error listing directory: {e}"),
            }
        }
        "get_working_directory" => std::env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| "unknown".to_string()),
        "get_git_status" => {
            let branch = run_cmd("git", &["rev-parse", "--abbrev-ref", "HEAD"])
                .unwrap_or_else(|| "not a git repo".to_string());
            let status = run_cmd("git", &["status", "--short"])
                .unwrap_or_else(|| "N/A".to_string());
            format!("Branch: {branch}\nStatus:\n{status}")
        }
        _ => format!("Unknown tool: {name}"),
    }
}

/// Execute a shell command via `bash -c` and return combined output.
fn execute_shell_command(command: &str) -> String {
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

            // Truncate extremely large output to stay within context limits.
            if result.len() > 20_000 {
                format!(
                    "{}...\n[output truncated, {} total bytes]",
                    &result[..20_000],
                    result.len()
                )
            } else {
                result
            }
        }
        Err(e) => format!("Failed to execute command: {e}"),
    }
}

/// Small helper — run a program and return trimmed stdout on success.
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

// ---------------------------------------------------------------------------
// Convenience: emit an agent step event
// ---------------------------------------------------------------------------

fn emit_step(app: &AppHandle, session_id: &str, step: &str, data: serde_json::Value) {
    let _ = app.emit(
        "agent-step",
        AgentStepEvent {
            session_id: session_id.to_string(),
            step: step.to_string(),
            data,
        },
    );
}

// ---------------------------------------------------------------------------
// Agent loop
// ---------------------------------------------------------------------------

/// Run the multi-step agent loop.
///
/// * `grok` — a cloned [`GrokClient`] (cheap; reqwest uses Arc internally).
/// * `session_id` — unique ID for this agent session.
/// * `prompt` — the user's natural-language request.
/// * `context_info` — pre-collected system-prompt context string.
/// * `block_context` — serialized recent terminal blocks for extra context.
///
/// The loop emits Tauri events (`agent-step`, `agent-thinking-token`) so the
/// frontend can render each phase in real time.  When the agent needs to run
/// shell commands it stores a `oneshot::Sender` in
/// [`crate::AppState::agent_approval`] and awaits the paired receiver.
pub async fn run_agent(
    app: AppHandle,
    grok: GrokClient,
    session_id: String,
    prompt: String,
    context_info: String,
    block_context: String,
) -> Result<(), String> {
    let tools = build_tools();

    let system = agent_system_prompt(&context_info);
    let system_with_blocks = if block_context.is_empty() {
        system
    } else {
        format!("{system}\n\nRecent terminal output:\n{block_context}")
    };

    let mut messages: Vec<serde_json::Value> = vec![
        json!({ "role": "system", "content": system_with_blocks }),
        json!({ "role": "user", "content": prompt }),
    ];

    // Notify frontend that the agent has started thinking.
    emit_step(&app, &session_id, "thinking", json!("Planning..."));

    for _iteration in 0..MAX_ITERATIONS {
        // ---- Call Grok with tools (streaming) ----------------------------
        let response = grok
            .stream_complete_with_tools(
                &app,
                messages.clone(),
                tools.clone(),
                "agent-thinking-token",
            )
            .await?;

        // ---- Check for final_answer tool call ----------------------------
        if let Some(final_call) = response
            .tool_calls
            .iter()
            .find(|c| c.function.name == "final_answer")
        {
            let args: serde_json::Value =
                serde_json::from_str(&final_call.function.arguments)
                    .unwrap_or(json!({}));
            let summary = args
                .get("summary")
                .and_then(|v| v.as_str())
                .unwrap_or("Task complete.")
                .to_string();

            emit_step(&app, &session_id, "done", json!({ "summary": summary }));
            mark_finished(&app);
            return Ok(());
        }

        // ---- No tool calls — treat content as final answer ---------------
        if response.tool_calls.is_empty() {
            let summary = if response.content.is_empty() {
                "Task complete.".to_string()
            } else {
                response.content.clone()
            };
            emit_step(&app, &session_id, "done", json!({ "summary": summary }));
            mark_finished(&app);
            return Ok(());
        }

        // ---- Append assistant message (with tool_calls) to history -------
        let tc_json: Vec<serde_json::Value> = response
            .tool_calls
            .iter()
            .map(|tc| {
                json!({
                    "id": tc.id,
                    "type": "function",
                    "function": {
                        "name": tc.function.name,
                        "arguments": tc.function.arguments,
                    }
                })
            })
            .collect();

        let mut assistant_msg = json!({ "role": "assistant", "tool_calls": tc_json });
        if !response.content.is_empty() {
            assistant_msg["content"] = json!(response.content);
        }
        messages.push(assistant_msg);

        // ---- Partition into safe (auto) vs shell (needs approval) --------
        let (safe_calls, shell_calls): (Vec<&ToolCall>, Vec<&ToolCall>) = response
            .tool_calls
            .iter()
            .partition(|tc| tc.function.name != "run_shell_command");

        // Auto-execute read-only tools.
        for tc in &safe_calls {
            let args: serde_json::Value =
                serde_json::from_str(&tc.function.arguments).unwrap_or(json!({}));
            let result = execute_safe_tool(&tc.function.name, &args);
            messages.push(json!({
                "role": "tool",
                "tool_call_id": tc.id,
                "content": result,
            }));
        }

        // Shell commands: request approval.
        if !shell_calls.is_empty() {
            let previews: Vec<AgentCommandPreview> = shell_calls
                .iter()
                .map(|tc| {
                    let args: serde_json::Value =
                        serde_json::from_str(&tc.function.arguments)
                            .unwrap_or(json!({}));
                    let cmd = args
                        .get("command")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown")
                        .to_string();
                    AgentCommandPreview {
                        tool_call_id: tc.id.clone(),
                        command: cmd.clone(),
                        is_destructive: is_destructive(&cmd),
                    }
                })
                .collect();

            // Emit command preview for the frontend.
            emit_step(
                &app,
                &session_id,
                "commands",
                serde_json::to_value(&previews).unwrap_or(json!([])),
            );

            // Create a oneshot channel and park the sender in AppState.
            let (tx, rx) = tokio::sync::oneshot::channel();
            {
                let state = app.state::<crate::AppState>();
                *state.agent_approval.lock().unwrap() = Some(tx);
            }

            // Await user decision.
            match rx.await {
                Ok(AgentApproval::Approve) => {
                    for tc in &shell_calls {
                        let args: serde_json::Value =
                            serde_json::from_str(&tc.function.arguments)
                                .unwrap_or(json!({}));
                        let cmd = args
                            .get("command")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");

                        emit_step(
                            &app,
                            &session_id,
                            "executing",
                            json!({ "command": cmd }),
                        );

                        let result = execute_shell_command(cmd);

                        emit_step(
                            &app,
                            &session_id,
                            "output",
                            json!({ "command": cmd, "output": result }),
                        );

                        messages.push(json!({
                            "role": "tool",
                            "tool_call_id": tc.id,
                            "content": result,
                        }));
                    }
                }
                Ok(AgentApproval::Cancel) | Err(_) => {
                    emit_step(
                        &app,
                        &session_id,
                        "cancelled",
                        json!("Agent cancelled by user."),
                    );
                    mark_finished(&app);
                    return Ok(());
                }
            }
        }

        // Signal the frontend that the next iteration is starting.
        emit_step(&app, &session_id, "thinking", json!("Analyzing results..."));
    }

    // Safety limit reached.
    emit_step(
        &app,
        &session_id,
        "done",
        json!({ "summary": "Reached maximum iterations. Please review the results." }),
    );
    mark_finished(&app);
    Ok(())
}

/// Clear the `agent_running` flag so a new session can start.
fn mark_finished(app: &AppHandle) {
    let state = app.state::<crate::AppState>();
    *state.agent_running.lock().unwrap() = false;
}
