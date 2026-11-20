use std::collections::HashMap;
use std::path::PathBuf;

use zbus::blocking::Connection;
use zbus::zvariant::OwnedValue;

/// 桌面色彩主题
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorScheme {
    Light,
    Dark,
}

#[zbus::proxy(
    interface = "org.freedesktop.portal.Settings",
    default_service = "org.freedesktop.portal.Desktop",
    default_path = "/org/freedesktop/portal/desktop",
    gen_async = false,
    gen_blocking = true
)]
trait PortalSettings {
    fn read(&self, namespace: &str, key: &str) -> zbus::Result<OwnedValue>;
}

type MutterMonitorSpec = (String, String, String, String);
type MutterMode = (
    String,
    i32,
    i32,
    f64,
    f64,
    Vec<f64>,
    HashMap<String, OwnedValue>,
);
type MutterMonitor = (
    MutterMonitorSpec,
    Vec<MutterMode>,
    HashMap<String, OwnedValue>,
);
type MutterLogicalMonitor = (
    i32,
    i32,
    f64,
    u32,
    bool,
    Vec<MutterMonitorSpec>,
    HashMap<String, OwnedValue>,
);
type MutterDisplayState = (
    u32,
    Vec<MutterMonitor>,
    Vec<MutterLogicalMonitor>,
    HashMap<String, OwnedValue>,
);

#[zbus::proxy(
    interface = "org.gnome.Mutter.DisplayConfig",
    default_service = "org.gnome.Mutter.DisplayConfig",
    default_path = "/org/gnome/Mutter/DisplayConfig",
    gen_async = false,
    gen_blocking = true
)]
trait MutterDisplayConfig {
    fn get_current_state(&self) -> zbus::Result<MutterDisplayState>;
}

/// 通过 XDG Portal 读取当前系统深浅色偏好（`1` = 深色，其它回退浅色）
pub fn detect_color_scheme() -> ColorScheme {
    let Ok(conn) = Connection::session() else {
        return ColorScheme::Light;
    };
    let Ok(proxy) = PortalSettingsProxy::new(&conn) else {
        return ColorScheme::Light;
    };
    let Ok(outer) = proxy.read("org.freedesktop.appearance", "color-scheme") else {
        return ColorScheme::Light;
    };

    // Portal Settings.Read 返回外层 Variant 包裹的内层 Variant(u32)
    let code = extract_nested_u32(&outer).unwrap_or(0);
    if code == 1 {
        ColorScheme::Dark
    } else {
        ColorScheme::Light
    }
}

/// 探测主显示器缩放倍率（GNOME 通过 Mutter DisplayConfig 精确获取分数缩放，其他环境回退 1.0）
pub fn detect_display_scale() -> f64 {
    let Ok(conn) = Connection::session() else {
        return 1.0;
    };
    let Ok(proxy) = MutterDisplayConfigProxy::new(&conn) else {
        return 1.0;
    };
    let Ok((_serial, _monitors, logical_monitors, _props)) = proxy.get_current_state() else {
        return 1.0;
    };

    let selected = logical_monitors
        .iter()
        .find(|m| m.4)
        .or_else(|| logical_monitors.first());

    match selected {
        Some(monitor) if monitor.2.is_finite() && monitor.2 > 0.0 => monitor.2,
        _ => 1.0,
    }
}

/// 解码 `file://` URI 为本地文件系统路径（支持 `file:///...`、`file://localhost/...` 与 UTF-8 百分号编码）
pub fn decode_file_uri(uri: &str) -> Option<PathBuf> {
    let trimmed = uri.trim();
    let rest = if let Some(path) = trimmed.strip_prefix("file://localhost/") {
        format!("/{path}")
    } else {
        let path = trimmed.strip_prefix("file:///")?;
        format!("/{path}")
    };

    let decoded = percent_decode_utf8(&rest)?;
    Some(PathBuf::from(decoded))
}

fn extract_nested_u32(val: &OwnedValue) -> Option<u32> {
    if let Ok(n) = u32::try_from(val) {
        return Some(n);
    }
    if let zbus::zvariant::Value::Value(inner) = &**val {
        return u32::try_from(inner.as_ref()).ok();
    }
    None
}

fn percent_decode_utf8(input: &str) -> Option<String> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut idx = 0;

    while idx < bytes.len() {
        if bytes[idx] == b'%' {
            if idx + 2 >= bytes.len() {
                return None;
            }
            let hi = hex_nibble(bytes[idx + 1])?;
            let lo = hex_nibble(bytes[idx + 2])?;
            out.push((hi << 4) | lo);
            idx += 3;
        } else {
            out.push(bytes[idx]);
            idx += 1;
        }
    }

    String::from_utf8(out).ok()
}

fn hex_nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_file_uri_handles_spaces_and_chinese_characters() {
        let uri = "file:///home/daguo/Pictures/%E6%88%AA%E5%9B%BE%2001.png";
        let decoded = decode_file_uri(uri).expect("应成功解码中文与空格 URI");
        assert_eq!(decoded, PathBuf::from("/home/daguo/Pictures/截图 01.png"));
    }

    #[test]
    fn decode_file_uri_handles_localhost_and_rejects_non_file_schemes() {
        let local = decode_file_uri("file://localhost/tmp/test.png").expect("应支持 localhost");
        assert_eq!(local, PathBuf::from("/tmp/test.png"));

        assert!(decode_file_uri("https://example.com/a.png").is_none());
        assert!(decode_file_uri("file:///tmp/%ZZbad.png").is_none());
    }
}
