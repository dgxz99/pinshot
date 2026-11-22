use image::RgbaImage;
use image::imageops::crop_imm;

use super::failure;
use crate::Result;

/// 相对于所选显示器左上角的逻辑坐标，不使用整个桌面的全局原点
#[derive(Debug, Clone, Copy)]
pub struct LogicalRegion {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl std::str::FromStr for LogicalRegion {
    type Err = &'static str;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        const INVALID: &str = "选区应为 x,y,宽,高（非负整数，宽、高不能为零）";
        let numbers = value.split(',').map(|part| part.trim().parse::<u32>());
        let [x, y, width, height] = numbers
            .collect::<Vec<_>>()
            .try_into()
            .map_err(|_| INVALID)?;
        let (x, y, width, height) = (
            x.map_err(|_| INVALID)?,
            y.map_err(|_| INVALID)?,
            width.map_err(|_| INVALID)?,
            height.map_err(|_| INVALID)?,
        );
        if width == 0 || height == 0 {
            return Err(INVALID);
        }
        Ok(Self {
            x,
            y,
            width,
            height,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct PixelRegion {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

impl PixelRegion {
    /// 依据实际视频像素和 Portal 逻辑大小换算边界，向外取整避免漏掉分数缩放边缘
    pub(super) fn from_logical(
        region: LogicalRegion,
        logical_size: Option<(i32, i32)>,
        pixel_size: (u32, u32),
    ) -> Result<Self> {
        let Some((width, height)) = logical_size.filter(|&(w, h)| w > 0 && h > 0) else {
            return Err(failure("换算长图选区", "Portal 未提供有效的显示器逻辑尺寸"));
        };
        let map_axis = |start: u32, len: u32, logical: u32, pixels: u32| -> Option<(u32, u32)> {
            let end = start.checked_add(len)?;
            if len == 0 || end > logical || pixels == 0 {
                return None;
            }
            let first = u64::from(start) * u64::from(pixels) / u64::from(logical);
            let last = (u64::from(end) * u64::from(pixels)).div_ceil(u64::from(logical));
            Some((first as u32, (last - first) as u32))
        };
        let invalid = || {
            failure(
                "换算长图选区",
                format_args!("选区超出所共享显示器的 {width}×{height} 逻辑范围"),
            )
        };
        let (x, width) =
            map_axis(region.x, region.width, width as u32, pixel_size.0).ok_or_else(invalid)?;
        let (y, height) =
            map_axis(region.y, region.height, height as u32, pixel_size.1).ok_or_else(invalid)?;
        Ok(Self {
            x,
            y,
            width,
            height,
        })
    }

    pub(super) fn crop(self, frame: RgbaImage) -> RgbaImage {
        if self.x == 0 && self.y == 0 && frame.dimensions() == (self.width, self.height) {
            return frame;
        }
        crop_imm(&frame, self.x, self.y, self.width, self.height).to_image()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    #[test]
    fn maps_edges_at_fractional_and_different_axis_scales() {
        // 同一个局部选区无须知道显示器在虚拟桌面上是否位于负坐标
        let region = "1,3,799,597".parse().unwrap();
        let pixels = PixelRegion::from_logical(region, Some((800, 600)), (1000, 901)).unwrap();
        assert_eq!(
            pixels,
            PixelRegion {
                x: 1,
                y: 4,
                width: 999,
                height: 897
            }
        );
        let image = RgbaImage::from_fn(1000, 901, |x, y| {
            Rgba([(x % 256) as u8, (y % 256) as u8, 0, 255])
        });
        let crop = pixels.crop(image);
        assert_eq!(crop.dimensions(), (999, 897));
        assert_eq!(*crop.get_pixel(0, 0), Rgba([1, 4, 0, 255]));
        assert_eq!(*crop.get_pixel(998, 896), Rgba([231, 132, 0, 255]));
    }

    #[test]
    fn rejects_missing_metadata_and_regions_outside_one_monitor() {
        let region = "700,500,200,150".parse().unwrap();
        assert!(PixelRegion::from_logical(region, None, (1000, 900)).is_err());
        assert!(PixelRegion::from_logical(region, Some((800, 600)), (1000, 900)).is_err());
        let overflow = "4294967295,0,2,1".parse().unwrap();
        assert!(PixelRegion::from_logical(overflow, Some((800, 600)), (1000, 900)).is_err());
        for invalid in ["3,5,0,10", "-1,0,100,100", "1,1,4", "1,1,4,5,6"] {
            assert!(invalid.parse::<LogicalRegion>().is_err());
        }
    }
}
