mod export;
mod monitor;

use monitor::{Monitor, Snapshot, POLL_MS};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;
use tauri::{Emitter, State};

#[tauri::command]
async fn get_snapshot(monitor: State<'_, Monitor>) -> Result<Snapshot, String> {
    let monitor = monitor.inner().clone();
    tauri::async_runtime::spawn_blocking(move || monitor.snapshot())
        .await
        .map_err(|_| "无法取得监控快照".to_owned())
}

pub fn run() {
    let monitor = Monitor::from_env();
    let stopped = Arc::new(AtomicBool::new(false));
    let stop_worker = stopped.clone();
    tauri::Builder::default()
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(monitor.clone())
        .invoke_handler(tauri::generate_handler![get_snapshot, export::save_export])
        .setup(move |app| {
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                while !stop_worker.load(Ordering::Relaxed) {
                    let update = monitor.scan();
                    let loading = update.status.discovering || update.status.pending_files > 0;
                    let _ = handle.emit("monitor-update", update);
                    std::thread::sleep(Duration::from_millis(if loading { 100 } else { POLL_MS }));
                }
            });
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building Work Token Monitor")
        .run(move |_, event| {
            if matches!(event, tauri::RunEvent::Exit) {
                stopped.store(true, Ordering::Relaxed);
            }
        });
}
