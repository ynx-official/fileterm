//! File manager — directory listing + virtual-scroll table + context menu.
//!
//! G4 phase of `docs/plans/active/gpui-refactor.md` section 6.5.
//!
//! Renders remote directory contents with virtual scrolling (only the
//! visible rows + small overscan are shaped/painted) so 10k-file dirs
//! stay smooth. Right-click opens a context menu (rename / delete /
//! chmod / download).
//!
//! ## View structure
//!
//! `FileManager` is a GPUI `Entity` that holds:
//! * `cwd: PathBuf` — current directory
//! * `entries: Vec<RemoteFileEntry>` — full listing (not just visible)
//! * `selection: HashSet<String>` — selected entry names
//! * `sort: SortKey` — name / size / modified / type
//!
//! G4 stub — the struct + types are here; the `Render` impl (GPUI view)
//! lands in G4.2 when we wire it into the workspace layout.

use std::collections::HashSet;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::client::RemoteFileEntry;

/// How the file table is sorted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SortKey {
    Name,
    Size,
    Modified,
    Type,
}

/// Sort direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SortDir {
    Asc,
    Desc,
}

/// File manager state. Held as `Entity<FileManager>` inside the
/// workspace view.
#[derive(Default)]
pub struct FileManager {
    /// Current working directory on the remote host.
    pub cwd: PathBuf,
    /// Full directory listing. Updated by `SftpClient::list_dir`. The
    /// view only renders the visible slice (virtual scroll) but the
    /// full list is kept here for sort/filter.
    pub entries: Vec<RemoteFileEntry>,
    /// Selected entry names (not paths). Multi-select via Ctrl+click,
    /// range via Shift+click.
    pub selection: HashSet<String>,
    /// Current sort key + direction.
    pub sort: Option<(SortKey, SortDir)>,
}

impl FileManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Sort `entries` by the current sort key + direction. Called after
    /// `list_dir` returns and whenever the user changes the sort.
    pub fn sort_entries(&mut self) {
        let Some((key, dir)) = self.sort else {
            return;
        };
        self.entries.sort_by(|a, b| {
            let ord = match key {
                SortKey::Name => a.name.cmp(&b.name),
                SortKey::Size => a.size.cmp(&b.size),
                SortKey::Modified => a.modified.cmp(&b.modified),
                SortKey::Type => a.is_dir.cmp(&b.is_dir).then(a.name.cmp(&b.name)),
            };
            if dir == SortDir::Desc {
                ord.reverse()
            } else {
                ord
            }
        });
    }

    /// Select a single entry (replaces prior selection).
    pub fn select_one(&mut self, name: &str) {
        self.selection.clear();
        self.selection.insert(name.to_string());
    }

    /// Toggle an entry in the selection (Ctrl+click).
    pub fn toggle_select(&mut self, name: &str) {
        if self.selection.contains(name) {
            self.selection.remove(name);
        } else {
            self.selection.insert(name.to_string());
        }
    }

    /// Select a range from the last-selected to `name` (Shift+click).
    /// G4 stub — real impl tracks `last_anchor` and selects the
    /// inclusive range in `entries` order.
    pub fn select_range(&mut self, _name: &str) {
        // G4.2 TODO
    }

    /// Clear selection.
    pub fn clear_selection(&mut self) {
        self.selection.clear();
    }

    /// Number of entries currently selected.
    pub fn selection_count(&self) -> usize {
        self.selection.len()
    }

    /// Whether `name` is in the current selection.
    pub fn is_selected(&self, name: &str) -> bool {
        self.selection.contains(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, size: u64, is_dir: bool) -> RemoteFileEntry {
        RemoteFileEntry {
            name: name.into(),
            path: format!("/tmp/{}", name),
            is_dir,
            is_symlink: false,
            size,
            modified: Some(0),
            permissions: None,
            owner: None,
            group: None,
        }
    }

    #[test]
    fn sort_by_size_descending() {
        let mut fm = FileManager::new();
        fm.entries = vec![
            entry("a", 100, false),
            entry("b", 300, false),
            entry("c", 200, false),
        ];
        fm.sort = Some((SortKey::Size, SortDir::Desc));
        fm.sort_entries();
        let names: Vec<_> = fm.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["b", "c", "a"]);
    }

    #[test]
    fn sort_by_name_ascending() {
        let mut fm = FileManager::new();
        fm.entries = vec![
            entry("charlie", 0, false),
            entry("alpha", 0, false),
            entry("bravo", 0, false),
        ];
        fm.sort = Some((SortKey::Name, SortDir::Asc));
        fm.sort_entries();
        let names: Vec<_> = fm.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["alpha", "bravo", "charlie"]);
    }

    #[test]
    fn select_one_replaces_selection() {
        let mut fm = FileManager::new();
        fm.selection.insert("a".into());
        fm.select_one("b");
        assert_eq!(fm.selection_count(), 1);
        assert!(fm.is_selected("b"));
        assert!(!fm.is_selected("a"));
    }

    #[test]
    fn toggle_select() {
        let mut fm = FileManager::new();
        fm.toggle_select("a");
        fm.toggle_select("b");
        assert_eq!(fm.selection_count(), 2);
        fm.toggle_select("a");
        assert_eq!(fm.selection_count(), 1);
        assert!(fm.is_selected("b"));
    }
}
