//! Grok Terminal — library root.
//!
//! Declares all modules and exposes the `run()` entry point that Tauri calls.

pub mod agent;
pub mod block;
pub mod context;
pub mod grok;
pub mod image;
pub mod memory;
pub mod multi_agent;
pub mod pty;
pub mod rules;
pub mod safety;
pub mod tools;

use agent::{AgentApproval, GrokAgent};
use block::BlockManager;
use context::ContextCollector;
use grok::{ChatMessage, GrokClient};
use memory::PersistentMemory;
use pty::PtyManager;
use safety::{AutonomyLevel, UndoStack};

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager, State};

/// Shared application state managed by Tauri.
pub struct AppState {
    /// All active PTY instances keyed by their ID (e.g. "main", "tab-2").
    pub ptys: Mutex<HashMap<String, PtyManager>>,
    pub grok: GrokClient,
    pub blocks: BlockManager,
    pub context: ContextCollector,
    /// Oneshot sender for the in-flight agent approval request (if any).
    pub agent_approval:
        Mutex<Option<tokio::sync::oneshot::Sender<AgentApproval>>>,
    /// Whether an agent session is currently running.
    pub agent_running: Mutex<bool>,
    /// Whether an orchestrator session is currently running.
    pub orchestrator_running: Mutex<bool>,
    /// SQLite-backed persistent memory for agent sessions.
    pub memory: Arc<PersistentMemory>,
    /// Current autonomy level (controls auto-approval behaviour).
    pub autonomy: Mutex<AutonomyLevel>,
    /// Whether dry-run mode is active.
    pub dry_run: Mutex<bool>,
    /// Undo stack for file operations.
    pub undo: Arc<UndoStack>,
}

// ---------------------------------------------------------------------------
// Tauri commands — these are invoked from the Svelte frontend.
// ---------------------------------------------------------------------------

/// Spawn a PTY session (or create a new terminal tab).
/// `pty_id` defaults to `"main"` when omitted.
#[tauri::command]
fn spawn_pty(
    app: AppHandle,
    state: State<'_, AppState>,
    rows: u16,
    cols: u16,
    pty_id: Option<String>,
) -> Result<(), String> {
    let id = pty_id.unwrap_or_else(|| "main".to_string());
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());
    let mgr = PtyManager::spawn(app, rows, cols, &shell, id.clone()).map_err(|e| e.to_string())?;
    state.ptys.lock().unwrap().insert(id, mgr);
    Ok(())
}

/// Write raw keystrokes into the PTY. `pty_id` defaults to `"main"`.
#[tauri::command]
fn write_pty(state: State<'_, AppState>, data: String, pty_id: Option<String>) -> Result<(), String> {
    let id = pty_id.unwrap_or_else(|| "main".to_string());
    let guard = state.ptys.lock().unwrap();
    if let Some(pty) = guard.get(&id) {
        pty.write(data.as_bytes()).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Resize the PTY when the frontend terminal dimensions change. `pty_id` defaults to `"main"`.
#[tauri::command]
fn resize_pty(state: State<'_, AppState>, rows: u16, cols: u16, pty_id: Option<String>) -> Result<(), String> {
    let id = pty_id.unwrap_or_else(|| "main".to_string());
    let guard = state.ptys.lock().unwrap();
    if let Some(pty) = guard.get(&id) {
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

/// Store the user's currently selected/highlighted terminal text so the
/// agent can see it in context.
#[tauri::command]
fn set_selected_text(state: State<'_, AppState>, text: Option<String>) {
    state.context.set_selected_text(text);
}

/// Start a new agent session from a natural-language prompt.
/// Returns the session ID immediately; the agent loop runs in the background
/// and communicates progress via `agent-step` / `agent-thinking-token` events.
/// Optionally attach `image_data_urls` (base64 data URLs) for vision queries.
#[tauri::command]
async fn agent_run(
    app: AppHandle,
    state: State<'_, AppState>,
    prompt: String,
    _autocorrect: Option<bool>,
    image_data_urls: Option<Vec<String>>,
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
    log::info!("[cmd] agent_run session={} prompt={:.80}", &session_id[..8], prompt);

    // Build full-depth context (block history, env diff, selected text, git).
    let context_info = state.context.as_full_system_prompt(&state.blocks);

    // Build optional multimodal content for vision-enabled prompts.
    let initial_content: Option<serde_json::Value> =
        image_data_urls.as_ref().filter(|imgs| !imgs.is_empty()).map(|imgs| {
            let mut parts = vec![serde_json::json!({ "type": "text", "text": &prompt })];
            for url in imgs {
                parts.push(serde_json::json!({
                    "type": "image_url",
                    "image_url": { "url": url }
                }));
            }
            serde_json::json!(parts)
        });

    // Construct the GrokAgent for this session.
    let agent = GrokAgent {
        grok: state.grok.clone(),
        context: context_info,
        tools: tools::build_tools(),
        memory: Arc::clone(&state.memory),
        autonomy: *state.autonomy.lock().unwrap(),
        dry_run: *state.dry_run.lock().unwrap(),
        undo: Arc::clone(&state.undo),
    };

    // Mark running.
    *state.agent_running.lock().unwrap() = true;

    let sid = session_id.clone();
    let app_clone = app.clone();

    tokio::spawn(async move {
        if let Err(e) = agent::run_agent(
            app_clone.clone(),
            agent,
            sid.clone(),
            prompt,
            initial_content,
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
// Autonomy, dry-run, and undo commands
// ---------------------------------------------------------------------------

/// Set the agent autonomy level (0-4 or name string).
#[tauri::command]
fn set_autonomy_level(state: State<'_, AppState>, level: String) -> String {
    let new_level = AutonomyLevel::from_str_loose(&level);
    *state.autonomy.lock().unwrap() = new_level;
    new_level.label().to_string()
}

/// Get the current autonomy level index (0-4).
#[tauri::command]
fn get_autonomy_level(state: State<'_, AppState>) -> u8 {
    state.autonomy.lock().unwrap().as_index()
}

/// Toggle dry-run mode on/off.
#[tauri::command]
fn set_dry_run(state: State<'_, AppState>, enabled: bool) {
    *state.dry_run.lock().unwrap() = enabled;
}

/// Get dry-run mode state.
#[tauri::command]
fn get_dry_run(state: State<'_, AppState>) -> bool {
    *state.dry_run.lock().unwrap()
}

/// Undo the most recent agent file modification.
#[tauri::command]
fn agent_undo(state: State<'_, AppState>) -> Result<String, String> {
    let entry = state.undo.undo_last()?;
    Ok(format!("Undone: {}", entry.label))
}

/// Retrieve recent agent session history from persistent memory.
#[tauri::command]
fn get_agent_history(state: State<'_, AppState>, limit: Option<usize>) -> Vec<memory::SessionRecord> {
    state.memory.get_recent_sessions(limit.unwrap_or(20))
}

// ---------------------------------------------------------------------------
// Image upload & vision commands
// ---------------------------------------------------------------------------

/// Encode an image file to a base64 data URL for the vision API.
#[tauri::command]
async fn upload_image(path: String) -> Result<String, String> {
    image::encode_image_to_data_url(&path).await
}

/// Send a multimodal (text + images) chat to Grok Vision (streaming).
#[tauri::command]
async fn grok_vision_chat(
    app: AppHandle,
    state: State<'_, AppState>,
    user_message: String,
    image_data_urls: Vec<String>,
) -> Result<(), String> {
    if !state.grok.is_configured() {
        return Err("XAI_API_KEY not set".to_string());
    }
    let system_prompt = state.context.as_system_prompt();
    let sys = format!(
        "You are Grok, an AI terminal assistant with vision capabilities. \
         Analyze the provided image(s) alongside the user's question. \
         If you identify errors, suggest fixes. If you see UI, describe it.\n{system_prompt}"
    );
    state
        .grok
        .stream_vision_chat(&app, &user_message, image_data_urls, &sys)
        .await
}

// ---------------------------------------------------------------------------
// Multi-agent orchestrator commands
// ---------------------------------------------------------------------------

/// Start a 4-agent orchestrator pipeline for a complex task.
/// Returns the session ID immediately; progress is emitted via
/// `orchestrator-step` and `orchestrator-{role}-token` events.
#[tauri::command]
async fn orchestrate_task(
    app: AppHandle,
    state: State<'_, AppState>,
    task: String,
) -> Result<String, String> {
    if !state.grok.is_configured() {
        return Err("XAI_API_KEY not set".to_string());
    }
    {
        let running = state.orchestrator_running.lock().unwrap();
        if *running {
            return Err("An orchestrator session is already running.".to_string());
        }
    }

    let session_id = format!("orch-{}", uuid::Uuid::new_v4());
    log::info!("[cmd] orchestrate_task session={} task={:.80}", &session_id[..8], task);
    let context_info = state.context.as_full_system_prompt(&state.blocks);
    let grok = state.grok.clone();
    let memory = Arc::clone(&state.memory);
    let autonomy = *state.autonomy.lock().unwrap();
    let undo = Arc::clone(&state.undo);

    *state.orchestrator_running.lock().unwrap() = true;

    let sid = session_id.clone();
    let app_clone = app.clone();

    tokio::spawn(async move {
        let result = multi_agent::run_orchestrator(
            app_clone.clone(),
            grok,
            sid.clone(),
            task,
            context_info,
            memory,
            autonomy,
            undo,
        )
        .await;

        if let Err(e) = result {
            let _ = app_clone.emit(
                "orchestrator-step",
                multi_agent::OrchestratorStepEvent {
                    session_id: sid,
                    role: "orchestrator".to_string(),
                    step: "error".to_string(),
                    data: serde_json::json!({ "error": e }),
                },
            );
        }

        let state = app_clone.state::<AppState>();
        *state.orchestrator_running.lock().unwrap() = false;
    });

    Ok(session_id)
}

/// Check whether an orchestrator session is active.
#[tauri::command]
fn orchestrator_status(state: State<'_, AppState>) -> bool {
    *state.orchestrator_running.lock().unwrap()
}

// ---------------------------------------------------------------------------
// Inline NL suggestion & block context menu
// ---------------------------------------------------------------------------

/// Generate a shell command from natural language (inline `# ` prefix).
/// Streams result tokens via `grok-token` events, same as sidebar.
#[tauri::command]
async fn grok_inline_suggest(
    app: AppHandle,
    state: State<'_, AppState>,
    partial: String,
) -> Result<(), String> {
    if !state.grok.is_configured() {
        return Err("XAI_API_KEY not set".to_string());
    }

    let system_prompt = state.context.as_system_prompt();
    let messages = vec![
        ChatMessage {
            role: "system".to_string(),
            content: format!(
                "You are a command generator inside a terminal. Given a natural language \
                 description, output ONLY the exact shell command with no explanation, \
                 no markdown fences, no commentary. {system_prompt}"
            ),
        },
        ChatMessage {
            role: "user".to_string(),
            content: partial,
        },
    ];

    state.grok.stream_complete(&app, messages).await
}

/// Perform an AI action on a specific block (context menu).
/// Actions: "explain", "fix", "script", "tests"
#[tauri::command]
async fn block_action(
    app: AppHandle,
    state: State<'_, AppState>,
    block_id: String,
    action: String,
) -> Result<(), String> {
    if !state.grok.is_configured() {
        return Err("XAI_API_KEY not set".to_string());
    }

    let block = state
        .blocks
        .get_block(&block_id)
        .ok_or("block not found")?;

    let system_prompt = state.context.as_system_prompt();

    let user_content = match action.as_str() {
        "explain" => format!(
            "Explain this command and its output:\n\n$ {}\n{}",
            block.command, block.output
        ),
        "fix" => format!(
            "This command failed:\n$ {}\nOutput:\n{}\n\nProvide the corrected command only.",
            block.command, block.output
        ),
        "script" => format!(
            "Turn this command into a reusable shell script with error handling \
             and comments:\n\n$ {}\n{}",
            block.command, block.output
        ),
        "tests" => format!(
            "Generate test cases (assertions / expected outputs) for this command:\n\n$ {}\n{}",
            block.command, block.output
        ),
        _ => return Err(format!("Unknown block action: {action}")),
    };

    let system_content = match action.as_str() {
        "fix" => format!(
            "You are a terminal assistant that fixes failed commands. \
             Give the corrected command only, no explanation. {system_prompt}"
        ),
        "script" => format!(
            "You are a shell script generator. Output a complete, well-commented \
             script. {system_prompt}"
        ),
        "tests" => format!(
            "You are a test generator for shell commands. Output test cases \
             that verify the command works correctly. {system_prompt}"
        ),
        _ => format!("You are a helpful terminal assistant. {system_prompt}"),
    };

    let messages = vec![
        ChatMessage {
            role: "system".to_string(),
            content: system_content,
        },
        ChatMessage {
            role: "user".to_string(),
            content: user_content,
        },
    ];

    state.grok.stream_complete(&app, messages).await
}

// ---------------------------------------------------------------------------
// App entry point
// ---------------------------------------------------------------------------

/// Build and run the Tauri application.
pub fn run() {
    // Initialise structured logging.
    // Set RUST_LOG=debug for verbose agent/tool traces, RUST_LOG=info (default) for key events.
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    log::info!("[falcon] starting up");

    let api_key = std::env::var("XAI_API_KEY").unwrap_or_default();

    let state = AppState {
        ptys: Mutex::new(HashMap::new()),
        grok: GrokClient::new(api_key),
        blocks: BlockManager::new(),
        context: ContextCollector::new(),
        agent_approval: Mutex::new(None),
        agent_running: Mutex::new(false),
        orchestrator_running: Mutex::new(false),
        memory: Arc::new(PersistentMemory::new()),
        // Default to FullAuto: all commands execute without approval dialogs.
        autonomy: Mutex::new(AutonomyLevel::FullAuto),
        dry_run: Mutex::new(false),
        undo: Arc::new(UndoStack::new()),
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
            set_selected_text,
            agent_run,
            agent_approve,
            agent_cancel,
            agent_status,
            set_autonomy_level,
            get_autonomy_level,
            set_dry_run,
            get_dry_run,
            agent_undo,
            get_agent_history,
            grok_inline_suggest,
            block_action,
            upload_image,
            grok_vision_chat,
            orchestrate_task,
            orchestrator_status,
        ])
        .run(tauri::generate_context!())
        .expect("failed to run Grok Terminal");
}
