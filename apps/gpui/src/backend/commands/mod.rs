//! Framework-independent desktop commands used by the GPUI bridge.
//!
//! Protocol, transfer, and window operations are intentionally not exposed
//! here: GPUI owns windows directly, while live sessions own their protocol
//! controllers and transfer services. Keeping this module limited to process-
//! scoped OS and preference operations prevents a second workspace runtime.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{backend::app_handle::AppHandle, error::AppError};

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct UiPreferences {
    pub theme: String,
    pub locale: String,
}

#[derive(Clone, Deserialize, Debug)]
pub struct UiPreferencesInput {
    pub theme: Option<String>,
    pub locale: Option<String>,
}

fn write_json_object(app: &AppHandle, name: &str, value: &Value) -> Result<(), AppError> {
    let path = crate::backend::storage::workspace_file(app, name)?;
    let temporary = path.with_file_name(format!(".{name}.{}.tmp", uuid::Uuid::new_v4()));
    let content = serde_json::to_vec_pretty(value)
        .map_err(|error| AppError::Serialization(error.to_string()))?;
    std::fs::write(&temporary, content).map_err(|error| AppError::Storage(error.to_string()))?;
    crate::backend::storage::replace_file_atomically(&temporary, &path)
}

pub fn app_get_platform() -> String {
    std::env::consts::OS.to_string()
}

pub fn app_read_clipboard_text() -> Result<String, AppError> {
    let mut clipboard =
        arboard::Clipboard::new().map_err(|error| AppError::Clipboard(error.to_string()))?;
    clipboard
        .get_text()
        .map_err(|error| AppError::Clipboard(error.to_string()))
}

pub fn app_write_clipboard_text(text: String) -> Result<(), AppError> {
    let mut clipboard =
        arboard::Clipboard::new().map_err(|error| AppError::Clipboard(error.to_string()))?;
    clipboard
        .set_text(text)
        .map_err(|error| AppError::Clipboard(error.to_string()))
}

pub fn app_open_external_url(url: String) -> Result<(), AppError> {
    let parsed = validate_external_url(&url)?;
    open::that(parsed.as_str()).map_err(|error| AppError::Command(error.to_string()))
}

fn validate_external_url(url: &str) -> Result<url::Url, AppError> {
    let parsed = url::Url::parse(url)
        .map_err(|error| AppError::Command(format!("外部链接无效: {error}")))?;
    if matches!(parsed.scheme(), "http" | "https") {
        Ok(parsed)
    } else {
        Err(AppError::Command(
            "仅允许打开 http 或 https 外部链接".to_string(),
        ))
    }
}

pub fn app_open_logs_directory(app: &AppHandle) -> Result<(), AppError> {
    let log_directory = crate::backend::storage::state_path(app)?.with_file_name("logs");
    std::fs::create_dir_all(&log_directory)
        .map_err(|error| AppError::Storage(error.to_string()))?;
    open::that(log_directory).map_err(|error| AppError::Command(error.to_string()))
}

pub fn app_get_ui_preferences(app: &AppHandle) -> Result<UiPreferences, AppError> {
    let path = crate::backend::storage::state_path(app)?;
    if !path.exists() {
        return Ok(UiPreferences {
            theme: "default-dark".to_string(),
            locale: "zhCN".to_string(),
        });
    }
    let content =
        std::fs::read_to_string(path).map_err(|error| AppError::Storage(error.to_string()))?;
    serde_json::from_str(&content).map_err(|error| AppError::Serialization(error.to_string()))
}

pub fn app_set_ui_preferences(
    app: &AppHandle,
    input: UiPreferencesInput,
) -> Result<UiPreferences, AppError> {
    let current = app_get_ui_preferences(app)?;
    let theme = input.theme.unwrap_or(current.theme);
    let locale = input.locale.unwrap_or(current.locale);
    if !matches!(theme.as_str(), "default-dark" | "default-light") {
        return Err(AppError::Command("主题设置无效".to_string()));
    }
    if !matches!(locale.as_str(), "zhCN" | "enUS") {
        return Err(AppError::Command("语言设置无效".to_string()));
    }
    let preferences = UiPreferences { theme, locale };
    let value = serde_json::to_value(&preferences)
        .map_err(|error| AppError::Serialization(error.to_string()))?;
    write_json_object(app, "ui-preferences.json", &value)?;
    Ok(preferences)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn external_urls_are_fail_closed() {
        assert!(validate_external_url("https://fileterm.example/docs").is_ok());
        assert!(validate_external_url("http://127.0.0.1:8080").is_ok());
        assert!(validate_external_url("file:///tmp/secret").is_err());
        assert!(validate_external_url("javascript:alert(1)").is_err());
        assert!(validate_external_url("not a url").is_err());
    }

    #[test]
    fn preferences_validate_theme_and_locale() {
        let directory =
            std::env::temp_dir().join(format!("fileterm-gpui-prefs-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let app = AppHandle::new(directory.clone());

        let saved = app_set_ui_preferences(
            &app,
            UiPreferencesInput {
                theme: Some("default-light".to_string()),
                locale: Some("enUS".to_string()),
            },
        )
        .unwrap();
        assert_eq!(saved.theme, "default-light");
        assert_eq!(saved.locale, "enUS");
        assert!(app_set_ui_preferences(
            &app,
            UiPreferencesInput {
                theme: Some("unknown".to_string()),
                locale: None,
            },
        )
        .is_err());
        let _ = std::fs::remove_dir_all(directory);
    }
}
