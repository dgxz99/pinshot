use std::ops::Range;

/// 沿拼接轴不随内容移动的两端；横向录制在进入此模块前已经转置
#[derive(Clone, Copy, Default)]
pub(super) struct FixedBands {
    leading: usize,
    trailing: usize,
}

impl FixedBands {
    pub(super) fn body(self, height: usize) -> Range<usize> {
        self.leading..height - self.trailing
    }

    pub(super) fn length(self) -> usize {
        self.leading + self.trailing
    }

    /// 只接受连续不动且有细节或清晰色差分界的边缘，正文留白不作为固定栏证据
    pub(super) fn detect(before: &[u8], after: &[u8], width: usize, height: usize) -> Self {
        let pitch = width * 4;
        let margin = side_inset(width) * 4;
        let max_band = height / 3;
        let sample = |y: usize| y * pitch + margin..(y + 1) * pitch - margin;
        let find = |rows: &mut dyn Iterator<Item = usize>| {
            let mut count = 0;
            let mut flat_color = None;
            let mut has_detail = false;
            for y in rows {
                let range = sample(y);
                let row = &before[range.clone()];
                let next = &after[range];
                if row != next {
                    if let Some(color) = flat_color {
                        has_detail |=
                            distinct_background(row, color) && distinct_background(next, color);
                    }
                    break;
                }
                count += 1;
                if count > max_band {
                    return 0;
                }
                if !has_detail {
                    let color = flat_color.get_or_insert(&row[..3]);
                    has_detail = row
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .any(|pixel| distinct_color(pixel, color));
                }
            }
            if has_detail { count } else { 0 }
        };
        Self {
            leading: find(&mut (0..height)),
            trailing: find(&mut (0..height).rev()),
        }
    }
}

fn distinct_color(pixel: &[u8], background: &[u8]) -> bool {
    pixel[..3]
        .iter()
        .zip(background)
        .any(|(a, b)| a.abs_diff(*b) > 12)
}

fn distinct_background(row: &[u8], background: &[u8]) -> bool {
    let different = row
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|pixel| distinct_color(*pixel, background))
        .count();
    different * 4 >= (row.len() / 4) * 3
}

/// 侧面的边框和细滚动条不参与匹配；不改变成品宽度
pub(super) fn side_inset(width: usize) -> usize {
    width / 24
}
