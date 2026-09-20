mod collector;
mod collector_config;
mod model_audit;
mod monitor;

use collector::{Collector, CollectorStatus};
use monitor::{Monitor, Snapshot, POLL_MS};
use std::time::Duration;
use tauri::{Emitter, Manager, State};

#[tauri::command]
fn get_snapshot(monitor: State<'_, Monitor>) -> Snapshot {
    #[cfg(debug_assertions)]
    eprintln!("UI snapshot requested");
    monitor.snapshot()
}

async fn collector_action(
    app: tauri::AppHandle,
    collector: Collector,
    operation: impl FnOnce(Collector) -> CollectorStatus + Send + 'static,
) -> Result<CollectorStatus, String> {
    let status = tauri::async_runtime::spawn_blocking(move || operation(collector))
        .await
        .map_err(|_| "采集控制任务异常退出".to_owned())?;
    let _ = app.emit("collector-status", &status);
    Ok(status)
}

#[tauri::command]
async fn get_collector_status(
    app: tauri::AppHandle,
    collector: State<'_, Collector>,
) -> Result<CollectorStatus, String> {
    collector_action(app, collector.inner().clone(), |c| c.status()).await
}
#[tauri::command]
async fn set_collection_enabled(
    app: tauri::AppHandle,
    collector: State<'_, Collector>,
    enabled: bool,
) -> Result<CollectorStatus, String> {
    collector_action(app, collector.inner().clone(), move |c| {
        c.set_enabled(enabled)
    })
    .await
}
#[tauri::command]
async fn set_collector_auto_start(
    app: tauri::AppHandle,
    collector: State<'_, Collector>,
    enabled: bool,
) -> Result<CollectorStatus, String> {
    collector_action(app, collector.inner().clone(), move |c| {
        c.set_auto_start(enabled)
    })
    .await
}
#[tauri::command]
async fn restore_collector_route(
    app: tauri::AppHandle,
    collector: State<'_, Collector>,
) -> Result<CollectorStatus, String> {
    collector_action(app, collector.inner().clone(), |c| c.restore_route()).await
}
#[tauri::command]
async fn stop_collector_forwarder(
    app: tauri::AppHandle,
    collector: State<'_, Collector>,
) -> Result<CollectorStatus, String> {
    collector_action(app, collector.inner().clone(), |c| c.stop_forwarder()).await
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
    let collector = Collector::from_env();
    tauri::Builder::default()
        .plugin(tauri_plugin_clipboard_manager::init())
        .manage(monitor.clone())
        .manage(collector.clone())
        .invoke_handler(tauri::generate_handler![
            get_snapshot,
            get_collector_status,
            set_collection_enabled,
            set_collector_auto_start,
            restore_collector_route,
            stop_collector_forwarder
        ])
        .setup(move |app| {
            let capture_handle = app.handle().clone();
            let capture = collector.clone();
            std::thread::spawn(move || {
                let _ = capture_handle.emit("collector-status", capture.initialize());
                loop {
                    std::thread::sleep(Duration::from_secs(2));
                    let _ = capture_handle.emit("collector-status", capture.poll());
                }
            });
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
        .build(tauri::generate_context!())
        .expect("error while building Work Token Monitor")
        .run(|app, event| {
            if matches!(event, tauri::RunEvent::Exit) {
                app.state::<Collector>().shutdown();
            }
        });
}
