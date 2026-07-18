//! 可拆分会话窗口注册表。
//!
//! 维护 `windowId -> 有序 tabIds` 与 `tabId -> ownerWindowId` 双索引，
//! 提供统一移动入口、空窗口清理与崩溃恢复。详见
//! `docs/plans/active/detachable-session-windows-tauri.md`。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// 主窗口的固定 windowId。第一个启动窗口只是默认入口，
/// 不作为功能权限边界——任何 workspace 窗口都能承载首页和会话标签。
pub const MAIN_WINDOW_ID: &str = "main";

/// 窗口类型。与 `packages/core::WorkspaceWindowKind` 对应。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WorkspaceWindowKind {
    Main,
    DetachedSession,
}

impl WorkspaceWindowKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Main => "main",
            Self::DetachedSession => "detached-session",
        }
    }
}

/// 窗口稳定身份。windowId 在窗口整个生命周期不变；
/// initialTabId 仅用于新窗口首次认领提示，窗口建立后不能依赖该字段限制可见标签。
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceWindowContext {
    pub window_id: String,
    pub kind: WorkspaceWindowKind,
    /// 仅 detached-session 窗口首次认领时使用。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initial_tab_id: Option<String>,
}

impl WorkspaceWindowContext {
    pub fn main() -> Self {
        Self {
            window_id: MAIN_WINDOW_ID.to_string(),
            kind: WorkspaceWindowKind::Main,
            initial_tab_id: None,
        }
    }
}

/// 标签归属与窗口内顺序的权威记录。
/// 新连接必须由 Rust 根据 command 调用方窗口解析发起窗口，
/// 并在广播 workspace snapshot 前写入 placement。
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceTabPlacement {
    pub tab_id: String,
    pub owner_window_id: String,
    pub owner_kind: WorkspaceWindowKind,
    pub order: usize,
}

/// 独立窗口注册项。
#[derive(Clone, Debug)]
pub struct DetachedWindowRecord {
    pub context: WorkspaceWindowContext,
    /// Tauri window label，用于反查 WebviewWindow。
    pub label: String,
    pub tab_ids: Vec<String>,
    /// 窗口是否已就绪接收标签。
    pub ready: bool,
}

/// 进行中的拖拽记录。pointerdown 时记录，pointerup 时结算。
#[derive(Clone, Debug)]
pub struct TabDragRecord {
    pub tab_id: String,
    pub source_window_id: String,
}

/// 注册表内部状态。所有字段由 `Mutex` 保护，命令层只持有 `Arc<...>`。
#[derive(Default)]
pub struct RegistryState {
    /// windowId -> 独立窗口记录。主窗口不在此表中。
    detached: HashMap<String, DetachedWindowRecord>,
    /// tabId -> ownerWindowId。主窗口标签的 owner 为 `main`。
    owner_by_tab: HashMap<String, String>,
    /// 主窗口标签顺序（不含已迁出标签）。
    main_tab_ids: Vec<String>,
    /// 当前进行中的拖拽。同一时间只允许一个。
    active_drag: Option<TabDragRecord>,
    /// 下一个独立窗口编号，用于生成 `detached-<n>`。
    next_window_number: u64,
}

impl RegistryState {
    fn next_window_id(&mut self) -> String {
        self.next_window_number += 1;
        format!("detached-{}", self.next_window_number)
    }
}

/// 可拆分会话窗口注册表。线程安全，由 Tauri managed state 持有。
#[derive(Clone)]
pub struct WorkspaceWindowRegistry {
    state: Arc<Mutex<RegistryState>>,
}

impl WorkspaceWindowRegistry {
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(RegistryState::default())),
        }
    }

    /// 同步主窗口标签列表。当主进程创建/关闭标签后调用，保持 main_tab_ids 与实际标签一致。
    /// 已迁出到独立窗口的标签不会重新出现在主窗口顺序中。
    pub fn sync_main_tabs(&self, all_tab_ids: &[String]) {
        let mut state = self.state.lock().expect("registry mutex poisoned");
        state.main_tab_ids = all_tab_ids
            .iter()
            .filter(|tab_id| !state.owner_by_tab.contains_key(*tab_id))
            .cloned()
            .collect::<Vec<_>>();
    }

    /// 注册一个新的独立窗口，返回其 windowId。
    /// `initial_tab_id` 可选，用于首次认领提示。
    ///
    /// windowId 同时作为 Tauri WebviewWindow 的 label，使
    /// `find_window_id_by_label` 能直接 1:1 映射。
    pub fn register_detached(
        &self,
        initial_tab_id: Option<String>,
    ) -> WorkspaceWindowContext {
        let mut state = self.state.lock().expect("registry mutex poisoned");
        let window_id = state.next_window_id();
        let context = WorkspaceWindowContext {
            window_id: window_id.clone(),
            kind: WorkspaceWindowKind::DetachedSession,
            initial_tab_id,
        };
        state.detached.insert(
            window_id.clone(),
            DetachedWindowRecord {
                context: context.clone(),
                label: window_id.clone(),
                tab_ids: Vec::new(),
                ready: false,
            },
        );
        context
    }

    /// 标记独立窗口已就绪（renderer 完成挂载）。
    pub fn mark_detached_ready(&self, window_id: &str) {
        let mut state = self.state.lock().expect("registry mutex poisoned");
        if let Some(record) = state.detached.get_mut(window_id) {
            record.ready = true;
        }
    }

    /// 注销独立窗口（崩溃/销毁时调用），其标签 owner 归还 main。
    /// 返回需要归还的标签列表，由调用方广播 placement 更新。
    pub fn unregister_detached(&self, window_id: &str) -> Vec<String> {
        let mut state = self.state.lock().expect("registry mutex poisoned");
        let Some(record) = state.detached.remove(window_id) else {
            return Vec::new();
        };
        // 标签 owner 归还 main
        for tab_id in &record.tab_ids {
            state.owner_by_tab.remove(tab_id);
        }
        // 重新加入主窗口顺序末尾
        let returned = record.tab_ids.clone();
        for tab_id in &returned {
            if !state.main_tab_ids.contains(tab_id) {
                state.main_tab_ids.push(tab_id.clone());
            }
        }
        returned
    }

    /// 将标签从源窗口移除，插入目标窗口指定位置。
    /// `target_window_id = "main"` 表示移回主窗口。
    /// `target_index` 超出范围时追加到末尾。
    /// 返回更新后的完整 placement 列表。
    pub fn move_tab(
        &self,
        tab_id: &str,
        target_window_id: &str,
        target_index: usize,
    ) -> Vec<WorkspaceTabPlacement> {
        let mut state = self.state.lock().expect("registry mutex poisoned");
        // 从源窗口移除
        Self::remove_tab_from_owner(&mut state, tab_id);
        // 插入目标窗口
        if target_window_id == MAIN_WINDOW_ID {
            let idx = target_index.min(state.main_tab_ids.len());
            state.main_tab_ids.insert(idx, tab_id.to_string());
            state.owner_by_tab.remove(tab_id);
        } else if let Some(record) = state.detached.get_mut(target_window_id) {
            let idx = target_index.min(record.tab_ids.len());
            record.tab_ids.insert(idx, tab_id.to_string());
            state
                .owner_by_tab
                .insert(tab_id.to_string(), target_window_id.to_string());
        }
        // 返回完整 placement
        Self::collect_placements(&state)
    }

    /// 将标签分离到新创建的独立窗口。调用方应先 `register_detached` 拿到 windowId。
    /// 返回更新后的完整 placement 列表。
    pub fn detach_tab_to_window(
        &self,
        tab_id: &str,
        target_window_id: &str,
    ) -> Vec<WorkspaceTabPlacement> {
        self.move_tab(tab_id, target_window_id, 0)
    }

    /// 记录拖拽开始。同一时间只允许一个进行中的拖拽。
    pub fn start_drag(&self, tab_id: &str, source_window_id: &str) {
        let mut state = self.state.lock().expect("registry mutex poisoned");
        state.active_drag = Some(TabDragRecord {
            tab_id: tab_id.to_string(),
            source_window_id: source_window_id.to_string(),
        });
    }

    /// 结束拖拽，返回 (tabId, source_window_id)。若无进行中的拖拽返回 None。
    /// 注意：此方法只清理 drag 状态，不执行移动。移动由 `move_tab` / `detach_tab_to_window` 完成。
    pub fn finish_drag(&self) -> Option<TabDragRecord> {
        let mut state = self.state.lock().expect("registry mutex poisoned");
        state.active_drag.take()
    }

    /// 返回当前完整 placement 列表。
    pub fn list_placements(&self) -> Vec<WorkspaceTabPlacement> {
        let state = self.state.lock().expect("registry mutex poisoned");
        Self::collect_placements(&state)
    }

    /// 返回所有窗口上下文列表（含主窗口）。
    pub fn list_windows(&self) -> Vec<WorkspaceWindowContext> {
        let state = self.state.lock().expect("registry mutex poisoned");
        let mut windows = vec![WorkspaceWindowContext::main()];
        for record in state.detached.values() {
            windows.push(record.context.clone());
        }
        windows
    }

    /// 根据 windowId 返回窗口上下文。未知 windowId 回退到 main。
    pub fn get_context(&self, window_id: &str) -> WorkspaceWindowContext {
        let state = self.state.lock().expect("registry mutex poisoned");
        if window_id == MAIN_WINDOW_ID {
            return WorkspaceWindowContext::main();
        }
        state
            .detached
            .get(window_id)
            .map(|r| r.context.clone())
            .unwrap_or_else(WorkspaceWindowContext::main)
    }

    /// 根据 windowId 返回该窗口的标签顺序。主窗口返回 main_tab_ids。
    pub fn list_tabs_for_window(&self, window_id: &str) -> Vec<String> {
        let state = self.state.lock().expect("registry mutex poisoned");
        if window_id == MAIN_WINDOW_ID {
            return state.main_tab_ids.clone();
        }
        state
            .detached
            .get(window_id)
            .map(|r| r.tab_ids.clone())
            .unwrap_or_default()
    }

    /// 返回所有独立窗口的 (windowId, label) 列表，用于销毁/广播。
    pub fn list_detached_labels(&self) -> Vec<(String, String)> {
        let state = self.state.lock().expect("registry mutex poisoned");
        state
            .detached
            .values()
            .map(|r| (r.context.window_id.clone(), r.label.clone()))
            .collect()
    }

    /// 根据 Tauri window label 反查 windowId。主窗口 label 直接映射为 `main`。
    /// 用于命令层从 `WebviewWindow::label()` 解析当前调用方窗口身份。
    pub fn find_window_id_by_label(&self, label: &str) -> Option<String> {
        if label == MAIN_WINDOW_ID {
            return Some(MAIN_WINDOW_ID.to_string());
        }
        let state = self.state.lock().expect("registry mutex poisoned");
        state
            .detached
            .values()
            .find(|r| r.label == label)
            .map(|r| r.context.window_id.clone())
    }

    /// 根据 windowId 反查 Tauri window label。主窗口返回 `main`。
    pub fn find_label_by_window_id(&self, window_id: &str) -> Option<String> {
        if window_id == MAIN_WINDOW_ID {
            return Some(MAIN_WINDOW_ID.to_string());
        }
        let state = self.state.lock().expect("registry mutex poisoned");
        state
            .detached
            .get(window_id)
            .map(|r| r.label.clone())
    }

    /// 返回指定窗口是否为空（无标签）。主窗口永远返回 false。
    pub fn is_window_empty(&self, window_id: &str) -> bool {
        let state = self.state.lock().expect("registry mutex poisoned");
        if window_id == MAIN_WINDOW_ID {
            return false;
        }
        state
            .detached
            .get(window_id)
            .map(|r| r.tab_ids.is_empty())
            .unwrap_or(true)
    }

    /// 返回指定标签的 owner windowId。无 owner 视为 main。
    pub fn owner_of(&self, tab_id: &str) -> String {
        let state = self.state.lock().expect("registry mutex poisoned");
        state
            .owner_by_tab
            .get(tab_id)
            .cloned()
            .unwrap_or_else(|| MAIN_WINDOW_ID.to_string())
    }

    /// 关闭标签时清理其在所有窗口中的引用。
    pub fn remove_tab(&self, tab_id: &str) {
        let mut state = self.state.lock().expect("registry mutex poisoned");
        Self::remove_tab_from_owner(&mut state, tab_id);
        state.owner_by_tab.remove(tab_id);
        state.main_tab_ids.retain(|id| id != tab_id);
    }

    /// 内部：从标签当前所在窗口顺序中移除（不清理 owner 索引）。
    fn remove_tab_from_owner(state: &mut RegistryState, tab_id: &str) {
        let owner = state.owner_by_tab.get(tab_id).cloned();
        match owner {
            None => state.main_tab_ids.retain(|id| id != tab_id),
            Some(owner_id) if owner_id == MAIN_WINDOW_ID => {
                state.main_tab_ids.retain(|id| id != tab_id)
            }
            Some(owner_id) => {
                if let Some(record) = state.detached.get_mut(&owner_id) {
                    record.tab_ids.retain(|id| id != tab_id);
                }
            }
        }
    }

    /// 内部：从当前状态构造完整 placement 列表。
    fn collect_placements(state: &RegistryState) -> Vec<WorkspaceTabPlacement> {
        let mut placements = Vec::new();
        for (order, tab_id) in state.main_tab_ids.iter().enumerate() {
            placements.push(WorkspaceTabPlacement {
                tab_id: tab_id.clone(),
                owner_window_id: MAIN_WINDOW_ID.to_string(),
                owner_kind: WorkspaceWindowKind::Main,
                order,
            });
        }
        for record in state.detached.values() {
            for (order, tab_id) in record.tab_ids.iter().enumerate() {
                placements.push(WorkspaceTabPlacement {
                    tab_id: tab_id.clone(),
                    owner_window_id: record.context.window_id.clone(),
                    owner_kind: WorkspaceWindowKind::DetachedSession,
                    order,
                });
            }
        }
        placements
    }
}

impl Default for WorkspaceWindowRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tab(id: &str) -> String {
        id.to_string()
    }

    #[test]
    fn main_tab_without_owner_belongs_to_main() {
        let registry = WorkspaceWindowRegistry::new();
        registry.sync_main_tabs(&[tab("a"), tab("b")]);
        let placements = registry.list_placements();
        assert_eq!(placements.len(), 2);
        assert_eq!(placements[0].tab_id, "a");
        assert_eq!(placements[0].owner_window_id, "main");
        assert_eq!(placements[0].order, 0);
        assert_eq!(placements[1].tab_id, "b");
        assert_eq!(placements[1].order, 1);
    }

    #[test]
    fn detach_tab_moves_to_new_window() {
        let registry = WorkspaceWindowRegistry::new();
        registry.sync_main_tabs(&[tab("a"), tab("b"), tab("c")]);
        let ctx = registry.register_detached(Some("b".to_string()));
        let placements = registry.detach_tab_to_window("b", &ctx.window_id);
        // a, c 仍在 main；b 在 detached
        let main_tabs: Vec<_> = placements
            .iter()
            .filter(|p| p.owner_window_id == "main")
            .map(|p| p.tab_id.clone())
            .collect();
        assert_eq!(main_tabs, vec!["a", "c"]);
        let detached_tabs: Vec<_> = placements
            .iter()
            .filter(|p| p.owner_window_id == ctx.window_id)
            .map(|p| p.tab_id.clone())
            .collect();
        assert_eq!(detached_tabs, vec!["b"]);
    }

    #[test]
    fn register_detached_uses_window_id_as_label() {
        let registry = WorkspaceWindowRegistry::new();
        let ctx = registry.register_detached(None);
        // label 与 windowId 相同，使 Tauri window label 能直接反查
        assert_eq!(
            registry.find_label_by_window_id(&ctx.window_id).as_deref(),
            Some(ctx.window_id.as_str())
        );
        assert_eq!(
            registry.find_window_id_by_label(&ctx.window_id).as_deref(),
            Some(ctx.window_id.as_str())
        );
    }

    #[test]
    fn find_window_id_by_label_resolves_main_window() {
        let registry = WorkspaceWindowRegistry::new();
        assert_eq!(
            registry.find_window_id_by_label("main").as_deref(),
            Some("main")
        );
        // 未注册的 label 返回 None
        assert!(registry.find_window_id_by_label("detached-999").is_none());
    }

    #[test]
    fn move_tab_back_to_main_inserts_at_index() {
        let registry = WorkspaceWindowRegistry::new();
        registry.sync_main_tabs(&[tab("a"), tab("b"), tab("c")]);
        let ctx = registry.register_detached(None);
        registry.detach_tab_to_window("b", &ctx.window_id);
        // 移回 main，插入到 index 1（a 和 c 之间）
        let placements = registry.move_tab("b", "main", 1);
        let main_tabs: Vec<_> = placements
            .iter()
            .filter(|p| p.owner_window_id == "main")
            .map(|p| p.tab_id.clone())
            .collect();
        assert_eq!(main_tabs, vec!["a", "b", "c"]);
    }

    #[test]
    fn unregister_detached_returns_tabs_to_main() {
        let registry = WorkspaceWindowRegistry::new();
        registry.sync_main_tabs(&[tab("a"), tab("b")]);
        let ctx = registry.register_detached(None);
        registry.detach_tab_to_window("b", &ctx.window_id);
        let returned = registry.unregister_detached(&ctx.window_id);
        assert_eq!(returned, vec!["b"]);
        let placements = registry.list_placements();
        let main_tabs: Vec<_> = placements
            .iter()
            .filter(|p| p.owner_window_id == "main")
            .map(|p| p.tab_id.clone())
            .collect();
        assert_eq!(main_tabs, vec!["a", "b"]);
    }

    #[test]
    fn remove_tab_cleans_up_everywhere() {
        let registry = WorkspaceWindowRegistry::new();
        registry.sync_main_tabs(&[tab("a"), tab("b")]);
        let ctx = registry.register_detached(None);
        registry.detach_tab_to_window("b", &ctx.window_id);
        registry.remove_tab("b");
        assert_eq!(registry.owner_of("b"), "main");
        assert!(registry.list_tabs_for_window(&ctx.window_id).is_empty());
    }

    #[test]
    fn drag_record_starts_and_finishes() {
        let registry = WorkspaceWindowRegistry::new();
        registry.start_drag("a", "main");
        let record = registry.finish_drag();
        assert!(record.is_some());
        assert_eq!(record.unwrap().tab_id, "a");
        // 二次 finish 应返回 None
        assert!(registry.finish_drag().is_none());
    }

    #[test]
    fn is_window_empty_for_detached_without_tabs() {
        let registry = WorkspaceWindowRegistry::new();
        let ctx = registry.register_detached(None);
        assert!(registry.is_window_empty(&ctx.window_id));
        assert!(!registry.is_window_empty("main"));
    }
}
