# 可拆分会话窗口实施计划

> **执行要求：** 按任务顺序实施，每一步完成后运行对应验证。当前会话采用 inline execution，不使用 subagent，不自动创建 Git 提交。

**目标：** 让远程会话标签以移动模式拆出为独立 Electron 窗口，关闭独立窗口后无断连地返回主窗口。

**架构：** `WorkspaceWindowRegistry` 在 main process 中维护 `tabId -> BrowserWindow` 展示归属；`WorkspaceSessionRuntime` 保持 controller 不动，只安全切换终端事件 sender。Renderer 根据类型化窗口上下文渲染主窗口或单会话窗口，通过 preload IPC 请求拆出、认领和收回。

**技术栈：** Electron 42、React 19、TypeScript 6、Node test runner、现有 workspace snapshot/runtime/IPC 分层。

---

## 文件结构

### 新建

- `apps/desktop/src/main/services/windows/workspace-window-registry.ts`
  - workspace 窗口注册、placement、detach/attach 状态机。
- `apps/desktop/src/main/ipc/workspace-window-handlers.ts`
  - workspace window IPC 注册。
- `apps/desktop/src/renderer/hooks/useWorkspaceWindowContext.ts`
  - 加载窗口上下文、placement 并认领 tab。
- `apps/desktop/src/renderer/features/windowing/DetachedSessionWindow.tsx`
  - 单会话窗口壳。
- `apps/desktop/test/workspace/workspace-window-placement.test.ts`
  - 纯 placement 状态转换测试。

### 修改

- `packages/core/src/index.ts`
  - 窗口上下文、placement、detach input 和 preload API 类型。
- `apps/desktop/src/main/services/workspace/workspace-session-runtime.ts`
  - `claimTabRenderer/releaseTabRenderer/getTabRenderer`。
- `apps/desktop/src/main/services/workspace/workspace-tab-lifecycle.ts`
  - 去除读取 snapshot 时的全 tab sender 覆盖。
- `apps/desktop/src/main/services/workspace-service.ts`
  - 暴露显式 tab renderer 认领接口。
- `apps/desktop/src/main/ipc/types.ts`
  - 注入窗口 registry 能力。
- `apps/desktop/src/main/ipc/index.ts`
  - 注册 workspace window handlers。
- `apps/desktop/src/main/ipc/workspace-handlers.ts`
  - `getSnapshot` 变为纯读取。
- `apps/desktop/src/main/main.ts`
  - 创建 registry、detached BrowserWindow、退出/隐藏集成。
- `apps/desktop/src/preload/preload.cts`
  - 暴露类型化窗口 API 与 placement 事件。
- `apps/desktop/src/renderer/App.tsx`
  - 窗口上下文接入、main/detached 路由。
- `apps/desktop/src/renderer/hooks/useWorkspaceTabs.ts`
  - 过滤 detached tabs，支持 detach action。
- `apps/desktop/src/renderer/features/layout/TabBar.tsx`
  - 报告 drag end 的 screen 坐标并增加拆出入口。
- `apps/desktop/src/renderer/features/workspace/WorkspaceStage.tsx`
  - 允许 detached 壳复用单会话内容。
- `apps/desktop/src/renderer/i18n.ts`
  - “在独立窗口打开”“返回主窗口”等文案。
- `apps/desktop/src/renderer/styles/features/shell.css`
  - detached 窗口壳样式。
- `docs/architecture.md`
  - 固化窗口 ownership 边界。

## Task 1：核心类型与 placement 纯模型

- [ ] 在 `packages/core/src/index.ts` 增加：

```ts
export type WorkspaceWindowKind = 'main' | 'detached-session'

export interface WorkspaceWindowContext {
  windowId: string
  kind: WorkspaceWindowKind
  tabId?: string
}

export interface WorkspaceTabPlacement {
  tabId: string
  ownerWindowId: string
  ownerKind: WorkspaceWindowKind
}

export interface DetachWorkspaceTabInput {
  tabId: string
  screenPoint?: { x: number; y: number }
}
```

- [ ] 扩展 `AppWindowMode`，加入 `detached-session` 与已有 `file-editor`。
- [ ] 扩展 `FileTermDesktopApi`：

```ts
getWorkspaceWindowContext(): Promise<WorkspaceWindowContext>
getWorkspaceTabPlacements(): Promise<WorkspaceTabPlacement[]>
detachWorkspaceTab(input: DetachWorkspaceTabInput): Promise<void>
attachWorkspaceTab(tabId: string): Promise<void>
claimWorkspaceTab(tabId: string): Promise<void>
onWorkspaceTabPlacementChanged(listener: (placements: WorkspaceTabPlacement[]) => void): () => void
```

- [ ] 新建 placement 纯函数测试，验证 main-owned、detached-owned、重复 detach 幂等和 attach 回主窗口。
- [ ] 运行：

```bash
npm run test:unit -w @fileterm/desktop
npm run typecheck -w @fileterm/core
```

预期：新增测试通过，core 类型检查通过。

## Task 2：Runtime renderer owner 安全 API

- [ ] 在 `workspace-session-runtime.ts` 用以下 API 替代公开 `setSender/getSender`：

```ts
claimTabRenderer(tabId: string, sender: WebContents): void
releaseTabRenderer(tabId: string, sender: WebContents): void
getTabRenderer(tabId: string): WebContents | undefined
```

- [ ] `releaseTabRenderer()` 必须 compare-and-release：当前 sender 不等于传入 sender 时不删除。
- [ ] sender destroyed 时只释放仍归属于该 sender 的 tabs。
- [ ] `workspace-tab-lifecycle.ts` 的 open/reconnect 路径改用 `claimTabRenderer()`。
- [ ] 删除 `bindWorkspaceSender()` 对全部 tabs 的覆盖行为。
- [ ] `WorkspaceService` 增加显式代理方法：

```ts
claimTabRenderer(tabId: string, sender: WebContents): void
releaseTabRenderer(tabId: string, sender: WebContents): void
```

- [ ] 增加 runtime owner 竞态测试：owner 从 A 切换到 B 后，释放 A 不影响 B。
- [ ] 运行 workspace 相关单测和 desktop main typecheck。

## Task 3：WorkspaceWindowRegistry 与 main 窗口生命周期

- [ ] 新建 `WorkspaceWindowRegistry`，构造依赖采用函数注入，避免服务直接依赖全局变量：

```ts
interface WorkspaceWindowRegistryOptions {
  getMainWindow(): BrowserWindow | null
  createDetachedWindow(context: WorkspaceWindowContext, point?: { x: number; y: number }): BrowserWindow
  tabExists(tabId: string): boolean
  claimTabRenderer(tabId: string, sender: WebContents): void
  releaseTabRenderer(tabId: string, sender: WebContents): void
  isQuitting(): boolean
}
```

- [ ] registry 实现：
  - `registerMainWindow(window)`
  - `getContext(sender)`
  - `listPlacements()`
  - `detach(tabId, point)`
  - `claim(tabId, sender)`
  - `attach(tabId)`
  - `closeAll()`
- [ ] `detach()` 重复调用时聚焦已有窗口。
- [ ] detached window `close` 在非退出状态下 `preventDefault()` 并执行 attach。
- [ ] renderer 崩溃或 window destroyed 时 placement 恢复 main。
- [ ] 在 `main.ts` 增加顶层 detached 窗口创建函数，不使用 `parent`。
- [ ] 使用 `screen.getDisplayNearestPoint()` 和 `workArea` 修正初始 bounds。
- [ ] 将 detached windows 纳入 hide/show 和真正退出链路。
- [ ] 运行 main typecheck。

## Task 4：IPC 与 preload 边界

- [ ] 新建 `workspace-window-handlers.ts`，注册：

```txt
workspaceWindow:getContext
workspaceWindow:getPlacements
workspaceWindow:detachTab
workspaceWindow:attachTab
workspaceWindow:claimTab
```

- [ ] `getContext/claimTab` 必须以 `event.sender` 为身份来源，不能相信 renderer 传入 windowId。
- [ ] registry placement 变化时只广播给 main 与 detached workspace 窗口：

```txt
workspaceWindow:placementsChanged
```

- [ ] 在 preload 中实现 core API 对应的方法和取消订阅函数。
- [ ] `workspace:getSnapshot` 只返回 snapshot，不再隐式绑定全部 tab。
- [ ] 主窗口启动后通过新的 claim 流程认领 main-owned tabs。
- [ ] 运行 desktop typecheck。

## Task 5：Renderer 窗口上下文与独立会话窗口

- [ ] 新建 `useWorkspaceWindowContext`：
  - 加载 context 与 placements。
  - 订阅 placement 变化。
  - detached context 加载后调用 `claimWorkspaceTab(tabId)`。
  - 暴露 `detachedTabIds`。
- [ ] `useWorkspaceTabs` 接受 `hiddenSessionTabIds: ReadonlySet<string>`，只过滤 renderer 可见 tabs，不修改领域 snapshot。
- [ ] 新建 `DetachedSessionWindow`，固定使用 context `tabId` 解析 tab/session/profile。
- [ ] detached window 不调用 `activateTab()`，不读取全局 `activeTabId` 选择内容。
- [ ] 独立窗口复用 `SessionWorkspace`、系统侧栏、transfer center 与现有文件操作 hooks。
- [ ] `App.tsx` 在最外层根据 context 分流 main 与 detached；不把独立窗口状态继续堆进普通 modal 分支。
- [ ] detached close/返回按钮调用 `attachWorkspaceTab(tabId)`。
- [ ] 运行 renderer typecheck 与 lint。

## Task 6：标签拖出与可访问入口

- [ ] `TabBar` 的 session tab `dragend` 将 `event.screenX/screenY` 上报。
- [ ] `useWorkspaceTabs` 仅在 session tab 上调用：

```ts
desktopApi.detachWorkspaceTab({
  tabId,
  screenPoint: { x: event.screenX, y: event.screenY }
})
```

- [ ] 第一版 detach 判定：drag end 坐标位于当前窗口屏幕矩形之外时拆出；窗口内继续只排序。
- [ ] 标签右键菜单增加“在独立窗口打开”，保证键盘和不稳定 DnD 环境可用。
- [ ] detached 窗口提供“返回主窗口”。
- [ ] 首页和 system local tab 不允许拆出。
- [ ] 失败时保留标签并进入现有错误提示链。
- [ ] 补中英文文案与 shell 样式。

## Task 7：关闭与退出语义

- [ ] detached `Ctrl/Cmd+W` 触发 attach，不触发 `workspace.closeTab()`。
- [ ] detached 窗口关闭按钮触发 attach。
- [ ] “关闭连接”仍走现有 close tab 确认和 runtime teardown。
- [ ] `Cmd/Ctrl+Q` 继续走统一退出确认；退出期间 detached close handler 不执行 attach。
- [ ] 托盘隐藏/显示同时处理当前可见 detached windows。
- [ ] `requestCloseFocusedWindow()` 区分 main 和 detached。
- [ ] 手工验证连接中、传输中、MFA 中的窗口关闭行为。

## Task 8：验证与文档

- [ ] 更新 `docs/architecture.md`：

```txt
WorkspaceSessionRuntime owns protocol sessions
WorkspaceWindowRegistry owns renderer placement
one tab -> one renderer owner
snapshot broadcast != terminal event ownership
```

- [ ] 更新规格状态与实际实现差异。
- [ ] 运行：

```bash
npm run typecheck
npm run lint
npm run format:check
npm test
npm run build
```

- [ ] Windows 手工验证：
  - 两个 SSH 标签分别输出。
  - 拖出后不串流、不重连。
  - resize、SFTP、CWD、sudo/root 正常。
  - 关闭独立窗口后标签返回。
  - 多显示器与 125%/150% DPI 定位合理。
  - `Ctrl+W`、退出确认、托盘隐藏链正确。

- [ ] 最终检查 `git diff --check` 与编辑文件 lints。
