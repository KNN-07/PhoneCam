use crate::decode::Nv12Frame;

#[cfg(target_os = "linux")]
use std::path::PathBuf;

#[cfg(target_os = "linux")]
use phonecam_driver_linux::{ensure_v4l2loopback_loaded, list_devices, PixelFormat, V4l2Device};

#[cfg(target_os = "linux")]
use crate::convert::Nv12ToYuyvConverter;
#[cfg(target_os = "macos")]
use crate::driver_macos::MacOsDriver;
#[cfg(target_os = "windows")]
use crate::driver_windows::WindowsDriver;

#[cfg(target_os = "linux")]
pub struct OutputDevice {
    device: V4l2Device,
    converter: Option<Nv12ToYuyvConverter>,
    width: u32,
    height: u32,
}

#[cfg(target_os = "linux")]
impl OutputDevice {
    pub fn open() -> Result<Self, String> {
        ensure_v4l2loopback_loaded()
            .map_err(|err| format!("v4l2loopback is unavailable: {err}"))?;

        let device_path =
            preferred_output_device_path().or_else(|| list_devices().into_iter().next());
        let Some(device_path) = device_path else {
            return Err("no V4L2 device found for output".to_string());
        };

        let device = V4l2Device::open(&device_path).map_err(|err| {
            format!(
                "failed to open V4L2 device {}: {err}",
                device_path.display()
            )
        })?;

        Ok(Self {
            device,
            converter: None,
            width: 0,
            height: 0,
        })
    }

    pub fn write_frame(&mut self, frame: &Nv12Frame, _timestamp_ns: u64) -> Result<(), String> {
        if self.converter.is_none() || self.width != frame.width || self.height != frame.height {
            self.device
                .set_format(frame.width, frame.height, PixelFormat::YUYV)
                .map_err(|err| {
                    format!(
                        "failed to configure v4l2 output format {}x{}: {err}",
                        frame.width, frame.height
                    )
                })?;

            self.converter = Some(
                Nv12ToYuyvConverter::new(frame.width, frame.height)
                    .map_err(|err| format!("converter initialization failed: {err:?}"))?,
            );
            self.width = frame.width;
            self.height = frame.height;
        }

        let converter = self
            .converter
            .as_mut()
            .ok_or_else(|| "converter missing after initialization".to_string())?;
        let yuyv_frame = converter
            .convert(frame)
            .map_err(|err| format!("failed to convert NV12 frame to YUYV: {err:?}"))?;

        self.device.write_frame(&yuyv_frame.data).map_err(|err| {
            format!(
                "failed writing frame to {}: {err}",
                self.device.path().display()
            )
        })
    }
}

#[cfg(target_os = "linux")]
fn preferred_output_device_path() -> Option<PathBuf> {
    let value = std::env::var("PHONECAM_V4L2_DEVICE").ok()?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }

    Some(PathBuf::from(trimmed))
}

#[cfg(target_os = "macos")]
pub struct OutputDevice(MacOsDriver);

#[cfg(target_os = "macos")]
impl OutputDevice {
    pub fn open() -> Result<Self, String> {
        let mut driver = MacOsDriver::new();
        driver
            .connect()
            .map_err(|err| format!("failed to connect to the CMIO extension: {err}"))?;
        Ok(Self(driver))
    }

    pub fn write_frame(&mut self, frame: &Nv12Frame, timestamp_ns: u64) -> Result<(), String> {
        let data = packed_nv12(frame)?;
        self.0
            .write_frame(frame.width, frame.height, timestamp_ns, &data)
            .map_err(|err| format!("failed writing frame to the CMIO extension: {err}"))
    }
}

#[cfg(target_os = "windows")]
pub struct OutputDevice(WindowsDriver);

#[cfg(target_os = "windows")]
impl OutputDevice {
    pub fn open() -> Result<Self, String> {
        let mut driver = WindowsDriver::new();
        driver
            .connect()
            .map_err(|err| format!("failed to connect to the DirectShow filter: {err}"))?;
        Ok(Self(driver))
    }

    pub fn write_frame(&mut self, frame: &Nv12Frame, timestamp_ns: u64) -> Result<(), String> {
        let data = packed_nv12(frame)?;
        self.0
            .write_frame(frame.width, frame.height, timestamp_ns, &data)
            .map_err(|err| format!("failed writing frame to the DirectShow filter: {err}"))
    }
}

#[cfg(any(target_os = "macos", target_os = "windows", test))]
fn packed_nv12(frame: &Nv12Frame) -> Result<Vec<u8>, String> {
    let width = frame.width as usize;
    let height = frame.height as usize;
    if width == 0
        || height == 0
        || width % 2 != 0
        || height % 2 != 0
        || frame.y_stride < width
        || frame.uv_stride < width
    {
        return Err("invalid NV12 frame layout for native output driver".to_string());
    }

    let required_y = frame
        .y_stride
        .checked_mul(height)
        .ok_or_else(|| "NV12 Y plane size overflow".to_string())?;
    let required_uv = frame
        .uv_stride
        .checked_mul(height / 2)
        .ok_or_else(|| "NV12 UV plane size overflow".to_string())?;
    if frame.y_plane.len() < required_y || frame.uv_plane.len() < required_uv {
        return Err("NV12 frame planes are shorter than their stride layout".to_string());
    }

    let capacity = width
        .checked_mul(height)
        .and_then(|pixels| pixels.checked_add(pixels / 2))
        .ok_or_else(|| "NV12 output size overflow".to_string())?;
    let mut data = Vec::with_capacity(capacity);
    for row in 0..height {
        let start = row * frame.y_stride;
        data.extend_from_slice(&frame.y_plane[start..start + width]);
    }
    for row in 0..height / 2 {
        let start = row * frame.uv_stride;
        data.extend_from_slice(&frame.uv_plane[start..start + width]);
    }
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::packed_nv12;
    use crate::decode::Nv12Frame;

    #[test]
    fn native_output_packs_strided_nv12_planes() {
        let frame = Nv12Frame {
            width: 2,
            height: 2,
            pts_us: 0,
            y_stride: 4,
            uv_stride: 4,
            y_plane: vec![1, 2, 0, 0, 3, 4, 0, 0],
            uv_plane: vec![5, 6, 0, 0],
        };

        assert_eq!(packed_nv12(&frame).unwrap(), vec![1, 2, 3, 4, 5, 6]);
    }
}
