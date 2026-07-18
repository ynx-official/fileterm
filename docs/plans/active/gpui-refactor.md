# FileTerm GPUI 重构计划

| 项目       | 值                                                                                  |
| ---------- | ----------------------------------------------------------------------------------- |
| 文档版本   | v0.3（拆分草案，待评审）                                                            |
| 更新日期   | 2026-07-18                                                                          |
| 状态       | 规划中，未进入实施                                                                  |
| 编写人     | 架构组                                                                              |
| 适用分支   | `gpui`                                                                              |
| 仓库根目录 | `/workspace`                                                                        |
| 关联文档   | [gpui-migration-inventory.md](./gpui-migration-inventory.md)、[gpui-spike.md](./gpui-spike.md)、`docs/architecture.md`、`docs/design.md` |

> 本文档是 GPUI 重构的总览。逐项迁移清单（108 command / 77 component / 14 event / 315 token）见 `gpui-migration-inventory.md`；Phase G-1 终端 spike 的可开工工单见 `gpui-spike.md`。

---

## 1. 背景与目标

### 1.1 为什么做 GPUI 重构

FileTerm 当前唯一受维护的运行时是 `apps/tauri`（Tauri v2 + Rust + React + xterm.js + Monaco）。Tauri 解决了 Electron 的 V8 主进程开销，但 renderer 仍依赖 WebView2/WKWebView，带来三类长期成本：

1. **WebView 渲染管线**：xterm.js 与 Monaco 在大规模文本下需要靠 WebGL canvas + DOM overlay 维持 60fps，WebView 合成层开销不可忽视。
2. **IPC 序列化边界**：终端输出、传输进度、系统指标都经过 `invoke` / `Channel` 序列化，高频流式数据有可观测的合并延迟（已通过 16ms batcher + 持久 Channel 优化，但本质仍是跨进程）。
3. **组件耦合**：xterm.js 与 Monaco 都是 web-only，主题、字体度量、IME、DPI 要分别在 WebView 与原生之间处理。

GPUI 是 Zed 编辑器使用的 GPU 加速混合渲染框架（即时 + 保留模式），通过 `gpui-unofficial` v1.9.0 已在 crates.io 可用，支持 macOS（Metal）/ Linux（Wayland + X11）/ Windows（Win32 + DirectWrite）。本计划描述把 FileTerm 的桌面 UI 与交互方式用 GPUI 完整复刻一版，作为与 Tauri 并存的第三 runtime。

### 1.2 复刻目标

本次重构是**等价复刻**：在保留 Tauri 主链路稳定发行的前提下，让 GPUI 版本达到与 Tauri 相同的交互与视觉表现。

- **UI 一致**：顶部标签栏、左侧系统侧栏、终端 + 文件面板主区、底部传输栏、所有模态弹窗、独立子窗口（连接管理器、命令管理器、文件编辑器、可拆分会话窗口）的布局、间距、字号、配色都向 Tauri 看齐。
- **交互一致**：键盘快捷键、拖拽（标签拖出分离、原生文件拖入上传）、上下文菜单、focus 模式、抽屉式面板、原生菜单与托盘行为对齐。
- **协议一致**：SSH/SFTP、FTP/FTPS、Telnet、Serial、WebDAV、Transfer journal、CWD/sudo 联动、SSH 隧道等能力**完全等价**，数据格式（`profiles.json` / `profile-secrets.json` / `ssh-keys.json` / `ssh-key-secrets.json` / `transfer-journal.json` / `ui-state.json` / `ui-preferences.json`）保持兼容，用户可在两个 runtime 之间无缝切换。
- **主题一致**：CSS 变量链路（`tokens → theme vars → component skins → terminal colors`）映射为 GPUI 的 `Hsla` 设计 token；明暗主题、终端配色、Monaco 替代编辑器配色都从同一份 token 派生。

### 1.3 非目标

- 不替换 Tauri：Tauri 仍为主发行链路，GPUI 作为实验性并存 runtime。
- 不重写协议层：russh / russh-sftp / suppaftp / tokio-serial / reqwest 等协议依赖直接 fork 复用。
- 不引入新业务能力：所有 GPUI 工作都对应 Tauri 已有能力。
- 不实现 GPUI 自动更新链路：v1 仅支持手动下载替换；签名 updater 留作后续单独评估。

---

## 2. 关键决策

| 决策点         | 选择                              | 理由                                                                                            |
| -------------- | --------------------------------- | ----------------------------------------------------------------------------------------------- |
| 共存策略       | **并存为第三 runtime**            | 风险最低，可分阶段迁移；与现有 Tauri/Electron 双 runtime 模式一致                                |
| Rust 后端复用  | **GPUI fork 一份**                | 避免短期大重构 `apps/tauri/src-tauri/src/{sessions,services,storage}`；以拷贝方式起步，独立演进 |
| 终端模拟器     | **vte + 自研 GPUI 终端元素**      | 对标 alacritty/zed terminal，性能最好、可控；避开 alacritty_terminal crate API 不稳定           |
| 代码编辑器     | **GPUI text editor + tree-sitter**| v1 支持基本编辑/保存/语法高亮；多光标、智能补全等 Monaco 高级能力作为已知债务                   |
| Spike 预研     | **Phase G-1 先验证终端**          | 终端是性能风险最高点，先做最小 spike 验证 vte + GPUI 能否在 `yes`/`find /`/`htop` 场景达 60fps  |

### 2.1 待评审决策（建议默认值）

| 决策点                | 推荐默认                                                                                          |
| --------------------- | ------------------------------------------------------------------------------------------------- |
| 托盘与原生菜单        | 自研薄壳：macOS 用 `objc2` crate，Windows 用 `windows-sys`，Linux 暂用应用内菜单                  |
| 字体资源              | 复用 `apps/tauri/src/renderer/assets/fonts/` 的 Inter / JetBrains Mono / Material Symbols         |
| i18n                  | 复用 `apps/tauri/src/renderer/i18n.ts` 的 zhCN/enUS 字典，迁移到 Rust 简易 `&'static str` 静态表   |
| WebDAV / OpenCC       | 复用 reqwest；OpenCC 用 `opencc-rs` crate                                                          |
| 数据目录              | 与 Tauri 共享 `~/Library/Application Support/FileTerm`（macOS）/`%APPDATA%/FileTerm`（Windows）   |
| 测试策略              | GPUI 视图层用 `gpui::TestAppContext`；协议层复用 `.github/fixtures/tauri-socket-lifecycle/` 夹具  |

---

## 3. 影响范围

### 3.1 新增代码

```txt
apps/gpui/                                # GPUI 应用主目录（workspace member）
  Cargo.toml
  build.rs                                # 平台资源打包（图标、字体、Info.plist）
  src/
    main.rs                               # Application::run 入口
    app.rs                                # 根 App state + Render
    backend/                              # fork 自 apps/tauri/src-tauri/src/
      sessions/                           # ssh/ftp/telnet/serial/local_files（直接拷贝）
      services/                           # workspace/transfers/connections/ssh_keys/updates/logging
      storage/                            # JSON 文件原子读写（与 Tauri 共享数据目录）
      commands/                           # 原 #[tauri::command] 函数体抽为普通 async fn
    bridge/                               # FileTermDesktopApi trait + GpuiDesktopApi impl
    state/                                # Entity<T> 状态层（WorkspaceState/ModalRegistry/...）
    window/                               # 多窗口管理 + WindowRegistry
    view/                                 # GPUI View（对应 React 组件）
      layout/  workspace/  terminal/  files/
      connections/  commands/  editor/  settings/  common/
    theme/                                # tokens.rs + dark.rs + light.rs + skins.rs + terminal_palette.rs
    platform/                             # macOS/Windows/Linux 平台分支（托盘、菜单、文件对话框）
    i18n.rs                               # 静态字典
  tests/                                  # GPUI TestAppContext 视图测试 + 协议夹具复用
```

### 3.2 fork 范围

直接拷贝并改写的代码：

- `apps/tauri/src-tauri/src/sessions/` → `apps/gpui/src/backend/sessions/`
- `apps/tauri/src-tauri/src/services/` → `apps/gpui/src/backend/services/`（移除 `tauri::AppHandle` 依赖，改用自定义 `AppHandle` 类型别名）
- `apps/tauri/src-tauri/src/storage/` → `apps/gpui/src/backend/storage/`
- `apps/tauri/src-tauri/src/commands/` 中 `#[tauri::command]` 函数体 → 普通 `pub async fn`，签名不变

### 3.3 不变范围

- `packages/core`、`packages/storage`、`packages/shared`：Tauri 与 GPUI 共享类型定义（仅 TS 端使用，GPUI 端用 serde 派生等价 Rust 类型）。
- `apps/tauri/`：Tauri 主链路完全不动，照常发行。
- `apps/electron/`：历史代码不动。
- 用户数据目录：两个 runtime 共用同一目录，互不干扰。

---

## 4. 总体架构

### 4.1 分层

```txt
GPUI View (Render + Element)
  ↓ 调用
Bridge (in-process async fn, FileTermDesktopApi trait)
  ↓ 委托
Backend Services (fork from Tauri)
  ↓ 驱动
Session Controllers (russh / suppaftp / tokio-serial)
  ↓
Protocol Adapters → Remote Servers
```

与 Tauri 的核心差异：

- **没有 IPC 边界**：bridge 是同进程 async fn，不再有序列化开销。终端输出、传输进度、系统指标直接走 `tokio::broadcast` 或 GPUI 的 `Entity::update` + `cx.notify()`。
- **没有 WebView**：所有渲染走 GPUI 的 GPU 管线，字体度量、IME、DPI 由 GPUI 统一处理。
- **没有 Tauri events**：用 `tokio::broadcast::Sender<T>` 替代 `app.emit()`；订阅端用 `Entity::subscribe` 或 `cx.spawn` 桥接。

### 4.2 多窗口消息路由

Tauri 当前的多窗口模型由 `WindowRegistry`（`apps/tauri/src-tauri/src/services/workspace_window.rs`）+ `app.emit()` / `window.emit()` 双通道构成。GPUI 复刻这一模型时必须显式区分**广播事件**与**定向事件**，否则会在多窗口下出现泄漏或路由错误。

#### 4.2.1 窗口清单与生命周期

| 窗口 label                  | 数量       | 装饰            | 持有 tab | 关闭策略                                            |
| --------------------------- | ---------- | --------------- | -------- | --------------------------------------------------- |
| `main`                      | 1          | macOS server / Win client | 是       | `Cmd+Q` 走 quit 流程；`Cmd+W` 走 close-active-item  |
| `connection-manager`        | 0 或 1     | client          | 否       | 直接销毁                                            |
| `command-manager`           | 0 或 1     | client          | 否       | 直接销毁                                            |
| `connection-form`           | 0 或 1     | client          | 否       | 重建即销毁原窗口                                    |
| `command-form`              | 0 或 1     | client          | 否       | 重建即销毁原窗口                                    |
| `file-editor-{hash}`        | 0..N       | client          | 否       | 有未保存改动时阻止关闭，emit `app:file-editor-close-request` |
| `detached-session-{id}`     | 0..N       | server          | 是       | `CloseRequested` 阻止原生关闭，emit `app:window-close-request`；`Destroyed` 时归还 tab 给 main + 广播 placements |
| 托盘菜单                    | 全局       | N/A             | N/A      | 平台分支                                            |

#### 4.2.2 事件路由表

| 事件                                | Tauri 端发出方式                        | GPUI 端路由方式                                                                                       |
| ----------------------------------- | --------------------------------------- | ----------------------------------------------------------------------------------------------------- |
| `workspace:snapshot`                | `app.emit()` 全局                       | `broadcast::Sender<WorkspaceSnapshot>`，所有 `WorkspaceView` 各持一份 receiver                       |
| `workspace:placements-changed`      | `app.emit()` 全局                       | `broadcast::Sender<Vec<WorkspaceTabPlacement>>`，所有窗口的 `WindowContext` 都订阅                    |
| `workspace:sessionMetrics`          | `app.emit()` 全局（payload 含 tabId）   | 按 tabId 路由：`per-tab broadcast::Sender<SystemMetrics>`，只有持有该 tab 的窗口订阅                  |
| `terminal:data`                     | Tauri Channel（每窗口独立）             | `per-tab broadcast::Sender<TerminalChunk>`，只有该 tab 的 `TermView` 订阅；背压策略见 4.4              |
| `terminal:state`                    | `app.emit()` 全局（payload 含 tabId）   | 按 tabId 路由：`per-tab broadcast::Sender<TerminalState>`                                              |
| `transfer:update`                   | `app.emit()` 全局                       | `broadcast::Sender<TransferTask>`，所有 `TransferCenter` 视图订阅                                     |
| `ssh:interaction`                   | `app.emit()` 全局（payload 含 tabId）   | 按 tabId 路由到持有该 tab 的窗口；用 `oneshot::channel` 收集响应，超时 60s                            |
| `app:ui-preferences-changed`        | `app.emit()` 全局                       | `broadcast::Sender<UiPreferences>`，所有窗口订阅以刷新原生菜单与 locale                              |
| `app:window-close-request`          | `window.emit()` 定向                    | GPUI 用 `window.update(cx, |view, cx| view.handle_close_request(cx))` 直接调用目标窗口的根 view      |
| `app:file-editor-close-request`     | `window.emit()` 定向                    | 同上，定向到 `file-editor-{hash}` 窗口                                                                |
| `app:close-active-workspace-item-request` | `app.emit()` 主窗口                | 主窗口 `RootView` 独占订阅                                                                            |
| `app:window-maximized-change`       | `window.emit()` 定向                    | 同窗口的 `WindowContext` 独占订阅                                                                     |
| `app:update-status`                 | `app.emit()` 全局                       | 主窗口订阅                                                                                            |
| `sshKeys:changed`                   | `app.emit()` 全局                       | `broadcast::Sender<Vec<SshKeyMetadata>>`，manager 窗口与 connection-form 都订阅                      |

#### 4.2.3 WindowRegistry

```rust
// apps/gpui/src/window/registry.rs
use std::collections::HashMap;
use gpui::{Entity, WindowHandle};
use parking_lot::RwLock;

pub struct WindowRegistry {
    /// detached 窗口持有的 tab 列表；main 窗口的 tab 由 WorkspaceState 单独管理
    detached_tabs: RwLock<HashMap<String /* window_id */, Vec<String /* tab_id */>>>,
    /// tab_id -> window_id 反查
    tab_owner: RwLock<HashMap<String, String>>,
    /// window_id -> WindowHandle
    handles: RwLock<HashMap<String, WindowHandle<RootView>>>,
}

impl WindowRegistry {
    pub fn detach_tab(&self, tab_id: &str, target_window_id: &str) { /* ... */ }
    pub fn return_tabs_to_main(&self, window_id: &str) -> Vec<String> { /* ... */ }
    pub fn window_for_tab(&self, tab_id: &str) -> Option<WindowHandle<RootView>> { /* ... */ }
    pub fn list_placements(&self) -> Vec<WorkspaceTabPlacement> { /* ... */ }
}
```

#### 4.2.4 detached-session 关闭链路

复刻 Tauri 的三段式关闭：

1. 用户点击关闭 → GPUI 触发 `on_window_should_close` 回调。
2. 回调中 `prevent_close`，调用 `window.update(cx, |root, cx| root.handle_close_request(cx))`，由 RootView 决定是否弹"未保存"对话框。
3. RootView 决定关闭 → `window.remove_window()`；GPUI 触发 `on_window_removed` → `WindowRegistry::return_tabs_to_main(window_id)` + `broadcast placements-changed`。

崩溃恢复路径：如果窗口被系统强杀（GPUI 收不到 `on_window_removed`），启动时扫描 `WindowRegistry.detached_tabs` 中存在但 `handles` 中已不存在的 window_id，归还其 tab。

### 4.3 Entity 与 broadcast 的边界

GPUI 提供两套状态原语，必须明确分工，否则要么丢失更新要么性能崩塌：

| 原语                          | 适用场景                                          | 频率上限        | 订阅生命周期                       |
| ----------------------------- | ------------------------------------------------- | --------------- | ---------------------------------- |
| `Entity<T>` + `cx.notify()`   | UI 状态（选中、悬停、表单输入、modal 开关）       | 每帧最多一次    | Entity 强引用持有即订阅            |
| `tokio::broadcast::Sender<T>` | 异步数据流（终端输出、传输进度、snapshot）        | 无上限          | `Receiver::drop` 即退订            |
| `oneshot::channel`            | 单次请求-响应（ssh interaction、文件对话框结果）  | 一次            | `Sender::send` 后 channel 关闭     |

#### 4.3.1 判定规则

- **数据源在 backend 异步任务中产生，且需要被多个 view 共享** → broadcast。
- **数据源在 view 自身的交互中产生，且只与本 view 相关** → Entity。
- **数据源在 backend 异步任务中产生，但只与单个 view 相关** → broadcast + 1 个 receiver（不要为了"省一个 channel"把它做成 Entity，否则会阻塞 backend 任务）。

#### 4.3.2 桥接模式

broadcast → Entity 的桥接是 GPUI 端的标准模式：

```rust
// apps/gpui/src/state/snapshot_sync.rs
use gpui::{Entity, ModelContext};
use tokio::sync::broadcast;

pub struct SnapshotSync {
    pub snapshot: WorkspaceSnapshot,
}

impl SnapshotSync {
    pub fn spawn(
        mut rx: broadcast::Receiver<WorkspaceSnapshot>,
        entity: Entity<Self>,
    ) {
        entity.update(&mut ModelContext::default(), |_, cx| {
            cx.spawn(async move |this, mut cx| {
                while let Ok(snapshot) = rx.recv().await {
                    let _ = this.update(&mut cx, |state, cx| {
                        state.snapshot = snapshot;
                        cx.notify();
                    });
                }
            }).detach();
        });
    }
}
```

关键约束：

- **桥接任务必须 detach**：不能 join，否则会阻塞 GPUI 主循环。
- **`cx.notify()` 在 update 闭包内调用**：保证订阅了 `Entity<Self>` 的 view 在下一帧重渲染。
- **高频流的桥接要加节流**：终端输出每秒可能上百条 broadcast，桥接里用 `tokio::time::interval(Duration::from_millis(16))` 合并，只保留最后一条；详见 4.4。

#### 4.3.3 不引入 Zustand / Redux

Tauri 端的 ADR-0004 决策是 hooks 方案已足够。GPUI 端的 `Entity<T>` + `cx.notify()` 等价于 React hooks 的 `useState` + `useSyncExternalStore` 组合，足以覆盖当前 7 个 hooks（`useWorkspaceTabs` / `useWorkspaceModals` / `useFileOperations` / `useSshInteractions` / `useFileEditor` / `useWorkspaceIpcSync` / `useWorkspaceDataOps`）。映射表见 `gpui-migration-inventory.md` 第 3 节。

### 4.4 终端渲染管线

终端是性能风险最高点，必须把 PTY → 屏幕像素的每一步都讲清楚。Phase G-1 spike 就是为了验证这条管线。

```txt
┌──────────────┐   bytes   ┌──────────────┐  ANSI events  ┌──────────────┐
│ portable-pty │ ─────────>│  vte::Parser │ ─────────────>│ TermPerform  │
└──────────────┘           └──────────────┘                └──────────────┘
                                                                │
                                                                │ mutate
                                                                ▼
                                     ┌────────────────────────────────────┐
                                     │ TermModel                          │
                                     │  - grid: Vec<Row>  (visible)       │
                                     │  - scrollback: RingBuffer<Row>     │
                                     │  - alt_grid: Option<Vec<Row>>      │
                                     │  - cursor: Cursor { row, col, .. } │
                                     │  - sgr: SgrState (current attr)    │
                                     │  - dirty_rows: RangeSet<usize>     │
                                     └────────────────────────────────────┘
                                                                │
                                                                │ notify
                                                                ▼
                                     ┌────────────────────────────────────┐
                                     │ Entity<TermModel>                  │
                                     │  broadcast::Receiver<TermChunk>    │
                                     │  + 16ms throttle                   │
                                     └────────────────────────────────────┘
                                                                │
                                                                │ render (animation frame)
                                                                ▼
                                     ┌────────────────────────────────────┐
                                     │ TermView                           │
                                     │  1. 取 visible rows（scrollback +  │
                                     │     grid + alt_grid）              │
                                     │  2. 按 SGR 属性合并同色 run         │
                                     │  3. 每个 run 调 ShapedLine::shape  │
                                     │  4. 先画背景色 quad，再画 shaped   │
                                     │     text                           │
                                     │  5. cursor + selection 作 overlay  │
                                     └────────────────────────────────────┘
                                                                │
                                                                ▼
                                                          GPUI GPU pipeline
```

#### 4.4.1 各阶段职责

- **PTY**：`portable-pty` crate（wezterm 抽出，跨平台）。提供 `Master`/`Slave` 句柄，`Master` 写入用户输入，`Slave` 是子进程的 stdin/stdout/stderr。SSH 场景下不使用 PTY，直接用 russh 的 channel 读写字节流；Spike 阶段先验证本地 PTY 路径。
- **vte::Parser**：状态机，按字节消费输入，触发 `Perform` trait 回调。无内存分配，性能接近零成本。
- **TermPerform**：实现 `vte::Perform`，把 CSI/SGR/OSC 回调翻译成对 `TermModel` 的 mutation。OSC 7（CWD）、OSC 52（剪贴板）、1337（RemoteUser）必须解析，对应 Tauri 端的 `shell-cwd-integration.ts`。
- **TermModel**：cell grid + scrollback ring buffer。关键约束：
  - `scrollback` 上限默认 10000 行，超出 LRU 淘汰；可配置。
  - `alt_grid` 用于 vim/tmux 全屏模式，退出 alt 时丢弃内容。
  - `dirty_rows` 用 `RangeSet<usize>` 跟踪未渲染的行，避免每帧全量重绘。
  - resize 时重新分配 grid，保留可保留内容（参考 alacritty `Term::resize`）。
- **Entity<TermModel>**：包裹 `TermModel`，提供 `cx.notify()` 通知。同时持有一个 `broadcast::Receiver<TermChunk>`，spawn 一个节流任务把 chunk feed 给 model。
- **TermView**：GPUI View，`Render::render` 中读取 model 的 visible rows，按 SGR run 分组，每组用 `ShapedLine::shape` 一次（缓存 shape 结果，dirty rows 变化才重新 shape）。

#### 4.4.2 背压与节流

终端输出可能瞬时超过渲染能力（如 `yes` 每秒数 MB）。背压链路：

1. `broadcast::Sender` 容量 256 条 `TermChunk`（每条最大 8KB）。
2. 队列满时 sender 走 `send_timeout(50ms)`，超时丢弃最旧 chunk 并计数（`dropped_chunks` 暴露给 status bar）。
3. 桥接任务用 `tokio::time::interval(16ms)` 合并所有可用 chunk，一次性 feed 给 `TermModel`。
4. `cx.notify()` 在合并完成后调用一次，保证每帧最多一次重渲染。
5. 渲染时只重绘 `dirty_rows` 范围，其他行复用上一帧的 `ShapedLine` 缓存。

#### 4.4.3 性能预算与验收

| 场景                 | 输入速率       | 目标帧率 | 验收阈值                                              |
| -------------------- | -------------- | -------- | ----------------------------------------------------- |
| 80×24，4KB/s         | 慢速日志       | 60fps    | 帧时间 < 16ms，无掉帧                                 |
| 80×24，1MB/s `yes`   | 极速输出       | 30fps+   | 帧时间 < 33ms，允许丢 chunk 但用户可见行无丢失        |
| 200×50，`find /`     | 大量短行       | 60fps    | 帧时间 < 16ms，scrollback 滚动流畅                    |
| `htop` 全屏刷新      | 1Hz 全屏重绘   | 60fps    | 帧时间 < 16ms，CPU < 5%                               |
| `vim` alt screen     | 交互式         | 60fps    | 进入/退出 alt 无闪烁，cursor blink 不卡               |

详细 spike 工单与代码骨架见 [gpui-spike.md](./gpui-spike.md)。

---

## 5. 实施阶段（高层）

每阶段的可执行子任务、文件路径、验收用例见对应文档。

| Phase | 名称                | 目标                                                       | 文档                              |
| ----- | ------------------- | ---------------------------------------------------------- | --------------------------------- |
| G-1   | 终端 Spike          | 验证 vte + GPUI 终端管线在 4.4.3 阈值内                    | [gpui-spike.md](./gpui-spike.md)  |
| G0    | 脚手架              | `apps/gpui/` workspace member、空主窗口、bridge trait 骨架 | 本文档第 6.1 节                   |
| G1    | 存储 fork           | 拷贝 `storage/` + `commands/` 函数体，单测覆盖             | 本文档第 6.2 节                   |
| G2    | 桌面壳              | 多窗口 + WindowRegistry + 托盘 + 原生菜单                  | 本文档第 6.3 节                   |
| G3    | SSH 终端主链路      | open_profile + 终端 + 系统侧栏                              | 本文档第 6.4 节                   |
| G4    | 文件管理 + 传输     | SFTP + 本地文件 + Transfer journal                         | 本文档第 6.5 节                   |
| G5    | 可拆分窗口 + 发行   | detached-session + 三平台打包                              | 本文档第 6.6 节                   |

### 6.1 Phase G0：脚手架

- 把 `apps/gpui/` 加入根 `Cargo.toml` workspace。
- 写最小 `Application::run`，打开一个空主窗口。
- 定义 `FileTermDesktopApi` trait（签名见 inventory 第 1 节）。
- 写 `GpuiDesktopApi` 空实现（全部返回 `AppError::Unsupported`）。
- 验收：`cargo run -p fileterm-gpui` 能打开空白窗口；`cargo test -p fileterm-gpui` 通过。

### 6.2 Phase G1：存储 fork

- 拷贝 `apps/tauri/src-tauri/src/storage/` 到 `apps/gpui/src/backend/storage/`。
- 把 `tauri::AppHandle` 替换为自定义 `AppHandle` 类型别名（实际是 `Arc<AppContext>`）。
- 拷贝 `apps/tauri/src-tauri/src/commands/mod.rs` 与 `commands/workspace_window.rs`、`sessions/local_files.rs` 中所有 `#[tauri::command]` 函数体，去掉宏标注。
- 验收：原 Tauri 端 contract test 仓库夹具能在 GPUI 端通过。

### 6.3 Phase G2：桌面壳

- 实现 `WindowRegistry`（4.2.3）。
- 实现 `open_main_window`、`open_child_window`、`open_detached_session_window`。
- 实现 `TrayHandler` trait（macOS/Windows/Linux 三分支）。
- 实现 `build_application_menu`。
- 验收：能打开 7 种窗口、托盘菜单可点击、原生菜单可触发快捷键。

### 6.4 Phase G3：SSH 终端主链路

- 接入 `russh` shell channel，把 SSH 输出喂给 `TermModel`。
- 实现 `TermView`（4.4）。
- 实现 `TerminalDock`（命令输入栏 + 历史回放 + 路径补全）。
- 实现 `SystemSidebar`（CPU/内存/网络/进程表）。
- 验收：能连接真实 SSH 主机，终端在 4.4.3 阈值内；CWD 跟随触发文件面板切换目录。

### 6.5 Phase G4：文件管理 + 传输

- 接入 `russh-sftp` 实现 SFTP list/read/write/rename/delete/chmod。
- 实现 `FileManager` + `FileTable`（虚拟滚动）+ `FileContextMenu`。
- 接入 `TransferService`（journal + 断点续传 + 暂停恢复）。
- 实现 `TransferCenter` + `TransferPopover`。
- 验收：上传/下载/暂停/恢复/取消全链路；冲突对话框与权限对话框可用。

### 6.6 Phase G5：可拆分窗口 + 发行

- 实现 `workspace_detach_tab` + detached-session 窗口 + 标签拖拽。
- 三平台打包脚本（macOS DMG / Windows NSIS / Linux AppImage）。
- 验收：标签能拖出到新窗口、能拖回；三平台安装包可手动安装运行。

---

## 7. 风险与缓解

| 风险                                      | 缓解                                                                                  |
| ----------------------------------------- | ------------------------------------------------------------------------------------- |
| GPUI API 在 `gpui-unofficial` 下不稳定    | 锁定 1.9 版本；fork 后只在 major bump 时升级                                          |
| 终端 spike 不达 60fps                     | G-1 阶段就验收；不达标则改用 `alacritty_terminal` crate 或退回 Tauri 终端             |
| 多窗口路由泄漏（tab owner 不一致）        | `WindowRegistry` 提供 `assert_consistency()` 在 debug build 每帧检查                  |
| fork 后协议代码与 Tauri 主链路长期分叉    | 每月 cherry-pick Tauri 的协议修复；diff 超过 500 行则评估回抽 `crates/fileterm-core-rs` |
| GPUI IME 在 Linux Wayland 下不完整        | spike 阶段单独测 IME；不达标则 v1 仅支持 macOS/Windows IME                            |
| 数据目录与 Tauri 同时打开冲突             | 启动时 `flock` 文件锁；提示用户另一 runtime 正在运行                                  |
| 315 个 CSS token 全量迁移工作量过大       | 按使用频率分批；P0 组件用到的 ~80 个 token 在 G2 前完成，其余按 Phase 补              |
| Monaco → GPUI editor 功能降级明显         | v1 明确标注已知债务；多光标/补全作为 P3 后置                                          |
| 三平台打包脚本维护成本                    | 优先 macOS + Windows；Linux AppImage 作为 P2                                          |

---

## 8. 验收门禁

### 8.1 每阶段门禁

| Phase | 门禁命令                                              | 通过标准                                |
| ----- | ----------------------------------------------------- | --------------------------------------- |
| G-1   | `cargo run -p fileterm-gpui --example term_spike`     | 4.4.3 全部场景达标                      |
| G0    | `cargo test -p fileterm-gpui`                         | 空窗口 + bridge 骨架测试通过            |
| G1    | `cargo test -p fileterm-gpui --test storage_contract` | 原 Tauri 夹具全通过                     |
| G2    | `cargo test -p fileterm-gpui --test window_lifecycle` | 7 种窗口 open/close 测试通过            |
| G3    | 手测 + `cargo test --test ssh_terminal`               | 真实 SSH 主机连接 + 4.4.3 性能阈值      |
| G4    | `cargo test --test transfer_journal` + 手测           | 上传/下载/暂停/恢复全链路               |
| G5    | 三平台打包 + 手测                                     | 安装包可安装运行                        |

### 8.2 最终发行门禁

- 全部 108 个 command 在 GPUI 端可调用且行为与 Tauri 一致（contract test 覆盖）。
- 全部 77 个 React 组件对应 GPUI View 已实现（inventory 第 2 节打勾）。
- 全部 14 个 event 路由正确（4.2.2 表）。
- 全部 315 个 CSS token 在 GPUI 端有对应 `Hsla` / `f32` / `BoxShadow`（inventory 第 4 节打勾）。
- 三平台安装包可手动安装运行，与 Tauri 共享数据目录无缝切换。

---

## 9. 进度记录

| 日期       | 事件                                                    |
| ---------- | ------------------------------------------------------- |
| 2026-07-18 | 创建 `gpui` 分支；v0.3 规划文档拆分为三份，待评审       |

---

## 10. 已做决策

1. 共存策略：并存为第三 runtime。
2. Rust 后端复用：fork 一份。
3. 终端模拟器：vte + 自研 GPUI 终端元素。
4. 代码编辑器：GPUI text editor + tree-sitter。
5. Spike 预研：Phase G-1 先验证终端。
6. 状态管理：不引入 Zustand/Redux，用 `Entity<T>` + `cx.notify()`。
7. 事件路由：broadcast + oneshot 双通道，按 4.2.2 表分工。
8. 数据目录：与 Tauri 共享。

---

## 11. 待评审问题

1. fork 后协议代码长期维护策略：每月 cherry-pick 是否足够？何时回抽 `crates/fileterm-core-rs`？
2. 是否允许 Tauri 与 GPUI 互斥运行（启动时 flock），还是允许同时运行？
3. GPUI 自动更新链路是否在 v1 范围内？
4. Linux 发行版本范围：Ubuntu/Fedora/Arch 之外是否支持 Silverblue？
5. 托盘在 Linux 下是否做（GNOME 默认无托盘）？
6. Monaco 的繁简转换（opencc-js）在 GPUI 端是否 v1 必须？
