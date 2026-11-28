pub mod layout;
pub mod models;

#[cfg(feature = "ocr")]
pub mod engine;

use serde::{Deserialize, Serialize};

/// OCR 缓存与 IPC 协议版本号
pub const OCR_SCHEMA_VERSION: u32 = 1;

/// 单张图片的离线 OCR 识别文档（原图像素坐标系）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OcrDocument {
    pub version: u32,
    pub model_id: String,
    pub image_width: u32,
    pub image_height: u32,
    pub lines: Vec<OcrLine>,
}

/// 单条识别文本行（含精确边界框与归一化字符水平区间）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OcrLine {
    /// 原图像素边界框 `[x, y, width, height]`
    pub bbox: [f64; 4],
    pub text: String,
    pub confidence: f32,
    /// 长度为 `text.chars().count() + 1` 的单调递增归一化水平坐标 `[0.0..=1.0]`
    pub char_x: Vec<f64>,
    /// 是否与下一条记录处于同一视觉行（用于多行复制时智能拼空格或换行）
    #[serde(default)]
    pub same_visual_line_next: bool,
}

impl OcrDocument {
    /// 拼接全部识别文本（同一视觉行以空格连接，不同视觉行以换行连接）
    pub fn full_text(&self) -> String {
        layout::join_document_text(&self.lines)
    }
}

/// 供 GNOME 扩展调用 `internal ocr-pin` 的标准 JSON 响应
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum OcrPinResponse {
    Ok { document: OcrDocument },
    MissingModel { model_id: String },
    Disabled,
    Skipped { reason: String },
    Unavailable { reason: String },
}

/// 当前二进制是否编译了离线 OCR 推理能力
pub const fn engine_compiled() -> bool {
    cfg!(feature = "ocr")
}
