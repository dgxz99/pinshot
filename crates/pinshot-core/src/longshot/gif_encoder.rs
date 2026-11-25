use std::fs::File;
use std::io::{BufWriter, Seek, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use gif::{Encoder, Frame, Repeat};
use image::{RgbaImage, imageops};
use tempfile::NamedTempFile;

use super::failure;
use crate::capture::persist_unique_image;
use crate::{PinshotError, Result};

pub(super) const MAX_FILE_BYTES: usize = 64 * 1024 * 1024;
const MAX_PIXELS: u64 = 1280 * 720;
const MAX_EDGE: u32 = 1280;
// 部分播放器将小于 20 ms 的延迟按 100 ms 播放
const MIN_DELAY: u16 = 2;

/// 已完成的 GIF 临时文件，只有显式保存或达到体积上限后才发布
pub struct GifRecording {
    file: File,
    limit_reached: bool,
}

impl GifRecording {
    /// 是否因为文件大小上限而结束
    pub fn limit_reached(&self) -> bool {
        self.limit_reached
    }

    /// 复制到同目录临时文件后原子发布，同名时追加后缀
    pub fn save(mut self, directory: &Path, stem: &str) -> Result<PathBuf> {
        let mut temp =
            NamedTempFile::new_in(directory).map_err(|error| PinshotError::io(directory, error))?;
        self.file
            .rewind()
            .map_err(|error| PinshotError::io(directory, error))?;
        std::io::copy(&mut self.file, temp.as_file_mut())
            .map_err(|error| PinshotError::io(temp.path(), error))?;
        persist_unique_image(temp, directory, stem, "gif")
    }
}

/// 逐帧量化并写盘，只保留上一张已压缩的帧，等待计算其实际显示时长
pub(super) struct GifEncoder {
    encoder: Encoder<BufWriter<File>>,
    size: usize,
    max_bytes: usize,
    dimensions: (u16, u16),
    pending: Option<(Frame<'static>, u128)>,
    frames: usize,
    limit_reached: bool,
}

impl GifEncoder {
    pub(super) fn new(directory: &Path, width: u32, height: u32, max_bytes: usize) -> Result<Self> {
        let dimensions = gif_dimensions(width, height);
        // 录制中没有可见文件，取消或进程退出时由操作系统回收
        let output =
            tempfile::tempfile_in(directory).map_err(|error| PinshotError::io(directory, error))?;
        let mut encoder = Encoder::new(BufWriter::new(output), dimensions.0, dimensions.1, &[])
            .map_err(|error| failure("初始化 GIF 编码器", error))?;
        encoder
            .set_repeat(Repeat::Infinite)
            .map_err(|error| failure("设置 GIF 循环", error))?;
        Ok(Self {
            encoder,
            // 13 字节逻辑头、2 色全局色表、19 字节循环扩展、1 字节结束符
            size: 13 + 6 + 19 + 1,
            max_bytes,
            dimensions,
            pending: None,
            frames: 0,
            limit_reached: false,
        })
    }

    /// 包含保留帧的估算成品字节数，用于录制反馈
    pub(super) fn bytes(&self) -> usize {
        self.size
            + self
                .pending
                .as_ref()
                .map_or(0, |(frame, _)| frame_bytes(frame))
    }

    pub(super) fn push(&mut self, mut image: RgbaImage, elapsed: Duration) -> Result<bool> {
        let timestamp = gif_timestamp(elapsed);
        if let Some((frame, previous)) = self.pending.take() {
            self.write_frame(frame, timestamp.saturating_sub(previous))?;
            if self.limit_reached {
                return Ok(false);
            }
        }
        let (width, height) = self.dimensions;
        if image.dimensions() != (u32::from(width), u32::from(height)) {
            image = imageops::resize(
                &image,
                width.into(),
                height.into(),
                imageops::FilterType::Triangle,
            );
        }
        // 桌面录制是已合成的画面，不把 PipeWire 的 alpha 转成 GIF 透明像素
        for pixel in image.as_mut().as_chunks_mut::<4>().0 {
            pixel[3] = 255;
        }
        let mut frame = Frame::from_rgba_speed(width, height, image.as_mut(), 20);
        frame.make_lzw_pre_encoded();
        if self.size + frame_bytes(&frame) > self.max_bytes {
            if self.frames == 0 {
                return Err(failure("编码 GIF", "首帧超过文件大小上限"));
            }
            self.limit_reached = true;
            return Ok(false);
        }
        self.pending = Some((frame, timestamp));
        Ok(true)
    }

    fn write_frame(&mut self, mut frame: Frame<'static>, delay: u128) -> Result<()> {
        let mut delay = delay.max(u128::from(MIN_DELAY));
        let bytes = frame_bytes(&frame);
        while delay > 0 {
            if self.size + bytes > self.max_bytes {
                self.limit_reached = true;
                break;
            }
            // GIF 单帧延迟用 16 位百分秒表示；长间隔拆成多个同帧，而不是丢失时间
            frame.delay = delay.min(u128::from(u16::MAX)) as u16;
            self.encoder
                .write_lzw_pre_encoded_frame(&frame)
                .map_err(|error| failure("写入 GIF 帧", error))?;
            self.size += bytes;
            self.frames += 1;
            delay -= u128::from(frame.delay);
        }
        Ok(())
    }

    pub(super) fn finish(mut self, elapsed: Duration) -> Result<GifRecording> {
        if let Some((frame, timestamp)) = self.pending.take() {
            self.write_frame(frame, gif_timestamp(elapsed).saturating_sub(timestamp))?;
        }
        if self.frames == 0 {
            return Err(failure("完成 GIF", "还没有可保存的帧"));
        }
        let mut output = self
            .encoder
            .into_inner()
            .map_err(|error| failure("完成 GIF 编码", error))?;
        output
            .flush()
            .map_err(|error| failure("刷新 GIF 数据", error))?;
        let file = output.into_parts().0;
        Ok(GifRecording {
            file,
            limit_reached: self.limit_reached,
        })
    }
}

// 按绝对时间舍入，再计算相邻帧间隔，避免长录制时累计误差
fn gif_timestamp(elapsed: Duration) -> u128 {
    elapsed.as_millis().saturating_add(5) / 10
}

fn gif_dimensions(width: u32, height: u32) -> (u16, u16) {
    let edge = f64::from(width.max(height));
    let area = f64::from(width) * f64::from(height);
    let ratio = 1.0_f64
        .min(f64::from(MAX_EDGE) / edge)
        .min((MAX_PIXELS as f64 / area).sqrt());
    let scaled = |value| (f64::from(value) * ratio).floor().max(1.0) as u16;
    (scaled(width), scaled(height))
}

fn frame_bytes(frame: &Frame<'_>) -> usize {
    let palette = frame.palette.as_ref().map_or(0, |colors| {
        (colors.len() / 3).max(2).next_power_of_two() * 3
    });
    let payload = frame.buffer.len() - 1;
    // 8 字节控制扩展、10 字节画面描述、LZW 码宽、子块长度、终止字节
    8 + 10 + palette + 1 + payload + payload.div_ceil(255) + 1
}

#[cfg(test)]
mod tests {
    use super::*;
    use gif::{ColorOutput, DecodeOptions};
    use image::Rgba;

    #[test]
    fn saves_in_order_with_actual_timing_and_no_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let mut encoder = GifEncoder::new(dir.path(), 40, 32, MAX_FILE_BYTES).unwrap();
        for (time, color) in [
            (0, [241, 8, 5, 0]),
            (140, [8, 231, 5, 0]),
            (410, [2, 8, 225, 255]),
        ] {
            assert!(
                encoder
                    .push(
                        RgbaImage::from_pixel(40, 32, Rgba(color)),
                        Duration::from_millis(time)
                    )
                    .unwrap()
            );
        }
        let recording = encoder.finish(Duration::from_millis(530)).unwrap();
        std::fs::write(dir.path().join("clip.gif"), b"keep").unwrap();
        let path = recording.save(dir.path(), "clip").unwrap();
        assert_eq!(path.file_name().unwrap(), "clip-1.gif");
        assert_eq!(std::fs::read(dir.path().join("clip.gif")).unwrap(), b"keep");
        let mut options = DecodeOptions::new();
        options.set_color_output(ColorOutput::RGBA);
        let mut reader = options
            .read_info(std::fs::File::open(path).unwrap())
            .unwrap();
        for (delay, red, green, blue) in [(14, 241, 8, 5), (27, 8, 231, 5), (12, 2, 8, 225)] {
            let frame = reader.read_next_frame().unwrap().unwrap();
            assert_eq!(frame.delay, delay);
            assert_eq!(frame.buffer[..4], [red, green, blue, 255]);
        }
        assert!(reader.read_next_frame().unwrap().is_none());
    }

    #[test]
    fn stays_below_file_limit_and_leaves_a_decodable_gif() {
        let dir = tempfile::tempdir().unwrap();
        let mut sample = GifEncoder::new(dir.path(), 40, 32, MAX_FILE_BYTES).unwrap();
        let frame = || RgbaImage::from_pixel(40, 32, Rgba([200, 4, 6, 255]));
        assert!(sample.push(frame(), Duration::ZERO).unwrap());
        let limit = sample.bytes();
        drop(sample);
        let mut encoder = GifEncoder::new(dir.path(), 40, 32, limit).unwrap();
        assert!(encoder.push(frame(), Duration::ZERO).unwrap());
        assert!(!encoder.push(frame(), Duration::from_millis(100)).unwrap());
        let recording = encoder.finish(Duration::from_millis(200)).unwrap();
        assert!(recording.limit_reached());
        let path = recording.save(dir.path(), "cap").unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().len() as usize, limit);
        let mut reader = gif::Decoder::new(std::fs::File::open(path).unwrap()).unwrap();
        assert!(reader.read_next_frame().unwrap().is_some());
        assert!(reader.read_next_frame().unwrap().is_none());
    }

    #[test]
    fn represents_long_static_intervals_without_overflowing_gif_frame_delay() {
        let dir = tempfile::tempdir().unwrap();
        let mut encoder = GifEncoder::new(dir.path(), 32, 32, MAX_FILE_BYTES).unwrap();
        assert!(
            encoder
                .push(
                    RgbaImage::from_pixel(32, 32, Rgba([9, 20, 30, 255])),
                    Duration::ZERO
                )
                .unwrap()
        );
        let path = encoder
            .finish(Duration::from_secs(660))
            .unwrap()
            .save(dir.path(), "static")
            .unwrap();
        let mut reader = gif::Decoder::new(File::open(path).unwrap()).unwrap();
        assert_eq!(reader.read_next_frame().unwrap().unwrap().delay, u16::MAX);
        assert_eq!(reader.read_next_frame().unwrap().unwrap().delay, 465);
        assert!(reader.read_next_frame().unwrap().is_none());
    }

    #[test]
    fn bounds_large_frames_and_removes_cancelled_recordings() {
        assert_eq!(gif_dimensions(3840, 2160), (1280, 720));
        assert_eq!(gif_dimensions(400, 400), (400, 400));
        assert_eq!(gif_dimensions(1000, 16000), (80, 1280));
        let dir = tempfile::tempdir().unwrap();
        let encoder = GifEncoder::new(dir.path(), 32, 32, MAX_FILE_BYTES).unwrap();
        assert_eq!(dir.path().read_dir().unwrap().count(), 0);
        drop(encoder);
        assert_eq!(dir.path().read_dir().unwrap().count(), 0);
    }
}
