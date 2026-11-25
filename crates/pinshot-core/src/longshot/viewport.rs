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
        let find = |rows: &mut dyn Iterator<Item = usize>, corners: usize| {
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
                    return corners;
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
            if has_detail || corners > 0 {
                count.max(corners)
            } else {
                0
            }
        };
        let leading_corners = corner_band(before, after, width, max_band, |offset| offset);
        let trailing_corners =
            corner_band(before, after, width, max_band, |offset| height - 1 - offset);
        Self {
            leading: find(&mut (0..height), leading_corners),
            trailing: find(&mut (0..height).rev(), trailing_corners),
        }
    }
}

/// 圆角可能只出现在被排除出匹配的两侧；两边都出现固定的轮廓变化才单独保留这一段
fn corner_band(
    before: &[u8],
    after: &[u8],
    width: usize,
    max_band: usize,
    edge_row: impl Fn(usize) -> usize,
) -> usize {
    let inset = side_inset(width);
    if inset < 2 {
        return 0;
    }
    let reference = edge_row(max_band) * width * 4;
    let side_changes = |row_start, side: Range<usize>| {
        side.into_iter().any(|x| {
            let offset = x * 4;
            let edge = row_start + offset..row_start + offset + 4;
            let straight = reference + offset..reference + offset + 4;
            before[edge.clone()] == after[edge.clone()]
                && before[straight.clone()] == after[straight.clone()]
                && before[edge] != before[straight]
        })
    };
    let length = (0..max_band)
        .take_while(|&offset| {
            let row_start = edge_row(offset) * width * 4;
            side_changes(row_start, 0..inset) && side_changes(row_start, width - inset..width)
        })
        .count();
    if (2..max_band).contains(&length) {
        length
    } else {
        0
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
