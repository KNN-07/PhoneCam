use crate::decode::Nv12Frame;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct YuyvFrame {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

#[derive(Debug)]
pub enum ConvertError {
    InvalidDimensions,
    InvalidPlaneLayout,
    ConversionFailed(String),
}

pub struct Nv12ToYuyvConverter;

impl Nv12ToYuyvConverter {
    pub fn new(_width: u32, _height: u32) -> Result<Self, ConvertError> {
        todo!("implemented in green phase")
    }

    pub fn convert(&mut self, _frame: &Nv12Frame) -> Result<YuyvFrame, ConvertError> {
        todo!("implemented in green phase")
    }
}

#[cfg(test)]
mod tests {
    use super::Nv12ToYuyvConverter;
    use crate::decode::Nv12Frame;

    #[test]
    fn convert_nv12_to_yuyv_for_2x2_frame() {
        let mut converter = Nv12ToYuyvConverter::new(2, 2).expect("converter must initialize");

        let input = Nv12Frame {
            width: 2,
            height: 2,
            pts_us: 123,
            y_stride: 2,
            uv_stride: 2,
            y_plane: vec![10, 20, 30, 40],
            uv_plane: vec![128, 128],
        };

        let output = converter
            .convert(&input)
            .expect("valid NV12 should convert to YUYV");

        assert_eq!(output.width, 2);
        assert_eq!(output.height, 2);
        assert_eq!(output.data.len(), 8);
        assert_eq!(output.data, vec![10, 128, 20, 128, 30, 128, 40, 128]);
    }

    #[test]
    fn convert_rejects_invalid_nv12_plane_sizes() {
        let mut converter = Nv12ToYuyvConverter::new(4, 2).expect("converter must initialize");

        let invalid = Nv12Frame {
            width: 4,
            height: 2,
            pts_us: 0,
            y_stride: 4,
            uv_stride: 4,
            y_plane: vec![0; 3],
            uv_plane: vec![0; 4],
        };

        let err = converter
            .convert(&invalid)
            .expect_err("invalid plane shape must fail conversion");

        assert!(
            format!("{err:?}").contains("InvalidPlaneLayout"),
            "conversion should report invalid layout"
        );
    }
}
