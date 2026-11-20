use std::collections::HashMap;
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

use image::ImageFormat;
use zbus::blocking::Connection;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

use crate::desktop::{decode_file_uri, detect_display_scale};
use crate::error::{PinshotError, Result};

/// 区域截屏成功后的输出图像
#[derive(Debug, Clone)]
pub struct CapturedImage {
    pub png_bytes: Vec<u8>,
    pub source_scale: f64,
}

#[zbus::proxy(
    interface = "org.freedesktop.portal.Screenshot",
    default_service = "org.freedesktop.portal.Desktop",
    default_path = "/org/freedesktop/portal/desktop",
    gen_async = false,
    gen_blocking = true
)]
trait PortalScreenshot {
    fn screenshot(
        &self,
        parent_window: &str,
        options: HashMap<&str, Value<'_>>,
    ) -> zbus::Result<OwnedObjectPath>;
}

#[zbus::proxy(
    interface = "org.freedesktop.portal.Request",
    default_service = "org.freedesktop.portal.Desktop",
    gen_async = false,
    gen_blocking = true
)]
trait PortalRequest {
    #[zbus(signal)]
    fn response(&self, response: u32, results: HashMap<String, OwnedValue>) -> zbus::Result<()>;
}

/// 调用 `org.freedesktop.portal.Screenshot` 进行交互式区域截屏
///
/// 返回 `Ok(None)` 表示用户取消了截屏操作
pub fn capture_interactive_region() -> Result<Option<CapturedImage>> {
    let conn = Connection::session().map_err(|e| PinshotError::Dbus(e.to_string()))?;
    let unique_name = conn
        .unique_name()
        .ok_or_else(|| PinshotError::Dbus("无法获取 D-Bus 会话连接唯一名称".to_string()))?
        .as_str()
        .to_string();

    let handle_token = build_handle_token();
    let expected_path = build_portal_request_path(&unique_name, &handle_token);

    // 必须在发起 Screenshot 调用之前订阅 Response 信号，避免快速返回时丢失信号
    let expected_req_proxy = PortalRequestProxy::builder(&conn)
        .path(expected_path.as_str())
        .map_err(|e| PinshotError::Dbus(e.to_string()))?
        .build()
        .map_err(|e| PinshotError::Dbus(e.to_string()))?;
    let mut responses = expected_req_proxy
        .receive_response()
        .map_err(|e| PinshotError::Dbus(e.to_string()))?;

    let screenshot_proxy =
        PortalScreenshotProxy::new(&conn).map_err(|e| PinshotError::Dbus(e.to_string()))?;
    let mut options: HashMap<&str, Value<'_>> = HashMap::new();
    options.insert("interactive", Value::Bool(true));
    options.insert("handle_token", Value::from(handle_token.as_str()));

    let actual_path = screenshot_proxy
        .screenshot("", options)
        .map_err(|e| PinshotError::Dbus(format!("调用 Portal 截屏失败: {e}")))?;

    // 兼容少数不遵循 handle_token 路径约定的 Portal 实现
    let signal = if actual_path.as_str() != expected_path {
        let actual_req_proxy = PortalRequestProxy::builder(&conn)
            .path(actual_path)
            .map_err(|e| PinshotError::Dbus(e.to_string()))?
            .build()
            .map_err(|e| PinshotError::Dbus(e.to_string()))?;
        let mut alt_responses = actual_req_proxy
            .receive_response()
            .map_err(|e| PinshotError::Dbus(e.to_string()))?;
        alt_responses.next()
    } else {
        responses.next()
    };

    let Some(msg) = signal else {
        return Err(PinshotError::Dbus("等待 Portal 截屏响应中断".to_string()));
    };
    let args = msg
        .args()
        .map_err(|e| PinshotError::Dbus(format!("解析 Portal 截屏响应失败: {e}")))?;

    match args.response {
        0 => {
            let uri_val = args
                .results
                .get("uri")
                .ok_or_else(|| PinshotError::Dbus("Portal 截屏响应缺少 uri 字段".to_string()))?;
            let uri = <&str>::try_from(uri_val).map_err(|e| {
                PinshotError::Dbus(format!("Portal 截屏 uri 字段不是有效字符串: {e}"))
            })?;
            let path = decode_file_uri(uri).ok_or_else(|| {
                PinshotError::Dbus(format!("无法解码 Portal 截屏返回的 URI: {uri}"))
            })?;

            let raw_bytes = fs::read(&path).map_err(|e| PinshotError::io(&path, e))?;
            let png_bytes = ensure_png_bytes(&raw_bytes)?;
            let source_scale = detect_display_scale();

            Ok(Some(CapturedImage {
                png_bytes,
                source_scale,
            }))
        }
        1 => Ok(None),
        other => Err(PinshotError::Dbus(format!(
            "Portal 截屏返回异常状态码: {other}"
        ))),
    }
}

/// 确保图像字节流为标准 PNG（已是 PNG 时零重编码开销直接复用）
pub(crate) fn ensure_png_bytes(raw: &[u8]) -> Result<Vec<u8>> {
    const PNG_MAGIC: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
    if raw.len() >= PNG_MAGIC.len() && &raw[..PNG_MAGIC.len()] == PNG_MAGIC {
        // 校验 PNG 头部有效性后再返回，防止损坏数据进入缓存
        image::load_from_memory_with_format(raw, ImageFormat::Png)?;
        return Ok(raw.to_vec());
    }

    let img = image::load_from_memory(raw)?;
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, ImageFormat::Png)?;
    Ok(out.into_inner())
}

fn build_handle_token() -> String {
    let pid = std::process::id();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("pinshot_{pid}_{nanos}")
}

fn build_portal_request_path(unique_name: &str, token: &str) -> String {
    let sender = unique_name
        .trim_start_matches(':')
        .replace('.', "_");
    format!("/org/freedesktop/portal/desktop/request/{sender}/{token}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_portal_request_path_converts_dbus_unique_name() {
        let path = build_portal_request_path(":1.245", "pinshot_42_100");
        assert_eq!(
            path,
            "/org/freedesktop/portal/desktop/request/1_245/pinshot_42_100"
        );
    }
}
