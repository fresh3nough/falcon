//! Grok Terminal — library root.
//!
//! Declares all modules and exposes the `run()` entry point that Tauri calls.

pub mod agent;
pub mod block;
pub mod context;
pub mod grok;
pub mod pty;

use agent::AgentApproval;
use block::BlockManager;
use context::ContextCollector;
use grok::{ChatMessage, GrokClient};
use pty::PtyManager;

use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, State};

/// Shared application state managed by Tauri.
pub struct AppState {
    pub pty: Mutex<Option<PtyManager>>,
    pub grok: GrokClient,
    pub blocks: BlockManager,
    pub context: ContextCollector,
    /// Oneshot sender for the in-flight agent approval request (if any).
    pub agent_approval:
        Mutex<Option<tokio::sync::oneshot::Sender<AgentApproval>>>,
    /// Whether an agent session is currently running.
    pub agent_running: Mutex<bool>,
}

// ---------------------------------------------------------------------------
// Tauri commands — these are invoked from the Svelte frontend.
// ---------------------------------------------------------------------------

/// Spawn a new PTY session.
#[tauri::command]
fn spawn_pty(
    app: AppHandle,
    state: State<'_, AppState>,
    rows: u16,
    cols: u16,
) -> Result<(), String> {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());
    let mgr = PtyManager::spawn(app, rows, cols, &shell).map_err(|e| e.to_string())?;
    *state.pty.lock().unwrap() = Some(mgr);
    Ok(())
}

/// Write raw keystrokes into the PTY.
#[tauri::command]
fn write_pty(state: State<'_, AppState>, data: String) -> Result<(), String> {
    let guard = state.pty.lock().unwrap();
    if let Some(pty) = guard.as_ref() {
        pty.write(data.as_bytes()).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Resize the PTY when the frontend terminal dimensions change.
#[tauri::command]
fn resize_pty(state: State<'_, AppState>, rows: u16, cols: u16) -> Result<(), String> {
    let guard = state.pty.lock().unwrap();
    if let Some(pty) = guard.as_ref() {
        pty.resize(rows, cols).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Create a new command block.
#[tauri::command]
fn create_block(state: State<'_, AppState>, command: String, cwd: String) -> String {
    state.context.record_command(&command);
    state.blocks.create_block(&command, &cwd)
}

/// Get all blocks for rendering.
#[tauri::command]
fn get_blocks(state: State<'_, AppState>) -> Vec<block::Block> {
    state.blocks.get_all_blocks()
}

/// Get the current session context for the sidebar.
#[tauri::command]
fn get_context(state: State<'_, AppState>) -> context::SessionContext {
    state.context.collect()
}

/// Ask Grok to explain a block's output (streaming).
#[tauri::command]
async fn grok_explain(
    app: AppHandle,
    state: State<'_, AppState>,
    block_id: String,
) -> Result<(), String> {
    if !state.grok.is_configured() {
        return Err("XAI_API_KEY not set".to_string());
    }

    let block = state
        .blocks
        .get_block(&block_id)
        .ok_or("block not found")?;

    let system_prompt = state.context.as_system_prompt();
    let messages = vec![
        ChatMessage {
            role: "system".to_string(),
            content: format!(
                "You are a helpful terminal assistant. {system_prompt}"
            ),
        },
        ChatMessage {
            role: "user".to_string(),
            content: format!(
                "Explain this command and its output:\n\n$ {}\n{}",
                block.command, block.output
            ),
        },
    ];

    state.grok.stream_complete(&app, messages).await
}

/// Ask Grok to suggest a fix for a failed command (streaming).
#[tauri::command]
async fn grok_fix(
    app: AppHandle,
    state: State<'_, AppState>,
    block_id: String,
) -> Result<(), String> {
    if !state.grok.is_configured() {
        return Err("XAI_API_KEY not set".to_string());
    }

    let block = state
        .blocks
        .get_block(&block_id)
        .ok_or("block not found")?;

    let system_prompt = state.context.as_system_prompt();
    let messages = vec![
        ChatMessage {
            role: "system".to_string(),
            content: format!(
                "You are a terminal assistant that fixes failed commands. \
                 Give the corrected command only, no explanation. {system_prompt}"
            ),
        },
        ChatMessage {
            role: "user".to_string(),
            content: format!(
                "This command failed:\n$ {}\nOutput:\n{}\n\nProvide the fixed command.",
                block.command, block.output
            ),
        },
    ];

    state.grok.stream_complete(&app, messages).await
}

/// Free-form chat with Grok (sidebar), with full session context.
#[tauri::command]
async fn grok_chat(
    app: AppHandle,
    state: State<'_, AppState>,
    user_message: String,
) -> Result<(), String> {
    if !state.grok.is_configured() {
        return Err("XAI_API_KEY not set".to_string());
    }

    let system_prompt = state.context.as_system_prompt();

    // Include recent blocks for context.
    let recent = state.blocks.get_recent_blocks(5);
    let block_context: String = recent
        .iter()
        .map(|b| format!("$ {}\n{}", b.command, b.output))
        .collect::<Vec<_>>()
        .join("\n---\n");

    let messages = vec![
        ChatMessage {
            role: "system".to_string(),
            content: format!(
                "You are Grok, an AI terminal assistant inside Grok Terminal. \
                 Help the user with shell commands, debugging, and scripting.\n\
                 {system_prompt}\n\nRecent terminal output:\n{block_context}"
            ),
        },
        ChatMessage {
            role: "user".to_string(),
            content: user_message,
        },
    ];

    state.grok.stream_complete(&app, messages).await
}

/// Generate a command from natural language (inline suggestion).
#[tauri::command]
async fn grok_generate_command(
    app: AppHandle,
    state: State<'_, AppState>,
    description: String,
) -> Result<(), String> {
    if !state.grok.is_configured() {
        return Err("XAI_API_KEY not set".to_string());
    }

    let system_prompt = state.context.as_system_prompt();
    let messages = vec![
        ChatMessage {
            role: "system".to_string(),
            content: format!(
                "You are a command generator. Given a natural language description, \
                 output ONLY the shell command with no explanation or markdown. \
                 {system_prompt}"
            ),
        },
        ChatMessage {
            role: "user".to_string(),
            content: description,
        },
    ];

    state.grok.stream_complete(&app, messages).await
}

/// Check if Grok API is configured.
#[tauri::command]
fn grok_status(state: State<'_, AppState>) -> bool {
    state.grok.is_configured()
}

// ---------------------------------------------------------------------------
// Agent commands
// ---------------------------------------------------------------------------

/// Start a new agent session from a natural-language prompt.
/// Returns the session ID immediately; the agent loop runs in the background
/// and communicates progress via `agent-step` / `agent-thinking-token` events.
#[tauri::command]
async fn agent_run(
    app: AppHandle,
    state: State<'_, AppState>,
    prompt: String,
    autocorrect: Option<bool>,
) -> Result<String, String> {
    if !state.grok.is_configured() {
        return Err("XAI_API_KEY not set".to_string());
    }
    {
        let running = state.agent_running.lock().unwrap();
        if *running {
            return Err("An agent session is already running.".to_string());
        }
    }

    let session_id = uuid::Uuid::new_v4().to_string();

    // Collect context and clone what the background task needs.
    let grok = state.grok.clone();
    let context_info = state.context.as_system_prompt();
    let recent = state.blocks.get_recent_blocks(10);
    let block_context: String = recent
        .iter()
        .map(|b| format!("$ {}\n{}", b.command, b.output))
        .collect::<Vec<_>>()
        .join("\n---\n");

    // Mark running.
    *state.agent_running.lock().unwrap() = true;

    let sid = session_id.clone();
    let app_clone = app.clone();

    let ac = autocorrect.unwrap_or(false);

    tokio::spawn(async move {
        if let Err(e) = agent::run_agent(
            app_clone.clone(),
            grok,
            sid.clone(),
            prompt,
            context_info,
            block_context,
            ac,
        )
        .await
        {
            let _ = app_clone.emit(
                "agent-step",
                agent::AgentStepEvent {
                    session_id: sid,
                    step: "error".to_string(),
                    data: serde_json::json!({ "error": e }),
                },
            );
            // Ensure the running flag is cleared on error.
            let state = app_clone.state::<AppState>();
            *state.agent_running.lock().unwrap() = false;
        }
    });

    Ok(session_id)
}

/// Approve the currently pending agent shell commands.
#[tauri::command]
fn agent_approve(state: State<'_, AppState>) -> Result<(), String> {
    let tx = state
        .agent_approval
        .lock()
        .unwrap()
        .take()
        .ok_or("No pending approval request.")?;
    tx.send(AgentApproval::Approve)
        .map_err(|_| "Approval channel closed.".to_string())
}

/// Cancel the currently running agent session.
#[tauri::command]
fn agent_cancel(state: State<'_, AppState>) -> Result<(), String> {
    if let Some(tx) = state.agent_approval.lock().unwrap().take() {
        let _ = tx.send(AgentApproval::Cancel);
    }
    *state.agent_running.lock().unwrap() = false;
    Ok(())
}

/// Check whether an agent session is active.
#[tauri::command]
fn agent_status(state: State<'_, AppState>) -> bool {
    *state.agent_running.lock().unwrap()
}

// ---------------------------------------------------------------------------
// App entry point
// ---------------------------------------------------------------------------

/// Build and run the Tauri application.
pub fn run() {
    let api_key = std::env::var("XAI_API_KEY").unwrap_or_default();

    let state = AppState {
        pty: Mutex::new(None),
        grok: GrokClient::new(api_key),
        blocks: BlockManager::new(),
        context: ContextCollector::new(),
        agent_approval: Mutex::new(None),
        agent_running: Mutex::new(false),
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            spawn_pty,
            write_pty,
            resize_pty,
            create_block,
            get_blocks,
            get_context,
            grok_explain,
            grok_fix,
            grok_chat,
            grok_generate_command,
            grok_status,
            agent_run,
            agent_approve,
            agent_cancel,
            agent_status,
        ])
        .run(tauri::generate_context!())
        .expect("failed to run Grok Terminal");
}
