use std::io::Write;
use std::ops::ControlFlow;
use std::path::Path;
use std::time::Duration;

use image::{ExtendedColorType, ImageEncoder, RgbaImage};

use super::region::{LogicalRegion, PixelRegion};
use super::stitcher::{Progress, Stitcher};
use super::{Monitor, failure, run_capture};
use crate::capture::ScrollAxis;
use crate::{PinshotError, Result};

const MAX_LENGTH: u32 = 24_000;

fn orient_for_stitching(frame: RgbaImage, axis: ScrollAxis) -> RgbaImage {
    match axis {
        ScrollAxis::Vertical => frame,
        ScrollAxis::Horizontal => {
            RgbaImage::from_fn(frame.height(), frame.width(), |x, y| *frame.get_pixel(y, x))
        }
    }
}

/// 已结束的滚动录制，包含最终拼接像素和显示器缩放比例
pub struct Recording {
    image: RgbaImage,
    pub source_scale: f64,
}

impl Recording {
    pub fn dimensions(&self) -> (u32, u32) {
        self.image.dimensions()
    }

    /// 编码缓存与剪贴板共用的无损 PNG
    pub fn png_bytes(&self) -> Result<Vec<u8>> {
        let mut bytes = Vec::new();
        self.write_png(&mut bytes)?;
        Ok(bytes)
    }

    /// 将成品编码为 PNG，再提交到目标路径；拒绝覆盖已有文件
    pub fn save_png(&self, path: &Path) -> Result<()> {
        let dir = path
            .parent()
            .filter(|dir| !dir.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let mut temp =
            tempfile::NamedTempFile::new_in(dir).map_err(|error| PinshotError::io(dir, error))?;
        self.write_png(temp.as_file_mut())?;
        temp.persist_noclobber(path)
            .map_err(|error| PinshotError::io(path, error.error))?;
        Ok(())
    }

    fn write_png(&self, writer: impl Write) -> Result<()> {
        let (width, height) = self.dimensions();
        image::codecs::png::PngEncoder::new(writer).write_image(
            self.image.as_raw(),
            width,
            height,
            ExtendedColorType::Rgba8,
        )?;
        Ok(())
    }
}

/// 共享单个屏幕，从首帧起定时采集；取消或共享被撤销时丢弃中间画布
pub fn record(
    region: Option<LogicalRegion>,
    axis: ScrollAxis,
    duration: Duration,
    mut update: impl FnMut(Progress),
) -> Result<Option<Recording>> {
    run_capture(None, async |_, screen| {
        let mut recorder = Recorder::new(screen.monitor, region, axis);
        screen
            .read(Some(duration), |frame| {
                let progress = recorder.push(frame)?;
                update(progress);
                Ok(progress.flow())
            })
            .await?;
        recorder.finish().map(Some)
    })
}

/// 将协商出的帧映射到一个固定选区，并维护该次录制的拼接画布
pub(super) struct Recorder {
    monitor: Monitor,
    region: Option<LogicalRegion>,
    crop: Option<PixelRegion>,
    axis: ScrollAxis,
    stitcher: Option<Stitcher>,
    source_scale: f64,
}

impl Recorder {
    pub(super) fn new(monitor: Monitor, region: Option<LogicalRegion>, axis: ScrollAxis) -> Self {
        Self {
            monitor,
            region,
            crop: None,
            axis,
            stitcher: None,
            source_scale: 1.0,
        }
    }

    pub(super) fn push(&mut self, frame: RgbaImage) -> Result<Progress> {
        if self.stitcher.is_none() {
            if let Some(region) = self.region {
                self.crop = Some(PixelRegion::from_logical(
                    region,
                    self.monitor.logical_size,
                    frame.dimensions(),
                )?);
            }
            if let Some((logical_width, _)) =
                self.monitor.logical_size.filter(|&(width, _)| width > 0)
            {
                self.source_scale = f64::from(frame.width()) / f64::from(logical_width);
            }
        }
        let frame = if let Some(crop) = self.crop {
            crop.crop(frame)
        } else {
            frame
        };
        let frame = orient_for_stitching(frame, self.axis);
        if let Some(stitcher) = &mut self.stitcher {
            return stitcher.push(&frame);
        }
        let first = Stitcher::new(frame, MAX_LENGTH)?;
        let length = first.height();
        let progress = if first.is_full() {
            Progress::LimitReached { length }
        } else {
            Progress::Started { length }
        };
        self.stitcher = Some(first);
        Ok(progress)
    }

    pub(super) fn finish(self) -> Result<Recording> {
        let stitcher = self
            .stitcher
            .ok_or_else(|| failure("生成长图", "没有接收到首帧"))?;
        Ok(Recording {
            // 转置是自身的逆操作，横向成品恢复正常阅读方向
            image: orient_for_stitching(stitcher.finish()?, self.axis),
            source_scale: self.source_scale,
        })
    }
}

impl Progress {
    pub(super) fn flow(self) -> ControlFlow<()> {
        if matches!(self, Self::LimitReached { .. }) {
            return ControlFlow::Break(());
        }
        ControlFlow::Continue(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    #[test]
    fn horizontal_recording_extends_both_sides_and_restores_image_orientation() {
        let page = RgbaImage::from_fn(310, 95, |x, y| {
            let bytes = (x * 97 + y)
                .wrapping_mul(0x45d9f3b)
                .rotate_left(11)
                .to_le_bytes();
            Rgba([bytes[0], bytes[1], bytes[2], 255])
        });
        let mut recorder = Recorder::new(
            Monitor {
                logical_size: Some((120, 95)),
                logical_position: None,
            },
            Some(LogicalRegion {
                x: 10,
                y: 5,
                width: 90,
                height: 80,
            }),
            ScrollAxis::Horizontal,
        );
        for (left, expected) in [
            (100, Progress::Started { length: 90 }),
            (142, Progress::Extended { length: 132 }),
            (100, Progress::Unchanged),
            (62, Progress::Extended { length: 170 }),
            (26, Progress::Extended { length: 206 }),
        ] {
            let frame = image::imageops::crop_imm(&page, left, 0, 120, 95).to_image();
            assert_eq!(recorder.push(frame).unwrap(), expected);
        }
        let recording = recorder.finish().unwrap();
        assert_eq!(
            recording.image,
            image::imageops::crop_imm(&page, 36, 5, 206, 80).to_image()
        );
    }

    #[test]
    fn writes_png_without_overwriting_existing_output() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("长图.png");
        let frame = RgbaImage::from_pixel(32, 70, Rgba([20, 40, 60, 255]));
        let original = Recording {
            image: frame.clone(),
            source_scale: 1.0,
        };
        original.save_png(&path).unwrap();
        let different = Recording {
            image: RgbaImage::from_pixel(20, 40, Rgba([1, 2, 3, 255])),
            source_scale: 1.0,
        };
        assert!(different.save_png(&path).is_err());
        assert_eq!(image::open(path).unwrap().into_rgba8(), frame);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }
}
