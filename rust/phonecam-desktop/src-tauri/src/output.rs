use crate::decode::Nv12Frame;
use phonecam_protocol::StreamProfile;
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NativePixelFormat {
    Nv12,
    Yuyv,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct NativeOutputFormat {
    pub width: u16,
    pub height: u16,
    pub fps: u8,
    pub pixel_format: NativePixelFormat,
}

#[cfg(any(target_os = "macos", target_os = "windows", test))]
const FORMAT_EVENT_MAGIC: &[u8; 4] = b"PCFM";
#[cfg(any(target_os = "macos", target_os = "windows", test))]
const FORMAT_EVENT_SIZE: usize = 16;

#[cfg(any(target_os = "macos", target_os = "windows", test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeFormatEvent {
    pub format: NativeOutputFormat,
}

#[cfg(any(target_os = "macos", target_os = "windows", test))]
impl NativeFormatEvent {
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() != FORMAT_EVENT_SIZE || &bytes[..4] != FORMAT_EVENT_MAGIC {
            return Err("invalid native format event header".to_owned());
        }
        let width_u32 = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
        let height_u32 = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
        let width = u16::try_from(width_u32)
            .map_err(|_| "native format event width exceeds u16".to_owned())?;
        let height = u16::try_from(height_u32)
            .map_err(|_| "native format event height exceeds u16".to_owned())?;
        let fps = bytes[12];
        if !phonecam_protocol::SUPPORTED_DIMENSIONS.contains(&(width, height))
            || !phonecam_protocol::SUPPORTED_FRAME_RATES.contains(&fps)
        {
            return Err("unsupported native format event tuple".to_owned());
        }
        let pixel_format = match bytes[13] {
            0 => NativePixelFormat::Nv12,
            1 => NativePixelFormat::Yuyv,
            _ => return Err("unsupported native pixel format".to_owned()),
        };
        if bytes[14] != 0 || bytes[15] != 0 {
            return Err("native format event reserved bytes must be zero".to_owned());
        }
        Ok(Self {
            format: NativeOutputFormat {
                width,
                height,
                fps,
                pixel_format,
            },
        })
    }

    #[cfg(test)]
    pub fn encode(self) -> [u8; FORMAT_EVENT_SIZE] {
        let mut bytes = [0u8; FORMAT_EVENT_SIZE];
        bytes[..4].copy_from_slice(FORMAT_EVENT_MAGIC);
        bytes[4..8].copy_from_slice(&u32::from(self.format.width).to_le_bytes());
        bytes[8..12].copy_from_slice(&u32::from(self.format.height).to_le_bytes());
        bytes[12] = self.format.fps;
        bytes[13] = match self.format.pixel_format {
            NativePixelFormat::Nv12 => 0,
            NativePixelFormat::Yuyv => 1,
        };
        bytes
    }
}

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
    committed_profile: Option<StreamProfile>,
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
            committed_profile: None,
        })
    }
    pub fn preflight(&self, profile: &StreamProfile) -> Result<(), String> {
        profile
            .validate()
            .map_err(|error| format!("unsupported output profile: {error}"))?;
        let supported = self
            .device
            .supports_format(
                u32::from(profile.width),
                u32::from(profile.height),
                PixelFormat::YUYV,
                u32::from(profile.fps),
            )
            .map_err(|error| format!("failed to probe V4L2 output profile: {error}"))?;
        if !supported {
            return Err(format!(
                "V4L2 output does not support {}x{}@{}",
                profile.width, profile.height, profile.fps
            ));
        }
        Ok(())
    }

    pub fn commit(&mut self, profile: &StreamProfile) -> Result<(), String> {
        self.preflight(profile)?;
        self.device
            .set_format(
                u32::from(profile.width),
                u32::from(profile.height),
                PixelFormat::YUYV,
                u32::from(profile.fps),
            )
            .map_err(|error| format!("failed to commit V4L2 output format: {error}"))?;
        self.converter = Some(
            Nv12ToYuyvConverter::new(u32::from(profile.width), u32::from(profile.height))
                .map_err(|error| format!("converter initialization failed: {error:?}"))?,
        );
        self.committed_profile = Some(*profile);
        Ok(())
    }
    pub fn output_format(&self) -> Option<NativeOutputFormat> {
        self.committed_profile.map(|profile| NativeOutputFormat {
            width: profile.width,
            height: profile.height,
            fps: profile.fps,
            pixel_format: NativePixelFormat::Yuyv,
        })
    }

    pub fn write_frame(&mut self, frame: &Nv12Frame, _timestamp_ns: u64) -> Result<(), String> {
        let profile = self
            .committed_profile
            .ok_or_else(|| "output profile has not been committed".to_owned())?;
        if frame.width != u32::from(profile.width) || frame.height != u32::from(profile.height) {
            return Err(format!(
                "decoded frame {}x{} does not match committed input {}x{}",
                frame.width, frame.height, profile.width, profile.height
            ));
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
pub struct OutputDevice {
    driver: MacOsDriver,
    committed_profile: Option<StreamProfile>,
}

#[cfg(target_os = "macos")]
impl OutputDevice {
    pub fn open() -> Result<Self, String> {
        let mut driver = MacOsDriver::new();
        driver
            .connect()
            .map_err(|err| format!("failed to connect to the CMIO extension: {err}"))?;
        Ok(Self {
            driver,
            committed_profile: None,
        })
    }

    pub fn preflight(&self, profile: &StreamProfile) -> Result<(), String> {
        profile
            .validate()
            .map_err(|error| format!("unsupported output profile: {error}"))
    }

    pub fn commit(&mut self, profile: &StreamProfile) -> Result<(), String> {
        self.preflight(profile)?;
        let bytes = self
            .driver
            .read_format_event()
            .map_err(|error| format!("failed reading CMIO negotiated format: {error}"))?;
        let native = NativeFormatEvent::parse(&bytes)?;
        if native.format.width != profile.width
            || native.format.height != profile.height
            || native.format.fps != profile.fps
            || native.format.pixel_format != NativePixelFormat::Nv12
        {
            return Err(format!(
                "CMIO selected {}x{}@{} {:?}, not requested {}x{}@{} NV12",
                native.format.width,
                native.format.height,
                native.format.fps,
                native.format.pixel_format,
                profile.width,
                profile.height,
                profile.fps
            ));
        }
        self.committed_profile = Some(*profile);
        Ok(())
    }
    pub fn output_format(&self) -> Option<NativeOutputFormat> {
        self.committed_profile.map(|profile| NativeOutputFormat {
            width: profile.width,
            height: profile.height,
            fps: profile.fps,
            pixel_format: NativePixelFormat::Nv12,
        })
    }

    pub fn write_frame(&mut self, frame: &Nv12Frame, timestamp_ns: u64) -> Result<(), String> {
        let profile = self
            .committed_profile
            .ok_or_else(|| "output profile has not been committed".to_owned())?;
        if frame.width != u32::from(profile.width) || frame.height != u32::from(profile.height) {
            return Err("decoded frame does not match committed input profile".to_owned());
        }
        let data = packed_nv12(frame)?;
        self.driver
            .write_frame(frame.width, frame.height, timestamp_ns, &data)
            .map_err(|err| format!("failed writing frame to the CMIO extension: {err}"))
    }
}

#[cfg(target_os = "windows")]
pub struct OutputDevice {
    driver: WindowsDriver,
    committed_profile: Option<StreamProfile>,
}

#[cfg(target_os = "windows")]
impl OutputDevice {
    pub fn open() -> Result<Self, String> {
        let mut driver = WindowsDriver::new();
        driver
            .connect()
            .map_err(|err| format!("failed to connect to the DirectShow filter: {err}"))?;
        Ok(Self {
            driver,
            committed_profile: None,
        })
    }

    pub fn preflight(&self, profile: &StreamProfile) -> Result<(), String> {
        profile
            .validate()
            .map_err(|error| format!("unsupported output profile: {error}"))
    }

    pub fn commit(&mut self, profile: &StreamProfile) -> Result<(), String> {
        self.preflight(profile)?;
        let bytes = self
            .driver
            .read_format_event()
            .map_err(|error| format!("failed reading DirectShow negotiated format: {error}"))?;
        let native = NativeFormatEvent::parse(&bytes)?;
        if native.format.width != profile.width
            || native.format.height != profile.height
            || native.format.fps != profile.fps
            || native.format.pixel_format != NativePixelFormat::Nv12
        {
            return Err(format!(
                "DirectShow selected {}x{}@{} {:?}, not requested {}x{}@{} NV12",
                native.format.width,
                native.format.height,
                native.format.fps,
                native.format.pixel_format,
                profile.width,
                profile.height,
                profile.fps
            ));
        }
        self.committed_profile = Some(*profile);
        Ok(())
    }
    pub fn output_format(&self) -> Option<NativeOutputFormat> {
        self.committed_profile.map(|profile| NativeOutputFormat {
            width: profile.width,
            height: profile.height,
            fps: profile.fps,
            pixel_format: NativePixelFormat::Nv12,
        })
    }

    pub fn write_frame(&mut self, frame: &Nv12Frame, timestamp_ns: u64) -> Result<(), String> {
        let profile = self
            .committed_profile
            .ok_or_else(|| "output profile has not been committed".to_owned())?;
        if frame.width != u32::from(profile.width) || frame.height != u32::from(profile.height) {
            return Err("decoded frame does not match committed input profile".to_owned());
        }
        let data = packed_nv12(frame)?;
        self.driver
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
    use super::{
        packed_nv12, NativeFormatEvent, NativeOutputFormat, NativePixelFormat, FORMAT_EVENT_SIZE,
    };
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

    #[test]
    fn native_format_event_round_trips_exact_tuple() {
        let event = NativeFormatEvent {
            format: NativeOutputFormat {
                width: 3840,
                height: 2160,
                fps: 60,
                pixel_format: NativePixelFormat::Nv12,
            },
        };
        let bytes = event.encode();
        assert_eq!(bytes.len(), FORMAT_EVENT_SIZE);
        assert_eq!(NativeFormatEvent::parse(&bytes), Ok(event));
    }

    #[test]
    fn native_format_event_rejects_invalid_tuples_and_reserved_bytes() {
        let event = NativeFormatEvent {
            format: NativeOutputFormat {
                width: 1920,
                height: 1080,
                fps: 30,
                pixel_format: NativePixelFormat::Yuyv,
            },
        };
        let mut bytes = event.encode();
        bytes[12] = 59;
        assert!(NativeFormatEvent::parse(&bytes).is_err());
        bytes = event.encode();
        bytes[15] = 1;
        assert!(NativeFormatEvent::parse(&bytes).is_err());
    }
}
