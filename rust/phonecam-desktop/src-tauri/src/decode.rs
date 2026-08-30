use std::{
    panic::{catch_unwind, AssertUnwindSafe},
    thread,
    time::{Duration, Instant},
};

use crossbeam_channel::{Receiver, Sender, TryRecvError, TrySendError};
use openh264::{decoder::Decoder as OpenH264Decoder, formats::YUVSource};
use phonecam_protocol::{StreamProfile, VideoCodec, VideoFrame};
use rust_h265::{Decoder as HevcDecoder, Frame as HevcFrame, NalUnitType};
use tokio::sync::{mpsc, oneshot};

const FRAME_QUEUE_CAPACITY: usize = 2;
const CONTROL_QUEUE_CAPACITY: usize = 2;
const EVENT_QUEUE_CAPACITY: usize = 2;

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
pub enum DecodeEvent {
    Frame(Nv12Frame),
    RecoveryRequired,
    Error(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    DecoderUnavailable,
    DecodeFailed(String),
    Main10Unsupported,
    WorkerStopped,
}

enum DecodeControl {
    Reset {
        codec: VideoCodec,
        acknowledgement: oneshot::Sender<Result<(), DecodeError>>,
    },
    Shutdown,
}

pub struct DecodeWorker {
    frame_tx: Sender<VideoFrame>,
    control_tx: Sender<DecodeControl>,
    event_rx: mpsc::Receiver<DecodeEvent>,
    thread: Option<thread::JoinHandle<()>>,
}

impl DecodeWorker {
    pub fn new() -> Result<Self, DecodeError> {
        let (frame_tx, frame_rx) = crossbeam_channel::bounded(FRAME_QUEUE_CAPACITY);
        let (control_tx, control_rx) = crossbeam_channel::bounded(CONTROL_QUEUE_CAPACITY);
        let (event_tx, event_rx) = mpsc::channel(EVENT_QUEUE_CAPACITY);
        let (startup_tx, startup_rx) = std::sync::mpsc::sync_channel(1);
        let worker = thread::Builder::new()
            .name("phonecam-video-decoder".to_owned())
            .spawn(move || decoder_thread(frame_rx, control_rx, event_tx, startup_tx))
            .map_err(|error| DecodeError::DecodeFailed(error.to_string()))?;
        startup_rx
            .recv()
            .map_err(|_| DecodeError::WorkerStopped)??;
        Ok(Self {
            frame_tx,
            control_tx,
            event_rx,
            thread: Some(worker),
        })
    }

    pub fn try_send(&self, frame: VideoFrame) -> Result<(), TrySendError<VideoFrame>> {
        self.frame_tx.try_send(frame)
    }

    pub async fn reset(&mut self, codec: VideoCodec) -> Result<(), DecodeError> {
        let (acknowledgement, received) = oneshot::channel();
        self.control_tx
            .send(DecodeControl::Reset {
                codec,
                acknowledgement,
            })
            .map_err(|_| DecodeError::WorkerStopped)?;
        received.await.map_err(|_| DecodeError::WorkerStopped)??;
        while self.event_rx.try_recv().is_ok() {}
        Ok(())
    }

    pub async fn recv(&mut self) -> Option<DecodeEvent> {
        self.event_rx.recv().await
    }
}

impl Drop for DecodeWorker {
    fn drop(&mut self) {
        self.event_rx.close();
        let _ = self.control_tx.send(DecodeControl::Shutdown);
        if let Some(worker) = self.thread.take() {
            let _ = worker.join();
        }
    }
}

struct H264Backend {
    decoder: OpenH264Decoder,
}

impl H264Backend {
    fn new() -> Result<Self, DecodeError> {
        OpenH264Decoder::new()
            .map(|decoder| Self { decoder })
            .map_err(|error| DecodeError::DecodeFailed(error.to_string()))
    }

    fn decode(&mut self, frame: &VideoFrame) -> Result<Vec<Nv12Frame>, DecodeError> {
        let payload = normalize_h264_annex_b(&frame.data)
            .ok_or_else(|| DecodeError::DecodeFailed("malformed H.264 Annex-B data".to_owned()))?;
        match self.decoder.decode(&payload) {
            Ok(Some(decoded)) => Ok(vec![h264_frame_to_nv12(&decoded, frame.pts_us)?]),
            Ok(None) => Ok(Vec::new()),
            Err(error) => Err(DecodeError::DecodeFailed(error.to_string())),
        }
    }
}

struct HevcBackend {
    decoder: HevcDecoder,
    saw_vps: bool,
    saw_sps: bool,
    saw_pps: bool,
}

impl HevcBackend {
    fn new() -> Self {
        Self {
            decoder: HevcDecoder::new(),
            saw_vps: false,
            saw_sps: false,
            saw_pps: false,
        }
    }

    fn decode(&mut self, frame: &VideoFrame) -> Result<Vec<Nv12Frame>, DecodeError> {
        let nals = rust_h265::parse_annex_b(&frame.data);
        if nals.is_empty() {
            return Err(DecodeError::DecodeFailed(
                "malformed HEVC Annex-B data".to_owned(),
            ));
        }
        let mut decoded = Vec::new();
        for nal in &nals {
            match nal.nal_unit_type {
                NalUnitType::Vps => self.saw_vps = true,
                NalUnitType::Sps => self.saw_sps = true,
                NalUnitType::Pps => self.saw_pps = true,
                kind if kind.is_idr() && !(self.saw_vps && self.saw_sps && self.saw_pps) => {
                    return Err(DecodeError::DecodeFailed(
                        "HEVC IDR arrived before VPS/SPS/PPS".to_owned(),
                    ));
                }
                _ => {}
            }
            if let Some(output) = self
                .decoder
                .decode_nal(nal)
                .map_err(|error| DecodeError::DecodeFailed(format!("{error:?}")))?
            {
                decoded.push(hevc_frame_to_nv12(output, frame.pts_us)?);
            }
        }
        Ok(decoded)
    }
}

fn decoder_thread(
    frame_rx: Receiver<VideoFrame>,
    control_rx: Receiver<DecodeControl>,
    event_tx: mpsc::Sender<DecodeEvent>,
    startup_tx: std::sync::mpsc::SyncSender<Result<(), DecodeError>>,
) {
    let mut h264 = match H264Backend::new() {
        Ok(decoder) => decoder,
        Err(error) => {
            let _ = startup_tx.send(Err(error));
            return;
        }
    };
    let mut hevc = HevcBackend::new();
    let _ = startup_tx.send(Ok(()));
    let mut codec = VideoCodec::H264;
    let mut waiting_for_idr = true;

    loop {
        while let Ok(control) = control_rx.try_recv() {
            if !apply_control(
                control,
                &frame_rx,
                &mut h264,
                &mut hevc,
                &mut codec,
                &mut waiting_for_idr,
            ) {
                return;
            }
        }

        crossbeam_channel::select_biased! {
            recv(control_rx) -> control => {
                match control {
                    Ok(control) => {
                        if !apply_control(
                            control,
                            &frame_rx,
                            &mut h264,
                            &mut hevc,
                            &mut codec,
                            &mut waiting_for_idr,
                        ) {
                            return;
                        }
                    }
                    Err(_) => return,
                }
            }
            recv(frame_rx) -> frame => {
                let Ok(frame) = frame else {
                    return;
                };
                if frame.codec != codec || (waiting_for_idr && !frame.is_keyframe) {
                    if event_tx.blocking_send(DecodeEvent::RecoveryRequired).is_err() {
                        return;
                    }
                    continue;
                }
                let result = catch_unwind(AssertUnwindSafe(|| match codec {
                    VideoCodec::H264 => h264.decode(&frame),
                    VideoCodec::Hevc => hevc.decode(&frame),
                }));
                match result {
                    Ok(Ok(frames)) => {
                        if frame.is_keyframe {
                            waiting_for_idr = false;
                        }
                        for frame in frames {
                            if event_tx.blocking_send(DecodeEvent::Frame(frame)).is_err() {
                                return;
                            }
                        }
                    }
                    Ok(Err(error)) => {
                        let message = format!("{error:?}");
                        waiting_for_idr = true;
                        match codec {
                            VideoCodec::H264 => {
                                let Ok(recreated) = H264Backend::new() else {
                                    return;
                                };
                                h264 = recreated;
                            }
                            VideoCodec::Hevc => hevc = HevcBackend::new(),
                        }
                        if event_tx.blocking_send(DecodeEvent::Error(message)).is_err()
                            || event_tx.blocking_send(DecodeEvent::RecoveryRequired).is_err()
                        {
                            return;
                        }
                    }
                    Err(_) => {
                        waiting_for_idr = true;
                        match codec {
                            VideoCodec::H264 => {
                                let Ok(recreated) = H264Backend::new() else {
                                    return;
                                };
                                h264 = recreated;
                            }
                            VideoCodec::Hevc => hevc = HevcBackend::new(),
                        }
                        if event_tx
                            .blocking_send(DecodeEvent::Error("decoder panicked".to_owned()))
                            .is_err()
                        {
                            return;
                        }
                    }
                }
            }
        }
    }
}

fn apply_control(
    control: DecodeControl,
    frame_rx: &Receiver<VideoFrame>,
    h264: &mut H264Backend,
    hevc: &mut HevcBackend,
    codec: &mut VideoCodec,
    waiting_for_idr: &mut bool,
) -> bool {
    match control {
        DecodeControl::Reset {
            codec: next_codec,
            acknowledgement,
        } => {
            while !matches!(
                frame_rx.try_recv(),
                Err(TryRecvError::Empty | TryRecvError::Disconnected)
            ) {}
            let result = match next_codec {
                VideoCodec::H264 => H264Backend::new().map(|decoder| *h264 = decoder),
                VideoCodec::Hevc => {
                    *hevc = HevcBackend::new();
                    Ok(())
                }
            };
            if result.is_ok() {
                *codec = next_codec;
                *waiting_for_idr = true;
            }
            let _ = acknowledgement.send(result);
            true
        }
        DecodeControl::Shutdown => false,
    }
}

fn h264_frame_to_nv12(decoded: &impl YUVSource, pts_us: u64) -> Result<Nv12Frame, DecodeError> {
    let (width, height) = decoded.dimensions();
    let (y_stride, u_stride, v_stride) = decoded.strides();
    planar_to_nv12(
        width as u32,
        height as u32,
        pts_us,
        decoded.y(),
        y_stride,
        decoded.u(),
        u_stride,
        decoded.v(),
        v_stride,
    )
}

fn hevc_frame_to_nv12(frame: HevcFrame, pts_us: u64) -> Result<Nv12Frame, DecodeError> {
    if frame.bit_depth != 8 {
        return Err(DecodeError::Main10Unsupported);
    }
    let y = frame.y.as_u8().ok_or(DecodeError::Main10Unsupported)?;
    let u = frame.u.as_u8().ok_or(DecodeError::Main10Unsupported)?;
    let v = frame.v.as_u8().ok_or(DecodeError::Main10Unsupported)?;
    planar_to_nv12(
        frame.width,
        frame.height,
        pts_us,
        y,
        frame.width as usize,
        u,
        (frame.width / 2) as usize,
        v,
        (frame.width / 2) as usize,
    )
}

#[allow(clippy::too_many_arguments)]
fn planar_to_nv12(
    width: u32,
    height: u32,
    pts_us: u64,
    y: &[u8],
    y_stride: usize,
    u: &[u8],
    u_stride: usize,
    v: &[u8],
    v_stride: usize,
) -> Result<Nv12Frame, DecodeError> {
    let width_usize = usize::try_from(width)
        .map_err(|_| DecodeError::DecodeFailed("width overflow".to_owned()))?;
    let height_usize = usize::try_from(height)
        .map_err(|_| DecodeError::DecodeFailed("height overflow".to_owned()))?;
    if width == 0 || height == 0 || width % 2 != 0 || height % 2 != 0 {
        return Err(DecodeError::DecodeFailed(format!(
            "decoder produced invalid dimensions {width}x{height}"
        )));
    }
    let y_plane = copy_plane_rows(y, y_stride, width_usize, height_usize)?;
    let chroma_width = width_usize / 2;
    let chroma_height = height_usize / 2;
    let u_plane = copy_plane_rows(u, u_stride, chroma_width, chroma_height)?;
    let v_plane = copy_plane_rows(v, v_stride, chroma_width, chroma_height)?;
    let uv_len = width_usize
        .checked_mul(chroma_height)
        .ok_or_else(|| DecodeError::DecodeFailed("NV12 chroma size overflow".to_owned()))?;
    let mut uv_plane = Vec::with_capacity(uv_len);
    for (&u, &v) in u_plane.iter().zip(&v_plane) {
        uv_plane.push(u);
        uv_plane.push(v);
    }
    Ok(Nv12Frame {
        width,
        height,
        pts_us,
        y_stride: width_usize,
        uv_stride: width_usize,
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
            "decoded plane stride is smaller than visible width".to_owned(),
        ));
    }
    let required = source_stride
        .checked_mul(row_count)
        .ok_or_else(|| DecodeError::DecodeFailed("decoded plane size overflow".to_owned()))?;
    if source.len() < required {
        return Err(DecodeError::DecodeFailed(
            "decoded plane is shorter than its stride layout".to_owned(),
        ));
    }
    let capacity = row_width
        .checked_mul(row_count)
        .ok_or_else(|| DecodeError::DecodeFailed("decoded output size overflow".to_owned()))?;
    let mut output = Vec::with_capacity(capacity);
    for row in 0..row_count {
        let start = row * source_stride;
        output.extend_from_slice(&source[start..start + row_width]);
    }
    Ok(output)
}

fn normalize_h264_annex_b(data: &[u8]) -> Option<Vec<u8>> {
    if data.is_empty() {
        return None;
    }
    if data.starts_with(&[0, 0, 1]) || data.starts_with(&[0, 0, 0, 1]) {
        return validate_h264_annex_b(data).then(|| data.to_vec());
    }
    if data[0] & 0x80 == 0 && (1..=23).contains(&(data[0] & 0x1f)) {
        let mut annex_b = Vec::with_capacity(data.len() + 4);
        annex_b.extend_from_slice(&[0, 0, 0, 1]);
        annex_b.extend_from_slice(data);
        return Some(annex_b);
    }
    None
}

fn validate_h264_annex_b(data: &[u8]) -> bool {
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
        let header = cursor + start_code_len;
        if header >= data.len()
            || data[header] & 0x80 != 0
            || !(1..=23).contains(&(data[header] & 0x1f))
        {
            return false;
        }
        found = true;
        cursor = header + 1;
    }
    found
}

pub fn profile_meets_timing(profile: StreamProfile, timings: &[Duration]) -> bool {
    if timings.is_empty() || profile.fps == 0 {
        return false;
    }
    let mut ordered = timings.to_vec();
    ordered.sort_unstable();
    let p95_index = ((ordered.len() * 95).div_ceil(100)).saturating_sub(1);
    let frame_period = Duration::from_secs_f64(1.0 / f64::from(profile.fps));
    ordered[p95_index] <= frame_period.mul_f64(0.8)
}

pub fn measure_decode_times<F>(mut decode: F) -> Result<Vec<Duration>, DecodeError>
where
    F: FnMut(usize) -> Result<(), DecodeError>,
{
    let mut timings = Vec::with_capacity(55);
    for index in 0..60 {
        let started = Instant::now();
        decode(index)?;
        if index >= 5 {
            timings.push(started.elapsed());
        }
    }
    Ok(timings)
}

#[cfg(test)]
mod tests {
    use openh264::{encoder::Encoder, formats::YUVBuffer};

    use super::*;

    fn h264_frame(is_keyframe: bool) -> VideoFrame {
        let image = YUVBuffer::new(64, 64);
        let data = Encoder::new().unwrap().encode(&image).unwrap().to_vec();
        VideoFrame {
            data: data.into(),
            pts_us: 33_333,
            codec: VideoCodec::H264,
            width: 64,
            height: 64,
            is_keyframe,
        }
    }

    #[tokio::test]
    async fn worker_decodes_h264_to_tight_nv12() {
        let mut worker = DecodeWorker::new().unwrap();
        worker.reset(VideoCodec::H264).await.unwrap();
        worker.try_send(h264_frame(true)).unwrap();
        let event = tokio::time::timeout(Duration::from_secs(2), worker.recv())
            .await
            .unwrap()
            .unwrap();
        let DecodeEvent::Frame(frame) = event else {
            panic!("expected decoded frame, got {event:?}");
        };
        assert_eq!((frame.width, frame.height), (64, 64));
        assert_eq!(frame.y_plane.len(), 64 * 64);
        assert_eq!(frame.uv_plane.len(), 64 * 64 / 2);
    }

    #[tokio::test]
    async fn reset_drains_stale_frames_and_requires_new_idr() {
        let mut worker = DecodeWorker::new().unwrap();
        worker.try_send(h264_frame(true)).unwrap();
        worker.reset(VideoCodec::Hevc).await.unwrap();
        worker
            .try_send(VideoFrame {
                data: vec![0, 0, 0, 1, 0x02, 0x01].into(),
                pts_us: 1,
                codec: VideoCodec::Hevc,
                width: 64,
                height: 64,
                is_keyframe: false,
            })
            .unwrap();
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(2), worker.recv())
                .await
                .unwrap(),
            Some(DecodeEvent::RecoveryRequired)
        );
    }
    #[tokio::test]
    async fn worker_decodes_hevc_main_to_tight_nv12() {
        let mut worker = DecodeWorker::new().unwrap();
        worker.reset(VideoCodec::Hevc).await.unwrap();
        worker
            .try_send(VideoFrame {
                data: include_bytes!("../testdata/tiny_intra.h265")
                    .as_slice()
                    .into(),
                pts_us: 7,
                codec: VideoCodec::Hevc,
                width: 16,
                height: 16,
                is_keyframe: true,
            })
            .unwrap();
        let event = tokio::time::timeout(Duration::from_secs(2), worker.recv())
            .await
            .unwrap()
            .unwrap();
        let DecodeEvent::Frame(frame) = event else {
            panic!("expected HEVC frame, got {event:?}");
        };
        assert_eq!((frame.width, frame.height), (16, 16));
        assert_eq!(frame.y_plane.len(), 256);
        assert_eq!(frame.uv_plane.len(), 128);
    }

    #[test]
    fn hevc_main10_output_is_rejected() {
        let mut decoder = HevcBackend::new();
        let frame = VideoFrame {
            data: include_bytes!("../testdata/10bit_128x128.h265")
                .as_slice()
                .into(),
            pts_us: 0,
            codec: VideoCodec::Hevc,
            width: 128,
            height: 128,
            is_keyframe: true,
        };
        assert!(matches!(
            decoder.decode(&frame),
            Err(DecodeError::Main10Unsupported)
        ));
    }

    #[test]
    fn decoder_calibration_filters_p95_at_eighty_percent_of_frame_period() {
        let profile = StreamProfile {
            codec: VideoCodec::Hevc,
            width: 3840,
            height: 2160,
            fps: 60,
        };
        assert!(profile_meets_timing(
            profile,
            &vec![Duration::from_millis(10); 55]
        ));
        let mut slow = vec![Duration::from_millis(10); 55];
        slow[52] = Duration::from_millis(14);
        slow[53] = Duration::from_millis(14);
        slow[54] = Duration::from_millis(14);
        assert!(!profile_meets_timing(profile, &slow));
    }

    #[test]
    fn malformed_hevc_requires_parameter_sets_before_idr() {
        let mut decoder = HevcBackend::new();
        let frame = VideoFrame {
            data: vec![0, 0, 0, 1, 0x26, 0x01, 0x80].into(),
            pts_us: 0,
            codec: VideoCodec::Hevc,
            width: 64,
            height: 64,
            is_keyframe: true,
        };
        assert!(matches!(
            decoder.decode(&frame),
            Err(DecodeError::DecodeFailed(message)) if message.contains("VPS/SPS/PPS")
        ));
    }
}
