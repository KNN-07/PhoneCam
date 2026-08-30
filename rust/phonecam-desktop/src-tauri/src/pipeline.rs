use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::Arc,
    time::Duration,
};

use phonecam_discovery::ServicePublisher;
use phonecam_protocol::{
    CameraControl, Handshake, Message, StreamConfigurationOutcome, StreamProfile, VideoCodec,
    PROTOCOL_VERSION, SUPPORTED_DIMENSIONS, SUPPORTED_FRAME_RATES,
};
use phonecam_transport::{ConnectionState, PhoneCamServer, TransportConnection};
use tokio::{
    sync::{mpsc, oneshot, watch, Mutex as TokioMutex},
    task::JoinHandle,
    time::{interval, Instant, MissedTickBehavior},
};

use crate::{
    adb::AdbManager,
    decode::{DecodeEvent, DecodeWorker},
    output::{NativeOutputFormat, OutputDevice},
};

pub const DEFAULT_LISTEN_PORT: u16 = 7_878;
const DEFAULT_DEVICE_NAME: &str = "PhoneCam Desktop";
const CONFIGURATION_TIMEOUT: Duration = Duration::from_secs(5);
const PIPELINE_COMMAND_CAPACITY: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodecPreference {
    H264,
    Hevc,
    Auto,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct PipelineStatus {
    pub connected: bool,
    pub state: String,
    pub last_error: Option<String>,
    pub supported_profiles: Vec<StreamProfile>,
    pub active_profile: Option<StreamProfile>,
    pub output_format: Option<NativeOutputFormat>,
}

impl PipelineStatus {
    fn disconnected() -> Self {
        Self {
            connected: false,
            state: "disconnected".to_owned(),
            last_error: None,
            supported_profiles: Vec::new(),
            active_profile: None,
            output_format: None,
        }
    }

    fn listening() -> Self {
        Self {
            state: "listening".to_owned(),
            ..Self::disconnected()
        }
    }

    fn connected(
        supported_profiles: Vec<StreamProfile>,
        active_profile: StreamProfile,
        output_format: Option<NativeOutputFormat>,
    ) -> Self {
        Self {
            connected: true,
            state: "connected".to_owned(),
            last_error: None,
            supported_profiles,
            active_profile: Some(active_profile),
            output_format,
        }
    }

    fn error(message: String) -> Self {
        Self {
            connected: false,
            state: "error".to_owned(),
            last_error: Some(message),
            supported_profiles: Vec::new(),
            active_profile: None,
            output_format: None,
        }
    }
}

#[derive(Clone)]
pub struct PipelineManager {
    runtime: Arc<TokioMutex<PipelineRuntime>>,
    configure_lock: Arc<TokioMutex<()>>,
    adb_manager: AdbManager,
}

struct PipelineRuntime {
    status_tx: watch::Sender<PipelineStatus>,
    status_rx: watch::Receiver<PipelineStatus>,
    shutdown_tx: Option<watch::Sender<bool>>,
    worker: Option<JoinHandle<()>>,
    active_connection_sender: Arc<TokioMutex<Option<mpsc::Sender<Message>>>>,
    active_command_sender: Arc<TokioMutex<Option<mpsc::Sender<PipelineCommand>>>>,
    usb_forward: Option<UsbForwardSession>,
}

struct PipelineCommand {
    width: u16,
    height: u16,
    fps: u8,
    preference: CodecPreference,
    reply: oneshot::Sender<Result<StreamProfile, String>>,
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
        Self {
            runtime: Arc::new(TokioMutex::new(PipelineRuntime {
                status_tx,
                status_rx,
                shutdown_tx: None,
                worker: None,
                active_connection_sender: Arc::new(TokioMutex::new(None)),
                active_command_sender: Arc::new(TokioMutex::new(None)),
                usb_forward: None,
            })),
            configure_lock: Arc::new(TokioMutex::new(())),
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
                    .reverse(listen_port, listen_port, serial.as_deref())
                    .await
                    .map_err(|error| format!("failed to set up ADB USB reverse tunnel: {error}"))?;
                Some(UsbForwardSession {
                    serial: selected_serial,
                    local_port: listen_port,
                })
            }
        };
        let status_tx = runtime.status_tx.clone();
        let active_connection_sender = runtime.active_connection_sender.clone();
        let active_command_sender = runtime.active_command_sender.clone();
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        runtime.usb_forward = usb_forward;
        runtime.shutdown_tx = Some(shutdown_tx);
        runtime.worker = Some(tokio::spawn(async move {
            run_pipeline(
                listen_port,
                status_tx,
                shutdown_rx,
                active_connection_sender,
                active_command_sender,
            )
            .await;
        }));
        Ok(())
    }

    pub async fn stop(&self) -> Result<(), String> {
        let (shutdown_tx, worker, connection_sender, command_sender, usb_forward) = {
            let mut runtime = self.runtime.lock().await;
            (
                runtime.shutdown_tx.take(),
                runtime.worker.take(),
                runtime.active_connection_sender.clone(),
                runtime.active_command_sender.clone(),
                runtime.usb_forward.take(),
            )
        };
        if let Some(shutdown_tx) = shutdown_tx {
            let _ = shutdown_tx.send(true);
        }
        if let Some(worker) = worker {
            worker
                .await
                .map_err(|error| format!("pipeline worker join failed: {error}"))?;
        }
        *connection_sender.lock().await = None;
        *command_sender.lock().await = None;
        if let Some(usb_forward) = usb_forward {
            if let Err(error) = self
                .adb_manager
                .kill_reverse(usb_forward.local_port, Some(&usb_forward.serial))
                .await
            {
                log::warn!("failed to remove ADB reverse tunnel: {error}");
            }
        }
        let runtime = self.runtime.lock().await;
        let _ = runtime.status_tx.send(PipelineStatus::disconnected());
        Ok(())
    }

    pub async fn status(&self) -> PipelineStatus {
        self.runtime.lock().await.status_rx.borrow().clone()
    }

    pub async fn switch_camera(&self, front: bool) -> Result<(), String> {
        let sender = {
            let runtime = self.runtime.lock().await;
            runtime.active_connection_sender.clone()
        }
        .lock()
        .await
        .clone()
        .ok_or_else(|| "no active phone connection for camera switch".to_owned())?;
        sender
            .send(Message::CameraControl(CameraControl::SwitchCamera {
                front,
            }))
            .await
            .map_err(|_| "failed to send camera switch: connection closed".to_owned())
    }

    pub async fn configure_stream(
        &self,
        width: u16,
        height: u16,
        fps: u8,
        preference: CodecPreference,
    ) -> Result<StreamProfile, String> {
        let _serialized = self.configure_lock.lock().await;
        let command_sender = {
            let runtime = self.runtime.lock().await;
            runtime.active_command_sender.clone()
        }
        .lock()
        .await
        .clone()
        .ok_or_else(|| "no active phone connection for stream configuration".to_owned())?;
        let (reply, response) = oneshot::channel();
        command_sender
            .send(PipelineCommand {
                width,
                height,
                fps,
                preference,
                reply,
            })
            .await
            .map_err(|_| "stream configuration actor stopped".to_owned())?;
        response
            .await
            .map_err(|_| "stream configuration actor dropped its reply".to_owned())?
    }
}

impl Default for PipelineManager {
    fn default() -> Self {
        Self::new()
    }
}

async fn run_pipeline(
    listen_port: u16,
    status_tx: watch::Sender<PipelineStatus>,
    mut shutdown_rx: watch::Receiver<bool>,
    active_connection_sender: Arc<TokioMutex<Option<mpsc::Sender<Message>>>>,
    active_command_sender: Arc<TokioMutex<Option<mpsc::Sender<PipelineCommand>>>>,
) {
    *active_connection_sender.lock().await = None;
    *active_command_sender.lock().await = None;

    let output = match OutputDevice::open() {
        Ok(output) => output,
        Err(error) => {
            let _ = status_tx.send(PipelineStatus::error(error));
            return;
        }
    };
    let local_profiles = qualified_output_profiles(&output);
    if !local_profiles.contains(&StreamProfile::H264_720P30) {
        let _ = status_tx.send(PipelineStatus::error(
            "decoder/output qualification rejected mandatory H.264 720p30".to_owned(),
        ));
        return;
    }
    let local_handshake = Handshake {
        version: PROTOCOL_VERSION,
        device_name: DEFAULT_DEVICE_NAME.to_owned(),
        supported_profiles: local_profiles.clone(),
        active_profile: None,
    };
    let _publisher = match ServicePublisher::publish(
        DEFAULT_DEVICE_NAME,
        listen_port,
        env!("CARGO_PKG_VERSION"),
    ) {
        Ok(publisher) => publisher,
        Err(error) => {
            let _ = status_tx.send(PipelineStatus::error(format!(
                "mDNS advertisement failed: {error}"
            )));
            return;
        }
    };
    let listen_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), listen_port);
    let server = match PhoneCamServer::bind(listen_addr, local_handshake).await {
        Ok(server) => server,
        Err(error) => {
            let _ = status_tx.send(PipelineStatus::error(format!(
                "transport bind failed on {listen_addr}: {error}"
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
        Err(error) => {
            let _ = status_tx.send(PipelineStatus::error(format!(
                "transport accept failed: {error}"
            )));
            return;
        }
    };
    let (command_tx, command_rx) = mpsc::channel(PIPELINE_COMMAND_CAPACITY);
    *active_connection_sender.lock().await = Some(connection.sender().clone());
    *active_command_sender.lock().await = Some(command_tx);

    let result = stream_connection(
        &mut connection,
        output,
        local_profiles,
        command_rx,
        &status_tx,
        &mut shutdown_rx,
    )
    .await;
    match result {
        Ok(StreamExit::PeerDisconnected | StreamExit::ShutdownRequested) => {
            let _ = status_tx.send(PipelineStatus::disconnected());
        }
        Err(error) => {
            let _ = status_tx.send(PipelineStatus::error(error));
        }
    }
    *active_connection_sender.lock().await = None;
    *active_command_sender.lock().await = None;
}

fn qualified_output_profiles(output: &OutputDevice) -> Vec<StreamProfile> {
    let mut profiles = Vec::new();
    for codec in [VideoCodec::H264, VideoCodec::Hevc] {
        for (width, height) in SUPPORTED_DIMENSIONS {
            for fps in SUPPORTED_FRAME_RATES {
                let profile = StreamProfile {
                    codec,
                    width,
                    height,
                    fps,
                };
                if output.preflight(&profile).is_ok() {
                    profiles.push(profile);
                }
            }
        }
    }
    profiles
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StreamExit {
    PeerDisconnected,
    ShutdownRequested,
}

enum PendingKind {
    Configure {
        prior: StreamProfile,
        reply: Option<oneshot::Sender<Result<StreamProfile, String>>>,
    },
    Compensation {
        prior: StreamProfile,
        error: String,
        reply: Option<oneshot::Sender<Result<StreamProfile, String>>>,
    },
}

struct PendingConfiguration {
    request_id: u32,
    requested: StreamProfile,
    deadline: Instant,
    kind: PendingKind,
}

fn next_request_id(counter: &mut u32) -> u32 {
    *counter = counter.wrapping_add(1);
    if *counter == 0 {
        *counter = 1;
    }
    *counter
}

fn common_profiles(local: &[StreamProfile], peer: &[StreamProfile]) -> Vec<StreamProfile> {
    local
        .iter()
        .copied()
        .filter(|profile| peer.contains(profile))
        .collect()
}

fn resolve_profile(
    common: &[StreamProfile],
    width: u16,
    height: u16,
    fps: u8,
    preference: CodecPreference,
) -> Result<StreamProfile, String> {
    let codec_order: &[VideoCodec] = match preference {
        CodecPreference::H264 => &[VideoCodec::H264],
        CodecPreference::Hevc => &[VideoCodec::Hevc],
        CodecPreference::Auto => &[VideoCodec::Hevc, VideoCodec::H264],
    };
    codec_order
        .iter()
        .find_map(|codec| {
            common.iter().copied().find(|profile| {
                profile.codec == *codec
                    && profile.width == width
                    && profile.height == height
                    && profile.fps == fps
            })
        })
        .ok_or_else(|| format!("unsupported common stream profile {width}x{height}@{fps}"))
}

fn accepts_applied_profile(
    common: &[StreamProfile],
    requested: StreamProfile,
    applied: StreamProfile,
) -> bool {
    common.contains(&applied)
        && applied.width == requested.width
        && applied.height == requested.height
        && applied.fps == requested.fps
}

async fn send_configuration(
    connection: &TransportConnection,
    request_id: u32,
    profile: StreamProfile,
) -> Result<(), String> {
    connection
        .sender()
        .send(Message::CameraControl(CameraControl::ConfigureStream {
            request_id,
            profile,
        }))
        .await
        .map_err(|_| "failed to send stream configuration: connection closed".to_owned())
}

async fn stream_connection(
    connection: &mut TransportConnection,
    mut output: OutputDevice,
    local_profiles: Vec<StreamProfile>,
    mut command_rx: mpsc::Receiver<PipelineCommand>,
    status_tx: &watch::Sender<PipelineStatus>,
    shutdown_rx: &mut watch::Receiver<bool>,
) -> Result<StreamExit, String> {
    let peer_handshake = connection.peer_handshake().clone();
    let mut peer_profiles = peer_handshake.supported_profiles;
    let mut common = common_profiles(&local_profiles, &peer_profiles);
    let mut active = peer_handshake
        .active_profile
        .ok_or_else(|| "mobile handshake omitted its active stream profile".to_owned())?;
    if !common.contains(&active) {
        return Err("mobile active profile is outside the common profile set".to_owned());
    }
    output.commit(&active)?;
    let mut decoder = DecodeWorker::new()
        .map_err(|error| format!("decoder worker initialization failed: {error:?}"))?;
    decoder
        .reset(active.codec)
        .await
        .map_err(|error| format!("initial decoder reset failed: {error:?}"))?;
    let _ = status_tx.send(PipelineStatus::connected(
        common.clone(),
        active,
        output.output_format(),
    ));

    let mut state_rx = connection.subscribe_state();
    let mut ticker = interval(Duration::from_millis(100));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut request_counter = 0u32;
    let mut pending: Option<PendingConfiguration> = None;
    let mut keyframe_request_in_flight = false;

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
            _ = ticker.tick() => {
                if pending.as_ref().is_some_and(|pending| Instant::now() >= pending.deadline) {
                    let expired = pending.take().expect("checked pending");
                    match expired.kind {
                        PendingKind::Configure { reply, .. } => {
                            if let Some(reply) = reply {
                                let _ = reply.send(Err("stream configuration timed out".to_owned()));
                            }
                        }
                        PendingKind::Compensation { error, reply, .. } => {
                            if let Some(reply) = reply {
                                let _ = reply.send(Err(error.clone()));
                            }
                            return Err(format!("stream compensation timed out after: {error}"));
                        }
                    }
                }
            }
            maybe_command = command_rx.recv(), if pending.is_none() => {
                let Some(command) = maybe_command else {
                    return Err("pipeline command channel closed".to_owned());
                };
                let requested = match resolve_profile(
                    &common,
                    command.width,
                    command.height,
                    command.fps,
                    command.preference,
                ) {
                    Ok(profile) => profile,
                    Err(error) => {
                        let _ = command.reply.send(Err(error));
                        continue;
                    }
                };
                if let Err(error) = output.preflight(&requested) {
                    let _ = command.reply.send(Err(error));
                    continue;
                }
                if requested == active {
                    let _ = command.reply.send(Ok(active));
                    continue;
                }
                let request_id = next_request_id(&mut request_counter);
                send_configuration(connection, request_id, requested).await?;
                pending = Some(PendingConfiguration {
                    request_id,
                    requested,
                    deadline: Instant::now() + CONFIGURATION_TIMEOUT,
                    kind: PendingKind::Configure {
                        prior: active,
                        reply: Some(command.reply),
                    },
                });
            }
            maybe_event = decoder.recv() => {
                let Some(event) = maybe_event else {
                    return Err("decoder worker stopped".to_owned());
                };
                match event {
                    DecodeEvent::Frame(frame) => {
                        output.write_frame(&frame, frame.pts_us.saturating_mul(1_000))?;
                    }
                    DecodeEvent::RecoveryRequired => {
                        if !keyframe_request_in_flight {
                            connection.sender()
                                .send(Message::CameraControl(CameraControl::RequestKeyframe))
                                .await
                                .map_err(|_| "failed to request recovery keyframe".to_owned())?;
                            keyframe_request_in_flight = true;
                        }
                    }
                    DecodeEvent::Error(error) => {
                        log::warn!("decoder error: {error}");
                    }
                }
            }
            maybe_message = connection.receiver().recv() => {
                let Some(message) = maybe_message else {
                    return Ok(StreamExit::PeerDisconnected);
                };
                match message {
                    Message::VideoFrame(frame) => {
                        if frame.validate_against(active).is_err() {
                            continue;
                        }
                        if frame.is_keyframe {
                            keyframe_request_in_flight = false;
                        }
                        if let Err(error) = decoder.try_send(frame) {
                            if error.into_inner().is_keyframe && !keyframe_request_in_flight {
                                connection.sender()
                                    .send(Message::CameraControl(CameraControl::RequestKeyframe))
                                    .await
                                    .map_err(|_| "failed to request keyframe after decoder backlog".to_owned())?;
                                keyframe_request_in_flight = true;
                            }
                        }
                    }
                    Message::VideoCapabilitiesUpdate(update) => {
                        if update.validate(active).is_ok() {
                            peer_profiles = update.supported_profiles;
                            common = common_profiles(&local_profiles, &peer_profiles);
                            let _ = status_tx.send(PipelineStatus::connected(
                                common.clone(),
                                active,
                                output.output_format(),
                            ));
                        }
                    }
                    Message::StreamConfigurationResult(result) => {
                        if result.request_id == 0 && pending.is_none() {
                            if let StreamConfigurationOutcome::Applied(applied) = result.result {
                                if common.contains(&applied) {
                                    let prior = active;
                                    let commit_result = match output.commit(&applied) {
                                        Ok(()) => decoder
                                            .reset(applied.codec)
                                            .await
                                            .map_err(|error| {
                                                format!("decoder reset failed: {error:?}")
                                            }),
                                        Err(error) => Err(error),
                                    };
                                    match commit_result {
                                        Ok(()) => {
                                            active = applied;
                                            let _ = status_tx.send(PipelineStatus::connected(
                                                common.clone(),
                                                active,
                                                output.output_format(),
                                            ));
                                        }
                                        Err(error) => {
                                            let request_id = next_request_id(&mut request_counter);
                                            send_configuration(connection, request_id, prior).await?;
                                            pending = Some(PendingConfiguration {
                                                request_id,
                                                requested: prior,
                                                deadline: Instant::now() + CONFIGURATION_TIMEOUT,
                                                kind: PendingKind::Compensation {
                                                    prior,
                                                    error,
                                                    reply: None,
                                                },
                                            });
                                        }
                                    }
                                }
                            }
                            continue;
                        }
                        let Some(current) = pending.take() else {
                            continue;
                        };
                        if result.request_id != current.request_id {
                            pending = Some(current);
                            continue;
                        }
                        match current.kind {
                            PendingKind::Configure { prior, reply } => {
                                match result.result {
                                    StreamConfigurationOutcome::Applied(applied)
                                        if accepts_applied_profile(&common, current.requested, applied) =>
                                    {
                                        let commit = output.commit(&applied);
                                        let reset = if commit.is_ok() {
                                            decoder.reset(applied.codec).await
                                                .map_err(|error| format!("decoder reset failed: {error:?}"))
                                        } else {
                                            Ok(())
                                        };
                                        if let Err(error) = commit.and(reset) {
                                            let request_id = next_request_id(&mut request_counter);
                                            send_configuration(connection, request_id, prior).await?;
                                            pending = Some(PendingConfiguration {
                                                request_id,
                                                requested: prior,
                                                deadline: Instant::now() + CONFIGURATION_TIMEOUT,
                                                kind: PendingKind::Compensation {
                                                    prior,
                                                    error: format!("failed to commit applied output profile: {error}"),
                                                    reply,
                                                },
                                            });
                                        } else {
                                            active = applied;
                                            let _ = status_tx.send(PipelineStatus::connected(
                                                common.clone(),
                                                active,
                                                output.output_format(),
                                            ));
                                            if let Some(reply) = reply {
                                                let _ = reply.send(Ok(applied));
                                            }
                                        }
                                    }
                                    outcome => {
                                        if let Some(reply) = reply {
                                            let _ = reply.send(Err(format!(
                                                "phone rejected stream configuration: {outcome:?}"
                                            )));
                                        }
                                    }
                                }
                            }
                            PendingKind::Compensation { prior, error, reply } => {
                                match result.result {
                                    StreamConfigurationOutcome::Applied(applied) if applied == prior => {
                                        output.commit(&prior)
                                            .map_err(|restore| format!(
                                                "stream compensation output restore failed after {error}: {restore}"
                                            ))?;
                                        decoder.reset(prior.codec).await
                                            .map_err(|restore| format!(
                                                "stream compensation decoder restore failed after {error}: {restore:?}"
                                            ))?;
                                        active = prior;
                                        let _ = status_tx.send(PipelineStatus::connected(
                                            common.clone(),
                                            active,
                                            output.output_format(),
                                        ));
                                        if let Some(reply) = reply {
                                            let _ = reply.send(Err(error));
                                        }
                                    }
                                    outcome => {
                                        if let Some(reply) = reply {
                                            let _ = reply.send(Err(error.clone()));
                                        }
                                        return Err(format!(
                                            "stream compensation failed after {error}: {outcome:?}"
                                        ));
                                    }
                                }
                            }
                        }
                    }
                    Message::Disconnect(_) => return Ok(StreamExit::PeerDisconnected),
                    _ => {}
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_status_and_port_are_stable() {
        let status = PipelineStatus::disconnected();
        assert!(!status.connected);
        assert_eq!(status.state, "disconnected");
        assert!(status.supported_profiles.is_empty());
        assert_eq!(DEFAULT_LISTEN_PORT, 7_878);
    }

    #[test]
    fn auto_prefers_hevc_and_falls_back_to_h264() {
        let h264 = StreamProfile::H264_720P30;
        let hevc = StreamProfile {
            codec: VideoCodec::Hevc,
            ..h264
        };
        assert_eq!(
            resolve_profile(&[h264, hevc], 1280, 720, 30, CodecPreference::Auto),
            Ok(hevc)
        );
        assert_eq!(
            resolve_profile(&[h264], 1280, 720, 30, CodecPreference::Auto),
            Ok(h264)
        );
    }

    #[test]
    fn applied_result_accepts_exact_tuple_and_codec_fallback_only_when_common() {
        let requested = StreamProfile {
            codec: VideoCodec::Hevc,
            width: 3840,
            height: 2160,
            fps: 60,
        };
        let fallback = StreamProfile {
            codec: VideoCodec::H264,
            ..requested
        };
        assert!(accepts_applied_profile(
            &[requested, fallback],
            requested,
            requested
        ));
        assert!(accepts_applied_profile(
            &[requested, fallback],
            requested,
            fallback
        ));
        assert!(!accepts_applied_profile(&[requested], requested, fallback));
        assert!(!accepts_applied_profile(
            &[requested, fallback],
            requested,
            StreamProfile {
                fps: 30,
                ..fallback
            }
        ));
    }

    #[test]
    fn request_ids_wrap_without_zero() {
        let mut counter = u32::MAX;
        assert_eq!(next_request_id(&mut counter), 1);
    }
}
