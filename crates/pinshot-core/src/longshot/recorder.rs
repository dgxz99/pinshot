use std::ops::ControlFlow;
use std::path::Path;
use std::time::Duration;

use image::{ExtendedColorType, ImageEncoder, RgbaImage};

use super::region::{LogicalRegion, PixelRegion};
use super::stitcher::{Progress, Stitcher};
use super::{failure, run_capture};
use crate::{PinshotError, Result};

const MAX_HEIGHT: u32 = 24_000;

/// 已结束的滚动录制，仅持有最终拼接图像
pub struct Recording(RgbaImage);

impl Recording {
    pub fn dimensions(&self) -> (u32, u32) {
        self.0.dimensions()
    }

    /// 将成品编码为 PNG，再提交到目标路径；拒绝覆盖已有文件
    pub fn save_png(&self, path: &Path) -> Result<()> {
        let dir = path
            .parent()
            .filter(|dir| !dir.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let mut temp =
            tempfile::NamedTempFile::new_in(dir).map_err(|error| PinshotError::io(dir, error))?;
        let (width, height) = self.dimensions();
        image::codecs::png::PngEncoder::new(temp.as_file_mut()).write_image(
            self.0.as_raw(),
            width,
            height,
            ExtendedColorType::Rgba8,
        )?;
        temp.persist_noclobber(path)
            .map_err(|error| PinshotError::io(path, error.error))?;
        Ok(())
    }
}

/// 共享单个屏幕，从首帧起定时采集；取消或共享被撤销时丢弃中间画布
pub fn record(
    region: Option<LogicalRegion>,
    duration: Duration,
    mut update: impl FnMut(Progress),
) -> Result<Option<Recording>> {
    let mut stitcher: Option<Stitcher> = None;
    let mut crop = None;
    let monitor = run_capture(Some(duration), |monitor, frame| {
        if stitcher.is_none()
            && let Some(region) = region
        {
            crop = Some(PixelRegion::from_logical(
                region,
                monitor.logical_size,
                frame.dimensions(),
            )?);
        }
        let frame = if let Some(crop) = crop {
            crop.crop(frame)
        } else {
            frame
        };
        let progress = if let Some(stitcher) = &mut stitcher {
            stitcher.push(&frame)?
        } else {
            let first = Stitcher::new(frame, MAX_HEIGHT)?;
            let height = first.height();
            let progress = if first.is_full() {
                Progress::LimitReached { height }
            } else {
                Progress::Started { height }
            };
            stitcher = Some(first);
            progress
        };
        update(progress);
        if matches!(progress, Progress::LimitReached { .. }) {
            return Ok(ControlFlow::Break(()));
        }
        Ok(ControlFlow::Continue(()))
    })?;
    if monitor.is_none() {
        return Ok(None);
    }
    let stitcher = stitcher.ok_or_else(|| failure("生成长图", "没有接收到首帧"))?;
    Ok(Some(Recording(stitcher.finish()?)))
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
        let original = Recording(frame.clone());
        original.save_png(&path).unwrap();
        let different = Recording(RgbaImage::from_pixel(20, 40, Rgba([1, 2, 3, 255])));
        assert!(different.save_png(&path).is_err());
        assert_eq!(image::open(path).unwrap().into_rgba8(), frame);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }
}
