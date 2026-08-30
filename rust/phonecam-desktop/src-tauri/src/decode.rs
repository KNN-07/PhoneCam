use std::time::{Duration, Instant};

use openh264::{decoder::Decoder, formats::YUVSource};
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

pub struct H264Decoder {
    decoder: Decoder,
    waiting_for_keyframe: bool,
}

impl H264Decoder {
    pub fn new() -> Result<Self, DecodeError> {
        let decoder = Decoder::new().map_err(|err| DecodeError::DecodeFailed(err.to_string()))?;

        Ok(Self {
            decoder,
            waiting_for_keyframe: false,
        })
    }

    pub fn decode(&mut self, video_frame: &VideoFrame) -> Result<DecodeOutput, DecodeError> {
        let started = Instant::now();

        if self.waiting_for_keyframe && !video_frame.is_keyframe {
            return Ok(DecodeOutput {
                frames: Vec::new(),
                request_keyframe: true,
                decode_time: started.elapsed(),
            });
        }

        let payload = match normalize_annex_b(&video_frame.nal_unit) {
            Ok(payload) => payload,
            Err(()) => {
                self.waiting_for_keyframe = true;
                return Ok(DecodeOutput {
                    frames: Vec::new(),
                    request_keyframe: true,
                    decode_time: started.elapsed(),
                });
            }
        };

        self.waiting_for_keyframe = false;
        let frames = match self.decoder.decode(&payload) {
            Ok(Some(decoded)) => vec![decoded_frame_to_nv12(&decoded, video_frame.pts_us)?],
            Ok(None) => Vec::new(),
            Err(_) => {
                self.waiting_for_keyframe = true;
                Vec::new()
            }
        };

        Ok(DecodeOutput {
            frames,
            request_keyframe: self.waiting_for_keyframe,
            decode_time: started.elapsed(),
        })
    }
}

fn decoded_frame_to_nv12(
    decoded: &impl YUVSource,
    fallback_pts_us: u64,
) -> Result<Nv12Frame, DecodeError> {
    let (width, height) = decoded.dimensions();
    if width == 0 || height == 0 || width % 2 != 0 || height % 2 != 0 {
        return Err(DecodeError::DecodeFailed(format!(
            "decoder produced invalid dimensions {width}x{height}"
        )));
    }

    let (y_stride, u_stride, v_stride) = decoded.strides();
    let y_plane = copy_plane_rows(decoded.y(), y_stride, width, height)?;
    let chroma_width = width / 2;
    let chroma_height = height / 2;
    let u_plane = copy_plane_rows(decoded.u(), u_stride, chroma_width, chroma_height)?;
    let v_plane = copy_plane_rows(decoded.v(), v_stride, chroma_width, chroma_height)?;
    let mut uv_plane = Vec::with_capacity(width * chroma_height);
    for (&u, &v) in u_plane.iter().zip(v_plane.iter()) {
        uv_plane.push(u);
        uv_plane.push(v);
    }

    Ok(Nv12Frame {
        width: width as u32,
        height: height as u32,
        pts_us: fallback_pts_us,
        y_stride: width,
        uv_stride: width,
        y_plane,
        uv_plane,
    })
}

fn copy_plane_rows(
    source: &[u8],
    source_stride: usize,
    row_width: usize,
    row_count: usize,
) -> Result<Vec<u8>, DecodeError> {
    if source_stride < row_width {
        return Err(DecodeError::DecodeFailed(
            "decoded plane stride is smaller than its visible width".to_string(),
        ));
    }

    let required = source_stride
        .checked_mul(row_count)
        .ok_or_else(|| DecodeError::DecodeFailed("decoded plane size overflow".to_string()))?;
    if source.len() < required {
        return Err(DecodeError::DecodeFailed(
            "decoded plane is shorter than its stride layout".to_string(),
        ));
    }

    let output_len = row_width
        .checked_mul(row_count)
        .ok_or_else(|| DecodeError::DecodeFailed("decoded output size overflow".to_string()))?;
    let mut output = Vec::with_capacity(output_len);
    for row in 0..row_count {
        let start = row * source_stride;
        output.extend_from_slice(&source[start..start + row_width]);
    }
    Ok(output)
}

fn normalize_annex_b(data: &[u8]) -> Result<Vec<u8>, ()> {
    if data.is_empty() {
        return Err(());
    }

    if has_annex_b_start_code(data) {
        return validate_annex_b(data).then(|| data.to_vec()).ok_or(());
    }

    if let Ok(converted) = convert_avcc_to_annex_b(data) {
        return Ok(converted);
    }

    if valid_nal_header(data[0]) {
        let mut converted = Vec::with_capacity(data.len() + 4);
        converted.extend_from_slice(&[0, 0, 0, 1]);
        converted.extend_from_slice(data);
        return Ok(converted);
    }

    Err(())
}

fn has_annex_b_start_code(data: &[u8]) -> bool {
    data.starts_with(&[0, 0, 1]) || data.starts_with(&[0, 0, 0, 1])
}

fn validate_annex_b(data: &[u8]) -> bool {
    let mut cursor = 0;
    let mut found = false;

    while cursor + 3 <= data.len() {
        let start_code_len = if data[cursor..].starts_with(&[0, 0, 0, 1]) {
            4
        } else if data[cursor..].starts_with(&[0, 0, 1]) {
            3
        } else {
            cursor += 1;
            continue;
        };

        let header_index = cursor + start_code_len;
        if header_index >= data.len() || !valid_nal_header(data[header_index]) {
            return false;
        }
        found = true;
        cursor = header_index + 1;
    }

    found
}

fn convert_avcc_to_annex_b(data: &[u8]) -> Result<Vec<u8>, ()> {
    let mut cursor = 0usize;
    let mut converted = Vec::with_capacity(data.len() + 4);

    while cursor < data.len() {
        if data.len() - cursor < 4 {
            return Err(());
        }
        let nal_len =
            u32::from_be_bytes(data[cursor..cursor + 4].try_into().map_err(|_| ())?) as usize;
        cursor += 4;

        if nal_len == 0 || nal_len > data.len() - cursor || !valid_nal_header(data[cursor]) {
            return Err(());
        }

        converted.extend_from_slice(&[0, 0, 0, 1]);
        converted.extend_from_slice(&data[cursor..cursor + nal_len]);
        cursor += nal_len;
    }

    (!converted.is_empty()).then_some(converted).ok_or(())
}

fn valid_nal_header(header: u8) -> bool {
    let nal_type = header & 0x1f;
    header & 0x80 == 0 && (1..=23).contains(&nal_type)
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use openh264::{encoder::Encoder, formats::YUVBuffer};
    use phonecam_protocol::VideoFrame;

    use super::H264Decoder;

    fn sample_h264_annex_b() -> Vec<u8> {
        let image = YUVBuffer::new(64, 64);
        Encoder::new()
            .expect("test encoder must initialize")
            .encode(&image)
            .expect("test frame must encode")
            .to_vec()
    }

    #[test]
    fn decode_h264_annex_b_to_nv12() {
        let mut decoder = H264Decoder::new().expect("decoder must initialize");
        let video_frame = VideoFrame {
            nal_unit: sample_h264_annex_b().into(),
            pts_us: 33_333,
            width: 64,
            height: 64,
            is_keyframe: true,
        };
        let frames = decoder
            .decode(&video_frame)
            .expect("decode must succeed for sample stream")
            .frames;

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
            nal_unit: sample_h264_annex_b().into(),
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
            nal_unit: sample_h264_annex_b().into(),
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
        let sample = sample_h264_annex_b();
        let mut total = Duration::ZERO;
        let iterations: u32 = 30;

        for index in 0..iterations {
            let frame = VideoFrame {
                nal_unit: sample.clone().into(),
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
}
