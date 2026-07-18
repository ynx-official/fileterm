//! 独立窗口位置计算（多显示器感知）。
//!
//! 替代 Electron 版的 `screen.getAllDisplays()` + `BrowserWindow.getBounds()`，
//! 使用 Tauri 的 `app.available_monitors()` + `WebviewWindow.outer_position()`
//! + `inner_size()` 计算拖拽释放点对应的窗口或屏幕空白区。

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, PhysicalPosition, PhysicalSize, WebviewWindow};

/// 屏幕坐标系下的窗口边界。
#[derive(Clone, Copy, Debug)]
pub struct WindowBounds {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl WindowBounds {
    /// 判断点 (px, py) 是否落在窗口内（含边界）。
    pub fn contains(&self, px: i32, py: i32) -> bool {
        px >= self.x
            && px <= self.x + self.width as i32
            && py >= self.y
            && py <= self.y + self.height as i32
    }
}

/// 显示器边界信息。Tauri 的 `Monitor` 提供 position + size。
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct MonitorBounds {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl MonitorBounds {
    pub fn contains(&self, px: i32, py: i32) -> bool {
        px >= self.x
            && px <= self.x + self.width as i32
            && py >= self.y
            && py <= self.y + self.height as i32
    }

    /// 计算点 (px, py) 在此显示器内的相对位置（0.0 ~ 1.0）。
    /// 超出范围返回 None。
    pub fn relative(&self, px: i32, py: i32) -> Option<(f64, f64)> {
        if !self.contains(px, py) {
            return None;
        }
        let rx = (px - self.x) as f64 / self.width as f64;
        let ry = (py - self.y) as f64 / self.height as f64;
        Some((rx, ry))
    }
}

impl From<&tauri::Monitor> for MonitorBounds {
    fn from(monitor: &tauri::Monitor) -> Self {
        let position = monitor.position();
        let size = monitor.size();
        Self {
            x: position.x,
            y: position.y,
            width: size.width,
            height: size.height,
        }
    }
}

/// 读取所有显示器边界。失败时返回空向量（容错降级）。
pub fn list_monitor_bounds(app: &AppHandle) -> Vec<MonitorBounds> {
    app.available_monitors()
        .map(|monitors| monitors.iter().map(MonitorBounds::from).collect())
        .unwrap_or_default()
}

/// 读取 WebviewWindow 的物理边界（屏幕坐标系）。
pub fn window_bounds(window: &WebviewWindow) -> Option<WindowBounds> {
    let position = window.outer_position().ok()?;
    let size = window.outer_size().ok()?;
    Some(WindowBounds {
        x: position.x,
        y: position.y,
        width: size.width,
        height: size.height,
    })
}

/// 判断释放点 (px, py) 落在哪个窗口内。
/// 返回窗口 label。无匹配返回 None（表示释放在屏幕空白区）。
pub fn find_window_at(
    app: &AppHandle,
    px: i32,
    py: i32,
    exclude_label: Option<&str>,
) -> Option<String> {
    for window in app.webview_windows().values() {
        if let Some(exclude) = exclude_label {
            if window.label() == exclude {
                continue;
            }
        }
        if let Some(bounds) = window_bounds(window) {
            if bounds.contains(px, py) {
                return Some(window.label().to_string());
            }
        }
    }
    None
}

/// 为新独立窗口计算初始位置。优先使用拖拽释放点；若释放点不在任何显示器内，
/// 回退到源窗口右侧偏移位置。
///
/// - `release_x`, `release_y`: 拖拽释放点（屏幕坐标）
/// - `source_window`: 源窗口，用于回退位置计算
/// - `new_width`, `new_height`: 新窗口期望尺寸
pub fn compute_detached_window_position(
    app: &AppHandle,
    release_x: i32,
    release_y: i32,
    source_window: &WebviewWindow,
    new_width: u32,
    new_height: u32,
) -> PhysicalPosition<i32> {
    let monitors = list_monitor_bounds(app);

    // 1. 释放点在某个显示器内：以释放点为左上角，但要确保完整窗口落在屏幕内
    for monitor in &monitors {
        if monitor.contains(release_x, release_y) {
            let x = release_x
                .min(monitor.x + monitor.width as i32 - new_width as i32)
                .max(monitor.x);
            let y = release_y
                .min(monitor.y + monitor.height as i32 - new_height as i32)
                .max(monitor.y);
            return PhysicalPosition::new(x, y);
        }
    }

    // 2. 释放点不在任何显示器内（罕见，多显示器拔出场景）：回退到源窗口右侧
    if let Some(source_bounds) = window_bounds(source_window) {
        let candidate_x = source_bounds.x + source_bounds.width as i32 + 16;
        let candidate_y = source_bounds.y;
        // 确保回退位置在某显示器内
        for monitor in &monitors {
            if monitor.contains(candidate_x, candidate_y) {
                return PhysicalPosition::new(candidate_x, candidate_y);
            }
        }
    }

    // 3. 兜底：释放点原值
    PhysicalPosition::new(release_x, release_y)
}

/// 读取源窗口尺寸，用于新独立窗口继承。
pub fn source_window_size(window: &WebviewWindow) -> Option<PhysicalSize<u32>> {
    window.inner_size().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_bounds_contains_point() {
        let bounds = WindowBounds {
            x: 100,
            y: 200,
            width: 800,
            height: 600,
        };
        assert!(bounds.contains(100, 200)); // 左上角
        assert!(bounds.contains(900, 800)); // 右下角
        assert!(bounds.contains(500, 500)); // 内部
        assert!(!bounds.contains(99, 200)); // 左外
        assert!(!bounds.contains(100, 199)); // 上外
        assert!(!bounds.contains(901, 800)); // 右外
        assert!(!bounds.contains(500, 801)); // 下外
    }

    #[test]
    fn monitor_bounds_relative_position() {
        let monitor = MonitorBounds {
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
        };
        let (rx, ry) = monitor.relative(960, 540).unwrap();
        assert!((rx - 0.5).abs() < 1e-9);
        assert!((ry - 0.5).abs() < 1e-9);
        assert!(monitor.relative(2000, 540).is_none()); // 超出右边界
    }

    #[test]
    fn compute_position_clamps_to_monitor_bounds() {
        // 释放点接近右下角，窗口应被钳制到显示器内
        // 这里只测试纯计算逻辑，不依赖 Tauri runtime
        let monitor = MonitorBounds {
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
        };
        let new_width: u32 = 800;
        let new_height: u32 = 600;
        let release_x = 1900;
        let release_y = 1000;
        // 模拟 compute 逻辑
        let x = release_x
            .min(monitor.x + monitor.width as i32 - new_width as i32)
            .max(monitor.x);
        let y = release_y
            .min(monitor.y + monitor.height as i32 - new_height as i32)
            .max(monitor.y);
        assert_eq!(x, 1120); // 1920 - 800
        assert_eq!(y, 480); // 1080 - 600
    }
}
