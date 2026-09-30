//! User-initiated local export. The webview never supplies a filesystem path.
use serde::Deserialize;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use tauri_plugin_dialog::DialogExt;

const MAX_EXPORT_BYTES: usize = 64 * 1024 * 1024;
static EXPORTING: AtomicBool = AtomicBool::new(false);
static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);
#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExportFormat {
    Csv,
    Json,
}
impl ExportFormat {
    fn extension(self) -> &'static str {
        match self {
            Self::Csv => "csv",
            Self::Json => "json",
        }
    }
}
fn validate(format: ExportFormat, content: &str) -> Result<(), String> {
    if content.is_empty() || content.len() > MAX_EXPORT_BYTES || content.contains('\0') {
        return Err("导出为空、过大或包含无效字符，请缩小筛选范围。".into());
    }
    if matches!(format, ExportFormat::Json)
        && serde_json::from_str::<serde_json::Value>(content).is_err()
    {
        return Err("导出 JSON 格式无效。".into());
    }
    Ok(())
}
fn destination(mut path: PathBuf, format: ExportFormat) -> Result<PathBuf, String> {
    if path.extension().is_none() {
        path.set_extension(format.extension());
    }
    if path
        .extension()
        .and_then(|s| s.to_str())
        .is_none_or(|s| !s.eq_ignore_ascii_case(format.extension()))
    {
        return Err("请选择与导出格式一致的文件扩展名。".into());
    }
    if path.is_dir() || path.is_symlink() {
        return Err("请选择普通导出文件，不能覆盖目录或符号链接。".into());
    }
    Ok(path)
}
fn write_atomic(path: &Path, content: &[u8]) -> std::io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| std::io::Error::other("missing parent"))?;
    for _ in 0..20 {
        let temp = parent.join(format!(
            ".work-token-monitor-{}-{}.tmp",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = match options.open(&temp) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        };
        let result = file.write_all(content).and_then(|_| file.sync_all());
        drop(file);
        let result = result.and_then(|_| fs::rename(&temp, path));
        if result.is_err() {
            let _ = fs::remove_file(&temp);
        }
        return result;
    }
    Err(std::io::Error::other("temporary export names unavailable"))
}
struct ExportGuard;
impl Drop for ExportGuard {
    fn drop(&mut self) {
        EXPORTING.store(false, Ordering::Release);
    }
}
#[tauri::command]
pub async fn save_export(
    app: tauri::AppHandle,
    format: ExportFormat,
    content: String,
) -> Result<bool, String> {
    validate(format, &content)?;
    if EXPORTING
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        return Err("已有导出对话框打开。".into());
    }
    let guard = ExportGuard;
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        let selected = app
            .dialog()
            .file()
            .add_filter("Token usage", &[format.extension()])
            .set_file_name(format!(
                "token-usage-{}.{}",
                chrono::Utc::now().format("%Y%m%d-%H%M%S"),
                format.extension()
            ))
            .blocking_save_file();
        let Some(selected) = selected else {
            return Ok(false);
        };
        let path = destination(
            selected
                .into_path()
                .map_err(|_| "请选择本地文件。".to_owned())?,
            format,
        )?;
        write_atomic(&path, content.as_bytes())
            .map_err(|_| "写入导出失败，原文件未被直接截断。请检查目录权限和空间。".to_owned())?;
        Ok(true)
    })
    .await
    .map_err(|_| "导出任务异常退出。".to_owned())?
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_wrong_format_empty_and_invalid_json() {
        assert!(validate(ExportFormat::Json, "not json").is_err());
        assert!(validate(ExportFormat::Csv, "").is_err());
        assert!(destination(PathBuf::from("report.txt"), ExportFormat::Csv).is_err());
        assert_eq!(
            destination(PathBuf::from("report"), ExportFormat::Json).unwrap(),
            PathBuf::from("report.json")
        );
    }
    #[test]
    fn atomic_export_is_private_and_does_not_leave_a_temporary_file() {
        let dir = std::env::temp_dir().join(format!(
            "wtm-export-test-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap()
        ));
        fs::create_dir(&dir).unwrap();
        let path = dir.join("sample.csv");
        fs::write(&path, "original").unwrap();
        write_atomic(&path, b"replacement").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"replacement");
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        fs::remove_dir_all(dir).unwrap();
    }
}
