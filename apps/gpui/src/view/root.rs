mod connection_manager;
mod ssh_key_manager;
mod webdav;

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use connection_manager::PendingConnectionEditor;
use ssh_key_manager::{PendingSshKeyEditor, SshKeyDeleteTarget, SshKeyDragItem};
use webdav::PendingWebDavEditor;

use serde_json::Value;
use zeroize::Zeroize;

use gpui::{
    div, prelude::*, px, size, App, Bounds, Context, Entity, FocusHandle, Focusable, IntoElement,
    KeyDownEvent, MouseButton, MouseDownEvent, MouseUpEvent, Render, Subscription, TitlebarOptions,
    Window, WindowBounds, WindowDecorations, WindowHandle, WindowKind as GpuiWindowKind,
    WindowOptions,
};

use crate::{
    backend::{FileTermDesktopApi, SshConnectOptions, UiPreferencesInput},
    state::{AppState, DataLoadState, NavigationSection, TabStatus},
    theme::{ThemeMode, ThemePalette},
    view::{
        DetachedSessionContent, DetachedSessionTab, DetachedSessionWindow, FtpWorkspace,
        LocalSessionWorkspace, SessionWorkspace, StreamSessionWorkspace,
    },
    window::{
        detach_tab_to_new_window,
        menu::{
            CloseTab, NewConnection, OpenCommandManager, OpenConnectionManager, OpenDocs,
            OpenSshKeyManager, ToggleTheme,
        },
        DragDropTarget, ScreenBounds, SharedWindowRegistry, TabDragState,
    },
};

pub struct RootView {
    api: Arc<dyn FileTermDesktopApi>,
    window_registry: SharedWindowRegistry,
    state: Entity<AppState>,
    focus: FocusHandle,
    sessions: HashMap<String, Entity<SessionWorkspace>>,
    ftp_sessions: HashMap<String, Entity<FtpWorkspace>>,
    local_sessions: HashMap<String, Entity<LocalSessionWorkspace>>,
    stream_sessions: HashMap<String, Entity<StreamSessionWorkspace>>,
    tab_drag: TabDragState,
    pending_host_verification: Option<PendingHostVerification>,
    pending_authentication: Option<PendingAuthentication>,
    pending_connection_editor: Option<PendingConnectionEditor>,
    pending_ssh_key_editor: Option<PendingSshKeyEditor>,
    pending_ssh_key_delete: Option<SshKeyDeleteTarget>,
    ssh_key_dragging: Option<SshKeyDragItem>,
    expanded_ssh_key_folders: HashSet<String>,
    ssh_key_query: String,
    ssh_key_search_focused: bool,
    active_ssh_key_folder: Option<String>,
    pending_command_editor: Option<PendingCommandEditor>,
    pending_command_delete: Option<(String, String)>,
    pending_webdav_editor: Option<PendingWebDavEditor>,
    webdav_config: Option<Value>,
    webdav_busy: bool,
    webdav_message: Option<String>,
    window_label: String,
    standalone_section: Option<NavigationSection>,
    _state_subscription: Subscription,
}

#[derive(Clone)]
struct PendingHostVerification {
    profile_id: String,
    title: String,
    host: String,
    port: u16,
    fingerprint: String,
    changed: bool,
    options: SshConnectOptions,
}

#[derive(Clone)]
struct PendingAuthentication {
    profile_id: String,
    title: String,
    prompts: Vec<crate::error::SshAuthenticationPrompt>,
    answers: Vec<String>,
    input: String,
    options: SshConnectOptions,
}

#[derive(Clone)]
struct PendingCommandEditor {
    command_id: Option<String>,
    input: String,
}

impl Drop for PendingAuthentication {
    fn drop(&mut self) {
        self.input.zeroize();
        self.answers.zeroize();
        self.options.clear_transient_secrets();
    }
}

impl RootView {
    pub fn new(
        api: Arc<dyn FileTermDesktopApi>,
        window_registry: SharedWindowRegistry,
        cx: &mut Context<Self>,
    ) -> Self {
        let state = cx.new(|_| AppState::default());
        let state_subscription = cx.observe(&state, |_, _, cx| cx.notify());
        let state_for_load = state.downgrade();
        let api_for_load = api.clone();
        cx.spawn(async move |_, cx| {
            let result = api_for_load.app_get_connection_library().await;
            let _ = state_for_load.update(cx, |state, cx| {
                match result {
                    Ok(library) => state.apply_connection_library(library),
                    Err(error) => state.fail_data_load(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();

        let state_for_preferences = state.downgrade();
        let api_for_preferences = api.clone();
        cx.spawn(async move |_, cx| {
            let result = api_for_preferences.app_get_ui_preferences().await;
            let _ = state_for_preferences.update(cx, |state, cx| {
                match result {
                    Ok(preferences) => {
                        state.apply_ui_preferences(&preferences.theme, &preferences.locale)
                    }
                    Err(error) => state.data_error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();

        let state_for_commands = state.downgrade();
        let api_for_commands = api.clone();
        cx.spawn(async move |_, cx| {
            let result = api_for_commands.app_get_command_library().await;
            let _ = state_for_commands.update(cx, |state, cx| {
                match result {
                    Ok(library) => state.apply_command_library(library),
                    Err(error) => state.data_error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();

        let state_for_keys = state.downgrade();
        let api_for_keys = api.clone();
        cx.spawn(async move |_, cx| {
            let result = tokio::try_join!(
                api_for_keys.ssh_keys_list(),
                api_for_keys.ssh_keys_get_layout()
            );
            let _ = state_for_keys.update(cx, |state, cx| {
                match result {
                    Ok((keys, layout)) => state.apply_ssh_key_library(keys, layout),
                    Err(error) => state.data_error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();

        let view = Self {
            api: api.clone(),
            window_registry,
            state,
            focus: cx.focus_handle(),
            sessions: HashMap::new(),
            ftp_sessions: HashMap::new(),
            local_sessions: HashMap::new(),
            stream_sessions: HashMap::new(),
            tab_drag: TabDragState::new(),
            pending_host_verification: None,
            pending_authentication: None,
            pending_connection_editor: None,
            pending_ssh_key_editor: None,
            pending_ssh_key_delete: None,
            ssh_key_dragging: None,
            expanded_ssh_key_folders: HashSet::new(),
            ssh_key_query: String::new(),
            ssh_key_search_focused: false,
            active_ssh_key_folder: None,
            pending_command_editor: None,
            pending_command_delete: None,
            pending_webdav_editor: None,
            webdav_config: None,
            webdav_busy: false,
            webdav_message: None,
            window_label: "main".to_string(),
            standalone_section: None,
            _state_subscription: state_subscription,
        };
        cx.spawn(async move |this, cx| {
            let result = api.webdav_get_config().await;
            let _ = this.update(cx, |root, cx| {
                match result {
                    Ok(config) => root.webdav_config = Some(config),
                    Err(error) => root.webdav_message = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
        view
    }

    pub fn new_standalone(
        api: Arc<dyn FileTermDesktopApi>,
        window_registry: SharedWindowRegistry,
        window_label: impl Into<String>,
        section: NavigationSection,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut view = Self::new(api, window_registry, cx);
        view.window_label = window_label.into();
        view.standalone_section = Some(section);
        view.state.update(cx, |state, cx| {
            state.select_navigation(section);
            cx.notify();
        });
        view
    }

    pub fn restore_detached_tabs(&mut self, tab_ids: Vec<String>, cx: &mut Context<Self>) {
        if let Some(tab_id) = tab_ids.last().cloned() {
            self.update_state(cx, |state| state.activate_tab(&tab_id));
        }
        cx.notify();
    }

    fn update_state(&self, cx: &mut Context<Self>, update: impl FnOnce(&mut AppState)) {
        self.state.update(cx, |state, cx| {
            update(state);
            cx.notify();
        });
    }

    fn select_navigation(&mut self, section: NavigationSection, cx: &mut Context<Self>) {
        if section != NavigationSection::SshKeys {
            self.ssh_key_search_focused = false;
        }
        self.update_state(cx, |state| state.select_navigation(section));
    }

    fn open_connection_manager(
        &mut self,
        _: &OpenConnectionManager,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_standalone_manager(
            "connection-manager",
            "FileTerm · 连接管理器",
            NavigationSection::Connections,
            cx,
        );
    }

    fn new_connection(&mut self, _: &NewConnection, _: &mut Window, cx: &mut Context<Self>) {
        self.open_standalone_manager(
            "connection-manager",
            "FileTerm · 连接管理器",
            NavigationSection::Connections,
            cx,
        );
        if let Some(window_id) = self.window_registry.handle_for("connection-manager") {
            let handle = WindowHandle::<RootView>::new(window_id);
            let _ = handle.update(cx, |root, window, _| {
                root.pending_connection_editor = Some(PendingConnectionEditor::new());
                window.refresh();
                window.activate_window();
            });
        }
    }

    fn open_command_manager(
        &mut self,
        _: &OpenCommandManager,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_standalone_manager(
            "command-manager",
            "FileTerm · 命令管理器",
            NavigationSection::Commands,
            cx,
        );
    }

    fn open_ssh_key_manager(
        &mut self,
        _: &OpenSshKeyManager,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_standalone_manager(
            "ssh-key-manager",
            "FileTerm · 密钥管理器",
            NavigationSection::SshKeys,
            cx,
        );
    }

    fn open_standalone_manager(
        &mut self,
        window_label: &'static str,
        title: &'static str,
        section: NavigationSection,
        cx: &mut Context<Self>,
    ) {
        if let Some(window_id) = self.window_registry.handle_for(window_label) {
            let handle = WindowHandle::<RootView>::new(window_id);
            let _ = handle.update(cx, |_, window, _| window.activate_window());
            return;
        }

        let api = self.api.clone();
        let registry = self.window_registry.clone();
        let registry_for_view = registry.clone();
        let bounds = Bounds::centered(None, size(px(980.0), px(700.0)), cx);
        match cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some(title.into()),
                    appears_transparent: cfg!(target_os = "macos"),
                    ..Default::default()
                }),
                window_decorations: if cfg!(target_os = "windows") {
                    Some(WindowDecorations::Client)
                } else {
                    Some(WindowDecorations::Server)
                },
                kind: GpuiWindowKind::Normal,
                ..Default::default()
            },
            move |window, cx| {
                let root = cx.new(|cx| {
                    RootView::new_standalone(
                        api.clone(),
                        registry_for_view.clone(),
                        window_label,
                        section,
                        cx,
                    )
                });
                root.focus_handle(cx).focus(window, cx);
                root
            },
        ) {
            Ok(handle) => registry.register_handle(window_label, handle.window_id()),
            Err(error) => self.update_state(cx, |state| {
                state.data_error = Some(format!("打开{title}失败: {error}"));
            }),
        }
    }

    fn close_active_tab(&mut self, _: &CloseTab, _: &mut Window, cx: &mut Context<Self>) {
        let tab_id = self.state.read(cx).active_tab_id.clone();
        self.close_tab(&tab_id, cx);
    }

    fn open_docs(&mut self, _: &OpenDocs, _: &mut Window, cx: &mut Context<Self>) {
        let api = self.api.clone();
        cx.spawn(async move |this, cx| {
            let result = api
                .app_open_external_url("https://github.com/yunanxing/fileterm".to_string())
                .await;
            if let Err(error) = result {
                let _ = this.update(cx, |root, cx| {
                    root.update_state(cx, |state| state.data_error = Some(error.to_string()));
                });
            }
        })
        .detach();
    }

    fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.update_state(cx, |state| {
            state.sidebar_collapsed = !state.sidebar_collapsed
        });
    }

    fn toggle_focus_mode(&mut self, cx: &mut Context<Self>) {
        self.update_state(cx, |state| state.workspace_focus = !state.workspace_focus);
    }

    fn toggle_theme(&mut self, _: &ToggleTheme, _: &mut Window, cx: &mut Context<Self>) {
        let next = self.state.read(cx).theme.toggled();
        self.update_state(cx, |state| state.theme = next);
        self.persist_preferences(next, self.state.read(cx).locale.clone(), cx);
    }

    fn set_locale(&mut self, locale: &'static str, cx: &mut Context<Self>) {
        let theme = self.state.read(cx).theme;
        self.update_state(cx, |state| state.locale = locale.to_string());
        self.persist_preferences(theme, locale.to_string(), cx);
    }

    fn persist_preferences(&self, theme: ThemeMode, locale: String, cx: &mut Context<Self>) {
        let api = self.api.clone();
        let state = self.state.downgrade();
        let theme = match theme {
            ThemeMode::Dark => "default-dark",
            ThemeMode::Light => "default-light",
        }
        .to_string();
        cx.spawn(async move |_, cx| {
            let result = api
                .app_set_ui_preferences(UiPreferencesInput {
                    theme: Some(theme),
                    locale: Some(locale),
                })
                .await;
            if let Err(error) = result {
                let _ = state.update(cx, |state, cx| {
                    state.data_error = Some(error.to_string());
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn open_local_terminal(&mut self, cx: &mut Context<Self>) {
        let tab_id = format!("local:{}", uuid::Uuid::new_v4());
        let title = "本地终端".to_string();
        let app_state = self.state.clone();
        let workspace = cx.new(|cx| LocalSessionWorkspace::spawn(app_state, cx));
        self.local_sessions.insert(tab_id.clone(), workspace);
        self.update_state(cx, |state| {
            state.open_session_tab(tab_id.clone(), title);
            state.set_tab_status(&tab_id, TabStatus::Connected);
        });
    }

    fn detached_tab(&self, tab_id: &str, cx: &App) -> Option<DetachedSessionTab> {
        let content = if let Some(session) = self.sessions.get(tab_id) {
            DetachedSessionContent::Ssh(session.clone())
        } else if let Some(session) = self.ftp_sessions.get(tab_id) {
            DetachedSessionContent::Ftp(session.clone())
        } else if let Some(session) = self.local_sessions.get(tab_id) {
            DetachedSessionContent::Local(session.clone())
        } else {
            let session = self.stream_sessions.get(tab_id)?;
            DetachedSessionContent::Stream(session.clone())
        };
        let title = self
            .state
            .read(cx)
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .map(|tab| tab.title.clone())
            .unwrap_or_else(|| "FileTerm 会话".to_string());
        Some(DetachedSessionTab {
            tab_id: tab_id.to_string(),
            title,
            content,
        })
    }

    fn start_main_tab_drag(&mut self, tab_id: &str, _: &MouseDownEvent, cx: &mut Context<Self>) {
        self.tab_drag.cancel();
        self.tab_drag.start(tab_id, "main");
        cx.notify();
    }

    fn finish_main_tab_drag(
        &mut self,
        event: &MouseUpEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab_id) = self.tab_drag.active_tab_id().map(str::to_string) else {
            return;
        };
        let bounds = window.window_bounds().get_bounds();
        let screen_x = (f32::from(bounds.origin.x) + f32::from(event.position.x)) as i32;
        let screen_y = (f32::from(bounds.origin.y) + f32::from(event.position.y)) as i32;
        let Some(target) =
            self.tab_drag
                .finish(screen_x, screen_y, &self.window_registry.bounds_snapshot())
        else {
            return;
        };
        match target {
            DragDropTarget::SameWindow => {}
            DragDropTarget::OtherWindow(target_window) => {
                let Some(tab) = self.detached_tab(&tab_id, cx) else {
                    return;
                };
                let Some(handle_id) = self.window_registry.handle_for(&target_window) else {
                    return;
                };
                let handle = WindowHandle::<DetachedSessionWindow>::new(handle_id);
                if handle
                    .update(cx, |target, _, cx| target.add_tab(tab, cx))
                    .is_ok()
                {
                    self.window_registry.detach_tab(&tab_id, &target_window);
                    let registry = self.window_registry.clone();
                    self.update_state(cx, |state| {
                        state.active_tab_id = state
                            .tabs
                            .iter()
                            .rev()
                            .find(|tab| registry.window_for_tab(&tab.id).is_none())
                            .map(|tab| tab.id.clone())
                            .unwrap_or_else(|| "overview".to_string());
                    });
                }
            }
            DragDropTarget::NewWindow => self.detach_session_tab(&tab_id, cx),
        }
    }

    fn detach_session_tab(&mut self, tab_id: &str, cx: &mut Context<Self>) {
        if self.window_registry.window_for_tab(tab_id).is_some() {
            return;
        }
        let Some(detached_tab) = self.detached_tab(tab_id, cx) else {
            return;
        };
        let DetachedSessionTab {
            tab_id: detached_tab_id,
            title,
            content,
        } = detached_tab;
        let result = detach_tab_to_new_window(&self.window_registry, tab_id);
        let registry_window_id = result.new_window_id.clone();
        let detached_view_window_id = registry_window_id.clone();
        let registry = self.window_registry.clone();
        let registry_for_view = registry.clone();
        let state = self.state.clone();
        let detached_title = title.clone();
        let bounds = Bounds::centered(None, size(px(1040.0), px(720.0)), cx);
        let open_result = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some(title.into()),
                    appears_transparent: cfg!(target_os = "macos"),
                    ..Default::default()
                }),
                window_decorations: Some(WindowDecorations::Server),
                kind: GpuiWindowKind::Normal,
                ..Default::default()
            },
            move |window, cx| {
                let view = cx.new(|cx| {
                    DetachedSessionWindow::new(
                        detached_view_window_id.clone(),
                        detached_tab_id,
                        detached_title,
                        state,
                        registry_for_view.clone(),
                        content,
                        cx,
                    )
                });
                view.focus_handle(cx).focus(window, cx);
                view
            },
        );

        match open_result {
            Ok(handle) => {
                registry.register_handle(&registry_window_id, handle.window_id());
                let registry = self.window_registry.clone();
                self.update_state(cx, |state| {
                    if state.active_tab_id == tab_id {
                        state.active_tab_id = state
                            .tabs
                            .iter()
                            .rev()
                            .find(|tab| {
                                tab.id != tab_id && registry.window_for_tab(&tab.id).is_none()
                            })
                            .map(|tab| tab.id.clone())
                            .unwrap_or_else(|| "overview".to_string());
                    }
                });
            }
            Err(error) => {
                self.window_registry
                    .return_tabs_to_main(&registry_window_id);
                self.update_state(cx, |state| state.data_error = Some(error.to_string()));
            }
        }
    }

    fn reload_connection_library(&mut self, cx: &mut Context<Self>) {
        self.update_state(cx, |state| {
            state.data_load_state = DataLoadState::Loading;
            state.data_error = None;
        });
        let state = self.state.downgrade();
        let api = self.api.clone();
        cx.spawn(async move |_, cx| {
            let result = api.app_get_connection_library().await;
            let _ = state.update(cx, |state, cx| {
                match result {
                    Ok(library) => state.apply_connection_library(library),
                    Err(error) => state.fail_data_load(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn reload_command_library(&mut self, cx: &mut Context<Self>) {
        let state = self.state.downgrade();
        let api = self.api.clone();
        cx.spawn(async move |_, cx| {
            let result = api.app_get_command_library().await;
            let _ = state.update(cx, |state, cx| {
                match result {
                    Ok(library) => state.apply_command_library(library),
                    Err(error) => state.data_error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn edit_command(&mut self, command_id: Option<String>, input: String, cx: &mut Context<Self>) {
        self.pending_command_editor = Some(PendingCommandEditor { command_id, input });
        cx.notify();
    }

    fn save_command(&mut self, cx: &mut Context<Self>) {
        let Some(pending) = self.pending_command_editor.take() else {
            return;
        };
        let api = self.api.clone();
        cx.spawn(async move |this, cx| {
            let result = api
                .app_save_command_template(pending.command_id, pending.input)
                .await;
            let _ = this.update(cx, |root, cx| {
                match result {
                    Ok(_) => root.reload_command_library(cx),
                    Err(error) => {
                        root.update_state(cx, |state| state.data_error = Some(error.to_string()))
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn request_delete_command(
        &mut self,
        command_id: String,
        command_name: String,
        cx: &mut Context<Self>,
    ) {
        self.pending_command_delete = Some((command_id, command_name));
        cx.notify();
    }

    fn delete_command(&mut self, command_id: String, cx: &mut Context<Self>) {
        self.pending_command_delete = None;
        let api = self.api.clone();
        cx.spawn(async move |this, cx| {
            let result = api.app_delete_command_template(command_id).await;
            let _ = this.update(cx, |root, cx| {
                match result {
                    Ok(()) => root.reload_command_library(cx),
                    Err(error) => {
                        root.update_state(cx, |state| state.data_error = Some(error.to_string()))
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn execute_command(
        &mut self,
        command: String,
        append_carriage_return: bool,
        cx: &mut Context<Self>,
    ) {
        let terminal_tab_id = self.state.read(cx).last_terminal_tab_id.clone();
        let result = if let Some(tab_id) = terminal_tab_id.as_deref() {
            if let Some(session) = self.sessions.get(tab_id) {
                session
                    .read(cx)
                    .send_command(&command, append_carriage_return)
            } else if let Some(session) = self.local_sessions.get(tab_id) {
                session
                    .read(cx)
                    .send_command(&command, append_carriage_return)
            } else if let Some(session) = self.stream_sessions.get(tab_id) {
                session
                    .read(cx)
                    .send_command(&command, append_carriage_return)
            } else {
                Err(anyhow::anyhow!("终端会话已关闭，请重新打开终端"))
            }
        } else {
            Err(anyhow::anyhow!("请先打开一个终端标签"))
        };
        if let Err(error) = result {
            self.update_state(cx, |state| state.data_error = Some(error.to_string()));
        }
    }

    fn handle_command_editor_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        let Some(pending) = self.pending_command_editor.as_mut() else {
            return;
        };
        match event.keystroke.key.as_str() {
            "escape" => {
                self.pending_command_editor = None;
                cx.notify();
            }
            "enter" | "return" => self.save_command(cx),
            "backspace" => {
                pending.input.pop();
                cx.notify();
            }
            _ if !event.keystroke.modifiers.control && !event.keystroke.modifiers.platform => {
                if let Some(text) = event.keystroke.key_char.as_deref() {
                    pending.input.push_str(text);
                    cx.notify();
                }
            }
            _ => {}
        }
    }

    fn handle_root_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.pending_connection_editor.is_some() {
            self.handle_connection_editor_key(event, cx);
        } else if self.pending_ssh_key_editor.is_some() {
            self.handle_ssh_key_editor_key(event, cx);
        } else if self.pending_webdav_editor.is_some() {
            self.handle_webdav_editor_key(event, cx);
        } else if self.pending_authentication.is_some() {
            self.handle_authentication_key(event, window, cx);
        } else if self.pending_command_editor.is_some() {
            self.handle_command_editor_key(event, cx);
        } else if self.pending_command_delete.is_some() && event.keystroke.key == "escape" {
            self.pending_command_delete = None;
            cx.notify();
        } else if self.pending_ssh_key_delete.is_some() && event.keystroke.key == "escape" {
            self.pending_ssh_key_delete = None;
            cx.notify();
        } else if self.ssh_key_search_focused {
            match event.keystroke.key.as_str() {
                "escape" => self.ssh_key_search_focused = false,
                "backspace" => {
                    self.ssh_key_query.pop();
                }
                _ if !event.keystroke.modifiers.control && !event.keystroke.modifiers.platform => {
                    if let Some(text) = event.keystroke.key_char.as_deref() {
                        self.ssh_key_query.push_str(text);
                    }
                }
                _ => {}
            }
            cx.notify();
        }
    }

    fn open_stream_profile(
        &mut self,
        profile_id: String,
        protocol: String,
        title: String,
        cx: &mut Context<Self>,
    ) {
        let tab_id = format!("{protocol}:{profile_id}");
        if self.stream_sessions.contains_key(&tab_id) {
            self.update_state(cx, |state| state.activate_tab(&tab_id));
            return;
        }
        self.update_state(cx, |state| state.open_session_tab(tab_id.clone(), title));
        let api = self.api.clone();
        cx.spawn(async move |this, cx| {
            let result = api.stream_connect(&profile_id).await;
            let _ = this.update(cx, |root, cx| match result {
                Ok(session) => {
                    let workspace = cx.new(|cx| {
                        StreamSessionWorkspace::new(
                            tab_id.clone(),
                            profile_id.clone(),
                            root.api.clone(),
                            session,
                            root.state.clone(),
                            cx,
                        )
                    });
                    root.stream_sessions.insert(tab_id.clone(), workspace);
                    root.update_state(cx, |state| {
                        state.set_tab_status(&tab_id, TabStatus::Connected)
                    });
                }
                Err(error) => root.update_state(cx, |state| {
                    state.set_tab_status(&tab_id, TabStatus::Error);
                    state.data_error = Some(error.to_string());
                }),
            });
        })
        .detach();
    }

    fn open_ftp_profile(&mut self, profile_id: String, title: String, cx: &mut Context<Self>) {
        let tab_id = format!("ftp:{profile_id}");
        if self.ftp_sessions.contains_key(&tab_id) {
            self.update_state(cx, |state| state.activate_tab(&tab_id));
            return;
        }
        self.update_state(cx, |state| state.open_session_tab(tab_id.clone(), title));
        let api = self.api.clone();
        cx.spawn(async move |this, cx| {
            let result = api.ftp_connect(&profile_id).await;
            let _ = this.update(cx, |root, cx| match result {
                Ok(connected) => {
                    let workspace = cx.new(|cx| {
                        FtpWorkspace::new(
                            tab_id.clone(),
                            connected.session,
                            connected.remote_path,
                            connected.transfer_journal_path,
                            root.state.clone(),
                            cx,
                        )
                    });
                    root.ftp_sessions.insert(tab_id.clone(), workspace);
                    root.update_state(cx, |state| {
                        state.set_tab_status(&tab_id, TabStatus::Connected)
                    });
                }
                Err(error) => root.update_state(cx, |state| {
                    state.set_tab_status(&tab_id, TabStatus::Error);
                    state.data_error = Some(error.to_string());
                }),
            });
        })
        .detach();
    }

    fn open_ssh_profile(&mut self, profile_id: String, title: String, cx: &mut Context<Self>) {
        self.connect_ssh_profile(profile_id, title, SshConnectOptions::default(), cx);
    }

    fn connect_ssh_profile(
        &mut self,
        profile_id: String,
        title: String,
        options: SshConnectOptions,
        cx: &mut Context<Self>,
    ) {
        let tab_id = format!("ssh:{profile_id}");
        if self.sessions.contains_key(&tab_id) {
            self.update_state(cx, |state| state.activate_tab(&tab_id));
            return;
        }

        self.update_state(cx, |state| {
            state.open_session_tab(tab_id.clone(), title.clone())
        });
        let api = self.api.clone();
        let retry_options = options.clone();
        cx.spawn(async move |this, cx| {
            let result = api.ssh_connect(&profile_id, 80, 24, options).await;
            let _ = this.update(cx, |root, cx| match result {
                Ok(session) => {
                    let workspace = cx.new(|cx| {
                        SessionWorkspace::new(
                            tab_id.clone(),
                            session.controller,
                            session.output,
                            session.transfer_journal_path,
                            root.state.clone(),
                            cx,
                        )
                    });
                    root.sessions.insert(tab_id.clone(), workspace);
                    root.update_state(cx, |state| {
                        state.set_tab_status(&tab_id, TabStatus::Connected)
                    });
                }
                Err(crate::error::AppError::SshHostVerification {
                    host,
                    port,
                    fingerprint,
                    changed,
                }) => {
                    root.pending_host_verification = Some(PendingHostVerification {
                        profile_id: profile_id.clone(),
                        title: title.clone(),
                        host,
                        port,
                        fingerprint,
                        changed,
                        options: retry_options.clone(),
                    });
                    root.update_state(cx, |state| {
                        state.set_tab_status(&tab_id, TabStatus::Error);
                        state.data_error = None;
                    });
                    cx.notify();
                }
                Err(crate::error::AppError::SshAuthenticationRequired { prompts }) => {
                    let mut options = retry_options.clone();
                    for prompt in &prompts {
                        match prompt.kind {
                            crate::error::SshAuthenticationPromptKind::Password => {
                                options.transient_password = None;
                            }
                            crate::error::SshAuthenticationPromptKind::PrivateKeyPassphrase => {
                                options.transient_passphrase = None;
                            }
                            crate::error::SshAuthenticationPromptKind::KeyboardInteractive => {
                                options.keyboard_interactive_answers.clear();
                            }
                        }
                    }
                    root.pending_authentication = Some(PendingAuthentication {
                        profile_id: profile_id.clone(),
                        title: title.clone(),
                        prompts,
                        answers: Vec::new(),
                        input: String::new(),
                        options,
                    });
                    root.update_state(cx, |state| {
                        state.set_tab_status(&tab_id, TabStatus::Connecting);
                        state.data_error = None;
                    });
                    cx.notify();
                }
                Err(error) => {
                    root.update_state(cx, |state| {
                        state.set_tab_status(&tab_id, TabStatus::Error);
                        state.data_error = Some(error.to_string());
                    });
                }
            });
        })
        .detach();
    }

    fn close_tab(&mut self, tab_id: &str, cx: &mut Context<Self>) {
        if let Some(session) = self.sessions.remove(tab_id) {
            session.update(cx, |workspace, _| workspace.close());
        }
        if let Some(session) = self.ftp_sessions.remove(tab_id) {
            session.update(cx, |workspace, cx| workspace.close(cx));
        }
        if let Some(session) = self.local_sessions.remove(tab_id) {
            session.update(cx, |workspace, _| workspace.close());
        }
        if let Some(session) = self.stream_sessions.remove(tab_id) {
            session.update(cx, |workspace, _| workspace.close());
        }
        if self
            .pending_authentication
            .as_ref()
            .is_some_and(|pending| format!("ssh:{}", pending.profile_id) == tab_id)
        {
            self.pending_authentication = None;
        }
        if self
            .pending_host_verification
            .as_ref()
            .is_some_and(|pending| format!("ssh:{}", pending.profile_id) == tab_id)
        {
            self.pending_host_verification = None;
        }
        self.update_state(cx, |state| state.close_tab(tab_id));
    }

    fn accept_host_key(&mut self, save: bool, cx: &mut Context<Self>) {
        let Some(pending) = self.pending_host_verification.take() else {
            return;
        };
        let mut options = pending.options;
        options.accepted_host_fingerprint = Some(pending.fingerprint);
        options.save_host_fingerprint = save;
        self.connect_ssh_profile(pending.profile_id, pending.title, options, cx);
    }

    fn reject_host_key(&mut self, cx: &mut Context<Self>) {
        if let Some(pending) = self.pending_host_verification.take() {
            self.close_tab(&format!("ssh:{}", pending.profile_id), cx);
        }
        cx.notify();
    }

    fn cancel_authentication(&mut self, cx: &mut Context<Self>) {
        if let Some(pending) = self.pending_authentication.take() {
            self.close_tab(&format!("ssh:{}", pending.profile_id), cx);
        }
        cx.notify();
    }

    fn advance_authentication(&mut self, cx: &mut Context<Self>) {
        let Some(pending) = self.pending_authentication.as_mut() else {
            return;
        };
        pending.answers.push(std::mem::take(&mut pending.input));
        if pending.answers.len() < pending.prompts.len() {
            cx.notify();
            return;
        }

        let mut pending = self
            .pending_authentication
            .take()
            .expect("authentication pending");
        let prompts = pending.prompts.clone();
        let mut answers = std::mem::take(&mut pending.answers);
        let mut options = std::mem::take(&mut pending.options);
        for (prompt, answer) in prompts.iter().zip(answers.drain(..)) {
            match prompt.kind {
                crate::error::SshAuthenticationPromptKind::Password => {
                    options.transient_password = Some(answer);
                }
                crate::error::SshAuthenticationPromptKind::PrivateKeyPassphrase => {
                    options.transient_passphrase = Some(answer);
                }
                crate::error::SshAuthenticationPromptKind::KeyboardInteractive => {
                    options.keyboard_interactive_answers.push(answer);
                }
            }
        }
        options.authentication_attempts = options.authentication_attempts.saturating_add(1);
        let profile_id = std::mem::take(&mut pending.profile_id);
        let title = std::mem::take(&mut pending.title);
        self.connect_ssh_profile(profile_id, title, options, cx);
    }

    fn handle_authentication_key(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(pending) = self.pending_authentication.as_mut() else {
            return;
        };
        match event.keystroke.key.as_str() {
            "escape" => self.cancel_authentication(cx),
            "enter" | "return" => self.advance_authentication(cx),
            "backspace" => {
                pending.input.pop();
                cx.notify();
            }
            _ if !event.keystroke.modifiers.control && !event.keystroke.modifiers.platform => {
                if let Some(text) = event.keystroke.key_char.as_deref() {
                    pending.input.push_str(text);
                    cx.notify();
                }
            }
            _ => {}
        }
    }

    fn render_authentication(
        &self,
        pending: &PendingAuthentication,
        palette: ThemePalette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let prompt_index = pending
            .answers
            .len()
            .min(pending.prompts.len().saturating_sub(1));
        let prompt = pending.prompts.get(prompt_index);
        let value = match prompt {
            Some(prompt) if prompt.echo => pending.input.clone(),
            Some(_) => "•".repeat(pending.input.chars().count()),
            None => String::new(),
        };

        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(gpui::black().opacity(0.65))
            .child(
                div()
                    .w(px(480.0))
                    .flex()
                    .flex_col()
                    .gap_3()
                    .p_5()
                    .rounded_lg()
                    .bg(palette.surface)
                    .border_1()
                    .border_color(palette.border_strong)
                    .child(
                        div()
                            .text_lg()
                            .text_color(palette.text)
                            .child("SSH 身份验证"),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(palette.text_muted)
                            .child(format!(
                                "步骤 {} / {} · 凭据仅用于本次连接",
                                prompt_index + 1,
                                pending.prompts.len()
                            )),
                    )
                    .child(
                        div().text_sm().text_color(palette.text).child(
                            prompt
                                .map(|prompt| prompt.label.clone())
                                .unwrap_or_default(),
                        ),
                    )
                    .child(
                        div()
                            .h(px(40.0))
                            .px_3()
                            .flex()
                            .items_center()
                            .rounded_md()
                            .bg(palette.background)
                            .border_1()
                            .border_color(palette.accent)
                            .text_color(palette.text)
                            .child(if value.is_empty() {
                                "输入后按 Enter".to_string()
                            } else {
                                value
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap_2()
                            .child(
                                div()
                                    .id("cancel-ssh-authentication")
                                    .px_3()
                                    .py_2()
                                    .rounded_md()
                                    .cursor_pointer()
                                    .text_sm()
                                    .text_color(palette.text_muted)
                                    .hover(move |style| style.bg(palette.surface_hover))
                                    .on_click(
                                        cx.listener(|this, _, _, cx| {
                                            this.cancel_authentication(cx)
                                        }),
                                    )
                                    .child("取消"),
                            )
                            .child(
                                div()
                                    .id("submit-ssh-authentication")
                                    .px_3()
                                    .py_2()
                                    .rounded_md()
                                    .cursor_pointer()
                                    .bg(palette.accent)
                                    .text_sm()
                                    .text_color(palette.background)
                                    .on_click(
                                        cx.listener(|this, _, _, cx| {
                                            this.advance_authentication(cx)
                                        }),
                                    )
                                    .child(if prompt_index + 1 == pending.prompts.len() {
                                        "连接"
                                    } else {
                                        "下一步"
                                    }),
                            ),
                    ),
            )
    }

    fn render_host_verification(
        &self,
        pending: &PendingHostVerification,
        palette: ThemePalette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(gpui::black().opacity(0.65))
            .child(
                div()
                    .w(px(520.0))
                    .flex()
                    .flex_col()
                    .gap_3()
                    .p_5()
                    .rounded_lg()
                    .bg(palette.surface)
                    .border_1()
                    .border_color(if pending.changed {
                        palette.danger
                    } else {
                        palette.border_strong
                    })
                    .child(
                        div()
                            .text_lg()
                            .text_color(palette.text)
                            .child(if pending.changed {
                                "远端主机密钥已变化"
                            } else {
                                "验证远端主机密钥"
                            }),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(palette.text_muted)
                            .child(format!("{}:{}", pending.host, pending.port)),
                    )
                    .child(
                        div()
                            .p_3()
                            .rounded_md()
                            .bg(palette.background)
                            .text_xs()
                            .text_color(palette.text)
                            .child(pending.fingerprint.clone()),
                    )
                    .when(pending.changed, |view| {
                        view.child(div().text_sm().text_color(palette.danger).child(
                            "保存的指纹与当前服务器不一致。继续前请确认服务器密钥确实已更换。",
                        ))
                    })
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap_2()
                            .child(
                                div()
                                    .id("reject-host-key")
                                    .px_3()
                                    .py_2()
                                    .rounded_md()
                                    .cursor_pointer()
                                    .text_sm()
                                    .text_color(palette.text_muted)
                                    .hover(move |style| style.bg(palette.surface_hover))
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.reject_host_key(cx)),
                                    )
                                    .child("拒绝"),
                            )
                            .child(
                                div()
                                    .id("accept-host-key-once")
                                    .px_3()
                                    .py_2()
                                    .rounded_md()
                                    .cursor_pointer()
                                    .text_sm()
                                    .text_color(palette.accent)
                                    .hover(move |style| style.bg(palette.accent_surface))
                                    .on_click(
                                        cx.listener(|this, _, _, cx| {
                                            this.accept_host_key(false, cx)
                                        }),
                                    )
                                    .child("仅本次接受"),
                            )
                            .child(
                                div()
                                    .id("accept-host-key-save")
                                    .px_3()
                                    .py_2()
                                    .rounded_md()
                                    .cursor_pointer()
                                    .bg(palette.accent)
                                    .text_sm()
                                    .text_color(palette.background)
                                    .on_click(
                                        cx.listener(|this, _, _, cx| {
                                            this.accept_host_key(true, cx)
                                        }),
                                    )
                                    .child("接受并保存"),
                            ),
                    ),
            )
    }

    fn render_command_editor(
        &self,
        pending: PendingCommandEditor,
        palette: ThemePalette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(gpui::black().opacity(0.65))
            .child(
                div()
                    .w(px(560.0))
                    .flex()
                    .flex_col()
                    .gap_3()
                    .p_5()
                    .rounded_lg()
                    .bg(palette.surface)
                    .border_1()
                    .border_color(palette.border_strong)
                    .child(div().text_lg().text_color(palette.text).child(
                        if pending.command_id.is_some() {
                            "编辑命令"
                        } else {
                            "新建命令"
                        },
                    ))
                    .child(
                        div()
                            .text_xs()
                            .text_color(palette.text_muted)
                            .child("输入完整命令，Enter 保存，Esc 取消。"),
                    )
                    .child(
                        div()
                            .min_h(px(72.0))
                            .p_3()
                            .rounded_md()
                            .bg(palette.background)
                            .border_1()
                            .border_color(palette.accent)
                            .text_sm()
                            .text_color(palette.text)
                            .child(if pending.input.is_empty() {
                                "例如：journalctl -u nginx -n 100".to_string()
                            } else {
                                pending.input
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap_2()
                            .child(
                                div()
                                    .id("cancel-command-editor")
                                    .px_3()
                                    .py_2()
                                    .rounded_md()
                                    .cursor_pointer()
                                    .text_sm()
                                    .text_color(palette.text_muted)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.pending_command_editor = None;
                                        cx.notify();
                                    }))
                                    .child("取消"),
                            )
                            .child(
                                div()
                                    .id("save-command-editor")
                                    .px_3()
                                    .py_2()
                                    .rounded_md()
                                    .cursor_pointer()
                                    .bg(palette.accent)
                                    .text_sm()
                                    .text_color(palette.background)
                                    .on_click(cx.listener(|this, _, _, cx| this.save_command(cx)))
                                    .child("保存"),
                            ),
                    ),
            )
    }

    fn render_command_delete_confirmation(
        &self,
        command_id: String,
        command_name: String,
        palette: ThemePalette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(gpui::black().opacity(0.65))
            .child(
                div()
                    .w(px(440.0))
                    .flex()
                    .flex_col()
                    .gap_3()
                    .p_5()
                    .rounded_lg()
                    .bg(palette.surface)
                    .border_1()
                    .border_color(palette.border_strong)
                    .child(div().text_lg().text_color(palette.text).child("删除命令"))
                    .child(
                        div()
                            .text_sm()
                            .text_color(palette.text_muted)
                            .child(format!("确定删除“{command_name}”吗？此操作无法撤销。")),
                    )
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap_2()
                            .child(
                                div()
                                    .id("cancel-command-delete")
                                    .px_3()
                                    .py_2()
                                    .rounded_md()
                                    .cursor_pointer()
                                    .text_sm()
                                    .text_color(palette.text_muted)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.pending_command_delete = None;
                                        cx.notify();
                                    }))
                                    .child("取消"),
                            )
                            .child(
                                div()
                                    .id("confirm-command-delete")
                                    .px_3()
                                    .py_2()
                                    .rounded_md()
                                    .cursor_pointer()
                                    .bg(palette.danger)
                                    .text_sm()
                                    .text_color(palette.background)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.delete_command(command_id.clone(), cx)
                                    }))
                                    .child("删除"),
                            ),
                    ),
            )
    }

    fn render_sidebar(
        &self,
        state: &AppState,
        palette: ThemePalette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let collapsed = state.sidebar_collapsed || state.workspace_focus;
        div()
            .w(px(if collapsed { 64.0 } else { 220.0 }))
            .h_full()
            .flex()
            .flex_col()
            .flex_shrink_0()
            .bg(palette.sidebar)
            .border_r_1()
            .border_color(palette.border)
            .child(
                div()
                    .h(px(58.0))
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_4()
                    .border_b_1()
                    .border_color(palette.border)
                    .child(
                        div()
                            .size(px(28.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_md()
                            .bg(palette.accent_surface)
                            .text_color(palette.accent)
                            .text_sm()
                            .child("FT"),
                    )
                    .when(!collapsed, |view| {
                        view.child(
                            div()
                                .flex()
                                .flex_col()
                                .child(div().text_sm().text_color(palette.text).child("FileTerm"))
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(palette.text_soft)
                                        .child("Remote Workstation"),
                                ),
                        )
                    }),
            )
            .child(
                div().flex_1().flex().flex_col().gap_1().p_2().children(
                    NavigationSection::ALL
                        .into_iter()
                        .enumerate()
                        .map(|(index, section)| {
                            let active = state.navigation == section;
                            div()
                                .id(("nav", index))
                                .h(px(40.0))
                                .flex()
                                .items_center()
                                .gap_3()
                                .px_3()
                                .rounded_md()
                                .cursor_pointer()
                                .bg(if active {
                                    palette.surface_active
                                } else {
                                    palette.sidebar
                                })
                                .hover(move |style| style.bg(palette.surface_hover))
                                .text_color(if active {
                                    palette.text
                                } else {
                                    palette.text_muted
                                })
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.select_navigation(section, cx);
                                }))
                                .child(
                                    div()
                                        .w(px(28.0))
                                        .text_center()
                                        .text_xs()
                                        .text_color(if active {
                                            palette.accent
                                        } else {
                                            palette.text_soft
                                        })
                                        .child(section.glyph()),
                                )
                                .when(!collapsed, |view| view.child(section.label()))
                        }),
                ),
            )
            .child(
                div()
                    .h(px(48.0))
                    .flex()
                    .items_center()
                    .px_3()
                    .border_t_1()
                    .border_color(palette.border)
                    .child(
                        div()
                            .id("toggle-sidebar")
                            .w_full()
                            .h(px(32.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_md()
                            .cursor_pointer()
                            .text_color(palette.text_muted)
                            .hover(move |style| style.bg(palette.surface_hover))
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_sidebar(cx)))
                            .child(if collapsed { ">" } else { "收起侧栏" }),
                    ),
            )
    }

    fn render_tabbar(
        &self,
        state: &AppState,
        palette: ThemePalette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .h(px(48.0))
            .w_full()
            .flex()
            .items_end()
            .justify_between()
            .bg(palette.surface)
            .border_b_1()
            .border_color(palette.border)
            .child(
                div().h_full().flex().items_end().gap_1().px_2().children(
                    state
                        .tabs
                        .iter()
                        .filter(|tab| self.window_registry.window_for_tab(&tab.id).is_none())
                        .enumerate()
                        .map(|(index, tab)| {
                            let tab_id = tab.id.clone();
                            let close_id = tab.id.clone();
                            let detach_id = tab.id.clone();
                            let drag_id = tab.id.clone();
                            let detachable = tab.id.starts_with("ssh:")
                                || tab.id.starts_with("ftp:")
                                || tab.id.starts_with("local:")
                                || tab.id.starts_with("telnet:")
                                || tab.id.starts_with("serial:");
                            let active = state.active_tab_id == tab.id;
                            let status_color = match tab.status {
                                TabStatus::Connected => palette.success,
                                TabStatus::Connecting => palette.warning,
                                TabStatus::Error => palette.danger,
                                TabStatus::Idle | TabStatus::Closed => palette.text_soft,
                            };
                            div()
                                .id(("tab", index))
                                .h(px(38.0))
                                .min_w(px(120.0))
                                .max_w(px(220.0))
                                .flex()
                                .items_center()
                                .gap_2()
                                .px_3()
                                .rounded_t_md()
                                .cursor_pointer()
                                .bg(if active {
                                    palette.background
                                } else {
                                    palette.surface
                                })
                                .border_1()
                                .border_b_0()
                                .border_color(if active {
                                    palette.border_strong
                                } else {
                                    palette.border
                                })
                                .text_color(if active {
                                    palette.text
                                } else {
                                    palette.text_muted
                                })
                                .hover(move |style| style.bg(palette.surface_hover))
                                .when(detachable, |view| {
                                    view.on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(move |this, event, _, cx| {
                                            this.start_main_tab_drag(&drag_id, event, cx)
                                        }),
                                    )
                                })
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.update_state(cx, |state| state.activate_tab(&tab_id));
                                }))
                                .child(div().size(px(7.0)).rounded_full().bg(status_color))
                                .child(div().flex_1().truncate().text_sm().child(tab.title.clone()))
                                .when(detachable, |view| {
                                    view.child(
                                        div()
                                            .id(("detach-tab", index))
                                            .size(px(22.0))
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .rounded_sm()
                                            .text_xs()
                                            .text_color(palette.text_soft)
                                            .hover(move |style| style.bg(palette.surface_active))
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.detach_session_tab(&detach_id, cx);
                                            }))
                                            .child("↗"),
                                    )
                                })
                                .when(tab.id != "overview", |view| {
                                    view.child(
                                        div()
                                            .id(("close-tab", index))
                                            .size(px(22.0))
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .rounded_sm()
                                            .hover(move |style| style.bg(palette.surface_active))
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.close_tab(&close_id, cx);
                                            }))
                                            .child("×"),
                                    )
                                })
                        }),
                ),
            )
            .child(
                div()
                    .h_full()
                    .flex()
                    .items_center()
                    .gap_1()
                    .px_3()
                    .child(
                        div()
                            .id("new-local-terminal")
                            .px_3()
                            .py_1()
                            .rounded_md()
                            .cursor_pointer()
                            .text_xs()
                            .text_color(palette.accent)
                            .hover(move |style| style.bg(palette.accent_surface))
                            .on_click(cx.listener(|this, _, _, cx| this.open_local_terminal(cx)))
                            .child("+ 本地终端"),
                    )
                    .child(
                        div()
                            .id("focus-mode")
                            .px_3()
                            .py_1()
                            .rounded_md()
                            .cursor_pointer()
                            .text_xs()
                            .text_color(if state.workspace_focus {
                                palette.accent
                            } else {
                                palette.text_muted
                            })
                            .hover(move |style| style.bg(palette.surface_hover))
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_focus_mode(cx)))
                            .child(if state.workspace_focus {
                                "退出专注"
                            } else {
                                "专注模式"
                            }),
                    )
                    .child(
                        div()
                            .id("theme-toggle")
                            .px_3()
                            .py_1()
                            .rounded_md()
                            .cursor_pointer()
                            .text_xs()
                            .text_color(palette.text_muted)
                            .hover(move |style| style.bg(palette.surface_hover))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.toggle_theme(&ToggleTheme, window, cx)
                            }))
                            .child(match state.theme {
                                ThemeMode::Dark => "浅色",
                                ThemeMode::Light => "深色",
                            }),
                    ),
            )
    }

    fn render_connection_library_content(
        &self,
        state: &AppState,
        palette: ThemePalette,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        match state.data_load_state {
            DataLoadState::Loading => div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_sm()
                .text_color(palette.text_muted)
                .child("正在读取连接库…")
                .into_any_element(),
            DataLoadState::Error => div()
                .size_full()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_3()
                .child(
                    div().text_sm().text_color(palette.danger).child(
                        state
                            .data_error
                            .clone()
                            .unwrap_or_else(|| "连接库读取失败".to_string()),
                    ),
                )
                .child(
                    div()
                        .id("retry-connection-library")
                        .px_3()
                        .py_2()
                        .rounded_md()
                        .cursor_pointer()
                        .bg(palette.accent_surface)
                        .text_sm()
                        .text_color(palette.accent)
                        .on_click(cx.listener(|this, _, _, cx| this.reload_connection_library(cx)))
                        .child("重新读取"),
                )
                .into_any_element(),
            DataLoadState::Ready if state.connections.is_empty() => div()
                .size_full()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_3()
                .child(div().text_lg().text_color(palette.text).child("连接库为空"))
                .child(
                    div()
                        .text_sm()
                        .text_color(palette.text_muted)
                        .child("创建第一个 SSH、FTP、Telnet 或 Serial 连接。"),
                )
                .into_any_element(),
            DataLoadState::Ready => div()
                .size_full()
                .flex()
                .flex_col()
                .children(state.connections.iter().take(8).enumerate().map(
                    |(index, connection)| {
                        let profile_id = connection.id.clone();
                        let title = connection.name.clone();
                        let protocol = connection.protocol.clone();
                        let connectable =
                            matches!(protocol.as_str(), "ssh" | "ftp" | "telnet" | "serial");
                        div()
                            .id(("connection-summary", index))
                            .when(connectable, |view| {
                                view.cursor_pointer().on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        if protocol == "ssh" {
                                            this.open_ssh_profile(
                                                profile_id.clone(),
                                                title.clone(),
                                                cx,
                                            );
                                        } else if protocol == "ftp" {
                                            this.open_ftp_profile(
                                                profile_id.clone(),
                                                title.clone(),
                                                cx,
                                            );
                                        } else {
                                            this.open_stream_profile(
                                                profile_id.clone(),
                                                protocol.clone(),
                                                title.clone(),
                                                cx,
                                            );
                                        }
                                    },
                                ))
                            })
                            .h(px(52.0))
                            .flex()
                            .items_center()
                            .gap_3()
                            .px_4()
                            .border_b_1()
                            .border_color(palette.border)
                            .child(
                                div()
                                    .w(px(54.0))
                                    .text_xs()
                                    .text_color(palette.accent)
                                    .child(connection.protocol.to_uppercase()),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.0))
                                    .flex()
                                    .flex_col()
                                    .child(
                                        div()
                                            .truncate()
                                            .text_sm()
                                            .text_color(palette.text)
                                            .child(connection.name.clone()),
                                    )
                                    .child(
                                        div()
                                            .truncate()
                                            .text_xs()
                                            .text_color(palette.text_soft)
                                            .child(connection.endpoint.clone()),
                                    ),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(palette.text_muted)
                                    .child(connection.group.clone()),
                            )
                    },
                ))
                .into_any_element(),
        }
    }

    fn render_overview(
        &self,
        state: &AppState,
        palette: ThemePalette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let connection_count = state.connections.len().to_string();
        let active_session_count = state
            .tabs
            .iter()
            .filter(|tab| matches!(tab.status, TabStatus::Connecting | TabStatus::Connected))
            .count()
            .to_string();
        div()
            .size_full()
            .flex()
            .flex_col()
            .gap_5()
            .p_6()
            .child(
                div()
                    .flex()
                    .justify_between()
                    .items_end()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(
                                div()
                                    .text_2xl()
                                    .text_color(palette.text)
                                    .child("远程工作台"),
                            )
                            .child(
                                div().text_sm().text_color(palette.text_muted).child(
                                    "连接、终端、文件与传输将在同一个原生 GPU 工作区中协作。",
                                ),
                            ),
                    )
                    .child(
                        div()
                            .px_3()
                            .py_2()
                            .rounded_md()
                            .bg(palette.accent_surface)
                            .text_color(palette.accent)
                            .text_xs()
                            .child("GPUI Runtime"),
                    ),
            )
            .child(
                div().flex().gap_4().children(
                    [
                        ("连接", connection_count, "共享连接库"),
                        ("活动会话", active_session_count, "当前工作区"),
                        ("传输任务", "0".to_string(), "队列当前为空"),
                    ]
                    .into_iter()
                    .map(|(title, value, description)| {
                        div()
                            .flex_1()
                            .min_h(px(132.0))
                            .flex()
                            .flex_col()
                            .justify_between()
                            .p_4()
                            .rounded_lg()
                            .bg(palette.surface)
                            .border_1()
                            .border_color(palette.border)
                            .child(div().text_sm().text_color(palette.text_muted).child(title))
                            .child(div().text_3xl().text_color(palette.text).child(value))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(palette.text_soft)
                                    .child(description),
                            )
                    }),
                ),
            )
            .child(
                div()
                    .flex_1()
                    .overflow_hidden()
                    .rounded_lg()
                    .bg(palette.surface)
                    .border_1()
                    .border_color(palette.border)
                    .child(self.render_connection_library_content(state, palette, cx)),
            )
    }

    fn render_section(
        &self,
        state: &AppState,
        section: NavigationSection,
        palette: ThemePalette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        if section == NavigationSection::Overview {
            return self.render_overview(state, palette, cx).into_any_element();
        }

        if section == NavigationSection::Connections {
            return self
                .render_connection_manager(state, palette, cx)
                .into_any_element();
        }

        if section == NavigationSection::Commands {
            let terminal_target = state.last_terminal_tab_id.as_deref().and_then(|tab_id| {
                let available =
                    self.sessions.contains_key(tab_id) || self.local_sessions.contains_key(tab_id);
                available.then(|| {
                    state
                        .tabs
                        .iter()
                        .find(|tab| tab.id == tab_id)
                        .map(|tab| tab.title.clone())
                        .unwrap_or_else(|| "终端".to_string())
                })
            });
            let has_terminal_target = terminal_target.is_some();
            return div()
                .size_full()
                .flex()
                .flex_col()
                .gap_4()
                .p_6()
                .child(
                    div()
                        .flex()
                        .items_end()
                        .justify_between()
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap_2()
                                .child(
                                    div()
                                        .text_2xl()
                                        .text_color(palette.text)
                                        .child("命令管理器"),
                                )
                                .child(div().text_sm().text_color(palette.text_muted).child(
                                    format!(
                                            "{} 个命令，{} 个文件夹 · {}",
                                            state.commands.len(),
                                            state.command_folders.len(),
                                            terminal_target
                                                .as_deref()
                                                .map(|title| format!("发送到 {title}"))
                                                .unwrap_or_else(
                                                    || "未打开终端，命令运行已禁用".to_string()
                                                )
                                        ),
                                )),
                        )
                        .child(
                            div()
                                .id("new-command-template")
                                .px_3()
                                .py_2()
                                .rounded_md()
                                .cursor_pointer()
                                .bg(palette.accent)
                                .text_sm()
                                .text_color(palette.background)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.edit_command(None, String::new(), cx)
                                }))
                                .child("新建命令"),
                        ),
                )
                .child(
                    div()
                        .flex_1()
                        .overflow_hidden()
                        .rounded_lg()
                        .bg(palette.surface)
                        .border_1()
                        .border_color(palette.border)
                        .when(state.commands.is_empty(), |view| {
                            view.flex()
                                .items_center()
                                .justify_center()
                                .text_sm()
                                .text_color(palette.text_muted)
                                .child("命令库为空")
                        })
                        .when(!state.commands.is_empty(), |view| {
                            view.flex()
                                .flex_col()
                                .children(state.commands.iter().enumerate().map(
                                    |(index, command)| {
                                        let command_text = command.command.clone();
                                        let run_text = command.command.clone();
                                        let append = command.append_carriage_return;
                                        let edit_id = command.id.clone();
                                        let delete_id = command.id.clone();
                                        let delete_name = command.name.clone();
                                        div()
                                            .id(("command-template", index))
                                            .min_h(px(64.0))
                                            .flex()
                                            .items_center()
                                            .gap_3()
                                            .px_4()
                                            .border_b_1()
                                            .border_color(palette.border)
                                            .child(
                                                div()
                                                    .flex_1()
                                                    .min_w(px(0.0))
                                                    .flex()
                                                    .flex_col()
                                                    .gap_1()
                                                    .child(
                                                        div()
                                                            .truncate()
                                                            .text_sm()
                                                            .text_color(palette.text)
                                                            .child(command.name.clone()),
                                                    )
                                                    .child(
                                                        div()
                                                            .truncate()
                                                            .text_xs()
                                                            .text_color(palette.text_soft)
                                                            .child(command.command.clone()),
                                                    ),
                                            )
                                            .child(
                                                div()
                                                    .id(("run-command", index))
                                                    .px_3()
                                                    .py_1()
                                                    .rounded_md()
                                                    .bg(if has_terminal_target {
                                                        palette.accent_surface
                                                    } else {
                                                        palette.background
                                                    })
                                                    .text_xs()
                                                    .text_color(if has_terminal_target {
                                                        palette.accent
                                                    } else {
                                                        palette.text_soft
                                                    })
                                                    .when(has_terminal_target, |button| {
                                                        button.cursor_pointer().on_click(
                                                            cx.listener(move |this, _, _, cx| {
                                                                this.execute_command(
                                                                    run_text.clone(),
                                                                    append,
                                                                    cx,
                                                                )
                                                            }),
                                                        )
                                                    })
                                                    .child(if has_terminal_target {
                                                        "运行"
                                                    } else {
                                                        "无终端"
                                                    }),
                                            )
                                            .child(
                                                div()
                                                    .id(("edit-command", index))
                                                    .px_2()
                                                    .py_1()
                                                    .rounded_md()
                                                    .cursor_pointer()
                                                    .text_xs()
                                                    .text_color(palette.text_muted)
                                                    .on_click(cx.listener(move |this, _, _, cx| {
                                                        this.edit_command(
                                                            Some(edit_id.clone()),
                                                            command_text.clone(),
                                                            cx,
                                                        )
                                                    }))
                                                    .child("编辑"),
                                            )
                                            .child(
                                                div()
                                                    .id(("delete-command", index))
                                                    .px_2()
                                                    .py_1()
                                                    .rounded_md()
                                                    .cursor_pointer()
                                                    .text_xs()
                                                    .text_color(palette.danger)
                                                    .on_click(cx.listener(move |this, _, _, cx| {
                                                        this.request_delete_command(
                                                            delete_id.clone(),
                                                            delete_name.clone(),
                                                            cx,
                                                        )
                                                    }))
                                                    .child("删除"),
                                            )
                                    },
                                ))
                        }),
                )
                .into_any_element();
        }

        if section == NavigationSection::SshKeys {
            return self
                .render_ssh_key_manager(state, palette, cx)
                .into_any_element();
        }

        if section == NavigationSection::Settings {
            let dark = state.theme == ThemeMode::Dark;
            let english = state.locale == "enUS";
            return div()
                .size_full()
                .flex()
                .flex_col()
                .gap_4()
                .p_6()
                .child(div().text_2xl().text_color(palette.text).child("设置"))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_4()
                        .p_5()
                        .rounded_lg()
                        .bg(palette.surface)
                        .border_1()
                        .border_color(palette.border)
                        .child(div().text_sm().text_color(palette.text).child("外观主题"))
                        .child(
                            div().flex().gap_2().children(
                                [("深色", dark), ("浅色", !dark)]
                                    .into_iter()
                                    .enumerate()
                                    .map(|(index, (label, active))| {
                                        div()
                                            .id(("settings-theme", index))
                                            .px_4()
                                            .py_2()
                                            .rounded_md()
                                            .cursor_pointer()
                                            .bg(if active {
                                                palette.accent_surface
                                            } else {
                                                palette.background
                                            })
                                            .border_1()
                                            .border_color(if active {
                                                palette.accent
                                            } else {
                                                palette.border
                                            })
                                            .text_sm()
                                            .text_color(if active {
                                                palette.accent
                                            } else {
                                                palette.text_muted
                                            })
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                let requested = if index == 0 {
                                                    ThemeMode::Dark
                                                } else {
                                                    ThemeMode::Light
                                                };
                                                if this.state.read(cx).theme != requested {
                                                    this.update_state(cx, |state| {
                                                        state.theme = requested
                                                    });
                                                    let locale = this.state.read(cx).locale.clone();
                                                    this.persist_preferences(requested, locale, cx);
                                                }
                                            }))
                                            .child(label)
                                    }),
                            ),
                        )
                        .child(
                            div()
                                .mt_2()
                                .text_sm()
                                .text_color(palette.text)
                                .child("界面语言"),
                        )
                        .child(
                            div().flex().gap_2().children(
                                [("简体中文", !english, "zhCN"), ("English", english, "enUS")]
                                    .into_iter()
                                    .enumerate()
                                    .map(|(index, (label, active, locale))| {
                                        div()
                                            .id(("settings-locale", index))
                                            .px_4()
                                            .py_2()
                                            .rounded_md()
                                            .cursor_pointer()
                                            .bg(if active {
                                                palette.accent_surface
                                            } else {
                                                palette.background
                                            })
                                            .border_1()
                                            .border_color(if active {
                                                palette.accent
                                            } else {
                                                palette.border
                                            })
                                            .text_sm()
                                            .text_color(if active {
                                                palette.accent
                                            } else {
                                                palette.text_muted
                                            })
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                if this.state.read(cx).locale != locale {
                                                    this.set_locale(locale, cx);
                                                }
                                            }))
                                            .child(label)
                                    }),
                            ),
                        )
                        .child(
                            div()
                                .mt_3()
                                .text_xs()
                                .text_color(palette.text_soft)
                                .child("设置与桌面运行时共用 ui-preferences.json。"),
                        ),
                )
                .child(self.render_webdav_settings(palette, cx))
                .into_any_element();
        }

        unreachable!("all navigation sections are rendered above")
    }
}

impl Focusable for RootView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for RootView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let global_bounds = window.window_bounds().get_bounds();
        self.window_registry.update_bounds(
            &self.window_label,
            ScreenBounds::new(
                f32::from(global_bounds.origin.x) as i32,
                f32::from(global_bounds.origin.y) as i32,
                f32::from(global_bounds.size.width) as i32,
                f32::from(global_bounds.size.height) as i32,
            ),
        );
        let state = self.state.read(cx).clone();
        let active_session = self.sessions.get(&state.active_tab_id).cloned();
        let active_ftp_session = self.ftp_sessions.get(&state.active_tab_id).cloned();
        let active_local_session = self.local_sessions.get(&state.active_tab_id).cloned();
        let active_stream_session = self.stream_sessions.get(&state.active_tab_id).cloned();
        let pending_host_verification = self.pending_host_verification.clone();
        let pending_authentication = self.pending_authentication.clone();
        let pending_connection_editor = self.pending_connection_editor.clone();
        let pending_ssh_key_editor = self.pending_ssh_key_editor.clone();
        let pending_ssh_key_delete = self.pending_ssh_key_delete.clone();
        let pending_command_editor = self.pending_command_editor.clone();
        let pending_command_delete = self.pending_command_delete.clone();
        let pending_webdav_editor = self.pending_webdav_editor.clone();
        let palette = ThemePalette::for_mode(state.theme);
        let content = if let Some(section) = self.standalone_section {
            self.render_section(&state, section, palette, cx)
                .into_any_element()
        } else {
            let workspace = match (
                active_session,
                active_ftp_session,
                active_local_session,
                active_stream_session,
            ) {
                (Some(session), _, _, _) => session.into_any_element(),
                (_, Some(session), _, _) => session.into_any_element(),
                (_, _, Some(session), _) => session.into_any_element(),
                (_, _, _, Some(session)) => session.into_any_element(),
                (None, None, None, None) => self
                    .render_section(&state, state.navigation, palette, cx)
                    .into_any_element(),
            };
            div()
                .size_full()
                .flex()
                .child(self.render_sidebar(&state, palette, cx))
                .child(
                    div()
                        .min_w(px(0.0))
                        .h_full()
                        .flex_1()
                        .flex()
                        .flex_col()
                        .child(self.render_tabbar(&state, palette, cx))
                        .child(
                            div()
                                .min_h(px(0.0))
                                .flex_1()
                                .overflow_hidden()
                                .child(workspace),
                        ),
                )
                .into_any_element()
        };

        div()
            .id("fileterm-root")
            .key_context("FileTerm")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::handle_root_key))
            .on_action(cx.listener(Self::toggle_theme))
            .on_action(cx.listener(Self::new_connection))
            .on_action(cx.listener(Self::open_connection_manager))
            .on_action(cx.listener(Self::open_command_manager))
            .on_action(cx.listener(Self::open_ssh_key_manager))
            .on_action(cx.listener(Self::close_active_tab))
            .on_action(cx.listener(Self::open_docs))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, event, window, cx| {
                    this.finish_main_tab_drag(event, window, cx);
                    this.cancel_ssh_key_drag(cx);
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, event, window, cx| {
                    this.finish_main_tab_drag(event, window, cx);
                    this.cancel_ssh_key_drag(cx);
                }),
            )
            .size_full()
            .relative()
            .flex()
            .bg(palette.background)
            .text_color(palette.text)
            .child(content)
            .when_some(pending_host_verification, |view, pending| {
                view.child(self.render_host_verification(&pending, palette, cx))
            })
            .when_some(pending_authentication, |view, pending| {
                view.child(self.render_authentication(&pending, palette, cx))
            })
            .when_some(pending_connection_editor, |view, editor| {
                view.child(self.render_connection_editor(editor, palette, cx))
            })
            .when_some(pending_ssh_key_editor, |view, editor| {
                view.child(self.render_ssh_key_editor(editor, palette, cx))
            })
            .when_some(pending_ssh_key_delete, |view, target| {
                view.child(self.render_ssh_key_delete_confirmation(target, palette, cx))
            })
            .when_some(pending_command_editor, |view, pending| {
                view.child(self.render_command_editor(pending, palette, cx))
            })
            .when_some(pending_webdav_editor, |view, editor| {
                view.child(self.render_webdav_editor(editor, palette, cx))
            })
            .when_some(
                pending_command_delete,
                |view, (command_id, command_name)| {
                    view.child(self.render_command_delete_confirmation(
                        command_id,
                        command_name,
                        palette,
                        cx,
                    ))
                },
            )
    }
}
