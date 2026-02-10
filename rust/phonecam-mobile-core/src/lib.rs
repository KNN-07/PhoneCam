#![allow(dead_code)]

use std::ffi::CString;
use std::os::raw::c_char;
use std::sync::{Mutex, OnceLock};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FrameMetadata {
    len: usize,
    pts: u64,
    is_keyframe: bool,
}

static LAST_FRAME: OnceLock<Mutex<Option<FrameMetadata>>> = OnceLock::new();

fn last_frame_slot() -> &'static Mutex<Option<FrameMetadata>> {
    LAST_FRAME.get_or_init(|| Mutex::new(None))
}

pub fn ffi_test_message() -> String {
    "phonecam-mobile-core ffi ok".to_string()
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

    if let Ok(mut slot) = last_frame_slot().lock() {
        *slot = Some(FrameMetadata {
            len,
            pts,
            is_keyframe,
        });
    }
}

#[no_mangle]
pub extern "C" fn phonecam_ffi_test_message() -> *mut c_char {
    CString::new(ffi_test_message())
        .expect("message string must not contain interior NULs")
        .into_raw()
}

/// Frees a C string allocated by `phonecam_ffi_test_message`.
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

uniffi::include_scaffolding!("phonecam");

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CStr;

    #[test]
    fn ffi_test_message_is_available() {
        assert_eq!(ffi_test_message(), "phonecam-mobile-core ffi ok");
    }

    #[test]
    fn send_video_frame_records_metadata() {
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
}
