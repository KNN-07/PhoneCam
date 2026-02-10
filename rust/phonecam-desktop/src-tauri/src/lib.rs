use std::sync::{Arc, Mutex};
use tauri::{State, Manager};
use phonecam_transport::client::PhoneCamClient;
use phonecam_discovery::{ServiceBrowser, DiscoveredService};
use tokio::sync::Mutex as TokioMutex;

pub mod convert;
pub mod decode;

pub struct AppState {
    pub client: Arc<TokioMutex<Option<PhoneCamClient>>>,
    pub discovered_devices: Arc<Mutex<Vec<DiscoveredService>>>,
}

#[tauri::command]
pub async fn connect(state: State<'_, AppState>, ip: String, port: u16) -> Result<(), String> {
    let mut client_guard = state.client.lock().await;
    
    if client_guard.is_some() {
        return Err("Already connected".into());
    }

    let addr = format!("{}:{}", ip, port);
    let socket_addr: std::net::SocketAddr = addr.parse().map_err(|e| format!("Invalid address: {}", e))?;

    match PhoneCamClient::connect(socket_addr).await {
        Ok(client) => {
            *client_guard = Some(client);
            Ok(())
        }
        Err(e) => Err(format!("Failed to connect: {}", e)),
    }
}

#[tauri::command]
pub async fn disconnect(state: State<'_, AppState>) -> Result<(), String> {
    let mut client_guard = state.client.lock().await;
    *client_guard = None;
    Ok(())
}

#[derive(serde::Serialize)]
pub struct Status {
    pub connected: bool,
}

#[tauri::command]
pub async fn get_status(state: State<'_, AppState>) -> Result<Status, String> {
    let client_guard = state.client.lock().await;
    Ok(Status {
        connected: client_guard.is_some(),
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
    devices.iter().map(|d| DeviceInfo {
        name: d.name.clone(),
        ip: d.ip.to_string(),
        port: d.port,
    }).collect()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let discovered_devices = Arc::new(Mutex::new(Vec::new()));
    let discovered_devices_clone = discovered_devices.clone();

    tauri::Builder::default()
        .setup(move |app| {
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
                            let mut devices = discovered_devices_clone.lock().unwrap();
                            *devices = services;
                        }
                        Err(e) => {
                            eprintln!("Discovery error: {}", e);
                        }
                    }
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                }
            });

            app.manage(AppState {
                client: Arc::new(TokioMutex::new(None)),
                discovered_devices,
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![connect, disconnect, get_status, get_discovered_devices])
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
        
        let infos: Vec<DeviceInfo> = services.iter().map(|d| DeviceInfo {
            name: d.name.clone(),
            ip: d.ip.to_string(),
            port: d.port,
        }).collect();

        assert_eq!(infos.len(), 1);
        assert_eq!(infos[0].name, "PhoneCam Test");
        assert_eq!(infos[0].ip, "192.168.1.100");
        assert_eq!(infos[0].port, 8080);
    }
}
