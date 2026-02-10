pub mod device;
pub mod errors;
pub mod format;

use std::path::Path;

pub use device::{list_devices, V4l2Device};
pub use errors::{DriverError, Result};
pub use format::PixelFormat;

const MODULE_NAME: &str = "v4l2loopback";

pub fn is_v4l2loopback_loaded() -> bool {
    is_v4l2loopback_loaded_in(Path::new("/sys/module"))
}

pub fn is_v4l2loopback_loaded_in(module_root: &Path) -> bool {
    module_root.join(MODULE_NAME).exists()
}

pub fn ensure_v4l2loopback_loaded() -> Result<()> {
    ensure_v4l2loopback_loaded_in(Path::new("/sys/module"))
}

pub fn ensure_v4l2loopback_loaded_in(module_root: &Path) -> Result<()> {
    if is_v4l2loopback_loaded_in(module_root) {
        Ok(())
    } else {
        Err(DriverError::ModuleNotLoaded {
            instructions: "v4l2loopback module not found. Please install it:\n\
                - Ubuntu/Debian: sudo apt-get install v4l2loopback-dkms\n\
                - Fedora: sudo dnf install v4l2loopback\n\
                - Arch: sudo pacman -S v4l2loopback-dkms\n\
                Then load it with: sudo modprobe v4l2loopback exclusive_caps=1"
                .to_string(),
        })
    }
}
