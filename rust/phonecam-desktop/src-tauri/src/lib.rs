use std::{
    net::IpAddr,
    sync::{Arc, Mutex},
};

use local_ip_address::list_afinet_netifas;
use phonecam_discovery::{format_qr_code_uri, DiscoveredService, ServiceBrowser};
use phonecam_protocol::{CameraControl, Message};
use qrcode::{render::svg, QrCode};
use tauri::State;

pub mod adb;
pub mod convert;
pub mod decode;
pub mod pipeline;

#[cfg(target_os = "macos")]
pub mod driver_macos;

#[cfg(target_os = "windows")]
pub mod driver_windows;

pub struct AppState {
    pub pipeline: pipeline::PipelineManager,
    pub discovered_devices: Arc<Mutex<Vec<DiscoveredService>>>,
}

const DEFAULT_QR_DEVICE_NAME: &str = "PhoneCam Desktop";

fn desktop_device_name() -> String {
    std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .map(|value| value.trim().to_string())
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_QR_DEVICE_NAME.to_string())
}

fn local_interface_ips() -> Result<Vec<IpAddr>, String> {
    let mut ips: Vec<IpAddr> = list_afinet_netifas()
        .map_err(|err| format!("failed to list local network interfaces: {err}"))?
        .into_iter()
        .map(|(_, ip)| ip)
        .filter(|ip| !ip.is_loopback())
        .collect();

    ips.sort_unstable_by(|a, b| {
        let a_family = if a.is_ipv4() { 0 } else { 1 };
        let b_family = if b.is_ipv4() { 0 } else { 1 };
        a_family
            .cmp(&b_family)
            .then_with(|| a.to_string().cmp(&b.to_string()))
    });
    ips.dedup();

    if ips.is_empty() {
        return Err("no non-loopback local network interface detected".to_string());
    }

    Ok(ips)
}

fn qr_connection_uris() -> Result<Vec<String>, String> {
    let device_name = desktop_device_name();
    let uris = local_interface_ips()?
        .into_iter()
        .map(|ip| format_qr_code_uri(ip, pipeline::DEFAULT_LISTEN_PORT, &device_name))
        .collect();

    Ok(uris)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ConnectionTarget {
    Wifi,
    Usb { serial: Option<String> },
}

fn connection_target_from_input(ip: &str) -> ConnectionTarget {
    let trimmed = ip.trim();

    let serial = trimmed
        .strip_prefix("usb:")
        .or_else(|| trimmed.strip_prefix("adb:"))
        .or_else(|| trimmed.strip_prefix("usb://"))
        .or_else(|| trimmed.strip_prefix("adb://"))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);

    if serial.is_some() {
        return ConnectionTarget::Usb { serial };
    }

    if trimmed.eq_ignore_ascii_case("usb")
        || trimmed.eq_ignore_ascii_case("adb")
        || trimmed.eq_ignore_ascii_case("localhost")
        || trimmed == "127.0.0.1"
    {
        return ConnectionTarget::Usb { serial: None };
    }

    ConnectionTarget::Wifi
}

#[tauri::command]
pub async fn connect(state: State<'_, AppState>, ip: String, port: u16) -> Result<(), String> {
    let listen_port = if port == 0 {
        pipeline::DEFAULT_LISTEN_PORT
    } else {
        port
    };

    state.pipeline.stop().await?;

    match connection_target_from_input(&ip) {
        ConnectionTarget::Wifi => state.pipeline.start(listen_port).await,
        ConnectionTarget::Usb { serial } => state.pipeline.start_usb(listen_port, serial).await,
    }
}

#[tauri::command]
pub async fn disconnect(state: State<'_, AppState>) -> Result<(), String> {
    state.pipeline.stop().await
}

#[tauri::command]
pub async fn switch_camera(state: State<'_, AppState>, front: bool) -> Result<(), String> {
    let _message = Message::CameraControl(CameraControl::SwitchCamera { front });
    state
        .pipeline
        .switch_camera(front)
        .await
        .map_err(|err| format!("failed to send camera switch command: {err}"))
}

#[derive(serde::Serialize)]
pub struct Status {
    pub connected: bool,
    pub state: String,
    pub last_error: Option<String>,
}

#[tauri::command]
pub async fn get_status(state: State<'_, AppState>) -> Result<Status, String> {
    let pipeline_status = state.pipeline.status().await;

    Ok(Status {
        connected: pipeline_status.connected,
        state: pipeline_status.state,
        last_error: pipeline_status.last_error,
    })
}

#[derive(serde::Serialize)]
pub struct DeviceInfo {
    pub name: String,
    pub ip: String,
    pub port: u16,
}

#[tauri::command]
pub fn get_discovered_devices(state: State<'_, AppState>) -> Vec<DeviceInfo> {
    let devices = state.discovered_devices.lock().unwrap();
    devices
        .iter()
        .map(|d| DeviceInfo {
            name: d.name.clone(),
            ip: d.ip.to_string(),
            port: d.port,
        })
        .collect()
}

#[tauri::command]
pub fn generate_qr_code() -> Result<String, String> {
    let uris = qr_connection_uris()?;
    let primary_uri = uris
        .first()
        .ok_or_else(|| "no QR connection URI available".to_string())?;

    let qr_code = QrCode::new(primary_uri.as_bytes())
        .map_err(|err| format!("failed to generate QR code: {err}"))?;

    Ok(qr_code
        .render::<svg::Color>()
        .min_dimensions(240, 240)
        .build())
}

#[tauri::command]
pub fn get_qr_connection_uris() -> Result<Vec<String>, String> {
    qr_connection_uris()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let discovered_devices = Arc::new(Mutex::new(Vec::new()));
    let pipeline = pipeline::PipelineManager::new();

    tauri::Builder::default()
        .setup(move |app| {
            let discovered_devices_for_discovery = discovered_devices.clone();
            tokio::spawn(async move {
                let browser = match ServiceBrowser::new() {
                    Ok(b) => b,
                    Err(e) => {
                        eprintln!("Failed to create ServiceBrowser: {}", e);
                        return;
                    }
                };

                loop {
                    match browser.discover(std::time::Duration::from_secs(3)).await {
                        Ok(services) => {
                            let mut devices = discovered_devices_for_discovery.lock().unwrap();
                            *devices = services;
                        }
                        Err(e) => {
                            eprintln!("Discovery error: {}", e);
                        }
                    }
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                }
            });

            let pipeline_for_start = pipeline.clone();
            tokio::spawn(async move {
                if let Err(err) = pipeline_for_start
                    .start(pipeline::DEFAULT_LISTEN_PORT)
                    .await
                {
                    log::error!("failed to start streaming pipeline: {err}");
                }
            });

            app.manage(AppState {
                pipeline: pipeline.clone(),
                discovered_devices: discovered_devices.clone(),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            connect,
            disconnect,
            switch_camera,
            get_status,
            get_discovered_devices,
            generate_qr_code,
            get_qr_connection_uris
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};

    #[test]
    fn test_device_info_mapping() {
        let service = DiscoveredService {
            name: "PhoneCam Test".to_string(),
            ip: IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100)),
            port: 8080,
            version: "0.1.0".to_string(),
        };

        let services = vec![service];

        let infos: Vec<DeviceInfo> = services
            .iter()
            .map(|d| DeviceInfo {
                name: d.name.clone(),
                ip: d.ip.to_string(),
                port: d.port,
            })
            .collect();

        assert_eq!(infos.len(), 1);
        assert_eq!(infos[0].name, "PhoneCam Test");
        assert_eq!(infos[0].ip, "192.168.1.100");
        assert_eq!(infos[0].port, 8080);
    }
}
