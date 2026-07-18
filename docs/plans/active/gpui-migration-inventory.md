# FileTerm GPUI 迁移清单

| 项目     | 值                          |
| -------- | --------------------------- |
| 文档版本 | v0.3                        |
| 更新日期 | 2026-07-18                  |
| 状态     | 配套 [gpui-refactor.md](./gpui-refactor.md) |
| 数据来源 | 直接扫描 `apps/tauri/src-tauri/src/` 与 `apps/tauri/src/renderer/` |

> 本文档是 GPUI 重构的逐项迁移清单。每条记录都基于真实源码扫描，含路径锚点、类型签名、调用频率与迁移优先级。

---

## 1. Tauri Command → Bridge fn（108 项）

`invoke_handler!` 在 `apps/tauri/src-tauri/src/lib.rs#L1406-L1519` 共注册 108 条命令。GPUI 端把每个 `#[tauri::command]` 函数体抽成 `pub async fn`，签名的 `app: AppHandle` / `state: State<...>` / `window: WebviewWindow` 框架参数剥离，业务参数保留。

**频率**：高=终端输入/传输进度；中=snapshot/resize/文件读写；低=CRUD/连接生命周期/UI 配置。
**阻塞**：阻塞=async + 等待 worker 响应；半阻塞=同步文件读写/系统调用；非阻塞=纯计算。
**优先级**：P0=终端+SSH+SFTP+存储核心+Profile-Folder CRUD+ssh-key；P1=Transfer/Window/Tunnel/Workspace/Command-template；P2=Update/Clipboard/External/UI-state/WebDAV/导入导出/运行时信息。

### 1.1 app / 运行时 / Profile-Folder CRUD（17 项）

| 命令名 | 路径#行 | 输入 | 返回 | async | 频率 | 阻塞 | 优先级 |
|---|---|---|---|---|---|---|---|
| `app_get_platform` | apps/tauri/src-tauri/src/commands/mod.rs#L125 | (无) | String | 否 | 低 | 非阻塞 | P2 |
| `app_get_arch` | apps/tauri/src-tauri/src/commands/mod.rs#L162 | (无) | String | 否 | 低 | 非阻塞 | P2 |
| `app_get_runtime_version` | apps/tauri/src-tauri/src/commands/mod.rs#L171 | (无) | String | 否 | 低 | 非阻塞 | P2 |
| `app_open_logs_directory` | apps/tauri/src-tauri/src/commands/mod.rs#L232 | (无) | () | 否 | 低 | 半阻塞 | P2 |
| `app_get_snapshot` | apps/tauri/src-tauri/src/commands/mod.rs#L446 | (无) | serde_json::Value | 是 | 高 | 阻塞 | P0 |
| `app_get_connection_library` | apps/tauri/src-tauri/src/commands/mod.rs#L451 | (无) | serde_json::Value | 是 | 低 | 阻塞 | P0 |
| `app_workspace_mutation` | apps/tauri/src-tauri/src/commands/mod.rs#L675 | operation: String, payload: serde_json::Value | serde_json::Value | 是 | 低 | 阻塞 | P0 |
| `app_preview_connection_import` | apps/tauri/src-tauri/src/commands/mod.rs#L520 | source: Option\<String\> | Option\<serde_json::Value\> | 是 | 低 | 阻塞 | P2 |
| `app_commit_connection_json_import` | apps/tauri/src-tauri/src/commands/mod.rs#L549 | plan_id: String, options: serde_json::Value | serde_json::Value | 是 | 低 | 阻塞 | P2 |
| `app_export_connections` | apps/tauri/src-tauri/src/commands/mod.rs#L574 | format: String | bool | 是 | 低 | 阻塞 | P2 |
| `app_export_connections_as_files` | apps/tauri/src-tauri/src/commands/mod.rs#L596 | format: String | bool | 是 | 低 | 阻塞 | P2 |
| `app_create_profile` | apps/tauri/src-tauri/src/commands/mod.rs#L2015 | input: serde_json::Value | serde_json::Value | 是 | 低 | 阻塞 | P0 |
| `app_update_profile` | apps/tauri/src-tauri/src/commands/mod.rs#L2025 | profile_id: String, input: serde_json::Value | serde_json::Value | 是 | 低 | 阻塞 | P0 |
| `app_delete_profile` | apps/tauri/src-tauri/src/commands/mod.rs#L2045 | profile_id: String | serde_json::Value | 是 | 低 | 阻塞 | P0 |
| `app_update_folder` | apps/tauri/src-tauri/src/commands/mod.rs#L2055 | folder_id: String, updates: serde_json::Value | serde_json::Value | 是 | 低 | 阻塞 | P0 |
| `app_delete_folder` | apps/tauri/src-tauri/src/commands/mod.rs#L2066 | folder_id: String | serde_json::Value | 是 | 低 | 阻塞 | P0 |
| `app_update_entity_order` | apps/tauri/src-tauri/src/commands/mod.rs#L2076 | id: String, new_parent_id: Option\<String\>, new_order: f64 | serde_json::Value | 是 | 低 | 阻塞 | P0 |

### 1.2 workspace（可拆分窗口 / Tab 归属）（8 项）

| 命令名 | 路径#行 | 输入 | 返回 | async | 频率 | 阻塞 | 优先级 |
|---|---|---|---|---|---|---|---|
| `workspace_get_window_context` | apps/tauri/src-tauri/src/commands/workspace_window.rs#L85 | (无) | WorkspaceWindowContext | 否 | 低 | 非阻塞 | P1 |
| `workspace_get_tab_placements` | apps/tauri/src-tauri/src/commands/workspace_window.rs#L96 | (无) | Vec\<WorkspaceTabPlacement\> | 否 | 中 | 非阻塞 | P1 |
| `workspace_list_windows` | apps/tauri/src-tauri/src/commands/workspace_window.rs#L104 | (无) | Vec\<WorkspaceWindowContext\> | 否 | 低 | 非阻塞 | P1 |
| `workspace_move_tab` | apps/tauri/src-tauri/src/commands/workspace_window.rs#L113 | input: MoveTabInput | Vec\<WorkspaceTabPlacement\> | 是 | 低 | 半阻塞 | P1 |
| `workspace_detach_tab` | apps/tauri/src-tauri/src/commands/workspace_window.rs#L276 | input: DetachTabInput | Vec\<WorkspaceTabPlacement\> | 是 | 低 | 阻塞 | P1 |
| `workspace_start_tab_drag` | apps/tauri/src-tauri/src/commands/workspace_window.rs#L127 | input: StartDragInput | () | 否 | 低 | 非阻塞 | P1 |
| `workspace_finish_tab_drag` | apps/tauri/src-tauri/src/commands/workspace_window.rs#L144 | input: FinishDragInput | Vec\<WorkspaceTabPlacement\> | 是 | 低 | 阻塞 | P1 |
| `workspace_mark_detached_ready` | apps/tauri/src-tauri/src/commands/workspace_window.rs#L300 | (无) | () | 否 | 低 | 非阻塞 | P1 |

### 1.3 ssh / 会话生命周期 + 终端 + 交互认证（9 项，FTP/Serial/Telnet 共用）

| 命令名 | 路径#行 | 输入 | 返回 | async | 频率 | 阻塞 | 优先级 |
|---|---|---|---|---|---|---|---|
| `app_open_profile` | apps/tauri/src-tauri/src/commands/mod.rs#L1062 | profile_id: String | serde_json::Value | 是 | 低 | 阻塞 | P0 |
| `app_activate_tab` | apps/tauri/src-tauri/src/commands/mod.rs#L1183 | tab_id: String | serde_json::Value | 是 | 中 | 阻塞 | P0 |
| `app_reconnect_tab` | apps/tauri/src-tauri/src/commands/mod.rs#L1196 | tab_id: String | serde_json::Value | 是 | 低 | 阻塞 | P0 |
| `app_disconnect_tab` | apps/tauri/src-tauri/src/commands/mod.rs#L1327 | tab_id: String | serde_json::Value | 是 | 低 | 阻塞 | P0 |
| `app_close_tab` | apps/tauri/src-tauri/src/commands/mod.rs#L1377 | tab_id: String | serde_json::Value | 是 | 低 | 阻塞 | P0 |
| `app_write_terminal` | apps/tauri/src-tauri/src/commands/mod.rs#L1402 | tab_id: String, data: String | () | 是 | 高 | 阻塞 | P0 |
| `app_subscribe_terminal_data` | apps/tauri/src-tauri/src/commands/mod.rs#L1412 | channel: Channel\<serde_json::Value\> | () | 否 | 低 | 非阻塞 | P0 |
| `app_resize_terminal` | apps/tauri/src-tauri/src/commands/mod.rs#L1418 | tab_id: String, cols: u32, rows: u32, width: u32, height: u32 | () | 是 | 中 | 阻塞 | P0 |
| `app_resolve_ssh_interaction` | apps/tauri/src-tauri/src/commands/mod.rs#L1988 | request_id: String, response: serde_json::Value | () | 是 | 低 | 半阻塞 | P0 |

> FTP / Serial / Telnet / system-metrics / proxy 无独立 command。`app_open_profile` / `app_write_terminal` 等通用入口在 `start_session_worker`（mod.rs#L986）按 `profile.type` 分发到 `sessions::{ftp,serial,telnet}::start_*_worker`。system_metrics 由各 worker 内部 collector 采集，经 `workspace:snapshot` / `workspace:sessionMetrics` 事件下发。

### 1.4 sftp / 远端文件操作（12 项）

| 命令名 | 路径#行 | 输入 | 返回 | async | 频率 | 阻塞 | 优先级 |
|---|---|---|---|---|---|---|---|
| `app_open_remote_path` | apps/tauri/src-tauri/src/commands/mod.rs#L1444 | tab_id: String, target_path: String | serde_json::Value | 是 | 中 | 阻塞 | P0 |
| `app_set_follow_shell_cwd` | apps/tauri/src-tauri/src/commands/mod.rs#L1461 | tab_id: String, enabled: bool | serde_json::Value | 是 | 低 | 阻塞 | P0 |
| `app_read_remote_file` | apps/tauri/src-tauri/src/commands/mod.rs#L1513 | tab_id: String, target_path: String, encoding: Option\<String\> | String | 是 | 中 | 阻塞 | P0 |
| `app_write_remote_file` | apps/tauri/src-tauri/src/commands/mod.rs#L1529 | tab_id: String, target_path: String, content: String, encoding: Option\<String\> | serde_json::Value | 是 | 中 | 阻塞 | P0 |
| `app_create_remote_directory` | apps/tauri/src-tauri/src/commands/mod.rs#L1554 | tab_id: String, parent_path: String, name: String | serde_json::Value | 是 | 低 | 阻塞 | P0 |
| `app_create_remote_file` | apps/tauri/src-tauri/src/commands/mod.rs#L1572 | tab_id: String, parent_path: String, name: String | serde_json::Value | 是 | 低 | 阻塞 | P0 |
| `app_copy_remote_path` | apps/tauri/src-tauri/src/commands/mod.rs#L1590 | tab_id: String, target_path: String, destination_path: String, target_type: String | serde_json::Value | 是 | 低 | 阻塞 | P0 |
| `app_move_remote_path` | apps/tauri/src-tauri/src/commands/mod.rs#L1614 | tab_id: String, target_path: String, destination_path: String | serde_json::Value | 是 | 低 | 阻塞 | P0 |
| `app_rename_remote_path` | apps/tauri/src-tauri/src/commands/mod.rs#L1644 | tab_id: String, target_path: String, new_name: String | serde_json::Value | 是 | 低 | 阻塞 | P0 |
| `app_delete_remote_path` | apps/tauri/src-tauri/src/commands/mod.rs#L1666 | tab_id: String, target_path: String, target_type: String | serde_json::Value | 是 | 低 | 阻塞 | P0 |
| `app_change_remote_permissions` | apps/tauri/src-tauri/src/commands/mod.rs#L1730 | tab_id: String, target_path: String, options: RemotePermissionChangeOptions | serde_json::Value | 是 | 低 | 阻塞 | P0 |
| `app_set_remote_file_access_mode` | apps/tauri/src-tauri/src/commands/mod.rs#L1761 | tab_id: String, mode: String, options: Option\<serde_json::Value\> | serde_json::Value | 是 | 低 | 阻塞 | P0 |

### 1.5 transfer / 上传下载（9 项）

| 命令名 | 路径#行 | 输入 | 返回 | async | 频率 | 阻塞 | 优先级 |
|---|---|---|---|---|---|---|---|
| `app_queue_upload` | apps/tauri/src-tauri/src/commands/mod.rs#L1789 | file_names: Vec\<String\> | serde_json::Value | 是 | 中 | 阻塞 | P1 |
| `app_upload_file` | apps/tauri/src-tauri/src/commands/mod.rs#L1798 | tab_id: String, local_path: String, remote_directory: String, options: Option\<serde_json::Value\> | serde_json::Value | 是 | 中 | 阻塞 | P1 |
| `app_download_file` | apps/tauri/src-tauri/src/commands/mod.rs#L1823 | tab_id: String, remote_path: String, local_directory: String, options: Option\<serde_json::Value\> | serde_json::Value | 是 | 中 | 阻塞 | P1 |
| `app_download_remote_path` | apps/tauri/src-tauri/src/commands/mod.rs#L1848 | tab_id: String, remote_path: String, target_type: String, local_directory: String, options: Option\<serde_json::Value\> | serde_json::Value | 是 | 中 | 阻塞 | P1 |
| `app_cancel_transfer` | apps/tauri/src-tauri/src/commands/mod.rs#L1880 | transfer_id: String | serde_json::Value | 是 | 中 | 阻塞 | P1 |
| `app_pause_transfer` | apps/tauri/src-tauri/src/commands/mod.rs#L1889 | transfer_id: String | serde_json::Value | 是 | 中 | 阻塞 | P1 |
| `app_resume_transfer` | apps/tauri/src-tauri/src/commands/mod.rs#L1898 | transfer_id: String | serde_json::Value | 是 | 中 | 阻塞 | P1 |
| `app_discard_transfer` | apps/tauri/src-tauri/src/commands/mod.rs#L1907 | transfer_id: String | serde_json::Value | 是 | 中 | 阻塞 | P1 |
| `app_clear_transfers` | apps/tauri/src-tauri/src/commands/mod.rs#L1916 | transfer_ids: Vec\<String\> | serde_json::Value | 是 | 低 | 阻塞 | P1 |

### 1.6 window / 子窗口生命周期（5 项）

| 命令名 | 路径#行 | 输入 | 返回 | async | 频率 | 阻塞 | 优先级 |
|---|---|---|---|---|---|---|---|
| `app_open_window` | apps/tauri/src-tauri/src/commands/mod.rs#L733 | input: OpenWindowInput | () | 是 | 低 | 阻塞 | P1 |
| `app_window_action` | apps/tauri/src-tauri/src/commands/mod.rs#L762 | action: String | () | 是 | 中 | 阻塞 | P1 |
| `app_is_window_maximized` | apps/tauri/src-tauri/src/commands/mod.rs#L856 | (无) | bool | 否 | 低 | 非阻塞 | P1 |
| `app_cancel_file_editor_close` | apps/tauri/src-tauri/src/commands/mod.rs#L861 | (无) | () | 否 | 低 | 非阻塞 | P1 |
| `app_show_window_menu` | apps/tauri/src-tauri/src/commands/mod.rs#L866 | menu_type: String, x: f64, y: f64 | () | 否 | 低 | 半阻塞 | P1 |

### 1.7 tunnel / SSH 端口转发（5 项）

| 命令名 | 路径#行 | 输入 | 返回 | async | 频率 | 阻塞 | 优先级 |
|---|---|---|---|---|---|---|---|
| `app_list_ssh_tunnels` | apps/tauri/src-tauri/src/commands/mod.rs#L1925 | tab_id: String | Vec\<serde_json::Value\> | 是 | 低 | 阻塞 | P1 |
| `app_create_ssh_tunnel` | apps/tauri/src-tauri/src/commands/mod.rs#L1936 | tab_id: String, rule: serde_json::Value | Vec\<serde_json::Value\> | 是 | 低 | 阻塞 | P1 |
| `app_start_ssh_tunnel` | apps/tauri/src-tauri/src/commands/mod.rs#L1949 | tab_id: String, rule_id: String | Vec\<serde_json::Value\> | 是 | 低 | 阻塞 | P1 |
| `app_stop_ssh_tunnel` | apps/tauri/src-tauri/src/commands/mod.rs#L1962 | tab_id: String, rule_id: String | Vec\<serde_json::Value\> | 是 | 低 | 阻塞 | P1 |
| `app_delete_ssh_tunnel` | apps/tauri/src-tauri/src/commands/mod.rs#L1975 | tab_id: String, rule_id: String | Vec\<serde_json::Value\> | 是 | 低 | 阻塞 | P1 |

### 1.8 webdav / 云同步（4 项）

| 命令名 | 路径#行 | 输入 | 返回 | async | 频率 | 阻塞 | 优先级 |
|---|---|---|---|---|---|---|---|
| `app_get_webdav_sync_config` | apps/tauri/src-tauri/src/commands/mod.rs#L643 | (无) | serde_json::Value | 否 | 低 | 半阻塞 | P2 |
| `app_set_webdav_sync_config` | apps/tauri/src-tauri/src/commands/mod.rs#L648 | input: serde_json::Value | serde_json::Value | 否 | 低 | 半阻塞 | P2 |
| `app_upload_webdav_sync` | apps/tauri/src-tauri/src/commands/mod.rs#L656 | (无) | serde_json::Value | 是 | 低 | 阻塞 | P2 |
| `app_download_webdav_sync` | apps/tauri/src-tauri/src/commands/mod.rs#L661 | (无) | serde_json::Value | 是 | 低 | 阻塞 | P2 |

### 1.9 ssh-key / 密钥管理（5 项）

| 命令名 | 路径#行 | 输入 | 返回 | async | 频率 | 阻塞 | 优先级 |
|---|---|---|---|---|---|---|---|
| `app_list_ssh_keys` | apps/tauri/src-tauri/src/commands/mod.rs#L470 | (无) | Vec\<serde_json::Value\> | 否 | 低 | 半阻塞 | P0 |
| `app_select_ssh_key_file` | apps/tauri/src-tauri/src/commands/mod.rs#L475 | (无) | Option\<serde_json::Value\> | 是 | 低 | 阻塞 | P0 |
| `app_import_ssh_key` | apps/tauri/src-tauri/src/commands/mod.rs#L482 | input: Option\<ImportSshKeyInput\> | Option\<serde_json::Value\> | 否 | 低 | 半阻塞 | P0 |
| `app_update_ssh_key_note` | apps/tauri/src-tauri/src/commands/mod.rs#L498 | key_id: String, note: String | serde_json::Value | 否 | 低 | 半阻塞 | P0 |
| `app_delete_ssh_key` | apps/tauri/src-tauri/src/commands/mod.rs#L509 | key_id: String | () | 否 | 低 | 半阻塞 | P0 |

### 1.10 command-template / 命令模板 CRUD + 执行（6 项）

| 命令名 | 路径#行 | 输入 | 返回 | async | 频率 | 阻塞 | 优先级 |
|---|---|---|---|---|---|---|---|
| `app_update_command_folder` | apps/tauri/src-tauri/src/commands/mod.rs#L2088 | folder_id: String, updates: serde_json::Value | serde_json::Value | 是 | 低 | 阻塞 | P1 |
| `app_delete_command_folder` | apps/tauri/src-tauri/src/commands/mod.rs#L2099 | folder_id: String | serde_json::Value | 是 | 低 | 阻塞 | P1 |
| `app_update_command_order` | apps/tauri/src-tauri/src/commands/mod.rs#L2109 | id: String, new_parent_id: Option\<String\>, new_order: f64 | serde_json::Value | 是 | 低 | 阻塞 | P1 |
| `app_update_command_template` | apps/tauri/src-tauri/src/commands/mod.rs#L2121 | command_id: String, input: serde_json::Value | serde_json::Value | 是 | 低 | 阻塞 | P1 |
| `app_delete_command_template` | apps/tauri/src-tauri/src/commands/mod.rs#L2132 | command_id: String | serde_json::Value | 是 | 低 | 阻塞 | P1 |
| `app_execute_command_template` | apps/tauri/src-tauri/src/commands/mod.rs#L2149 | tab_id: String, command_id: String, args: Option\<Vec\<String\>\>, options: Option\<Value\> | Value | 是 | 中 | 阻塞 | P1 |

### 1.11 ui-state / UI 偏好 / 命令历史 / 发送偏好（9 项）

| 命令名 | 路径#行 | 输入 | 返回 | async | 频率 | 阻塞 | 优先级 |
|---|---|---|---|---|---|---|---|
| `app_get_ui_preferences` | apps/tauri/src-tauri/src/commands/mod.rs#L240 | (无) | UiPreferences | 否 | 低 | 半阻塞 | P2 |
| `app_set_ui_preferences` | apps/tauri/src-tauri/src/commands/mod.rs#L257 | input: UiPreferencesInput | UiPreferences | 否 | 低 | 半阻塞 | P2 |
| `app_get_ui_state_item` | apps/tauri/src-tauri/src/commands/mod.rs#L327 | key: String | Option\<String\> | 否 | 低 | 半阻塞 | P2 |
| `app_set_ui_state_item` | apps/tauri/src-tauri/src/commands/mod.rs#L335 | key: String, value: String | () | 否 | 低 | 半阻塞 | P2 |
| `app_remove_ui_state_item` | apps/tauri/src-tauri/src/commands/mod.rs#L342 | key: String | () | 否 | 低 | 半阻塞 | P2 |
| `app_get_terminal_command_history` | apps/tauri/src-tauri/src/commands/mod.rs#L349 | profile_id: String | Vec\<TerminalCommandHistoryEntry\> | 否 | 低 | 半阻塞 | P2 |
| `app_set_terminal_command_history` | apps/tauri/src-tauri/src/commands/mod.rs#L366 | profile_id: String, entries: Vec\<TerminalCommandHistoryEntry\> | () | 否 | 中 | 半阻塞 | P2 |
| `app_get_command_send_preferences` | apps/tauri/src-tauri/src/commands/mod.rs#L389 | (无) | CommandSendPreferences | 否 | 低 | 半阻塞 | P2 |
| `app_set_command_send_preferences` | apps/tauri/src-tauri/src/commands/mod.rs#L404 | preferences: CommandSendPreferences | () | 否 | 低 | 半阻塞 | P2 |

### 1.12 update / 应用更新（4 项）

| 命令名 | 路径#行 | 输入 | 返回 | async | 频率 | 阻塞 | 优先级 |
|---|---|---|---|---|---|---|---|
| `app_get_update_status` | apps/tauri/src-tauri/src/commands/mod.rs#L212 | (无) | serde_json::Value | 是 | 低 | 阻塞 | P2 |
| `app_check_for_updates` | apps/tauri/src-tauri/src/commands/mod.rs#L217 | (无) | serde_json::Value | 是 | 低 | 阻塞 | P2 |
| `app_download_update` | apps/tauri/src-tauri/src/commands/mod.rs#L222 | (无) | () | 是 | 低 | 阻塞 | P2 |
| `app_install_update` | apps/tauri/src-tauri/src/commands/mod.rs#L227 | (无) | () | 是 | 低 | 阻塞 | P2 |

### 1.13 clipboard / external（3 项）

| 命令名 | 路径#行 | 输入 | 返回 | async | 频率 | 阻塞 | 优先级 |
|---|---|---|---|---|---|---|---|
| `app_read_clipboard_text` | apps/tauri/src-tauri/src/commands/mod.rs#L176 | (无) | String | 否 | 中 | 半阻塞 | P2 |
| `app_write_clipboard_text` | apps/tauri/src-tauri/src/commands/mod.rs#L185 | text: String | () | 否 | 中 | 半阻塞 | P2 |
| `app_open_external_url` | apps/tauri/src-tauri/src/commands/mod.rs#L194 | url: String | () | 否 | 低 | 半阻塞 | P2 |

### 1.14 local-files / 本地文件存储核心（12 项）

| 命令名 | 路径#行 | 输入 | 返回 | async | 频率 | 阻塞 | 优先级 |
|---|---|---|---|---|---|---|---|
| `app_list_local_directory` | apps/tauri/src-tauri/src/sessions/local_files.rs#L173 | dir_path: Option\<String\> | DirectorySnapshot | 否 | 中 | 半阻塞 | P0 |
| `app_read_local_file` | apps/tauri/src-tauri/src/sessions/local_files.rs#L238 | file_path: String, encoding: Option\<String\> | String | 否 | 中 | 半阻塞 | P0 |
| `app_write_local_file` | apps/tauri/src-tauri/src/sessions/local_files.rs#L255 | file_path: String, content: String, encoding: Option\<String\> | () | 否 | 中 | 半阻塞 | P0 |
| `app_create_local_directory` | apps/tauri/src-tauri/src/sessions/local_files.rs#L272 | dir_path: String, name: String | () | 否 | 低 | 半阻塞 | P0 |
| `app_create_local_file` | apps/tauri/src-tauri/src/sessions/local_files.rs#L280 | dir_path: String, name: String | () | 否 | 低 | 半阻塞 | P0 |
| `app_copy_local_path` | apps/tauri/src-tauri/src/sessions/local_files.rs#L291 | source_path: String, destination_path: String | () | 否 | 低 | 半阻塞 | P0 |
| `app_move_local_path` | apps/tauri/src-tauri/src/sessions/local_files.rs#L333 | source_path: String, destination_path: String | () | 否 | 低 | 半阻塞 | P0 |
| `app_rename_local_path` | apps/tauri/src-tauri/src/sessions/local_files.rs#L357 | target_path: String, new_name: String | () | 否 | 低 | 半阻塞 | P0 |
| `app_delete_local_path` | apps/tauri/src-tauri/src/sessions/local_files.rs#L368 | target_path: String | () | 否 | 低 | 半阻塞 | P0 |
| `app_change_local_permissions` | apps/tauri/src-tauri/src/sessions/local_files.rs#L408 | target_path: String, options: PermissionChangeOptions | () | 否 | 低 | 半阻塞 | P0 |
| `app_select_local_files` | apps/tauri/src-tauri/src/sessions/local_files.rs#L494 | default_path: Option\<String\> | Vec\<String\> | 是 | 中 | 阻塞 | P0 |
| `app_select_local_directory` | apps/tauri/src-tauri/src/sessions/local_files.rs#L513 | default_path: Option\<String\> | Option\<String\> | 是 | 中 | 阻塞 | P0 |

### 1.15 汇总

- **总数：108**
- 按文件分布：`commands/mod.rs` = 87，`commands/workspace_window.rs` = 8，`sessions/local_files.rs` = 12，`lib.rs` 内 0（`open_child_window` / `open_detached_session_window` 为 `pub fn` 但无 `#[tauri::command]` 标注，仅被 native menu/tray 调用）。
- 按优先级：P0 = 52，P1 = 36，P2 = 20。
- 框架参数剥离：`app: AppHandle` / `state: State<'_, WorkspaceState>` / `window: WebviewWindow` 已从输入列移除；`app_subscribe_terminal_data` 的 `channel: Channel<serde_json::Value>` 是 Tauri IPC 流式通道，属业务订阅参数故保留。GPUI 端此命令变为 `subscribe_terminal_data(&self) -> broadcast::Receiver<TerminalDataPayload>`。
- 非标准返回：`app_get_platform` / `app_get_arch` / `app_get_runtime_version` / `app_is_window_maximized` 直接返回 `String` / `bool` 而非 `Result<T, AppError>`；`app_subscribe_terminal_data` / `app_cancel_file_editor_close` 返回 `()` 无 `Result` 包裹。GPUI 端统一为 `Result<T, AppError>`。

---

## 2. React Component → GPUI View（77 项）

扫描 `apps/tauri/src/renderer/` 全部 `.tsx` 文件，共 77 个 React 组件，分布在 51 个文件中（不含 `main.tsx`，它只做 ReactDOM 挂载）。

**优先级**：P0=主窗口骨架与终端/文件主区；P1=连接/命令管理器与表单；P2=文件编辑器；P3=模态弹窗与基础组件。

### 2.1 布局壳（11 项）

| 组件名 | 路径#行 | 主要 Props | hooks | 依赖 | 多窗口 | 优先级 |
|---|---|---|---|---|---|---|
| App | apps/tauri/src/renderer/App.tsx#L91 | (无 — 取 hooks 与 windowMode) | useWorkspaceIpcSync, useWorkspaceTabs, useWorkspaceModals, useFileOperations, useSshInteractions, useFileEditor, useWorkspaceDataOps, useWorkspaceWindowContext, useThemeMode | CSS variable | main + connection-manager + command-manager + connection-form + command-form + file-editor + detached-session | P0 |
| ModalPortalManager | apps/tauri/src/renderer/features/layout/ModalPortalManager.tsx#L69 | bindings: TabContextMenuBinding & ConnectionManagerBinding & ... | 无 | portal (createPortal), CSS variable | main + file-editor | P0 |
| StandaloneWindowFrame | apps/tauri/src/renderer/features/layout/StandaloneWindowFrame.tsx#L6 | title: string, isWindows: boolean, children?: ReactNode | 无 | CSS variable | connection-manager + command-manager + connection-form + command-form + file-editor + settings | P0 |
| StandaloneWindowTitlebar | apps/tauri/src/renderer/features/layout/StandaloneWindowFrame.tsx#L26 | isWindows: boolean, title: string | useState, useEffect | CSS variable | 同上 | P0 |
| TabBar | apps/tauri/src/renderer/features/layout/TabBar.tsx#L43 | entries: OrderedTabEntry[], ... | usePointerSortFallback | CSS variable | main + detached-session | P0 |
| TabContextMenu | apps/tauri/src/renderer/features/layout/TabContextMenu.tsx#L5 | target: TabContextTarget, ... | 无 | portal (via ContextMenu) | main + detached-session | P1 |
| WindowMenubar | apps/tauri/src/renderer/features/layout/WindowMenubar.tsx#L5 | desktopApi?, isMaximized: boolean | 无 | CSS variable | main（Windows 平台） | P0 |
| TransferBar | apps/tauri/src/renderer/features/transfers/TransferBar.tsx#L3 | activeCount: number, fullWidth?, isPending: boolean, onOpen(): void | 无 | CSS variable | main + detached-session | P0 |
| TransferCenterHost | apps/tauri/src/renderer/features/transfers/TransferCenterHost.tsx#L4 | activeProfileId?, activeTabId?, desktopApi?, ... | 无 | 无 | main + detached-session | P0 |
| TransferCenter | apps/tauri/src/renderer/features/transfers/TransferCenter.tsx#L8 | activeProfileId?, activeTabId?, desktopApi?, ... | useState, useEffect, useRef | CSS variable | main + detached-session | P0 |
| TransferPopover | apps/tauri/src/renderer/features/transfers/TransferPopover.tsx#L7 | transfers: TransferTask[], onClose(), onDiscardTransfer(), onPauseTransfer(), onResumeTransfer(), onClearTransfers() | useState, useEffect, useRef | CSS variable | main + detached-session | P0 |

### 2.2 工作区主区（20 项）

| 组件名 | 路径#行 | 主要 Props | hooks | 依赖 | 多窗口 | 优先级 |
|---|---|---|---|---|---|---|
| WorkspaceStage | apps/tauri/src/renderer/features/workspace/WorkspaceStage.tsx#L25 | activeLocalTab, ... | 无 | CSS variable | main + detached-session | P0 |
| SessionWorkspace | apps/tauri/src/renderer/features/workspace/SessionWorkspace.tsx#L28 | activeTab, ... | useState, useEffect, useRef, useMemo | CSS variable | main + detached-session | P0 |
| HomeWorkspace | apps/tauri/src/renderer/features/workspace/HomeWorkspace.tsx#L19 | ... | useState, useEffect, useMemo | CSS variable | main | P0 |
| OverviewPage | apps/tauri/src/renderer/features/workspace/OverviewPage.tsx#L4 | ... | 无 | CSS variable | main | P0 |
| QuickLinksPage | apps/tauri/src/renderer/features/workspace/QuickLinksPage.tsx#L8 | ... | useState, useMemo | CSS variable | main | P0 |
| SystemSidebarShell | apps/tauri/src/renderer/features/system/SystemSidebarShell.tsx#L5 | ... | 无 | CSS variable | main + detached-session | P0 |
| SystemSidebar | apps/tauri/src/renderer/features/system/SystemSidebar.tsx#L18 | ... | useState, useMemo, useEffect, useRef | CSS variable | main + detached-session | P0 |
| AddressLine | apps/tauri/src/renderer/features/system/SystemSidebar.tsx#L168 | label: string, value: string | 无 | CSS variable | 仅 SystemSidebar | P0 |
| Meter | apps/tauri/src/renderer/features/system/SystemSidebar.tsx#L192 | ... | 无 | CSS variable | 无 | P0 |
| MemoryMeter | apps/tauri/src/renderer/features/system/SystemSidebar.tsx#L224 | metrics?: SystemMetrics | 无 | CSS variable | 无 | P0 |
| CollapsedResourceMeters | apps/tauri/src/renderer/features/system/SystemSidebar.tsx#L295 | metrics?: SystemMetrics | 无 | CSS variable | 无 | P0 |
| ProcessTable | apps/tauri/src/renderer/features/system/SystemSidebar.tsx#L413 | rows: SystemMetrics['topProcesses'] | 无 | CSS variable | 无 | P0 |
| NetworkPanel | apps/tauri/src/renderer/features/system/SystemSidebar.tsx#L498 | metrics?: SystemMetrics | 无 | CSS variable | 无 | P0 |
| SystemInfoWorkspace | apps/tauri/src/renderer/features/system/SystemInfoWorkspace.tsx#L6 | ... | 无 | CSS variable | main | P0 |
| DataCard | apps/tauri/src/renderer/features/system/SystemInfoWorkspace.tsx#L178 | title: string, children: ReactNode | 无 | CSS variable | 无 | P0 |
| DescriptionList | apps/tauri/src/renderer/features/system/SystemInfoWorkspace.tsx#L189 | rows: Array<{ label: string; value: string }> | 无 | CSS variable | 无 | P0 |
| Table | apps/tauri/src/renderer/features/system/SystemInfoWorkspace.tsx#L202 | columns: string[], rows: string[][] | 无 | CSS variable | 无 | P0 |
| SshTunnelPanel | apps/tauri/src/renderer/features/workspace/SshTunnelPanel.tsx#L19 | tabId: string | useState, useEffect, useRef | portal (via TunnelEditorDialog) | main + detached-session | P0 |
| TunnelEditorDialog | apps/tauri/src/renderer/features/workspace/SshTunnelPanel.tsx#L250 | children: ReactNode, isSubmitting: boolean, onClose(): void | 无 | portal (createPortal) | 无 | P0 |
| TunnelRow | apps/tauri/src/renderer/features/workspace/SshTunnelPanel.tsx#L289 | tabId: string, tunnel: SshTunnelSnapshot, onChange(), onError() | useState, useRef | portal (via ConfirmActionDialog) | 无 | P0 |

### 2.3 终端（2 项）

| 组件名 | 路径#行 | 主要 Props | hooks | 依赖 | 多窗口 | 优先级 |
|---|---|---|---|---|---|---|
| TerminalView | apps/tauri/src/renderer/components/TerminalView.tsx#L111 | ... （memo 包裹） | useState, useEffect, useRef | **xterm.js**（Terminal/FitAddon/SearchAddon/Unicode11Addon/WebLinksAddon/OSC 52） | CSS variable | main + detached-session | P0 |
| TerminalDock | apps/tauri/src/renderer/features/terminal/TerminalDock.tsx#L38 | ... | useState, useMemo, useEffect, useRef | CSS variable | main + detached-session | P0 |

### 2.4 文件管理（12 项）

| 组件名 | 路径#行 | 主要 Props | hooks | 依赖 | 多窗口 | 优先级 |
|---|---|---|---|---|---|---|
| FileManager | apps/tauri/src/renderer/features/files/FileManager.tsx#L142 | ... | useState, useMemo, useEffect, useRef | CSS variable | main + detached-session | P0 |
| FileTable | apps/tauri/src/renderer/features/files/FileTables.tsx#L47 | ... | 无 | **@tanstack/react-virtual** | main + detached-session | P0 |
| LocalFileTable | apps/tauri/src/renderer/features/files/FileTables.tsx#L271 | ... | 无 | **@tanstack/react-virtual** | main + detached-session | P0 |
| FileNameCell | apps/tauri/src/renderer/features/files/FileTables.tsx#L375 | ... | useState, useEffect | CSS variable | 无 | P0 |
| PanePathBar | apps/tauri/src/renderer/features/files/FileTables.tsx#L15 | ... | 无 | CSS variable | main + detached-session | P0 |
| FileContextMenu | apps/tauri/src/renderer/features/files/FileContextMenu.tsx#L5 | ... | 无 | portal (via ContextMenu) | main + detached-session | P0 |
| FileActionModal | apps/tauri/src/renderer/features/files/FileActionModal.tsx#L5 | ... | useState, useEffect | CSS variable | main | P1 |
| FilePermissionModal | apps/tauri/src/renderer/features/files/FilePermissionModal.tsx#L15 | ... | useState, useMemo, useEffect | CSS variable | main | P1 |
| PermissionRow | apps/tauri/src/renderer/features/files/FilePermissionModal.tsx#L231 | ... | 无 | CSS variable | 无 | P1 |
| PermissionCell | apps/tauri/src/renderer/features/files/FilePermissionModal.tsx#L262 | ... | 无 | CSS variable | 无 | P1 |
| RootAccessModal | apps/tauri/src/renderer/features/files/RootAccessModal.tsx#L5 | ... | useState, useEffect | CSS variable | main | P1 |
| ConflictResolutionModal | apps/tauri/src/renderer/features/files/ConflictResolutionModal.tsx#L4 | ... | 无 | CSS variable | main | P1 |

### 2.5 连接管理（10 项）

| 组件名 | 路径#行 | 主要 Props | hooks | 依赖 | 多窗口 | 优先级 |
|---|---|---|---|---|---|---|
| ConnectionFormHost | apps/tauri/src/renderer/features/connections/ConnectionFormHost.tsx#L5 | ... | useEffect | CSS variable | main + connection-form | P1 |
| ConnectionModal | apps/tauri/src/renderer/features/connections/ConnectionModal.tsx#L14 | ... | useState | CSS variable | main + connection-form | P1 |
| TunnelRuleEditor | apps/tauri/src/renderer/features/connections/ConnectionModal.tsx#L700 | ... | useState, useEffect, useRef | CSS variable | main + connection-form | P1 |
| ConnectionManagerModal | apps/tauri/src/renderer/features/connections/ConnectionManagerModal.tsx#L15 | profiles, folders, ... | useState, useMemo, useEffect, useRef, usePointerSortFallback | CSS variable | main + connection-manager | P1 |
| ConnectionImportPreviewModal | apps/tauri/src/renderer/features/connections/ConnectionImportPreviewModal.tsx#L5 | plan, onClose(), onCommit() | useMemo, useState, useRef | CSS variable | main | P1 |
| SshCredentialsModal | apps/tauri/src/renderer/features/connections/SshCredentialsModal.tsx#L6 | request, errorMessage?, isSubmitting?, onCancel(), onSubmit() | useState, useEffect | CSS variable | main | P1 |
| SshHostVerificationModal | apps/tauri/src/renderer/features/connections/SshHostVerificationModal.tsx#L5 | request, isSubmitting?, onAcceptAndSave(), onAcceptOnce(), onReject() | 无 | CSS variable | main | P1 |
| SshKeyPassphraseModal | apps/tauri/src/renderer/features/connections/SshKeyPassphraseModal.tsx#L5 | request, errorMessage?, isSubmitting?, onCancel(), onSubmit() | useState, useEffect | CSS variable | main | P1 |
| SshKeyboardInteractiveModal | apps/tauri/src/renderer/features/connections/SshKeyboardInteractiveModal.tsx#L6 | request, errorMessage?, isSubmitting?, onCancel(), onSubmit() | useState, useEffect | CSS variable | main | P1 |
| SshPrivateKeyField | apps/tauri/src/renderer/features/connections/SshPrivateKeyField.tsx#L6 | form: CreateProfileInput, setForm() | useState, useSshKeyLibrary | CSS variable | main + connection-form | P1 |

### 2.6 命令管理（4 项）

| 组件名 | 路径#行 | 主要 Props | hooks | 依赖 | 多窗口 | 优先级 |
|---|---|---|---|---|---|---|
| CommandCenter | apps/tauri/src/renderer/features/commands/CommandCenter.tsx#L10 | activeTab, commandFolders, commandTemplates, isBusy, sendTargets, onExecute(), paneWidth, onPaneWidthChange() | useState, useMemo, useEffect, useRef | CSS variable | main + detached-session | P0 |
| CommandEditorModal | apps/tauri/src/renderer/features/commands/CommandEditorModal.tsx#L61 | folders, initialValue, isSubmitting?, mode, standalone?, onClose(), onSubmit() | useState, useEffect, useMemo, useRef | CSS variable | main + command-form | P1 |
| CommandDialogShell | apps/tauri/src/renderer/features/commands/CommandEditorModal.tsx#L28 | title: string, isSubmitting: boolean, onClose(), children: ReactNode | 无 | CSS variable | 无 | P1 |
| CommandManagerModal | apps/tauri/src/renderer/features/commands/CommandManagerModal.tsx#L15 | commandFolders, commandTemplates, ... | useState, useMemo, useEffect, useRef, usePointerSortFallback | CSS variable | main + command-manager | P1 |

### 2.7 文件编辑器（6 项）

| 组件名 | 路径#行 | 主要 Props | hooks | 依赖 | 多窗口 | 优先级 |
|---|---|---|---|---|---|---|
| FileEditorModal | apps/tauri/src/renderer/features/files/FileEditorModal.tsx#L57 | ... | useState, useEffect, useMemo, useRef | **Monaco**（@monaco-editor/react + monaco-editor + opencc-js） | main + file-editor | P2 |
| EditorMenuButton | apps/tauri/src/renderer/features/files/FileEditorModal.tsx#L456 | ... | 无 | CSS variable | 无 | P2 |
| StatusMenu | apps/tauri/src/renderer/features/files/FileEditorModal.tsx#L485 | ... | 无 | CSS variable | 无 | P2 |
| MenuAction | apps/tauri/src/renderer/features/files/FileEditorModal.tsx#L518 | disabled?: boolean, label: string, onClick(): void | 无 | CSS variable | 无 | P2 |
| MenuToggle | apps/tauri/src/renderer/features/files/FileEditorModal.tsx#L526 | checked: boolean, label: string, onClick(): void | 无 | CSS variable | 无 | P2 |
| MenuSeparator | apps/tauri/src/renderer/features/files/FileEditorModal.tsx#L535 | (无) | 无 | CSS variable | 无 | P2 |

### 2.8 模态与设置（12 项）

| 组件名 | 路径#行 | 主要 Props | hooks | 依赖 | 多窗口 | 优先级 |
|---|---|---|---|---|---|---|
| SettingsModal | apps/tauri/src/renderer/features/settings/SettingsModal.tsx#L6 | theme, onSetTheme(), locale, onSetLocale(), onOpenCommandManager(), onOpenConnectionManager(), onOpenLogsDirectory(), onClose(), standalone?, inline? | useState, useEffect, useRef | CSS variable | main（standalone/inline 两种形态） | P3 |
| SshKeyManagerPage | apps/tauri/src/renderer/features/ssh-keys/SshKeyManagerPage.tsx#L47 | onActiveFolderChange?(name), onStatsChange?(stats) | useState, useMemo, useEffect, useRef, useCallback, useSshKeyLibrary, usePointerSortFallback | CSS variable | main | P1 |
| SshKeyRow | apps/tauri/src/renderer/features/ssh-keys/SshKeyManagerPage.tsx#L895 | item: SshKeyMetadata, className?, draggable?, onDelete(), onEdit(), onDragStart?, ... | 无 | CSS variable | 无 | P1 |
| SshKeyNoteDialog | apps/tauri/src/renderer/features/ssh-keys/SshKeyNoteDialog.tsx#L6 | errorMessage?, folders?, initialFolderId?, initialNote?, initialSourcePath?, isSubmitting, mode, onClose(), onSelectFile?(), onSubmit() | useState, useEffect | portal (via ConfirmActionDialog) | main + connection-form | P1 |
| ConfirmActionDialog | apps/tauri/src/renderer/features/common/ConfirmActionDialog.tsx#L5 | ... | useState, useEffect | portal (createPortal) | 全部 | P3 |
| ContextMenu | apps/tauri/src/renderer/features/common/ContextMenu.tsx#L14 | ... | useLayoutEffect | portal (createPortal) | main + detached-session | P3 |
| CloseButton | apps/tauri/src/renderer/features/common/CloseButton.tsx#L7 | size?: 'compact' \| 'default' \| 'tab' \| 'window', disabled?, onClick(), ariaLabel? | 无 | CSS variable | 全部 | P3 |
| AppIcon | apps/tauri/src/renderer/features/common/AppIcon.tsx#L45 | name: AppIconName, size?: number | 无 | CSS variable | 全部 | P3 |
| VerticalScrollbar | apps/tauri/src/renderer/features/common/VerticalScrollbar.tsx#L16 | ... | useState, useEffect, useRef, useLayoutEffect | CSS variable | 全部 | P3 |
| SessionSendTargetPicker | apps/tauri/src/renderer/features/common/SessionSendTargetPicker.tsx#L7 | ... | useState, useEffect, useRef | portal (createPortal) | main + detached-session | P3 |
| ManagerInlineFolderRow | apps/tauri/src/renderer/features/common/ManagerInlineFolderRow.tsx#L4 | ... | useState, useEffect | CSS variable | main + connection-manager + command-manager | P3 |
| ErrorBoundary | apps/tauri/src/renderer/features/common/ErrorBoundary.tsx#L11 | children: ReactNode | 无（类组件） | CSS variable | 全部 | P0 |

### 2.9 主题（0 项）

FileTerm 没有独立的 `ThemeProvider` / `ThemeToggle` 组件：

- 主题切换在 `apps/tauri/src/renderer/App.tsx` 内通过 `useThemeMode` hook 处理。
- 主题色通过 CSS 变量驱动（如 `--sidebar-width`、`--file-panel-height`、`--terminal-bg` 等）。
- 主题预览卡位于 `SettingsModal.tsx` 第 185-223 行（general tab 中的 `.theme-card`）。

GPUI 端需要补一个 `Entity<AppTheme>` 全局主题实体（详见第 4 节）。

### 2.10 汇总

- **总数：77**，分布在 51 个 `.tsx` 文件中。
- 按分组：布局壳 11 / 工作区主区 20 / 终端 2 / 文件管理 12 / 连接管理 10 / 命令管理 4 / 文件编辑器 6 / 模态与设置 12 / 主题 0。
- 按优先级：P0 = 44，P1 = 21，P2 = 6，P3 = 6。
- **xterm.js 依赖**：仅 `TerminalView` 1 处。
- **Monaco 依赖**：仅 `FileEditorModal` 1 处（含 opencc-js 繁简转换）。
- **portal (createPortal)**：9 处。GPUI 端用 `cx.open_popover` / `cx.open_modal` 替代。
- **多窗口复用热点**：`CloseButton` / `AppIcon` / `ErrorBoundary` / `ConfirmActionDialog` / `VerticalScrollbar` 被几乎所有窗口复用。
- **`@tanstack/react-virtual`**：`FileTable` / `LocalFileTable` 用。GPUI 端用 `uniform_list` / `gpui::list_state` 替代。

---

## 3. React Hook → GPUI Entity 映射（7 项）

Tauri 端 `App.tsx` 通过 7 个 hooks 拆分状态。GPUI 端等价映射为 `Entity<T>`：

| Tauri hook                  | GPUI Entity                              | 状态内容                                                       | 订阅源                       |
| --------------------------- | ---------------------------------------- | -------------------------------------------------------------- | ---------------------------- |
| `useWorkspaceTabs`          | `Entity<WorkspaceTabsState>`             | tabs 列表、active tab、tab 顺序                                | `workspace:snapshot` 广播    |
| `useWorkspaceModals`        | `Entity<ModalRegistry>`                  | 当前打开的 modal 栈、modal props                               | view 内部交互                |
| `useFileOperations`         | `Entity<FileOpsState>`                   | 当前目录、selected paths、剪贴板、renaming 状态                | view 内部交互 + bridge fn    |
| `useSshInteractions`        | `Entity<SshInteractionQueue>`            | 待响应的 ssh interaction 请求队列                              | `ssh:interaction` 广播       |
| `useFileEditor`             | `Entity<FileEditorRegistry>`             | 打开的编辑器实例、未保存改动标记                               | view 内部交互                |
| `useWorkspaceIpcSync`       | `Entity<SnapshotSync>`                   | 最新 snapshot、UI preferences                                  | broadcast→Entity 桥接        |
| `useWorkspaceDataOps`       | `Entity<DataOpsState>`                   | 加载状态、错误信息                                             | view 内部交互                |
| `useWorkspaceWindowContext` | `Entity<WindowContext>`                  | 当前窗口 ID、isMaximized、placements                           | `workspace:placements-changed` 广播 |
| `useThemeMode`              | `Entity<AppTheme>`                       | 当前主题（dark/light）、locale                                 | `app:ui-preferences-changed` 广播 |

桥接模式见 [gpui-refactor.md](./gpui-refactor.md) 第 4.3.2 节。

---

## 4. Tauri Event → Broadcast Channel（14 项）

扫描 `apps/tauri/src-tauri/src/` 下所有 `app.emit(` / `window.emit(` / `app.emit_to(` 调用，与 `apps/tauri/src/bridge/tauri-api.ts` 中 `subscribe(...)` 监听对应。

| 事件名 | payload 类型（Rust + TS） | 发出位置#行 | 监听位置 (tauri-api.ts#行) | 频率 | GPUI 路由方式 |
|---|---|---|---|---|---|
| `terminal:state` | Rust: `json!{tabId, summary, transcript, connected}`<br>TS: `TerminalStatePayload` | sessions/terminal.rs#L80<br>sessions/ssh.rs#L904 | subscribe('terminal:state') #471 | 高 | per-tab broadcast::Sender\<TerminalState\> |
| `terminal:data` | （Tauri Channel，非 emit）<br>Rust: `TerminalDataPayload`<br>TS: `TerminalDataPayload` | sessions/ssh.rs#L915 内 `publish_terminal_output` 走 channel | subscribeTerminalData #72-78 | 高 | per-tab broadcast::Sender\<TerminalChunk\>，背压策略见 refactor 4.4 |
| `workspace:snapshot` | Rust: `WorkspaceSnapshot`<br>TS: `WorkspaceSnapshot` | sessions/terminal.rs#L90<br>sessions/ftp.rs#L456<br>sessions/ssh.rs#L907/#954/#996/#2944/#3474<br>commands/mod.rs#L668/#920/#1272<br>services/transfers.rs#L323/#1701<br>services/connections.rs#L554 | subscribe('workspace:snapshot') #473 | 中 | 全局 broadcast::Sender\<WorkspaceSnapshot\>，所有 WorkspaceView 订阅 |
| `workspace:sessionMetrics` | Rust: `json!{tabId, systemMetrics, mode:"append"}`<br>TS: `SessionMetricsUpdate` | sessions/ssh.rs#L3148 | subscribe('workspace:sessionMetrics') #474-475 | 高（采集任务运行时按 chunk） | per-tab broadcast::Sender\<SystemMetrics\> |
| `ssh:interaction` | Rust: `json!{requestId, kind, tabId, profileId, ...}`<br>TS: `SshInteractionRequest` union | sessions/ssh.rs#L323 (host-verification)<br>#L1716 (credentials)<br>#L2333 (key-passphrase)<br>#L2400 (keyboard-interactive) | subscribe('ssh:interaction') #476 | 低（按需） | 按 tabId 路由 + oneshot::channel 收集响应，超时 60s |
| `transfer:update` | Rust: `TransferTask`<br>TS: `TransferTask` | services/transfers.rs#L320 | subscribe('transfer:update') #472 | 高（传输进度 tick） | 全局 broadcast::Sender\<TransferTask\> |
| `app:update-status` | Rust: `AppUpdateStatus`<br>TS: `AppUpdateStatus` | services/updates.rs#L53/#L350 | subscribe('app:update-status') #224 | 低 | 主窗口 RootView 独占订阅 |
| `app:ui-preferences-changed` | Rust: `&UiPreferences`<br>TS: `{ theme, locale }` | commands/mod.rs#L291 | subscribe('app:ui-preferences-changed') #466 | 低 | 全局 broadcast::Sender\<UiPreferences\>，所有窗口订阅 |
| `app:window-maximized-change` | Rust: `bool`<br>TS: `boolean` | lib.rs#L1209 (Resized)<br>lib.rs#L1395 (window-toggle-maximize)<br>commands/mod.rs#L785 | subscribe('app:window-maximized-change') #467-468 | 低 | 同窗口 WindowContext 独占订阅 |
| `app:window-close-request` | Rust: `json!{ isQuit: bool }`<br>TS: `{ isQuit: boolean }` | lib.rs#L1046 (detached CloseRequested)<br>lib.rs#L1142 (request_main_window_close)<br>commands/mod.rs#L794 | subscribe('app:window-close-request') #478-479 | 低 | 定向：`window.update(cx, \|root, cx\| root.handle_close_request(cx))` |
| `app:close-active-workspace-item-request` | Rust: `()`<br>TS: `void` | lib.rs#L489 (label == "main") | subscribe('app:close-active-workspace-item-request') #480-481 | 低 | 主窗口 RootView 独占订阅 |
| `app:file-editor-close-request` | Rust: `()`<br>TS: `void` | lib.rs#L161<br>lib.rs#L923 | subscribe('app:file-editor-close-request') #469 | 低 | 定向：file-editor-{hash} 窗口 |
| `sshKeys:changed` | Rust: `Vec<SshKeyMetadata>`<br>TS: `SshKeyMetadata[]` | commands/mod.rs#L515 | subscribe('sshKeys:changed') #477 | 低 | 全局 broadcast::Sender\<Vec\<SshKeyMetadata\>\> |
| `workspace:placements-changed` | Rust: `&[WorkspaceTabPlacement]`<br>TS: `WorkspaceTabPlacement[]` | commands/workspace_window.rs#L73<br>lib.rs#L1077 | subscribe('workspace:placements-changed') #482-483 | 中（拖拽 / detach / 窗口销毁时） | 全局 broadcast::Sender\<Vec\<WorkspaceTabPlacement\>\> |

### 4.1 汇总

- **总数：14**（含 `terminal:data` 这个 channel-based 事件）。
- 按频率：高 4（terminal:data / terminal:state / workspace:sessionMetrics / transfer:update）；中 2（workspace:snapshot / workspace:placements-changed）；低 8。
- **必须 broadcast**（多窗口订阅）：terminal:data / terminal:state / workspace:sessionMetrics / workspace:snapshot / workspace:placements-changed / ssh:interaction / app:ui-preferences-changed / app:window-close-request / sshKeys:changed。
- **可定向**（单窗口订阅）：app:update-status / app:window-maximized-change / app:close-active-workspace-item-request / app:file-editor-close-request / transfer:update（当前主窗口唯一订阅，未来若支持 detached 传输面板则升级为 broadcast）。

---

## 5. CSS Token → GPUI Hsla / f32 / BoxShadow（315 项）

扫描 `apps/tauri/src/renderer/styles/themes/` 下所有 `.css` 文件，共 315 个 CSS 自定义属性。

- `tokens.css`：10 个主题无关常量（4 radius + 3 spacing + 3 shadow）。
- `default-dark.css`：151 个主题相关 token。
- `default-light.css`：154 个 token（151 与 dark 同名 + 3 个 light 独有）。
- `index.css`：0 个（仅 `@import`）。

GPUI 表示建议：颜色 = `Hsla`；半径/间距/字号 = `f32`（按 px 解析）；阴影 = `Vec<BoxShadow>` where `BoxShadow { color: Hsla, offset: Point<f32>, blur_radius: f32, spread_radius: f32, inset: bool }`；动画 = `duration_ms`（本目录无动画 token）。

### 5.1 tokens.css（10 项，主题无关）

| token 名 | 值 | 文件#行 | 类别 | GPUI 表示 |
|---|---|---|---|---|
| `--radius-sm` | `4px` | tokens.css#L2 | radius | f32 = 4.0 |
| `--radius-md` | `6px` | tokens.css#L3 | radius | f32 = 6.0 |
| `--radius-lg` | `10px` | tokens.css#L4 | radius | f32 = 10.0 |
| `--window-corner-radius` | `14px` | tokens.css#L5 | radius | f32 = 14.0 |
| `--button-height-md` | `36px` | tokens.css#L7 | spacing | f32 = 36.0 |
| `--button-padding-x-md` | `20px` | tokens.css#L8 | spacing | f32 = 20.0 |
| `--button-gap-md` | `8px` | tokens.css#L9 | spacing | f32 = 8.0 |
| `--shadow-sm` | `0 1px 2px rgba(15, 23, 42, 0.06)` | tokens.css#L11 | shadow | `vec![BoxShadow { color: hsla(222.0, 0.32, 0.11, 0.06), offset: (0.0, 1.0).into(), blur: 2.0, spread: 0.0, inset: false }]` |
| `--shadow-md` | `0 6px 16px rgba(15, 23, 42, 0.08)` | tokens.css#L12 | shadow | `vec![BoxShadow { color: hsla(222.0, 0.32, 0.11, 0.08), offset: (0.0, 6.0).into(), blur: 16.0, spread: 0.0, inset: false }]` |
| `--shadow-lg` | `0 18px 42px rgba(15, 23, 42, 0.14)` | tokens.css#L13 | shadow | `vec![BoxShadow { color: hsla(222.0, 0.32, 0.11, 0.14), offset: (0.0, 18.0).into(), blur: 42.0, spread: 0.0, inset: false }]` |

### 5.2 主题相关 token（dark / light 配对，151 项 + 3 项 light 独有 = 154 项）

下表按类别分组，给出 dark 与 light 的取值配对。`var(--xxx)` 引用已展开为最终值。GPUI 端在 `ThemeRegistry` 中定义为 `struct ThemeTokens { dark: ThemeSet, light: ThemeSet }`，切换主题时整体替换。

#### 5.2.1 背景色（8 项）

| token 名 | dark 值 | dark Hsla | light 值 | light Hsla | 主要使用 |
|---|---|---|---|---|---|
| `--bg-main` | `#151515` | hsla(0, 0%, 0.08, 1.0) | `#f4f5f7` | hsla(220, 0.20, 0.96, 1.0) | modals/overview/session/shell/ssh-keys/workstation-skin/global/commands/home/modal-components/ErrorBoundary |
| `--bg-sidebar` | `#242424` | hsla(0, 0%, 0.14, 1.0) | `#ffffff` | hsla(0, 0%, 1.0, 1.0) | 同上 13 个文件 |
| `--bg-card` | `#1e1e1e` | hsla(0, 0%, 0.12, 1.0) | `#ffffff` | hsla(0, 0%, 1.0, 1.0) | 同上 |
| `--bg-elevated` | `#2a2a2a` | hsla(0, 0%, 0.16, 1.0) | `#ffffff` | hsla(0, 0%, 1.0, 1.0) | modals/session/commands/modal-components |
| `--bg-hover` | `#303030` | hsla(0, 0%, 0.19, 1.0) | `#eceff3` | hsla(220, 0.20, 0.94, 1.0) | 同上 13 个文件 |
| `--bg-active` | `#3a3d42` | hsla(222, 0.07, 0.24, 1.0) | `#dde2e8` | hsla(216, 0.18, 0.89, 1.0) | 同上 |
| `--manager-head-bg` | `#242424` | hsla(0, 0%, 0.14, 1.0) | `#f3f6fa` | hsla(210, 0.33, 0.97, 1.0) | shell/commands/modal-components |
| `--input-bg` | `#1a1a1a` | hsla(0, 0%, 0.10, 1.0) | `#f7f8fa` | hsla(220, 0.20, 0.97, 1.0) | modals/session/ssh-keys/workstation-skin/commands/home/modal-components/ErrorBoundary/TerminalView |

#### 5.2.2 边框（3 项）

| token 名 | dark 值 | dark Hsla | light 值 | light Hsla |
|---|---|---|---|---|
| `--border-light` | `rgba(255, 255, 255, 0.08)` | hsla(0, 0%, 1.0, 0.08) | `rgba(15, 23, 42, 0.16)` | hsla(220, 0.47, 0.11, 0.16) |
| `--border-dark` | `rgba(255, 255, 255, 0.16)` | hsla(0, 0%, 1.0, 0.16) | `rgba(15, 23, 42, 0.28)` | hsla(220, 0.47, 0.11, 0.28) |
| `--border` | `var(--border-light)` | 引用 → 同 border-light | `var(--border-light)` | 引用 → 同 border-light |

#### 5.2.3 文本（5 项）

| token 名 | dark 值 | dark Hsla | light 值 | light Hsla |
|---|---|---|---|---|
| `--text-main` | `#e7e7e7` | hsla(0, 0%, 0.91, 1.0) | `#1f2933` | hsla(213, 0.24, 0.16, 1.0) |
| `--text-muted` | `#a4a4a4` | hsla(0, 0%, 0.64, 1.0) | `#66717f` | hsla(210, 0.10, 0.45, 1.0) |
| `--text-soft` | `#8f949d` | hsla(216, 0.06, 0.58, 1.0) | `#7b8794` | hsla(213, 0.10, 0.53, 1.0) |
| `--text-primary` | `var(--text-main)` | 引用 → text-main | `var(--text-main)` | 引用 → text-main |
| `--text-secondary` | `var(--text-muted)` | 引用 → text-muted | `var(--text-muted)` | 引用 → text-muted |

#### 5.2.4 主色与语义色（18 项）

| token 名 | dark 值 | dark Hsla | light 值 | light Hsla |
|---|---|---|---|---|
| `--primary` | `#6b737d` | hsla(216, 0.07, 0.45, 1.0) | `#4f7cff` | hsla(221, 1.0, 0.65, 1.0) |
| `--primary-hover` | `#7f8894` | hsla(216, 0.08, 0.54, 1.0) | `#3d69ea` | hsla(222, 0.79, 0.58, 1.0) |
| `--accent-primary` | `#8bbfff` | hsla(213, 1.0, 0.76, 1.0) | `var(--primary)` | 引用 → primary |
| `--accent-text` | `#c8d0da` | hsla(216, 0.18, 0.82, 1.0) | `#3f4b59` | hsla(213, 0.16, 0.30, 1.0) |
| `--sidebar-active-accent` | `#ffffff` | hsla(0, 0%, 1.0, 1.0) | `var(--text-main)` | 引用 → text-main |
| `--selection-bg` | `#383f48` | hsla(220, 0.13, 0.25, 1.0) | `#dfe5ec` | hsla(216, 0.20, 0.90, 1.0) |
| `--copy-link` | `#65a9ff` | hsla(210, 1.0, 0.70, 1.0) | `#4f7cff` | hsla(221, 1.0, 0.65, 1.0) |
| `--copy-link-hover` | `#8bbfff` | hsla(213, 1.0, 0.76, 1.0) | `#2f5fef` | hsla(222, 0.83, 0.56, 1.0) |
| `--folder-accent` | `#65a9ff` | hsla(210, 1.0, 0.70, 1.0) | `#3b82f6` | hsla(220, 0.91, 0.59, 1.0) |
| `--kernel-accent` | `#65a9ff` | hsla(210, 1.0, 0.70, 1.0) | `#2563eb` | hsla(220, 0.91, 0.53, 1.0) |
| `--mini-tab-active-bg` | `#294366` | hsla(212, 0.43, 0.28, 1.0) | `#dbeafe` | hsla(213, 1.0, 0.86, 1.0) |
| `--mini-tab-active-text` | `#8bbfff` | hsla(213, 1.0, 0.76, 1.0) | `#1d4ed8` | hsla(221, 0.83, 0.48, 1.0) |
| `--memory-warn` | `#ffcc00` | hsla(48, 1.0, 0.50, 1.0) | `#f2b000` | hsla(45, 1.0, 0.47, 1.0) |
| `--network-tx` | `#ff7474` | hsla(0, 1.0, 0.73, 1.0) | `rgba(239, 68, 68, 0.32)` | hsla(0, 0.85, 0.60, 0.32) |
| `--network-rx` | `#65a9ff` | hsla(210, 1.0, 0.70, 1.0) | `#3b82f6` | hsla(220, 0.91, 0.59, 1.0) |
| `--danger` | `#ff5f57` | hsla(2, 1.0, 0.68, 1.0) | `#c93d3d` | hsla(0, 0.56, 0.51, 1.0) |
| `--success` | `#39d98a` | hsla(153, 0.69, 0.54, 1.0) | `#168a53` | hsla(149, 0.74, 0.31, 1.0) |
| `--warning` | `#ffcc00` | hsla(48, 1.0, 0.50, 1.0) | `#b77900` | hsla(42, 1.0, 0.36, 1.0) |

#### 5.2.5 状态面/边/文本（8 项）

| token 名 | dark 值 | dark Hsla | light 值 | light Hsla |
|---|---|---|---|---|
| `--danger-text` | `#ff8f88` | hsla(2, 1.0, 0.77, 1.0) | `#b93838` | hsla(0, 0.56, 0.47, 1.0) |
| `--danger-surface` | `rgba(255, 95, 87, 0.12)` | hsla(2, 1.0, 0.67, 0.12) | `rgba(201, 61, 61, 0.08)` | hsla(0, 0.56, 0.51, 0.08) |
| `--danger-border` | `rgba(255, 95, 87, 0.3)` | hsla(2, 1.0, 0.67, 0.30) | `rgba(201, 61, 61, 0.26)` | hsla(0, 0.56, 0.51, 0.26) |
| `--info` | `var(--accent-primary)` | 引用 → accent-primary | `var(--primary)` | 引用 → primary |
| `--info-text` | `var(--accent-primary)` | 引用 → accent-primary | `#2f5fef` | hsla(222, 0.83, 0.56, 1.0) |
| `--info-surface` | `rgba(139, 191, 255, 0.12)` | hsla(213, 1.0, 0.77, 0.12) | `rgba(79, 124, 255, 0.1)` | hsla(221, 1.0, 0.65, 0.10) |
| `--info-border` | `rgba(139, 191, 255, 0.32)` | hsla(213, 1.0, 0.77, 0.32) | `rgba(79, 124, 255, 0.28)` | hsla(221, 1.0, 0.65, 0.28) |
| `--input-focus-ring` | `rgba(255, 255, 255, 0.08)` | hsla(0, 0%, 1.0, 0.08) | `rgba(79, 124, 255, 0.18)` | hsla(221, 1.0, 0.65, 0.18) |

#### 5.2.6 accent/success 色阶（6 项）

| token 名 | dark 值 | dark Hsla | light 值 | light Hsla |
|---|---|---|---|---|
| `--accent-tint-weak` | `rgba(64, 150, 255, 0.08)` | hsla(210, 1.0, 0.63, 0.08) | `rgba(79, 124, 255, 0.1)` | hsla(221, 1.0, 0.65, 0.10) |
| `--accent-tint` | `rgba(64, 150, 255, 0.15)` | hsla(210, 1.0, 0.63, 0.15) | `rgba(79, 124, 255, 0.16)` | hsla(221, 1.0, 0.65, 0.16) |
| `--accent-focus-ring` | `rgba(64, 150, 255, 0.25)` | hsla(210, 1.0, 0.63, 0.25) | `rgba(79, 124, 255, 0.28)` | hsla(221, 1.0, 0.65, 0.28) |
| `--success-text` | `#34c759` | hsla(145, 0.58, 0.49, 1.0) | `#168a53` | hsla(149, 0.74, 0.31, 1.0) |
| `--success-surface` | `rgba(52, 199, 89, 0.15)` | hsla(145, 0.58, 0.49, 0.15) | `rgba(22, 138, 83, 0.12)` | hsla(149, 0.74, 0.31, 0.12) |
| `--success-border` | `rgba(52, 199, 89, 0.32)` | hsla(145, 0.58, 0.49, 0.32) | `rgba(22, 138, 83, 0.28)` | hsla(149, 0.74, 0.31, 0.28) |

#### 5.2.7 中性叠层与阴影（21 项，节选关键 6 项）

完整 21 项见 `default-dark.css#L63-L83` 与 `default-light.css#L52-L75`。GPUI 端定义为 `Vec<BoxShadow>`，结构示例：

```rust
// apps/gpui/src/theme/tokens.rs
pub struct BoxShadow {
    pub color: Hsla,
    pub offset: gpui::Point<f32>,
    pub blur_radius: f32,
    pub spread_radius: f32,
    pub inset: bool,
}

pub struct ThemeSet {
    pub bg_main: Hsla,
    pub bg_sidebar: Hsla,
    // ... 151 个字段
    pub modal_card_shadow: Vec<BoxShadow>,
    pub floating_drawer_shadow: Vec<BoxShadow>,
    // ...
}
```

关键阴影 token 示例：

| token 名 | dark 值 | light 值 |
|---|---|---|
| `--surface-hover` | `rgba(255, 255, 255, 0.04)` | `rgba(15, 23, 42, 0.05)` |
| `--surface-chip` | `rgba(255, 255, 255, 0.1)` | `rgba(15, 23, 42, 0.08)` |
| `--surface-inset` | `rgba(255, 255, 255, 0.03)` | `rgba(15, 23, 42, 0.03)` |
| `--modal-backdrop-bg` | `rgba(0, 0, 0, 0.62)` | `rgba(15, 23, 42, 0.28)` |
| `--modal-card-shadow` | `0 20px 60px rgba(0, 0, 0, 0.5)` | `0 20px 60px rgba(15, 23, 42, 0.18)` |
| `--floating-drawer-shadow` | `0 4px 12px rgba(0, 0, 0, 0.25), 0 1px 3px rgba(0, 0, 0, 0.15)` | `0 10px 26px rgba(15, 23, 42, 0.14), 0 1px 2px rgba(15, 23, 42, 0.1)` |

#### 5.2.8 窗口控制与文件编辑器（19 项，节选关键 8 项）

完整 19 项见 `default-dark.css#L84-L102` 与 `default-light.css#L76-L94`。

| token 名 | dark 值 | dark Hsla | light 值 | light Hsla |
|---|---|---|---|---|
| `--traffic-close` | `#ff5f57` | hsla(2, 1.0, 0.68, 1.0) | `#ff5f57` | 同 dark |
| `--traffic-minimize` | `#ffbd2e` | hsla(43, 1.0, 0.58, 1.0) | `#ffbd2e` | 同 dark |
| `--traffic-maximize` | `#27c93f` | hsla(130, 0.69, 0.47, 1.0) | `#27c93f` | 同 dark |
| `--window-control-danger-bg` | `#c42b1c` | hsla(5, 0.74, 0.44, 1.0) | `#c42b1c` | 同 dark |
| `--window-control-danger-text` | `#ffffff` | hsla(0, 0%, 1.0, 1.0) | `#ffffff` | 同 dark |
| `--button-primary-bg` | `#1687e8` | hsla(210, 0.83, 0.50, 1.0) | `#4f7cff` | hsla(221, 1.0, 0.65, 1.0) |
| `--button-primary-hover` | `#1179d4` | hsla(210, 0.86, 0.45, 1.0) | `#3d69ea` | hsla(222, 0.79, 0.58, 1.0) |
| `--button-primary-text` | `#ffffff` | hsla(0, 0%, 1.0, 1.0) | `#ffffff` | 同 dark |

#### 5.2.9 Monaco 编辑器（10 项）

仅 `FileEditorModal.tsx` 使用。GPUI 端用 GPUI text editor 替代 Monaco 后，这些 token 映射为 editor 主题：

| token 名 | dark 值 | light 值 | 用途 |
|---|---|---|---|
| `--monaco-editor-bg` | `#111316` | `#111316` | editor 背景 |
| `--monaco-editor-foreground` | `#d6dde7` | `#d6dde7` | editor 前景 |
| `--monaco-line-number` | `#5f6875` | `#5f6875` | 行号 |
| `--monaco-line-number-active` | `#9faab8` | `#9faab8` | 当前行号 |
| `--monaco-cursor` | `#7cc7ff` | `#7cc7ff` | 光标 |
| `--monaco-selection` | `#21466b` | `#21466b` | 选区 |
| `--monaco-inactive-selection` | `#1a354d` | `#1a354d` | 非活动选区 |
| `--monaco-line-highlight` | `#161b22` | `#161b22` | 当前行高亮 |
| `--monaco-indent-guide` | `#1f2630` | `#1f2630` | 缩进辅助线 |
| `--monaco-indent-guide-active` | `#344150` | `#344150` | 活动缩进辅助线 |

注：Monaco 主题在 dark 与 light 下取值相同，因为 Monaco 编辑器在 FileTerm 中始终用 dark 主题。

#### 5.2.10 连接类型语义色（11 项）

仅 `overview.css` 使用，用于概览页统计卡片：

| token 名 | dark 值 | light 值 |
|---|---|---|
| `--type-total` | `#60a5fa` | `#2563eb` |
| `--type-total-surface` | `rgba(96, 165, 250, 0.15)` | `rgba(37, 99, 235, 0.1)` |
| `--type-ssh` | `#34d399` | `#059669` |
| `--type-ssh-surface` | `rgba(52, 211, 153, 0.15)` | `rgba(5, 150, 105, 0.1)` |
| `--type-sftp` | `#38bdf8` | `#0284c7` |
| `--type-sftp-surface` | `rgba(56, 189, 248, 0.15)` | `rgba(2, 132, 199, 0.1)` |
| `--type-ftp` | `#c084fc` | `#9333ea` |
| `--type-ftp-surface` | `rgba(192, 132, 252, 0.15)` | `rgba(147, 51, 234, 0.1)` |
| `--type-folder` | `#fb923c` | `#ea580c` |
| `--type-folder-surface` | `rgba(251, 146, 60, 0.15)` | `rgba(234, 88, 12, 0.1)` |
| `--type-muted` | `#4b5563` | `#64748b` |

#### 5.2.11 终端与命令面板（14 项，节选关键 8 项）

完整 14 项见 `default-dark.css#L120-L133` 与 `default-light.css#L158-L171`。

| token 名 | dark 值 | light 值 |
|---|---|---|
| `--terminal-bg` | `#181818` | `#ffffff` |
| `--terminal-text` | `#e0e0e0` | `#111827` |
| `--terminal-cmd-bg` | `rgba(148, 163, 184, 0.16)` | `rgba(15, 23, 42, 0.08)` |
| `--terminal-cmd-text` | `#f1f5f9` | `#111827` |
| `--terminal-selection-bg` | `rgba(148, 163, 184, 0.24)` | `rgba(79, 124, 255, 0.22)` |
| `--terminal-search-match-bg` | `#4b5563` | `#f6cf57` |
| `--terminal-search-active-bg` | `#ffd43b` | `#ffd43b` |
| `--terminal-frame-shadow` | `inset 0 0 14px rgba(0, 0, 0, 0.3)` | `none` |

> 终端 16 色 ANSI 调色板未在 CSS token 中显式定义，而是硬编码在 `TerminalView.tsx` 的 `xterm` theme 配置中。GPUI 端在 `theme/terminal_palette.rs` 中定义为 `[Hsla; 16]`，dark 与 light 分别一份。需在 spike 阶段从 `TerminalView.tsx` 抽取实际取值。

#### 5.2.12 指标色（6 项）

| token 名 | dark 值 | light 值 |
|---|---|---|
| `--metric-app` | `#f6bf26` | `#f2b000` |
| `--metric-cache` | `#54c772` | `#168a53` |
| `--metric-kernel` | `#65a9ff` | `#1e88e5` |
| `--metric-status-green` | `#39d98a` | `#168a53` |
| `--metric-status-yellow` | `#f6bf26` | `#f2b000` |
| `--metric-status-red` | `#ff5f57` | `#d94e4e` |

#### 5.2.13 Popover 与 ConfirmDialog（30 项）

完整 30 项见 `default-dark.css#L141-L170` 与 `default-light.css#L127-L156`。GPUI 端用 `cx.open_popover` 与 `cx.open_modal` 替代 portal，主题 token 仍按上述方式映射。表头与按钮 token 数量较多但结构简单（颜色 = Hsla、阴影 = `Vec<BoxShadow>`），此处不再展开。

#### 5.2.14 light 独有（3 项）

| token 名 | 值 | Hsla | 用途 |
|---|---|---|---|
| `--system-sidebar-frame` | `rgba(15, 23, 42, 0.32)` | hsla(220, 0.47, 0.11, 0.32) | light 主题下 system-sidebar 的边框 |
| `--system-sidebar-control-border` | `rgba(15, 23, 42, 0.26)` | hsla(220, 0.47, 0.11, 0.26) | light 主题下 system-sidebar 控件边框 |
| `--system-sidebar-divider` | `rgba(15, 23, 42, 0.18)` | hsla(220, 0.47, 0.11, 0.18) | light 主题下 system-sidebar 分割线 |

GPUI 端 `ThemeSet` 中这三个字段在 dark 主题下设为 `None` 或回退到 `border_light`。

### 5.3 汇总

- **总数：315**（10 主题无关 + 151 dark + 154 light）。
- dark 主题 151 项中：颜色 122 / 阴影 18 / 引用 11。
- light 主题 154 项中：3 项 light 独有，其余 151 项与 dark 同名（其中 10 项 Monaco 在 light 下取值与 dark 相同）。
- **GPUI ThemeRegistry 端**：dark 与 light 各一个 `ThemeSet` struct，约 160 个字段（含派生引用展开）；切换主题时整体替换 `Entity<AppTheme>`，调用 `cx.notify()` 触发全局重渲染。

---

## 6. 待评审问题

1. 315 个 token 是否需要在 GPUI 端 100% 复刻？还是按使用频率分批，P0 组件用到的 ~80 个 token 在 G2 前完成？
2. Monaco 主题在 dark/light 下取值相同，GPUI editor 是否也保持一致（始终 dark）？
3. 终端 16 色 ANSI 调色板未在 CSS token 中显式定义，是否需要在 spike 阶段从 `TerminalView.tsx` 抽取并补到 token 体系？
4. `app_subscribe_terminal_data` 在 GPUI 端变为返回 `broadcast::Receiver`，是否需要在 bridge trait 中保留这个方法，还是直接由 `WorkspaceState` 暴露？
