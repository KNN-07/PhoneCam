use v4l::FourCC;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    YUYV,
    MJPEG,
    RGB24,
    NV12,
}

impl From<PixelFormat> for FourCC {
    fn from(fmt: PixelFormat) -> Self {
        match fmt {
            PixelFormat::YUYV => FourCC::new(b"YUYV"),
            PixelFormat::MJPEG => FourCC::new(b"MJPG"),
            PixelFormat::RGB24 => FourCC::new(b"RGB3"),
            PixelFormat::NV12 => FourCC::new(b"NV12"),
        }
    }
}

impl PixelFormat {
    pub fn to_fourcc(&self) -> FourCC {
        (*self).into()
    }
}
