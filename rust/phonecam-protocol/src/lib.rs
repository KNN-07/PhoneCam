#![allow(dead_code)]

use bytes::Bytes;
use serde::{Deserialize, Serialize};

pub mod framing;
pub mod messages;

pub use messages::{Message, MessageCodecError, MessageType};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Handshake {
    pub version: u8,
    pub device_name: String,
    pub supported_resolutions: Vec<(u16, u16)>,
    pub supported_fps: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VideoFrame {
    pub nal_unit: Bytes,
    pub pts_us: u64,
    pub width: u16,
    pub height: u16,
    pub is_keyframe: bool,
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
    SwitchCamera { front: bool },
    RequestKeyframe,
    ConfigureStream { width: u16, height: u16, fps: u8 },
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
