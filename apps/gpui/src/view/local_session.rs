use std::sync::Arc;

use gpui::{div, prelude::*, px, Context, Entity, IntoElement, Render, Window};

use crate::{
    state::AppState,
    term::{PtyHandle, TermView},
    theme::ThemePalette,
};

/// Local shell workspace. It deliberately owns only PTY lifecycle and terminal
/// presentation; remote file/system concerns remain in `SessionWorkspace`.
pub struct LocalSessionWorkspace {
    shell: String,
    _pty: Option<Arc<PtyHandle>>,
    terminal: Option<Entity<TermView>>,
    error: Option<String>,
    app_state: Entity<AppState>,
}

impl LocalSessionWorkspace {
    pub fn send_command(&self, command: &str, append_carriage_return: bool) -> anyhow::Result<()> {
        let pty = self
            ._pty
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("本地 PTY 未运行"))?;
        pty.write_input(command.as_bytes())?;
        if append_carriage_return {
            pty.write_input(b"\r")?;
        }
        Ok(())
    }

    pub fn close(&mut self) {
        if let Some(pty) = self._pty.take() {
            pty.terminate();
        }
        self.terminal = None;
    }

    pub fn spawn(app_state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let shell = default_shell();
        match PtyHandle::spawn(&shell, 80, 24) {
            Ok((pty, _)) => {
                #[allow(clippy::arc_with_non_send_sync)]
                let pty = Arc::new(pty);
                let terminal = cx.new(|cx| TermView::new(cx, pty.clone(), 80, 24));
                Self {
                    shell,
                    _pty: Some(pty),
                    terminal: Some(terminal),
                    error: None,
                    app_state,
                }
            }
            Err(error) => Self {
                shell,
                _pty: None,
                terminal: None,
                error: Some(error.to_string()),
                app_state,
            },
        }
    }
}

impl Drop for LocalSessionWorkspace {
    fn drop(&mut self) {
        if let Some(pty) = self._pty.take() {
            pty.terminate();
        }
    }
}

impl Render for LocalSessionWorkspace {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = ThemePalette::for_mode(self.app_state.read(cx).theme);
        let shell = self.shell.clone();
        let terminal = self.terminal.clone();
        let error = self.error.clone();
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(palette.background)
            .child(
                div()
                    .h(px(36.0))
                    .flex()
                    .items_center()
                    .justify_between()
                    .px_3()
                    .bg(palette.surface)
                    .border_b_1()
                    .border_color(palette.border)
                    .child(
                        div()
                            .text_xs()
                            .text_color(palette.text_muted)
                            .child(format!("本地终端 · {shell}")),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(palette.success)
                            .child("本机 PTY"),
                    ),
            )
            .child(
                div()
                    .min_h(px(0.0))
                    .flex_1()
                    .when_some(terminal, |view, terminal| view.child(terminal))
                    .when_some(error, |view, error| {
                        view.flex()
                            .items_center()
                            .justify_center()
                            .text_sm()
                            .text_color(palette.danger)
                            .child(format!("本地终端启动失败：{error}"))
                    }),
            )
    }
}

fn default_shell() -> String {
    std::env::var("SHELL")
        .ok()
        .filter(|shell| !shell.trim().is_empty())
        .unwrap_or_else(|| {
            if cfg!(target_os = "windows") {
                "powershell.exe".to_string()
            } else if std::path::Path::new("/bin/zsh").exists() {
                "/bin/zsh".to_string()
            } else if std::path::Path::new("/bin/bash").exists() {
                "/bin/bash".to_string()
            } else {
                "sh".to_string()
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_resolution_never_returns_empty() {
        assert!(!default_shell().trim().is_empty());
    }
}
