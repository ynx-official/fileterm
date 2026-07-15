# 可拆分会话窗口实施记录

> 执行状态：多标签独立窗口重构已完成；不自动创建 Git 提交。

## 1. 实施结果

主窗口和独立窗口已统一为有序标签容器。会话可浏览器式跨窗口移动、组合和排序；连接与 controller 始终由 main process 的 `WorkspaceSessionRuntime` 持有。

已实现：

- `WorkspaceWindowContext` 以 `windowId/kind` 表示稳定窗口身份，`initialTabId` 仅用于新窗口首次认领。
- `WorkspaceTabPlacement` 包含窗口内 `order`。
- `WorkspaceWindowRegistry` 维护 `windowId -> ordered tabIds` 和 `tabId -> ownerWindowId` 两张索引。
- 主窗口、任意独立窗口之间可移动标签并按目标位置插入。
- 同一窗口内排序与跨窗口迁移共用统一移动接口。
- 拖放与右键菜单共用 main process 归属迁移链路。
- source `dragend` 延迟结算，避免 drop 与 dragend IPC 到达顺序导致误拆窗。
- 源独立窗口最后一个标签移出后只销毁空窗口，不关闭连接。
- 用户关闭独立窗口时逐个关闭其中全部连接；部分失败时保留未关闭标签和窗口。
- renderer 崩溃时标签恢复主窗口，连接保持。
- 移动完成后恢复、显示、置前并聚焦目标窗口，目标 renderer 选择移动标签。
- runtime owner 使用 compare-and-release，旧 renderer 延迟销毁不会清除新 owner。
- 新 renderer claim 后恢复完整 terminal transcript 和最新 workspace snapshot。

## 2. 模块职责

### `packages/core`

`packages/core/src/index.ts` 定义：

- 窗口上下文。
- placement 与窗口内顺序。
- 统一移动输入。
- 跨窗口拖拽输入与状态。
- preload 暴露的 `FileTermDesktopApi` 类型。

### Main process

`apps/desktop/src/main/services/windows/workspace-window-registry.ts`：

- 管理 workspace 窗口注册表和标签归属索引。
- 校验移动、计算插入位置并广播 placements。
- 管理跨 renderer drag/drop 结算。
- 清理空独立窗口。
- 处理独立窗口整组关闭、部分失败和崩溃恢复。
- 激活移动目标窗口。

`apps/desktop/src/main/services/windows/workspace-window-placement.ts`：

- 构造带顺序的 placement。
- 计算多显示器和跨平台窗口 bounds。

`apps/desktop/src/main/ipc/workspace-window-handlers.ts` 与 `apps/desktop/src/main/ipc/index.ts`：

- 注册窗口上下文、placements、移动和拖拽 IPC。
- 以 `event.sender` 识别调用窗口，不信任 renderer 自报身份。

`apps/desktop/src/main/main.ts`：

- 创建和注册顶层独立窗口。
- 接入应用隐藏、恢复、退出和 shutdown 生命周期。

### Preload

`apps/desktop/src/preload/preload.cts`：

- 暴露类型化窗口移动和拖拽 API。
- 暴露 placement 变化订阅。
- 不向 renderer 暴露 Electron 原始对象。

### Renderer

`apps/desktop/src/renderer/hooks/useWorkspaceTabs.ts`：

- 按当前 `windowId` 过滤标签。
- 按 placement `order` 构建标签顺序。
- 维护窗口本地活动标签。
- 在 placement 迁移完成后选择刚移入标签。
- 提交排序、跨窗口 drop 和右键移动。

`apps/desktop/src/renderer/features/layout/TabBar.tsx`：

- 提供标签拖拽源和跨窗口 drop 目标。
- 计算目标插入位置。
- 只报告交互意图，不直接修改 main process placement。

`apps/desktop/src/renderer/App.tsx`：

- 注入窗口上下文和当前窗口标签集合。
- 主窗口与独立窗口复用同一 workspace 渲染链路。

## 3. 数据流

```txt
Tab drag/drop or context menu
  -> preload typed API
  -> workspace window IPC
  -> WorkspaceWindowRegistry
  -> ordered placement update
  -> placementsChanged broadcast
  -> target renderer filters and selects moved tab
  -> WorkspaceSessionRuntime switches WebContents owner on claim
  -> transcript/snapshot hydrate target renderer
```

连接移动期间不创建新 controller，不调用 reconnect，不删除领域 tab。

## 4. 关闭状态机

```txt
user closes detached window
  -> preventDefault
  -> phase = closing-connections
  -> for tabId in current ordered tabIds
       close workspace tab
       on success: remove placement immediately
       on failure: stop and keep remaining window state
  -> destroy window only when no tabs remain
```

布局移动导致空窗口使用独立的空容器清理路径，不进入上述连接关闭状态机。

## 5. 自动化测试

`apps/desktop/test/controllers/workspace-windowing.test.mjs` 覆盖：

- owner 只在目标 renderer claim 后切换。
- 独立窗口接收多个有序标签。
- 标签在两个独立窗口之间移动。
- 源窗口为空后销毁。
- 整组连接关闭。
- 部分关闭失败后的剩余状态。
- renderer 崩溃恢复主窗口。
- 最后连接关闭后的窗口清理。
- transcript 恢复和 owner 延迟释放竞态。

`apps/desktop/test/workspace/workspace-window-placement.test.ts` 覆盖：

- 主窗口显式顺序。
- 独立窗口 ready 状态才成为稳定 owner。

Renderer 纯函数测试继续覆盖：

- 跨窗口移动后的目标标签识别。
- 初始 placement hydration 不误判为移动。
- Chromium 异常 dragend 坐标下的拖拽状态机。
- terminal transcript 增量合并。

## 6. 质量验证

最终验证命令：

```bash
node_modules/.bin/prettier --check <本功能修改文件>
node_modules/.bin/eslint --max-warnings=0 <本功能代码文件>
npm run build -w @fileterm/core
npm run typecheck -w @fileterm/desktop
npm run test:unit -w @fileterm/desktop
npm run test:controllers -w @fileterm/desktop
npm run build && npm test
git diff --check
```

仓库级 `npm run format:check` 存在大量与本功能无关的既有格式问题，因此本次使用针对性 Prettier 检查，不将仓库级格式门禁标记为通过。

## 7. 手工验证清单

- 主窗口标签拖入已有独立窗口并按落点插入。
- 独立窗口 A 标签拖入独立窗口 B。
- 同一窗口内排序。
- 拖到所有 FileTerm 窗口外时创建新独立窗口，且不因 IPC 顺序误创建第二个窗口。
- 移走最后一个标签后源独立窗口销毁，连接保持。
- 关闭多标签独立窗口时全部连接关闭。
- 单个标签关闭失败时已成功关闭的 placement 移除，剩余标签和窗口保留。
- 右键“移动到独立窗口/移动到主窗口”行为正确。
- 目标窗口最小化时恢复、置前、聚焦并选择移动标签。
- 终端历史、提示符、已输入内容和后续输出不丢失。
