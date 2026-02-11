use std::{
    collections::BTreeMap,
    env,
    fmt,
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::Duration,
};

use tokio::{
    process::Command,
    sync::{mpsc, Mutex as TokioMutex},
    time::{self, MissedTickBehavior},
};

const DEFAULT_MONITOR_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AdbDeviceState {
    Device,
    Unauthorized,
    Offline,
    Recovery,
    Bootloader,
    Sideload,
    Unknown(String),
}

impl AdbDeviceState {
    fn from_token(token: &str) -> Self {
        let normalized = token.trim().to_ascii_lowercase();
        match normalized.as_str() {
            "device" => Self::Device,
            "unauthorized" => Self::Unauthorized,
            "offline" => Self::Offline,
            "recovery" => Self::Recovery,
            "bootloader" => Self::Bootloader,
            "sideload" => Self::Sideload,
            _ => Self::Unknown(token.trim().to_string()),
        }
    }

    fn is_ready(&self) -> bool {
        matches!(self, Self::Device)
    }
}

impl fmt::Display for AdbDeviceState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Device => write!(f, "device"),
            Self::Unauthorized => write!(f, "unauthorized"),
            Self::Offline => write!(f, "offline"),
            Self::Recovery => write!(f, "recovery"),
            Self::Bootloader => write!(f, "bootloader"),
            Self::Sideload => write!(f, "sideload"),
            Self::Unknown(value) => write!(f, "{value}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct AdbDevice {
    pub serial: String,
    pub state: AdbDeviceState,
    pub product: Option<String>,
    pub model: Option<String>,
    pub device: Option<String>,
    pub transport_id: Option<u32>,
    pub properties: BTreeMap<String, String>,
}

impl AdbDevice {
    fn is_ready(&self) -> bool {
        self.state.is_ready()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AdbDeviceEventKind {
    Connected,
    Disconnected,
    StateChanged,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct AdbDeviceEvent {
    pub serial: String,
    pub kind: AdbDeviceEventKind,
    pub previous_state: Option<AdbDeviceState>,
    pub current_state: Option<AdbDeviceState>,
    pub device: Option<AdbDevice>,
}

#[derive(Debug)]
pub enum AdbError {
    NotFound { searched: Vec<PathBuf> },
    Io(String),
    Utf8(String),
    Parse(String),
    InvalidPort(u16),
    NoDevices,
    UnauthorizedDevices(Vec<String>),
    MultipleDevices(Vec<String>),
    DeviceNotFound(String),
    DeviceUnavailable {
        serial: String,
        state: AdbDeviceState,
    },
    CommandFailed {
        command: String,
        code: Option<i32>,
        stdout: String,
        stderr: String,
    },
}

impl fmt::Display for AdbError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound { searched } => {
                if searched.is_empty() {
                    write!(f, "ADB binary not found")
                } else {
                    let joined = searched
                        .iter()
                        .map(|path| path.display().to_string())
                        .collect::<Vec<_>>()
                        .join(", ");
                    write!(f, "ADB binary not found (searched: {joined})")
                }
            }
            Self::Io(message) => write!(f, "{message}"),
            Self::Utf8(message) => write!(f, "{message}"),
            Self::Parse(message) => write!(f, "{message}"),
            Self::InvalidPort(port) => write!(f, "invalid TCP port {port}"),
            Self::NoDevices => write!(f, "no Android devices detected"),
            Self::UnauthorizedDevices(serials) => {
                if serials.is_empty() {
                    write!(f, "device is unauthorized; allow USB debugging on the phone")
                } else {
                    write!(
                        f,
                        "unauthorized device(s): {} (allow USB debugging on the phone)",
                        serials.join(", ")
                    )
                }
            }
            Self::MultipleDevices(serials) => {
                write!(
                    f,
                    "multiple Android devices connected (specify serial): {}",
                    serials.join(", ")
                )
            }
            Self::DeviceNotFound(serial) => write!(f, "ADB device not found: {serial}"),
            Self::DeviceUnavailable { serial, state } => {
                write!(f, "ADB device {serial} is not ready (state: {state})")
            }
            Self::CommandFailed {
                command,
                code,
                stdout,
                stderr,
            } => {
                write!(
                    f,
                    "ADB command failed: `{command}` (exit: {:?})",
                    code
                )?;
                if !stderr.trim().is_empty() {
                    write!(f, "; stderr: {}", stderr.trim())?;
                }
                if !stdout.trim().is_empty() {
                    write!(f, "; stdout: {}", stdout.trim())?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for AdbError {}

#[derive(Debug, Clone, Default)]
pub struct AdbManager {
    resolved_adb_path: Arc<TokioMutex<Option<PathBuf>>>,
}

impl AdbManager {
    pub fn new() -> Self {
        Self {
            resolved_adb_path: Arc::new(TokioMutex::new(None)),
        }
    }

    pub async fn find_adb(&self) -> Result<PathBuf, AdbError> {
        self.resolve_adb_path().await
    }

    pub async fn devices(&self) -> Result<Vec<AdbDevice>, AdbError> {
        let devices = self.list_devices_raw().await?;
        if devices.is_empty() {
            return Err(AdbError::NoDevices);
        }

        Ok(devices)
    }

    pub async fn forward(
        &self,
        local_port: u16,
        remote_port: u16,
        serial: Option<&str>,
    ) -> Result<String, AdbError> {
        if local_port == 0 {
            return Err(AdbError::InvalidPort(local_port));
        }
        if remote_port == 0 {
            return Err(AdbError::InvalidPort(remote_port));
        }

        self.ensure_server_started().await?;

        let target_serial = self.resolve_target_serial(serial).await?;
        let args = vec![
            "-s".to_string(),
            target_serial.clone(),
            "forward".to_string(),
            format!("tcp:{local_port}"),
            format!("tcp:{remote_port}"),
        ];

        let _ = self.run_success(args).await?;
        Ok(target_serial)
    }

    pub async fn kill_forward(
        &self,
        local_port: u16,
        serial: Option<&str>,
    ) -> Result<String, AdbError> {
        if local_port == 0 {
            return Err(AdbError::InvalidPort(local_port));
        }

        self.ensure_server_started().await?;

        let target_serial = match serial.map(str::trim).filter(|value| !value.is_empty()) {
            Some(serial) => serial.to_string(),
            None => self.resolve_target_serial(None).await?,
        };

        let args = vec![
            "-s".to_string(),
            target_serial.clone(),
            "forward".to_string(),
            "--remove".to_string(),
            format!("tcp:{local_port}"),
        ];

        let _ = self.run_success(args).await?;
        Ok(target_serial)
    }

    pub fn monitor_devices(&self, poll_interval: Duration) -> mpsc::Receiver<AdbDeviceEvent> {
        let interval = if poll_interval.is_zero() {
            DEFAULT_MONITOR_INTERVAL
        } else {
            poll_interval
        };

        let (tx, rx) = mpsc::channel(32);
        let manager = self.clone();

        tokio::spawn(async move {
            let mut previous = BTreeMap::<String, AdbDevice>::new();
            let mut ticker = time::interval(interval);
            ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);

            loop {
                ticker.tick().await;

                let current_devices = match manager.list_devices_raw().await {
                    Ok(devices) => devices,
                    Err(AdbError::NoDevices) => Vec::new(),
                    Err(error) => {
                        log::warn!("ADB monitor poll failed: {error}");
                        continue;
                    }
                };

                let current = current_devices
                    .into_iter()
                    .map(|device| (device.serial.clone(), device))
                    .collect::<BTreeMap<_, _>>();

                for (serial, device) in &current {
                    match previous.get(serial) {
                        None => {
                            if tx
                                .send(AdbDeviceEvent {
                                    serial: serial.clone(),
                                    kind: AdbDeviceEventKind::Connected,
                                    previous_state: None,
                                    current_state: Some(device.state.clone()),
                                    device: Some(device.clone()),
                                })
                                .await
                                .is_err()
                            {
                                return;
                            }
                        }
                        Some(previous_device) if previous_device.state != device.state => {
                            if tx
                                .send(AdbDeviceEvent {
                                    serial: serial.clone(),
                                    kind: AdbDeviceEventKind::StateChanged,
                                    previous_state: Some(previous_device.state.clone()),
                                    current_state: Some(device.state.clone()),
                                    device: Some(device.clone()),
                                })
                                .await
                                .is_err()
                            {
                                return;
                            }
                        }
                        _ => {}
                    }
                }

                for (serial, previous_device) in &previous {
                    if current.contains_key(serial) {
                        continue;
                    }

                    if tx
                        .send(AdbDeviceEvent {
                            serial: serial.clone(),
                            kind: AdbDeviceEventKind::Disconnected,
                            previous_state: Some(previous_device.state.clone()),
                            current_state: None,
                            device: Some(previous_device.clone()),
                        })
                        .await
                        .is_err()
                    {
                        return;
                    }
                }

                previous = current;
            }
        });

        rx
    }

    async fn resolve_target_serial(&self, serial: Option<&str>) -> Result<String, AdbError> {
        let devices = self.list_devices_raw().await?;
        if devices.is_empty() {
            return Err(AdbError::NoDevices);
        }

        if let Some(requested_serial) = serial.map(str::trim).filter(|value| !value.is_empty()) {
            let Some(device) = devices.iter().find(|device| device.serial == requested_serial) else {
                return Err(AdbError::DeviceNotFound(requested_serial.to_string()));
            };

            return if device.is_ready() {
                Ok(requested_serial.to_string())
            } else if matches!(device.state, AdbDeviceState::Unauthorized) {
                Err(AdbError::UnauthorizedDevices(vec![requested_serial.to_string()]))
            } else {
                Err(AdbError::DeviceUnavailable {
                    serial: requested_serial.to_string(),
                    state: device.state.clone(),
                })
            };
        }

        let ready_devices = devices
            .iter()
            .filter(|device| device.is_ready())
            .collect::<Vec<_>>();

        match ready_devices.len() {
            0 => {
                let unauthorized_serials = devices
                    .iter()
                    .filter(|device| matches!(device.state, AdbDeviceState::Unauthorized))
                    .map(|device| device.serial.clone())
                    .collect::<Vec<_>>();

                if unauthorized_serials.is_empty() {
                    Err(AdbError::NoDevices)
                } else {
                    Err(AdbError::UnauthorizedDevices(unauthorized_serials))
                }
            }
            1 => Ok(ready_devices[0].serial.clone()),
            _ => Err(AdbError::MultipleDevices(
                ready_devices
                    .iter()
                    .map(|device| device.serial.clone())
                    .collect(),
            )),
        }
    }

    async fn list_devices_raw(&self) -> Result<Vec<AdbDevice>, AdbError> {
        self.ensure_server_started().await?;
        let stdout = self
            .run_success(vec!["devices".to_string(), "-l".to_string()])
            .await?;

        Ok(parse_devices_output(&stdout))
    }

    async fn ensure_server_started(&self) -> Result<(), AdbError> {
        let _ = self.run_success(vec!["start-server".to_string()]).await?;
        Ok(())
    }

    async fn resolve_adb_path(&self) -> Result<PathBuf, AdbError> {
        if let Some(path) = self.resolved_adb_path.lock().await.clone() {
            return Ok(path);
        }

        let resolved = discover_adb_binary().await?;
        let mut slot = self.resolved_adb_path.lock().await;
        *slot = Some(resolved.clone());
        Ok(resolved)
    }

    async fn run_success(&self, args: Vec<String>) -> Result<String, AdbError> {
        let adb_path = self.resolve_adb_path().await?;
        let output = Command::new(&adb_path)
            .args(&args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .output()
            .await
            .map_err(|error| AdbError::Io(format!("failed to execute {}: {error}", adb_path.display())))?;

        let stdout = String::from_utf8(output.stdout)
            .map_err(|error| AdbError::Utf8(format!("invalid UTF-8 in adb stdout: {error}")))?;
        let stderr = String::from_utf8(output.stderr)
            .map_err(|error| AdbError::Utf8(format!("invalid UTF-8 in adb stderr: {error}")))?;

        if output.status.success() {
            return Ok(stdout);
        }

        Err(AdbError::CommandFailed {
            command: format!("{} {}", adb_path.display(), args.join(" ")),
            code: output.status.code(),
            stdout,
            stderr,
        })
    }
}

pub fn parse_devices_output(output: &str) -> Vec<AdbDevice> {
    output
        .lines()
        .filter_map(parse_device_line)
        .collect::<Vec<_>>()
}

fn parse_device_line(line: &str) -> Option<AdbDevice> {
    let trimmed = line.trim();
    if trimmed.is_empty()
        || trimmed.starts_with("List of devices attached")
        || trimmed.starts_with('*')
    {
        return None;
    }

    let mut parts = trimmed.split_whitespace();
    let serial = parts.next()?.trim();
    let state_token = parts.next()?.trim();

    if serial.is_empty() || state_token.is_empty() {
        return None;
    }

    let mut properties = BTreeMap::new();
    for part in parts {
        if let Some((key, value)) = part.split_once(':') {
            if !key.trim().is_empty() && !value.trim().is_empty() {
                properties.insert(key.trim().to_string(), value.trim().to_string());
            }
        }
    }

    let transport_id = properties
        .get("transport_id")
        .and_then(|value| value.parse::<u32>().ok());

    Some(AdbDevice {
        serial: serial.to_string(),
        state: AdbDeviceState::from_token(state_token),
        product: properties.get("product").cloned(),
        model: properties.get("model").cloned(),
        device: properties.get("device").cloned(),
        transport_id,
        properties,
    })
}

async fn discover_adb_binary() -> Result<PathBuf, AdbError> {
    let mut searched = Vec::<PathBuf>::new();
    let mut candidates = Vec::<PathBuf>::new();

    push_candidate(&mut candidates, PathBuf::from("adb"));

    if let Some(path) = env::var_os("ADB_PATH") {
        let explicit = PathBuf::from(path);
        push_candidate(&mut candidates, explicit.clone());
        push_candidate(&mut candidates, explicit.join(adb_binary_name()));
    }

    if let Some(path) = env::var_os("ANDROID_HOME") {
        push_candidate(
            &mut candidates,
            PathBuf::from(path)
                .join("platform-tools")
                .join(adb_binary_name()),
        );
    }

    if let Some(path) = env::var_os("ANDROID_SDK_ROOT") {
        push_candidate(
            &mut candidates,
            PathBuf::from(path)
                .join("platform-tools")
                .join(adb_binary_name()),
        );
    }

    if let Some(home) = home_dir() {
        push_candidate(
            &mut candidates,
            home.join("Android")
                .join("Sdk")
                .join("platform-tools")
                .join(adb_binary_name()),
        );
    }

    #[cfg(target_os = "windows")]
    if let Some(local_app_data) = env::var_os("LOCALAPPDATA") {
        push_candidate(
            &mut candidates,
            PathBuf::from(local_app_data)
                .join("Android")
                .join("Sdk")
                .join("platform-tools")
                .join(adb_binary_name()),
        );
    }

    if let Ok(current_exe) = env::current_exe() {
        if let Some(exe_dir) = current_exe.parent() {
            push_candidate(&mut candidates, exe_dir.join(adb_binary_name()));
            push_candidate(
                &mut candidates,
                exe_dir.join("platform-tools").join(adb_binary_name()),
            );
            push_candidate(
                &mut candidates,
                exe_dir
                    .join("resources")
                    .join("platform-tools")
                    .join(adb_binary_name()),
            );

            if let Some(parent_dir) = exe_dir.parent() {
                push_candidate(
                    &mut candidates,
                    parent_dir
                        .join("Resources")
                        .join("platform-tools")
                        .join(adb_binary_name()),
                );
            }
        }
    }

    for candidate in candidates {
        if searched.iter().any(|searched_path| searched_path == &candidate) {
            continue;
        }

        searched.push(candidate.clone());

        if candidate != Path::new("adb") && !candidate.exists() {
            continue;
        }

        if probe_adb_binary(&candidate).await {
            return Ok(candidate);
        }
    }

    Err(AdbError::NotFound { searched })
}

async fn probe_adb_binary(candidate: &Path) -> bool {
    let output = Command::new(candidate)
        .arg("version")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .output()
        .await;

    let Ok(output) = output else {
        return false;
    };

    if !output.status.success() {
        return false;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    stdout.contains("Android Debug Bridge") || stderr.contains("Android Debug Bridge")
}

fn push_candidate(candidates: &mut Vec<PathBuf>, candidate: PathBuf) {
    if candidates
        .iter()
        .any(|existing_candidate| existing_candidate == &candidate)
    {
        return;
    }

    candidates.push(candidate);
}

fn adb_binary_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "adb.exe"
    } else {
        "adb"
    }
}

fn home_dir() -> Option<PathBuf> {
    env::var_os("HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("USERPROFILE").map(PathBuf::from))
}

#[cfg(test)]
mod tests {
    use super::{parse_devices_output, AdbDeviceState};

    #[test]
    fn parse_devices_output_extracts_connected_devices() {
        let output = r#"
List of devices attached
ABC123DEF456    device product:model device:device transport_id:1
XYZ789ABC012\tdevice product:model2 device:device2 transport_id:2
"#;

        let devices = parse_devices_output(output);
        assert_eq!(devices.len(), 2);

        assert_eq!(devices[0].serial, "ABC123DEF456");
        assert_eq!(devices[0].state, AdbDeviceState::Device);
        assert_eq!(devices[0].product.as_deref(), Some("model"));
        assert_eq!(devices[0].device.as_deref(), Some("device"));
        assert_eq!(devices[0].transport_id, Some(1));

        assert_eq!(devices[1].serial, "XYZ789ABC012");
        assert_eq!(devices[1].state, AdbDeviceState::Device);
        assert_eq!(devices[1].product.as_deref(), Some("model2"));
        assert_eq!(devices[1].device.as_deref(), Some("device2"));
        assert_eq!(devices[1].transport_id, Some(2));
    }

    #[test]
    fn parse_devices_output_tracks_unauthorized_state() {
        let output = r#"
List of devices attached
ABC123DEF456 unauthorized usb:1 transport_id:3
"#;

        let devices = parse_devices_output(output);
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].serial, "ABC123DEF456");
        assert_eq!(devices[0].state, AdbDeviceState::Unauthorized);
        assert_eq!(devices[0].transport_id, Some(3));
    }

    #[test]
    fn parse_devices_output_ignores_daemon_banner_lines() {
        let output = r#"
* daemon not running; starting now at tcp:5037
* daemon started successfully
List of devices attached

"#;

        let devices = parse_devices_output(output);
        assert!(devices.is_empty());
    }
}
