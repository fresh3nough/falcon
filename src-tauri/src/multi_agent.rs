//! Multi-Agent Orchestrator — 4-agent pipeline for complex tasks.
//!
//! Roles:
//!   1. **Researcher**: gathers context, reads files, searches codebase, checks docs
//!   2. **Architect**: produces a technical plan from the research
//!   3. **Implementer**: executes the plan (edits files, runs commands)
//!   4. **Reviewer**: verifies correctness, runs tests, suggests fixes
//!
//! Each role gets a scoped tool set and a role-specific system prompt.
//! The orchestrator passes accumulated context between roles so each
//! agent sees all prior work.

use crate::agent::AgentApproval;
use crate::grok::{GrokClient, ToolCall, ToolDef};
use crate::memory::PersistentMemory;
use crate::rules::RulesEngine;
use crate::safety::{self, AutonomyLevel, UndoStack};
use crate::tools::{self, ToolSafety};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Max iterations per role agent to prevent runaway loops.
const MAX_ROLE_ITERATIONS: usize = 12;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// The four roles in the orchestrator pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentRole {
    Researcher,
    Architect,
    Implementer,
    Reviewer,
}

impl AgentRole {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Researcher => "researcher",
            Self::Architect => "architect",
            Self::Implementer => "implementer",
            Self::Reviewer => "reviewer",
        }
    }

    fn thinking_event(&self) -> String {
        format!("orchestrator-{}-token", self.label())
    }
}

/// Payload emitted as `orchestrator-step` events to the frontend.
#[derive(Debug, Clone, Serialize)]
pub struct OrchestratorStepEvent {
    pub session_id: String,
    pub role: String,
    pub step: String,
    pub data: serde_json::Value,
}

/// Accumulated artifact from a single role execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoleArtifact {
    pub role: String,
    pub summary: String,
    pub tool_log: Vec<ToolLogEntry>,
}

/// A logged tool invocation for the artifact trail.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolLogEntry {
    pub tool: String,
    pub args_preview: String,
    pub output_preview: String,
}

// ---------------------------------------------------------------------------
// Role tool sets
// ---------------------------------------------------------------------------

/// Each role gets only the tools it needs.
fn tools_for_role(role: AgentRole) -> Vec<ToolDef> {
    match role {
        AgentRole::Researcher => tools::build_tools_by_names(&[
            "read_file",
            "list_directory",
            "search_files",
            "find_files",
            "get_working_directory",
            "get_git_status",
            "get_environment",
            "get_system_info",
            "run_shell_command",
            "git_log",
            "git_diff",
            "final_answer",
        ]),
        AgentRole::Architect => tools::build_tools_by_names(&[
            "read_file",
            "list_directory",
            "search_files",
            "find_files",
            "get_working_directory",
            "get_git_status",
            "final_answer",
        ]),
        AgentRole::Implementer => tools::build_tools_by_names(&[
            "read_file",
            "write_file",
            "edit_file",
            "list_directory",
            "search_files",
            "find_files",
            "get_working_directory",
            "run_shell_command",
            "run_script",
            "get_git_status",
            "git_commit",
            "git_diff",
            "git_branch",
            "git_push",
            "git_pull",
            "call_mcp_tool",
            "final_answer",
        ]),
        AgentRole::Reviewer => tools::build_tools_by_names(&[
            "read_file",
            "list_directory",
            "search_files",
            "find_files",
            "run_shell_command",
            "run_script",
            "get_git_status",
            "git_diff",
            "git_log",
            "final_answer",
        ]),
    }
}

// ---------------------------------------------------------------------------
// Role system prompts
// ---------------------------------------------------------------------------

fn role_system_prompt(role: AgentRole, task: &str, prior_artifacts: &[RoleArtifact], rules: &str, context: &str) -> String {
    let mut prompt = String::with_capacity(8192);

    // Identity.
    let identity = match role {
        AgentRole::Researcher => {
            "You are the RESEARCHER agent in a 4-agent pipeline.\n\
             Your job: thoroughly investigate the codebase, gather all context needed to \
             understand the task, read relevant files, check dependencies, and summarize \
             your findings. Do NOT make any file changes."
        }
        AgentRole::Architect => {
            "You are the ARCHITECT agent in a 4-agent pipeline.\n\
             Your job: using the researcher's findings, produce a clear, actionable \
             technical plan. List every file to create/modify, the changes needed, \
             and the order of operations. Output a structured plan the implementer \
             can follow step-by-step."
        }
        AgentRole::Implementer => {
            "You are the IMPLEMENTER agent in a 4-agent pipeline.\n\
             Your job: execute the architect's plan exactly. Create files, edit code, \
             run build commands, install dependencies. Follow the plan step-by-step \
             and report what you did."
        }
        AgentRole::Reviewer => {
            "You are the REVIEWER agent in a 4-agent pipeline.\n\
             Your job: verify the implementer's work. Read modified files, run tests \
             and linters, check for bugs, and confirm the original task is satisfied. \
             Report any issues found or confirm everything is correct."
        }
    };

    prompt.push_str(identity);
    prompt.push_str("\n\n");

    // Task.
    prompt.push_str(&format!("[TASK]\n{task}\n\n"));

    // Prior artifacts from earlier roles.
    if !prior_artifacts.is_empty() {
        prompt.push_str("[PRIOR AGENT WORK]\n");
        for art in prior_artifacts {
            prompt.push_str(&format!(
                "--- {} ---\n{}\n\n",
                art.role.to_uppercase(),
                art.summary
            ));
        }
    }

    // Rules.
    if !rules.is_empty() {
        prompt.push_str(rules);
        prompt.push('\n');
    }

    // Context.
    if !context.is_empty() {
        prompt.push_str(context);
    }

    // Operating instructions.
    prompt.push_str(
        "\n[INSTRUCTIONS]\n\
         - Work through your role systematically.\n\
         - After each tool call, briefly state what you learned or did.\n\
         - When finished, call final_answer with a summary of your work.\n\
         - Be concise and direct.\n"
    );

    prompt
}

// ---------------------------------------------------------------------------
// Orchestrator entry point
// ---------------------------------------------------------------------------

/// Run the full 4-agent orchestrator pipeline.
///
/// Spawns each role agent sequentially, passing accumulated artifacts
/// from prior roles into each successive agent's context.
pub async fn run_orchestrator(
    app: AppHandle,
    grok: GrokClient,
    session_id: String,
    task: String,
    context_info: String,
    memory: Arc<PersistentMemory>,
    autonomy: AutonomyLevel,
    undo: Arc<UndoStack>,
) -> Result<(), String> {
    let rules_fragment = RulesEngine::as_prompt_fragment();
    let mut artifacts: Vec<RoleArtifact> = Vec::new();

    let roles = [
        AgentRole::Researcher,
        AgentRole::Architect,
        AgentRole::Implementer,
        AgentRole::Reviewer,
    ];

    memory.start_session(&session_id, "", &task);

    for role in &roles {
        emit_orch_step(&app, &session_id, role.label(), "started", json!({}));

        let artifact = run_role_agent(
            &app,
            &grok,
            &session_id,
            *role,
            &task,
            &artifacts,
            &rules_fragment,
            &context_info,
            autonomy,
            &undo,
            &memory,
        )
        .await?;

        emit_orch_step(
            &app,
            &session_id,
            role.label(),
            "done",
            json!({ "summary": &artifact.summary }),
        );

        artifacts.push(artifact);
    }

    // Final summary combining all roles.
    let final_summary: String = artifacts
        .iter()
        .map(|a| format!("[{}] {}", a.role.to_uppercase(), a.summary))
        .collect::<Vec<_>>()
        .join("\n\n");

    memory.finish_session(&session_id, &final_summary);

    emit_orch_step(
        &app,
        &session_id,
        "orchestrator",
        "complete",
        json!({ "summary": final_summary }),
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// Single role agent loop
// ---------------------------------------------------------------------------

/// Run a single role agent to completion, returning its artifact.
async fn run_role_agent(
    app: &AppHandle,
    grok: &GrokClient,
    session_id: &str,
    role: AgentRole,
    task: &str,
    prior_artifacts: &[RoleArtifact],
    rules: &str,
    context: &str,
    autonomy: AutonomyLevel,
    undo: &UndoStack,
    memory: &PersistentMemory,
) -> Result<RoleArtifact, String> {
    let system = role_system_prompt(role, task, prior_artifacts, rules, context);
    let role_tools = tools_for_role(role);
    let thinking_event = role.thinking_event();

    let mut messages: Vec<serde_json::Value> = vec![
        json!({ "role": "system", "content": system }),
        json!({ "role": "user", "content": format!("Begin your work as the {} agent.", role.label()) }),
    ];

    let mut tool_log: Vec<ToolLogEntry> = Vec::new();

    for _iter in 0..MAX_ROLE_ITERATIONS {
        let response = grok
            .stream_complete_with_tools(
                app,
                messages.clone(),
                role_tools.clone(),
                &thinking_event,
            )
            .await?;

        // Check for final_answer.
        if let Some(final_call) = response
            .tool_calls
            .iter()
            .find(|c| c.function.name == "final_answer")
        {
            let args: serde_json::Value =
                serde_json::from_str(&final_call.function.arguments).unwrap_or(json!({}));
            let summary = args
                .get("summary")
                .and_then(|v| v.as_str())
                .unwrap_or("Role complete.")
                .to_string();

            return Ok(RoleArtifact {
                role: role.label().to_string(),
                summary,
                tool_log,
            });
        }

        // No tool calls = implicit completion.
        if response.tool_calls.is_empty() {
            let summary = if response.content.is_empty() {
                "Role complete.".to_string()
            } else {
                response.content.clone()
            };
            return Ok(RoleArtifact {
                role: role.label().to_string(),
                summary,
                tool_log,
            });
        }

        // Append assistant message with tool_calls.
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

        // Classify and execute each tool call.
        for tc in &response.tool_calls {
            let args: serde_json::Value =
                serde_json::from_str(&tc.function.arguments).unwrap_or(json!({}));
            let tool_safety = tools::classify_safety(&tc.function.name, &args);

            // Determine if auto-approve or need manual approval.
            let approved = if safety::should_auto_approve(autonomy, tool_safety) {
                true
            } else if tool_safety == ToolSafety::Destructive {
                // Always require manual approval for destructive ops.
                request_approval(app, session_id, role, tc, &args).await?
            } else {
                // Write tools in non-auto modes need approval.
                request_approval(app, session_id, role, tc, &args).await?
            };

            if !approved {
                messages.push(json!({
                    "role": "tool",
                    "tool_call_id": tc.id,
                    "content": "Operation cancelled by user.",
                }));
                continue;
            }

            // Capture undo state for writes.
            if tc.function.name == "write_file" || tc.function.name == "edit_file" {
                if let Some(path) = args.get("path").and_then(|v| v.as_str()) {
                    undo.capture_file(&tc.function.name, path);
                }
            }

            let result = tools::execute_tool(&tc.function.name, &args);

            // Emit step so frontend can show live progress.
            let label = tool_preview_label(&tc.function.name, &args);
            emit_orch_step(
                app,
                session_id,
                role.label(),
                "tool_call",
                json!({
                    "tool": tc.function.name,
                    "label": label,
                    "output_preview": truncate_preview(&result.output, 500),
                    "exit_code": result.exit_code,
                }),
            );

            // Log to persistent memory.
            memory.log_tool_call(
                session_id,
                &tc.function.name,
                &tc.function.arguments,
                &result.output,
                result.exit_code,
            );

            // Log to artifact trail.
            tool_log.push(ToolLogEntry {
                tool: tc.function.name.clone(),
                args_preview: truncate_preview(&tc.function.arguments, 200),
                output_preview: truncate_preview(&result.output, 300),
            });

            messages.push(json!({
                "role": "tool",
                "tool_call_id": tc.id,
                "content": result.output,
            }));
        }
    }

    // Iteration limit hit.
    Ok(RoleArtifact {
        role: role.label().to_string(),
        summary: "Reached iteration limit.".to_string(),
        tool_log,
    })
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Emit an orchestrator-step event.
fn emit_orch_step(
    app: &AppHandle,
    session_id: &str,
    role: &str,
    step: &str,
    data: serde_json::Value,
) {
    let _ = app.emit(
        "orchestrator-step",
        OrchestratorStepEvent {
            session_id: session_id.to_string(),
            role: role.to_string(),
            step: step.to_string(),
            data,
        },
    );
}

/// Request user approval for a tool call.  Returns `true` if approved.
async fn request_approval(
    app: &AppHandle,
    session_id: &str,
    role: AgentRole,
    tc: &ToolCall,
    args: &serde_json::Value,
) -> Result<bool, String> {
    let label = tool_preview_label(&tc.function.name, args);
    let is_destr = tools::classify_safety(&tc.function.name, args) == ToolSafety::Destructive;

    emit_orch_step(
        app,
        session_id,
        role.label(),
        "approval_needed",
        json!({
            "tool": tc.function.name,
            "label": label,
            "is_destructive": is_destr,
        }),
    );

    let (tx, rx) = tokio::sync::oneshot::channel();
    {
        let state = app.state::<crate::AppState>();
        *state.agent_approval.lock().unwrap() = Some(tx);
    }

    match rx.await {
        Ok(AgentApproval::Approve) => Ok(true),
        Ok(AgentApproval::Cancel) | Err(_) => Ok(false),
    }
}

/// Build a human-readable label for a tool call preview.
fn tool_preview_label(tool_name: &str, args: &serde_json::Value) -> String {
    match tool_name {
        "run_shell_command" => args
            .get("command")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string(),
        "write_file" | "edit_file" => {
            let path = args.get("path").and_then(|v| v.as_str()).unwrap_or("?");
            format!("{tool_name}({path})")
        }
        _ => format!("{tool_name}(...)"),
    }
}

/// Truncate a string for preview display.
fn truncate_preview(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max])
    }
}
