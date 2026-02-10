use thiserror::Error;

#[derive(Error, Debug)]
pub enum DriverError {
    #[error("V4L2 error: {0}")]
    V4l2Error(#[from] std::io::Error),

    #[error("Device not found: {0}")]
    DeviceNotFound(String),

    #[error("Invalid format: {0}")]
    InvalidFormat(String),

    #[error("Driver not loaded")]
    DriverNotLoaded,

    #[error("v4l2loopback module not loaded. Instructions: {instructions}")]
    ModuleNotLoaded { instructions: String },

    #[error("Operation failed: {0}")]
    OperationFailed(String),
}

pub type Result<T> = std::result::Result<T, DriverError>;
