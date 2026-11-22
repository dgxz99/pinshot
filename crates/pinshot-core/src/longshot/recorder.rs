use std::io::Write;
use std::ops::ControlFlow;
use std::path::Path;
use std::time::Duration;

use image::{ExtendedColorType, ImageEncoder, RgbaImage};

use super::region::{LogicalRegion, PixelRegion};
use super::stitcher::{Progress, Stitcher};
use super::{Monitor, failure, run_capture};
use crate::{PinshotError, Result};

const MAX_HEIGHT: u32 = 24_000;

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
    duration: Duration,
    mut update: impl FnMut(Progress),
) -> Result<Option<Recording>> {
    run_capture(async |_, screen| {
        let mut recorder = Recorder::new(screen.monitor, region);
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
    stitcher: Option<Stitcher>,
    source_scale: f64,
}

impl Recorder {
    pub(super) fn new(monitor: Monitor, region: Option<LogicalRegion>) -> Self {
        Self {
            monitor,
            region,
            crop: None,
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
        if let Some(stitcher) = &mut self.stitcher {
            return stitcher.push(&frame);
        }
        let first = Stitcher::new(frame, MAX_HEIGHT)?;
        let height = first.height();
        let progress = if first.is_full() {
            Progress::LimitReached { height }
        } else {
            Progress::Started { height }
        };
        self.stitcher = Some(first);
        Ok(progress)
    }

    pub(super) fn finish(self) -> Result<Recording> {
        let stitcher = self
            .stitcher
            .ok_or_else(|| failure("生成长图", "没有接收到首帧"))?;
        Ok(Recording {
            image: stitcher.finish()?,
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
