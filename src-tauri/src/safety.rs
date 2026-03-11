//! Safety & Autonomy — controls how much the agent can do without asking.
//!
//! Provides an autonomy slider (5 levels), dry-run simulation, and an undo
//! stack that captures file state before edits so the user can roll back.

use crate::tools::ToolSafety;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::Mutex;

// ---------------------------------------------------------------------------
// Autonomy levels
// ---------------------------------------------------------------------------

/// How much freedom the agent has to execute tools without user approval.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AutonomyLevel {
    /// Show plans and suggestions only — never execute anything.
    Suggest,
    /// Ask for approval on every single tool call (including reads).
    AskAll,
    /// Auto-execute read-only tools; ask for everything else (default).
    AutoReadOnly,
    /// Auto-execute reads and non-destructive writes; ask for destructive.
    AutoNonDestructive,
    /// Full auto — execute everything including destructive commands.
    FullAuto,
}

impl Default for AutonomyLevel {
    fn default() -> Self {
        Self::AutoReadOnly
    }
}

impl AutonomyLevel {
    /// Parse from a string (e.g. from frontend slider).
    pub fn from_str_loose(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "suggest" | "0" => Self::Suggest,
            "ask_all" | "askall" | "1" => Self::AskAll,
            "auto_read_only" | "autoreadonly" | "2" => Self::AutoReadOnly,
            "auto_non_destructive" | "autonondestructive" | "3" => Self::AutoNonDestructive,
            "full_auto" | "fullauto" | "4" => Self::FullAuto,
            _ => Self::AutoReadOnly,
        }
    }

    /// Numeric index for the frontend slider (0-4).
    pub fn as_index(&self) -> u8 {
        match self {
            Self::Suggest => 0,
            Self::AskAll => 1,
            Self::AutoReadOnly => 2,
            Self::AutoNonDestructive => 3,
            Self::FullAuto => 4,
        }
    }

    /// Human-readable label.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Suggest => "Suggest Only",
            Self::AskAll => "Ask Everything",
            Self::AutoReadOnly => "Auto Read-Only",
            Self::AutoNonDestructive => "Auto Non-Destructive",
            Self::FullAuto => "Full Auto",
        }
    }
}

/// Determine whether a tool call at the given safety level should be
/// auto-approved under the specified autonomy setting.
pub fn should_auto_approve(level: AutonomyLevel, safety: ToolSafety) -> bool {
    match level {
        AutonomyLevel::Suggest => false,
        AutonomyLevel::AskAll => false,
        AutonomyLevel::AutoReadOnly => safety == ToolSafety::ReadOnly,
        AutonomyLevel::AutoNonDestructive => {
            safety == ToolSafety::ReadOnly || safety == ToolSafety::Write
        }
        AutonomyLevel::FullAuto => true,
    }
}

// ---------------------------------------------------------------------------
// Dry-run
// ---------------------------------------------------------------------------

/// Simulated result of a tool call in dry-run mode.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DryRunResult {
    pub tool: String,
    pub description: String,
    pub would_affect: Vec<String>,
}

/// Generate a dry-run description for a tool call.
pub fn dry_run_preview(tool_name: &str, args: &serde_json::Value) -> DryRunResult {
    let desc = match tool_name {
        "run_shell_command" => {
            let cmd = args.get("command").and_then(|v| v.as_str()).unwrap_or("?");
            format!("Would run: $ {cmd}")
        }
        "write_file" => {
            let path = args.get("path").and_then(|v| v.as_str()).unwrap_or("?");
            let len = args
                .get("content")
                .and_then(|v| v.as_str())
                .map(|s| s.len())
                .unwrap_or(0);
            format!("Would write {len} bytes to {path}")
        }
        "edit_file" => {
            let path = args.get("path").and_then(|v| v.as_str()).unwrap_or("?");
            format!("Would edit {path} (find-and-replace)")
        }
        "run_script" => {
            let interp = args
                .get("interpreter")
                .and_then(|v| v.as_str())
                .unwrap_or("bash");
            format!("Would execute a {interp} script")
        }
        _ => format!("Would call {tool_name}"),
    };

    let affected = match tool_name {
        "write_file" | "edit_file" => {
            let path = args.get("path").and_then(|v| v.as_str()).unwrap_or("?");
            vec![path.to_string()]
        }
        _ => vec![],
    };

    DryRunResult {
        tool: tool_name.to_string(),
        description: desc,
        would_affect: affected,
    }
}

// ---------------------------------------------------------------------------
// Undo stack
// ---------------------------------------------------------------------------

/// Maximum number of undo entries to keep.
const UNDO_LIMIT: usize = 50;

/// A single undoable action.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UndoEntry {
    /// Human-readable label of the action.
    pub label: String,
    /// File path that was modified (if applicable).
    pub file_path: Option<String>,
    /// File content before the modification (for file edits/writes).
    pub previous_content: Option<String>,
    /// Timestamp of the action.
    pub timestamp: String,
}

/// Stack of undoable actions, capped at UNDO_LIMIT.
pub struct UndoStack {
    entries: Mutex<VecDeque<UndoEntry>>,
}

impl UndoStack {
    pub fn new() -> Self {
        Self {
            entries: Mutex::new(VecDeque::with_capacity(UNDO_LIMIT)),
        }
    }

    /// Capture file state before a write/edit and push onto the stack.
    pub fn capture_file(&self, label: &str, path: &str) {
        let previous = std::fs::read_to_string(path).ok();
        let entry = UndoEntry {
            label: label.to_string(),
            file_path: Some(path.to_string()),
            previous_content: previous,
            timestamp: chrono::Utc::now().to_rfc3339(),
        };
        self.push(entry);
    }

    /// Push a generic undo entry (e.g. for shell commands — no auto-undo).
    pub fn push(&self, entry: UndoEntry) {
        let mut stack = self.entries.lock().unwrap();
        if stack.len() >= UNDO_LIMIT {
            stack.pop_front();
        }
        stack.push_back(entry);
    }

    /// Undo the most recent file modification. Returns the entry if
    /// the file was successfully restored, or a descriptive error.
    pub fn undo_last(&self) -> Result<UndoEntry, String> {
        let mut stack = self.entries.lock().unwrap();
        let entry = stack.pop_back().ok_or("Nothing to undo.")?;

        if let (Some(path), Some(content)) = (&entry.file_path, &entry.previous_content) {
            std::fs::write(path, content)
                .map_err(|e| format!("Failed to restore {path}: {e}"))?;
        }

        Ok(entry)
    }

    /// Undo all stacked file modifications (most recent first).
    pub fn undo_all(&self) -> Vec<Result<UndoEntry, String>> {
        let mut results = Vec::new();
        loop {
            let has_entries = {
                let stack = self.entries.lock().unwrap();
                !stack.is_empty()
            };
            if !has_entries {
                break;
            }
            results.push(self.undo_last());
        }
        results
    }

    /// Get a snapshot of all pending undo entries (most recent last).
    pub fn entries(&self) -> Vec<UndoEntry> {
        self.entries.lock().unwrap().iter().cloned().collect()
    }
}
