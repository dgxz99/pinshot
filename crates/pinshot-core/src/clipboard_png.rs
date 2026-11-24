//! 在剪贴板 PNG 的私有辅助块中保留截图倍率，不重编码图像或占用额外的 MIME 类型

const PNG_HEADER: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR";
const IHDR_END: usize = 33;
// 编辑像素的程序不应把未知的倍率块复制到输出图像
const SCALE_CHUNK: &[u8; 4] = b"psCL";
const SCALE_CHUNK_LEN: usize = 20;

/// 仅接受 Pinshot 在 IHDR 后写入的倍率块，普通 PNG 不受影响
pub(super) fn source_scale(png: &[u8]) -> Option<f64> {
    let chunk = scale_chunk(png)?;
    let expected_crc = u32::from_be_bytes(chunk[16..20].try_into().ok()?);
    if crc32fast::hash(&chunk[4..16]) != expected_crc {
        return None;
    }
    let scale = f64::from_be_bytes(chunk[8..16].try_into().ok()?);
    valid_scale(scale).then_some(scale)
}

fn scale_chunk(png: &[u8]) -> Option<&[u8]> {
    if !png.starts_with(PNG_HEADER) {
        return None;
    }
    let chunk = png.get(IHDR_END..IHDR_END + SCALE_CHUNK_LEN)?;
    (chunk[..4] == 8u32.to_be_bytes() && chunk[4..8] == *SCALE_CHUNK).then_some(chunk)
}

fn valid_scale(scale: f64) -> bool {
    (0.25..=16.0).contains(&scale)
}

/// 替换已有倍率块或在 IHDR 后写入新块，避免重复复制时累积元数据
pub(super) fn with_source_scale(png: &[u8], scale: f64) -> Option<Vec<u8>> {
    if !valid_scale(scale) || png.len() < IHDR_END || !png.starts_with(PNG_HEADER) {
        return None;
    }
    let mut chunk = [0; SCALE_CHUNK_LEN];
    chunk[..4].copy_from_slice(&8u32.to_be_bytes());
    chunk[4..8].copy_from_slice(SCALE_CHUNK);
    chunk[8..16].copy_from_slice(&scale.to_be_bytes());
    let crc = crc32fast::hash(&chunk[4..16]);
    chunk[16..20].copy_from_slice(&crc.to_be_bytes());
    let previous_len = scale_chunk(png).map_or(0, <[u8]>::len);
    let mut output = Vec::with_capacity(png.len() + SCALE_CHUNK_LEN - previous_len);
    output.extend_from_slice(&png[..IHDR_END]);
    output.extend_from_slice(&chunk);
    output.extend_from_slice(&png[IHDR_END + previous_len..]);
    Some(output)
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use image::{ImageFormat, RgbImage};

    use super::*;

    fn png_fixture() -> (RgbImage, Vec<u8>) {
        let pixels = RgbImage::from_pixel(11, 17, image::Rgb([10, 20, 30]));
        let mut png = Cursor::new(Vec::new());
        pixels.write_to(&mut png, ImageFormat::Png).unwrap();
        (pixels, png.into_inner())
    }

    #[test]
    fn stores_scale_without_changing_pixels_or_other_chunks() {
        let (pixels, png) = png_fixture();
        assert_eq!(source_scale(&png), None);
        let tagged = with_source_scale(&png, 1.25).unwrap();
        assert_eq!(source_scale(&tagged), Some(1.25));
        assert_eq!(tagged[..IHDR_END], png[..IHDR_END]);
        assert_eq!(tagged[IHDR_END + SCALE_CHUNK_LEN..], png[IHDR_END..]);
        assert_eq!(
            image::load_from_memory(&tagged).unwrap().into_rgb8(),
            pixels
        );

        let replaced = with_source_scale(&tagged, 1.0).unwrap();
        assert_eq!(source_scale(&replaced), Some(1.0));
        assert_eq!(replaced.len(), tagged.len());
    }

    #[test]
    fn ignores_truncated_corrupt_or_unreasonable_scale_tags() {
        let (_, png) = png_fixture();
        let mut tagged = with_source_scale(&png, 1.25).unwrap();
        assert_eq!(
            source_scale(&tagged[..IHDR_END + SCALE_CHUNK_LEN - 1]),
            None
        );
        tagged[IHDR_END + 16] ^= 1;
        assert_eq!(source_scale(&tagged), None);
        for invalid in [0.0, f64::NAN, f64::INFINITY, 0.01, 1e8] {
            assert!(with_source_scale(&png, invalid).is_none());
            tagged[IHDR_END + 8..IHDR_END + 16].copy_from_slice(&invalid.to_be_bytes());
            let crc = crc32fast::hash(&tagged[IHDR_END + 4..IHDR_END + 16]);
            tagged[IHDR_END + 16..IHDR_END + 20].copy_from_slice(&crc.to_be_bytes());
            assert_eq!(source_scale(&tagged), None);
        }
    }
}
