# 可拆分会话窗口实施规格

> 状态：已实施，并扩展为可组合多标签独立窗口

## 1. 目标

FileTerm 的主窗口和独立窗口都是有序标签容器。SSH、FTP、Telnet、Serial 会话可在任意 FileTerm workspace 窗口之间移动、组合和排序，同时保持连接、终端历史、输入状态与焦点连续性。

核心约束：

- 移动标签只迁移展示所有权，不重建 controller 或协议连接。
- 同一标签任一时刻只属于一个窗口。
- 同一标签任一时刻只有一个有效终端事件 owner。
- 主窗口可承载首页和会话标签；独立窗口只承载会话标签。

## 2. 稳定模型

```txt
WorkspaceSessionRuntime
  └─ tabId -> controller/session/WebContents owner

WorkspaceWindowRegistry
  ├─ windowId -> { BrowserWindow, ordered tabIds, phase }
  └─ tabId -> ownerWindowId

Renderer
  ├─ 按 placement.ownerWindowId 过滤当前窗口标签
  └─ 每个窗口维护本地 activeTabId
```

`WorkspaceWindowContext` 的稳定身份是 `windowId + kind`。新独立窗口可携带 `initialTabId` 作为首次认领提示，但窗口建立后不能依赖该字段限制可见标签。

`WorkspaceTabPlacement` 是标签归属与窗口内顺序的权威记录：

```ts
interface WorkspaceTabPlacement {
  tabId: string
  ownerWindowId: string
  ownerKind: WorkspaceWindowKind
  order: number
}
```

## 3. 标签移动

### 3.1 统一移动入口

拖拽和右键菜单共用 main process 的归属迁移链路：

```txt
input(tabId, targetWindowId, targetIndex)
  -> 校验标签与目标窗口
  -> 从源窗口顺序中移除
  -> 插入目标窗口指定位置
  -> 广播 placements
  -> 恢复并聚焦目标窗口
  -> 目标 renderer 选择移动标签
  -> 源独立窗口为空时销毁空窗口
```

右键窗口操作只保留：

- “移动到独立窗口”：为当前标签创建新的独立窗口。
- “移动到主窗口”：把当前标签移动到主窗口。

### 3.2 跨窗口拖放

- 只接受 `application/x-fileterm-workspace-tab`，外部文件、文本和其他 feature 拖放不进入会话标签状态机。
- 目标标签栏是 `precise` drop zone：具体标签前按索引插入，标签栏空白处追加。
- workspace 的终端、侧栏、文件区、空白区和 portal 遮罩是 `workspace` fallback drop zone：跨窗口释放时追加到目标会话列表末尾。
- 同一窗口释放到 `workspace` fallback 只标记已处理并恢复拖动前顺序，不重排，也不触发拆窗。
- 文件编辑器、连接管理器、命令管理器等 standalone 窗口不注册 fallback drop zone。
- document 捕获阶段先识别内部 MIME；若事件路径命中 `precise` 区域则让标签栏独占结算，避免一次释放发送两次 IPC。
- main process 使用实际 IPC sender 解析源窗口和目标窗口，忽略 renderer 伪造的 `targetWindowId`。
- source `dragend` 不直接拆窗，而是等待短暂结算窗口。
- 成功 drop 立即把 drag record 标记为 `dropped`；重复 drop 和迟到 finish 都是幂等 no-op。
- 只有确认没有 FileTerm workspace 窗口接受 drop 后，才创建新的独立窗口。
- Chromium 在窗外 `dragend` 返回零坐标或异常 `dropEffect` 时，以显式拖拽状态机为准，不依赖单一原生字段判断。
- 拖动记录有最大生命周期，并在源窗口关闭、renderer 销毁或应用退出时清理。

### 3.3 目标激活

移动成功后：

1. 目标窗口若最小化则恢复。
2. 显示并置前目标窗口。
3. 聚焦目标窗口。
4. placement 变化使目标 renderer 立即选择被移动标签。

## 4. Renderer ownership 与状态恢复

`WorkspaceSessionRuntime` 持有协议连接和权威 terminal transcript。Renderer 只持有当前展示实例，不是连接生命周期所有者。

owner 切换规则：

- 新 renderer claim 标签后，runtime 立即发送完整 `terminal:state` 和最新 workspace snapshot。
- `releaseTabRenderer(tabId, sender)` 必须 compare-and-release；旧 owner 延迟销毁不能清除新 owner。
- 连接保持期间，新 renderer 只增量补齐权威 transcript，不清屏，也不允许旧快照覆盖已经显示的新输出。
- 移动标签不得调用 `workspace.closeTab()`，不得从领域 snapshot 删除标签。

## 5. 独立窗口生命周期

### 5.1 标签移出导致窗口为空

这是布局操作：

- 销毁空独立窗口。
- 不关闭已移动标签。
- 不断开连接。
- 不触发整组关闭流程。

### 5.2 用户关闭独立窗口

这是连接关闭操作：

```txt
prevent default close
  -> 读取窗口当前 ordered tabIds
  -> 逐个调用 workspace close lifecycle
  -> 每成功关闭一个，立即移除对应 placement
  -> 全部成功后销毁窗口
```

若中途某个标签关闭失败：

- 已成功关闭的标签保持关闭，placement 已移除。
- 失败标签及尚未处理的标签保留。
- 独立窗口保持存在并承载剩余标签。
- 不回滚已经完成的连接 teardown，避免伪造原子性。

### 5.3 Renderer 崩溃

Renderer 崩溃不等同于用户关闭窗口：

- 将该窗口尚存标签恢复到主窗口。
- 保持连接运行。
- 主窗口 renderer 恢复后重新 claim 对应标签。

### 5.4 应用退出

应用退出继续走统一 shutdown：

- 不执行独立窗口逐标签关闭处理器。
- 不重复 teardown controller。
- 等待传输日志和 workspace shutdown 后统一销毁窗口。

## 6. 窗口边界

独立 workspace 窗口是顶层 `BrowserWindow`，不是 child/modal window：

- 不设置 `parent`，`modal: false`。
- 支持最小化、最大化和多显示器移动。
- 使用 `screen.getDisplayNearestPoint()` 与目标显示器 `workArea` 修正新窗口 bounds。
- 保持 `contextIsolation: true`、`nodeIntegration: false`、`sandbox: true`。
- 文件编辑器、连接管理器等非 workspace 窗口不能成为标签 owner 或拖放目标。

## 7. 错误恢复

| 场景                                     | 处理                                          |
| ---------------------------------------- | --------------------------------------------- |
| 标签不存在或正在关闭                     | 拒绝移动，保留当前归属                        |
| 目标窗口不存在、关闭中或 renderer 已销毁 | 拒绝 drop，由 drag 结束链路决定是否创建新窗口 |
| 重复 drop 或迟到 finish                  | 幂等忽略，不重复移动或拆窗                    |
| 拖动记录超时或源窗口销毁                 | 清理记录，不污染后续拖动                      |
| 新窗口加载或首次 claim 失败              | 销毁新窗口并保留原 placement                  |
| 源独立窗口移动后为空                     | 仅销毁空窗口                                  |
| 关闭整组时某标签失败                     | 保留失败及未处理标签和窗口                    |
| 旧 renderer 延迟释放 owner               | compare-and-release，不影响新 owner           |
| 独立 renderer 崩溃                       | 标签恢复主窗口，连接保持                      |
| 应用退出                                 | 统一 shutdown，不执行窗口级重复关闭           |

## 8. 自动化验证

自动化覆盖：

- 主窗口与独立窗口之间移动。
- 两个独立窗口之间移动、组合和目标索引插入。
- 跨窗口释放到 workspace 任意内容区后追加到目标列表末尾。
- 同一窗口释放到内容区保持拖动前顺序。
- 外部文件与文本拖放不被会话标签 fallback 拦截。
- 实际 IPC sender 覆盖伪造目标窗口 ID。
- 重复 drop、迟到 finish、关闭中目标与过期 drag record 的竞态。
- 最后一个标签移出后的空窗口销毁。
- 关闭多标签独立窗口时逐个关闭全部连接。
- 部分关闭失败后保留剩余标签和窗口。
- renderer 崩溃后整组标签恢复主窗口。
- 最小化目标恢复、显示、置前和聚焦。
- runtime owner 延迟释放竞态。
- terminal transcript 在 renderer ownership 迁移后的完整恢复与增量补齐。

## 9. 手工验收

- 主窗口标签拖入已有独立窗口标签栏并按落点插入。
- 独立窗口 A 标签释放到独立窗口 B 的终端、侧栏、文件区、空白区或遮罩后追加到末尾。
- 同一窗口内标签栏排序；释放到内容区时恢复原顺序。
- 拖动本地文件、文本和 feature 内部对象时，会话标签 fallback 不抢占事件。
- 拖到所有 FileTerm 窗口外时只创建一个新独立窗口。
- 移走最后一个标签后源窗口关闭且连接保持。
- 关闭多标签独立窗口后全部连接断开。
- 模拟单标签关闭失败时，剩余标签和窗口保留。
- 右键“移动到独立窗口/移动到主窗口”行为正确。
- 目标窗口最小化时恢复、置前、聚焦并选中移动标签。
- 终端历史、提示符、已输入内容和后续输出不丢失。
- Windows 多屏和 125%/150% DPI 下新窗口位置合理。
