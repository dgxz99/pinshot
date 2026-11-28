use std::path::Path;

use image::{GenericImageView, RgbImage, imageops};
use pure_onnx_ocr::{
    DetInferenceSession, DetLimitType, DetPolygonUnclipper, DetPolygonUnclipperConfig,
    DetPostProcessor, DetPostProcessorConfig, DetPreProcessor, DetPreProcessorConfig,
    PADDLE_CONFIG_FILE, PADDLE_MODEL_FILE, PaddleInferenceConfig, Polygon, RecDictionary,
    RecInferenceSession, RecPreProcessor, RecPreProcessorConfig, default_inference_threads,
};

use crate::config::OcrConfig;
use crate::error::{PinshotError, Result};
use crate::ocr::layout::{RawOcrLine, merge_same_row_boxes, normalize_and_sort_lines};
use crate::ocr::models::ResolvedModelDir;
use crate::ocr::{OCR_SCHEMA_VERSION, OcrDocument};

const DET_MAX_SIDE_LIMIT: u32 = 1600;

fn crop_aspect(crop: &RgbImage) -> f64 {
    f64::from(crop.width()) / f64::from(crop.height().max(1))
}

/// 对磁盘图片执行离线 PP-OCR 推理：
/// 1. 原生分辨率 DBNet 文本检测（`DetLimitType::Min`，避免宽图被压糊漏掉 `--` 等短符号）
/// 2. 识别前自动合并同行近邻碎片框（兜住短间隙中的符号与标点）
/// 3. SVTR 批量识别 + CTC 时间步精确字符水平边界计算
pub fn recognize_image_file(
    image_path: &Path,
    model: &ResolvedModelDir,
    cfg: &OcrConfig,
) -> Result<OcrDocument> {
    let img = image::open(image_path)?;
    let (width, height) = img.dimensions();

    let det_yaml_path = model.det_dir.join(PADDLE_CONFIG_FILE);
    let rec_yaml_path = model.rec_dir.join(PADDLE_CONFIG_FILE);
    let det_model_path = model.det_dir.join(PADDLE_MODEL_FILE);
    let rec_model_path = model.rec_dir.join(PADDLE_MODEL_FILE);

    let det_model_cfg = PaddleInferenceConfig::from_path(&det_yaml_path)
        .map_err(|e| PinshotError::Ocr(format!("读取检测模型配置失败: {e}")))?;
    let rec_model_cfg = PaddleInferenceConfig::from_path(&rec_yaml_path)
        .map_err(|e| PinshotError::Ocr(format!("读取识别模型配置失败: {e}")))?;

    let threads = if cfg.threads > 0 {
        cfg.threads
    } else {
        default_inference_threads()
    };

    // 1. 原生分辨率文本检测
    let mut det_pre_cfg = DetPreProcessorConfig {
        limit_side_len: 64,
        limit_type: DetLimitType::Min,
        max_side_limit: DET_MAX_SIDE_LIMIT,
        ..DetPreProcessorConfig::default()
    };
    if let Some(order) = det_model_cfg.color_order {
        det_pre_cfg.color_order = order;
    }
    if let Some(mean) = det_model_cfg.normalize_mean {
        det_pre_cfg.mean = mean;
    }
    if let Some(std) = det_model_cfg.normalize_std {
        det_pre_cfg.std = std;
    }

    let det_pre = DetPreProcessor::new(det_pre_cfg);
    let det_input = det_pre
        .process(&img)
        .map_err(|e| PinshotError::Ocr(format!("检测预处理失败: {e}")))?;

    let mut det_session = DetInferenceSession::load(&det_model_path)
        .map_err(|e| PinshotError::Ocr(format!("加载检测模型失败: {e}")))?;
    det_session.set_inference_threads(threads);

    let det_output = det_session
        .run(&det_input)
        .map_err(|e| PinshotError::Ocr(format!("检测推理失败: {e}")))?;

    let det_post = DetPostProcessor::new(DetPostProcessorConfig {
        threshold: 0.25,
        box_threshold: 0.45,
        min_area: 8.0,
        max_candidates: 1000,
    });
    let contours = det_post
        .process(&det_output)
        .map_err(|e| PinshotError::Ocr(format!("检测后处理失败: {e}")))?;

    let unclipper = DetPolygonUnclipper::new(DetPolygonUnclipperConfig {
        unclip_ratio: 1.5,
        min_result_area: 16.0,
        ..DetPolygonUnclipperConfig::default()
    });
    let polygons = unclipper.unclip_contours(&contours);

    let (inv_sx, inv_sy) = det_input.inverse_scale();
    let raw_boxes: Vec<[f64; 4]> = polygons
        .iter()
        .filter_map(|poly| scaled_polygon_to_bbox(poly, inv_sx, inv_sy, width, height))
        .collect();

    // 2. 识别前合并同一视觉行的近邻框
    let merged_boxes = merge_same_row_boxes(raw_boxes, width, height);
    if merged_boxes.is_empty() {
        return Ok(OcrDocument {
            version: OCR_SCHEMA_VERSION,
            model_id: model.model_id.clone(),
            image_width: width,
            image_height: height,
            lines: Vec::new(),
        });
    }

    // 3. 批量裁切识别 + CTC 时间步精确字符边界
    let mut rec_pre_cfg = RecPreProcessorConfig::default();
    if let Some(order) = rec_model_cfg.color_order {
        rec_pre_cfg.color_order = order;
    }
    if let Some([_, h, w]) = rec_model_cfg.rec_image_shape {
        rec_pre_cfg.target_height = h;
        rec_pre_cfg.max_width = w;
    }
    let target_height = rec_pre_cfg.target_height;
    let rec_pre = RecPreProcessor::new(rec_pre_cfg);

    let mut rec_session =
        RecInferenceSession::load_with_input_height(&rec_model_path, target_height)
            .map_err(|e| PinshotError::Ocr(format!("加载识别模型失败: {e}")))?;

    let dict = RecDictionary::from_inference_yml(&rec_yaml_path)
        .map_err(|e| PinshotError::Ocr(format!("读取识别字典失败: {e}")))?
        .with_space_char();
    let blank_id = dict.blank_id();

    let rgb = img.to_rgb8();
    let crops: Vec<RgbImage> = merged_boxes
        .iter()
        .map(|&[x, y, w, h]| crop_rgb(&rgb, x, y, w, h))
        .collect();

    // 按宽高比升序分组：短/中行 4 条一批，超长行（宽高比 > 30）单条独立并行，
    // 既避免宽行把短行填充到 3200px，也消除单核长尾耗时
    let mut order: Vec<usize> = (0..crops.len()).collect();
    order.sort_by(|&a, &b| crop_aspect(&crops[a]).total_cmp(&crop_aspect(&crops[b])));
    let mut chunks: Vec<Vec<usize>> = Vec::new();
    let mut current: Vec<usize> = Vec::new();
    for idx in order {
        if crop_aspect(&crops[idx]) > 30.0 {
            if !current.is_empty() {
                chunks.push(std::mem::take(&mut current));
            }
            chunks.push(vec![idx]);
        } else {
            current.push(idx);
            if current.len() >= 4 {
                chunks.push(std::mem::take(&mut current));
            }
        }
    }
    if !current.is_empty() {
        chunks.push(current);
    }

    let batches: Vec<_> = chunks
        .iter()
        .map(|chunk| {
            let batch_crops: Vec<RgbImage> = chunk.iter().map(|&idx| crops[idx].clone()).collect();
            rec_pre
                .process_images(&batch_crops)
                .map_err(|e| PinshotError::Ocr(format!("识别预处理失败: {e}")))
        })
        .collect::<Result<Vec<_>>>()?;

    // 多批次时每批单线程并行推理；单批次时使用线程池加速单批矩阵计算
    let outputs: Vec<_> = if batches.len() > 1 && threads > 1 {
        let worker_limit = threads.min(batches.len());
        let mut all_outputs = Vec::with_capacity(batches.len());
        for group in batches.chunks(worker_limit) {
            let group_res: Vec<Result<_>> = std::thread::scope(|scope| {
                let handles: Vec<_> = group
                    .iter()
                    .map(|batch| {
                        scope.spawn(|| {
                            rec_session
                                .run(batch)
                                .map_err(|e| PinshotError::Ocr(format!("识别推理失败: {e}")))
                        })
                    })
                    .collect();
                handles
                    .into_iter()
                    .map(|h| {
                        h.join()
                            .unwrap_or_else(|_| Err(PinshotError::Ocr("识别线程异常终止".into())))
                    })
                    .collect()
            });
            for item in group_res {
                all_outputs.push(item?);
            }
        }
        all_outputs
    } else {
        rec_session.set_inference_threads(threads);
        batches
            .iter()
            .map(|batch| {
                rec_session
                    .run(batch)
                    .map_err(|e| PinshotError::Ocr(format!("识别推理失败: {e}")))
            })
            .collect::<Result<Vec<_>>>()?
    };

    let mut raw_lines = Vec::with_capacity(merged_boxes.len());
    for (chunk, output) in chunks.iter().zip(outputs.iter()) {
        for (offset, &orig_idx) in chunk.iter().enumerate() {
            let bbox = merged_boxes[orig_idx];
            let valid_t = output.valid_timesteps.get(offset).copied().unwrap_or(0);
            let (text, confidence, char_x) =
                decode_ctc_with_char_x(&output.logits, offset, valid_t, &dict, blank_id);
            raw_lines.push(RawOcrLine {
                bbox,
                text,
                confidence,
                char_x,
            });
        }
    }

    let lines = normalize_and_sort_lines(raw_lines, width, height, cfg.min_confidence);

    Ok(OcrDocument {
        version: OCR_SCHEMA_VERSION,
        model_id: model.model_id.clone(),
        image_width: width,
        image_height: height,
        lines,
    })
}

fn scaled_polygon_to_bbox(
    poly: &Polygon<f64>,
    inv_sx: f64,
    inv_sy: f64,
    img_w: u32,
    img_h: u32,
) -> Option<[f64; 4]> {
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for pt in poly.exterior().points() {
        min_x = min_x.min(pt.x() * inv_sx);
        min_y = min_y.min(pt.y() * inv_sy);
        max_x = max_x.max(pt.x() * inv_sx);
        max_y = max_y.max(pt.y() * inv_sy);
    }
    if ![min_x, min_y, max_x, max_y].iter().all(|v| v.is_finite()) {
        return None;
    }
    let mw = f64::from(img_w.max(1));
    let mh = f64::from(img_h.max(1));
    let x1 = min_x.clamp(0.0, mw);
    let y1 = min_y.clamp(0.0, mh);
    let x2 = max_x.clamp(0.0, mw);
    let y2 = max_y.clamp(0.0, mh);
    Some([x1, y1, (x2 - x1).max(0.0), (y2 - y1).max(0.0)])
}

fn crop_rgb(rgb: &RgbImage, x: f64, y: f64, w: f64, h: f64) -> RgbImage {
    let img_w = rgb.width().max(1);
    let img_h = rgb.height().max(1);
    let cx = (x.floor() as u32).min(img_w - 1);
    let cy = (y.floor() as u32).min(img_h - 1);
    let cw = (w.ceil() as u32).max(1).min(img_w - cx);
    let ch = (h.ceil() as u32).max(1).min(img_h - cy);
    imageops::crop_imm(rgb, cx, cy, cw, ch).to_image()
}

/// 贪心 CTC 解码并同时计算每个字符的归一化时间步区间 `[0.0..=1.0]`
fn decode_ctc_with_char_x(
    logits: &ndarray::Array3<f32>,
    sample_idx: usize,
    valid_timesteps: usize,
    dict: &RecDictionary,
    blank_id: usize,
) -> (String, f32, Option<Vec<f64>>) {
    let (_, max_t, num_classes) = logits.dim();
    let steps = valid_timesteps.min(max_t);
    if steps == 0 || num_classes == 0 {
        return (String::new(), 0.0, None);
    }

    struct TokenSpan {
        token: String,
        t_start: usize,
        t_end: usize,
    }

    let mut spans: Vec<TokenSpan> = Vec::new();
    let mut conf_sum = 0.0f64;
    let mut prev_cls = blank_id;

    for t in 0..steps {
        let row = logits.slice(ndarray::s![sample_idx, t, ..]);
        let mut best_cls = 0usize;
        let mut best_val = f32::NEG_INFINITY;
        let mut min_val = f32::INFINITY;
        let mut sum_val = 0.0f32;
        if let Some(slice) = row.as_slice() {
            for (c, &v) in slice.iter().enumerate() {
                if v > best_val {
                    best_val = v;
                    best_cls = c;
                }
                if v < min_val {
                    min_val = v;
                }
                sum_val += v;
            }
        } else {
            for (c, &v) in row.iter().enumerate() {
                if v > best_val {
                    best_val = v;
                    best_cls = c;
                }
                if v < min_val {
                    min_val = v;
                }
                sum_val += v;
            }
        }

        if best_cls == blank_id {
            prev_cls = blank_id;
            continue;
        }
        if best_cls == prev_cls {
            if let Some(last) = spans.last_mut() {
                last.t_end = t + 1;
            }
            continue;
        }

        prev_cls = best_cls;
        let prob = if min_val >= -1e-4 && (sum_val - 1.0).abs() < 1e-2 {
            best_val.clamp(0.0, 1.0)
        } else {
            let denom: f64 = if let Some(slice) = row.as_slice() {
                slice.iter().map(|&v| f64::from(v - best_val).exp()).sum()
            } else {
                row.iter().map(|&v| f64::from(v - best_val).exp()).sum()
            };
            (1.0 / denom.max(1.0)) as f32
        };

        if let Some(tok) = dict.token(best_cls)
            && !tok.is_empty()
        {
            conf_sum += f64::from(prob);
            spans.push(TokenSpan {
                token: tok.to_string(),
                t_start: t,
                t_end: t + 1,
            });
        }
    }

    if spans.is_empty() {
        return (String::new(), 0.0, None);
    }

    let confidence = (conf_sum / spans.len() as f64) as f32;
    let mut text = String::new();
    let mut char_centers: Vec<(f64, f64)> = Vec::new();
    let denom = steps as f64;

    for span in &spans {
        let chars: Vec<char> = span.token.chars().collect();
        let n = chars.len().max(1) as f64;
        let s0 = span.t_start as f64 / denom;
        let s1 = span.t_end as f64 / denom;
        for (i, ch) in chars.into_iter().enumerate() {
            text.push(ch);
            let a = s0 + (s1 - s0) * (i as f64 / n);
            let b = s0 + (s1 - s0) * ((i + 1) as f64 / n);
            char_centers.push((a, b));
        }
    }

    let count = char_centers.len();
    let mut char_x = Vec::with_capacity(count + 1);
    char_x.push(0.0);
    for i in 0..count.saturating_sub(1) {
        let boundary = ((char_centers[i].1 + char_centers[i + 1].0) * 0.5).clamp(0.0, 1.0);
        let prev = *char_x.last().unwrap_or(&0.0);
        char_x.push(boundary.max(prev));
    }
    char_x.push(1.0);

    (text, confidence, Some(char_x))
}
