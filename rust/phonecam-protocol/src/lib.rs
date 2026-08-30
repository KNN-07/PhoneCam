#![allow(dead_code)]

use bytes::Bytes;
use serde::{Deserialize, Serialize};

pub mod framing;
pub mod messages;

pub use messages::{Message, MessageCodecError, MessageType};
pub const PROTOCOL_VERSION: u8 = 2;
pub const SUPPORTED_DIMENSIONS: [(u16, u16); 5] = [
    (640, 480),
    (1280, 720),
    (1920, 1080),
    (2560, 1440),
    (3840, 2160),
];
pub const SUPPORTED_FRAME_RATES: [u8; 3] = [15, 30, 60];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
#[serde(rename_all = "snake_case")]
pub enum VideoCodec {
    H264 = 0,
    Hevc = 1,
}

impl TryFrom<u8> for VideoCodec {
    type Error = ProfileValidationError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::H264),
            1 => Ok(Self::Hevc),
            other => Err(ProfileValidationError::InvalidCodec(other)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct StreamProfile {
    pub codec: VideoCodec,
    pub width: u16,
    pub height: u16,
    pub fps: u8,
}

impl StreamProfile {
    pub const H264_720P30: Self = Self {
        codec: VideoCodec::H264,
        width: 1280,
        height: 720,
        fps: 30,
    };

    pub fn validate(&self) -> Result<(), ProfileValidationError> {
        if !SUPPORTED_DIMENSIONS.contains(&(self.width, self.height)) {
            return Err(ProfileValidationError::InvalidDimensions {
                width: self.width,
                height: self.height,
            });
        }
        if !SUPPORTED_FRAME_RATES.contains(&self.fps) {
            return Err(ProfileValidationError::InvalidFrameRate(self.fps));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProfileValidationError {
    #[error("unsupported protocol version {actual}; expected {expected}")]
    UnsupportedProtocolVersion { actual: u8, expected: u8 },
    #[error("supported profile list must not be empty")]
    EmptySupportedProfiles,
    #[error("duplicate supported profile: {0:?}")]
    DuplicateProfile(StreamProfile),
    #[error("unsupported video codec id: {0}")]
    InvalidCodec(u8),
    #[error("unsupported dimensions: {width}x{height}")]
    InvalidDimensions { width: u16, height: u16 },
    #[error("unsupported frame rate: {0}")]
    InvalidFrameRate(u8),
    #[error("active profile is not present in supported profiles: {0:?}")]
    ActiveProfileUnsupported(StreamProfile),
    #[error(
        "video frame metadata {actual_codec:?} {actual_width}x{actual_height} does not match active profile {expected:?}"
    )]
    FrameProfileMismatch {
        actual_codec: VideoCodec,
        actual_width: u16,
        actual_height: u16,
        expected: StreamProfile,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Handshake {
    pub version: u8,
    pub device_name: String,
    pub supported_profiles: Vec<StreamProfile>,
    pub active_profile: Option<StreamProfile>,
}

impl Handshake {
    pub fn validate(&self) -> Result<(), ProfileValidationError> {
        if self.version != PROTOCOL_VERSION {
            return Err(ProfileValidationError::UnsupportedProtocolVersion {
                actual: self.version,
                expected: PROTOCOL_VERSION,
            });
        }
        validate_profiles(&self.supported_profiles)?;
        if let Some(active_profile) = self.active_profile {
            if !self.supported_profiles.contains(&active_profile) {
                return Err(ProfileValidationError::ActiveProfileUnsupported(
                    active_profile,
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VideoCapabilitiesUpdate {
    pub supported_profiles: Vec<StreamProfile>,
}

impl VideoCapabilitiesUpdate {
    pub fn validate(&self, active_profile: StreamProfile) -> Result<(), ProfileValidationError> {
        validate_profiles(&self.supported_profiles)?;
        if !self.supported_profiles.contains(&active_profile) {
            return Err(ProfileValidationError::ActiveProfileUnsupported(
                active_profile,
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VideoFrame {
    pub data: Bytes,
    pub pts_us: u64,
    pub codec: VideoCodec,
    pub width: u16,
    pub height: u16,
    pub is_keyframe: bool,
}

impl VideoFrame {
    pub fn validate_against(
        &self,
        active_profile: StreamProfile,
    ) -> Result<(), ProfileValidationError> {
        if self.codec != active_profile.codec
            || self.width != active_profile.width
            || self.height != active_profile.height
        {
            return Err(ProfileValidationError::FrameProfileMismatch {
                actual_codec: self.codec,
                actual_width: self.width,
                actual_height: self.height,
                expected: active_profile,
            });
        }
        Ok(())
    }
}

fn validate_profiles(profiles: &[StreamProfile]) -> Result<(), ProfileValidationError> {
    if profiles.is_empty() {
        return Err(ProfileValidationError::EmptySupportedProfiles);
    }
    for (index, profile) in profiles.iter().enumerate() {
        profile.validate()?;
        if profiles[..index].contains(profile) {
            return Err(ProfileValidationError::DuplicateProfile(*profile));
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AudioCodec {
    Opus,
    Aac,
    Pcm16,
}

#[deprecated(note = "Reserved for v2")]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioFrame {
    pub codec: AudioCodec,
    pub sample_rate: u32,
    pub channels: u8,
    pub data: Bytes,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CameraControl {
    SwitchCamera {
        front: bool,
    },
    RequestKeyframe,
    ConfigureStream {
        request_id: u32,
        profile: StreamProfile,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamConfigurationResult {
    pub request_id: u32,
    pub result: StreamConfigurationOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum StreamConfigurationOutcome {
    Applied(StreamProfile),
    UnsupportedProfile,
    CaptureConfigurationFailed,
    EncoderInitializationFailed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusUpdate {
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Disconnect {
    pub reason: Option<String>,
}

#[cfg(test)]
mod tests;
