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

pub struct Nv12ToYuyvConverter {
    width: u32,
    height: u32,
}

impl Nv12ToYuyvConverter {
    pub fn new(width: u32, height: u32) -> Result<Self, ConvertError> {
        if width == 0 || height == 0 || width % 2 != 0 || height % 2 != 0 {
            return Err(ConvertError::InvalidDimensions);
        }

        Ok(Self { width, height })
    }

    pub fn convert(&mut self, frame: &Nv12Frame) -> Result<YuyvFrame, ConvertError> {
        if frame.width != self.width
            || frame.height != self.height
            || frame.y_stride < self.width as usize
            || frame.uv_stride < self.width as usize
        {
            return Err(ConvertError::InvalidDimensions);
        }

        let height = self.height as usize;
        let width = self.width as usize;
        let required_y_len = frame
            .y_stride
            .checked_mul(height)
            .ok_or(ConvertError::InvalidPlaneLayout)?;
        let required_uv_len = frame
            .uv_stride
            .checked_mul(height / 2)
            .ok_or(ConvertError::InvalidPlaneLayout)?;

        if frame.y_plane.len() < required_y_len || frame.uv_plane.len() < required_uv_len {
            return Err(ConvertError::InvalidPlaneLayout);
        }

        let output_len = width
            .checked_mul(height)
            .and_then(|pixels| pixels.checked_mul(2))
            .ok_or(ConvertError::InvalidDimensions)?;
        let mut data = Vec::with_capacity(output_len);

        for row in 0..height {
            let y_row_start = row * frame.y_stride;
            let uv_row_start = (row / 2) * frame.uv_stride;

            for column in (0..width).step_by(2) {
                data.push(frame.y_plane[y_row_start + column]);
                data.push(frame.uv_plane[uv_row_start + column]);
                data.push(frame.y_plane[y_row_start + column + 1]);
                data.push(frame.uv_plane[uv_row_start + column + 1]);
            }
        }

        Ok(YuyvFrame {
            width: self.width,
            height: self.height,
            data,
        })
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

    #[test]
    fn convert_respects_source_plane_strides() {
        let mut converter = Nv12ToYuyvConverter::new(2, 2).expect("converter must initialize");
        let input = Nv12Frame {
            width: 2,
            height: 2,
            pts_us: 0,
            y_stride: 4,
            uv_stride: 4,
            y_plane: vec![10, 20, 0, 0, 30, 40, 0, 0],
            uv_plane: vec![100, 150, 0, 0],
        };

        let output = converter.convert(&input).expect("padded NV12 must convert");
        assert_eq!(output.data, vec![10, 100, 20, 150, 30, 100, 40, 150]);
    }

    #[test]
    fn converter_rejects_odd_dimensions() {
        assert!(matches!(
            Nv12ToYuyvConverter::new(3, 2),
            Err(super::ConvertError::InvalidDimensions)
        ));
    }
}
