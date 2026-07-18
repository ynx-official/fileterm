//! 可拆分会话窗口 commands (forked from Tauri for G1).
//!
//! G1 migration status: every command in this module is stubbed because they
//! all depend on `WorkspaceWindowRegistry` / `WebviewWindow` / event emitter,
//! which land in G2 (WindowRegistry) and G3 (event system). The input struct
//! definitions and the `PLACEMENTS_CHANGED_EVENT` constant are kept
//! line-for-line from the Tauri source so the renderer-side IPC contract
//! (event names, camelCase payload shapes) is preserved.
//!
//! G5 update: `workspace_detach_tab` + `workspace_finish_tab_drag` now
//! delegate the *bookkeeping* half to [`crate::window::detach`] and
//! [`crate::window::tab_drag`] (both pure-logic and unit-tested). The
//! other half — actually opening a GPUI window via `cx.open_window` and
//! broadcasting the placement change via `broadcast::Sender` — still
//! requires the bridge to thread `&mut App` through, so those two
//! commands remain `Err(Unsupported)` from the bridge surface. The
//! pure-logic helpers are exposed as `pub` so the future view layer
//! (`view::workspace`) can call them directly with `&mut App` in scope.
//!
//! Original: `apps/tauri/src-tauri/src/commands/workspace_window.rs`.
//! 详见 `docs/plans/active/detachable-session-windows-tauri.md`.

use crate::error::AppError;
use crate::window::{
    detach_tab_to_new_window, DragDropTarget, ScreenBounds, SharedWindowRegistry, TabDragState,
    WorkspaceTabPlacement,
};
use serde::Deserialize;

/// 广播 placement 变更的事件名。所有 workspace renderer 监听此事件
/// 以同步标签归属与窗口内顺序。
pub const PLACEMENTS_CHANGED_EVENT: &str = "workspace:placements-changed";

/// 新独立窗口的默认尺寸（当源窗口尺寸不可读时使用）。
///
/// 当前未使用（仅在 `detach_tab_to_new_window` 中被引用，该 helper 在 G2
/// 重新落地前以 stub 形式存在），保留以便 G2 接入 `WindowRegistry` 时
/// 直接复用原值。
#[allow(dead_code)]
const DEFAULT_DETACHED_WINDOW_WIDTH: u32 = 1024;
#[allow(dead_code)]
const DEFAULT_DETACHED_WINDOW_HEIGHT: u32 = 768;

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MoveTabInput {
    pub tab_id: String,
    pub target_window_id: String,
    pub target_index: usize,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DetachTabInput {
    pub tab_id: String,
    pub source_window_id: String,
    /// 屏幕坐标系下的释放点（物理像素）。
    pub screen_x: i32,
    pub screen_y: i32,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StartDragInput {
    pub tab_id: String,
    pub source_window_id: String,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FinishDragInput {
    pub tab_id: String,
    pub screen_x: i32,
    pub screen_y: i32,
}

/// 返回当前调用方窗口的 context。renderer 启动后立即调用此命令
/// 以确认自己的 windowId 与 kind。
//
// G2: needs WindowRegistry — original signature took
// `(window: WebviewWindow, state: tauri::State<'_, WorkspaceState>)` and
// returned `WorkspaceWindowContext`. Both the framework param and the return
// type are stubbed until G2 forks `WorkspaceWindowRegistry`.
pub fn workspace_get_window_context() -> Result<(), AppError> {
    Err(AppError::Unsupported("G2: needs WindowRegistry"))
}

/// 返回完整 placement 列表。renderer 用此列表过滤本窗口可见标签。
//
// G2: needs WindowRegistry — original signature took
// `state: tauri::State<'_, WorkspaceState>` and returned
// `Vec<WorkspaceTabPlacement>`.
pub fn workspace_get_tab_placements() -> Result<(), AppError> {
    Err(AppError::Unsupported("G2: needs WindowRegistry"))
}

/// 列出所有窗口上下文（含主窗口）。renderer 用于「移动到窗口...」菜单。
//
// G2: needs WindowRegistry — original signature took
// `state: tauri::State<'_, WorkspaceState>` and returned
// `Vec<WorkspaceWindowContext>`.
pub fn workspace_list_windows() -> Result<(), AppError> {
    Err(AppError::Unsupported("G2: needs WindowRegistry"))
}

/// 在窗口间移动标签。target_window_id = "main" 表示移回主窗口。
/// target_index 超出范围时追加到末尾。
//
// G2: needs WindowRegistry + G3: needs event system — original signature took
// `(app: AppHandle, state: tauri::State<'_, WorkspaceState>, input: MoveTabInput)`
// and returned `Vec<WorkspaceTabPlacement>`. After moving the tab it broadcast
// `PLACEMENTS_CHANGED_EVENT` via `app.emit(...)`.
//
// G5: bridge-level call still errors because the GPUI window hasn't been
// wired to broadcast placement changes (no `cx: &mut App` reaches here).
// The pure-logic half is exposed as [`move_tab_with_registry`] so the
// future `view::workspace` layer can call it with `&mut App` in scope.
pub async fn workspace_move_tab(_input: MoveTabInput) -> Result<(), AppError> {
    Err(AppError::Unsupported(
        "G5: bookkeeping logic available via move_tab_with_registry; \
         bridge-level call needs GPUI window open + placement broadcast",
    ))
}

/// Pure-logic helper for `workspace_move_tab`.
///
/// Updates `registry` bookkeeping for the move (no GPUI window open, no
/// broadcast). The caller is responsible for:
/// 1. calling this helper,
/// 2. updating the source + target windows' views to reflect the moved
///    tab (close tab in old window, open tab in new window),
/// 3. broadcasting the returned placement list to all windows via the
///    placements-changed channel.
///
/// `target_window_id = "main"` is the special "move back to main" case;
/// any other value must be an existing `detached-session-{uuid}` window
/// id (the caller is responsible for verifying existence — this helper
/// does not check).
pub fn move_tab_with_registry(
    registry: &SharedWindowRegistry,
    input: &MoveTabInput,
) -> Vec<WorkspaceTabPlacement> {
    if input.target_window_id == "main" {
        registry.return_tab_to_main(&input.tab_id);
    } else {
        registry.detach_tab(&input.tab_id, &input.target_window_id);
    }
    registry.list_placements()
}

/// Pure-logic helper for `workspace_detach_tab`.
///
/// Generates a fresh `detached-session-{uuid}` window id, updates
/// `registry` bookkeeping (removes the tab from its current owner,
/// adds it to the new window), and returns the new placement list.
/// The caller is responsible for:
/// 1. calling this helper,
/// 2. calling `cx.open_window(...)` with the new window id encoded
///    (the [`crate::window::DetachResult`] returned by the underlying
///    [`detach_tab_to_new_window`] carries `new_window_id`),
/// 3. calling `registry.register_handle(new_window_id, handle)` from
///    the window-open callback,
/// 4. broadcasting the returned placement list.
///
/// Note: this helper discards `screen_x` / `screen_y` / `source_window_id`
/// — the registry already knows the source via its reverse-lookup map,
/// and the drop point is only useful to the GPUI window opener (which
/// the caller is responsible for). They're accepted on the input struct
/// for IPC contract compatibility with Tauri.
pub fn detach_tab_with_registry(
    registry: &SharedWindowRegistry,
    input: &DetachTabInput,
) -> Vec<WorkspaceTabPlacement> {
    let result = detach_tab_to_new_window(registry, &input.tab_id);
    result.placements
}

/// Pure-logic helper for `workspace_finish_tab_drag`.
///
/// Drives the [`TabDragState`] state machine: marks the drag as
/// finished, classifies the drop target via the provided window list,
/// then delegates to [`move_tab_with_registry`] (for `SameWindow` this
/// is a no-op; for `OtherWindow` / `NewWindow` the registry is updated).
///
/// Returns the updated placement list, or `None` if no drag was active.
/// The caller is responsible for the same post-conditions as
/// [`move_tab_with_registry`] + [`detach_tab_with_registry`]:
/// actually opening / re-rendering windows and broadcasting placements.
pub fn finish_tab_drag_with_registry(
    drag: &mut TabDragState,
    registry: &SharedWindowRegistry,
    screen_x: i32,
    screen_y: i32,
    windows_in_z_order: &[(String, ScreenBounds)],
) -> Option<Vec<WorkspaceTabPlacement>> {
    // Capture the tab_id *before* calling `drag.finish` — `finish` does
    // `active.take()` internally, so `active_tab_id()` would return
    // `None` afterwards.
    let tab_id = drag.active_tab_id()?.to_string();
    let target = drag.finish(screen_x, screen_y, windows_in_z_order)?;
    let placements = match target {
        DragDropTarget::SameWindow => registry.list_placements(),
        DragDropTarget::OtherWindow(target_window_id) => {
            // Reuse move_tab: detach from current owner, attach to target.
            // target_index is ignored by the registry (in-window ordering
            // is renderer-side).
            let input = MoveTabInput {
                tab_id,
                target_window_id,
                target_index: usize::MAX,
            };
            move_tab_with_registry(registry, &input)
        }
        DragDropTarget::NewWindow => {
            // Detach to a freshly-minted window id.
            let result = detach_tab_to_new_window(registry, &tab_id);
            result.placements
        }
    };
    Some(placements)
}

/// 记录拖拽开始。同一时间只允许一个进行中的拖拽。
//
// G2: needs WindowRegistry — original signature took
// `(state: tauri::State<'_, WorkspaceState>, input: StartDragInput)`.
pub fn workspace_start_tab_drag(_input: StartDragInput) -> Result<(), AppError> {
    Err(AppError::Unsupported("G2: needs WindowRegistry"))
}

/// 拖拽结束。Rust 判断释放点落在哪个窗口 bounds 内：
/// - 落在另一窗口内 → 移动到该窗口
/// - 落在屏幕空白区 → 创建新独立窗口并移入
/// - 落在原窗口内 → 不移动（窗口内排序由 renderer 负责）
///
/// 返回更新后的完整 placement 列表。
//
// G2: needs WindowRegistry + G3: needs event system — original signature took
// `(app: AppHandle, window: WebviewWindow, state: tauri::State<'_, WorkspaceState>, input: FinishDragInput)`
// and returned `Vec<WorkspaceTabPlacement>`. The body used
// `find_window_at`, `WorkspaceWindowRegistry::move_tab`, and
// `detach_tab_to_new_window`.
pub async fn workspace_finish_tab_drag(_input: FinishDragInput) -> Result<(), AppError> {
    Err(AppError::Unsupported(
        "G2: needs WindowRegistry + G3: needs event system",
    ))
}

/// 右键菜单「移动到新窗口」入口。与拖出分离等价，但提供独立 command
/// 以便 renderer 不必伪造屏幕坐标。
//
// G2: needs WindowRegistry + G3: needs event system — original signature took
// `(app: AppHandle, window: WebviewWindow, state: tauri::State<'_, WorkspaceState>, input: DetachTabInput)`
// and returned `Vec<WorkspaceTabPlacement>`. The body delegated to
// `detach_tab_to_new_window`, which created a `WebviewWindow` via
// `crate::open_detached_session_window`.
pub async fn workspace_detach_tab(_input: DetachTabInput) -> Result<(), AppError> {
    Err(AppError::Unsupported(
        "G2: needs WindowRegistry + G3: needs event system",
    ))
}

/// 标记独立窗口已就绪（renderer 完成挂载）。后续 placement 广播可
/// 安全假定该窗口能接收标签。
//
// G2: needs WindowRegistry — original signature took
// `(window: WebviewWindow, state: tauri::State<'_, WorkspaceState>)`.
pub fn workspace_mark_detached_ready() -> Result<(), AppError> {
    Err(AppError::Unsupported("G2: needs WindowRegistry"))
}

/// 内部测试用的纯逻辑校验，不依赖 Tauri runtime。
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn move_tab_input_deserializes_camel_case() {
        let input: MoveTabInput = serde_json::from_str(
            r#"{"tabId":"t1","targetWindowId":"main","targetIndex":2}"#,
        )
        .expect("MoveTabInput should accept camelCase");
        assert_eq!(input.tab_id, "t1");
        assert_eq!(input.target_window_id, "main");
        assert_eq!(input.target_index, 2);
    }

    #[test]
    fn detach_tab_input_rejects_unknown_fields() {
        let result: Result<DetachTabInput, _> = serde_json::from_str(
            r#"{"tabId":"t1","sourceWindowId":"main","screenX":10,"screenY":20,"extra":true}"#,
        );
        assert!(result.is_err(), "deny_unknown_fields should reject extra");
    }

    #[test]
    fn placements_changed_event_name_is_stable() {
        // renderer 监听此事件名，改名会破坏 contract
        assert_eq!(PLACEMENTS_CHANGED_EVENT, "workspace:placements-changed");
    }

    // ---- G5 pure-logic helper tests ----

    use crate::window::WindowRegistry;
    use std::sync::Arc;

    fn fresh_registry() -> SharedWindowRegistry {
        Arc::new(WindowRegistry::new())
    }

    #[test]
    fn detach_tab_with_registry_mints_new_window_id() {
        let reg = fresh_registry();
        let input = DetachTabInput {
            tab_id: "tab-1".into(),
            source_window_id: "main".into(),
            screen_x: 1500,
            screen_y: 200,
        };
        let placements = detach_tab_with_registry(&reg, &input);
        // One detached tab now tracked.
        assert_eq!(placements.len(), 1);
        assert_eq!(placements[0].tab_id, "tab-1");
        assert!(
            placements[0]
                .window_id
                .starts_with("detached-session-"),
            "window_id should be a detached-session-{{uuid}}, got {}",
            placements[0].window_id
        );
        // Drop point is intentionally ignored by the helper.
        // Source window id is intentionally ignored too — registry already
        // knows source via its reverse-lookup.
    }

    #[test]
    fn move_tab_with_registry_to_main_returns_single_tab() {
        let reg = fresh_registry();
        // Start with two tabs detached into the same window.
        reg.detach_tab("tab-1", "detached-session-A");
        reg.detach_tab("tab-2", "detached-session-A");

        // Move just tab-1 back to main; tab-2 must stay.
        let input = MoveTabInput {
            tab_id: "tab-1".into(),
            target_window_id: "main".into(),
            target_index: 0,
        };
        let placements = move_tab_with_registry(&reg, &input);
        // Only tab-2 remains detached.
        assert_eq!(placements.len(), 1);
        assert_eq!(placements[0].tab_id, "tab-2");
        assert_eq!(placements[0].window_id, "detached-session-A");
        assert!(reg.window_for_tab("tab-1").is_none());
    }

    #[test]
    fn move_tab_with_registry_to_other_window_swaps_owner() {
        let reg = fresh_registry();
        reg.detach_tab("tab-1", "detached-session-A");
        reg.detach_tab("tab-2", "detached-session-B");

        // Move tab-1 from session-A to session-B.
        let input = MoveTabInput {
            tab_id: "tab-1".into(),
            target_window_id: "detached-session-B".into(),
            target_index: 0,
        };
        let placements = move_tab_with_registry(&reg, &input);
        // Both tabs now in session-B.
        assert_eq!(placements.len(), 2);
        for p in &placements {
            assert_eq!(p.window_id, "detached-session-B");
        }
        assert_eq!(
            reg.window_for_tab("tab-1").as_deref(),
            Some("detached-session-B")
        );
    }

    #[test]
    fn finish_tab_drag_with_same_window_target_is_noop() {
        let reg = fresh_registry();
        let mut drag = TabDragState::new();
        assert!(drag.start("tab-1", "main"), "start should succeed");

        // Drop inside the main window — SameWindow target.
        let windows = vec![("main".to_string(), ScreenBounds::new(0, 0, 1200, 800))];
        let placements =
            finish_tab_drag_with_registry(&mut drag, &reg, 100, 100, &windows)
                .expect("should return Some");
        // No detach happened, so placements is empty.
        assert!(placements.is_empty());
        assert!(reg.window_for_tab("tab-1").is_none());
        // Drag state is cleared.
        assert!(!drag.is_active());
    }

    #[test]
    fn finish_tab_drag_with_other_window_target_moves_tab() {
        let reg = fresh_registry();
        let mut drag = TabDragState::new();
        assert!(drag.start("tab-1", "main"), "start should succeed");

        // Detached window already exists at (1300, 0, 800, 600).
        // Drop point (1400, 100) is inside it.
        let windows = vec![
            ("detached-session-1".to_string(), ScreenBounds::new(1300, 0, 800, 600)),
            ("main".to_string(), ScreenBounds::new(0, 0, 1200, 800)),
        ];
        let placements =
            finish_tab_drag_with_registry(&mut drag, &reg, 1400, 100, &windows)
                .expect("should return Some");
        assert_eq!(placements.len(), 1);
        assert_eq!(placements[0].tab_id, "tab-1");
        assert_eq!(placements[0].window_id, "detached-session-1");
        assert_eq!(
            reg.window_for_tab("tab-1").as_deref(),
            Some("detached-session-1")
        );
    }

    #[test]
    fn finish_tab_drag_with_new_window_target_detaches() {
        let reg = fresh_registry();
        let mut drag = TabDragState::new();
        assert!(drag.start("tab-1", "main"), "start should succeed");

        // Drop far away — NewWindow target.
        let windows = vec![("main".to_string(), ScreenBounds::new(0, 0, 1200, 800))];
        let placements =
            finish_tab_drag_with_registry(&mut drag, &reg, 5000, 5000, &windows)
                .expect("should return Some");
        assert_eq!(placements.len(), 1);
        assert_eq!(placements[0].tab_id, "tab-1");
        assert!(
            placements[0]
                .window_id
                .starts_with("detached-session-"),
            "drop on empty screen should mint a new detached-session-{{uuid}}, got {}",
            placements[0].window_id
        );
    }

    #[test]
    fn finish_tab_drag_without_active_drag_returns_none() {
        let reg = fresh_registry();
        let mut drag = TabDragState::new();
        let windows: Vec<(String, ScreenBounds)> = vec![];
        let result = finish_tab_drag_with_registry(&mut drag, &reg, 0, 0, &windows);
        assert!(result.is_none(), "finish without start should return None");
    }
}
