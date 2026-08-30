#![allow(dead_code, clippy::large_const_arrays)]

use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use phonecam_discovery::{parse_qr_code_uri, DiscoveredService, ServiceBrowser};
use phonecam_protocol::{
    CameraControl, Handshake, Message, StreamConfigurationOutcome, StreamConfigurationResult,
    StreamProfile, VideoCapabilitiesUpdate, VideoCodec, VideoFrame, PROTOCOL_VERSION,
};
use phonecam_transport::{ConnectionState, PhoneCamClient, TransportConnection};
use serde::Deserialize;
use tokio::runtime::{Builder, Runtime};
use tokio::sync::mpsc::error::TryRecvError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FrameMetadata {
    len: usize,
    pts: u64,
    codec: VideoCodec,
    width: u16,
    height: u16,
    is_keyframe: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CoreConfig {
    endpoint_host: String,
    endpoint_port: u16,
    streaming_enabled: bool,
}

impl Default for CoreConfig {
    fn default() -> Self {
        Self {
            endpoint_host: "127.0.0.1".to_string(),
            endpoint_port: 7878,
            streaming_enabled: false,
        }
    }
}

struct TransportClient {
    runtime: Runtime,
    connection: TransportConnection,
    supported_profiles: Vec<StreamProfile>,
    active_profile: StreamProfile,
    pending_configuration: Option<(u32, StreamProfile)>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct VideoConfig {
    active_profile: StreamProfile,
    supported_profiles: Vec<StreamProfile>,
}

impl VideoConfig {
    fn validate(&self) -> bool {
        Handshake {
            version: PROTOCOL_VERSION,
            device_name: "PhoneCam Mobile".to_owned(),
            supported_profiles: self.supported_profiles.clone(),
            active_profile: Some(self.active_profile),
        }
        .validate()
        .is_ok()
    }
}

static LAST_FRAME: OnceLock<Mutex<Option<FrameMetadata>>> = OnceLock::new();
static CORE_CONFIG: OnceLock<Mutex<CoreConfig>> = OnceLock::new();
static TRANSPORT_CLIENT: OnceLock<Mutex<Option<TransportClient>>> = OnceLock::new();

const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);

fn last_frame_slot() -> &'static Mutex<Option<FrameMetadata>> {
    LAST_FRAME.get_or_init(|| Mutex::new(None))
}

fn core_config_slot() -> &'static Mutex<CoreConfig> {
    CORE_CONFIG.get_or_init(|| Mutex::new(CoreConfig::default()))
}

fn transport_client_slot() -> &'static Mutex<Option<TransportClient>> {
    TRANSPORT_CLIENT.get_or_init(|| Mutex::new(None))
}

pub fn ffi_test_message() -> String {
    "phonecam-mobile-core ffi ok".to_string()
}

pub fn configure_endpoint(host: String, port: u16) {
    if host.trim().is_empty() {
        return;
    }

    if let Ok(mut config) = core_config_slot().lock() {
        config.endpoint_host = host;
        config.endpoint_port = port;
    }
}

pub fn current_endpoint() -> String {
    if let Ok(config) = core_config_slot().lock() {
        return format!("{}:{}", config.endpoint_host, config.endpoint_port);
    }

    format!(
        "{}:{}",
        CoreConfig::default().endpoint_host,
        CoreConfig::default().endpoint_port
    )
}

pub fn set_streaming_enabled(enabled: bool) {
    if let Ok(mut config) = core_config_slot().lock() {
        config.streaming_enabled = enabled;
    }
}

pub fn is_streaming_enabled() -> bool {
    if let Ok(config) = core_config_slot().lock() {
        return config.streaming_enabled;
    }

    false
}

fn initialize_transport_client(host: String, port: u16, video_config: VideoConfig) -> bool {
    if host.trim().is_empty() || !video_config.validate() {
        return false;
    }

    let endpoint = format!("{}:{}", host, port);
    let runtime = match Builder::new_multi_thread()
        .enable_all()
        .worker_threads(2)
        .thread_name("phonecam-mobile-transport")
        .build()
    {
        Ok(runtime) => runtime,
        Err(_) => return false,
    };
    let handshake = Handshake {
        version: PROTOCOL_VERSION,
        device_name: "PhoneCam Mobile".to_owned(),
        supported_profiles: video_config.supported_profiles.clone(),
        active_profile: Some(video_config.active_profile),
    };
    let connection = match runtime.block_on(tokio::time::timeout(
        CONNECT_TIMEOUT,
        PhoneCamClient::connect(endpoint, handshake),
    )) {
        Ok(Ok(connection)) => connection,
        Err(_) | Ok(Err(_)) => return false,
    };

    if let Ok(mut slot) = transport_client_slot().lock() {
        *slot = Some(TransportClient {
            runtime,
            connection,
            supported_profiles: video_config.supported_profiles,
            active_profile: video_config.active_profile,
            pending_configuration: None,
        });
        return true;
    }
    false
}

fn shutdown_transport_client() {
    if let Ok(mut slot) = transport_client_slot().lock() {
        *slot = None;
    }
}

/// Raw C FFI entry point for codec-bearing Annex-B frame submission.
///
/// # Safety
///
/// `data` must point to `len` readable bytes for this call.
#[no_mangle]
pub unsafe extern "C" fn phonecam_send_video_frame(
    data: *const u8,
    len: usize,
    pts: u64,
    codec: u8,
    width: u16,
    height: u16,
    is_keyframe: bool,
) -> bool {
    if data.is_null() || len == 0 {
        return false;
    }
    let codec = match VideoCodec::try_from(codec) {
        Ok(codec) => codec,
        Err(_) => return false,
    };
    let payload = unsafe { std::slice::from_raw_parts(data, len) };
    let mut slot = match transport_client_slot().lock() {
        Ok(slot) => slot,
        Err(_) => return false,
    };
    let Some(client) = slot.as_mut() else {
        return false;
    };
    let frame = VideoFrame {
        data: payload.to_vec().into(),
        pts_us: pts,
        codec,
        width,
        height,
        is_keyframe,
    };
    if frame.validate_against(client.active_profile).is_err() {
        return false;
    }
    if !try_enqueue(client.connection.sender(), Message::VideoFrame(frame)) {
        return false;
    }
    if let Ok(mut metadata) = last_frame_slot().lock() {
        *metadata = Some(FrameMetadata {
            len,
            pts,
            codec,
            width,
            height,
            is_keyframe,
        });
    }
    true
}

/// Initialize transport from exact, validated profile metadata.
///
/// # Safety
///
/// `host` and `video_config_json` must be valid NUL-terminated UTF-8 strings.
#[no_mangle]
pub unsafe extern "C" fn phonecam_transport_init(
    host: *const c_char,
    port: u16,
    video_config_json: *const c_char,
) -> bool {
    if host.is_null() || video_config_json.is_null() || port == 0 {
        return false;
    }
    let host = match unsafe { CStr::from_ptr(host) }.to_str() {
        Ok(host) if !host.trim().is_empty() => host.trim(),
        _ => return false,
    };
    let config_json = match unsafe { CStr::from_ptr(video_config_json) }.to_str() {
        Ok(config_json) => config_json,
        Err(_) => return false,
    };
    let video_config: VideoConfig = match serde_json::from_str::<VideoConfig>(config_json) {
        Ok(config) if config.validate() => config,
        _ => return false,
    };

    configure_endpoint(host.to_owned(), port);
    let initialized = initialize_transport_client(host.to_owned(), port, video_config);
    set_streaming_enabled(initialized);
    initialized
}

#[no_mangle]
pub extern "C" fn phonecam_transport_shutdown() {
    set_streaming_enabled(false);
    shutdown_transport_client();
}

#[no_mangle]
pub extern "C" fn phonecam_transport_is_connected() -> bool {
    transport_client_slot()
        .lock()
        .ok()
        .and_then(|slot| {
            slot.as_ref().map(|client| {
                matches!(
                    client.connection.current_state(),
                    ConnectionState::Handshaking | ConnectionState::Streaming
                )
            })
        })
        .unwrap_or(false)
}

fn control_command_json(control: &CameraControl) -> Option<String> {
    let value = match control {
        CameraControl::SwitchCamera { front } => {
            serde_json::json!({"type": "switch_camera", "front": front})
        }
        CameraControl::RequestKeyframe => serde_json::json!({"type": "request_keyframe"}),
        CameraControl::ConfigureStream {
            request_id,
            profile,
        } => serde_json::json!({
            "type": "configure_stream",
            "request_id": request_id,
            "profile": profile,
        }),
    };
    serde_json::to_string(&value).ok()
}

#[no_mangle]
pub extern "C" fn phonecam_poll_control_command_json() -> *mut c_char {
    let mut slot = match transport_client_slot().lock() {
        Ok(slot) => slot,
        Err(_) => return std::ptr::null_mut(),
    };
    let Some(client) = slot.as_mut() else {
        return std::ptr::null_mut();
    };
    if client.pending_configuration.is_some() {
        return std::ptr::null_mut();
    }

    loop {
        let control = match client.connection.receiver().try_recv() {
            Ok(Message::CameraControl(control)) => control,
            Ok(Message::Disconnect(_)) | Err(TryRecvError::Disconnected) => {
                *slot = None;
                return std::ptr::null_mut();
            }
            Ok(_) => continue,
            Err(TryRecvError::Empty) => return std::ptr::null_mut(),
        };
        if let CameraControl::ConfigureStream {
            request_id,
            profile,
        } = control
        {
            if profile.validate().is_err() {
                continue;
            }
            client.pending_configuration = Some((request_id, profile));
        }
        return control_command_json(&control)
            .and_then(|json| CString::new(json).ok())
            .map(CString::into_raw)
            .unwrap_or(std::ptr::null_mut());
    }
}

fn profile_from_ffi(codec: u8, width: u16, height: u16, fps: u8) -> Option<StreamProfile> {
    let profile = StreamProfile {
        codec: VideoCodec::try_from(codec).ok()?,
        width,
        height,
        fps,
    };
    profile.validate().ok()?;
    Some(profile)
}
fn try_enqueue(sender: &tokio::sync::mpsc::Sender<Message>, message: Message) -> bool {
    sender.try_send(message).is_ok()
}

#[no_mangle]
pub extern "C" fn phonecam_peer_supports_profile(
    codec: u8,
    width: u16,
    height: u16,
    fps: u8,
) -> bool {
    let Some(profile) = profile_from_ffi(codec, width, height, fps) else {
        return false;
    };
    transport_client_slot()
        .lock()
        .ok()
        .and_then(|slot| {
            slot.as_ref().map(|client| {
                client
                    .connection
                    .peer_handshake()
                    .supported_profiles
                    .contains(&profile)
            })
        })
        .unwrap_or(false)
}

/// Queue a capability update and commit it locally only after enqueue succeeds.
///
/// # Safety
///
/// `profiles_json` must be a valid NUL-terminated UTF-8 JSON array.
#[no_mangle]
pub unsafe extern "C" fn phonecam_update_video_capabilities(profiles_json: *const c_char) -> bool {
    if profiles_json.is_null() {
        return false;
    }
    let json = match unsafe { CStr::from_ptr(profiles_json) }.to_str() {
        Ok(json) => json,
        Err(_) => return false,
    };
    let profiles: Vec<StreamProfile> = match serde_json::from_str(json) {
        Ok(profiles) => profiles,
        Err(_) => return false,
    };
    let mut slot = match transport_client_slot().lock() {
        Ok(slot) => slot,
        Err(_) => return false,
    };
    let Some(client) = slot.as_mut() else {
        return false;
    };
    let update = VideoCapabilitiesUpdate {
        supported_profiles: profiles.clone(),
    };
    if update.validate(client.active_profile).is_err() {
        return false;
    }
    if !try_enqueue(
        client.connection.sender(),
        Message::VideoCapabilitiesUpdate(update),
    ) {
        return false;
    }
    client.supported_profiles = profiles;
    true
}

#[no_mangle]
pub extern "C" fn phonecam_report_stream_configuration(
    request_id: u32,
    result_code: u8,
    codec: u8,
    width: u16,
    height: u16,
    fps: u8,
) -> bool {
    let mut slot = match transport_client_slot().lock() {
        Ok(slot) => slot,
        Err(_) => return false,
    };
    let Some(client) = slot.as_mut() else {
        return false;
    };
    let pending = client.pending_configuration;
    if request_id != 0 && pending.map(|(id, _)| id) != Some(request_id) {
        return false;
    }

    let applied_profile = if result_code == 0 {
        let Some(profile) = profile_from_ffi(codec, width, height, fps) else {
            return false;
        };
        if !client.supported_profiles.contains(&profile)
            || !client
                .connection
                .peer_handshake()
                .supported_profiles
                .contains(&profile)
        {
            return false;
        }
        Some(profile)
    } else {
        None
    };
    let result = match result_code {
        0 => StreamConfigurationOutcome::Applied(applied_profile.expect("validated profile")),
        1 => StreamConfigurationOutcome::UnsupportedProfile,
        2 => StreamConfigurationOutcome::CaptureConfigurationFailed,
        3 => StreamConfigurationOutcome::EncoderInitializationFailed,
        _ => return false,
    };
    let message =
        Message::StreamConfigurationResult(StreamConfigurationResult { request_id, result });
    if !try_enqueue(client.connection.sender(), message) {
        return false;
    }
    if let Some(profile) = applied_profile {
        client.active_profile = profile;
    }
    if request_id != 0 {
        client.pending_configuration = None;
    }
    true
}

/// Parse a QR code URI and return the parsed result as a C string.
///
/// Returns a pointer to an allocated C string in the format "ip|port|name".
/// The caller is responsible for freeing the returned pointer using `phonecam_string_free`.
///
/// # Safety
///
/// - `uri` must be a valid, non-null pointer to a null-terminated UTF-8 string.
/// - The string pointed to by `uri` must remain valid for the duration of this call.
#[no_mangle]
pub unsafe extern "C" fn phonecam_parse_qr_code_uri(uri: *const c_char) -> *mut c_char {
    if uri.is_null() {
        return std::ptr::null_mut();
    }

    let uri = match unsafe { CStr::from_ptr(uri) }.to_str() {
        Ok(uri) => uri,
        Err(_) => return std::ptr::null_mut(),
    };

    let parsed = match parse_qr_code_uri(uri) {
        Ok(parsed) => parsed,
        Err(_) => return std::ptr::null_mut(),
    };

    CString::new(format!("{}|{}|{}", parsed.ip, parsed.port, parsed.name))
        .map(CString::into_raw)
        .unwrap_or(std::ptr::null_mut())
}

/// Discover PhoneCam desktop services over mDNS and return newline-delimited records.
///
/// Each record uses `name|ip|port|version`. The returned string must be released
/// with [`phonecam_string_free`]. A null pointer indicates discovery failure.
#[no_mangle]
pub extern "C" fn phonecam_discover_desktops(timeout_ms: u32) -> *mut c_char {
    let timeout = Duration::from_millis(u64::from(timeout_ms.clamp(100, 5_000)));
    let runtime = match Builder::new_current_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(_) => return std::ptr::null_mut(),
    };
    let browser = match ServiceBrowser::new() {
        Ok(browser) => browser,
        Err(_) => return std::ptr::null_mut(),
    };
    let services = match runtime.block_on(browser.discover(timeout)) {
        Ok(services) => services,
        Err(_) => return std::ptr::null_mut(),
    };

    CString::new(format_discovered_services(&services))
        .map(CString::into_raw)
        .unwrap_or(std::ptr::null_mut())
}

fn format_discovered_services(services: &[DiscoveredService]) -> String {
    services
        .iter()
        .map(|service| {
            let name = service.name.replace(['|', '\n', '\r'], " ");
            let version = service.version.replace(['|', '\n', '\r'], " ");
            format!("{name}|{}|{}|{version}", service.ip, service.port)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[no_mangle]
pub extern "C" fn phonecam_ffi_test_message() -> *mut c_char {
    CString::new(ffi_test_message())
        .expect("message string must not contain interior NULs")
        .into_raw()
}

/// Free a string pointer that was returned by a PhoneCam FFI function.
///
/// # Safety
///
/// - `ptr` must be a pointer returned by a PhoneCam FFI function that returns `*mut c_char`.
/// - `ptr` must not have been freed before.
/// - After calling this function, `ptr` becomes invalid and must not be used again.
#[no_mangle]
pub unsafe extern "C" fn phonecam_string_free(ptr: *mut c_char) {
    if ptr.is_null() {
        return;
    }

    // SAFETY: The pointer must come from `CString::into_raw` in this crate and be freed once.
    unsafe {
        drop(CString::from_raw(ptr));
    }
}

#[cfg(test)]
fn latest_frame_metadata() -> Option<FrameMetadata> {
    last_frame_slot().lock().ok().and_then(|slot| *slot)
}

#[cfg(test)]
fn current_config() -> CoreConfig {
    core_config_slot()
        .lock()
        .map(|config| config.clone())
        .unwrap_or_default()
}

uniffi::include_scaffolding!("phonecam");

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::{CStr, CString};
    use std::net::{IpAddr, Ipv4Addr};
    use std::sync::{LazyLock, Mutex};

    static TEST_MUTEX: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

    fn test_lock() -> std::sync::MutexGuard<'static, ()> {
        TEST_MUTEX
            .lock()
            .expect("test mutex should not be poisoned")
    }

    fn hevc_4k60() -> StreamProfile {
        StreamProfile {
            codec: VideoCodec::Hevc,
            width: 3840,
            height: 2160,
            fps: 60,
        }
    }

    #[test]
    fn video_config_json_requires_exact_valid_profiles() {
        let valid = r#"{"active_profile":{"codec":"h264","width":1280,"height":720,"fps":30},"supported_profiles":[{"codec":"h264","width":1280,"height":720,"fps":30},{"codec":"hevc","width":3840,"height":2160,"fps":60}]}"#;
        let config: VideoConfig = serde_json::from_str(valid).unwrap();
        assert!(config.validate());

        for invalid in [
            r#"{"active_profile":{"codec":"h264","width":1280,"height":720,"fps":30},"supported_profiles":[]}"#,
            r#"{"active_profile":{"codec":"h264","width":1280,"height":720,"fps":30},"supported_profiles":[{"codec":"hevc","width":3840,"height":2160,"fps":60}]}"#,
            r#"{"active_profile":{"codec":"av1","width":1280,"height":720,"fps":30},"supported_profiles":[]}"#,
            r#"{"active_profile":{"codec":"h264","width":1024,"height":768,"fps":30},"supported_profiles":[{"codec":"h264","width":1024,"height":768,"fps":30}]}"#,
            r#"{"active_profile":{"codec":"h264","width":1280,"height":720,"fps":30},"supported_profiles":[{"codec":"h264","width":1280,"height":720,"fps":30}],"extra":true}"#,
        ] {
            assert!(!serde_json::from_str::<VideoConfig>(invalid)
                .map(|config| config.validate())
                .unwrap_or(false));
        }
    }

    #[test]
    fn malformed_init_does_not_change_endpoint_or_open_transport() {
        let _lock = test_lock();
        shutdown_transport_client();
        configure_endpoint("127.0.0.1".to_owned(), 7878);
        let host = CString::new("203.0.113.1").unwrap();
        let invalid = CString::new(r#"{"supported_profiles":[]}"#).unwrap();
        assert!(!unsafe { phonecam_transport_init(host.as_ptr(), 9999, invalid.as_ptr()) });
        assert_eq!(current_endpoint(), "127.0.0.1:7878");
    }

    #[test]
    fn control_json_has_typed_profiles_and_request_id_zero() {
        let cases = [
            (
                CameraControl::SwitchCamera { front: true },
                serde_json::json!({"type":"switch_camera","front":true}),
            ),
            (
                CameraControl::RequestKeyframe,
                serde_json::json!({"type":"request_keyframe"}),
            ),
            (
                CameraControl::ConfigureStream {
                    request_id: 0,
                    profile: hevc_4k60(),
                },
                serde_json::json!({
                    "type":"configure_stream",
                    "request_id":0,
                    "profile":{"codec":"hevc","width":3840,"height":2160,"fps":60}
                }),
            ),
        ];
        for (control, expected) in cases {
            let json = control_command_json(&control).unwrap();
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(&json).unwrap(),
                expected
            );
        }
    }

    #[test]
    fn profile_queries_reject_unknown_codec_and_disconnected_peer() {
        let _lock = test_lock();
        shutdown_transport_client();
        assert!(profile_from_ffi(0, 1280, 720, 30).is_some());
        assert!(profile_from_ffi(1, 3840, 2160, 60).is_some());
        assert!(profile_from_ffi(2, 1280, 720, 30).is_none());
        assert!(!phonecam_peer_supports_profile(0, 1280, 720, 30));
    }

    #[test]
    fn queue_full_never_commits_followup_state() {
        let (sender, _receiver) = tokio::sync::mpsc::channel(1);
        let message = Message::StatusUpdate(phonecam_protocol::StatusUpdate {
            status: "fill".to_owned(),
        });
        assert!(try_enqueue(&sender, message));

        let old_profiles = vec![StreamProfile::H264_720P30];
        let new_profiles = vec![StreamProfile::H264_720P30, hevc_4k60()];
        let mut committed = old_profiles.clone();
        if try_enqueue(
            &sender,
            Message::VideoCapabilitiesUpdate(VideoCapabilitiesUpdate {
                supported_profiles: new_profiles.clone(),
            }),
        ) {
            committed = new_profiles;
        }
        assert_eq!(committed, old_profiles);
    }

    #[test]
    fn returned_json_string_uses_shared_free_function() {
        let ptr = CString::new(control_command_json(&CameraControl::RequestKeyframe).unwrap())
            .unwrap()
            .into_raw();
        assert_eq!(
            unsafe { CStr::from_ptr(ptr) }.to_str().unwrap(),
            r#"{"type":"request_keyframe"}"#
        );
        unsafe { phonecam_string_free(ptr) };
    }

    #[test]
    fn parse_qr_and_discovery_formats_remain_stable() {
        let uri = CString::new("phonecam://192.168.0.42:7878?name=Desktop").unwrap();
        let ptr = unsafe { phonecam_parse_qr_code_uri(uri.as_ptr()) };
        assert!(!ptr.is_null());
        assert_eq!(
            unsafe { CStr::from_ptr(ptr) }.to_str().unwrap(),
            "192.168.0.42|7878|Desktop"
        );
        unsafe { phonecam_string_free(ptr) };

        let services = vec![DiscoveredService {
            name: "Office|Desktop".to_owned(),
            ip: IpAddr::V4(Ipv4Addr::new(192, 168, 1, 20)),
            port: 7878,
            version: "0.1.0".to_owned(),
        }];
        assert_eq!(
            format_discovered_services(&services),
            "Office Desktop|192.168.1.20|7878|0.1.0"
        );
    }
}
