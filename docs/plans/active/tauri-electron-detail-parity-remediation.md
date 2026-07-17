# Tauri / Electron 细节功能对齐计划

## 背景

本计划用于收口 Tauri renderer 与 Rust backend 相比 Electron 版本仍存在的细节行为差异。
重点不是补齐 API 数量，而是修复同一 API 在数据结构、窗口生命周期、会话状态、异步反馈和
多窗口同步上的语义偏差。

本计划基于 2026-07-17 当前未提交工作区继续推进；既有
`tauri-code-audit-remediation-2026-07-17.md` 负责后端完整性与迁移，本计划不重复覆盖其改动。

## 已确认当前成立的能力

- [x] 连接管理器、命令管理器与 SSH Key 管理器已接入 pointer sort fallback。
- [x] 终端断开后按 Enter 重连链路已闭环到 Rust worker。
- [x] Monaco 保存按钮已有 spinner、disabled、read-only 与真实写入等待。
- [x] 连接/命令删除确认框已有提交态。
- [x] `sync:version` 已覆盖 Cargo.toml、Cargo.lock 与 tauri.conf.json。
- [x] Tauri 原生文件拖放路径 fallback 已改为消费后清理，不再派发后立即丢失。

## P0：崩溃与数据丢失

### A. 创建数据契约

- [ ] 新建连接文件夹必须写入 `type: "folder"`、`order` 和规范化 `parentId`。
- [ ] 新建命令文件夹必须写入 `type: "command-folder"`、`order` 和规范化 `parentId`。
- [ ] 新建命令必须写入 `type: "command-template"`、`order`、`command` 与
      `appendCarriageReturn` 默认值。
- [ ] 读取旧数据时自愈上述缺失字段，避免已经落盘的坏数据继续导致 renderer 崩溃。
- [ ] 为创建与旧数据自愈补 Rust contract/unit tests。

### B. 全应用退出与独立 Monaco 编辑器

- [ ] Cmd+Q、Alt+F4、托盘退出和原生菜单退出必须在 shutdown 前逐个请求独立编辑器关闭。
- [ ] 任一编辑器取消丢弃时必须中止整个退出，不停止 session/transfer worker。
- [ ] 全局退出等待期间要防止重复退出请求和重复关闭弹窗。
- [ ] 隐藏的脏编辑器收到退出询问时必须恢复可见并聚焦。
- [ ] 为关闭请求去重、同意与取消结果补 Rust tests。

## P1：行为契约对齐

### C. 会话状态

- [ ] Rust 只发布 core `TabStatus` 允许的 `idle/connecting/connected/error/closed`。
- [ ] 主动断开、正常远端断开映射为 `closed`；连接/worker 失败映射为 `error`。
- [ ] 修复右键 Connect、标签状态点和 System Info 对断开状态的判断。
- [ ] 为 SSH/FTP/Telnet/Serial 状态映射补回归测试。

### D. SSH 认证与初始路径

- [ ] password 模式密码为空时回退到 system authentication。
- [ ] system authentication 在 Agent 之外尝试默认私钥：`id_ed25519`、`id_ecdsa`、
      `id_rsa`、`id_dsa`。
- [ ] Windows 无 Agent 时仍允许默认私钥认证，不直接返回平台不支持。
- [ ] SSH session、首次 SFTP 列表与 shell CWD 使用 profile `remotePath`，不硬编码 `/`。
- [ ] 为认证候选顺序和 remotePath 归一化补单元测试；真实认证保留 fixture 验收。

### E. 多窗口一致性

- [ ] profile/folder/command 的全部增删改排序在持久化成功后广播 `workspace:snapshot`。
- [ ] `openProfile` 立即广播 connecting snapshot，独立管理器和主窗口同步显示。
- [ ] 统一 mutation command 的 snapshot 返回与广播逻辑，避免漏发或重复发。
- [ ] 补 command contract 与多窗口事件测试。

### F. 批量命令错误与重复弹窗

- [ ] 检查 `Promise.allSettled` 的 rejected 结果；保留成功目标，同时汇报失败目标数量与原因。
- [ ] 删除重复渲染的 SSH keyboard-interactive modal。
- [ ] 批量普通命令与命令模板使用同一错误汇总策略。

## P2：异步反馈与小交互

### G. Loading 与防重入

- [ ] ConnectionModal 保存期间禁用关闭、取消、字段与重复提交，并展示 spinner。
- [ ] CommandEditorModal 等待持久化成功后再关闭，失败时保留表单和错误。
- [ ] FilePermissionModal 在递归 chmod 期间禁用关闭与重复应用。
- [ ] SSH credentials、host verification、keyboard-interactive、key passphrase 在提交期间防重入。
- [ ] WebDAV 保存/上传/下载分别展示操作态并阻止重复请求。
- [ ] 导入 commit 期间禁止 backdrop、关闭与取消绕过。
- [ ] 删除 profile/command 失败时不得因上层吞错而自动关闭确认框。
- [ ] Monaco 对 Cmd/Ctrl+S 增加同步 guard，保存期间关闭先等待或明确确认。

### H. 右键菜单、拖拽与平台菜单

- [ ] 终端读取不到文本剪贴板时安静返回并恢复终端焦点。
- [ ] 通用 ContextMenu 增加 menu/menuitem 语义及方向键、Home/End 导航。
- [ ] 原生菜单和自定义窗口菜单统一命令管理器快捷键。
- [ ] 原生菜单标题跟随已持久化 locale 构建。
- [ ] pointer sort 在 window blur、pointer capture 丢失时可靠清理 ghost/state。
- [ ] TabBar 从高风险 HTML5 DnD 迁移到 pointer sort，保留键盘操作。
- [ ] macOS/Windows/Linux 打包态分别验证标签排序和外部文件拖放。

## 验证门禁

- `npm run typecheck`
- `npm run lint -- --max-warnings=0`
- `npm run format:check`
- `npm run test:tauri`
- `cargo fmt --all -- --check --manifest-path apps/tauri/src-tauri/Cargo.toml`
- `cargo clippy --locked --all-targets --all-features --manifest-path apps/tauri/src-tauri/Cargo.toml -- -D warnings`
- 打包态手测：全局退出脏编辑器、管理器拖拽、标签拖拽、右键重连、剪贴板无文本、
  SSH 空密码/default key/remotePath、多窗口 CRUD 同步。

## 完成定义

- P0/P1 自动化回归全部通过，且 Electron 行为差异有明确测试或文档化例外。
- P2 中可以自动化的 renderer 交互进入测试；只能由原生 WebView 验证的项目保留三平台验收记录。
- 完成后将本文件移动到 `docs/plans/completed/`，不在 `AGENTS.md` 堆积过程信息。
