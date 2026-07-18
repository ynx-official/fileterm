use serde_json::Value;

use crate::{
    backend::{CommandLibrary, ConnectionLibrary},
    theme::ThemeMode,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NavigationSection {
    Overview,
    Connections,
    Commands,
    Settings,
}

impl NavigationSection {
    pub const ALL: [Self; 4] = [
        Self::Overview,
        Self::Connections,
        Self::Commands,
        Self::Settings,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Overview => "概览",
            Self::Connections => "连接管理器",
            Self::Commands => "命令管理器",
            Self::Settings => "设置",
        }
    }

    pub fn glyph(self) -> &'static str {
        match self {
            Self::Overview => "O",
            Self::Connections => "C",
            Self::Commands => ">_",
            Self::Settings => "S",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabStatus {
    Idle,
    Connecting,
    Connected,
    Error,
    Closed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspaceTab {
    pub id: String,
    pub title: String,
    pub status: TabStatus,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DataLoadState {
    Loading,
    Ready,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConnectionListItem {
    pub id: String,
    pub name: String,
    pub protocol: String,
    pub endpoint: String,
    pub group: String,
    pub parent_id: Option<String>,
    pub has_saved_password: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConnectionFolderItem {
    pub id: String,
    pub name: String,
    pub parent_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandListItem {
    pub id: String,
    pub name: String,
    pub command: String,
    pub description: String,
    pub parent_id: Option<String>,
    pub append_carriage_return: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandFolderItem {
    pub id: String,
    pub name: String,
    pub parent_id: Option<String>,
}

#[derive(Clone, Debug)]
pub struct AppState {
    pub theme: ThemeMode,
    pub locale: String,
    pub navigation: NavigationSection,
    pub sidebar_collapsed: bool,
    pub workspace_focus: bool,
    pub active_tab_id: String,
    pub last_terminal_tab_id: Option<String>,
    pub tabs: Vec<WorkspaceTab>,
    pub data_load_state: DataLoadState,
    pub data_error: Option<String>,
    pub connections: Vec<ConnectionListItem>,
    pub connection_folders: Vec<ConnectionFolderItem>,
    pub commands: Vec<CommandListItem>,
    pub command_folders: Vec<CommandFolderItem>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            theme: ThemeMode::Dark,
            locale: "zhCN".to_string(),
            navigation: NavigationSection::Overview,
            sidebar_collapsed: false,
            workspace_focus: false,
            active_tab_id: "overview".to_string(),
            last_terminal_tab_id: None,
            tabs: vec![WorkspaceTab {
                id: "overview".to_string(),
                title: "概览".to_string(),
                status: TabStatus::Idle,
            }],
            data_load_state: DataLoadState::Loading,
            data_error: None,
            connections: Vec::new(),
            connection_folders: Vec::new(),
            commands: Vec::new(),
            command_folders: Vec::new(),
        }
    }
}

impl AppState {
    pub fn select_navigation(&mut self, section: NavigationSection) {
        self.navigation = section;
        self.active_tab_id = match section {
            NavigationSection::Overview => "overview",
            NavigationSection::Connections => "connections",
            NavigationSection::Commands => "commands",
            NavigationSection::Settings => "settings",
        }
        .to_string();

        if let Some(tab) = self
            .tabs
            .iter_mut()
            .find(|tab| tab.id == self.active_tab_id)
        {
            tab.title = section.label().to_string();
            return;
        }

        self.tabs.push(WorkspaceTab {
            id: self.active_tab_id.clone(),
            title: section.label().to_string(),
            status: TabStatus::Idle,
        });
    }

    pub fn apply_ui_preferences(&mut self, theme: &str, locale: &str) {
        self.theme = if theme == "default-light" {
            ThemeMode::Light
        } else {
            ThemeMode::Dark
        };
        self.locale = if locale == "enUS" { "enUS" } else { "zhCN" }.to_string();
    }

    pub fn open_session_tab(&mut self, tab_id: String, title: String) {
        self.last_terminal_tab_id = Some(tab_id.clone());
        if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == tab_id) {
            tab.status = TabStatus::Connecting;
            self.active_tab_id = tab_id;
            return;
        }
        self.tabs.push(WorkspaceTab {
            id: tab_id.clone(),
            title,
            status: TabStatus::Connecting,
        });
        self.active_tab_id = tab_id;
    }

    pub fn set_tab_status(&mut self, tab_id: &str, status: TabStatus) {
        if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == tab_id) {
            tab.status = status;
        }
    }

    pub fn activate_tab(&mut self, tab_id: &str) {
        if self.tabs.iter().any(|tab| tab.id == tab_id) {
            self.active_tab_id = tab_id.to_string();
            if is_terminal_tab(tab_id) {
                self.last_terminal_tab_id = Some(tab_id.to_string());
            }
        }
    }

    pub fn close_tab(&mut self, tab_id: &str) {
        if tab_id == "overview" {
            return;
        }
        let was_active = self.active_tab_id == tab_id;
        let was_last_terminal = self.last_terminal_tab_id.as_deref() == Some(tab_id);
        self.tabs.retain(|tab| tab.id != tab_id);
        if was_last_terminal {
            self.last_terminal_tab_id = self
                .tabs
                .iter()
                .rev()
                .find(|tab| is_terminal_tab(&tab.id))
                .map(|tab| tab.id.clone());
        }
        if was_active {
            self.active_tab_id = self
                .tabs
                .last()
                .map(|tab| tab.id.clone())
                .unwrap_or_else(|| "overview".to_string());
        }
    }

    pub fn apply_connection_library(&mut self, library: ConnectionLibrary) {
        self.connections = library
            .profiles
            .iter()
            .filter_map(connection_list_item)
            .collect();
        self.connection_folders = library
            .folders
            .iter()
            .filter_map(connection_folder_item)
            .collect();
        self.data_load_state = DataLoadState::Ready;
        self.data_error = None;
    }

    pub fn apply_command_library(&mut self, library: CommandLibrary) {
        self.commands = library
            .commands
            .iter()
            .filter_map(command_list_item)
            .collect();
        self.command_folders = library
            .folders
            .iter()
            .filter_map(command_folder_item)
            .collect();
    }

    pub fn fail_data_load(&mut self, error: impl Into<String>) {
        self.data_load_state = DataLoadState::Error;
        self.data_error = Some(error.into());
    }
}

fn is_terminal_tab(tab_id: &str) -> bool {
    tab_id.starts_with("ssh:")
        || tab_id.starts_with("ftp:")
        || tab_id.starts_with("local:")
        || tab_id.starts_with("telnet:")
        || tab_id.starts_with("serial:")
}

fn connection_list_item(value: &Value) -> Option<ConnectionListItem> {
    let id = value.get("id")?.as_str()?.to_string();
    let name = value.get("name")?.as_str()?.to_string();
    let protocol = value.get("type")?.as_str()?.to_string();
    let host = value
        .get("host")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let endpoint = if protocol == "serial" {
        value
            .get("devicePath")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    } else {
        let port = value
            .get("port")
            .and_then(Value::as_u64)
            .unwrap_or_default();
        if port == 0 {
            host.to_string()
        } else {
            format!("{host}:{port}")
        }
    };

    Some(ConnectionListItem {
        id,
        name,
        protocol,
        endpoint,
        group: value
            .get("group")
            .and_then(Value::as_str)
            .unwrap_or("默认")
            .to_string(),
        parent_id: value
            .get("parentId")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        has_saved_password: value
            .get("hasSavedPassword")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    })
}

fn connection_folder_item(value: &Value) -> Option<ConnectionFolderItem> {
    Some(ConnectionFolderItem {
        id: value.get("id")?.as_str()?.to_string(),
        name: value.get("name")?.as_str()?.to_string(),
        parent_id: value
            .get("parentId")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
    })
}

fn command_list_item(value: &Value) -> Option<CommandListItem> {
    Some(CommandListItem {
        id: value.get("id")?.as_str()?.to_string(),
        name: value
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("未命名命令")
            .to_string(),
        command: value.get("command")?.as_str()?.to_string(),
        description: value
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        parent_id: value
            .get("parentId")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        append_carriage_return: value
            .get("appendCarriageReturn")
            .and_then(Value::as_bool)
            .unwrap_or(true),
    })
}

fn command_folder_item(value: &Value) -> Option<CommandFolderItem> {
    Some(CommandFolderItem {
        id: value.get("id")?.as_str()?.to_string(),
        name: value.get("name")?.as_str()?.to_string(),
        parent_id: value
            .get("parentId")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn navigation_opens_one_stable_tab_per_section() {
        let mut state = AppState::default();
        state.select_navigation(NavigationSection::Connections);
        state.select_navigation(NavigationSection::Connections);
        assert_eq!(state.tabs.len(), 2);
        assert_eq!(state.active_tab_id, "connections");
    }

    #[test]
    fn command_target_tracks_last_open_terminal_across_navigation() {
        let mut state = AppState::default();
        state.open_session_tab("local:one".to_string(), "本地终端".to_string());
        state.select_navigation(NavigationSection::Commands);
        assert_eq!(state.last_terminal_tab_id.as_deref(), Some("local:one"));

        state.open_session_tab("ssh:server".to_string(), "server".to_string());
        state.activate_tab("local:one");
        assert_eq!(state.last_terminal_tab_id.as_deref(), Some("local:one"));

        state.close_tab("local:one");
        assert_eq!(state.last_terminal_tab_id.as_deref(), Some("ssh:server"));
        state.close_tab("ssh:server");
        assert!(state.last_terminal_tab_id.is_none());
    }

    #[test]
    fn overview_tab_cannot_be_closed() {
        let mut state = AppState::default();
        state.close_tab("overview");
        assert_eq!(state.tabs.len(), 1);
    }
}
