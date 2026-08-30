#![allow(deprecated)]

use bytes::Bytes;
use proptest::prelude::*;

use crate::{
    framing::{decode_frame, encode_frame},
    messages::{Message, MessageType},
    AudioCodec, AudioFrame, CameraControl, Disconnect, Handshake, StatusUpdate, VideoFrame,
};

#[test]
fn handshake_roundtrip() {
    let message = Message::Handshake(Handshake {
        version: 1,
        device_name: "Pixel 7".to_string(),
        supported_resolutions: vec![(1920, 1080), (1280, 720), (640, 480)],
        supported_fps: vec![15, 30, 60],
    });

    let encoded = encode_frame(&message).expect("handshake should encode");
    let decoded = decode_frame(&encoded).expect("handshake should decode");

    assert_eq!(message, decoded);
}

#[test]
fn handshake_all_resolution_and_fps_combinations_roundtrip() {
    let message = Message::Handshake(Handshake {
        version: 1,
        device_name: "QA Device".to_string(),
        supported_resolutions: vec![
            (320, 240),
            (640, 480),
            (1280, 720),
            (1920, 1080),
            (3840, 2160),
        ],
        supported_fps: vec![1, 15, 24, 30, 60, 90, 120],
    });

    let encoded = encode_frame(&message).expect("handshake should encode");
    let decoded = decode_frame(&encoded).expect("handshake should decode");

    assert_eq!(message, decoded);
}

#[test]
fn video_frame_roundtrip() {
    let message = Message::VideoFrame(VideoFrame {
        nal_unit: Bytes::from_static(&[
            0x00, 0x00, 0x00, 0x01, 0x67, 0x64, 0x00, 0x1F, 0xAC, 0xD9, 0x40,
        ]),
        pts_us: 1_234_567_890,
        width: 1920,
        height: 1080,
        is_keyframe: true,
    });

    let encoded = encode_frame(&message).expect("video frame should encode");
    let decoded = decode_frame(&encoded).expect("video frame should decode");

    assert_eq!(message, decoded);
}

#[test]
fn video_frame_roundtrip_with_empty_nal_unit() {
    let message = Message::VideoFrame(VideoFrame {
        nal_unit: Bytes::new(),
        pts_us: 42,
        width: 640,
        height: 480,
        is_keyframe: false,
    });

    let encoded = encode_frame(&message).expect("video frame should encode");
    let decoded = decode_frame(&encoded).expect("video frame should decode");

    assert_eq!(message, decoded);
}

#[test]
fn camera_control_roundtrip() {
    for control in [
        CameraControl::SwitchCamera { front: true },
        CameraControl::RequestKeyframe,
        CameraControl::ConfigureStream {
            width: 1920,
            height: 1080,
            fps: 60,
        },
    ] {
        let message = Message::CameraControl(control);
        let encoded = encode_frame(&message).expect("camera control should encode");
        let decoded = decode_frame(&encoded).expect("camera control should decode");
        assert_eq!(message, decoded);
    }
}

#[test]
fn status_update_roundtrip() {
    let message = Message::StatusUpdate(StatusUpdate {
        status: "streaming".to_string(),
    });

    let encoded = encode_frame(&message).expect("status update should encode");
    let decoded = decode_frame(&encoded).expect("status update should decode");

    assert_eq!(message, decoded);
}

#[test]
fn disconnect_roundtrip() {
    let message = Message::Disconnect(Disconnect {
        reason: Some("user requested disconnect".to_string()),
    });

    let encoded = encode_frame(&message).expect("disconnect should encode");
    let decoded = decode_frame(&encoded).expect("disconnect should decode");

    assert_eq!(message, decoded);
}

#[test]
fn audio_frame_roundtrip() {
    let message = Message::AudioFrame(AudioFrame {
        codec: AudioCodec::Opus,
        sample_rate: 48_000,
        channels: 2,
        data: Bytes::from_static(&[0xDE, 0xAD, 0xBE, 0xEF]),
    });

    let encoded = encode_frame(&message).expect("audio frame should encode");
    let decoded = decode_frame(&encoded).expect("audio frame should decode");

    assert_eq!(message, decoded);
}

#[test]
fn framing_large_payload() {
    let message = Message::VideoFrame(VideoFrame {
        nal_unit: Bytes::from(vec![0xAA; 1024 * 1024]),
        pts_us: 9_876_543_210,
        width: 1920,
        height: 1080,
        is_keyframe: false,
    });

    let encoded = encode_frame(&message).expect("large payload should encode");
    let decoded = decode_frame(&encoded).expect("large payload should decode");

    assert_eq!(message, decoded);
}

#[test]
fn framing_uses_big_endian_length_and_explicit_type_byte() {
    let message = Message::Handshake(Handshake {
        version: 1,
        device_name: "Frame Test".to_string(),
        supported_resolutions: vec![(1920, 1080)],
        supported_fps: vec![30],
    });

    let encoded = encode_frame(&message).expect("frame should encode");
    let declared_len = u32::from_be_bytes(encoded[0..4].try_into().expect("length bytes")) as usize;

    assert_eq!(declared_len, encoded.len() - 4);
    assert_eq!(encoded[4], MessageType::Handshake as u8);
}

proptest! {
    #[test]
    fn message_roundtrip_property(message in message_strategy()) {
        let encoded = encode_frame(&message).expect("message should encode");
        let decoded = decode_frame(&encoded).expect("message should decode");

        prop_assert_eq!(message, decoded);
    }
}

fn message_strategy() -> impl Strategy<Value = Message> {
    prop_oneof![
        handshake_strategy().prop_map(Message::Handshake),
        video_frame_strategy().prop_map(Message::VideoFrame),
        audio_frame_strategy().prop_map(Message::AudioFrame),
        camera_control_strategy().prop_map(Message::CameraControl),
        status_update_strategy().prop_map(Message::StatusUpdate),
        disconnect_strategy().prop_map(Message::Disconnect),
    ]
}

fn handshake_strategy() -> impl Strategy<Value = Handshake> {
    (
        any::<u8>(),
        ascii_string_strategy(64),
        prop::collection::vec((1u16..4096u16, 1u16..4096u16), 0..8),
        prop::collection::vec(1u8..121u8, 0..8),
    )
        .prop_map(
            |(version, device_name, supported_resolutions, supported_fps)| Handshake {
                version,
                device_name,
                supported_resolutions,
                supported_fps,
            },
        )
}

fn video_frame_strategy() -> impl Strategy<Value = VideoFrame> {
    (
        prop::collection::vec(any::<u8>(), 0..4096).prop_map(Bytes::from),
        any::<u64>(),
        1u16..4096u16,
        1u16..4096u16,
        any::<bool>(),
    )
        .prop_map(
            |(nal_unit, pts_us, width, height, is_keyframe)| VideoFrame {
                nal_unit,
                pts_us,
                width,
                height,
                is_keyframe,
            },
        )
}

#[allow(deprecated)]
fn audio_frame_strategy() -> impl Strategy<Value = AudioFrame> {
    (
        prop_oneof![
            Just(AudioCodec::Opus),
            Just(AudioCodec::Aac),
            Just(AudioCodec::Pcm16),
        ],
        prop_oneof![
            Just(8_000u32),
            Just(16_000u32),
            Just(44_100u32),
            Just(48_000u32)
        ],
        1u8..9u8,
        prop::collection::vec(any::<u8>(), 0..2048).prop_map(Bytes::from),
    )
        .prop_map(|(codec, sample_rate, channels, data)| AudioFrame {
            codec,
            sample_rate,
            channels,
            data,
        })
}

fn camera_control_strategy() -> impl Strategy<Value = CameraControl> {
    prop_oneof![
        any::<bool>().prop_map(|front| CameraControl::SwitchCamera { front }),
        Just(CameraControl::RequestKeyframe),
        (1u16..=u16::MAX, 1u16..=u16::MAX, 1u8..=120u8)
            .prop_map(|(width, height, fps)| CameraControl::ConfigureStream { width, height, fps }),
    ]
}

fn status_update_strategy() -> impl Strategy<Value = StatusUpdate> {
    ascii_string_strategy(64).prop_map(|status| StatusUpdate { status })
}

fn disconnect_strategy() -> impl Strategy<Value = Disconnect> {
    prop_oneof![
        Just(Disconnect { reason: None }),
        ascii_string_strategy(64).prop_map(|reason| Disconnect {
            reason: Some(reason)
        }),
    ]
}

fn ascii_string_strategy(max_len: usize) -> impl Strategy<Value = String> {
    prop::collection::vec(32u8..127u8, 0..max_len).prop_map(|bytes| {
        String::from_utf8(bytes).expect("ascii bytes should always produce valid utf8")
    })
}
