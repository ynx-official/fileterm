# 结构边界整理计划 (Active)

## 背景

FileTerm 已经具备 SSH / SFTP / FTP MVP 雏形，但核心逻辑目前高度向少数超大文件集中（如 `workspace-service.ts` 和 `App.tsx`）。为了控制继续迭代时的代码复杂度，我们需要坚定推行职责重构与边界划分。

## 目标

- 明确 IPC、服务层、协议控制器与 Renderer Feature 之间的单向流职责。
- 降低核心逻辑的膨胀速度，实现小而精的模块开发。
- 保证每一步重构均可独立运行、可回退、可验证。

## 待完成工作 (TODO)

### 1. 服务层解耦与重构
- [ ] **拆分 `workspace-service.ts`** (当前约 70KB)：
  - 剥离标签页生命周期到单独的 `tabs-manager.ts` 或 `TabService` 中。
  - 剥离文件传输中心调度到单独的 `transfer-dispatcher.ts` 或 `TransferService` 中。
  - 剥离会话 Runtime 的高频监听调度，保持 `workspace-service.ts` 仅作为外层调度和存储同步的轻量入口。

### 2. 渲染端大文件解耦
- [ ] **重构 `App.tsx`** (当前约 126KB)：
  - 将连接编辑表单（Connection Edit Form）封装为独立的 Feature 组件。
  - 将远程文件面板抽屉（File Explorer Drawer）剥离为 Feature 级别组件。
  - 将传输管理器中心（Transfer Manager Modal）独立出 `App.tsx`。
  - 将顶栏标签与多标签切换逻辑剥离为子组件，使 `App.tsx` 保持为只维护核心路由和主题状态的 Shell。

---

## 已完成工作 (Completed)

- **[x] IPC 处理器拆分**：将原来一整块的 `apps/desktop/src/main/ipc.ts` 彻底拆除，在 `src/main/ipc/` 下按领域注册为子处理器文件（如 `app-handlers.ts`, `workspace-handlers.ts`, `transfer-handlers.ts`）。
- **[x] 协议控制器物理隔离**：成功将原本堆在一起的协议逻辑剥离，SSH 终端和 FTP 会话控制器分别在 `sessions/` 目录下物理存放（[ssh-session-controller.ts](file:///Users/stoffel/CodeFile/fileterm/apps/desktop/src/main/services/sessions/ssh-session-controller.ts) 和 [ftp-session-controller.ts](file:///Users/stoffel/CodeFile/fileterm/apps/desktop/src/main/services/sessions/ftp-session-controller.ts)）。
- **[x] 前端布局组件下沉**：Renderer 侧已经将桌面外壳、状态统计和设置下沉到 `features/layout`、`features/workspace` 等子组件。

---

## 进度记录

- 2026-05-19：建立记录系统骨架，先把 `AGENTS.md` 收束为入口地图，并将结构边界整理作为 active plan 记录。
- 2026-06-30：renderer 已继续下沉桌面壳 UI 到 feature 组件。
- 2026-07-09：重整结构边界计划，归档已完成的 IPC 与协议控制器拆分工作，对齐最新 TODO。
