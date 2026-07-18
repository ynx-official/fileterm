//! 可拆分会话窗口 Tauri commands。
//!
//! 这一层把 `WorkspaceWindowRegistry` 的纯逻辑能力暴露给 renderer，
//! 并在所有归属变更后广播 `workspace:placements-changed` 事件。
//! 窗口创建（`open_child_window` 扩展 `detached-session` kind）由
//! `lib.rs` 提供，本模块通过 `crate::open_detached_session_window` 调用。
//!
//! 详见 `docs/plans/active/detachable-session-windows-tauri.md`。

use crate::services::workspace_window_placement::{
    compute_detached_window_position, source_window_size,
};
use crate::services::workspace_window_registry::{
    WorkspaceWindowContext, WorkspaceTabPlacement, MAIN_WINDOW_ID,
};
use crate::services::workspace::WorkspaceState;
use crate::AppError;
use serde::Deserialize;
use tauri::{AppHandle, Emitter, WebviewWindow};

/// 广播 placement 变更的事件名。所有 workspace renderer 监听此事件
/// 以同步标签归属与窗口内顺序。
pub const PLACEMENTS_CHANGED_EVENT: &str = "workspace:placements-changed";

/// 新独立窗口的默认尺寸（当源窗口尺寸不可读时使用）。
const DEFAULT_DETACHED_WINDOW_WIDTH: u32 = 1024;
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

/// 解析当前调用方窗口的 windowId。未知 label 回退到 main，
/// 使 detached 窗口未注册时也能拿到一个安全默认值。
fn resolve_caller_window_id(registry: &crate::services::WorkspaceWindowRegistry, window: &WebviewWindow) -> String {
    registry
        .find_window_id_by_label(window.label())
        .unwrap_or_else(|| MAIN_WINDOW_ID.to_string())
}

/// 广播 placement 变更。广播失败不应让已成功的归属迁移变成
/// renderer 可重试的错误——记录日志后吞掉。
fn emit_placements_changed(app: &AppHandle, placements: &[WorkspaceTabPlacement]) {
    if let Err(error) = app.emit(PLACEMENTS_CHANGED_EVENT, placements) {
        crate::services::logging::warn(
            app,
            "workspace-window",
            format!("failed to broadcast placements: {error}"),
        );
    }
}

/// 返回当前调用方窗口的 context。renderer 启动后立即调用此命令
/// 以确认自己的 windowId 与 kind。
#[tauri::command]
pub fn workspace_get_window_context(
    window: WebviewWindow,
    state: tauri::State<'_, WorkspaceState>,
) -> Result<WorkspaceWindowContext, AppError> {
    let registry = &state.window_registry;
    let window_id = resolve_caller_window_id(registry, &window);
    Ok(registry.get_context(&window_id))
}

/// 返回完整 placement 列表。renderer 用此列表过滤本窗口可见标签。
#[tauri::command]
pub fn workspace_get_tab_placements(
    state: tauri::State<'_, WorkspaceState>,
) -> Result<Vec<WorkspaceTabPlacement>, AppError> {
    Ok(state.window_registry.list_placements())
}

/// 列出所有窗口上下文（含主窗口）。renderer 用于「移动到窗口...」菜单。
#[tauri::command]
pub fn workspace_list_windows(
    state: tauri::State<'_, WorkspaceState>,
) -> Result<Vec<WorkspaceWindowContext>, AppError> {
    Ok(state.window_registry.list_windows())
}

/// 在窗口间移动标签。target_window_id = "main" 表示移回主窗口。
/// target_index 超出范围时追加到末尾。
#[tauri::command]
pub async fn workspace_move_tab(
    app: AppHandle,
    state: tauri::State<'_, WorkspaceState>,
    input: MoveTabInput,
) -> Result<Vec<WorkspaceTabPlacement>, AppError> {
    let placements = state
        .window_registry
        .move_tab(&input.tab_id, &input.target_window_id, input.target_index);
    emit_placements_changed(&app, &placements);
    Ok(placements)
}

/// 记录拖拽开始。同一时间只允许一个进行中的拖拽。
#[tauri::command]
pub fn workspace_start_tab_drag(
    state: tauri::State<'_, WorkspaceState>,
    input: StartDragInput,
) -> Result<(), AppError> {
    state
        .window_registry
        .start_drag(&input.tab_id, &input.source_window_id);
    Ok(())
}

/// 拖拽结束。Rust 判断释放点落在哪个窗口 bounds 内：
/// - 落在另一窗口内 → 移动到该窗口
/// - 落在屏幕空白区 → 创建新独立窗口并移入
/// - 落在原窗口内 → 不移动（窗口内排序由 renderer 负责）
///
/// 返回更新后的完整 placement 列表。
#[tauri::command]
pub async fn workspace_finish_tab_drag(
    app: AppHandle,
    window: WebviewWindow,
    state: tauri::State<'_, WorkspaceState>,
    input: FinishDragInput,
) -> Result<Vec<WorkspaceTabPlacement>, AppError> {
    // 清理 drag 状态（无论是否真的执行移动）
    let drag = state.window_registry.finish_drag();
    let Some(drag) = drag else {
        // 无进行中的拖拽：返回当前 placements，不报错
        return Ok(state.window_registry.list_placements());
    };

    // drag.tab_id 是权威源（pointerdown 时记录），input.tab_id 仅作校验
    if drag.tab_id != input.tab_id {
        crate::services::logging::warn(
            &app,
            "workspace-window",
            format!(
                "drag tab_id mismatch: drag={} input={}",
                drag.tab_id, input.tab_id
            ),
        );
    }
    let tab_id = drag.tab_id.as_str();
    let source_window_id = drag.source_window_id.as_str();

    // 判断释放点落在哪个窗口内（排除源窗口）
    let source_label = state
        .window_registry
        .find_label_by_window_id(source_window_id)
        .unwrap_or_else(|| MAIN_WINDOW_ID.to_string());
    let target_label = crate::services::workspace_window_placement::find_window_at(
        &app,
        input.screen_x,
        input.screen_y,
        Some(&source_label),
    );

    if let Some(target_label) = target_label {
        // 落在某窗口内 → 解析其 windowId 并移动
        let target_window_id = state
            .window_registry
            .find_window_id_by_label(&target_label)
            .unwrap_or_else(|| MAIN_WINDOW_ID.to_string());
        // 计算目标窗口内的插入位置（追加到末尾，窗口内精确排序由 renderer 处理）
        let target_index = state
            .window_registry
            .list_tabs_for_window(&target_window_id)
            .len();
        let placements = state
            .window_registry
            .move_tab(tab_id, &target_window_id, target_index);
        emit_placements_changed(&app, &placements);
        return Ok(placements);
    }

    // 落在屏幕空白区 → 创建新独立窗口
    detach_tab_to_new_window(app, window, state, tab_id, source_window_id, input.screen_x, input.screen_y)
        .await
}

/// 将标签分离到新创建的独立窗口。
///
/// 流程：
/// 1. 计算新窗口位置（多显示器感知）
/// 2. 在 registry 注册新窗口，拿到 windowId（同时作为 Tauri label）
/// 3. 通过 `open_detached_session_window` 创建 WebviewWindow
/// 4. 将标签移入新窗口
/// 5. 广播 placement 变更
async fn detach_tab_to_new_window(
    app: AppHandle,
    source_window: WebviewWindow,
    state: tauri::State<'_, WorkspaceState>,
    tab_id: &str,
    _source_window_id: &str,
    screen_x: i32,
    screen_y: i32,
) -> Result<Vec<WorkspaceTabPlacement>, AppError> {
    // 1. 计算位置和尺寸
    let (new_width, new_height) = source_window_size(&source_window)
        .map(|size| (size.width, size.height))
        .unwrap_or((DEFAULT_DETACHED_WINDOW_WIDTH, DEFAULT_DETACHED_WINDOW_HEIGHT));
    let position = compute_detached_window_position(
        &app,
        screen_x,
        screen_y,
        &source_window,
        new_width,
        new_height,
    );

    // 2. 注册到 registry（windowId = Tauri label）
    let context = state
        .window_registry
        .register_detached(Some(tab_id.to_string()));

    // 3. 创建 WebviewWindow（spawn_blocking 避免 WebView2 同步死锁）
    let app_clone = app.clone();
    let label = context.window_id.clone();
    let window_id = context.window_id.clone();
    let initial_tab_id = tab_id.to_string();
    tauri::async_runtime::spawn_blocking(move || {
        crate::open_detached_session_window(
            &app_clone,
            &label,
            &window_id,
            &initial_tab_id,
            position,
            new_width,
            new_height,
        )
    })
    .await
    .map_err(|error| {
        AppError::Window(format!("detached window creation task failed: {error}"))
    })??;

    // 4. 移动标签到新窗口
    let placements = state
        .window_registry
        .detach_tab_to_window(tab_id, &context.window_id);

    // 5. 广播
    emit_placements_changed(&app, &placements);

    Ok(placements)
}

/// 右键菜单「移动到新窗口」入口。与拖出分离等价，但提供独立 command
/// 以便 renderer 不必伪造屏幕坐标。
#[tauri::command]
pub async fn workspace_detach_tab(
    app: AppHandle,
    window: WebviewWindow,
    state: tauri::State<'_, WorkspaceState>,
    input: DetachTabInput,
) -> Result<Vec<WorkspaceTabPlacement>, AppError> {
    // 清理可能存在的 drag 状态（右键菜单不经过 start_drag）
    state.window_registry.finish_drag();

    detach_tab_to_new_window(
        app,
        window,
        state,
        &input.tab_id,
        &input.source_window_id,
        input.screen_x,
        input.screen_y,
    )
    .await
}

/// 标记独立窗口已就绪（renderer 完成挂载）。后续 placement 广播可
/// 安全假定该窗口能接收标签。
#[tauri::command]
pub fn workspace_mark_detached_ready(
    window: WebviewWindow,
    state: tauri::State<'_, WorkspaceState>,
) -> Result<(), AppError> {
    let registry = &state.window_registry;
    if let Some(window_id) = registry.find_window_id_by_label(window.label()) {
        if window_id != MAIN_WINDOW_ID {
            registry.mark_detached_ready(&window_id);
        }
    }
    Ok(())
}

/// 内部测试用的纯逻辑校验，不依赖 Tauri runtime。
#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::workspace_window_registry::WorkspaceWindowKind;

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

    #[test]
    fn window_kind_serializes_to_kebab_case() {
        let main = serde_json::to_value(WorkspaceWindowKind::Main).unwrap();
        let detached = serde_json::to_value(WorkspaceWindowKind::DetachedSession).unwrap();
        assert_eq!(main, serde_json::json!("main"));
        assert_eq!(detached, serde_json::json!("detached-session"));
    }
}
