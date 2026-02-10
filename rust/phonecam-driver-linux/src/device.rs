use crate::errors::Result;
use crate::format::PixelFormat;
use std::path::{Path, PathBuf};
use v4l::context;

pub struct V4l2Device {
    path: PathBuf,
}

impl V4l2Device {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        Ok(Self {
            path: path.as_ref().to_path_buf(),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn set_format(&self, _width: u32, _height: u32, _fmt: PixelFormat) -> Result<()> {
        // Stub for now
        Ok(())
    }

    pub fn write_frame(&self, _data: &[u8]) -> Result<()> {
        // Stub for now
        Ok(())
    }
}

pub fn list_devices() -> Vec<PathBuf> {
    context::enum_devices()
        .into_iter()
        .map(|node| node.path().to_path_buf())
        .collect()
}
