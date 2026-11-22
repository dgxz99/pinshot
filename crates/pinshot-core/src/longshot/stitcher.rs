use image::RgbaImage;

use super::failure;
use crate::Result;

const ROW_FEATURES: usize = 16;
const FEATURE_ROWS: usize = 40;
const CHECK_COLUMNS: usize = 48;
const LOOKBACK_VIEWPORTS: usize = 3;
const OUTPUT_BYTES: usize = 192 * 1024 * 1024;
const MIN_EDGE: usize = 32;
const MIN_OVERLAP: usize = 24;
const MAX_FEATURE_ERROR: u32 = 7 * 256;
const MAX_PIXEL_ERROR: u32 = 10 * 256;
const STATIONARY_ERROR: u32 = 64;
const MIN_SCORE_GAP: u32 = 96;

type RowFeature = [u8; ROW_FEATURES];

/// 一帧处理后的可见状态，已知静止或回滚不改变画布
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Progress {
    Started { height: u32 },
    Extended { height: u32 },
    Unchanged,
    Unmatched,
    LimitReached { height: u32 },
}

/// 只把可信重叠之后新出现的底部像素追加到画布
pub(super) struct Stitcher {
    pixels: Vec<u8>,
    rows: Vec<RowFeature>,
    width: usize,
    frame_height: usize,
    height_limit: usize,
    last_top: usize,
}

impl Stitcher {
    pub(super) fn new(first: RgbaImage, height_limit: u32) -> Result<Self> {
        let (width, height) = (first.width() as usize, first.height() as usize);
        if width < MIN_EDGE || height < MIN_EDGE {
            return Err(failure("初始化长图拼接", "选区至少需要 32×32 视频像素"));
        }
        let height_limit = (height_limit as usize).min(OUTPUT_BYTES / (width * 4));
        if height > height_limit {
            return Err(failure("初始化长图拼接", "首帧已超出输出尺寸上限"));
        }
        let rows = row_features(first.as_raw(), width);
        let mut pixels = first.into_raw();
        // 指数扩容会越过预算；预留受限画布空间，物理页随实际写入增长
        pixels
            .try_reserve_exact(width * height_limit * 4 - pixels.len())
            .map_err(|error| failure("预留长图画布", error))?;
        Ok(Self {
            pixels,
            rows,
            width,
            frame_height: height,
            height_limit,
            last_top: 0,
        })
    }

    pub(super) fn height(&self) -> u32 {
        self.rows.len() as u32
    }

    pub(super) fn is_full(&self) -> bool {
        self.rows.len() == self.height_limit
    }

    pub(super) fn push(&mut self, frame: &RgbaImage) -> Result<Progress> {
        if frame.dimensions() != (self.width as u32, self.frame_height as u32) {
            return Err(failure("拼接长图", "选区尺寸发生变化"));
        }
        if self.is_full() {
            return Ok(Progress::LimitReached {
                height: self.height(),
            });
        }
        let features = row_features(frame.as_raw(), self.width);
        let previous = &self.rows[self.last_top..][..self.frame_height];
        // 局部光标闪烁或滚动条变化不应被当作滚动
        if feature_error(previous, &features) <= STATIONARY_ERROR
            && self.pixel_error(frame, self.last_top, self.frame_height) <= STATIONARY_ERROR
        {
            return Ok(Progress::Unchanged);
        }

        let current_height = self.rows.len();
        let min_overlap = MIN_OVERLAP.max(self.frame_height / 3);
        let earliest_top = current_height.saturating_sub(LOOKBACK_VIEWPORTS * self.frame_height);
        let latest_top = current_height - min_overlap;
        let mut best = (u32::MAX, 0);
        let mut runner_up = u32::MAX;
        for top in earliest_top..=latest_top {
            let overlap = self.frame_height.min(current_height - top);
            let score = feature_error(&self.rows[top..top + overlap], &features[..overlap]);
            if score < best.0 {
                runner_up = best.0;
                best = (score, top);
            } else if score < runner_up {
                runner_up = score;
            }
        }
        let (score, top) = best;
        let overlap = self.frame_height.min(current_height - top);
        // 保留一个独立的 RGB 检查，防止亮度特征的碰撞导致错接
        if score > MAX_FEATURE_ERROR
            || runner_up - score < MIN_SCORE_GAP
            || self.pixel_error(frame, top, overlap) > MAX_PIXEL_ERROR
        {
            return Ok(Progress::Unmatched);
        }
        if overlap == self.frame_height {
            self.last_top = top;
            return Ok(Progress::Unchanged);
        }

        let added = (self.frame_height - overlap).min(self.height_limit - current_height);
        let pitch = self.width * 4;
        self.pixels
            .extend_from_slice(&frame.as_raw()[overlap * pitch..(overlap + added) * pitch]);
        self.rows
            .extend_from_slice(&features[overlap..overlap + added]);
        if self.is_full() {
            return Ok(Progress::LimitReached {
                height: self.height(),
            });
        }
        self.last_top = top;
        Ok(Progress::Extended {
            height: self.height(),
        })
    }

    fn pixel_error(&self, frame: &RgbaImage, top: usize, overlap: usize) -> u32 {
        let rows = overlap.min(FEATURE_ROWS);
        let inset = self.width / 24;
        let span = self.width - 2 * inset;
        let columns = span.min(CHECK_COLUMNS);
        let pitch = self.width * 4;
        let mut total = 0;
        for row in 0..rows {
            let y = sample_at(row, overlap, rows);
            let above = &self.pixels[(top + y) * pitch..];
            let below = &frame.as_raw()[y * pitch..];
            for column in 0..columns {
                let pixel = (inset + sample_at(column, span, columns)) * 4;
                for channel in pixel..pixel + 3 {
                    total += u32::from(above[channel].abs_diff(below[channel]));
                }
            }
        }
        total * 256 / (rows * columns * 3) as u32
    }

    pub(super) fn finish(self) -> Result<RgbaImage> {
        RgbaImage::from_raw(self.width as u32, self.rows.len() as u32, self.pixels)
            .ok_or_else(|| failure("生成长图", "无效的像素缓冲区"))
    }
}

fn feature_error(above: &[RowFeature], below: &[RowFeature]) -> u32 {
    let rows = above.len().min(FEATURE_ROWS);
    let mut total = 0;
    for row in 0..rows {
        let y = sample_at(row, above.len(), rows);
        for (a, b) in above[y].iter().zip(below[y]) {
            total += u32::from(a.abs_diff(b));
        }
    }
    total * 256 / (rows * ROW_FEATURES) as u32
}

fn row_features(pixels: &[u8], width: usize) -> Vec<RowFeature> {
    let inset = width / 24;
    let span = width - 2 * inset;
    let taps = (span / ROW_FEATURES).clamp(1, 5);
    pixels
        .chunks_exact(width * 4)
        .map(|row| {
            std::array::from_fn(|column| {
                let mut luma = 0;
                for tap in 0..taps {
                    let x = inset + sample_at(column * taps + tap, span, ROW_FEATURES * taps);
                    let rgb = &row[x * 4..];
                    luma += 3 * usize::from(rgb[0]) + 6 * usize::from(rgb[1]) + usize::from(rgb[2]);
                }
                (luma / (10 * taps)) as u8
            })
        })
        .collect()
}

fn sample_at(index: usize, length: usize, samples: usize) -> usize {
    (2 * index + 1) * length / (2 * samples)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgba, imageops::crop_imm};

    fn document(height: u32, seed: u32) -> RgbaImage {
        RgbaImage::from_fn(80, height, |x, y| {
            let bits = (y * 83 + x + seed).wrapping_mul(0x45d9f3b).rotate_left(11);
            let bits = (bits ^ (bits >> 16)).wrapping_mul(0x45d9f3b).to_le_bytes();
            Rgba([bits[0], bits[1], bits[2], 255])
        })
    }

    fn viewport(page: &RgbaImage, top: u32) -> RgbaImage {
        crop_imm(page, 0, top, page.width(), 90).to_image()
    }

    #[test]
    fn appends_exact_new_rows_and_ignores_static_rollback_and_return() {
        let page = document(220, 0);
        let mut stitcher = Stitcher::new(viewport(&page, 0), 1000).unwrap();
        assert_eq!(
            stitcher.push(&viewport(&page, 0)).unwrap(),
            Progress::Unchanged
        );
        assert_eq!(
            stitcher.push(&viewport(&page, 27)).unwrap(),
            Progress::Extended { height: 117 }
        );
        assert_eq!(
            stitcher.push(&viewport(&page, 51)).unwrap(),
            Progress::Extended { height: 141 }
        );
        assert_eq!(
            stitcher.push(&viewport(&page, 19)).unwrap(),
            Progress::Unchanged
        );
        assert_eq!(
            stitcher.push(&viewport(&page, 51)).unwrap(),
            Progress::Unchanged
        );
        assert_eq!(
            stitcher.push(&viewport(&page, 52)).unwrap(),
            Progress::Extended { height: 142 }
        );
        assert_eq!(
            stitcher.finish().unwrap(),
            crop_imm(&page, 0, 0, 80, 142).to_image()
        );
    }

    #[test]
    fn skips_a_gap_and_can_resume_from_last_good_overlap() {
        let page = document(310, 0);
        let mut stitcher = Stitcher::new(viewport(&page, 0), 1000).unwrap();
        assert_eq!(
            stitcher.push(&viewport(&page, 150)).unwrap(),
            Progress::Unmatched
        );
        assert_eq!(stitcher.height(), 90);
        assert_eq!(
            stitcher.push(&viewport(&page, 35)).unwrap(),
            Progress::Extended { height: 125 }
        );
        assert_eq!(
            stitcher.finish().unwrap(),
            crop_imm(&page, 0, 0, 80, 125).to_image()
        );
    }

    #[test]
    fn refuses_ambiguous_repeating_rows() {
        let pattern = document(10, 0);
        let page = RgbaImage::from_fn(80, 150, |x, y| *pattern.get_pixel(x, y % 10));
        let mut stitcher = Stitcher::new(viewport(&page, 0), 1000).unwrap();
        assert_eq!(
            stitcher.push(&viewport(&page, 7)).unwrap(),
            Progress::Unmatched
        );
        assert_eq!(stitcher.height(), 90);
    }

    #[test]
    fn rejects_same_luminance_when_rgb_mismatch() {
        let signal = document(150, 0);
        let red = RgbaImage::from_fn(80, 150, |x, y| {
            let value = 40 + signal.get_pixel(x, y).0[0] % 80;
            Rgba([value * 2, 0, 0, 255])
        });
        let green = RgbaImage::from_fn(80, 150, |x, y| {
            Rgba([0, red.get_pixel(x, y).0[0] / 2, 0, 255])
        });
        let mut stitcher = Stitcher::new(viewport(&red, 0), 1000).unwrap();
        assert_eq!(
            stitcher.push(&viewport(&green, 30)).unwrap(),
            Progress::Unmatched
        );
        assert_eq!(stitcher.height(), 90);
    }

    #[test]
    fn clips_last_append_at_height_limit() {
        let page = document(150, 0);
        let mut stitcher = Stitcher::new(viewport(&page, 0), 96).unwrap();
        assert_eq!(
            stitcher.push(&viewport(&page, 20)).unwrap(),
            Progress::LimitReached { height: 96 }
        );
        assert_eq!(
            stitcher.finish().unwrap(),
            crop_imm(&page, 0, 0, 80, 96).to_image()
        );
        assert!(Stitcher::new(viewport(&page, 0), 89).is_err());
    }
}
