use serde::{Deserialize, Serialize};

/// 前端 D-Bus 固定服务名
pub const FRONTEND_BUS_NAME: &str = "io.github.dgxz99.Pinshot.Frontend";

/// 前端 D-Bus 固定对象路径
pub const FRONTEND_OBJECT_PATH: &str = "/io/github/dgxz99/Pinshot/Frontend";

/// 前端 D-Bus 固定接口名
pub const FRONTEND_INTERFACE_NAME: &str = "io.github.dgxz99.Pinshot.Frontend1";

/// 贴图来源类型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PinKind {
    /// 区域截屏
    Screenshot,
    /// 剪贴板图片或本地图片文件
    ClipboardImage,
    /// 剪贴板纯文本渲染卡片
    ClipboardText,
    /// 滚动长图
    Longshot,
}

/// 初始位置坐标（逻辑像素）
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

/// 矩形区域（逻辑像素）
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// `CreatePin` / `UpdatePin` 传递给前端的贴图载荷
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PinPayload {
    pub id: String,
    pub image_path: String,
    pub thumb_path: String,
    pub width: u32,
    pub height: u32,
    /// 源屏幕缩放比例，用于分数缩放 1:1 逻辑尺寸还原
    pub source_scale: f64,
    pub kind: PinKind,
    pub has_text: bool,
    pub title_preview: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initial_pos: Option<Point>,
}

/// `ListActivePins` 返回的单条活跃贴图状态
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActivePinInfo {
    pub id: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub scale: f64,
    pub opacity: f64,
    pub mini: bool,
    pub ghost: bool,
    pub hidden: bool,
    pub title_preview: String,
    pub kind: PinKind,
    pub has_text: bool,
}

/// `SelectRegion` 返回的长图框选区域结果
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RegionSelection {
    pub cancelled: bool,
    pub monitor_index: i32,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub scale: f64,
}

/// `UpdateLongshotHud` 推送给前端的长图悬浮控制条状态
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LongshotHudState {
    pub active: bool,
    pub stitched_height: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rect: Option<Rect>,
}

/// `pinshot internal list-history` 返回给前端的历史条目摘要
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HistoryItemSummary {
    pub id: String,
    pub kind: PinKind,
    pub created_at: u64,
    pub last_used_at: u64,
    pub active: bool,
    pub thumb_path: String,
    pub has_text: bool,
    pub title_preview: String,
    pub width: u32,
    pub height: u32,
    pub size_bytes: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pin_payload_roundtrip_matches_dbus_contract() {
        let payload = PinPayload {
            id: "pin-001".to_string(),
            image_path: "/tmp/pin-001.png".to_string(),
            thumb_path: "/tmp/pin-001-thumb.png".to_string(),
            width: 1920,
            height: 1080,
            source_scale: 1.5,
            kind: PinKind::ClipboardText,
            has_text: true,
            title_preview: "fn main()".to_string(),
            initial_pos: None,
        };

        let json = serde_json::to_string(&payload).expect("序列化应成功");
        assert!(json.contains("\"kind\":\"clipboard_text\""));
        assert!(!json.contains("initial_pos"));

        let decoded: PinPayload = serde_json::from_str(&json).expect("反序列化应成功");
        assert_eq!(decoded, payload);
    }
}
