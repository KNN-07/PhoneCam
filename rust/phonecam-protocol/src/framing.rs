use thiserror::Error;

use crate::messages::{Message, MessageCodecError, MessageType};

pub const FRAME_LENGTH_PREFIX_BYTES: usize = 4;
pub const FRAME_TYPE_BYTES: usize = 1;
pub const MAX_FRAME_BYTES: usize = 32 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum FrameError {
    #[error("frame is too short; expected at least 5 bytes")]
    FrameTooShort,
    #[error("frame length mismatch: declared {declared} bytes, actual {actual} bytes")]
    LengthMismatch { declared: usize, actual: usize },
    #[error("frame length overflow for {0} bytes")]
    LengthOverflow(usize),
    #[error("frame exceeds the {MAX_FRAME_BYTES}-byte limit: {0} bytes")]
    FrameTooLarge(usize),
    #[error(transparent)]
    Codec(#[from] MessageCodecError),
}

pub fn encode_frame(message: &Message) -> Result<Vec<u8>, FrameError> {
    if let Message::VideoFrame(frame) = message {
        if frame.data.len() >= MAX_FRAME_BYTES {
            return Err(FrameError::FrameTooLarge(frame.data.len()));
        }
    }
    let message_type = message.message_type() as u8;
    let payload = message.encode_payload()?;

    let frame_length = FRAME_TYPE_BYTES
        .checked_add(payload.len())
        .ok_or(FrameError::LengthOverflow(payload.len()))?;
    if frame_length > MAX_FRAME_BYTES {
        return Err(FrameError::FrameTooLarge(frame_length));
    }
    let frame_length_u32 =
        u32::try_from(frame_length).map_err(|_| FrameError::LengthOverflow(frame_length))?;

    let mut frame = Vec::with_capacity(FRAME_LENGTH_PREFIX_BYTES + frame_length);
    frame.extend_from_slice(&frame_length_u32.to_be_bytes());
    frame.push(message_type);
    frame.extend_from_slice(&payload);

    Ok(frame)
}

pub fn decode_frame(frame: &[u8]) -> Result<Message, FrameError> {
    if frame.len() < FRAME_LENGTH_PREFIX_BYTES + FRAME_TYPE_BYTES {
        return Err(FrameError::FrameTooShort);
    }

    let declared_frame_length = u32::from_be_bytes(
        frame[..FRAME_LENGTH_PREFIX_BYTES]
            .try_into()
            .expect("length slice"),
    ) as usize;
    if declared_frame_length > MAX_FRAME_BYTES {
        return Err(FrameError::FrameTooLarge(declared_frame_length));
    }
    let actual_frame_length = frame.len() - FRAME_LENGTH_PREFIX_BYTES;

    if declared_frame_length != actual_frame_length {
        return Err(FrameError::LengthMismatch {
            declared: declared_frame_length,
            actual: actual_frame_length,
        });
    }

    let message_type = MessageType::try_from(frame[FRAME_LENGTH_PREFIX_BYTES])?;
    let payload = &frame[(FRAME_LENGTH_PREFIX_BYTES + FRAME_TYPE_BYTES)..];

    Message::decode_payload(message_type, payload).map_err(FrameError::from)
}
