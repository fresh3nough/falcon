//! Rules Engine — discovers, loads, and serializes user/project rules.
//!
//! Rules are markdown files that provide persistent instructions to the AI
//! agent.  They are loaded fresh on every agent invocation so edits take
//! effect immediately.
//!
//! ## Discovery order (ascending precedence):
//! 1. `~/.config/grok-terminal/rules.md`  — global user rules
//! 2. `GROK.md` in ancestor directories (root → cwd)
//! 3. `GROK.md` in the current working directory
//!
//! Legacy fallback: `~/.config/falcon/rules.md` and `.falcon-rules.md`
//! are still loaded if the new paths are absent.

use std::path::{Path, PathBuf};

/// A single loaded rule file.
#[derive(Debug, Clone)]
pub struct Rule {
    /// Absolute path the rule was loaded from.
    pub source: String,
    /// Raw markdown content.
    pub content: String,
    /// Higher value = higher precedence (overrides lower).
    pub precedence: usize,
}

/// Discovers and loads rule files from conventional locations.
pub struct RulesEngine;

impl RulesEngine {
    /// Discover all applicable rule files and return them sorted by
    /// ascending precedence (last entry wins on conflicts).
    pub fn load() -> Vec<Rule> {
        let mut rules: Vec<Rule> = Vec::new();
        let mut precedence: usize = 0;

        // 1. Global user rules: ~/.config/grok-terminal/rules.md
        //    (falls back to legacy ~/.config/falcon/rules.md)
        if let Some(home) = dirs_path() {
            let global = home.join(".config").join("grok-terminal").join("rules.md");
            let legacy = home.join(".config").join("falcon").join("rules.md");
            if let Some(rule) = try_load(&global, precedence) {
                rules.push(rule);
                precedence += 1;
            } else if let Some(rule) = try_load(&legacy, precedence) {
                rules.push(rule);
                precedence += 1;
            }
        }

        // 2. Walk from filesystem root → cwd, loading GROK.md at each
        //    ancestor.  Falls back to .falcon-rules.md per directory.
        //    Deeper directories get higher precedence.
        if let Ok(cwd) = std::env::current_dir() {
            let ancestors: Vec<&Path> = cwd.ancestors().collect();
            // Reverse so root comes first (lowest precedence).
            for ancestor in ancestors.into_iter().rev() {
                let candidate = ancestor.join("GROK.md");
                let legacy = ancestor.join(".falcon-rules.md");
                if let Some(rule) = try_load(&candidate, precedence) {
                    rules.push(rule);
                    precedence += 1;
                } else if let Some(rule) = try_load(&legacy, precedence) {
                    rules.push(rule);
                    precedence += 1;
                }
            }
        }

        rules
    }

    /// Serialize all loaded rules into a system-prompt fragment.
    /// Returns an empty string if no rules are found.
    pub fn as_prompt_fragment() -> String {
        let rules = Self::load();
        if rules.is_empty() {
            return String::new();
        }

        let mut fragment = String::from("[RULES]\n");
        fragment.push_str("The following rules are provided by the user. ");
        fragment.push_str("Rules listed later take precedence over earlier ones.\n\n");

        for (i, rule) in rules.iter().enumerate() {
            fragment.push_str(&format!(
                "--- Rule {} (source: {}, precedence: {}) ---\n{}\n\n",
                i + 1,
                rule.source,
                rule.precedence,
                rule.content.trim()
            ));
        }

        fragment
    }
}

/// Attempt to load a rule file.  Returns `None` if the file does not exist
/// or cannot be read.
fn try_load(path: &Path, precedence: usize) -> Option<Rule> {
    let content = std::fs::read_to_string(path).ok()?;
    if content.trim().is_empty() {
        return None;
    }
    Some(Rule {
        source: path.display().to_string(),
        content,
        precedence,
    })
}

/// Return the user's home directory, or `None` if unavailable.
fn dirs_path() -> Option<PathBuf> {
    std::env::var("HOME")
        .ok()
        .map(PathBuf::from)
}
