//! Desktop-owned capture controls. The bridge outlives the UI in forward-only mode
//! so already-running Codex clients never lose their cached loopback endpoint.
use crate::collector_config::{atomic_write, RouteJournal};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CollectorStatus {
    phase: String,
    enabled: bool,
    auto_start: bool,
    forwarding: bool,
    route_installed: bool,
    local_url: Option<String>,
    upstream_origin: Option<String>,
    config_path: String,
    requests: u64,
    observations: u64,
    active_requests: u64,
    write_errors: u64,
    rss_bytes: Option<u64>,
    cpu_percent: Option<f64>,
    message: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Endpoint {
    upstream: String,
    port: u16,
}

#[derive(Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct SavedState {
    auto_start: bool,
    route: Option<RouteJournal>,
    endpoint: Option<Endpoint>,
}
impl Default for SavedState {
    fn default() -> Self {
        Self {
            auto_start: true,
            route: None,
            endpoint: None,
        }
    }
}

#[derive(Clone)]
pub struct Collector(Arc<Mutex<Controller>>);
struct Controller {
    config: PathBuf,
    output: PathBuf,
    directory: PathBuf,
    saved: SavedState,
    status: CollectorStatus,
    owner_lock: Option<File>,
    child: Option<Child>,
    cpu_sample: Option<(Instant, f64)>,
    exiting: bool,
    ready: bool,
}

impl Collector {
    pub fn from_env() -> Self {
        let codex = std::env::var_os("CODEX_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(std::env::var_os("HOME").unwrap_or_else(|| ".".into())).join(".codex")
            });
        let output = std::env::var_os("CODEX_MODEL_AUDIT_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| codex.join("model-audit"));
        Self::new(
            codex.join("config.toml"),
            output,
            codex.join("work-token-monitor"),
        )
    }
    fn new(config: PathBuf, output: PathBuf, directory: PathBuf) -> Self {
        let status = CollectorStatus {
            phase: "starting".into(),
            enabled: false,
            auto_start: true,
            forwarding: false,
            route_installed: false,
            local_url: None,
            upstream_origin: None,
            config_path: config.display().to_string(),
            requests: 0,
            observations: 0,
            active_requests: 0,
            write_errors: 0,
            rss_bytes: None,
            cpu_percent: None,
            message: None,
        };
        Self(Arc::new(Mutex::new(Controller {
            config,
            output,
            directory,
            status,
            saved: SavedState::default(),
            owner_lock: None,
            child: None,
            cpu_sample: None,
            exiting: false,
            ready: false,
        })))
    }
    pub fn status(&self) -> CollectorStatus {
        self.0.lock().unwrap().status.clone()
    }
    fn perform(
        &self,
        operation: impl FnOnce(&mut Controller) -> Result<(), String>,
    ) -> CollectorStatus {
        let mut controller = self.0.lock().unwrap();
        if let Err(error) = operation(&mut controller) {
            controller.fail(error);
        }
        controller.status.clone()
    }
    pub fn initialize(&self) -> CollectorStatus {
        self.perform(Controller::initialize)
    }
    pub fn poll(&self) -> CollectorStatus {
        self.perform(Controller::poll)
    }
    pub fn set_enabled(&self, enabled: bool) -> CollectorStatus {
        self.perform(|c| {
            c.require_owner()?;
            if enabled {
                c.enable()
            } else {
                c.pause()
            }
        })
    }
    pub fn set_auto_start(&self, enabled: bool) -> CollectorStatus {
        self.perform(|c| {
            c.require_owner()?;
            let previous = c.saved.auto_start;
            c.saved.auto_start = enabled;
            if let Err(error) = c.save() {
                c.saved.auto_start = previous;
                return Err(error);
            }
            c.status.auto_start = enabled;
            Ok(())
        })
    }
    pub fn restore_route(&self) -> CollectorStatus {
        self.perform(|c| {
            c.require_owner()?;
            let paused = c.pause();
            c.restore()?;
            paused
        })
    }
    pub fn stop_forwarder(&self) -> CollectorStatus {
        self.perform(Controller::stop_forwarder)
    }
    pub fn shutdown(&self) {
        self.perform(Controller::shutdown);
    }
}

impl Controller {
    fn socket(&self) -> PathBuf {
        self.directory.join("relay.sock")
    }
    fn state_path(&self) -> PathBuf {
        self.directory.join("collector.json")
    }
    fn require_owner(&self) -> Result<(), String> {
        if !self.ready || self.owner_lock.is_none() || self.exiting {
            Err("采集控制尚未就绪，或另一个监控器正在控制采集".into())
        } else {
            Ok(())
        }
    }
    fn fail(&mut self, message: String) {
        self.status.phase = "error".into();
        self.status.message = Some(message);
    }
    fn save(&self) -> Result<(), String> {
        atomic_write(
            &self.state_path(),
            &serde_json::to_vec_pretty(&self.saved).map_err(|_| "无法保存采集设置")?,
            None,
        )
    }
    fn initialize(&mut self) -> Result<(), String> {
        if self.owner_lock.is_some() {
            return Ok(());
        }
        #[cfg(not(unix))]
        {
            return Err("应用内自动采集目前仅支持 Linux / Unix".into());
        }
        if self.socket().as_os_str().len() >= 100 {
            return Err("采集控制目录过长，无法创建本机控制 socket".into());
        }
        fs::create_dir_all(&self.directory).map_err(|_| "无法创建采集控制目录")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&self.directory, fs::Permissions::from_mode(0o700))
                .map_err(|_| "无法保护采集控制目录")?;
        }
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let lock = options
            .open(self.directory.join("controller.lock"))
            .map_err(|_| "无法创建采集控制锁")?;
        lock.try_lock()
            .map_err(|_| "另一个 Work Token Monitor 正在控制采集，请使用那个窗口")?;
        self.owner_lock = Some(lock);
        if self.state_path().exists() {
            let file = File::open(self.state_path()).map_err(|_| "无法读取采集设置")?;
            self.saved = serde_json::from_reader(file.take(64 * 1024))
                .map_err(|_| "采集设置或恢复日志无效，未修改 Codex 配置")?;
        }
        self.ready = true;
        self.status.auto_start = self.saved.auto_start;
        // Recover only our fields, preserving edits by Codex or the user.
        self.restore()?;
        if let Ok(status) = self.control(json!({"command":"capture","enabled":false})) {
            self.apply(status)?;
        }
        if self.saved.auto_start {
            self.enable()?;
        } else if !self.status.forwarding {
            self.status.phase = "stopped".into();
        }
        Ok(())
    }
    fn install_scripts(&self) -> Result<PathBuf, String> {
        let directory = self.directory.join("scripts");
        fs::create_dir_all(&directory).map_err(|_| "无法安装内置采集助手")?;
        for (name, content) in [
            (
                "model_collector_service.py",
                include_str!("../../scripts/model_collector_service.py"),
            ),
            (
                "model_audit.py",
                include_str!("../../scripts/model_audit.py"),
            ),
            (
                "model_evidence.py",
                include_str!("../../scripts/model_evidence.py"),
            ),
            (
                "model_probe.py",
                include_str!("../../scripts/model_probe.py"),
            ),
        ] {
            let file = directory.join(name);
            if fs::read(&file).ok().as_deref() != Some(content.as_bytes()) {
                atomic_write(&file, content.as_bytes(), None)?;
            }
        }
        Ok(directory.join("model_collector_service.py"))
    }
    #[cfg(unix)]
    fn control(&self, command: Value) -> Result<Value, String> {
        use std::os::unix::net::UnixStream;
        let mut socket = UnixStream::connect(self.socket()).map_err(|_| "采集转发器未运行")?;
        socket
            .set_read_timeout(Some(Duration::from_secs(2)))
            .map_err(|_| "无法连接采集转发器")?;
        socket
            .set_write_timeout(Some(Duration::from_secs(2)))
            .map_err(|_| "无法连接采集转发器")?;
        writeln!(socket, "{command}").map_err(|_| "无法控制采集转发器")?;
        let mut response = String::new();
        BufReader::new(socket)
            .take(16 * 1024)
            .read_line(&mut response)
            .map_err(|_| "采集转发器没有响应")?;
        let response: Value =
            serde_json::from_str(&response).map_err(|_| "采集转发器返回了无效状态")?;
        if response.get("error").is_some() {
            return Err("转发器还有正在进行的请求，或控制命令被拒绝，请稍后重试".into());
        }
        if response["protocolVersion"] != 1 {
            return Err("采集转发器版本不兼容，请恢复直连并停止旧转发器".into());
        }
        Ok(response)
    }
    #[cfg(not(unix))]
    fn control(&self, _: Value) -> Result<Value, String> {
        Err("应用内采集需要 Unix socket".into())
    }

    fn ensure_service(&mut self, upstream: &str) -> Result<Value, String> {
        if let Ok(status) = self.control(json!({"command":"status"})) {
            return self.check_service(status, upstream);
        }
        let port = match &self.saved.endpoint {
            Some(endpoint) if endpoint.upstream == upstream => endpoint.port,
            Some(_) => {
                return Err(
                    "上游已更改：请先恢复直连，重启 Codex 后停止旧转发器，再开启采集".into(),
                )
            }
            None => 0,
        };
        let script = self.install_scripts()?;
        let mut command = Command::new("python3");
        command
            .arg("-u")
            .arg(script)
            .arg("--socket")
            .arg(self.socket())
            .arg("--upstream")
            .arg(upstream)
            .arg("--output-dir")
            .arg(&self.output)
            .arg("--port")
            .arg(port.to_string())
            .env("PYTHONDONTWRITEBYTECODE", "1")
            .env_remove("PYTHONPATH")
            .env_remove("PYTHONHOME")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        let mut child = command
            .spawn()
            .map_err(|_| "无法启动采集助手，请安装 Python 3.11 或更新版本")?;
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if let Ok(status) = self.control(json!({"command":"status"})) {
                self.child = Some(child);
                return self.check_service(status, upstream);
            }
            if child.try_wait().map_err(|_| "无法检查采集助手")?.is_some() {
                return Err("采集助手启动失败：请检查 Python 3.11、目录权限或端口占用".into());
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let _ = child.kill();
        let _ = child.wait();
        Err("采集助手启动超时，Codex 路由未修改".into())
    }
    fn check_service(&self, status: Value, upstream: &str) -> Result<Value, String> {
        if status["upstream"].as_str() != Some(upstream)
            || status["outputDir"].as_str() != self.output.to_str()
        {
            return Err(
                "现有转发器使用不同的上游或采集目录，请恢复直连并停止旧转发器后重试".into(),
            );
        }
        Ok(status)
    }
    fn apply(&mut self, state: Value) -> Result<(), String> {
        let port = state["port"]
            .as_u64()
            .and_then(|v| u16::try_from(v).ok())
            .filter(|v| *v != 0)
            .ok_or("转发器端口无效")?;
        self.status.forwarding = true;
        self.status.enabled = state["enabled"].as_bool().unwrap_or(false);
        self.status.phase = if self.status.enabled {
            "running"
        } else {
            "paused"
        }
        .into();
        self.status.local_url = Some(format!("http://127.0.0.1:{port}/v1"));
        self.status.upstream_origin = state["upstream"]
            .as_str()
            .and_then(|u| tauri::Url::parse(u).ok())
            .map(|u| u.origin().ascii_serialization());
        self.status.requests = state["requests"].as_u64().unwrap_or(0);
        self.status.observations = state["observations"].as_u64().unwrap_or(0);
        self.status.active_requests = state["activeRequests"].as_u64().unwrap_or(0);
        self.status.write_errors = state["writeErrors"].as_u64().unwrap_or(0);
        self.status.rss_bytes = state["rssBytes"].as_u64();
        if let Some(cpu) = state["cpuSeconds"].as_f64() {
            let now = Instant::now();
            self.status.cpu_percent = self.cpu_sample.map(|(last, previous)| {
                ((cpu - previous).max(0.0) / now.duration_since(last).as_secs_f64().max(0.001)
                    * 100.0)
                    .max(0.0)
            });
            self.cpu_sample = Some((now, cpu));
        }
        self.status.route_installed = self.saved.route.is_some();
        Ok(())
    }
    fn enable(&mut self) -> Result<(), String> {
        if let Some(route) = &self.saved.route {
            if !route.still_installed() {
                self.pause()?;
                return Err("采集期间 Codex 路由在别处被修改，请先恢复直连配置再重新开启".into());
            }
        } else {
            let (plan, _, _) = RouteJournal::prepare(&self.config, 0)?;
            let state = self.ensure_service(&plan.upstream)?;
            let port = state["port"]
                .as_u64()
                .and_then(|v| u16::try_from(v).ok())
                .ok_or("转发器端口无效")?;
            let (route, original, patched) = RouteJournal::prepare(&self.config, port)?;
            self.check_service(state, &route.upstream)?;
            self.saved.endpoint = Some(Endpoint {
                upstream: route.upstream.clone(),
                port,
            });
            self.saved.route = Some(route.clone());
            self.save()?; // Write-ahead recovery journal, before changing any client setting.
            if let Err(error) =
                atomic_write(&route.config_path, patched.as_bytes(), Some(&original))
            {
                self.saved.route = None;
                let _ = self.save();
                return Err(error);
            }
            self.status.route_installed = true;
        }
        let upstream = self.saved.route.as_ref().unwrap().upstream.clone();
        self.ensure_service(&upstream)?;
        let state = self.control(json!({"command":"capture","enabled":true}))?;
        self.apply(state)?;
        self.status.message = None;
        Ok(())
    }
    fn pause(&mut self) -> Result<(), String> {
        self.status.enabled = false;
        if self.status.forwarding || self.socket().exists() {
            let state = self.control(json!({"command":"capture","enabled":false}))?;
            self.apply(state)?;
        } else {
            self.status.phase = "stopped".into();
        }
        self.status.message = None;
        Ok(())
    }
    fn restore(&mut self) -> Result<(), String> {
        if let Some(route) = self.saved.route.clone() {
            let preserved = route.restore()?;
            self.saved.route = None;
            self.save()?;
            self.status.message = Some(
                if preserved == 0 {
                    "已恢复直连配置。请重启 Codex 后再停止兼容转发器。"
                } else {
                    "已恢复仍由监控器管理的字段，保留了其他程序的改动。请重启 Codex。"
                }
                .into(),
            );
        }
        self.status.route_installed = false;
        Ok(())
    }
    fn poll(&mut self) -> Result<(), String> {
        if !self.ready || self.owner_lock.is_none() || self.exiting {
            return Ok(());
        }
        if let Some(child) = self.child.as_mut() {
            if child.try_wait().ok().flatten().is_some() {
                self.child = None;
            }
        }
        if self.saved.endpoint.is_none() {
            return Ok(());
        }
        let was_enabled = self.status.enabled;
        let command = if was_enabled { "heartbeat" } else { "status" };
        match self.control(json!({"command":command})) {
            Ok(state) => {
                self.apply(state)?;
                if was_enabled && !self.status.enabled {
                    self.apply(self.control(json!({"command":"capture","enabled":true}))?)?;
                }
                if self
                    .saved
                    .route
                    .as_ref()
                    .is_some_and(|r| !r.still_installed())
                {
                    self.pause()?;
                    return Err(
                        "Codex 路由已在其他程序中更改，已暂停采集；请恢复直连后重新接入".into(),
                    );
                }
                Ok(())
            }
            Err(_) => {
                self.status.enabled = false;
                self.status.forwarding = false;
                self.status.rss_bytes = None;
                self.status.cpu_percent = None;
                if self.saved.route.is_some() {
                    let upstream = self.saved.endpoint.as_ref().unwrap().upstream.clone();
                    match self.ensure_service(&upstream) {
                        Ok(state) => {
                            self.apply(state)?;
                            if was_enabled {
                                self.enable()?;
                            }
                            Ok(())
                        }
                        Err(error) => {
                            self.restore()?;
                            Err(error)
                        }
                    }
                } else {
                    self.status.phase = "stopped".into();
                    Ok(())
                }
            }
        }
    }
    fn stop_forwarder(&mut self) -> Result<(), String> {
        self.require_owner()?;
        if self.saved.route.is_some() {
            return Err("请先恢复直连配置并重启 Codex，再停止转发器".into());
        }
        let running = match self.control(json!({"command":"status"})) {
            Ok(_) => true,
            Err(error) if error == "采集转发器未运行" => false,
            Err(error) => return Err(error),
        };
        if running {
            self.control(json!({"command":"stop"}))?;
            // The service owns graceful draining; never kill a PID from a state file.
            let deadline = Instant::now() + Duration::from_secs(3);
            while self.socket().exists() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(50));
            }
            if self.socket().exists() {
                return Err("转发器正在结束连接，请稍后重试".into());
            }
        }
        self.saved.endpoint = None;
        self.save()?;
        self.status.enabled = false;
        self.status.forwarding = false;
        self.status.phase = "stopped".into();
        self.status.local_url = None;
        self.status.rss_bytes = None;
        self.status.cpu_percent = None;
        self.status.message = None;
        Ok(())
    }
    fn shutdown(&mut self) -> Result<(), String> {
        if !self.ready || self.owner_lock.is_none() || self.exiting {
            return Ok(());
        }
        self.exiting = true;
        // If the helper died, restoration must still run. Crash leases fail closed.
        let _ = self.pause();
        self.restore()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};
    struct Fixture {
        directory: PathBuf,
        collector: Collector,
    }
    impl Fixture {
        fn new() -> Self {
            let directory = std::env::temp_dir().join(format!(
                "wtmc-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir(&directory).unwrap();
            let config = directory.join("config.toml");
            fs::write(&config,"model_provider='fixture'\n[model_providers.fixture]\nname='Fixture'\nbase_url='https://fixture.example/v1'\n").unwrap();
            let collector =
                Collector::new(config, directory.join("audit"), directory.join("control"));
            Self {
                directory,
                collector,
            }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let mut c = self.collector.0.lock().unwrap();
            let _ = c.control(json!({"command":"capture","enabled":false}));
            let _ = c.restore();
            let _ = c.control(json!({"command":"stop"}));
            if let Some(mut child) = c.child.take() {
                let _ = child.wait();
            }
            let _ = fs::remove_dir_all(&self.directory);
        }
    }
    #[test]
    fn app_owns_routing_pause_preserves_bridge_and_exit_restores_configuration() {
        let fixture = Fixture::new();
        let running = fixture.collector.initialize();
        assert_eq!(running.phase, "running", "{:?}", running.message);
        assert!(running.route_installed);
        assert!(running.forwarding);
        let paused = fixture.collector.set_enabled(false);
        assert!(!paused.enabled);
        assert!(paused.forwarding);
        assert!(paused.route_installed);
        let second = Collector::new(
            fixture.directory.join("config.toml"),
            fixture.directory.join("audit"),
            fixture.directory.join("control"),
        );
        assert_eq!(second.initialize().phase, "error");
        second.shutdown();
        assert!(fixture.collector.set_enabled(true).enabled);
        fixture.collector.shutdown();
        let c = fixture.collector.0.lock().unwrap();
        let state = c.control(json!({"command":"status"})).unwrap();
        assert_eq!(state["enabled"], false);
        assert!(!fs::read_to_string(&c.config).unwrap().contains("127.0.0.1"));
        assert!(c.saved.route.is_none());
    }
    #[test]
    fn crash_journal_is_recovered_and_saved_preference_keeps_capture_off() {
        let fixture = Fixture::new();
        assert!(fixture.collector.initialize().enabled);
        assert!(!fixture.collector.set_auto_start(false).auto_start);
        {
            let mut abandoned = fixture.collector.0.lock().unwrap();
            abandoned.owner_lock = None;
            abandoned.exiting = true; // Simulate losing the controller without running cleanup.
        }
        let recovered = Collector::new(
            fixture.directory.join("config.toml"),
            fixture.directory.join("audit"),
            fixture.directory.join("control"),
        );
        let status = recovered.initialize();
        assert_eq!(status.phase, "paused", "{:?}", status.message);
        assert!(!status.enabled);
        assert!(!status.auto_start);
        assert!(!status.route_installed);
        assert!(!fs::read_to_string(fixture.directory.join("config.toml"))
            .unwrap()
            .contains("127.0.0.1"));
        assert_eq!(recovered.stop_forwarder().phase, "stopped");
        recovered.shutdown();
    }

    #[test]
    fn corrupt_recovery_state_never_overwrites_configuration() {
        let fixture = Fixture::new();
        let original = fs::read_to_string(fixture.directory.join("config.toml")).unwrap();
        {
            let c = fixture.collector.0.lock().unwrap();
            fs::create_dir_all(&c.directory).unwrap();
            fs::write(c.state_path(), "invalid-json").unwrap();
        }
        assert_eq!(fixture.collector.initialize().phase, "error");
        assert!(!fixture.collector.set_enabled(true).enabled);
        fixture.collector.shutdown();
        assert_eq!(
            fs::read_to_string(fixture.directory.join("config.toml")).unwrap(),
            original
        );
    }

    #[test]
    fn saved_autostart_off_does_not_touch_client_configuration() {
        let fixture = Fixture::new();
        {
            let c = fixture.collector.0.lock().unwrap();
            fs::create_dir_all(&c.directory).unwrap();
            fs::write(c.state_path(), "{\"autoStart\":false}").unwrap();
        }
        let status = fixture.collector.initialize();
        assert_eq!(status.phase, "stopped");
        assert!(!status.auto_start);
        assert!(!status.route_installed);
        assert!(!status.forwarding);
    }
}
