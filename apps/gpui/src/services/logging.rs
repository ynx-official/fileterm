//! Process-wide file logging for GPUI services.

use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
    sync::{Mutex, OnceLock},
    time::{SystemTime, UNIX_EPOCH},
};

use crate::{backend::AppHandle, error::AppError};

static LOG_FILE: OnceLock<PathBuf> = OnceLock::new();
static WRITE_LOCK: Mutex<()> = Mutex::new(());

pub fn init(app: &AppHandle) -> Result<(), AppError> {
    let directory = app.app_data_dir().join("logs");
    fs::create_dir_all(&directory).map_err(|error| AppError::Storage(error.to_string()))?;
    let _ = LOG_FILE.set(directory.join("fileterm-gpui.log"));
    Ok(())
}

pub fn debug_global(scope: &str, message: impl AsRef<str>) {
    write_global("DEBUG", scope, message.as_ref());
}

pub fn info_global(scope: &str, message: impl AsRef<str>) {
    write_global("INFO", scope, message.as_ref());
}

pub fn error_global(scope: &str, message: impl AsRef<str>) {
    write_global("ERROR", scope, message.as_ref());
}

fn write_global(level: &str, scope: &str, message: &str) {
    let Some(path) = LOG_FILE.get() else {
        return;
    };
    let Ok(_guard) = WRITE_LOCK.lock() else {
        return;
    };
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let sanitized = message.replace(['\r', '\n'], " ");
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{timestamp} {level} [{scope}] {sanitized}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logger_writes_one_line_records() {
        let directory =
            std::env::temp_dir().join(format!("fileterm-gpui-log-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&directory).unwrap();
        let app = AppHandle::new(directory.clone());
        init(&app).unwrap();
        info_global("test", "first\nsecond");
        let content = fs::read_to_string(directory.join("logs/fileterm-gpui.log")).unwrap();
        assert!(content.contains("INFO [test] first second"));
        let _ = fs::remove_dir_all(directory);
    }
}
