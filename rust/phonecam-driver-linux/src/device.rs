use crate::errors::{DriverError, Result};
use crate::format::PixelFormat;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use v4l::context;
use v4l::frameinterval::FrameIntervalEnum;
use v4l::video::output::Parameters;
use v4l::video::Output;
use v4l::{Device, Format};

pub struct V4l2Device {
    path: PathBuf,
    device: Device,
    writer: File,
}

impl V4l2Device {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let device = Device::with_path(&path)?;
        let writer = OpenOptions::new().write(true).open(&path)?;
        Ok(Self {
            path,
            device,
            writer,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn supports_format(
        &self,
        width: u32,
        height: u32,
        pixel_format: PixelFormat,
        fps: u32,
    ) -> Result<bool> {
        if width == 0 || height == 0 || fps == 0 {
            return Ok(false);
        }
        let requested = Format::new(width, height, pixel_format.to_fourcc());
        let mut raw_format = v4l::v4l_sys::v4l2_format {
            type_: v4l::buffer::Type::VideoOutput as u32,
            fmt: v4l::v4l_sys::v4l2_format__bindgen_ty_1 {
                pix: requested.into(),
            },
        };
        unsafe {
            v4l::v4l2::ioctl(
                self.device.handle().fd(),
                v4l::v4l2::vidioc::VIDIOC_TRY_FMT,
                &mut raw_format as *mut _ as *mut std::os::raw::c_void,
            )?;
        }
        let actual = unsafe { Format::from(raw_format.fmt.pix) };
        if actual.width != width
            || actual.height != height
            || actual.fourcc != pixel_format.to_fourcc()
            || actual.size < width.saturating_mul(height).saturating_mul(2)
        {
            return Ok(false);
        }

        let intervals = self
            .device
            .enum_frameintervals(pixel_format.to_fourcc(), width, height)
            .unwrap_or_default();
        if intervals.is_empty() {
            return Ok(true);
        }
        Ok(intervals.into_iter().any(|entry| match entry.interval {
            FrameIntervalEnum::Discrete(value) => {
                u64::from(value.numerator) * u64::from(fps) == u64::from(value.denominator)
            }
            FrameIntervalEnum::Stepwise(value) => {
                let requested = 1.0 / f64::from(fps);
                let minimum = f64::from(value.min.numerator) / f64::from(value.min.denominator);
                let maximum = f64::from(value.max.numerator) / f64::from(value.max.denominator);
                minimum <= requested && requested <= maximum
            }
        }))
    }

    pub fn set_format(
        &mut self,
        width: u32,
        height: u32,
        pixel_format: PixelFormat,
        fps: u32,
    ) -> Result<()> {
        let requested = Format::new(width, height, pixel_format.to_fourcc());
        let actual = self.device.set_format(&requested)?;
        let minimum_size = width
            .checked_mul(height)
            .and_then(|pixels| pixels.checked_mul(2))
            .ok_or_else(|| DriverError::InvalidFormat("frame size overflows u32".to_owned()))?;
        if actual.width != width
            || actual.height != height
            || actual.fourcc != pixel_format.to_fourcc()
            || actual.size < minimum_size
        {
            return Err(DriverError::InvalidFormat(format!(
                "driver returned {}x{} {} size {}, requested {}x{} {} size at least {}",
                actual.width,
                actual.height,
                actual.fourcc,
                actual.size,
                width,
                height,
                pixel_format.to_fourcc(),
                minimum_size,
            )));
        }
        let parameters = self.device.set_params(&Parameters::with_fps(fps))?;
        if u64::from(parameters.interval.numerator) * u64::from(fps)
            != u64::from(parameters.interval.denominator)
        {
            return Err(DriverError::InvalidFormat(format!(
                "driver returned interval {}, requested 1/{fps}",
                parameters.interval,
            )));
        }
        Ok(())
    }

    pub fn write_frame(&mut self, data: &[u8]) -> Result<()> {
        self.writer.write_all(data)?;
        Ok(())
    }
}

pub fn list_devices() -> Vec<PathBuf> {
    context::enum_devices()
        .into_iter()
        .map(|node| node.path().to_path_buf())
        .collect()
}
