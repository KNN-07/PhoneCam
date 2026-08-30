use std::net::IpAddr;

use local_ip_address::list_afinet_netifas;
use phonecam_discovery::format_qr_code_uri;
use qrcode::{render::svg, QrCode};
use tauri::{Manager, State};

pub mod adb;
pub mod convert;
pub mod decode;
mod output;
pub mod pipeline;

#[cfg(target_os = "macos")]
pub mod driver_macos;

#[cfg(target_os = "windows")]
pub mod driver_windows;

pub struct AppState {
    pub pipeline: pipeline::PipelineManager,
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
async fn connect(state: State<'_, AppState>, ip: String, port: u16) -> Result<(), String> {
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
async fn disconnect(state: State<'_, AppState>) -> Result<(), String> {
    state.pipeline.stop().await
}

#[tauri::command]
async fn switch_camera(state: State<'_, AppState>, front: bool) -> Result<(), String> {
    state
        .pipeline
        .switch_camera(front)
        .await
        .map_err(|err| format!("failed to send camera switch command: {err}"))
}

#[tauri::command]
async fn configure_stream(
    state: State<'_, AppState>,
    width: u16,
    height: u16,
    fps: u8,
) -> Result<(), String> {
    validate_stream_configuration(width, height, fps)?;
    state
        .pipeline
        .configure_stream(width, height, fps)
        .await
        .map_err(|err| format!("failed to configure phone stream: {err}"))
}

fn validate_stream_configuration(width: u16, height: u16, fps: u8) -> Result<(), String> {
    const RESOLUTIONS: &[(u16, u16)] = &[(640, 480), (1280, 720), (1920, 1080)];
    const FRAME_RATES: &[u8] = &[15, 30, 60];

    if !RESOLUTIONS.contains(&(width, height)) {
        return Err(format!("unsupported resolution {width}x{height}"));
    }
    if !FRAME_RATES.contains(&fps) {
        return Err(format!("unsupported frame rate {fps}"));
    }
    Ok(())
}

#[derive(serde::Serialize)]
pub struct Status {
    pub connected: bool,
    pub state: String,
    pub last_error: Option<String>,
}

#[tauri::command]
async fn get_status(state: State<'_, AppState>) -> Result<Status, String> {
    let pipeline_status = state.pipeline.status().await;

    Ok(Status {
        connected: pipeline_status.connected,
        state: pipeline_status.state,
        last_error: pipeline_status.last_error,
    })
}

#[tauri::command]
fn generate_qr_code() -> Result<String, String> {
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
fn get_qr_connection_uris() -> Result<Vec<String>, String> {
    qr_connection_uris()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let pipeline = pipeline::PipelineManager::new();

    tauri::Builder::default()
        .setup(move |app| {
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
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            connect,
            disconnect,
            switch_camera,
            configure_stream,
            get_status,
            generate_qr_code,
            get_qr_connection_uris
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_configuration_only_accepts_v1_presets() {
        assert!(validate_stream_configuration(640, 480, 15).is_ok());
        assert!(validate_stream_configuration(1280, 720, 30).is_ok());
        assert!(validate_stream_configuration(1920, 1080, 60).is_ok());
        assert!(validate_stream_configuration(800, 600, 30).is_err());
        assert!(validate_stream_configuration(1280, 720, 24).is_err());
    }
}
