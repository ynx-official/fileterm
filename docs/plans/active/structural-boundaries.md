# 结构边界整理计划 (Active)

## 背景

FileTerm 已经具备 SSH / SFTP / FTP MVP 雏形，但核心逻辑目前高度向少数超大文件集中（如 `workspace-service.ts` 和 `App.tsx`）。为了控制继续迭代时的代码复杂度，我们需要坚定推行职责重构与边界划分。

## 目标

- 明确 IPC、服务层、协议控制器与 Renderer Feature 之间的单向流职责。
- 降低核心逻辑的膨胀速度，实现小而精的模块开发。
- 保证每一步重构均可独立运行、可回退、可验证。

## 待完成工作 (TODO)

### 1. 服务层解耦与重构
- [ ] **继续拆分 `workspace-service.ts`**：
  - [x] 剥离标签页生命周期到 `workspace/workspace-tab-lifecycle.ts`。
  - [x] 剥离传输断点 staging、速度估算、目录进度等 runtime helper 到 `transfers/transfer-runtime-utils.ts`。
  - [ ] 将上传/下载执行编排继续下沉到独立 `TransferService`，让 `workspace-service.ts` 只保留外层调度和存储同步入口。
  - [ ] 剥离会话 Runtime 的高频监听调度，保持 runtime 内只管理会话状态和事件转发。

### 2. 渲染端大文件解耦
- [ ] **继续重构 `App.tsx`**：
  - [x] 将连接编辑表单（Connection Edit Form）封装为 `features/connections/ConnectionFormHost.tsx`。
  - [x] 将系统侧栏外壳封装为 `features/system/SystemSidebarShell.tsx`。
  - [x] 将 Windows 窗口菜单栏封装为 `features/layout/WindowMenubar.tsx`。
  - [x] 将传输管理器中心（Transfer Manager Modal）挂载层封装为 `features/transfers/TransferCenterHost.tsx`。
  - [x] 远程文件面板抽屉主体已由 `features/workspace/SessionWorkspace.tsx` 管理。
  - [ ] 将多标签本地 UI 状态继续下沉为 feature hook，使 `App.tsx` 进一步回归 Shell。

---

## 已完成工作 (Completed)

- **[x] IPC 处理器拆分**：将原来一整块的 `apps/desktop/src/main/ipc.ts` 彻底拆除，在 `src/main/ipc/` 下按领域注册为子处理器文件（如 `app-handlers.ts`, `workspace-handlers.ts`, `transfer-handlers.ts`）。
- **[x] 协议控制器物理隔离**：成功将原本堆在一起的协议逻辑剥离，SSH 终端和 FTP 会话控制器分别在 `sessions/` 目录下物理存放（[ssh-session-controller.ts](file:///Users/stoffel/CodeFile/fileterm/apps/desktop/src/main/services/sessions/ssh-session-controller.ts) 和 [ftp-session-controller.ts](file:///Users/stoffel/CodeFile/fileterm/apps/desktop/src/main/services/sessions/ftp-session-controller.ts)）。
- **[x] 前端布局组件下沉**：Renderer 侧已经将桌面外壳、状态统计和设置下沉到 `features/layout`、`features/workspace` 等子组件。
- **[x] 工作区标签生命周期服务**：`WorkspaceService` 已将打开、重连、激活、关闭和断开标签页的生命周期编排委托给 `workspace-tab-lifecycle.ts`。
- **[x] 传输 runtime helper 外移**：传输断点 staging、速度估算和目录进度计算已进入 `transfers/transfer-runtime-utils.ts`。
- **[x] App shell host 拆分**：连接表单、Windows 菜单栏、系统侧栏外壳和传输中心挂载层已从 `App.tsx` 拆出。

---

## 进度记录

- 2026-05-19：建立记录系统骨架，先把 `AGENTS.md` 收束为入口地图，并将结构边界整理作为 active plan 记录。
- 2026-06-30：renderer 已继续下沉桌面壳 UI 到 feature 组件。
- 2026-07-09：重整结构边界计划，归档已完成的 IPC 与协议控制器拆分工作，对齐最新 TODO。
- 2026-07-09：完成 `WorkspaceTabLifecycleService`、传输 runtime helper 和若干 renderer host 组件拆分；保留更深层 TransferService 与 tab state hook 化作为下一步小粒度重构。
