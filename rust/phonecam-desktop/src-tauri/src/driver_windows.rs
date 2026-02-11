#[cfg(target_os = "windows")]
use std::{
    ffi::c_void,
    io::{self, ErrorKind},
    ptr::null_mut,
};

#[cfg(target_os = "windows")]
const PIPE_NAME: &str = r"\\.\pipe\PhoneCam";
#[cfg(target_os = "windows")]
const FRAME_HEADER_SIZE: usize = 16;
#[cfg(target_os = "windows")]
const PIPE_WAIT_TIMEOUT_MS: u32 = 1_000;

#[cfg(target_os = "windows")]
const GENERIC_WRITE: u32 = 0x4000_0000;
#[cfg(target_os = "windows")]
const FILE_SHARE_READ: u32 = 0x0000_0001;
#[cfg(target_os = "windows")]
const FILE_SHARE_WRITE: u32 = 0x0000_0002;
#[cfg(target_os = "windows")]
const OPEN_EXISTING: u32 = 3;
#[cfg(target_os = "windows")]
const FILE_ATTRIBUTE_NORMAL: u32 = 0x0000_0080;

#[cfg(target_os = "windows")]
const ERROR_FILE_NOT_FOUND: i32 = 2;
#[cfg(target_os = "windows")]
const ERROR_PATH_NOT_FOUND: i32 = 3;
#[cfg(target_os = "windows")]
const ERROR_SEM_TIMEOUT: i32 = 121;
#[cfg(target_os = "windows")]
const ERROR_PIPE_BUSY: i32 = 231;

#[cfg(target_os = "windows")]
type Handle = *mut c_void;

#[cfg(target_os = "windows")]
const INVALID_HANDLE_VALUE: Handle = -1_isize as Handle;

#[cfg(target_os = "windows")]
pub struct WindowsDriver {
    pipe: Option<PipeHandle>,
}

#[cfg(target_os = "windows")]
impl WindowsDriver {
    pub fn new() -> Self {
        Self { pipe: None }
    }

    pub fn connect(&mut self) -> io::Result<()> {
        if self.pipe.is_some() {
            return Ok(());
        }

        let pipe_name = windows_wide_string(PIPE_NAME);
        self.pipe = Some(connect_named_pipe(&pipe_name)?);

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

        let pipe = self.pipe.as_ref().ok_or_else(|| {
            io::Error::new(
                ErrorKind::NotConnected,
                "DirectShow named pipe is not connected",
            )
        })?;

        let mut header = [0u8; FRAME_HEADER_SIZE];
        header[0..4].copy_from_slice(&width.to_le_bytes());
        header[4..8].copy_from_slice(&height.to_le_bytes());
        header[8..16].copy_from_slice(&timestamp_ns.to_le_bytes());

        write_all_to_pipe(pipe, &header)?;
        write_all_to_pipe(pipe, nv12_data)?;

        Ok(())
    }

    pub fn disconnect(&mut self) {
        self.pipe = None;
    }
}

#[cfg(target_os = "windows")]
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

#[cfg(target_os = "windows")]
fn windows_wide_string(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(target_os = "windows")]
fn connect_named_pipe(pipe_name: &[u16]) -> io::Result<PipeHandle> {
    let mut handle = unsafe {
        CreateFileW(
            pipe_name.as_ptr(),
            GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            null_mut(),
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            null_mut(),
        )
    };

    if handle != INVALID_HANDLE_VALUE {
        return Ok(PipeHandle { handle });
    }

    let connect_error = io::Error::last_os_error();
    if connect_error.raw_os_error() != Some(ERROR_PIPE_BUSY) {
        return Err(map_connect_error(connect_error));
    }

    let wait_result = unsafe { WaitNamedPipeW(pipe_name.as_ptr(), PIPE_WAIT_TIMEOUT_MS) };
    if wait_result == 0 {
        let wait_error = io::Error::last_os_error();
        if wait_error.raw_os_error() == Some(ERROR_SEM_TIMEOUT) {
            return Err(io::Error::new(
                ErrorKind::WouldBlock,
                format!(
                    "named pipe {PIPE_NAME} is busy and did not become available within {PIPE_WAIT_TIMEOUT_MS}ms"
                ),
            ));
        }

        return Err(map_connect_error(wait_error));
    }

    handle = unsafe {
        CreateFileW(
            pipe_name.as_ptr(),
            GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            null_mut(),
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            null_mut(),
        )
    };

    if handle == INVALID_HANDLE_VALUE {
        return Err(map_connect_error(io::Error::last_os_error()));
    }

    Ok(PipeHandle { handle })
}

#[cfg(target_os = "windows")]
fn map_connect_error(source: io::Error) -> io::Error {
    match source.raw_os_error() {
        Some(ERROR_FILE_NOT_FOUND) | Some(ERROR_PATH_NOT_FOUND) => io::Error::new(
            ErrorKind::NotFound,
            format!(
                "named pipe {PIPE_NAME} is unavailable (is the DirectShow filter running?): {source}"
            ),
        ),
        Some(ERROR_PIPE_BUSY) => io::Error::new(
            ErrorKind::WouldBlock,
            format!("named pipe {PIPE_NAME} is currently busy: {source}"),
        ),
        _ => io::Error::new(
            source.kind(),
            format!("failed to connect to named pipe {PIPE_NAME}: {source}"),
        ),
    }
}

#[cfg(target_os = "windows")]
fn write_all_to_pipe(pipe: &PipeHandle, mut bytes: &[u8]) -> io::Result<()> {
    while !bytes.is_empty() {
        let chunk_len = bytes.len().min(u32::MAX as usize) as u32;
        let mut bytes_written = 0u32;

        let write_result = unsafe {
            WriteFile(
                pipe.handle,
                bytes.as_ptr().cast(),
                chunk_len,
                &mut bytes_written,
                null_mut(),
            )
        };

        if write_result == 0 {
            let write_error = io::Error::last_os_error();
            return Err(io::Error::new(
                write_error.kind(),
                format!("failed writing to named pipe {PIPE_NAME}: {write_error}"),
            ));
        }

        if bytes_written == 0 {
            return Err(io::Error::new(
                ErrorKind::WriteZero,
                format!("named pipe {PIPE_NAME} returned zero-byte write"),
            ));
        }

        bytes = &bytes[bytes_written as usize..];
    }

    Ok(())
}

#[cfg(target_os = "windows")]
struct PipeHandle {
    handle: Handle,
}

#[cfg(target_os = "windows")]
impl Drop for PipeHandle {
    fn drop(&mut self) {
        if self.handle == INVALID_HANDLE_VALUE || self.handle.is_null() {
            return;
        }

        unsafe {
            let _ = CloseHandle(self.handle);
        }
    }
}

#[cfg(target_os = "windows")]
#[link(name = "kernel32")]
extern "system" {
    fn CreateFileW(
        file_name: *const u16,
        desired_access: u32,
        share_mode: u32,
        security_attributes: *mut c_void,
        creation_disposition: u32,
        flags_and_attributes: u32,
        template_file: Handle,
    ) -> Handle;
    fn WaitNamedPipeW(named_pipe_name: *const u16, timeout_ms: u32) -> i32;
    fn WriteFile(
        file: Handle,
        buffer: *const c_void,
        bytes_to_write: u32,
        bytes_written: *mut u32,
        overlapped: *mut c_void,
    ) -> i32;
    fn CloseHandle(object: Handle) -> i32;
}
