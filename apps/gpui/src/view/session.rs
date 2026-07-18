use std::{collections::HashMap, path::PathBuf, sync::Arc, time::Duration};

use gpui::{
    div, prelude::*, px, Context, Entity, IntoElement, KeyDownEvent, Render, Subscription, Window,
};

use crate::{
    sftp::{
        client::{RemoteFileEntry, SftpClient},
        file_manager::FileManager,
        transfer::{
            TransferControl, TransferDirection, TransferFileIdentity, TransferIoOutcome,
            TransferProgress, TransferService, TransferTask, TransferTaskStatus,
        },
    },
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
    transfers: TransferService,
    transfer_controls: HashMap<String, TransferControl>,
    connection_error: Option<String>,
    pending_file_operation: Option<PendingFileOperation>,
    _subscriptions: Vec<Subscription>,
}

#[derive(Clone)]
enum FileOperationKind {
    CreateFile,
    CreateDirectory,
    Rename { from: String },
    Chmod { path: String },
}

#[derive(Clone)]
struct PendingFileOperation {
    kind: FileOperationKind,
    title: &'static str,
    input: String,
}

impl SessionWorkspace {
    pub fn new(
        tab_id: String,
        controller: Arc<SshController>,
        output: tokio::sync::broadcast::Receiver<TermChunk>,
        transfer_journal_path: PathBuf,
        app_state: Entity<AppState>,
        cx: &mut Context<Self>,
    ) -> Self {
        let terminal = cx.new(|cx| {
            TermView::from_transport_receiver(cx, ssh_transport(controller.clone()), output, 80, 24)
        });
        let term_session = terminal.read(cx).session();
        let runtime_subscription = cx.observe(&term_session, |this, session, cx| {
            let session = session.read(cx);
            let cwd = session.model.cwd.clone();
            this.files.update_terminal_identity(
                session.model.remote_user.clone(),
                session.model.terminal_elevated,
            );
            if let Some(path) = cwd {
                let path = path.to_string_lossy().to_string();
                if this.files.cwd != PathBuf::from(&path) {
                    this.load_remote_path(path, cx);
                }
            }
            cx.notify();
        });

        let mut transfers = TransferService::new(transfer_journal_path);
        let transfer_load_error = transfers.load().err().map(|error| error.to_string());
        let mut workspace = Self {
            tab_id,
            controller,
            terminal,
            app_state,
            metrics: None,
            sftp: None,
            files: FileManager::new(),
            transfers,
            transfer_controls: HashMap::new(),
            connection_error: transfer_load_error,
            pending_file_operation: None,
            _subscriptions: vec![runtime_subscription],
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

    fn begin_file_operation(
        &mut self,
        kind: FileOperationKind,
        title: &'static str,
        input: String,
        cx: &mut Context<Self>,
    ) {
        self.pending_file_operation = Some(PendingFileOperation { kind, title, input });
        cx.notify();
    }

    fn handle_file_operation_key(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(pending) = self.pending_file_operation.as_mut() else {
            return;
        };
        match event.keystroke.key.as_str() {
            "escape" => {
                self.pending_file_operation = None;
                cx.notify();
            }
            "enter" | "return" => self.submit_file_operation(cx),
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

    fn submit_file_operation(&mut self, cx: &mut Context<Self>) {
        let Some(pending) = self.pending_file_operation.take() else {
            return;
        };
        let Some(client) = self.sftp.clone() else {
            self.connection_error = Some("SFTP 尚未连接".to_string());
            return;
        };
        let cwd = self.files.cwd.to_string_lossy().to_string();
        let input = pending.input.trim().to_string();
        if input.is_empty() {
            self.connection_error = Some("文件名或权限不能为空".to_string());
            cx.notify();
            return;
        }
        cx.spawn(async move |this, cx| {
            let result = match pending.kind {
                FileOperationKind::CreateFile => {
                    client.write(&join_remote_path(&cwd, &input), &[]).await
                }
                FileOperationKind::CreateDirectory => {
                    client.mkdir(&join_remote_path(&cwd, &input)).await
                }
                FileOperationKind::Rename { from } => {
                    client.rename(&from, &join_remote_path(&cwd, &input)).await
                }
                FileOperationKind::Chmod { path } => match u32::from_str_radix(&input, 8) {
                    Ok(mode) if mode <= 0o7777 => client.chmod(&path, mode).await,
                    _ => Err(anyhow::anyhow!("权限必须是 0000 到 7777 的八进制数")),
                },
            };
            let _ = this.update(cx, |workspace, cx| {
                match result {
                    Ok(()) => workspace.refresh_files(cx),
                    Err(error) => workspace.connection_error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn confirm_delete(&mut self, entry: RemoteFileEntry, cx: &mut Context<Self>) {
        let Some(client) = self.sftp.clone() else {
            return;
        };
        cx.spawn(async move |this, cx| {
            let confirmed = rfd::AsyncMessageDialog::new()
                .set_level(rfd::MessageLevel::Warning)
                .set_title("删除远端项目")
                .set_description(format!("确定删除 {}？此操作无法撤销。", entry.path))
                .set_buttons(rfd::MessageButtons::YesNo)
                .show()
                .await
                == rfd::MessageDialogResult::Yes;
            if !confirmed {
                return;
            }
            let result = client.delete(&entry.path).await;
            let _ = this.update(cx, |workspace, cx| {
                match result {
                    Ok(()) => workspace.refresh_files(cx),
                    Err(error) => workspace.connection_error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn start_upload_picker(&mut self, cx: &mut Context<Self>) {
        let remote_directory = self.files.cwd.to_string_lossy().to_string();
        cx.spawn(async move |this, cx| {
            let selected = rfd::AsyncFileDialog::new().pick_files().await;
            let _ = this.update(cx, |workspace, cx| {
                for file in selected.unwrap_or_default() {
                    let local_path = file.path().to_path_buf();
                    let Some(name) = local_path.file_name().and_then(|name| name.to_str()) else {
                        continue;
                    };
                    let destination = join_remote_path(&remote_directory, name);
                    match workspace.transfers.enqueue(
                        TransferDirection::Upload,
                        &local_path.to_string_lossy(),
                        &destination,
                        Some(&workspace.tab_id),
                    ) {
                        Ok(id) => workspace.start_transfer(&id, false, cx),
                        Err(error) => workspace.connection_error = Some(error.to_string()),
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn start_download_picker(&mut self, entry: RemoteFileEntry, cx: &mut Context<Self>) {
        if entry.is_dir {
            return;
        }
        cx.spawn(async move |this, cx| {
            let selected = rfd::AsyncFileDialog::new()
                .set_file_name(&entry.name)
                .save_file()
                .await;
            let _ = this.update(cx, |workspace, cx| {
                let Some(file) = selected else {
                    return;
                };
                let destination = file.path().to_path_buf();
                match workspace.transfers.enqueue(
                    TransferDirection::Download,
                    &entry.path,
                    &destination.to_string_lossy(),
                    Some(&workspace.tab_id),
                ) {
                    Ok(id) => workspace.start_transfer(&id, false, cx),
                    Err(error) => workspace.connection_error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn start_transfer(&mut self, id: &str, overwrite: bool, cx: &mut Context<Self>) {
        let Some(client) = self.sftp.clone() else {
            self.connection_error = Some("SFTP 尚未连接".to_string());
            return;
        };
        let Some(task) = self.transfers.get(id).cloned() else {
            return;
        };
        let control = self
            .transfer_controls
            .entry(id.to_string())
            .or_default()
            .clone();
        control.reset();
        let (progress_tx, mut progress_rx) = tokio::sync::watch::channel(TransferProgress {
            transferred_bytes: task.transferred_bytes.unwrap_or(0),
            total_bytes: task.total_bytes.unwrap_or(0),
        });
        let progress_id = id.to_string();
        cx.spawn(async move |this, cx| {
            while progress_rx.changed().await.is_ok() {
                let progress = *progress_rx.borrow_and_update();
                if this
                    .update(cx, |workspace, cx| {
                        if let Err(error) =
                            workspace.transfers.update_progress(&progress_id, progress)
                        {
                            workspace.connection_error = Some(error.to_string());
                        }
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();

        let transfer_id = id.to_string();
        cx.spawn(async move |this, cx| {
            let prepared = inspect_transfer(&client, &task).await;
            let (identity, offset) = match prepared {
                Ok(prepared) => prepared,
                Err(error) => {
                    let message = error.to_string();
                    let _ = this.update(cx, |workspace, cx| {
                        let _ = workspace.transfers.fail(&transfer_id, message.clone());
                        workspace.connection_error = Some(message);
                        workspace.transfer_controls.remove(&transfer_id);
                        cx.notify();
                    });
                    return;
                }
            };
            let ready = this.update(cx, |workspace, cx| {
                let result = workspace
                    .transfers
                    .prepare_running(&transfer_id, identity, offset);
                if let Err(error) = &result {
                    let message = error.to_string();
                    let _ = workspace.transfers.fail(&transfer_id, message.clone());
                    workspace.connection_error = Some(message);
                    workspace.transfer_controls.remove(&transfer_id);
                }
                cx.notify();
                result.is_ok()
            });
            if !matches!(ready, Ok(true)) {
                return;
            }

            let result = execute_transfer(&client, &task, overwrite, &control, &progress_tx).await;
            let result = if matches!(&result, Ok(TransferIoOutcome::Canceled)) {
                cleanup_partial(&client, &task)
                    .await
                    .map(|_| TransferIoOutcome::Canceled)
            } else {
                result
            };
            let _ = this.update(cx, |workspace, cx| {
                match result {
                    Ok(TransferIoOutcome::Completed) => {
                        if let Err(error) = workspace
                            .transfers
                            .finish_io(&transfer_id, TransferIoOutcome::Completed)
                            .and_then(|_| workspace.transfers.complete(&transfer_id))
                        {
                            workspace.connection_error = Some(error.to_string());
                        }
                    }
                    Ok(outcome) => {
                        if let Err(error) = workspace.transfers.finish_io(&transfer_id, outcome) {
                            workspace.connection_error = Some(error.to_string());
                        }
                    }
                    Err(error) => {
                        let message = error.to_string();
                        let _ = workspace.transfers.fail(&transfer_id, message.clone());
                        workspace.connection_error = Some(message);
                    }
                }
                workspace.transfer_controls.remove(&transfer_id);
                workspace.refresh_files(cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn pause_transfer(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Some(control) = self.transfer_controls.get(id) {
            control.pause();
        }
        if let Err(error) = self.transfers.pause(id) {
            self.connection_error = Some(error.to_string());
        }
        cx.notify();
    }

    fn cancel_transfer(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Some(control) = self.transfer_controls.get(id) {
            control.cancel();
        }
        if let Err(error) = self.transfers.cancel(id) {
            self.connection_error = Some(error.to_string());
        }
        cx.notify();
    }

    fn resume_transfer(&mut self, id: &str, overwrite: bool, cx: &mut Context<Self>) {
        match self.transfers.resume(id) {
            Ok(()) => self.start_transfer(id, overwrite, cx),
            Err(error) => self.connection_error = Some(error.to_string()),
        }
        cx.notify();
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
                "终端用户",
                self.files
                    .terminal_user
                    .clone()
                    .unwrap_or_else(|| "检测中".to_string()),
                palette,
            ))
            .child(metric_row(
                "文件权限",
                if self.files.terminal_elevated && !self.files.file_access_elevated {
                    "SFTP 未提升".to_string()
                } else if self.files.file_access_elevated {
                    "已提升".to_string()
                } else {
                    "连接用户".to_string()
                },
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
                let download_entry = entry.clone();
                let rename_entry = entry.clone();
                let chmod_entry = entry.clone();
                let delete_entry = entry.clone();
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
                    .when(!is_dir, |row| {
                        row.child(
                            div()
                                .id(("download-remote-file", index))
                                .px_1()
                                .py_1()
                                .rounded_sm()
                                .text_xs()
                                .text_color(palette.accent)
                                .hover(move |style| style.bg(palette.surface_hover))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.start_download_picker(download_entry.clone(), cx)
                                }))
                                .child("↓"),
                        )
                    })
                    .child(
                        div()
                            .id(("rename-remote-file", index))
                            .px_1()
                            .py_1()
                            .text_xs()
                            .text_color(palette.text_muted)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.begin_file_operation(
                                    FileOperationKind::Rename {
                                        from: rename_entry.path.clone(),
                                    },
                                    "重命名远端项目",
                                    rename_entry.name.clone(),
                                    cx,
                                )
                            }))
                            .child("改"),
                    )
                    .child(
                        div()
                            .id(("chmod-remote-file", index))
                            .px_1()
                            .py_1()
                            .text_xs()
                            .text_color(palette.text_muted)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.begin_file_operation(
                                    FileOperationKind::Chmod {
                                        path: chmod_entry.path.clone(),
                                    },
                                    "修改远端权限",
                                    if chmod_entry.is_dir { "0755" } else { "0644" }
                                        .to_string(),
                                    cx,
                                )
                            }))
                            .child("权"),
                    )
                    .child(
                        div()
                            .id(("delete-remote-file", index))
                            .px_1()
                            .py_1()
                            .text_xs()
                            .text_color(palette.danger)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.confirm_delete(delete_entry.clone(), cx)
                            }))
                            .child("删"),
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
                    .child(button("上传", "files-upload", palette, cx, |this, cx| {
                        this.start_upload_picker(cx)
                    }))
                    .child(button(
                        "文件+",
                        "files-create-file",
                        palette,
                        cx,
                        |this, cx| {
                            this.begin_file_operation(
                                FileOperationKind::CreateFile,
                                "新建远端文件",
                                String::new(),
                                cx,
                            )
                        },
                    ))
                    .child(button(
                        "目录+",
                        "files-create-directory",
                        palette,
                        cx,
                        |this, cx| {
                            this.begin_file_operation(
                                FileOperationKind::CreateDirectory,
                                "新建远端目录",
                                String::new(),
                                cx,
                            )
                        },
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

    fn render_file_operation(
        &self,
        pending: PendingFileOperation,
        palette: ThemePalette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(gpui::black().opacity(0.6))
            .child(
                div()
                    .w(px(420.0))
                    .p_5()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .rounded_lg()
                    .bg(palette.surface)
                    .border_1()
                    .border_color(palette.border_strong)
                    .child(
                        div()
                            .text_lg()
                            .text_color(palette.text)
                            .child(pending.title),
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
                            .child(if pending.input.is_empty() {
                                "输入后按 Enter".to_string()
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
                                    .id("cancel-file-operation")
                                    .px_3()
                                    .py_2()
                                    .cursor_pointer()
                                    .text_sm()
                                    .text_color(palette.text_muted)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.pending_file_operation = None;
                                        cx.notify();
                                    }))
                                    .child("取消"),
                            )
                            .child(
                                div()
                                    .id("submit-file-operation")
                                    .px_3()
                                    .py_2()
                                    .rounded_md()
                                    .cursor_pointer()
                                    .bg(palette.accent)
                                    .text_sm()
                                    .text_color(palette.background)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.submit_file_operation(cx)
                                    }))
                                    .child("确认"),
                            ),
                    ),
            )
    }

    fn render_transfers(&self, palette: ThemePalette, cx: &mut Context<Self>) -> impl IntoElement {
        let rows = self
            .transfers
            .list()
            .iter()
            .rev()
            .take(6)
            .cloned()
            .enumerate()
            .map(|(index, task)| {
                let pause_id = task.id.clone();
                let resume_id = task.id.clone();
                let overwrite_id = task.id.clone();
                let cancel_id = task.id.clone();
                let progress = format!("{:.0}%", task.progress * 100.0);
                let can_overwrite = task.status == TransferTaskStatus::Failed
                    && task
                        .message
                        .as_deref()
                        .is_some_and(|message| message.contains("destination already exists"));
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .border_t_1()
                    .border_color(palette.border)
                    .child(
                        div()
                            .min_w(px(0.0))
                            .flex_1()
                            .child(
                                div()
                                    .truncate()
                                    .text_xs()
                                    .text_color(palette.text)
                                    .child(task.name.clone()),
                            )
                            .child(div().text_xs().text_color(palette.text_soft).child(format!(
                                "{} · {}",
                                transfer_status_label(task.status),
                                progress
                            ))),
                    )
                    .when(task.status == TransferTaskStatus::Running, |row| {
                        row.child(
                            div()
                                .id(("pause-transfer", index))
                                .px_2()
                                .py_1()
                                .cursor_pointer()
                                .text_xs()
                                .text_color(palette.accent)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.pause_transfer(&pause_id, cx)
                                }))
                                .child("暂停"),
                        )
                    })
                    .when(
                        matches!(
                            task.status,
                            TransferTaskStatus::Paused | TransferTaskStatus::Failed
                        ),
                        |row| {
                            row.child(
                                div()
                                    .id(("resume-transfer", index))
                                    .px_2()
                                    .py_1()
                                    .cursor_pointer()
                                    .text_xs()
                                    .text_color(palette.accent)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.resume_transfer(&resume_id, false, cx)
                                    }))
                                    .child("继续"),
                            )
                        },
                    )
                    .when(can_overwrite, |row| {
                        row.child(
                            div()
                                .id(("overwrite-transfer", index))
                                .px_2()
                                .py_1()
                                .cursor_pointer()
                                .text_xs()
                                .text_color(palette.danger)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.resume_transfer(&overwrite_id, true, cx)
                                }))
                                .child("覆盖"),
                        )
                    })
                    .when(!task.status.terminal(), |row| {
                        row.child(
                            div()
                                .id(("cancel-transfer", index))
                                .px_2()
                                .py_1()
                                .cursor_pointer()
                                .text_xs()
                                .text_color(palette.danger)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.cancel_transfer(&cancel_id, cx)
                                }))
                                .child("取消"),
                        )
                    })
            })
            .collect::<Vec<_>>();

        div()
            .max_h(px(190.0))
            .flex()
            .flex_col()
            .border_t_1()
            .border_color(palette.border_strong)
            .child(
                div()
                    .px_3()
                    .py_2()
                    .text_sm()
                    .text_color(palette.text)
                    .child(format!("传输任务 · {}", self.transfers.list().len())),
            )
            .children(rows)
    }
}

impl Render for SessionWorkspace {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = ThemePalette::for_mode(self.app_state.read(cx).theme);
        let pending_file_operation = self.pending_file_operation.clone();
        div()
            .size_full()
            .relative()
            .on_key_down(cx.listener(Self::handle_file_operation_key))
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
                    .child(self.render_transfers(palette, cx))
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

async fn inspect_transfer(
    client: &SftpClient,
    task: &TransferTask,
) -> anyhow::Result<(TransferFileIdentity, u64)> {
    let source = task
        .source_path
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("transfer source path is missing"))?;
    let partial = task
        .partial_path
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("transfer partial path is missing"))?;
    match task.direction {
        TransferDirection::Upload => {
            let metadata = tokio::fs::metadata(source).await?;
            let modified_at = metadata
                .modified()
                .ok()
                .and_then(|value| value.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|value| value.as_secs());
            let offset = client.remote_file_size_if_exists(partial).await?;
            Ok((
                TransferFileIdentity {
                    size: metadata.len(),
                    modified_at,
                },
                offset,
            ))
        }
        TransferDirection::Download => {
            let identity = client.remote_file_identity(source).await?;
            let offset = match tokio::fs::metadata(partial).await {
                Ok(metadata) => metadata.len(),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
                Err(error) => return Err(error.into()),
            };
            Ok((identity, offset))
        }
    }
}

async fn cleanup_partial(client: &SftpClient, task: &TransferTask) -> anyhow::Result<()> {
    let partial = task
        .partial_path
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("transfer partial path is missing"))?;
    match task.direction {
        TransferDirection::Upload => client.remove_file_if_exists(partial).await,
        TransferDirection::Download => match tokio::fs::remove_file(partial).await {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        },
    }
}

async fn execute_transfer(
    client: &SftpClient,
    task: &TransferTask,
    overwrite: bool,
    control: &TransferControl,
    progress: &tokio::sync::watch::Sender<TransferProgress>,
) -> anyhow::Result<TransferIoOutcome> {
    let source = task
        .source_path
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("transfer source path is missing"))?;
    let destination = task
        .destination_path
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("transfer destination path is missing"))?;
    let partial = task
        .partial_path
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("transfer partial path is missing"))?;

    match task.direction {
        TransferDirection::Upload => {
            let outcome = client
                .upload_file(PathBuf::from(source).as_path(), partial, control, progress)
                .await?;
            if outcome == TransferIoOutcome::Completed {
                client
                    .finalize_upload(partial, destination, overwrite)
                    .await?;
            }
            Ok(outcome)
        }
        TransferDirection::Download => {
            let partial = PathBuf::from(partial);
            let destination = PathBuf::from(destination);
            let outcome = client
                .download_file(source, &partial, control, progress)
                .await?;
            if outcome == TransferIoOutcome::Completed {
                client
                    .finalize_download(&partial, &destination, overwrite)
                    .await?;
            }
            Ok(outcome)
        }
    }
}

fn join_remote_path(directory: &str, name: &str) -> String {
    if directory == "/" {
        format!("/{name}")
    } else {
        format!("{}/{name}", directory.trim_end_matches('/'))
    }
}

fn transfer_status_label(status: TransferTaskStatus) -> &'static str {
    match status {
        TransferTaskStatus::Queued => "等待中",
        TransferTaskStatus::Running => "传输中",
        TransferTaskStatus::Paused => "已暂停",
        TransferTaskStatus::Verifying => "校验中",
        TransferTaskStatus::Finalizing => "完成中",
        TransferTaskStatus::Done => "已完成",
        TransferTaskStatus::Failed => "失败",
        TransferTaskStatus::Canceled => "已取消",
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
