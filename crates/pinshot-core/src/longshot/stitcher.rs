use std::collections::VecDeque;

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

/// 一帧处理后的状态，长度是当前拼接轴上的像素数
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Progress {
    Started { length: u32 },
    Extended { length: u32 },
    Unchanged,
    Unmatched,
    LimitReached { length: u32 },
}

#[derive(Clone, Copy)]
struct Overlap {
    canvas_start: usize,
    frame_start: usize,
    len: usize,
}

impl Overlap {
    fn at(top: isize, canvas_height: usize, frame_height: usize) -> Self {
        let canvas_start = top.max(0) as usize;
        let frame_start = (-top).max(0) as usize;
        let len = (canvas_height - canvas_start).min(frame_height - frame_start);
        Self {
            canvas_start,
            frame_start,
            len,
        }
    }
}

/// 只在已验证重叠的前后两端补齐新出现的像素行
pub(super) struct Stitcher {
    pixels: VecDeque<u8>,
    rows: VecDeque<RowFeature>,
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
        let rows = row_features(first.as_raw(), width).into();
        let mut pixels = VecDeque::from(first.into_raw());
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
                length: self.height(),
            });
        }
        let features = row_features(frame.as_raw(), self.width);
        let current_height = self.rows.len();
        let last_top = self.last_top as isize;
        let previous = Overlap::at(last_top, current_height, self.frame_height);
        // 局部光标闪烁或滚动条变化不应被当作滚动
        if self.feature_error(&features, previous) <= STATIONARY_ERROR
            && self.pixel_error(frame, previous) <= STATIONARY_ERROR
        {
            return Ok(Progress::Unchanged);
        }

        let min_overlap = MIN_OVERLAP.max(self.frame_height / 3);
        let lookaround = (LOOKBACK_VIEWPORTS * self.frame_height) as isize;
        let earliest_top =
            (last_top - lookaround).max(-((self.frame_height - min_overlap) as isize));
        let latest_top = (last_top + lookaround).min((current_height - min_overlap) as isize);
        let mut best = (u32::MAX, last_top);
        let mut runner_up = u32::MAX;
        for top in earliest_top..=latest_top {
            let overlap = Overlap::at(top, current_height, self.frame_height);
            let score = self.feature_error(&features, overlap);
            if score < best.0 {
                runner_up = best.0;
                best = (score, top);
            } else if score < runner_up {
                runner_up = score;
            }
        }
        let (score, top) = best;
        let overlap = Overlap::at(top, current_height, self.frame_height);
        // 保留一个独立的 RGB 检查，防止亮度特征的碰撞导致错接
        if score > MAX_FEATURE_ERROR
            || runner_up - score < MIN_SCORE_GAP
            || self.pixel_error(frame, overlap) > MAX_PIXEL_ERROR
        {
            return Ok(Progress::Unmatched);
        }
        if overlap.len == self.frame_height {
            self.last_top = overlap.canvas_start;
            return Ok(Progress::Unchanged);
        }

        self.extend(frame, &features, overlap);
        if self.is_full() {
            return Ok(Progress::LimitReached {
                length: self.height(),
            });
        }
        self.last_top = overlap.canvas_start;
        Ok(Progress::Extended {
            length: self.height(),
        })
    }

    fn extend(&mut self, frame: &RgbaImage, features: &[RowFeature], overlap: Overlap) {
        let remaining = self.height_limit - self.rows.len();
        let pitch = self.width * 4;
        if overlap.frame_start > 0 {
            // 达到上限时保留紧邻旧画布的行，不留下缺口
            let end = overlap.frame_start;
            let start = end.saturating_sub(remaining);
            for &byte in frame.as_raw()[start * pitch..end * pitch].iter().rev() {
                self.pixels.push_front(byte);
            }
            for &row in features[start..end].iter().rev() {
                self.rows.push_front(row);
            }
        } else {
            let start = overlap.len;
            let end = self.frame_height.min(start + remaining);
            self.pixels
                .extend(&frame.as_raw()[start * pitch..end * pitch]);
            self.rows.extend(&features[start..end]);
        }
    }

    fn feature_error(&self, features: &[RowFeature], overlap: Overlap) -> u32 {
        let rows = overlap.len.min(FEATURE_ROWS);
        let mut total = 0;
        for row in 0..rows {
            let y = sample_at(row, overlap.len, rows);
            let above = &self.rows[overlap.canvas_start + y];
            let below = &features[overlap.frame_start + y];
            for (a, b) in above.iter().zip(below) {
                total += u32::from(a.abs_diff(*b));
            }
        }
        total * 256 / (rows * ROW_FEATURES) as u32
    }

    fn pixel_error(&self, frame: &RgbaImage, overlap: Overlap) -> u32 {
        let rows = overlap.len.min(FEATURE_ROWS);
        let inset = self.width / 24;
        let span = self.width - 2 * inset;
        let columns = span.min(CHECK_COLUMNS);
        let pitch = self.width * 4;
        let mut total = 0;
        for row in 0..rows {
            let y = sample_at(row, overlap.len, rows);
            let above = (overlap.canvas_start + y) * pitch;
            let below = &frame.as_raw()[(overlap.frame_start + y) * pitch..];
            for column in 0..columns {
                let pixel = (inset + sample_at(column, span, columns)) * 4;
                for (channel, &value) in below[pixel..pixel + 3].iter().enumerate() {
                    total += u32::from(self.pixels[above + pixel + channel].abs_diff(value));
                }
            }
        }
        total * 256 / (rows * columns * 3) as u32
    }

    pub(super) fn finish(self) -> Result<RgbaImage> {
        RgbaImage::from_raw(
            self.width as u32,
            self.rows.len() as u32,
            self.pixels.into(),
        )
        .ok_or_else(|| failure("生成长图", "无效的像素缓冲区"))
    }
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
            Progress::Extended { length: 117 }
        );
        assert_eq!(
            stitcher.push(&viewport(&page, 51)).unwrap(),
            Progress::Extended { length: 141 }
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
            Progress::Extended { length: 142 }
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
            Progress::Extended { length: 125 }
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
    fn extends_both_ends_and_traverses_a_long_existing_canvas_without_duplicates() {
        let page = document(640, 0);
        let mut stitcher = Stitcher::new(viewport(&page, 180), 1000).unwrap();
        for top in [215, 255, 295, 335, 375, 415, 455] {
            assert_eq!(
                stitcher.push(&viewport(&page, top)).unwrap(),
                Progress::Extended {
                    length: top + 90 - 180
                }
            );
        }
        for top in [415, 375, 335, 295, 255, 215, 180] {
            assert_eq!(
                stitcher.push(&viewport(&page, top)).unwrap(),
                Progress::Unchanged
            );
        }
        for top in [145, 105, 65] {
            assert_eq!(
                stitcher.push(&viewport(&page, top)).unwrap(),
                Progress::Extended { length: 545 - top }
            );
        }
        assert_eq!(
            stitcher.finish().unwrap(),
            crop_imm(&page, 0, 65, 80, 480).to_image()
        );
    }

    #[test]
    fn clips_last_prepend_without_a_gap_at_height_limit() {
        let page = document(150, 0);
        let mut stitcher = Stitcher::new(viewport(&page, 40), 96).unwrap();
        assert_eq!(
            stitcher.push(&viewport(&page, 20)).unwrap(),
            Progress::LimitReached { length: 96 }
        );
        assert_eq!(
            stitcher.finish().unwrap(),
            crop_imm(&page, 0, 34, 80, 96).to_image()
        );
    }

    #[test]
    fn clips_last_append_at_height_limit() {
        let page = document(150, 0);
        let mut stitcher = Stitcher::new(viewport(&page, 0), 96).unwrap();
        assert_eq!(
            stitcher.push(&viewport(&page, 20)).unwrap(),
            Progress::LimitReached { length: 96 }
        );
        assert_eq!(
            stitcher.finish().unwrap(),
            crop_imm(&page, 0, 0, 80, 96).to_image()
        );
        assert!(Stitcher::new(viewport(&page, 0), 89).is_err());
    }
}
