#[cfg(target_os = "macos")]
use std::{
    ffi::{c_char, c_void, CStr, CString},
    io::{self, ErrorKind, Write},
    net::Shutdown,
    os::unix::net::UnixStream,
    path::PathBuf,
};

#[cfg(target_os = "macos")]
const APP_GROUP_IDENTIFIER: &str = "group.com.phonecam.shared";
#[cfg(target_os = "macos")]
const SOCKET_FILE_NAME: &str = "phonecam.sock";
#[cfg(target_os = "macos")]
const FRAME_HEADER_SIZE: usize = 16;

#[cfg(target_os = "macos")]
pub struct MacOSDriver {
    stream: Option<UnixStream>,
}

#[cfg(target_os = "macos")]
impl MacOSDriver {
    pub fn new() -> Self {
        Self { stream: None }
    }

    pub fn connect(&mut self) -> io::Result<()> {
        if self.stream.is_some() {
            return Ok(());
        }

        let socket_path = app_group_socket_path(APP_GROUP_IDENTIFIER)?;
        let stream = UnixStream::connect(&socket_path).map_err(|err| {
            io::Error::new(
                err.kind(),
                format!(
                    "failed to connect to CMIO extension socket at {}: {err}",
                    socket_path.display()
                ),
            )
        })?;

        self.stream = Some(stream);

        Ok(())
    }

    pub fn write_frame(
        &mut self,
        width: u32,
        height: u32,
        timestamp_ns: u64,
        nv12_data: &[u8],
    ) -> io::Result<()> {
        let expected_payload_len = expected_nv12_payload_len(width, height)?;
        if nv12_data.len() != expected_payload_len {
            return Err(io::Error::new(
                ErrorKind::InvalidInput,
                format!(
                    "invalid NV12 payload size for {width}x{height}: expected {expected_payload_len} bytes, got {}",
                    nv12_data.len()
                ),
            ));
        }

        let stream = self.stream.as_mut().ok_or_else(|| {
            io::Error::new(
                ErrorKind::NotConnected,
                "CMIO extension IPC socket is not connected",
            )
        })?;

        let mut header = [0u8; FRAME_HEADER_SIZE];
        header[0..4].copy_from_slice(&width.to_le_bytes());
        header[4..8].copy_from_slice(&height.to_le_bytes());
        header[8..16].copy_from_slice(&timestamp_ns.to_le_bytes());

        stream.write_all(&header)?;
        stream.write_all(nv12_data)?;

        Ok(())
    }

    pub fn disconnect(&mut self) {
        if let Some(stream) = self.stream.take() {
            let _ = stream.shutdown(Shutdown::Both);
        }
    }
}

#[cfg(target_os = "macos")]
fn expected_nv12_payload_len(width: u32, height: u32) -> io::Result<usize> {
    if width == 0 || height == 0 {
        return Err(io::Error::new(
            ErrorKind::InvalidInput,
            "frame dimensions must be greater than zero",
        ));
    }

    if width % 2 != 0 || height % 2 != 0 {
        return Err(io::Error::new(
            ErrorKind::InvalidInput,
            "NV12 frame dimensions must be even",
        ));
    }

    let pixels = (width as usize)
        .checked_mul(height as usize)
        .ok_or_else(|| io::Error::new(ErrorKind::InvalidInput, "frame dimensions overflow"))?;

    let chroma = pixels / 2;
    pixels
        .checked_add(chroma)
        .ok_or_else(|| io::Error::new(ErrorKind::InvalidInput, "NV12 payload size overflow"))
}

#[cfg(target_os = "macos")]
fn app_group_socket_path(app_group_identifier: &str) -> io::Result<PathBuf> {
    Ok(app_group_container_path(app_group_identifier)?.join(SOCKET_FILE_NAME))
}

#[cfg(target_os = "macos")]
fn app_group_container_path(app_group_identifier: &str) -> io::Result<PathBuf> {
    let effective_user_id = unsafe { libc::geteuid() };
    let _pool = unsafe { AutoreleasePool::new() };

    let ns_string_class = get_objc_class(b"NSString\0")?;
    let ns_file_manager_class = get_objc_class(b"NSFileManager\0")?;

    let app_group_cstring = CString::new(app_group_identifier).map_err(|_| {
        io::Error::new(
            ErrorKind::InvalidInput,
            "app group identifier contains interior NUL byte",
        )
    })?;

    let string_with_utf8_selector = get_selector(b"stringWithUTF8String:\0")?;
    let app_group_ns_string = unsafe {
        objc_msg_send_id_with_cstr(
            ns_string_class,
            string_with_utf8_selector,
            app_group_cstring.as_ptr(),
        )
    };
    if app_group_ns_string.is_null() {
        return Err(io::Error::new(
            ErrorKind::Other,
            "failed to allocate NSString for app group identifier",
        ));
    }

    let default_manager_selector = get_selector(b"defaultManager\0")?;
    let file_manager = unsafe { objc_msg_send_id(ns_file_manager_class, default_manager_selector) };
    if file_manager.is_null() {
        return Err(io::Error::new(
            ErrorKind::Other,
            "NSFileManager.defaultManager returned nil",
        ));
    }

    let container_selector = get_selector(b"containerURLForSecurityApplicationGroupIdentifier:\0")?;
    let container_url =
        unsafe { objc_msg_send_id_with_id(file_manager, container_selector, app_group_ns_string) };

    if container_url.is_null() {
        return Err(io::Error::new(
            ErrorKind::NotFound,
            format!(
                "App Group container unavailable for '{}' (euid={effective_user_id})",
                app_group_identifier
            ),
        ));
    }

    let path_selector = get_selector(b"path\0")?;
    let path_ns_string = unsafe { objc_msg_send_id(container_url, path_selector) };
    if path_ns_string.is_null() {
        return Err(io::Error::new(
            ErrorKind::Other,
            "container URL path is nil",
        ));
    }

    let utf8_string_selector = get_selector(b"UTF8String\0")?;
    let utf8_path_pointer = unsafe { objc_msg_send_cstr(path_ns_string, utf8_string_selector) };
    if utf8_path_pointer.is_null() {
        return Err(io::Error::new(
            ErrorKind::Other,
            "container URL path UTF8String is nil",
        ));
    }

    let path = unsafe { CStr::from_ptr(utf8_path_pointer) }
        .to_str()
        .map_err(|err| {
            io::Error::new(
                ErrorKind::InvalidData,
                format!("container URL path is not valid UTF-8: {err}"),
            )
        })?
        .to_string();

    Ok(PathBuf::from(path))
}

#[cfg(target_os = "macos")]
fn get_objc_class(class_name: &'static [u8]) -> io::Result<*mut c_void> {
    let class = unsafe { objc_getClass(class_name.as_ptr().cast()) };
    if class.is_null() {
        return Err(io::Error::new(
            ErrorKind::Other,
            format!(
                "Objective-C class '{}' not found",
                String::from_utf8_lossy(&class_name[..class_name.len().saturating_sub(1)])
            ),
        ));
    }

    Ok(class)
}

#[cfg(target_os = "macos")]
fn get_selector(selector_name: &'static [u8]) -> io::Result<*mut c_void> {
    let selector = unsafe { sel_registerName(selector_name.as_ptr().cast()) };
    if selector.is_null() {
        return Err(io::Error::new(
            ErrorKind::Other,
            format!(
                "Objective-C selector '{}' not found",
                String::from_utf8_lossy(&selector_name[..selector_name.len().saturating_sub(1)])
            ),
        ));
    }

    Ok(selector)
}

#[cfg(target_os = "macos")]
struct AutoreleasePool(*mut c_void);

#[cfg(target_os = "macos")]
impl AutoreleasePool {
    unsafe fn new() -> Self {
        Self(objc_autoreleasePoolPush())
    }
}

#[cfg(target_os = "macos")]
impl Drop for AutoreleasePool {
    fn drop(&mut self) {
        unsafe {
            objc_autoreleasePoolPop(self.0);
        }
    }
}

#[cfg(target_os = "macos")]
unsafe fn objc_msg_send_id(receiver: *mut c_void, selector: *mut c_void) -> *mut c_void {
    let send: extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
        std::mem::transmute(objc_msgSend as *const ());
    send(receiver, selector)
}

#[cfg(target_os = "macos")]
unsafe fn objc_msg_send_id_with_id(
    receiver: *mut c_void,
    selector: *mut c_void,
    arg: *mut c_void,
) -> *mut c_void {
    let send: extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> *mut c_void =
        std::mem::transmute(objc_msgSend as *const ());
    send(receiver, selector, arg)
}

#[cfg(target_os = "macos")]
unsafe fn objc_msg_send_id_with_cstr(
    receiver: *mut c_void,
    selector: *mut c_void,
    arg: *const c_char,
) -> *mut c_void {
    let send: extern "C" fn(*mut c_void, *mut c_void, *const c_char) -> *mut c_void =
        std::mem::transmute(objc_msgSend as *const ());
    send(receiver, selector, arg)
}

#[cfg(target_os = "macos")]
unsafe fn objc_msg_send_cstr(receiver: *mut c_void, selector: *mut c_void) -> *const c_char {
    let send: extern "C" fn(*mut c_void, *mut c_void) -> *const c_char =
        std::mem::transmute(objc_msgSend as *const ());
    send(receiver, selector)
}

#[cfg(target_os = "macos")]
#[link(name = "Foundation", kind = "framework")]
extern "C" {}

#[cfg(target_os = "macos")]
#[link(name = "objc")]
extern "C" {
    fn objc_getClass(name: *const c_char) -> *mut c_void;
    fn sel_registerName(name: *const c_char) -> *mut c_void;
    fn objc_msgSend();
    fn objc_autoreleasePoolPush() -> *mut c_void;
    fn objc_autoreleasePoolPop(pool: *mut c_void);
}
