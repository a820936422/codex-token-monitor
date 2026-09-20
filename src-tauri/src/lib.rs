mod model_audit;
mod monitor;

use monitor::{Monitor, Snapshot, POLL_MS};
use std::time::Duration;
use tauri::{Emitter, State};

#[tauri::command]
fn get_snapshot(monitor: State<'_, Monitor>) -> Snapshot {
    #[cfg(debug_assertions)]
    eprintln!("UI snapshot requested");
    monitor.snapshot()
}

pub fn run() {
    let monitor = Monitor::from_env();
    let initial = monitor.scan();
    eprintln!(
        "Work Token Monitor: {} calls, {} conversations, {} projects, {} parse errors",
        initial.status.records,
        initial.status.conversations,
        initial.status.projects,
        initial.status.parse_errors
    );

    tauri::Builder::default()
        .plugin(tauri_plugin_clipboard_manager::init())
        .manage(monitor.clone())
        .invoke_handler(tauri::generate_handler![get_snapshot])
        .setup(move |app| {
            let handle = app.handle().clone();
            let worker = monitor.clone();
            std::thread::spawn(move || loop {
                std::thread::sleep(Duration::from_millis(POLL_MS));
                let result = worker.scan();
                for call in result.new_calls {
                    let _ = handle.emit("monitor-call", call);
                }
                if let Some(catalog) = result.catalog {
                    let _ = handle.emit("monitor-catalog", catalog);
                }
                let _ = handle.emit("monitor-status", result.status);
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Work Token Monitor");
}
