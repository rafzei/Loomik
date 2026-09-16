//! Checked conversion for V4L2 YUYV BT.601/709, full or limited range.
use anyhow::{Result, ensure};

pub fn yuyv(
    bytes: &[u8],
    width: u32,
    height: u32,
    stride: usize,
    rec709: bool,
    full: bool,
) -> Result<Vec<u8>> {
    let row = width as usize * 2;
    ensure!(
        width > 0 && height > 0 && width.is_multiple_of(2) && stride >= row,
        "Invalid YUYV camera dimensions or stride"
    );
    ensure!(
        stride
            .checked_mul(height as usize)
            .is_some_and(|size| size <= bytes.len()),
        "Truncated YUYV camera frame"
    );
    let mut pixels = Vec::with_capacity(width as usize * height as usize * 4);
    let (r_v, g_u, g_v, b_u) = match (rec709, full) {
        (false, false) => (409, 100, 208, 516),
        (true, false) => (459, 55, 136, 541),
        (false, true) => (359, 88, 183, 454),
        (true, true) => (403, 48, 120, 475),
    };
    for row in bytes.chunks_exact(stride).take(height as usize) {
        for pair in row[..width as usize * 2].as_chunks::<4>().0 {
            let (u, v) = (pair[1] as i32 - 128, pair[3] as i32 - 128);
            for y in [pair[0], pair[2]] {
                let y = if full {
                    y as i32 * 256
                } else {
                    (y as i32 - 16) * 298
                };
                let channel = |value: i32| ((value + 128) >> 8).clamp(0, 255) as u8;
                pixels.extend_from_slice(&[
                    channel(y + b_u * u),
                    channel(y - g_u * u - g_v * v),
                    channel(y + r_v * v),
                    255,
                ]);
            }
        }
    }
    Ok(pixels)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn camera_yuyv_respects_padding_and_rejects_truncated_rows() {
        let bytes = [
            16, 128, 235, 128, 99, 99, 99, 99, 81, 90, 81, 240, 99, 99, 99, 99,
        ];
        let image = yuyv(&bytes, 2, 2, 8, false, false).unwrap();
        assert_eq!(&image[..8], &[0, 0, 0, 255, 255, 255, 255, 255]);
        assert_eq!(&image[8..], &[0, 0, 255, 255, 0, 0, 255, 255]);
        assert!(yuyv(&bytes[..12], 2, 2, 8, false, false).is_err());
        assert!(yuyv(&bytes, 3, 2, 8, false, false).is_err());
        assert!(yuyv(&bytes, 4, 2, 4, false, false).is_err());
        assert_eq!(
            yuyv(&[0, 128, 255, 128], 2, 1, 4, true, true).unwrap(),
            [0, 0, 0, 255, 255, 255, 255, 255]
        );
        let red709 = yuyv(&[63, 102, 63, 240], 2, 1, 4, true, false).unwrap();
        assert!(red709[0] <= 1 && red709[1] <= 1 && red709[2] >= 254);
    }
}
