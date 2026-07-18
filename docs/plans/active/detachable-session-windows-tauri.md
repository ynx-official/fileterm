# 可拆分会话窗口实施规格（Tauri 架构）

> 状态：第一/二/三层完成（类型 + Rust 基础 + Bridge + Renderer + 窗口创建与生命周期）；第四层测试与回归待补（沙箱缺 glib-2.0 无法运行 cargo clippy/test，需在本地或 CI 验证）
> 关联：`docs/plans/active/detachable-session-windows.md`（Electron 版历史参考，已实施）
> 架构：Tauri + Rust（`apps/electron` 仅作历史参考，不参与构建）

## 1. 目标

FileTerm 的所有 workspace 窗口都是能力对等的有序标签容器。SSH、FTP、Telnet、Serial 会话可在任意 workspace 之间移动、组合和排序，同时每个窗口都能打开本地首页、新建连接并独立关闭，且连接、终端历史、输入状态与焦点保持连续。

核心约束：

- 移动标签只迁移展示所有权，不重建 session/controller（Rust 侧 session runtime 持有连接）。
- 同一标签任一时刻只属于一个窗口。
- 同一标签任一时刻只有一个有效终端事件 owner。
- 第一个启动窗口只是默认入口；`main / detached-session` 仅作为兼容标记，不作为功能权限边界。
- 所有 workspace 都可承载首页和会话标签，并从当前窗口发起新连接。

## 2. 稳定模型

```txt
Rust WorkspaceWindowRegistry (Mutex<State>)
  ├─ windowId -> { label, ordered tabIds, phase, ready }
  └─ tabId -> ownerWindowId

WorkspaceTabPlacement (广播给所有 renderer)
  ├─ tabId
  ├─ ownerWindowId  ('main' | 'detached-<n>')
  ├─ ownerKind      ('main' | 'detached-session')
  └─ order          (窗口内顺序)

每个 WebviewWindow 加载同一 renderer entry
  └─ URL query 携带 windowId + initialTabId（仅首次认领用）
```

`WorkspaceWindowContext` 的稳定身份是 `windowId + kind`。新独立窗口可携带 `initialTabId` 作为首次认领提示，但窗口建立后不能依赖该字段限制可见标签。

`WorkspaceTabPlacement` 是标签归属与窗口内顺序的权威记录。新连接必须由 Rust 根据 command 调用方窗口解析发起窗口，并在广播 workspace snapshot 前写入 placement；不能由 renderer 提供目标窗口 ID。

## 3. 交互设计

### 3.1 触发方式

| 方式     | 触发                                | 适用场景             |
| -------- | ----------------------------------- | -------------------- |
| 拖出分离 | 从 TabBar 拖动会话标签到窗口外      | 最直觉，浏览器式体验 |
| 右键菜单 | 标签右键 → 移动到新窗口             | 精确操作             |
| 右键菜单 | 标签右键 → 移动到窗口... → 选择目标 | 多窗口间精确归并     |

### 3.2 拖出分离交互

```
1. 用户在 TabBar 按住会话标签拖动
2. 拖出当前窗口可视区域边界（margin 8px）
3. 释放鼠标
   ├─ 光标在屏幕空白区 → 创建新独立窗口，标签移入
   ├─ 光标在另一窗口内 → 标签移入该窗口
   └─ 光标在原窗口内 → 窗口内排序（已有 usePointerSortFallback）
4. 新窗口出现位置 = 拖拽释放点附近（多显示器感知）
5. 源窗口若变空且是独立窗口 → 自动销毁（不关连接）
```

### 3.3 Tauri 拖拽技术方案

webview 间不共享 DOM 事件，采用 Rust 全局坐标协调方案：

- renderer pointerdown 启动拖拽 → `invoke('workspace_start_tab_drag', {tabId, windowId})`
- Rust 记录 drag 状态
- pointerup 时 renderer `invoke('workspace_finish_tab_drag', {screenX, screenY})`
- Rust 判断释放点落在哪个窗口 bounds 内，执行移动或创建
- 窗口内排序仍走 main 已有的 `usePointerSortFallback`

### 3.4 视觉反馈

| 状态             | 反馈                                              |
| ---------------- | ------------------------------------------------- |
| 拖动中（窗口内） | 标签间插入指示线（usePointerSortFallback）        |
| 拖出窗口边界     | 光标变分离图标，TabBar 边缘高亮「释放创建新窗口」 |
| 拖向另一窗口     | 目标窗口边框高亮                                  |
| 创建新窗口中     | 新窗口淡入                                        |

## 4. 窗口生命周期

```txt
创建独立窗口
  └─ Rust: WebviewWindowBuilder + label="detached-<n>"
     URL: index.html?window=detached-session&windowId=detached-<n>&initialTabId=<tabId>
     尺寸: 继承源窗口尺寸
     位置: 拖拽释放点（多显示器 bounds 计算）

窗口内标签移空
  └─ 独立窗口 → 销毁窗口（不关连接），标签 owner 归还 main
  └─ 主窗口 → 保留（显示空状态）

关闭独立窗口（用户点关闭）
  └─ 逐个关闭其中标签的连接 → 全部成功则销毁
  └─ 部分失败 → 保留未关闭标签，窗口不销毁，提示用户

renderer 崩溃
  └─ Rust 监听 WindowEvent::Destroyed
  └─ 该窗口所有标签 owner 归还 main，连接保持
  └─ 广播 placement 更新

应用退出（Cmd+Q / tray quit）
  └─ 所有窗口逐个确认关闭 → 全部连接关闭后退出
```

## 5. 终端 transcript 恢复

```txt
Rust session runtime
  └─ tabId -> SessionHandle (持有连接 + transcript buffer)
  └─ terminal event owner 用 compare-and-release 切换

新 renderer claim 流程
  1. 新窗口 renderer 启动 → invoke('workspace_claim_tab', {tabId, windowId})
  2. Rust: compare-and-release 旧 owner，新 owner 生效
  3. Rust: 推送完整 transcript buffer + 最新 workspace snapshot
  4. 旧 renderer 延迟销毁不会清除新 owner
```

## 6. 技术映射（Electron → Tauri）

| Electron 版                          | Tauri 版                                            | 说明                                 |
| ------------------------------------ | --------------------------------------------------- | ------------------------------------ |
| `BrowserWindow`                      | `WebviewWindowBuilder`                              | main 已有 `open_child_window` 可扩展 |
| `webContents.id` 识别调用方          | window label 反查 windowId                          | Rust 侧                              |
| `BrowserWindow.getBounds()`          | `outer_position()` + `inner_size()`                 | 多显示器 bounds                      |
| `screen.getAllDisplays()`            | `app.available_monitors()`                          | Tauri monitor API                    |
| `ipcMain.handle`                     | Tauri `#[command]`                                  | 已有模式                             |
| `webContents.send` 广播              | `app.emit('workspace:placements-changed', payload)` | Tauri 事件广播                       |
| preload `contextBridge`              | `tauri-api.ts` invoke/listen                        | 已有模式                             |
| HTML5 DnD `screenX/Y`                | Rust 全局坐标监听                                   | 跨窗口拖拽关键差异                   |
| `WorkspaceWindowRegistry` (TS class) | Rust struct + `Mutex<State>`                        | 线程安全                             |

## 7. 实施分层

### 第一层：类型与 Rust 基础（无 UI 改动）

1. `packages/core` 补充窗口类型
2. Rust `workspace_window_registry.rs`：双索引注册表 + 移动计算 + 空窗口清理
3. Rust `workspace_window_placement.rs`：多显示器 bounds 计算
4. Rust commands + 事件广播

### 第二层：Bridge 与 Renderer

5. `tauri-api.ts` 封装窗口管理 API
6. `useWorkspaceWindowContext.ts`（新建）
7. `useWorkspaceTabs.ts` 接入 placement 过滤
8. `TabBar.tsx` 拖出信号 + 右键菜单

### 第三层：窗口创建与生命周期

9. `lib.rs` 扩展 `open_child_window` 支持 `detached-session`
10. `WindowEvent` 处理：空窗口销毁、崩溃恢复、整组关闭
11. `StandaloneWindowFrame.tsx` 接入 windowId

### 第四层：测试与回归

12. Rust 单元测试
13. `contract.rs` 契约测试
14. 手动回归

## 8. 关键风险

1. **Tauri 跨窗口拖拽**：webview 间不共享 DOM 事件，Rust 全局坐标方案需验证能实时获取鼠标位置
2. **renderer 状态隔离**：每个 `WebviewWindow` 独立 webview，localStorage 持久化需确认不冲突
3. **终端 owner 切换**：compare-and-release 需保证旧 renderer 延迟销毁不清除新 owner
4. **Windows WebView2 死锁**：`WebviewWindowBuilder` 在同步 command 中会死锁，必须走 async command + worker thread
