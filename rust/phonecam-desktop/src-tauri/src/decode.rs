use std::time::Duration;

use phonecam_protocol::VideoFrame;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Nv12Frame {
    pub width: u32,
    pub height: u32,
    pub pts_us: u64,
    pub y_stride: usize,
    pub uv_stride: usize,
    pub y_plane: Vec<u8>,
    pub uv_plane: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodeOutput {
    pub frames: Vec<Nv12Frame>,
    pub request_keyframe: bool,
    pub decode_time: Duration,
}

#[derive(Debug)]
pub enum DecodeError {
    DecoderUnavailable,
    DecodeFailed(String),
}

pub struct H264Decoder;

impl H264Decoder {
    pub fn new() -> Result<Self, DecodeError> {
        todo!("implemented in green phase")
    }

    pub fn decode(&mut self, _video_frame: &VideoFrame) -> Result<DecodeOutput, DecodeError> {
        todo!("implemented in green phase")
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use phonecam_protocol::VideoFrame;

    use super::H264Decoder;

    const SAMPLE_H264_ANNEX_B: &[u8] = &[
        0x00, 0x00, 0x00, 0x01, 0x67, 0x42, 0xC0, 0x0A, 0xDC, 0x42, 0x6C, 0x04, 0x40, 0x00, 0x00,
        0x03, 0x00, 0x40, 0x00, 0x00, 0x03, 0x00, 0xA3, 0xC4, 0x89, 0xE0, 0x00, 0x00, 0x00, 0x01,
        0x68, 0xCE, 0x0F, 0xC8, 0x00, 0x00, 0x00, 0x01, 0x65, 0x88, 0x84, 0x3A, 0x26, 0x28, 0x00,
        0x09, 0x02, 0xC9, 0xC9, 0xC9, 0xD7, 0x5D, 0x75, 0xD7, 0x5D, 0x75, 0xD7,
    ];

    #[test]
    fn decode_h264_annex_b_to_nv12() {
        let mut decoder = H264Decoder::new().expect("decoder must initialize");
        let nal_units = split_annex_b_nalus(SAMPLE_H264_ANNEX_B);
        let mut frames = Vec::new();

        for (index, nal_unit) in nal_units.iter().enumerate() {
            let video_frame = VideoFrame {
                nal_unit: nal_unit.to_vec().into(),
                pts_us: (index as u64) * 33_333,
                width: 64,
                height: 64,
                is_keyframe: index + 1 == nal_units.len(),
            };

            let output = decoder
                .decode(&video_frame)
                .expect("decode must succeed for sample stream");
            frames.extend(output.frames);
        }

        assert!(!frames.is_empty(), "must emit at least one decoded frame");

        let frame = &frames[0];
        assert_eq!(frame.width, 64);
        assert_eq!(frame.height, 64);
        assert_eq!(frame.y_plane.len(), 64 * 64);
        assert_eq!(frame.uv_plane.len(), (64 * 64) / 2);
    }

    #[test]
    fn decoder_requests_keyframe_after_decode_error() {
        let mut decoder = H264Decoder::new().expect("decoder must initialize");

        let invalid_frame = VideoFrame {
            nal_unit: vec![0x00, 0x00, 0x00, 0x01, 0xFF, 0xFF].into(),
            pts_us: 1,
            width: 64,
            height: 64,
            is_keyframe: false,
        };

        let output = decoder
            .decode(&invalid_frame)
            .expect("invalid frame should not hard-fail decode pipeline");
        assert!(
            output.request_keyframe,
            "decode errors should trigger keyframe request"
        );

        let non_keyframe_after_error = VideoFrame {
            nal_unit: split_annex_b_nalus(SAMPLE_H264_ANNEX_B)[0].to_vec().into(),
            pts_us: 2,
            width: 64,
            height: 64,
            is_keyframe: false,
        };

        let output = decoder
            .decode(&non_keyframe_after_error)
            .expect("pipeline should continue running while waiting for keyframe");
        assert!(
            output.request_keyframe,
            "decoder should keep requesting keyframe until an IDR is received"
        );

        let recovery_keyframe = VideoFrame {
            nal_unit: SAMPLE_H264_ANNEX_B.to_vec().into(),
            pts_us: 3,
            width: 64,
            height: 64,
            is_keyframe: true,
        };

        let output = decoder
            .decode(&recovery_keyframe)
            .expect("decoder should recover when keyframe arrives");
        assert!(
            !output.request_keyframe,
            "recovered decoder should clear keyframe request"
        );
    }

    #[test]
    fn decode_latency_benchmark_sample_under_20ms() {
        let mut decoder = H264Decoder::new().expect("decoder must initialize");
        let mut total = Duration::ZERO;
        let iterations: u32 = 30;

        for index in 0..iterations {
            let frame = VideoFrame {
                nal_unit: SAMPLE_H264_ANNEX_B.to_vec().into(),
                pts_us: index as u64,
                width: 64,
                height: 64,
                is_keyframe: true,
            };

            let started = Instant::now();
            let output = decoder
                .decode(&frame)
                .expect("benchmark decode should succeed");
            let elapsed = started.elapsed();

            if output.decode_time > Duration::ZERO {
                total += output.decode_time;
            } else {
                total += elapsed;
            }
        }

        let average = total / iterations;
        assert!(
            average < Duration::from_millis(20),
            "expected <20ms average decode latency for sample stream, got {average:?}"
        );
    }

    fn split_annex_b_nalus(data: &[u8]) -> Vec<&[u8]> {
        let mut units = Vec::new();
        let mut i = 0;

        while i + 3 < data.len() {
            let start_code_len = if data[i..].starts_with(&[0x00, 0x00, 0x00, 0x01]) {
                4
            } else if data[i..].starts_with(&[0x00, 0x00, 0x01]) {
                3
            } else {
                i += 1;
                continue;
            };

            let nal_start = i + start_code_len;
            let mut nal_end = data.len();
            let mut cursor = nal_start;

            while cursor + 3 < data.len() {
                if data[cursor..].starts_with(&[0x00, 0x00, 0x00, 0x01])
                    || data[cursor..].starts_with(&[0x00, 0x00, 0x01])
                {
                    nal_end = cursor;
                    break;
                }
                cursor += 1;
            }

            units.push(&data[i..nal_end]);
            i = nal_end;
        }

        units
    }
}
