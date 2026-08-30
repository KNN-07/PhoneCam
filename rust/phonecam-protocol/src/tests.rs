#![allow(deprecated)]

use bytes::Bytes;
use proptest::prelude::*;

use crate::{
    framing::{decode_frame, encode_frame, FrameError, MAX_FRAME_BYTES},
    messages::{Message, MessageType},
    AudioCodec, AudioFrame, CameraControl, Disconnect, Handshake, ProfileValidationError,
    StatusUpdate, StreamConfigurationOutcome, StreamConfigurationResult, StreamProfile,
    VideoCapabilitiesUpdate, VideoCodec, VideoFrame, PROTOCOL_VERSION, SUPPORTED_DIMENSIONS,
    SUPPORTED_FRAME_RATES,
};

fn profile(codec: VideoCodec, width: u16, height: u16, fps: u8) -> StreamProfile {
    StreamProfile {
        codec,
        width,
        height,
        fps,
    }
}

#[test]
fn protocol_v2_messages_roundtrip() {
    let h264 = StreamProfile::H264_720P30;
    let hevc = profile(VideoCodec::Hevc, 3840, 2160, 60);
    let messages = [
        Message::Handshake(Handshake {
            version: PROTOCOL_VERSION,
            device_name: "Pixel".to_owned(),
            supported_profiles: vec![h264, hevc],
            active_profile: Some(h264),
        }),
        Message::VideoFrame(VideoFrame {
            data: Bytes::from_static(&[0, 0, 0, 1, 0x65]),
            pts_us: 42,
            codec: VideoCodec::H264,
            width: 1280,
            height: 720,
            is_keyframe: true,
        }),
        Message::CameraControl(CameraControl::ConfigureStream {
            request_id: 17,
            profile: hevc,
        }),
        Message::StreamConfigurationResult(StreamConfigurationResult {
            request_id: 17,
            result: StreamConfigurationOutcome::Applied(hevc),
        }),
        Message::VideoCapabilitiesUpdate(VideoCapabilitiesUpdate {
            supported_profiles: vec![h264, hevc],
        }),
        Message::StatusUpdate(StatusUpdate {
            status: "streaming".to_owned(),
        }),
        Message::Disconnect(Disconnect {
            reason: Some("done".to_owned()),
        }),
    ];

    for message in messages {
        let encoded = encode_frame(&message).expect("message should encode");
        assert_eq!(
            decode_frame(&encoded).expect("message should decode"),
            message
        );
    }
}

#[test]
fn exact_profile_validation_rejects_invalid_handshakes() {
    let active = StreamProfile::H264_720P30;
    let valid = Handshake {
        version: PROTOCOL_VERSION,
        device_name: "phone".to_owned(),
        supported_profiles: vec![active],
        active_profile: Some(active),
    };
    assert_eq!(valid.validate(), Ok(()));

    let mut invalid = valid.clone();
    invalid.version = 1;
    assert!(matches!(
        invalid.validate(),
        Err(ProfileValidationError::UnsupportedProtocolVersion { .. })
    ));

    invalid = valid.clone();
    invalid.supported_profiles.clear();
    assert_eq!(
        invalid.validate(),
        Err(ProfileValidationError::EmptySupportedProfiles)
    );

    invalid = valid.clone();
    invalid.supported_profiles.push(active);
    assert_eq!(
        invalid.validate(),
        Err(ProfileValidationError::DuplicateProfile(active))
    );

    invalid = valid.clone();
    invalid.supported_profiles[0].width = 1024;
    assert!(matches!(
        invalid.validate(),
        Err(ProfileValidationError::InvalidDimensions { .. })
    ));

    invalid = valid.clone();
    invalid.supported_profiles[0].fps = 24;
    assert_eq!(
        invalid.validate(),
        Err(ProfileValidationError::InvalidFrameRate(24))
    );

    invalid = valid;
    invalid.active_profile = Some(profile(VideoCodec::Hevc, 1280, 720, 30));
    assert!(matches!(
        invalid.validate(),
        Err(ProfileValidationError::ActiveProfileUnsupported(_))
    ));
}

#[test]
fn capability_updates_are_exact_and_retain_active_profile() {
    let active = StreamProfile::H264_720P30;
    let update = VideoCapabilitiesUpdate {
        supported_profiles: vec![active, profile(VideoCodec::Hevc, 2560, 1440, 60)],
    };
    assert_eq!(update.validate(active), Ok(()));
    assert!(matches!(
        update.validate(profile(VideoCodec::H264, 640, 480, 30)),
        Err(ProfileValidationError::ActiveProfileUnsupported(_))
    ));
}

#[test]
fn frames_must_match_the_committed_profile() {
    let frame = VideoFrame {
        data: Bytes::new(),
        pts_us: 0,
        codec: VideoCodec::Hevc,
        width: 1280,
        height: 720,
        is_keyframe: false,
    };
    assert_eq!(
        frame.validate_against(profile(VideoCodec::Hevc, 1280, 720, 60)),
        Ok(())
    );
    assert!(matches!(
        frame.validate_against(StreamProfile::H264_720P30),
        Err(ProfileValidationError::FrameProfileMismatch { .. })
    ));
}

#[test]
fn message_type_bytes_remain_stable_and_extend_v2() {
    assert_eq!(MessageType::Handshake as u8, 1);
    assert_eq!(MessageType::VideoFrame as u8, 2);
    assert_eq!(MessageType::AudioFrame as u8, 3);
    assert_eq!(MessageType::CameraControl as u8, 4);
    assert_eq!(MessageType::StatusUpdate as u8, 5);
    assert_eq!(MessageType::Disconnect as u8, 6);
    assert_eq!(MessageType::StreamConfigurationResult as u8, 7);
    assert_eq!(MessageType::VideoCapabilitiesUpdate as u8, 8);
}

#[test]
fn sixteen_mib_video_frame_is_accepted() {
    let message = Message::VideoFrame(VideoFrame {
        data: Bytes::from(vec![0xAA; 16 * 1024 * 1024]),
        pts_us: 9,
        codec: VideoCodec::Hevc,
        width: 3840,
        height: 2160,
        is_keyframe: true,
    });
    let encoded = encode_frame(&message).expect("16 MiB payload should encode");
    assert_eq!(
        decode_frame(&encoded).expect("16 MiB payload should decode"),
        message
    );
}

#[test]
fn messages_over_thirty_two_mib_are_rejected() {
    let message = Message::VideoFrame(VideoFrame {
        data: Bytes::from(vec![0; MAX_FRAME_BYTES]),
        pts_us: 0,
        codec: VideoCodec::H264,
        width: 1280,
        height: 720,
        is_keyframe: false,
    });
    assert!(matches!(
        encode_frame(&message),
        Err(FrameError::FrameTooLarge(_))
    ));

    let declared = (MAX_FRAME_BYTES as u32 + 1).to_be_bytes();
    let mut oversized = Vec::from(declared);
    oversized.push(MessageType::VideoFrame as u8);
    assert!(matches!(
        decode_frame(&oversized),
        Err(FrameError::FrameTooLarge(_))
    ));
}

#[test]
fn framing_uses_big_endian_length_and_explicit_type_byte() {
    let message = Message::Handshake(Handshake {
        version: PROTOCOL_VERSION,
        device_name: "Frame Test".to_owned(),
        supported_profiles: vec![StreamProfile::H264_720P30],
        active_profile: None,
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
        profile_strategy().prop_map(|active| Message::Handshake(Handshake {
            version: PROTOCOL_VERSION,
            device_name: "generated".to_owned(),
            supported_profiles: vec![active],
            active_profile: Some(active),
        })),
        (
            profile_strategy(),
            prop::collection::vec(any::<u8>(), 0..4096).prop_map(Bytes::from),
            any::<u64>(),
            any::<bool>(),
        )
            .prop_map(|(profile, data, pts_us, is_keyframe)| Message::VideoFrame(
                VideoFrame {
                    data,
                    pts_us,
                    codec: profile.codec,
                    width: profile.width,
                    height: profile.height,
                    is_keyframe,
                }
            )),
        (any::<u32>(), profile_strategy()).prop_map(|(request_id, profile)| {
            Message::CameraControl(CameraControl::ConfigureStream {
                request_id,
                profile,
            })
        }),
        (any::<u32>(), profile_strategy()).prop_map(|(request_id, profile)| {
            Message::StreamConfigurationResult(StreamConfigurationResult {
                request_id,
                result: StreamConfigurationOutcome::Applied(profile),
            })
        }),
        profile_strategy().prop_map(|profile| {
            Message::VideoCapabilitiesUpdate(VideoCapabilitiesUpdate {
                supported_profiles: vec![profile],
            })
        }),
        audio_frame_strategy().prop_map(Message::AudioFrame),
    ]
}

fn profile_strategy() -> impl Strategy<Value = StreamProfile> {
    (
        prop_oneof![Just(VideoCodec::H264), Just(VideoCodec::Hevc)],
        prop::sample::select(SUPPORTED_DIMENSIONS.to_vec()),
        prop::sample::select(SUPPORTED_FRAME_RATES.to_vec()),
    )
        .prop_map(|(codec, (width, height), fps)| profile(codec, width, height, fps))
}

#[allow(deprecated)]
fn audio_frame_strategy() -> impl Strategy<Value = AudioFrame> {
    (
        prop_oneof![
            Just(AudioCodec::Opus),
            Just(AudioCodec::Aac),
            Just(AudioCodec::Pcm16),
        ],
        prop::sample::select(vec![8_000u32, 16_000, 44_100, 48_000]),
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
