use std::{path::PathBuf, sync::Arc, time::Duration};

use gpui::{div, prelude::*, px, Context, Entity, IntoElement, Render, Subscription, Window};

use crate::{
    sftp::{client::SftpClient, file_manager::FileManager},
    ssh::{
        controller::{SshController, SshSessionEvent},
        system_sidebar::{SystemMetrics, SystemSidebarCollector},
    },
    state::{AppState, TabStatus},
    term::{ssh_transport, TermChunk, TermView},
    theme::ThemePalette,
};

pub struct SessionWorkspace {
    tab_id: String,
    controller: Arc<SshController>,
    terminal: Entity<TermView>,
    app_state: Entity<AppState>,
    metrics: Option<SystemMetrics>,
    sftp: Option<Arc<SftpClient>>,
    files: FileManager,
    connection_error: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl SessionWorkspace {
    pub fn new(
        tab_id: String,
        controller: Arc<SshController>,
        output: tokio::sync::broadcast::Receiver<TermChunk>,
        app_state: Entity<AppState>,
        cx: &mut Context<Self>,
    ) -> Self {
        let terminal = cx.new(|cx| {
            TermView::from_transport_receiver(cx, ssh_transport(controller.clone()), output, 80, 24)
        });
        let term_session = terminal.read(cx).session();
        let cwd_subscription = cx.observe(&term_session, |this, session, cx| {
            let cwd = session.read(cx).model.cwd.clone();
            if let Some(path) = cwd {
                let path = path.to_string_lossy().to_string();
                if this.files.cwd != PathBuf::from(&path) {
                    this.load_remote_path(path, cx);
                }
            }
        });

        let mut workspace = Self {
            tab_id,
            controller,
            terminal,
            app_state,
            metrics: None,
            sftp: None,
            files: FileManager::new(),
            connection_error: None,
            _subscriptions: vec![cwd_subscription],
        };
        workspace.spawn_session_events(cx);
        workspace.spawn_metrics(cx);
        workspace.connect_sftp(cx);
        workspace
    }

    fn spawn_session_events(&mut self, cx: &mut Context<Self>) {
        let mut events = self.controller.subscribe_events();
        cx.spawn(async move |this, cx| {
            while let Ok(event) = events.recv().await {
                let should_stop = matches!(event, SshSessionEvent::Closed);
                let _ = this.update(cx, |workspace, cx| {
                    let status = match &event {
                        SshSessionEvent::Connected => TabStatus::Connected,
                        SshSessionEvent::Closed => TabStatus::Closed,
                        SshSessionEvent::Error(error) => {
                            workspace.connection_error = Some(error.clone());
                            TabStatus::Error
                        }
                    };
                    workspace.app_state.update(cx, |state, cx| {
                        state.set_tab_status(&workspace.tab_id, status);
                        cx.notify();
                    });
                    cx.notify();
                });
                if should_stop {
                    break;
                }
            }
        })
        .detach();
    }

    fn spawn_metrics(&mut self, cx: &mut Context<Self>) {
        let mut collector = SystemSidebarCollector::new(self.controller.clone(), 5);
        cx.spawn(async move |this, cx| loop {
            let metrics = collector.collect().await;
            if this
                .update(cx, |workspace, cx| {
                    workspace.metrics = Some(metrics);
                    cx.notify();
                })
                .is_err()
            {
                break;
            }
            tokio::time::sleep(Duration::from_secs(collector.interval_secs())).await;
        })
        .detach();
    }

    fn connect_sftp(&mut self, cx: &mut Context<Self>) {
        self.files.loading = true;
        let controller = self.controller.clone();
        cx.spawn(async move |this, cx| {
            let result = async {
                let client = Arc::new(SftpClient::connect(controller).await?);
                let entries = client.list_dir(None).await?;
                Ok::<_, anyhow::Error>((client, entries))
            }
            .await;
            let _ = this.update(cx, |workspace, cx| {
                match result {
                    Ok((client, entries)) => {
                        let cwd = client.cwd();
                        workspace.sftp = Some(client);
                        workspace.files.replace_listing(cwd, entries);
                    }
                    Err(error) => workspace.files.fail_loading(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn load_remote_path(&mut self, path: String, cx: &mut Context<Self>) {
        let Some(client) = self.sftp.clone() else {
            return;
        };
        self.files.loading = true;
        self.files.error = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = async {
                let cwd = client.cd(&path).await?;
                let entries = client.list_dir(Some(&cwd)).await?;
                Ok::<_, anyhow::Error>((cwd, entries))
            }
            .await;
            let _ = this.update(cx, |workspace, cx| {
                match result {
                    Ok((cwd, entries)) => workspace.files.replace_listing(cwd, entries),
                    Err(error) => workspace.files.fail_loading(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn refresh_files(&mut self, cx: &mut Context<Self>) {
        let cwd = self.files.cwd.to_string_lossy().to_string();
        self.load_remote_path(cwd, cx);
    }

    fn open_parent(&mut self, cx: &mut Context<Self>) {
        let Some(client) = self.sftp.as_ref() else {
            return;
        };
        self.load_remote_path(client.parent_path(), cx);
    }

    fn render_metrics(&self, palette: ThemePalette) -> impl IntoElement {
        let metrics = self.metrics.as_ref();
        let percent = |value: Option<f32>| {
            value
                .map(|value| format!("{value:.1}%"))
                .unwrap_or_else(|| "—".to_string())
        };
        let bytes = |value: Option<u64>| value.map(format_bytes).unwrap_or_else(|| "—".to_string());
        div()
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .border_b_1()
            .border_color(palette.border)
            .child(div().text_sm().text_color(palette.text).child("远端状态"))
            .child(metric_row(
                "平台",
                metrics
                    .and_then(|value| value.platform.clone())
                    .unwrap_or_else(|| "检测中".into()),
                palette,
            ))
            .child(metric_row(
                "CPU",
                percent(metrics.and_then(|value| value.cpu_usage)),
                palette,
            ))
            .child(metric_row(
                "内存",
                format!(
                    "{} / {}",
                    bytes(metrics.and_then(|value| value.memory_used)),
                    bytes(metrics.and_then(|value| value.memory_total))
                ),
                palette,
            ))
            .child(metric_row(
                "网络",
                format!(
                    "↓ {}  ↑ {}",
                    bytes(metrics.and_then(|value| value.network_rx_bytes)),
                    bytes(metrics.and_then(|value| value.network_tx_bytes))
                ),
                palette,
            ))
    }

    fn render_files(&self, palette: ThemePalette, cx: &mut Context<Self>) -> impl IntoElement {
        let rows = self
            .files
            .entries
            .iter()
            .take(40)
            .cloned()
            .collect::<Vec<_>>()
            .into_iter()
            .enumerate()
            .map(|(index, entry)| {
                let path = entry.path.clone();
                let is_dir = entry.is_dir;
                div()
                    .id(("remote-file", index))
                    .h(px(30.0))
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .cursor_pointer()
                    .hover(move |style| style.bg(palette.surface_hover))
                    .when(is_dir, |row| {
                        row.on_click(cx.listener(move |this, _, _, cx| {
                            this.load_remote_path(path.clone(), cx)
                        }))
                    })
                    .child(
                        div()
                            .w(px(18.0))
                            .text_xs()
                            .text_color(if is_dir {
                                palette.accent
                            } else {
                                palette.text_soft
                            })
                            .child(if is_dir { "D" } else { "F" }),
                    )
                    .child(
                        div()
                            .flex_1()
                            .truncate()
                            .text_xs()
                            .text_color(palette.text)
                            .child(entry.name.clone()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(palette.text_soft)
                            .child(if is_dir {
                                String::new()
                            } else {
                                format_bytes(entry.size)
                            }),
                    )
            })
            .collect::<Vec<_>>();

        div()
            .min_h(px(0.0))
            .flex_1()
            .flex()
            .flex_col()
            .child(
                div()
                    .h(px(38.0))
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .border_b_1()
                    .border_color(palette.border)
                    .child(button("上级", "files-up", palette, cx, |this, cx| {
                        this.open_parent(cx)
                    }))
                    .child(button(
                        "刷新",
                        "files-refresh",
                        palette,
                        cx,
                        |this, cx| this.refresh_files(cx),
                    ))
                    .child(
                        div()
                            .flex_1()
                            .truncate()
                            .text_xs()
                            .text_color(palette.text_muted)
                            .child(self.files.cwd.to_string_lossy().to_string()),
                    ),
            )
            .child(
                div()
                    .min_h(px(0.0))
                    .flex_1()
                    .overflow_hidden()
                    .when(self.files.loading, |view| {
                        view.child(
                            div()
                                .p_3()
                                .text_xs()
                                .text_color(palette.text_muted)
                                .child("正在读取目录…"),
                        )
                    })
                    .when_some(self.files.error.clone(), |view, error| {
                        view.child(
                            div()
                                .p_3()
                                .text_xs()
                                .text_color(palette.danger)
                                .child(error),
                        )
                    })
                    .when(!self.files.loading && self.files.error.is_none(), |view| {
                        view.children(rows)
                    }),
            )
    }
}

impl Render for SessionWorkspace {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = ThemePalette::for_mode(self.app_state.read(cx).theme);
        div()
            .size_full()
            .flex()
            .bg(palette.background)
            .child(
                div()
                    .min_w(px(0.0))
                    .flex_1()
                    .h_full()
                    .child(self.terminal.clone()),
            )
            .child(
                div()
                    .w(px(320.0))
                    .h_full()
                    .flex()
                    .flex_col()
                    .bg(palette.surface)
                    .border_l_1()
                    .border_color(palette.border)
                    .child(self.render_metrics(palette))
                    .child(self.render_files(palette, cx))
                    .when_some(self.connection_error.clone(), |view, error| {
                        view.child(
                            div()
                                .p_3()
                                .text_xs()
                                .text_color(palette.danger)
                                .child(error),
                        )
                    }),
            )
    }
}

fn metric_row(label: &'static str, value: String, palette: ThemePalette) -> impl IntoElement {
    div()
        .flex()
        .justify_between()
        .gap_3()
        .text_xs()
        .child(div().text_color(palette.text_muted).child(label))
        .child(div().truncate().text_color(palette.text).child(value))
}

fn button(
    label: &'static str,
    id: &'static str,
    palette: ThemePalette,
    cx: &mut Context<SessionWorkspace>,
    on_click: impl Fn(&mut SessionWorkspace, &mut Context<SessionWorkspace>) + 'static,
) -> impl IntoElement {
    div()
        .id(id)
        .px_2()
        .py_1()
        .rounded_sm()
        .cursor_pointer()
        .text_xs()
        .text_color(palette.text_muted)
        .hover(move |style| style.bg(palette.surface_hover))
        .on_click(cx.listener(move |this, _, _, cx| on_click(this, cx)))
        .child(label)
}

fn format_bytes(value: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = value as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{} {}", value as u64, UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byte_sizes_are_compact() {
        assert_eq!(format_bytes(42), "42 B");
        assert_eq!(format_bytes(2048), "2.0 KB");
    }
}
