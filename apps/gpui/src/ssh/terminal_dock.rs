//! Terminal dock — command input bar + history replay + path completion.
//!
//! G3 phase of `docs/plans/active/gpui-refactor.md` section 6.4.
//!
//! Sits below the terminal in the workspace layout. The user types a
//! command here, it's sent to the SSH channel (or local PTY), and the
//! command is recorded in history. Arrow-up/down replays prior commands;
//! Tab triggers path completion against the remote filesystem.
//!
//! ## Why a separate dock (not just type in the terminal)
//!
//! Tauri's terminal already supports direct typing, but the dock adds:
//! 1. **Multi-line compose** — paste a script, edit it, then send.
//! 2. **History search** — Ctrl+R fuzzy-search across all prior commands.
//! 3. **Path completion** — Tab completes remote paths without needing
//!    bash's readline (useful on minimal shells like `sh`).

use std::collections::VecDeque;

/// Command history with a size cap.
///
/// Mirrors Tauri's `terminal-command-history.json` storage shape. The
/// cap prevents unbounded growth; oldest entries are evicted FIFO.
pub struct CommandHistory {
    /// Ring buffer of past commands, oldest first.
    entries: VecDeque<String>,
    /// Maximum entries to retain. Default 1000 matches Tauri's
    /// `MAX_COMMAND_HISTORY`.
    max_entries: usize,
    /// Current position when navigating with arrow keys. `None` means
    /// "at the bottom (new entry)"; `Some(i)` means "showing entry i".
    cursor: Option<usize>,
}

impl CommandHistory {
    pub fn new(max_entries: usize) -> Self {
        Self {
            entries: VecDeque::with_capacity(max_entries.min(256)),
            max_entries,
            cursor: None,
        }
    }

    /// Record a command. Dedupes consecutive identical entries (so
    /// running `ls` three times in a row only stores one `ls`).
    pub fn push(&mut self, command: String) {
        if command.trim().is_empty() {
            return;
        }
        if self.entries.back().is_some_and(|last| last == &command) {
            // Dedupe consecutive.
            return;
        }
        if self.entries.len() >= self.max_entries {
            self.entries.pop_front();
        }
        self.entries.push_back(command);
        // Reset navigation cursor — new entry means "at the bottom".
        self.cursor = None;
    }

    /// Navigate up (toward older entries). Returns the entry to show,
    /// or `None` if already at the oldest.
    pub fn up(&mut self) -> Option<&str> {
        let len = self.entries.len();
        if len == 0 {
            return None;
        }
        self.cursor = match self.cursor {
            None => Some(len - 1),
            Some(0) => return self.entries.front().map(|s| s.as_str()),
            Some(i) => Some(i - 1),
        };
        self.cursor
            .and_then(|i| self.entries.get(i).map(|s| s.as_str()))
    }

    /// Navigate down (toward newer entries). Returns the entry to show,
    /// or `None` if at the bottom (new entry — caller clears the input).
    pub fn down(&mut self) -> Option<&str> {
        let len = self.entries.len();
        if len == 0 {
            return None;
        }
        self.cursor = match self.cursor {
            None => None,
            Some(i) if i + 1 >= len => None,
            Some(i) => Some(i + 1),
        };
        self.cursor
            .and_then(|i| self.entries.get(i).map(|s| s.as_str()))
    }

    /// Fuzzy-search history for a substring. Returns matching entries
    /// (newest first). Used by Ctrl+R.
    pub fn search(&self, query: &str) -> Vec<&str> {
        let q = query.to_lowercase();
        self.entries
            .iter()
            .rev()
            .filter(|cmd| cmd.to_lowercase().contains(&q))
            .map(|s| s.as_str())
            .collect()
    }

    /// Number of entries currently stored.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether history is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Iterate over all entries (oldest first).
    pub fn iter(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(|s| s.as_str())
    }
}

impl Default for CommandHistory {
    fn default() -> Self {
        Self::new(1000)
    }
}

/// Path completion engine.
///
/// G3 stub — real impl queries the remote filesystem via SFTP (G4) or
/// SSH exec `ls -d <prefix>*`. Returns candidate paths to complete the
/// user's Tab press.
#[allow(dead_code)]
#[derive(Default)]
pub struct PathCompleter {
    /// Cached last-completion prefix to avoid re-querying on every
    /// keystroke. Cleared when the input changes.
    last_prefix: Option<String>,
    /// Cached candidates for `last_prefix`.
    candidates: Vec<String>,
}

impl PathCompleter {
    pub fn new() -> Self {
        Self::default()
    }

    /// G3 stub — returns empty. G3.4 TODO: query remote FS.
    pub async fn complete(&mut self, _prefix: &str) -> Vec<String> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_and_navigate() {
        let mut h = CommandHistory::new(100);
        h.push("ls".into());
        h.push("cd /tmp".into());
        h.push("make test".into());

        // Up from bottom → newest.
        assert_eq!(h.up(), Some("make test"));
        assert_eq!(h.up(), Some("cd /tmp"));
        assert_eq!(h.up(), Some("ls"));
        // Already at oldest — stays.
        assert_eq!(h.up(), Some("ls"));

        // Down → newer.
        assert_eq!(h.down(), Some("cd /tmp"));
        assert_eq!(h.down(), Some("make test"));
        // At bottom → None (caller clears input).
        assert_eq!(h.down(), None);
    }

    #[test]
    fn dedupes_consecutive() {
        let mut h = CommandHistory::new(100);
        h.push("ls".into());
        h.push("ls".into());
        h.push("ls".into());
        assert_eq!(h.len(), 1);
    }

    #[test]
    fn empty_command_ignored() {
        let mut h = CommandHistory::new(100);
        h.push("".into());
        h.push("   ".into());
        assert_eq!(h.len(), 0);
    }

    #[test]
    fn evicts_oldest_when_full() {
        let mut h = CommandHistory::new(2);
        h.push("a".into());
        h.push("b".into());
        h.push("c".into());
        assert_eq!(h.len(), 2);
        // "a" should be evicted.
        assert_eq!(h.iter().collect::<Vec<_>>(), vec!["b", "c"]);
    }

    #[test]
    fn search_case_insensitive() {
        let mut h = CommandHistory::new(100);
        h.push("ls -la".into());
        h.push("make test".into());
        h.push("LS /tmp".into());

        let results = h.search("ls");
        assert_eq!(results.len(), 2);
        // Newest first.
        assert_eq!(results[0], "LS /tmp");
        assert_eq!(results[1], "ls -la");
    }
}
