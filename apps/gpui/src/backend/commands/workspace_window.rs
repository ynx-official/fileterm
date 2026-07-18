//! 可拆分会话窗口 commands (forked from Tauri for G1).
//!
//! G1 migration status: every command in this module is stubbed because they
//! all depend on `WorkspaceWindowRegistry` / `WebviewWindow` / event emitter,
//! which land in G2 (WindowRegistry) and G3 (event system). The input struct
//! definitions and the `PLACEMENTS_CHANGED_EVENT` constant are kept
//! line-for-line from the Tauri source so the renderer-side IPC contract
//! (event names, camelCase payload shapes) is preserved.
//!
//! Original: `apps/tauri/src-tauri/src/commands/workspace_window.rs`.
//! 详见 `docs/plans/active/detachable-session-windows-tauri.md`.

use crate::error::AppError;
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
pub async fn workspace_move_tab(_input: MoveTabInput) -> Result<(), AppError> {
    Err(AppError::Unsupported(
        "G2: needs WindowRegistry + G3: needs event system",
    ))
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
}
