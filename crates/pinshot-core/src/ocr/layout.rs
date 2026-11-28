use crate::ocr::OcrLine;

/// 引擎原始识别行（尚未排序或过滤）
#[derive(Debug, Clone)]
pub struct RawOcrLine {
    pub bbox: [f64; 4],
    pub text: String,
    pub confidence: f32,
    /// 可选：由 CTC 时间步计算出的精确字符归一化水平边界（长度 = `text.chars().count() + 1`）
    pub char_x: Option<Vec<f64>>,
}

/// 识别前合并同一视觉行上的近邻检测框：
/// - 兜住 DBNet 漏检在短间隙中的 `--`、`-D`、标点符号
/// - 消除因行内代码块底色导致的碎框与重叠框
pub fn merge_same_row_boxes(
    boxes: Vec<[f64; 4]>,
    image_width: u32,
    image_height: u32,
) -> Vec<[f64; 4]> {
    let max_w = f64::from(image_width.max(1));
    let max_h = f64::from(image_height.max(1));

    let mut valid: Vec<[f64; 4]> = boxes
        .into_iter()
        .filter_map(|[x, y, w, h]| {
            if ![x, y, w, h].iter().all(|v| v.is_finite()) {
                return None;
            }
            let x1 = x.clamp(0.0, max_w);
            let y1 = y.clamp(0.0, max_h);
            let x2 = (x + w).clamp(0.0, max_w);
            let y2 = (y + h).clamp(0.0, max_h);
            let cw = x2 - x1;
            let ch = y2 - y1;
            (cw >= 2.0 && ch >= 2.0).then_some([x1, y1, cw, ch])
        })
        .collect();

    if valid.is_empty() {
        return valid;
    }

    valid.sort_by(|a, b| {
        let ay = a[1] + a[3] * 0.5;
        let by = b[1] + b[3] * 0.5;
        ay.total_cmp(&by).then_with(|| a[0].total_cmp(&b[0]))
    });

    // 先按视觉行分组（要求垂直重叠充分且字高接近，避免跨行/大小标题误并）
    let mut rows: Vec<Vec<[f64; 4]>> = Vec::new();
    for b in valid {
        let mut placed = false;
        for row in &mut rows {
            let ref_box = row[row.len() - 1];
            if boxes_on_same_visual_row(ref_box, b) {
                row.push(b);
                placed = true;
                break;
            }
        }
        if !placed {
            rows.push(vec![b]);
        }
    }

    let mut merged = Vec::new();
    for mut row in rows {
        row.sort_by(|a, b| a[0].total_cmp(&b[0]));
        let mut cur = row[0];
        for next in row.into_iter().skip(1) {
            let cur_right = cur[0] + cur[2];
            let next_right = next[0] + next[2];
            let gap = next[0] - cur_right;
            let min_h = cur[3].min(next[3]);
            let combined_y1 = cur[1].min(next[1]);
            let combined_y2 = (cur[1] + cur[3]).max(next[1] + next[3]);
            let combined_h = (combined_y2 - combined_y1).max(1.0);
            let combined_w = (cur_right.max(next_right) - cur[0]).max(1.0);

            // 间距 <= 1.8 * 行高 且合并后宽高比不超过 Rec 模型动态宽度安全上限
            if gap <= 1.8 * min_h && (combined_w / combined_h) <= 52.0 {
                cur = [cur[0], combined_y1, combined_w, combined_h];
            } else {
                merged.push(pad_box_horizontal(cur, 2.0, max_w));
                cur = next;
            }
        }
        merged.push(pad_box_horizontal(cur, 2.0, max_w));
    }

    merged
}

fn boxes_on_same_visual_row(a: [f64; 4], b: [f64; 4]) -> bool {
    let min_h = a[3].min(b[3]).max(1.0);
    let max_h = a[3].max(b[3]);
    if max_h / min_h > 1.85 {
        return false;
    }
    let overlap_y = (a[1] + a[3]).min(b[1] + b[3]) - a[1].max(b[1]);
    let cy_diff = ((a[1] + a[3] * 0.5) - (b[1] + b[3] * 0.5)).abs();
    overlap_y >= 0.45 * min_h || cy_diff <= 0.4 * min_h
}

fn pad_box_horizontal(b: [f64; 4], pad: f64, max_w: f64) -> [f64; 4] {
    let x1 = (b[0] - pad).clamp(0.0, max_w);
    let x2 = (b[0] + b[2] + pad).clamp(0.0, max_w);
    [x1, b[1], (x2 - x1).max(1.0), b[3]]
}

/// 估算每个字符的归一化水平边界 `[0.0..=1.0]`（长度 = `chars.count() + 1`）
pub fn estimate_char_x(text: &str) -> Vec<f64> {
    let weights: Vec<f64> = text.chars().map(char_visual_weight).collect();
    if weights.is_empty() {
        return vec![0.0];
    }
    let total: f64 = weights.iter().sum::<f64>().max(f64::EPSILON);
    let mut positions = Vec::with_capacity(weights.len() + 1);
    positions.push(0.0);
    let mut acc = 0.0;
    for w in weights {
        acc += w;
        positions.push((acc / total).clamp(0.0, 1.0));
    }
    if let Some(last) = positions.last_mut() {
        *last = 1.0;
    }
    positions
}

fn char_visual_weight(ch: char) -> f64 {
    if is_wide_cjk(ch) {
        return 1.0;
    }
    match ch {
        ' ' | '\t' => 0.38,
        'i' | 'l' | 'I' | '1' | '!' | '|' | '.' | ',' | ':' | ';' | '\'' | '`' => 0.34,
        'f' | 'j' | 'r' | 't' | '(' | ')' | '[' | ']' | '{' | '}' | '/' | '\\' | '-' => 0.44,
        'm' | 'w' | 'M' | 'W' | '@' | '%' | '#' => 0.78,
        _ if ch.is_ascii_uppercase() => 0.64,
        _ => 0.56,
    }
}

fn is_wide_cjk(ch: char) -> bool {
    matches!(
        ch as u32,
        0x2E80..=0x303F
            | 0x3040..=0x30FF
            | 0x3100..=0x31FF
            | 0x3400..=0x4DBF
            | 0x4E00..=0x9FFF
            | 0xAC00..=0xD7AF
            | 0xF900..=0xFAFF
            | 0xFE30..=0xFE6F
            | 0xFF01..=0xFF60
            | 0xFFE0..=0xFFE6
    )
}

/// 裁剪首尾空白并同步收缩 `bbox` 与精确 `char_x`
fn trim_text_and_char_x(
    raw_text: &str,
    bbox: [f64; 4],
    raw_char_x: Option<Vec<f64>>,
) -> Option<(String, [f64; 4], Vec<f64>)> {
    let chars: Vec<char> = raw_text.chars().collect();
    let start = chars.iter().position(|c| !c.is_whitespace())?;
    let end = chars.iter().rposition(|c| !c.is_whitespace())? + 1;
    let trimmed: String = chars[start..end].iter().collect();

    if let Some(xs) = raw_char_x
        && xs.len() == chars.len() + 1
    {
        let left_frac = xs[start].clamp(0.0, 1.0);
        let right_frac = xs[end].clamp(left_frac, 1.0);
        let span = right_frac - left_frac;
        if span > 1e-4 {
            let new_x = bbox[0] + bbox[2] * left_frac;
            let new_w = (bbox[2] * span).max(2.0);
            let mut sliced = Vec::with_capacity(end - start + 1);
            let mut prev = 0.0f64;
            for &v in &xs[start..=end] {
                let norm = ((v - left_frac) / span).clamp(prev, 1.0);
                sliced.push(norm);
                prev = norm;
            }
            sliced[0] = 0.0;
            if let Some(last) = sliced.last_mut() {
                *last = 1.0;
            }
            return Some((trimmed, [new_x, bbox[1], new_w, bbox[3]], sliced));
        }
    }

    let char_x = estimate_char_x(&trimmed);
    Some((trimmed, bbox, char_x))
}

/// 过滤低置信度/无效文本框，按视觉阅读顺序排序、修剪水平重叠并标记同行关系
pub fn normalize_and_sort_lines(
    raw: Vec<RawOcrLine>,
    image_width: u32,
    image_height: u32,
    min_confidence: f32,
) -> Vec<OcrLine> {
    let max_w = f64::from(image_width.max(1));
    let max_h = f64::from(image_height.max(1));

    let mut cleaned: Vec<OcrLine> = raw
        .into_iter()
        .filter_map(|item| {
            if item.confidence < min_confidence {
                return None;
            }
            let [x, y, w, h] = item.bbox;
            if !x.is_finite() || !y.is_finite() || !w.is_finite() || !h.is_finite() {
                return None;
            }
            let x1 = x.clamp(0.0, max_w);
            let y1 = y.clamp(0.0, max_h);
            let x2 = (x + w).clamp(0.0, max_w);
            let y2 = (y + h).clamp(0.0, max_h);
            let cw = x2 - x1;
            let ch = y2 - y1;
            if cw < 2.0 || ch < 2.0 {
                return None;
            }
            let (text, bbox, char_x) =
                trim_text_and_char_x(&item.text, [x1, y1, cw, ch], item.char_x)?;
            Some(OcrLine {
                bbox,
                text,
                confidence: item.confidence,
                char_x,
                same_visual_line_next: false,
            })
        })
        .collect();

    if cleaned.is_empty() {
        return cleaned;
    }

    cleaned.sort_by(|a, b| {
        let ay = a.bbox[1] + a.bbox[3] * 0.5;
        let by = b.bbox[1] + b.bbox[3] * 0.5;
        ay.total_cmp(&by)
            .then_with(|| a.bbox[0].total_cmp(&b.bbox[0]))
    });

    let mut rows: Vec<Vec<OcrLine>> = Vec::new();
    for line in cleaned {
        let cy = line.bbox[1] + line.bbox[3] * 0.5;
        let mut placed = false;
        for row in &mut rows {
            let row_cy: f64 =
                row.iter().map(|l| l.bbox[1] + l.bbox[3] * 0.5).sum::<f64>() / row.len() as f64;
            let row_h: f64 = row.iter().map(|l| l.bbox[3]).sum::<f64>() / row.len() as f64;
            let tol = 0.55 * row_h.min(line.bbox[3]);
            if (cy - row_cy).abs() <= tol {
                row.push(line.clone());
                placed = true;
                break;
            }
        }
        if !placed {
            rows.push(vec![line]);
        }
    }

    rows.sort_by(|ra, rb| {
        let ya = ra.iter().map(|l| l.bbox[1] + l.bbox[3] * 0.5).sum::<f64>() / ra.len() as f64;
        let yb = rb.iter().map(|l| l.bbox[1] + l.bbox[3] * 0.5).sum::<f64>() / rb.len() as f64;
        ya.total_cmp(&yb)
    });

    let mut ordered = Vec::new();
    for mut row in rows {
        row.sort_by(|a, b| a.bbox[0].total_cmp(&b.bbox[0]));
        trim_horizontal_overlaps(&mut row);
        let row_len = row.len();
        for (idx, mut item) in row.into_iter().enumerate() {
            item.same_visual_line_next = idx + 1 < row_len;
            ordered.push(item);
        }
    }
    ordered
}

/// 修剪同一视觉行中相邻框的轻微水平重叠，避免高亮叠色
fn trim_horizontal_overlaps(row: &mut [OcrLine]) {
    for i in 0..row.len().saturating_sub(1) {
        let left_end = row[i].bbox[0] + row[i].bbox[2];
        let right_start = row[i + 1].bbox[0];
        if left_end > right_start {
            let mid = (left_end + right_start) * 0.5;
            let new_left_w = (mid - row[i].bbox[0]).max(2.0);
            let right_end = row[i + 1].bbox[0] + row[i + 1].bbox[2];
            let new_right_x = mid.min(right_end - 2.0);
            row[i].bbox[2] = new_left_w;
            row[i + 1].bbox[0] = new_right_x;
            row[i + 1].bbox[2] = (right_end - new_right_x).max(2.0);
        }
    }
}

/// 按视觉行关系拼接识别文本
pub fn join_document_text(lines: &[OcrLine]) -> String {
    let mut out = String::new();
    for (idx, line) in lines.iter().enumerate() {
        out.push_str(&line.text);
        if idx + 1 < lines.len() {
            if line.same_visual_line_next {
                out.push(' ');
            } else {
                out.push('\n');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimate_char_x_is_monotonic_and_weights_cjk_wider_than_narrow_ascii() {
        let xs = estimate_char_x("i中W");
        assert_eq!(xs.len(), 4);
        assert_eq!(xs[0], 0.0);
        assert_eq!(xs[3], 1.0);
        let w_i = xs[1] - xs[0];
        let w_cjk = xs[2] - xs[1];
        let w_w = xs[3] - xs[2];
        assert!(w_cjk > w_w && w_w > w_i);
    }

    #[test]
    fn merge_same_row_boxes_bridges_short_gaps_but_keeps_distant_columns_separate() {
        let merged = merge_same_row_boxes(
            vec![
                [40.0, 100.0, 200.0, 26.0],
                [258.0, 101.0, 150.0, 25.0], // 18px 间距 <= 1.8 * 25 -> 合并
                [600.0, 100.0, 120.0, 26.0], // 192px 间距 -> 独立列不合并
            ],
            1000,
            400,
        );
        assert_eq!(merged.len(), 2);
        assert!(merged[0][0] <= 40.0 && merged[0][0] + merged[0][2] >= 408.0);
        assert!(merged[1][0] >= 596.0);
    }

    #[test]
    fn normalize_filters_low_confidence_trims_overlap_and_groups_visual_rows() {
        let raw = vec![
            RawOcrLine {
                bbox: [154.0, 22.0, 100.0, 24.0],
                text: "Right".into(),
                confidence: 0.95,
                char_x: None,
            },
            RawOcrLine {
                bbox: [40.0, 80.0, 140.0, 24.0],
                text: "Second row".into(),
                confidence: 0.92,
                char_x: None,
            },
            RawOcrLine {
                bbox: [40.0, 20.0, 120.0, 24.0], // right=160 与 154 重叠 6px
                text: "Left".into(),
                confidence: 0.98,
                char_x: Some(vec![0.0, 0.2, 0.45, 0.72, 1.0]),
            },
            RawOcrLine {
                bbox: [500.0, 20.0, 16.0, 16.0],
                text: "□".into(),
                confidence: 0.35,
                char_x: None,
            },
        ];

        let sorted = normalize_and_sort_lines(raw, 800, 600, 0.60);
        assert_eq!(sorted.len(), 3);
        assert_eq!(sorted[0].text, "Left");
        assert_eq!(sorted[0].char_x, vec![0.0, 0.2, 0.45, 0.72, 1.0]);
        assert!(sorted[0].same_visual_line_next);
        // 重叠应在中心 157.0 处裁齐
        assert!((sorted[0].bbox[0] + sorted[0].bbox[2] - sorted[1].bbox[0]).abs() < 1e-6);
        assert_eq!(sorted[1].text, "Right");
        assert!(!sorted[1].same_visual_line_next);
        assert_eq!(sorted[2].text, "Second row");
        assert_eq!(join_document_text(&sorted), "Left Right\nSecond row");
    }
}
