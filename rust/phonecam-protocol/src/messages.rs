#![allow(deprecated)]

use serde::{de::DeserializeOwned, Serialize};
use thiserror::Error;

use crate::{AudioFrame, CameraControl, Disconnect, Handshake, StatusUpdate, VideoFrame};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    Handshake(Handshake),
    VideoFrame(VideoFrame),
    #[allow(deprecated)]
    AudioFrame(AudioFrame),
    CameraControl(CameraControl),
    StatusUpdate(StatusUpdate),
    Disconnect(Disconnect),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum MessageType {
    Handshake = 1,
    VideoFrame = 2,
    AudioFrame = 3,
    CameraControl = 4,
    StatusUpdate = 5,
    Disconnect = 6,
}

#[derive(Debug, Error)]
pub enum MessageCodecError {
    #[error("failed to encode message payload: {0}")]
    Encode(#[from] bincode::error::EncodeError),
    #[error("failed to decode message payload: {0}")]
    Decode(#[from] bincode::error::DecodeError),
    #[error("unknown message type byte: {0}")]
    UnknownMessageType(u8),
    #[error("payload had trailing bytes after decode (decoded {decoded} of {payload_len} bytes)")]
    TrailingBytes { decoded: usize, payload_len: usize },
}

impl Message {
    pub fn message_type(&self) -> MessageType {
        #[allow(deprecated)]
        match self {
            Message::Handshake(_) => MessageType::Handshake,
            Message::VideoFrame(_) => MessageType::VideoFrame,
            Message::AudioFrame(_) => MessageType::AudioFrame,
            Message::CameraControl(_) => MessageType::CameraControl,
            Message::StatusUpdate(_) => MessageType::StatusUpdate,
            Message::Disconnect(_) => MessageType::Disconnect,
        }
    }

    pub(crate) fn encode_payload(&self) -> Result<Vec<u8>, MessageCodecError> {
        #[allow(deprecated)]
        match self {
            Message::Handshake(message) => encode_payload(message),
            Message::VideoFrame(message) => encode_payload(message),
            Message::AudioFrame(message) => encode_payload(message),
            Message::CameraControl(message) => encode_payload(message),
            Message::StatusUpdate(message) => encode_payload(message),
            Message::Disconnect(message) => encode_payload(message),
        }
    }

    pub(crate) fn decode_payload(
        message_type: MessageType,
        payload: &[u8],
    ) -> Result<Self, MessageCodecError> {
        #[allow(deprecated)]
        let message = match message_type {
            MessageType::Handshake => Message::Handshake(decode_payload(payload)?),
            MessageType::VideoFrame => Message::VideoFrame(decode_payload(payload)?),
            MessageType::AudioFrame => Message::AudioFrame(decode_payload(payload)?),
            MessageType::CameraControl => Message::CameraControl(decode_payload(payload)?),
            MessageType::StatusUpdate => Message::StatusUpdate(decode_payload(payload)?),
            MessageType::Disconnect => Message::Disconnect(decode_payload(payload)?),
        };

        Ok(message)
    }
}

impl TryFrom<u8> for MessageType {
    type Error = MessageCodecError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            x if x == MessageType::Handshake as u8 => Ok(MessageType::Handshake),
            x if x == MessageType::VideoFrame as u8 => Ok(MessageType::VideoFrame),
            x if x == MessageType::AudioFrame as u8 => Ok(MessageType::AudioFrame),
            x if x == MessageType::CameraControl as u8 => Ok(MessageType::CameraControl),
            x if x == MessageType::StatusUpdate as u8 => Ok(MessageType::StatusUpdate),
            x if x == MessageType::Disconnect as u8 => Ok(MessageType::Disconnect),
            other => Err(MessageCodecError::UnknownMessageType(other)),
        }
    }
}

fn encode_payload<T>(message: &T) -> Result<Vec<u8>, MessageCodecError>
where
    T: Serialize,
{
    Ok(bincode::serde::encode_to_vec(
        message,
        bincode::config::standard(),
    )?)
}

fn decode_payload<T>(payload: &[u8]) -> Result<T, MessageCodecError>
where
    T: DeserializeOwned,
{
    let (decoded, bytes_read): (T, usize) =
        bincode::serde::decode_from_slice(payload, bincode::config::standard())?;

    if bytes_read != payload.len() {
        return Err(MessageCodecError::TrailingBytes {
            decoded: bytes_read,
            payload_len: payload.len(),
        });
    }

    Ok(decoded)
}
