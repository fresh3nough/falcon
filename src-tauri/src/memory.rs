//! Persistent Memory — SQLite-backed session history and conversation recall.
//!
//! Stores agent sessions, tool call results, and conversation turns so the
//! agent can recall past interactions and learn from them across restarts.
//! Database is stored at `~/.config/grok-terminal/memory.db`.

use chrono::Utc;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A recorded agent session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRecord {
    pub id: String,
    pub started_at: String,
    pub cwd: String,
    pub prompt: String,
    pub summary: Option<String>,
}

/// A recorded tool call within a session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallRecord {
    pub session_id: String,
    pub tool: String,
    pub args: String,
    pub output: String,
    pub exit_code: Option<i32>,
    pub timestamp: String,
}

/// A recorded conversation turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationRecord {
    pub session_id: String,
    pub role: String,
    pub content: String,
    pub timestamp: String,
}

// ---------------------------------------------------------------------------
// PersistentMemory
// ---------------------------------------------------------------------------

/// Thread-safe wrapper around a SQLite connection for agent memory.
pub struct PersistentMemory {
    conn: Mutex<Connection>,
}

impl PersistentMemory {
    /// Open (or create) the memory database at the conventional path.
    /// Falls back to an in-memory database if the filesystem path is
    /// not writable.
    pub fn new() -> Self {
        let conn = if let Some(path) = db_path() {
            // Ensure parent directory exists.
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            Connection::open(&path).unwrap_or_else(|_| {
                log::warn!("Could not open memory DB at {}, using in-memory", path.display());
                Connection::open_in_memory().expect("in-memory SQLite")
            })
        } else {
            Connection::open_in_memory().expect("in-memory SQLite")
        };

        let mem = Self {
            conn: Mutex::new(conn),
        };
        mem.init_tables();
        mem
    }

    /// Create tables if they don't exist.
    fn init_tables(&self) {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS sessions (
                id          TEXT PRIMARY KEY,
                started_at  TEXT NOT NULL,
                cwd         TEXT NOT NULL,
                prompt      TEXT NOT NULL,
                summary     TEXT
            );
            CREATE TABLE IF NOT EXISTS tool_calls (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id  TEXT NOT NULL,
                tool        TEXT NOT NULL,
                args        TEXT NOT NULL,
                output      TEXT NOT NULL,
                exit_code   INTEGER,
                timestamp   TEXT NOT NULL,
                FOREIGN KEY (session_id) REFERENCES sessions(id)
            );
            CREATE TABLE IF NOT EXISTS conversations (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id  TEXT NOT NULL,
                role        TEXT NOT NULL,
                content     TEXT NOT NULL,
                timestamp   TEXT NOT NULL,
                FOREIGN KEY (session_id) REFERENCES sessions(id)
            );
            CREATE INDEX IF NOT EXISTS idx_tc_session ON tool_calls(session_id);
            CREATE INDEX IF NOT EXISTS idx_conv_session ON conversations(session_id);",
        )
        .expect("failed to initialize memory tables");
    }

    // -- Write methods -------------------------------------------------------

    /// Start a new session record.
    pub fn start_session(&self, session_id: &str, cwd: &str, prompt: &str) {
        let conn = self.conn.lock().unwrap();
        let _ = conn.execute(
            "INSERT INTO sessions (id, started_at, cwd, prompt) VALUES (?1, ?2, ?3, ?4)",
            params![session_id, Utc::now().to_rfc3339(), cwd, prompt],
        );
    }

    /// Update a session with its final summary.
    pub fn finish_session(&self, session_id: &str, summary: &str) {
        let conn = self.conn.lock().unwrap();
        let _ = conn.execute(
            "UPDATE sessions SET summary = ?1 WHERE id = ?2",
            params![summary, session_id],
        );
    }

    /// Log a tool call result.
    pub fn log_tool_call(
        &self,
        session_id: &str,
        tool: &str,
        args: &str,
        output: &str,
        exit_code: Option<i32>,
    ) {
        let conn = self.conn.lock().unwrap();
        let _ = conn.execute(
            "INSERT INTO tool_calls (session_id, tool, args, output, exit_code, timestamp)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                session_id,
                tool,
                args,
                output,
                exit_code,
                Utc::now().to_rfc3339()
            ],
        );
    }

    /// Log a conversation turn (system/user/assistant/tool).
    pub fn log_message(&self, session_id: &str, role: &str, content: &str) {
        let conn = self.conn.lock().unwrap();
        let _ = conn.execute(
            "INSERT INTO conversations (session_id, role, content, timestamp)
             VALUES (?1, ?2, ?3, ?4)",
            params![session_id, role, content, Utc::now().to_rfc3339()],
        );
    }

    // -- Read methods --------------------------------------------------------

    /// Retrieve the N most recent sessions.
    pub fn get_recent_sessions(&self, limit: usize) -> Vec<SessionRecord> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT id, started_at, cwd, prompt, summary
                 FROM sessions ORDER BY started_at DESC LIMIT ?1",
            )
            .unwrap();

        stmt.query_map(params![limit as i64], |row| {
            Ok(SessionRecord {
                id: row.get(0)?,
                started_at: row.get(1)?,
                cwd: row.get(2)?,
                prompt: row.get(3)?,
                summary: row.get(4)?,
            })
        })
        .unwrap()
        .filter_map(|r| r.ok())
        .collect()
    }

    /// Full-text search across session prompts and summaries.
    pub fn search_history(&self, query: &str, limit: usize) -> Vec<SessionRecord> {
        let conn = self.conn.lock().unwrap();
        let pattern = format!("%{query}%");
        let mut stmt = conn
            .prepare(
                "SELECT id, started_at, cwd, prompt, summary
                 FROM sessions
                 WHERE prompt LIKE ?1 OR summary LIKE ?1
                 ORDER BY started_at DESC LIMIT ?2",
            )
            .unwrap();

        stmt.query_map(params![pattern, limit as i64], |row| {
            Ok(SessionRecord {
                id: row.get(0)?,
                started_at: row.get(1)?,
                cwd: row.get(2)?,
                prompt: row.get(3)?,
                summary: row.get(4)?,
            })
        })
        .unwrap()
        .filter_map(|r| r.ok())
        .collect()
    }

    /// Get tool calls for a specific session.
    pub fn get_session_tools(&self, session_id: &str) -> Vec<ToolCallRecord> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT session_id, tool, args, output, exit_code, timestamp
                 FROM tool_calls WHERE session_id = ?1 ORDER BY timestamp",
            )
            .unwrap();

        stmt.query_map(params![session_id], |row| {
            Ok(ToolCallRecord {
                session_id: row.get(0)?,
                tool: row.get(1)?,
                args: row.get(2)?,
                output: row.get(3)?,
                exit_code: row.get(4)?,
                timestamp: row.get(5)?,
            })
        })
        .unwrap()
        .filter_map(|r| r.ok())
        .collect()
    }
}

/// Resolve the database path: `~/.config/grok-terminal/memory.db`.
fn db_path() -> Option<PathBuf> {
    std::env::var("HOME")
        .ok()
        .map(|h| PathBuf::from(h).join(".config").join("grok-terminal").join("memory.db"))
}
