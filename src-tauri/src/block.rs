//! Block Manager — groups shell commands and their output into discrete,
//! selectable, copyable blocks (Warp's signature UX pattern).
//!
//! Each block captures the command string, raw output, working directory,
//! timestamp, and an optional exit code.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use uuid::Uuid;

/// A single terminal block (command + output pair).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Block {
    pub id: String,
    pub command: String,
    pub output: String,
    pub cwd: String,
    pub exit_code: Option<i32>,
    pub created_at: DateTime<Utc>,
}

impl Block {
    /// Start a new block for a command being entered.
    pub fn new(command: &str, cwd: &str) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            command: command.to_string(),
            output: String::new(),
            cwd: cwd.to_string(),
            exit_code: None,
            created_at: Utc::now(),
        }
    }

    /// Append raw output bytes to this block.
    pub fn append_output(&mut self, data: &str) {
        self.output.push_str(data);
    }

    /// Finalize the block with an exit code.
    pub fn finish(&mut self, exit_code: i32) {
        self.exit_code = Some(exit_code);
    }
}

/// Manages the ordered list of blocks for the current session.
pub struct BlockManager {
    blocks: Mutex<Vec<Block>>,
}

impl BlockManager {
    pub fn new() -> Self {
        Self {
            blocks: Mutex::new(Vec::new()),
        }
    }

    /// Create and register a new block, returning its ID.
    pub fn create_block(&self, command: &str, cwd: &str) -> String {
        let block = Block::new(command, cwd);
        let id = block.id.clone();
        self.blocks.lock().unwrap().push(block);
        id
    }

    /// Append output to the most recent block (or a specific block by ID).
    pub fn append_to_current(&self, data: &str) {
        let mut blocks = self.blocks.lock().unwrap();
        if let Some(block) = blocks.last_mut() {
            block.append_output(data);
        }
    }

    /// Append output to a specific block.
    pub fn append_to_block(&self, block_id: &str, data: &str) {
        let mut blocks = self.blocks.lock().unwrap();
        if let Some(block) = blocks.iter_mut().find(|b| b.id == block_id) {
            block.append_output(data);
        }
    }

    /// Finalize a block with its exit code.
    pub fn finish_block(&self, block_id: &str, exit_code: i32) {
        let mut blocks = self.blocks.lock().unwrap();
        if let Some(block) = blocks.iter_mut().find(|b| b.id == block_id) {
            block.finish(exit_code);
        }
    }

    /// Return a snapshot of all blocks (for the frontend).
    pub fn get_all_blocks(&self) -> Vec<Block> {
        self.blocks.lock().unwrap().clone()
    }

    /// Return the last N blocks for Grok context injection.
    pub fn get_recent_blocks(&self, n: usize) -> Vec<Block> {
        let blocks = self.blocks.lock().unwrap();
        blocks.iter().rev().take(n).cloned().collect()
    }

    /// Find a single block by ID.
    pub fn get_block(&self, block_id: &str) -> Option<Block> {
        self.blocks
            .lock()
            .unwrap()
            .iter()
            .find(|b| b.id == block_id)
            .cloned()
    }
}
