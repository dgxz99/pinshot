use std::collections::VecDeque;
use std::ops::Range;

use image::RgbaImage;

use super::failure;
use super::viewport::{FixedBands, side_inset};
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

struct FrameRows<'a> {
    pixels: &'a [u8],
    features: &'a [RowFeature],
}

impl<'a> FrameRows<'a> {
    fn body(&self, range: Range<usize>, pitch: usize) -> Self {
        Self {
            pixels: &self.pixels[range.start * pitch..range.end * pitch],
            features: &self.features[range],
        }
    }
}

struct FixedLayout {
    bands: FixedBands,
    leading: Vec<u8>,
    trailing: Vec<u8>,
}

/// 只在已验证重叠的前后两端补齐新出现的像素行
pub(super) struct Stitcher {
    pixels: VecDeque<u8>,
    rows: VecDeque<RowFeature>,
    width: usize,
    frame_height: usize,
    height_limit: usize,
    last_top: usize,
    fixed: Option<FixedLayout>,
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
            fixed: None,
        })
    }

    fn output_len(&self) -> usize {
        self.rows.len()
            + self
                .fixed
                .as_ref()
                .map_or(0, |layout| layout.bands.length())
    }

    pub(super) fn height(&self) -> u32 {
        self.output_len() as u32
    }

    pub(super) fn is_full(&self) -> bool {
        self.output_len() == self.height_limit
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
        let frame = FrameRows {
            pixels: frame.as_raw(),
            features: &features,
        };
        let mut canvas = 0..self.rows.len();
        let bands = if let Some(layout) = &self.fixed {
            layout.bands
        } else {
            if self.is_stationary(&frame, Overlap::at(0, self.frame_height, self.frame_height)) {
                return Ok(Progress::Unchanged);
            }
            let bands = FixedBands::detect(
                self.pixels.make_contiguous(),
                frame.pixels,
                self.width,
                self.frame_height,
            );
            if self.frame_height - bands.length() < MIN_EDGE {
                return Ok(Progress::Unmatched);
            }
            canvas = bands.body(self.frame_height);
            bands
        };
        let body = frame.body(bands.body(self.frame_height), self.width * 4);
        let Some(overlap) = self.find_overlap(&body, canvas) else {
            return Ok(Progress::Unmatched);
        };
        if overlap.len == body.features.len() {
            self.last_top = overlap.canvas_start;
            return Ok(Progress::Unchanged);
        }

        // 只有正文的位移得到验证后才把推断出的固定栏从首帧分离
        if self.fixed.is_none() {
            self.lock_layout(bands);
        }
        self.extend(&body, frame.pixels, overlap);
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

    fn find_overlap(&self, frame: &FrameRows<'_>, canvas: Range<usize>) -> Option<Overlap> {
        let frame_height = frame.features.len();
        let current_height = canvas.len();
        let last_top = self.last_top as isize;
        let overlap_at = |top| Overlap::at(top, current_height, frame_height);
        let in_canvas = |mut overlap: Overlap| {
            overlap.canvas_start += canvas.start;
            overlap
        };
        let previous = overlap_at(last_top);
        if self.is_stationary(frame, in_canvas(previous)) {
            return Some(previous);
        }

        let min_overlap = MIN_OVERLAP.max(frame_height / 3);
        let lookaround = (LOOKBACK_VIEWPORTS * frame_height) as isize;
        let earliest_top = (last_top - lookaround).max(-((frame_height - min_overlap) as isize));
        let latest_top = (last_top + lookaround).min((current_height - min_overlap) as isize);
        let mut best = (u32::MAX, last_top);
        let mut runner_up = u32::MAX;
        for top in earliest_top..=latest_top {
            let overlap = in_canvas(overlap_at(top));
            let score = self.feature_error(frame.features, overlap);
            if score < best.0 {
                runner_up = best.0;
                best = (score, top);
            } else if score < runner_up {
                runner_up = score;
            }
        }
        let (score, top) = best;
        let overlap = overlap_at(top);
        // 保留一个独立的 RGB 检查，防止亮度特征的碰撞导致错接
        if score > MAX_FEATURE_ERROR
            || runner_up - score < MIN_SCORE_GAP
            || self.pixel_error(frame.pixels, in_canvas(overlap)) > MAX_PIXEL_ERROR
        {
            return None;
        }
        Some(overlap)
    }

    fn is_stationary(&self, frame: &FrameRows<'_>, overlap: Overlap) -> bool {
        self.feature_error(frame.features, overlap) <= STATIONARY_ERROR
            && self.pixel_error(frame.pixels, overlap) <= STATIONARY_ERROR
    }

    fn lock_layout(&mut self, bands: FixedBands) {
        let body = bands.body(self.frame_height);
        let pitch = self.width * 4;
        let leading = self.pixels.drain(..body.start * pitch).collect();
        let trailing = self.pixels.drain(body.len() * pitch..).collect();
        self.rows.truncate(body.end);
        self.rows.drain(..body.start);
        self.fixed = Some(FixedLayout {
            bands,
            leading,
            trailing,
        });
    }

    fn extend(&mut self, frame: &FrameRows<'_>, full_frame: &[u8], overlap: Overlap) {
        let remaining = self.height_limit - self.output_len();
        let pitch = self.width * 4;
        if overlap.frame_start > 0 {
            // 达到上限时保留紧邻旧画布的行，不留下缺口
            let end = overlap.frame_start;
            let start = end.saturating_sub(remaining);
            for &byte in frame.pixels[start * pitch..end * pitch].iter().rev() {
                self.pixels.push_front(byte);
            }
            for &row in frame.features[start..end].iter().rev() {
                self.rows.push_front(row);
            }
            // 固定栏可能连带少量碰巧相同的正文留白，使用画布最外侧的帧以保留这里的内容
            if start == 0
                && let Some(layout) = &mut self.fixed
            {
                let end = layout.leading.len();
                layout.leading.copy_from_slice(&full_frame[..end]);
            }
        } else {
            let start = overlap.len;
            let end = frame.features.len().min(start + remaining);
            self.pixels
                .extend(&frame.pixels[start * pitch..end * pitch]);
            self.rows.extend(&frame.features[start..end]);
            if end == frame.features.len()
                && let Some(layout) = &mut self.fixed
            {
                let start = full_frame.len() - layout.trailing.len();
                layout.trailing.copy_from_slice(&full_frame[start..]);
            }
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

    fn pixel_error(&self, pixels: &[u8], overlap: Overlap) -> u32 {
        let rows = overlap.len.min(FEATURE_ROWS);
        let inset = side_inset(self.width);
        let span = self.width - 2 * inset;
        let columns = span.min(CHECK_COLUMNS);
        let pitch = self.width * 4;
        let mut total = 0;
        for row in 0..rows {
            let y = sample_at(row, overlap.len, rows);
            let above = (overlap.canvas_start + y) * pitch;
            let below = &pixels[(overlap.frame_start + y) * pitch..];
            for column in 0..columns {
                let pixel = (inset + sample_at(column, span, columns)) * 4;
                for (channel, &value) in below[pixel..pixel + 3].iter().enumerate() {
                    total += u32::from(self.pixels[above + pixel + channel].abs_diff(value));
                }
            }
        }
        total * 256 / (rows * columns * 3) as u32
    }

    pub(super) fn finish(mut self) -> Result<RgbaImage> {
        let height = self.height();
        if let Some(layout) = self.fixed {
            for byte in layout.leading.into_iter().rev() {
                self.pixels.push_front(byte);
            }
            self.pixels.extend(layout.trailing);
        }
        RgbaImage::from_raw(self.width as u32, height, self.pixels.into())
            .ok_or_else(|| failure("生成长图", "无效的像素缓冲区"))
    }
}

fn row_features(pixels: &[u8], width: usize) -> Vec<RowFeature> {
    let inset = side_inset(width);
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

    const HEADER_HEIGHT: u32 = 22;
    const FOOTER_HEIGHT: u32 = 14;
    const CODE_VIEW_HEIGHT: u32 = 96;
    const CARD_HEIGHT: u32 = HEADER_HEIGHT + CODE_VIEW_HEIGHT + FOOTER_HEIGHT;

    fn code_card(page: &RgbaImage, top: u32, show_scrollbar: bool) -> RgbaImage {
        RgbaImage::from_fn(page.width(), CARD_HEIGHT, |x, y| {
            if y < HEADER_HEIGHT {
                // 不随正文滚动的语言标签与工具按钮
                let label = (7..28).contains(&x) && (6..14).contains(&y) && y % 3 != 0;
                return if label {
                    Rgba([210, 210, 215, 255])
                } else {
                    Rgba([58, 58, 62, 255])
                };
            }
            let body_y = y - HEADER_HEIGHT;
            if body_y >= CODE_VIEW_HEIGHT {
                return Rgba([28, 28, 32, 255]);
            }
            if show_scrollbar && x >= page.width() - 2 {
                let offset = top / 5;
                return if (offset..offset + 12).contains(&body_y) {
                    Rgba([168, 168, 169, 255])
                } else {
                    Rgba([43, 43, 46, 255])
                };
            }
            *page.get_pixel(x, top + body_y)
        })
    }

    #[test]
    fn scrolls_inside_a_card_and_keeps_the_fixed_header_and_footer_only_once() {
        let page = document(300, 13);
        let first = code_card(&page, 70, true);
        let mut stitcher = Stitcher::new(first.clone(), 1000).unwrap();
        assert_eq!(stitcher.push(&first).unwrap(), Progress::Unchanged);
        assert_eq!(
            stitcher.push(&code_card(&page, 103, true)).unwrap(),
            Progress::Extended {
                length: CARD_HEIGHT + 33
            }
        );
        assert_eq!(
            stitcher.push(&code_card(&page, 70, true)).unwrap(),
            Progress::Unchanged
        );
        assert_eq!(
            stitcher.push(&code_card(&page, 35, true)).unwrap(),
            Progress::Extended {
                length: CARD_HEIGHT + 68
            }
        );
        assert_eq!(
            stitcher.push(&code_card(&page, 65, true)).unwrap(),
            Progress::Unchanged
        );
        let output = stitcher.finish().unwrap();
        let width = page.width();
        let body_height = CODE_VIEW_HEIGHT + 68;
        assert_eq!(
            output.dimensions(),
            (width, body_height + HEADER_HEIGHT + FOOTER_HEIGHT)
        );
        assert_eq!(
            crop_imm(&output, 0, 0, width, HEADER_HEIGHT).to_image(),
            crop_imm(&first, 0, 0, width, HEADER_HEIGHT).to_image()
        );
        // 滚动条不参与匹配；验证正文每个像素，且标题栏与底边只在原位出现一次
        assert_eq!(
            crop_imm(&output, 0, HEADER_HEIGHT, width - 2, body_height).to_image(),
            crop_imm(&page, 0, 35, width - 2, body_height).to_image()
        );
        assert_eq!(
            crop_imm(
                &output,
                0,
                HEADER_HEIGHT + body_height,
                width,
                FOOTER_HEIGHT
            )
            .to_image(),
            crop_imm(&first, 0, CARD_HEIGHT - FOOTER_HEIGHT, width, FOOTER_HEIGHT).to_image()
        );
    }

    #[test]
    fn preserves_content_next_to_a_header_when_initial_frames_share_a_blank_line() {
        let mut page = document(270, 17);
        // 首次两帧紧靠标题栏的位置正好都是相同的行间留白
        for y in [70, 71, 104, 105] {
            for x in 0..page.width() {
                page.put_pixel(x, y, Rgba([58, 58, 62, 255]));
            }
        }
        let mut stitcher = Stitcher::new(code_card(&page, 70, false), 1000).unwrap();
        assert_eq!(
            stitcher.push(&code_card(&page, 104, false)).unwrap(),
            Progress::Extended {
                length: CARD_HEIGHT + 34
            }
        );
        assert_eq!(
            stitcher.push(&code_card(&page, 70, false)).unwrap(),
            Progress::Unchanged
        );
        assert_eq!(
            stitcher.push(&code_card(&page, 35, false)).unwrap(),
            Progress::Extended {
                length: CARD_HEIGHT + 69
            }
        );
        let output = stitcher.finish().unwrap();
        let actual = crop_imm(
            &output,
            0,
            HEADER_HEIGHT,
            page.width(),
            CODE_VIEW_HEIGHT + 69,
        )
        .to_image();
        assert_eq!(
            actual,
            crop_imm(&page, 0, 35, page.width(), CODE_VIEW_HEIGHT + 69).to_image()
        );
    }

    #[test]
    fn rounded_container_edges_do_not_repeat_in_the_stitched_body() {
        let texture = document(300, 27);
        let background = Rgba([64, 64, 64, 255]);
        let outside = Rgba([17, 17, 17, 255]);
        let page = RgbaImage::from_fn(384, 300, |x, y| {
            if (90..270).contains(&x) {
                *texture.get_pixel(x % texture.width(), y)
            } else {
                background
            }
        });
        let render = |top: u32, height: u32| {
            RgbaImage::from_fn(page.width(), height, |x, y| {
                // 圆角全部落在匹配时忽略的侧边区域，不能靠正文的色差找出
                let edge_y = y.min(height - 1 - y);
                let curve = 12_u32.saturating_sub((18 * edge_y).isqrt());
                let distance = x.min(page.width() - 1 - x);
                if distance < 2 + curve {
                    outside
                } else {
                    *page.get_pixel(x, top + y)
                }
            })
        };
        let mut stitcher = Stitcher::new(render(60, 140), 1000).unwrap();
        assert_eq!(
            stitcher.push(&render(90, 140)).unwrap(),
            Progress::Extended { length: 170 }
        );
        assert_eq!(
            stitcher.push(&render(60, 140)).unwrap(),
            Progress::Unchanged
        );
        assert_eq!(
            stitcher.push(&render(31, 140)).unwrap(),
            Progress::Extended { length: 199 }
        );
        let output = stitcher.finish().unwrap();
        let expected = render(31, 199);
        assert_eq!(output.dimensions(), expected.dimensions());
        assert!(
            output.as_raw() == expected.as_raw(),
            "圆角只能出现在完整长图的两端"
        );
    }

    #[test]
    fn does_not_use_a_fixed_header_as_overlap_when_the_body_jumps() {
        let page = document(310, 7);
        let first = code_card(&page, 0, true);
        let mut stitcher = Stitcher::new(first.clone(), 1000).unwrap();
        assert_eq!(
            stitcher.push(&code_card(&page, 167, true)).unwrap(),
            Progress::Unmatched
        );
        assert_eq!(stitcher.height(), CARD_HEIGHT);
        assert_eq!(stitcher.finish().unwrap(), first);
    }

    #[test]
    fn output_limit_still_includes_the_fixed_bands() {
        let page = document(200, 13);
        let first = code_card(&page, 0, false);
        let mut stitcher = Stitcher::new(first.clone(), CARD_HEIGHT + 6).unwrap();
        assert_eq!(
            stitcher.push(&code_card(&page, 25, false)).unwrap(),
            Progress::LimitReached {
                length: CARD_HEIGHT + 6
            }
        );
        let output = stitcher.finish().unwrap();
        assert_eq!(
            crop_imm(
                &output,
                0,
                HEADER_HEIGHT,
                page.width(),
                CODE_VIEW_HEIGHT + 6
            )
            .to_image(),
            crop_imm(&page, 0, 0, page.width(), CODE_VIEW_HEIGHT + 6).to_image()
        );
        assert_eq!(
            crop_imm(
                &output,
                0,
                CARD_HEIGHT + 6 - FOOTER_HEIGHT,
                page.width(),
                FOOTER_HEIGHT
            )
            .to_image(),
            crop_imm(
                &first,
                0,
                CARD_HEIGHT - FOOTER_HEIGHT,
                page.width(),
                FOOTER_HEIGHT
            )
            .to_image()
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
