use serde::{Deserialize, Serialize};

/// 前端 D-Bus 固定服务名
pub const FRONTEND_BUS_NAME: &str = "io.github.dgxz99.Pinshot.Frontend";

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
        };

        let json = serde_json::to_string(&payload).expect("序列化应成功");
        assert!(json.contains("\"kind\":\"clipboard_text\""));

        let decoded: PinPayload = serde_json::from_str(&json).expect("反序列化应成功");
        assert_eq!(decoded, payload);
    }
}
