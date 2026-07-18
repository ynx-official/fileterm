//! Window kind enum — the 7 window kinds from refactor.md 4.2.1.
//!
//! Typed so the registry can't accidentally mix labels. The kebab-case
//! string form matches Tauri's window labels so log lines and storage
//! keys stay interchangeable.

use std::fmt;

/// One of the 7 window kinds the GPUI runtime manages.
///
/// | Variant             | Count     | Decorations              | Holds tabs |
/// |---------------------|-----------|--------------------------|------------|
/// | Main                | 1         | macOS server / Win client | yes        |
/// | ConnectionManager   | 0 or 1    | client                   | no         |
/// | CommandManager      | 0 or 1    | client                   | no         |
/// | ConnectionForm      | 0 or 1    | client                   | no         |
/// | CommandForm         | 0 or 1    | client                   | no         |
/// | FileEditor          | 0..N      | client                   | no         |
/// | DetachedSession     | 0..N      | server                   | yes        |
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WindowKind {
    /// The primary workspace window. Exactly one exists at any time.
    Main,
    /// Connection library modal window. 0 or 1.
    ConnectionManager,
    /// Command library modal window. 0 or 1.
    CommandManager,
    /// Create/edit connection form. 0 or 1; reopens destroy the prior.
    ConnectionForm,
    /// Create/edit command form. 0 or 1; reopens destroy the prior.
    CommandForm,
    /// Standalone file editor. 0..N, keyed by content hash.
    FileEditor,
    /// Tab detached into its own window. 0..N, keyed by session id.
    DetachedSession,
}

impl WindowKind {
    /// Kebab-case label matching Tauri's window label convention.
    ///
    /// `Main` → `"main"`, `ConnectionManager` → `"connection-manager"`,
    /// etc. Used as the registry key and in log messages.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Main => "main",
            Self::ConnectionManager => "connection-manager",
            Self::CommandManager => "command-manager",
            Self::ConnectionForm => "connection-form",
            Self::CommandForm => "command-form",
            Self::FileEditor => "file-editor",
            Self::DetachedSession => "detached-session",
        }
    }

    /// Whether this window kind holds workspace tabs.
    ///
    /// Only `Main` and `DetachedSession` do — the rest are modals or
    /// standalone editors with no tab affiliation. The registry uses
    /// this to decide whether to track tab ownership for a window.
    pub fn holds_tabs(&self) -> bool {
        matches!(self, Self::Main | Self::DetachedSession)
    }

    /// Whether this window uses client-side decorations (custom titlebar).
    ///
    /// Matches refactor.md 4.2.1: all modal/editor windows use client
    /// decorations; `Main` uses server decorations on macOS (so it gets
    /// the native traffic-light buttons) and client on Windows;
    /// `DetachedSession` always uses server decorations.
    // clippy::match_like_matches_macro suggests `!matches!(self, Main | DetachedSession)`
    // but that loses the cfg!(target_os = "windows") branch on Main — the lint is a
    // false positive here because the Main arm returns a macro expansion, not a
    // bool literal. Suppress rather than mangle the logic into an if-else chain.
    #[allow(clippy::match_like_matches_macro)]
    pub fn uses_client_decorations(&self) -> bool {
        match self {
            Self::Main => cfg!(target_os = "windows"),
            Self::DetachedSession => false,
            _ => true,
        }
    }

    /// Build the full window label for a specific instance.
    ///
    /// Most kinds have a single instance so the label is just the kind
    /// (`"main"`). `FileEditor` and `DetachedSession` have multiple
    /// instances, so their labels include a suffix:
    /// `"file-editor-{hash}"`, `"detached-session-{id}"`.
    pub fn label_for(&self, instance_id: &str) -> String {
        match self {
            Self::FileEditor => format!("file-editor-{}", instance_id),
            Self::DetachedSession => format!("detached-session-{}", instance_id),
            _ => self.as_str().to_string(),
        }
    }
}

impl fmt::Display for WindowKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kebab_case_labels() {
        assert_eq!(WindowKind::Main.as_str(), "main");
        assert_eq!(WindowKind::ConnectionManager.as_str(), "connection-manager");
        assert_eq!(WindowKind::CommandManager.as_str(), "command-manager");
        assert_eq!(WindowKind::ConnectionForm.as_str(), "connection-form");
        assert_eq!(WindowKind::CommandForm.as_str(), "command-form");
        assert_eq!(WindowKind::FileEditor.as_str(), "file-editor");
        assert_eq!(WindowKind::DetachedSession.as_str(), "detached-session");
    }

    #[test]
    fn only_main_and_detached_hold_tabs() {
        assert!(WindowKind::Main.holds_tabs());
        assert!(WindowKind::DetachedSession.holds_tabs());
        assert!(!WindowKind::ConnectionManager.holds_tabs());
        assert!(!WindowKind::CommandManager.holds_tabs());
        assert!(!WindowKind::ConnectionForm.holds_tabs());
        assert!(!WindowKind::CommandForm.holds_tabs());
        assert!(!WindowKind::FileEditor.holds_tabs());
    }

    #[test]
    fn label_for_includes_instance_id_for_multi_instance_kinds() {
        assert_eq!(WindowKind::Main.label_for("ignored"), "main");
        assert_eq!(
            WindowKind::FileEditor.label_for("abc123"),
            "file-editor-abc123"
        );
        assert_eq!(
            WindowKind::DetachedSession.label_for("sess-7"),
            "detached-session-sess-7"
        );
    }
}
