#![allow(dead_code)]

use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::sync::{Mutex, OnceLock};

use phonecam_discovery::parse_qr_code_uri;
use phonecam_protocol::{Message, VideoFrame};
use phonecam_transport::{ConnectionState, PhoneCamClient, TransportConnection};
use tokio::runtime::{Builder, Runtime};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FrameMetadata {
    len: usize,
    pts: u64,
    is_keyframe: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CoreConfig {
    endpoint_host: String,
    endpoint_port: u16,
    streaming_enabled: bool,
    video_width: u16,
    video_height: u16,
}

impl Default for CoreConfig {
    fn default() -> Self {
        Self {
            endpoint_host: "127.0.0.1".to_string(),
            endpoint_port: 7878,
            streaming_enabled: false,
            video_width: 1280,
            video_height: 720,
        }
    }
}

struct TransportClient {
    runtime: Runtime,
    connection: TransportConnection,
}

static LAST_FRAME: OnceLock<Mutex<Option<FrameMetadata>>> = OnceLock::new();
static CORE_CONFIG: OnceLock<Mutex<CoreConfig>> = OnceLock::new();
static TRANSPORT_CLIENT: OnceLock<Mutex<Option<TransportClient>>> = OnceLock::new();

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

pub fn set_video_resolution(width: u16, height: u16) {
    if width == 0 || height == 0 {
        return;
    }

    if let Ok(mut config) = core_config_slot().lock() {
        config.video_width = width;
        config.video_height = height;
    }
}

fn video_resolution() -> (u16, u16) {
    if let Ok(config) = core_config_slot().lock() {
        return (config.video_width, config.video_height);
    }

    (
        CoreConfig::default().video_width,
        CoreConfig::default().video_height,
    )
}

pub fn is_streaming_enabled() -> bool {
    if let Ok(config) = core_config_slot().lock() {
        return config.streaming_enabled;
    }

    false
}

fn initialize_transport_client(host: String, port: u16) -> bool {
    if host.trim().is_empty() {
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

    let connection = match runtime.block_on(PhoneCamClient::connect(endpoint.clone())) {
        Ok(connection) => connection,
        Err(_) => return false,
    };

    if let Ok(mut slot) = transport_client_slot().lock() {
        *slot = Some(TransportClient {
            runtime,
            connection,
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

/// Raw C FFI entry point for high-frequency H.264 frame payload submission.
///
/// Android and iOS share this exact ABI for performance-sensitive frame ingress.
#[no_mangle]
pub unsafe extern "C" fn phonecam_send_video_frame(
    data: *const u8,
    len: usize,
    pts: u64,
    is_keyframe: bool,
) {
    if data.is_null() || len == 0 {
        return;
    }

    // SAFETY: The caller provides a non-null pointer and length for the duration of this call.
    let _payload = unsafe { std::slice::from_raw_parts(data, len) };

    if let Ok(mut slot) = transport_client_slot().lock() {
        if let Some(client) = slot.as_mut() {
            let (width, height) = video_resolution();
            let message = Message::VideoFrame(VideoFrame {
                nal_unit: _payload.to_vec().into(),
                pts_us: pts,
                width,
                height,
                is_keyframe,
            });

            match client.connection.sender().try_send(message) {
                Ok(()) => {}
                Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                    *slot = None;
                }
                Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {}
            }
        }
    }

    if let Ok(mut slot) = last_frame_slot().lock() {
        *slot = Some(FrameMetadata {
            len,
            pts,
            is_keyframe,
        });
    }
}

#[no_mangle]
pub unsafe extern "C" fn phonecam_transport_init(host: *const c_char, port: u16) -> bool {
    if host.is_null() {
        return false;
    }

    if port == 0 {
        return false;
    }

    let host = match unsafe { CStr::from_ptr(host) }.to_str() {
        Ok(host) => host.trim(),
        Err(_) => return false,
    };

    if host.is_empty() {
        return false;
    }

    configure_endpoint(host.to_string(), port);
    let initialized = initialize_transport_client(host.to_string(), port);
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

#[no_mangle]
pub extern "C" fn phonecam_set_video_resolution(width: u16, height: u16) {
    set_video_resolution(width, height);
}

#[no_mangle]
pub extern "C" fn phonecam_set_video_dimensions(width: u16, height: u16) {
    phonecam_set_video_resolution(width, height);
}

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

#[no_mangle]
pub extern "C" fn phonecam_ffi_test_message() -> *mut c_char {
    CString::new(ffi_test_message())
        .expect("message string must not contain interior NULs")
        .into_raw()
}

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
    use std::sync::{Mutex, OnceLock};

    static TEST_MUTEX: OnceLock<Mutex<()>> = OnceLock::new();

    fn test_lock() -> std::sync::MutexGuard<'static, ()> {
        TEST_MUTEX
            .get_or_init(|| Mutex::new(()))
            .lock()
            .expect("test mutex should not be poisoned")
    }

    #[test]
    fn ffi_test_message_is_available() {
        let _lock = test_lock();
        assert_eq!(ffi_test_message(), "phonecam-mobile-core ffi ok");
    }

    #[test]
    fn send_video_frame_records_metadata() {
        let _lock = test_lock();
        let sample = [0x00_u8, 0x00, 0x01, 0x65];

        // SAFETY: Valid pointer/length pair from an in-scope byte array.
        unsafe {
            phonecam_send_video_frame(sample.as_ptr(), sample.len(), 42, true);
        }

        let metadata = latest_frame_metadata().expect("expected metadata to be recorded");
        assert_eq!(metadata.len, sample.len());
        assert_eq!(metadata.pts, 42);
        assert!(metadata.is_keyframe);
    }

    #[test]
    fn c_string_message_roundtrip_and_free() {
        let _lock = test_lock();
        let ptr = phonecam_ffi_test_message();
        assert!(!ptr.is_null());

        // SAFETY: Pointer is expected to refer to a valid NUL-terminated C string.
        let msg = unsafe { CStr::from_ptr(ptr) }
            .to_str()
            .expect("ffi test string must be valid UTF-8")
            .to_owned();

        assert_eq!(msg, "phonecam-mobile-core ffi ok");

        // SAFETY: Pointer was returned by `phonecam_ffi_test_message` and is freed once here.
        unsafe {
            phonecam_string_free(ptr);
        }
    }

    #[test]
    fn endpoint_configuration_roundtrip() {
        let _lock = test_lock();

        configure_endpoint("192.168.1.25".to_string(), 9000);

        assert_eq!(current_endpoint(), "192.168.1.25:9000");

        let config = current_config();
        assert_eq!(config.endpoint_host, "192.168.1.25");
        assert_eq!(config.endpoint_port, 9000);
    }

    #[test]
    fn streaming_enable_flag_can_toggle() {
        let _lock = test_lock();

        set_streaming_enabled(true);
        assert!(is_streaming_enabled());

        set_streaming_enabled(false);
        assert!(!is_streaming_enabled());
    }

    #[test]
    fn video_resolution_roundtrip_updates_config() {
        let _lock = test_lock();

        set_video_resolution(1920, 1080);

        let config = current_config();
        assert_eq!(config.video_width, 1920);
        assert_eq!(config.video_height, 1080);
    }

    #[test]
    fn parse_qr_uri_roundtrip_through_c_ffi() {
        let _lock = test_lock();
        let uri = CString::new("phonecam://192.168.0.42:7878?name=Desktop").unwrap();

        let ptr = unsafe { phonecam_parse_qr_code_uri(uri.as_ptr()) };
        assert!(!ptr.is_null(), "expected valid pointer for valid QR URI");

        let payload = unsafe { CStr::from_ptr(ptr) }
            .to_str()
            .expect("parsed QR payload must be valid UTF-8")
            .to_string();

        assert_eq!(payload, "192.168.0.42|7878|Desktop");

        unsafe {
            phonecam_string_free(ptr);
        }
    }

    #[test]
    fn parse_qr_uri_rejects_invalid_payload() {
        let _lock = test_lock();
        let uri = CString::new("not-a-phonecam-uri").unwrap();

        let ptr = unsafe { phonecam_parse_qr_code_uri(uri.as_ptr()) };
        assert!(ptr.is_null(), "expected null pointer for invalid QR URI");
    }
}
