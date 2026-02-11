use std::sync::{Arc, Mutex};

use phonecam_discovery::{DiscoveredService, ServiceBrowser};
use tauri::State;

pub mod convert;
pub mod decode;
pub mod pipeline;

pub struct AppState {
    pub pipeline: pipeline::PipelineManager,
    pub discovered_devices: Arc<Mutex<Vec<DiscoveredService>>>,
}

#[tauri::command]
pub async fn connect(state: State<'_, AppState>, _ip: String, port: u16) -> Result<(), String> {
    let listen_port = if port == 0 {
        pipeline::DEFAULT_LISTEN_PORT
    } else {
        port
    };

    state.pipeline.start(listen_port).await
}

#[tauri::command]
pub async fn disconnect(state: State<'_, AppState>) -> Result<(), String> {
    state.pipeline.stop().await
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
            get_status,
            get_discovered_devices
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
