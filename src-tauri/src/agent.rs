//! Agent Orchestrator — multi-step agentic loop powered by Grok tool-calling.
//!
//! Provides a ReAct-style agent that can plan, gather context, execute shell
//! commands, edit files, search codebases, and introspect the system.  Tools
//! are classified by safety level:
//!   - **ReadOnly**: auto-execute (read_file, search_files, etc.)
//!   - **Write**: requires user approval (write_file, edit_file, shell commands)
//!   - **Destructive**: requires approval + warning badge (rm, sudo, etc.)
//!
//! The agent follows a forced PLAN → EXECUTE → VERIFY operating procedure
//! injected via the system prompt.

use crate::grok::{GrokClient, ToolCall};
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

/// Maximum number of agent loop iterations to prevent runaway execution.
const MAX_ITERATIONS: usize = 15;

/// Extended iteration limit when autocorrect is enabled (more room for retries).
const MAX_ITERATIONS_AUTOCORRECT: usize = 25;

/// Maximum verification attempts before accepting the final answer.
const MAX_VERIFY_ATTEMPTS: usize = 2;

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
// GrokAgent — the core agent struct
// ---------------------------------------------------------------------------

/// Full agentic struct encapsulating context, tools, memory, and safety.
/// Created per-session but references shared resources via Arc.
pub struct GrokAgent {
    /// The Grok LLM client.
    pub grok: GrokClient,
    /// Full system context string (cwd, git, blocks, env, etc.).
    pub context: String,
    /// Available tools for this session.
    pub tools: Vec<crate::grok::ToolDef>,
    /// Persistent session/conversation memory (shared across sessions).
    pub memory: Arc<PersistentMemory>,
    /// Current autonomy level controlling approval behaviour.
    pub autonomy: AutonomyLevel,
    /// Whether dry-run mode is active (simulate but don't execute writes).
    pub dry_run: bool,
    /// Undo stack for file operations (shared with AppState).
    pub undo: Arc<UndoStack>,
}

// ---------------------------------------------------------------------------
// System prompt
// ---------------------------------------------------------------------------

/// Build the full system prompt injected into every agent conversation.
///
/// This forces a PLAN → EXECUTE → VERIFY operating procedure and injects
/// user rules + deep session context.
fn agent_system_prompt(context_info: &str, rules_fragment: &str) -> String {
    let mut prompt = String::with_capacity(16384);

    // -- Identity --
    prompt.push_str(
        "You are Falcon Agent, an AI-powered terminal assistant with deep shell access.\n\
         You can read/write files, search codebases, run commands, execute scripts, \n\
         and introspect the system to accomplish any task the user describes.\n\n",
    );

    // -- Rules (from ~/.config/falcon/rules.md and .falcon-rules.md) --
    if !rules_fragment.is_empty() {
        prompt.push_str(rules_fragment);
        prompt.push('\n');
    }

    // -- Operating procedure --
    prompt.push_str(
        "[OPERATING PROCEDURE]\n\
         Follow this sequence for every task:\n\n\
         1. PLAN\n\
            Before taking any action, state your step-by-step approach in 2-4 sentences.\n\
            Identify what information you need and which tools you will use.\n\n\
         2. EXECUTE\n\
            Work through your plan one step at a time.\n\
            - Use read-only tools freely to gather context (read_file, list_directory, \n\
              search_files, find_files, get_git_status, get_environment, get_system_info).\n\
            - Use write tools (write_file, edit_file, run_shell_command, run_script) \n\
              when you need to make changes.\n\
            - After each tool call, briefly state what you learned or what you will do next.\n\n\
         3. VERIFY\n\
            Before calling final_answer, verify your work:\n\
            - If you edited code, read the file back to confirm the edit is correct.\n\
            - If you ran a command, check the exit code and output for errors.\n\
            - If a command failed, diagnose the error and retry with a corrected approach.\n\n\
         4. COMPLETE\n\
            Call final_answer with a concise summary of what was accomplished.\n\n",
    );

    // -- Output format --
    prompt.push_str(
        "[OUTPUT FORMAT]\n\
         - Be concise and direct in your reasoning.\n\
         - Do not explain what common commands do.\n\
         - For file edits, describe the change (not the full file content).\n\
         - For multi-step tasks, number your steps.\n\
         - When showing diffs or edits, use a clear before/after format.\n\n",
    );

    // -- Safety --
    prompt.push_str(
        "[SAFETY]\n\
         - Never run destructive commands (rm, sudo, kill, etc.) without explicit reasoning.\n\
         - Prefer read-only investigation before making changes.\n\
         - If unsure about the user's intent, ask via final_answer rather than guessing.\n\
         - Never expose secrets, API keys, or passwords in output.\n\n",
    );

    // -- Context --
    if !context_info.is_empty() {
        prompt.push_str(context_info);
    }

    prompt
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
/// Accepts a fully-configured `GrokAgent` instance. The agent loop emits
/// Tauri events (`agent-step`, `agent-thinking-token`) so the frontend can
/// render each phase in real time.
pub async fn run_agent(
    app: AppHandle,
    agent: GrokAgent,
    session_id: String,
    prompt: String,
) -> Result<(), String> {
    let GrokAgent {
        grok,
        context: context_info,
        tools: agent_tools,
        memory,
        autonomy,
        dry_run: _dry_run,
        undo,
    } = agent;

    // Load rules fresh on every invocation.
    let rules_fragment = RulesEngine::as_prompt_fragment();

    let system = agent_system_prompt(&context_info, &rules_fragment);

    // Derive autocorrect behaviour from autonomy level.
    let autocorrect = matches!(
        autonomy,
        AutonomyLevel::AutoNonDestructive | AutonomyLevel::FullAuto
    );

    // Augment system prompt based on autonomy.
    let system = if autocorrect {
        format!(
            "{system}\n\n\
             [AUTOCORRECT MODE]\n\
             - Non-destructive write operations are auto-approved (no user confirmation).\n\
             - If a command fails, you MUST immediately analyze the error and run a corrected command.\n\
             - Do NOT call final_answer until all errors are resolved and the task is verified complete.\n\
             - After fixing errors, re-run verification steps to confirm the fix worked."
        )
    } else {
        system
    };

    // Record session start in persistent memory.
    let cwd = std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    memory.start_session(&session_id, &cwd, &prompt);
    memory.log_message(&session_id, "system", &system);
    memory.log_message(&session_id, "user", &prompt);

    let mut messages: Vec<serde_json::Value> = vec![
        json!({ "role": "system", "content": system }),
        json!({ "role": "user", "content": prompt }),
    ];

    // Autocorrect tracking state.
    let mut had_errors = false;
    let mut verification_attempts: usize = 0;
    let iteration_limit = if autocorrect {
        MAX_ITERATIONS_AUTOCORRECT
    } else {
        MAX_ITERATIONS
    };

    // Notify frontend that the agent has started thinking.
    emit_step(&app, &session_id, "thinking", json!("Planning..."));

    for _iteration in 0..iteration_limit {
        // ---- Call Grok with tools (streaming) ----------------------------
        let response = grok
            .stream_complete_with_tools(
                &app,
                messages.clone(),
                agent_tools.clone(),
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

            // Record summary in persistent memory.
            memory.finish_session(&session_id, &summary);

            // In autocorrect mode, if there were errors, verify before
            // accepting the final answer.
            if autocorrect && had_errors && verification_attempts < MAX_VERIFY_ATTEMPTS {
                verification_attempts += 1;
                emit_step(
                    &app,
                    &session_id,
                    "verifying",
                    json!("Verifying task completion..."),
                );

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

                let mut assistant_msg =
                    json!({ "role": "assistant", "tool_calls": tc_json });
                if !response.content.is_empty() {
                    assistant_msg["content"] = json!(response.content);
                }
                messages.push(assistant_msg);

                for tc in &response.tool_calls {
                    if tc.function.name == "final_answer" {
                        messages.push(json!({
                            "role": "tool",
                            "tool_call_id": tc.id,
                            "content": "VERIFICATION REQUIRED: Some commands had errors \
                                during this session. Please verify all errors are \
                                resolved and the original task is fully accomplished. \
                                If issues remain, fix them now. If everything is good, \
                                call final_answer again.",
                        }));
                    } else {
                        let tc_args: serde_json::Value =
                            serde_json::from_str(&tc.function.arguments)
                                .unwrap_or(json!({}));
                        let result = tools::execute_tool(&tc.function.name, &tc_args);
                        messages.push(json!({
                            "role": "tool",
                            "tool_call_id": tc.id,
                            "content": result.output,
                        }));
                    }
                }

                had_errors = false;
                emit_step(
                    &app,
                    &session_id,
                    "thinking",
                    json!("Verifying and fixing remaining errors..."),
                );
                continue;
            }

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

        // ---- Classify each tool call by safety level --------------------
        let mut auto_calls: Vec<&ToolCall> = Vec::new();
        let mut approval_needed: Vec<&ToolCall> = Vec::new();
        let mut destructive_calls: Vec<&ToolCall> = Vec::new();

        for tc in &response.tool_calls {
            let args: serde_json::Value =
                serde_json::from_str(&tc.function.arguments).unwrap_or(json!({}));
            let tool_safety = tools::classify_safety(&tc.function.name, &args);

            if safety::should_auto_approve(autonomy, tool_safety) {
                auto_calls.push(tc);
            } else if tool_safety == ToolSafety::Destructive {
                destructive_calls.push(tc);
            } else {
                approval_needed.push(tc);
            }
        }

        // ---- Auto-execute approved tools (based on autonomy level) -------
        for tc in &auto_calls {
            let args: serde_json::Value =
                serde_json::from_str(&tc.function.arguments).unwrap_or(json!({}));

            // Capture undo state for file writes/edits.
            if tc.function.name == "write_file" || tc.function.name == "edit_file" {
                if let Some(path) = args.get("path").and_then(|v| v.as_str()) {
                    undo.capture_file(&tc.function.name, path);
                }
            }

            let result = tools::execute_tool(&tc.function.name, &args);

            // Log to persistent memory.
            memory.log_tool_call(
                &session_id,
                &tc.function.name,
                &tc.function.arguments,
                &result.output,
                result.exit_code,
            );

            messages.push(json!({
                "role": "tool",
                "tool_call_id": tc.id,
                "content": result.output,
            }));
        }

        // ---- Tools that need approval (write + destructive) ---------------
        let approval_calls: Vec<&ToolCall> = approval_needed
            .iter()
            .chain(destructive_calls.iter())
            .copied()
            .collect();

        if !approval_calls.is_empty() {
            let previews: Vec<AgentCommandPreview> = approval_calls
                .iter()
                .map(|tc| {
                    let args: serde_json::Value =
                        serde_json::from_str(&tc.function.arguments)
                            .unwrap_or(json!({}));
                    let label = tool_preview_label(&tc.function.name, &args);
                    let is_destr = tools::classify_safety(&tc.function.name, &args)
                        == ToolSafety::Destructive;
                    AgentCommandPreview {
                        tool_call_id: tc.id.clone(),
                        command: label,
                        is_destructive: is_destr,
                    }
                })
                .collect();

            let any_destructive = !destructive_calls.is_empty();

            // Autocorrect mode: auto-approve non-destructive write tools.
            if autocorrect && !any_destructive {
                emit_step(
                    &app,
                    &session_id,
                    "auto-approved",
                    serde_json::to_value(&previews).unwrap_or(json!([])),
                );

                let mut error_cmds: Vec<String> = Vec::new();

                for tc in &approval_calls {
                    let args: serde_json::Value =
                        serde_json::from_str(&tc.function.arguments)
                            .unwrap_or(json!({}));
                    let label = tool_preview_label(&tc.function.name, &args);

                    emit_step(
                        &app,
                        &session_id,
                        "executing",
                        json!({ "command": label }),
                    );

                    let result = tools::execute_tool(&tc.function.name, &args);
                    let errored = result.exit_code.map(|c| c != 0).unwrap_or(false);

                    emit_step(
                        &app,
                        &session_id,
                        "output",
                        json!({ "command": label, "output": result.output }),
                    );

                    if errored {
                        had_errors = true;
                        error_cmds.push(format!("$ {label}\n{}", result.output));
                    }

                    messages.push(json!({
                        "role": "tool",
                        "tool_call_id": tc.id,
                        "content": result.output,
                    }));
                }

                if !error_cmds.is_empty() {
                    emit_step(
                        &app,
                        &session_id,
                        "auto-correcting",
                        json!({ "errors": error_cmds }),
                    );
                    messages.push(json!({
                        "role": "user",
                        "content": format!(
                            "AUTOCORRECT: The following operation(s) failed. \
                             Analyze the errors and run corrected operations \
                             immediately:\n{}",
                            error_cmds.join("\n---\n")
                        ),
                    }));
                }
            } else {
                // Manual approval flow.
                emit_step(
                    &app,
                    &session_id,
                    "commands",
                    serde_json::to_value(&previews).unwrap_or(json!([])),
                );

                let (tx, rx) = tokio::sync::oneshot::channel();
                {
                    let state = app.state::<crate::AppState>();
                    *state.agent_approval.lock().unwrap() = Some(tx);
                }

                match rx.await {
                    Ok(AgentApproval::Approve) => {
                        for tc in &approval_calls {
                            let args: serde_json::Value =
                                serde_json::from_str(&tc.function.arguments)
                                    .unwrap_or(json!({}));
                            let label = tool_preview_label(&tc.function.name, &args);

                            emit_step(
                                &app,
                                &session_id,
                                "executing",
                                json!({ "command": label }),
                            );

                            let result = tools::execute_tool(&tc.function.name, &args);

                            emit_step(
                                &app,
                                &session_id,
                                "output",
                                json!({ "command": label, "output": result.output }),
                            );

                            messages.push(json!({
                                "role": "tool",
                                "tool_call_id": tc.id,
                                "content": result.output,
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

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build a human-readable label for a tool call (shown in approval preview).
fn tool_preview_label(tool_name: &str, args: &serde_json::Value) -> String {
    match tool_name {
        "run_shell_command" => args
            .get("command")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string(),
        "run_script" => {
            let interp = args
                .get("interpreter")
                .and_then(|v| v.as_str())
                .unwrap_or("bash");
            let script = args
                .get("script")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let preview = if script.len() > 60 {
                format!("{}...", &script[..60])
            } else {
                script.to_string()
            };
            format!("[{interp} script] {preview}")
        }
        "write_file" => {
            let path = args.get("path").and_then(|v| v.as_str()).unwrap_or("?");
            format!("write_file({path})")
        }
        "edit_file" => {
            let path = args.get("path").and_then(|v| v.as_str()).unwrap_or("?");
            format!("edit_file({path})")
        }
        _ => format!("{tool_name}(...)")
    }
}

/// Clear the `agent_running` flag so a new session can start.
fn mark_finished(app: &AppHandle) {
    let state = app.state::<crate::AppState>();
    *state.agent_running.lock().unwrap() = false;
}
