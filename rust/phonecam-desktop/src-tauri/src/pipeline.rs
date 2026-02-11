use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    panic::{catch_unwind, AssertUnwindSafe},
    path::PathBuf,
    sync::Arc,
};

use phonecam_discovery::ServicePublisher;
use phonecam_driver_linux::{ensure_v4l2loopback_loaded, list_devices, PixelFormat, V4l2Device};
use phonecam_protocol::{CameraControl, Message, VideoFrame};
use phonecam_transport::{ConnectionState, PhoneCamServer, TransportConnection};
use tokio::{
    sync::{mpsc, watch, Mutex as TokioMutex},
    task::JoinHandle,
};

use crate::{adb::AdbManager, convert::Nv12ToYuyvConverter, decode::H264Decoder};

pub const DEFAULT_LISTEN_PORT: u16 = 7_878;
const DEFAULT_DEVICE_NAME: &str = "PhoneCam Desktop";

#[derive(Debug, Clone, serde::Serialize)]
pub struct PipelineStatus {
    pub connected: bool,
    pub state: String,
    pub last_error: Option<String>,
}

impl PipelineStatus {
    fn disconnected() -> Self {
        Self {
            connected: false,
            state: "disconnected".to_string(),
            last_error: None,
        }
    }

    fn listening() -> Self {
        Self {
            connected: false,
            state: "listening".to_string(),
            last_error: None,
        }
    }

    fn connected() -> Self {
        Self {
            connected: true,
            state: "connected".to_string(),
            last_error: None,
        }
    }

    fn error(message: String) -> Self {
        Self {
            connected: false,
            state: "error".to_string(),
            last_error: Some(message),
        }
    }
}

#[derive(Clone)]
pub struct PipelineManager {
    runtime: Arc<TokioMutex<PipelineRuntime>>,
    adb_manager: AdbManager,
}

struct PipelineRuntime {
    status_tx: watch::Sender<PipelineStatus>,
    status_rx: watch::Receiver<PipelineStatus>,
    shutdown_tx: Option<watch::Sender<bool>>,
    worker: Option<JoinHandle<()>>,
    active_connection_sender: Arc<TokioMutex<Option<mpsc::Sender<Message>>>>,
    usb_forward: Option<UsbForwardSession>,
}

#[derive(Debug, Clone)]
struct UsbForwardSession {
    serial: String,
    local_port: u16,
}

#[derive(Debug, Clone)]
enum PipelineStartMode {
    Wifi,
    Usb { serial: Option<String> },
}

impl PipelineManager {
    pub fn new() -> Self {
        let (status_tx, status_rx) = watch::channel(PipelineStatus::disconnected());
        let active_connection_sender = Arc::new(TokioMutex::new(None));

        Self {
            runtime: Arc::new(TokioMutex::new(PipelineRuntime {
                status_tx,
                status_rx,
                shutdown_tx: None,
                worker: None,
                active_connection_sender,
                usb_forward: None,
            })),
            adb_manager: AdbManager::new(),
        }
    }

    pub async fn start(&self, port: u16) -> Result<(), String> {
        self.start_with_mode(port, PipelineStartMode::Wifi).await
    }

    pub async fn start_usb(&self, port: u16, serial: Option<String>) -> Result<(), String> {
        self.start_with_mode(port, PipelineStartMode::Usb { serial })
            .await
    }

    async fn start_with_mode(&self, port: u16, mode: PipelineStartMode) -> Result<(), String> {
        let listen_port = if port == 0 { DEFAULT_LISTEN_PORT } else { port };

        let mut runtime = self.runtime.lock().await;

        if let Some(existing_worker) = runtime.worker.as_ref() {
            if existing_worker.is_finished() {
                runtime.worker.take();
                runtime.shutdown_tx.take();
            } else {
                return Ok(());
            }
        }

        let usb_forward = match mode {
            PipelineStartMode::Wifi => None,
            PipelineStartMode::Usb { serial } => {
                let selected_serial = self
                    .adb_manager
                    .forward(listen_port, listen_port, serial.as_deref())
                    .await
                    .map_err(|err| format!("failed to set up ADB USB forward: {err}"))?;

                Some(UsbForwardSession {
                    serial: selected_serial,
                    local_port: listen_port,
                })
            }
        };

        let status_tx = runtime.status_tx.clone();
        let active_connection_sender = runtime.active_connection_sender.clone();
        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        runtime.usb_forward = usb_forward;
        runtime.shutdown_tx = Some(shutdown_tx);
        runtime.worker = Some(tokio::spawn(async move {
            run_pipeline(listen_port, status_tx, shutdown_rx, active_connection_sender).await;
        }));

        Ok(())
    }

    pub async fn stop(&self) -> Result<(), String> {
        let (shutdown_tx, worker, active_connection_sender, usb_forward) = {
            let mut runtime = self.runtime.lock().await;
            let shutdown_tx = runtime.shutdown_tx.take();
            let worker = runtime.worker.take();
            let active_connection_sender = runtime.active_connection_sender.clone();
            let usb_forward = runtime.usb_forward.take();

            if worker.is_none() {
                let _ = runtime.status_tx.send(PipelineStatus::disconnected());
            }

            (shutdown_tx, worker, active_connection_sender, usb_forward)
        };

        if let Some(shutdown_tx) = shutdown_tx {
            let _ = shutdown_tx.send(true);
        }

        if let Some(worker) = worker {
            worker
                .await
                .map_err(|err| format!("pipeline worker join failed: {err}"))?;
        }

        {
            let mut sender = active_connection_sender.lock().await;
            *sender = None;
        }

        if let Some(usb_forward) = usb_forward {
            if let Err(err) = self
                .adb_manager
                .kill_forward(usb_forward.local_port, Some(&usb_forward.serial))
                .await
            {
                log::warn!(
                    "failed to remove ADB forward for {} on tcp:{}: {}",
                    usb_forward.serial,
                    usb_forward.local_port,
                    err
                );
            }
        }

        let runtime = self.runtime.lock().await;
        let _ = runtime.status_tx.send(PipelineStatus::disconnected());

        Ok(())
    }

    pub async fn status(&self) -> PipelineStatus {
        let runtime = self.runtime.lock().await;
        runtime.status_rx.borrow().clone()
    }

    pub async fn switch_camera(&self, front: bool) -> Result<(), String> {
        let active_connection_sender = {
            let runtime = self.runtime.lock().await;
            runtime.active_connection_sender.clone()
        };

        let sender = {
            let sender = active_connection_sender.lock().await;
            sender.clone()
        }
        .ok_or_else(|| "no active phone connection for camera switch".to_string())?;

        sender
            .send(Message::CameraControl(CameraControl::SwitchCamera { front }))
            .await
            .map_err(|_| "failed to send camera switch command: connection closed".to_string())
    }
}

async fn run_pipeline(
    listen_port: u16,
    status_tx: watch::Sender<PipelineStatus>,
    mut shutdown_rx: watch::Receiver<bool>,
    active_connection_sender: Arc<TokioMutex<Option<mpsc::Sender<Message>>>>,
) {
    {
        let mut sender = active_connection_sender.lock().await;
        *sender = None;
    }

    let _publisher = match ServicePublisher::publish(
        DEFAULT_DEVICE_NAME,
        listen_port,
        env!("CARGO_PKG_VERSION"),
    ) {
        Ok(publisher) => publisher,
        Err(err) => {
            let _ = status_tx.send(PipelineStatus::error(format!(
                "mDNS advertisement failed: {err}"
            )));
            return;
        }
    };

    let listen_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), listen_port);
    let server = match PhoneCamServer::bind(listen_addr).await {
        Ok(server) => server,
        Err(err) => {
            let _ = status_tx.send(PipelineStatus::error(format!(
                "transport bind failed on {listen_addr}: {err}"
            )));
            return;
        }
    };

    let _ = status_tx.send(PipelineStatus::listening());

    let accepted = tokio::select! {
        _ = shutdown_rx.changed() => {
            let _ = status_tx.send(PipelineStatus::disconnected());
            return;
        }
        accepted = server.accept() => accepted,
    };

    let mut connection = match accepted {
        Ok(connection) => connection,
        Err(err) => {
            let _ = status_tx.send(PipelineStatus::error(format!(
                "transport accept failed: {err}"
            )));
            return;
        }
    };

    let _ = status_tx.send(PipelineStatus::connected());

    {
        let mut sender = active_connection_sender.lock().await;
        *sender = Some(connection.sender().clone());
    }

    match stream_connection(&mut connection, &mut shutdown_rx).await {
        Ok(StreamExit::PeerDisconnected) | Ok(StreamExit::ShutdownRequested) => {
            let _ = status_tx.send(PipelineStatus::disconnected());
        }
        Err(err) => {
            let _ = status_tx.send(PipelineStatus::error(err));
        }
    }

    let mut sender = active_connection_sender.lock().await;
    *sender = None;
}

enum StreamExit {
    PeerDisconnected,
    ShutdownRequested,
}

async fn stream_connection(
    connection: &mut TransportConnection,
    shutdown_rx: &mut watch::Receiver<bool>,
) -> Result<StreamExit, String> {
    let device = open_output_device()?;
    let mut decoder = catch_unwind(AssertUnwindSafe(H264Decoder::new))
        .map_err(|_| "decoder initialization panicked".to_string())?
        .map_err(|err| format!("decoder initialization failed: {err:?}"))?;

    let mut converter: Option<Nv12ToYuyvConverter> = None;
    let mut converter_width = 0u32;
    let mut converter_height = 0u32;
    let mut state_rx = connection.subscribe_state();

    loop {
        tokio::select! {
            changed = shutdown_rx.changed() => {
                let _ = changed;
                return Ok(StreamExit::ShutdownRequested);
            }
            changed = state_rx.changed() => {
                if changed.is_err() || *state_rx.borrow() == ConnectionState::Disconnected {
                    return Ok(StreamExit::PeerDisconnected);
                }
            }
            maybe_message = connection.receiver().recv() => {
                let Some(message) = maybe_message else {
                    return Ok(StreamExit::PeerDisconnected);
                };

                match message {
                    Message::VideoFrame(video_frame) => {
                        process_video_frame(
                            &device,
                            &mut decoder,
                            &mut converter,
                            &mut converter_width,
                            &mut converter_height,
                            video_frame,
                        )?;
                    }
                    Message::Disconnect(_) => {
                        return Ok(StreamExit::PeerDisconnected);
                    }
                    _ => {}
                }
            }
        }
    }
}

fn process_video_frame(
    device: &V4l2Device,
    decoder: &mut H264Decoder,
    converter: &mut Option<Nv12ToYuyvConverter>,
    converter_width: &mut u32,
    converter_height: &mut u32,
    video_frame: VideoFrame,
) -> Result<(), String> {
    let decode_output = match catch_unwind(AssertUnwindSafe(|| decoder.decode(&video_frame))) {
        Ok(Ok(output)) => output,
        Ok(Err(err)) => {
            log::warn!("failed to decode H.264 frame: {err:?}");
            return Ok(());
        }
        Err(_) => {
            return Err("decoder panicked while processing frame".to_string());
        }
    };

    for nv12_frame in decode_output.frames {
        if converter.is_none()
            || *converter_width != nv12_frame.width
            || *converter_height != nv12_frame.height
        {
            device
                .set_format(nv12_frame.width, nv12_frame.height, PixelFormat::YUYV)
                .map_err(|err| {
                    format!(
                        "failed to configure v4l2 output format {}x{}: {err}",
                        nv12_frame.width, nv12_frame.height
                    )
                })?;

            let converter_result = catch_unwind(AssertUnwindSafe(|| {
                Nv12ToYuyvConverter::new(nv12_frame.width, nv12_frame.height)
            }))
            .map_err(|_| "converter initialization panicked".to_string())?;

            *converter = Some(
                converter_result
                    .map_err(|err| format!("converter initialization failed: {err:?}"))?,
            );

            *converter_width = nv12_frame.width;
            *converter_height = nv12_frame.height;
        }

        let Some(converter_impl) = converter.as_mut() else {
            return Err("converter missing after initialization".to_string());
        };

        let yuyv_frame =
            match catch_unwind(AssertUnwindSafe(|| converter_impl.convert(&nv12_frame))) {
                Ok(Ok(frame)) => frame,
                Ok(Err(err)) => {
                    log::warn!("failed to convert NV12 frame to YUYV: {err:?}");
                    continue;
                }
                Err(_) => {
                    return Err("converter panicked while processing frame".to_string());
                }
            };

        device
            .write_frame(&yuyv_frame.data)
            .map_err(|err| format!("failed writing frame to {}: {err}", device.path().display()))?;
    }

    Ok(())
}

fn open_output_device() -> Result<V4l2Device, String> {
    ensure_v4l2loopback_loaded().map_err(|err| format!("v4l2loopback is unavailable: {err}"))?;

    let device_path = preferred_output_device_path().or_else(|| list_devices().into_iter().next());

    let Some(device_path) = device_path else {
        return Err("no V4L2 device found for output".to_string());
    };

    V4l2Device::open(&device_path).map_err(|err| {
        format!(
            "failed to open V4L2 device {}: {err}",
            device_path.display()
        )
    })
}

fn preferred_output_device_path() -> Option<PathBuf> {
    let value = std::env::var("PHONECAM_V4L2_DEVICE").ok()?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }

    Some(PathBuf::from(trimmed))
}

#[cfg(test)]
mod tests {
    use super::{PipelineStatus, DEFAULT_LISTEN_PORT};

    #[test]
    fn pipeline_status_defaults_are_consistent() {
        let disconnected = PipelineStatus::disconnected();
        assert!(!disconnected.connected);
        assert_eq!(disconnected.state, "disconnected");
        assert_eq!(disconnected.last_error, None);

        let listening = PipelineStatus::listening();
        assert!(!listening.connected);
        assert_eq!(listening.state, "listening");
        assert_eq!(listening.last_error, None);

        let connected = PipelineStatus::connected();
        assert!(connected.connected);
        assert_eq!(connected.state, "connected");
        assert_eq!(connected.last_error, None);
    }

    #[test]
    fn default_listen_port_matches_android_default() {
        assert_eq!(DEFAULT_LISTEN_PORT, 7_878);
    }
}
