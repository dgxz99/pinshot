use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::capture::ensure_png_bytes;
use crate::desktop::decode_file_uri;
use crate::error::{PinshotError, Result};

/// 支持从 `text/uri-list` 直接贴出的本地图片扩展名
const SUPPORTED_IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "webp", "bmp"];

/// 剪贴板解析结果
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClipboardContent {
    /// 已统一转码为标准 PNG 的图片
    Image {
        png_bytes: Vec<u8>,
        title_preview: String,
    },
    /// 非空 UTF-8 纯文本
    Text(String),
    /// 剪贴板为空或无支持的内容类型
    Empty,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ClipboardCandidate {
    ImageMime(String),
    UriList,
    TextMime(String),
}

/// 读取当前 Wayland 剪贴板，按「图片 MIME > 本地图片 URI > 纯文本」优先级解析
pub fn read_clipboard() -> Result<ClipboardContent> {
    let types = list_clipboard_types()?;
    if types.is_empty() {
        return Ok(ClipboardContent::Empty);
    }

    for candidate in select_mime_candidates(&types) {
        match candidate {
            ClipboardCandidate::ImageMime(mime) => {
                if let Some(raw) = paste_bytes(&mime)?
                    && !raw.is_empty()
                {
                    let png_bytes = ensure_png_bytes(&raw)?;
                    return Ok(ClipboardContent::Image {
                        png_bytes,
                        title_preview: "剪贴板图片".to_string(),
                    });
                }
            }
            ClipboardCandidate::UriList => {
                if let Some(raw) = paste_bytes("text/uri-list")? {
                    let uri_text = String::from_utf8_lossy(&raw);
                    if let Some(path) = first_existing_image_path(&uri_text) {
                        let file_bytes = fs::read(&path).map_err(|e| PinshotError::io(&path, e))?;
                        let png_bytes = ensure_png_bytes(&file_bytes)?;
                        let title = path
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("本地图片")
                            .to_string();
                        return Ok(ClipboardContent::Image {
                            png_bytes,
                            title_preview: title,
                        });
                    }
                }
            }
            ClipboardCandidate::TextMime(mime) => {
                if let Some(raw) = paste_bytes(&mime)? {
                    let text = String::from_utf8_lossy(&raw).into_owned();
                    if !text.trim().is_empty() {
                        return Ok(ClipboardContent::Text(text));
                    }
                }
            }
        }
    }

    Ok(ClipboardContent::Empty)
}

/// 将 PNG 字节流写入 Wayland 剪贴板
pub fn copy_png_bytes(bytes: &[u8]) -> Result<()> {
    run_wl_copy("image/png", bytes)
}

/// 读取本地 PNG 文件并写入 Wayland 剪贴板
pub fn copy_png_file(path: &Path) -> Result<()> {
    let bytes = fs::read(path).map_err(|e| PinshotError::io(path, e))?;
    copy_png_bytes(&bytes)
}

/// 将 UTF-8 纯文本写入 Wayland 剪贴板
pub fn copy_text(text: &str) -> Result<()> {
    run_wl_copy("text/plain;charset=utf-8", text.as_bytes())
}

/// 调用 `wl-copy` 写入数据；重定向 stdout/stderr 为 null 以避免被后台 fork 子进程阻塞管道
fn run_wl_copy(mime: &str, data: &[u8]) -> Result<()> {
    let mut child = Command::new("wl-copy")
        .arg("--type")
        .arg(mime)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| PinshotError::ExternalCommand {
            cmd: "wl-copy".to_string(),
            detail: e.to_string(),
        })?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(data)
            .map_err(|e| PinshotError::ExternalCommand {
                cmd: "wl-copy".to_string(),
                detail: format!("写入标准输入失败: {e}"),
            })?;
    }

    let status = child.wait().map_err(|e| PinshotError::ExternalCommand {
        cmd: "wl-copy".to_string(),
        detail: e.to_string(),
    })?;

    if !status.success() {
        return Err(PinshotError::ExternalCommand {
            cmd: "wl-copy".to_string(),
            detail: format!("退出码: {status}"),
        });
    }

    Ok(())
}

fn list_clipboard_types() -> Result<Vec<String>> {
    let output = Command::new("wl-paste")
        .arg("--list-types")
        .output()
        .map_err(|e| PinshotError::ExternalCommand {
            cmd: "wl-paste --list-types".to_string(),
            detail: e.to_string(),
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // 剪贴板为空或当前无可用数据时，wl-paste 返回非零退出码，此处视为 Empty
        if stderr.contains("Nothing is copied") || stderr.contains("No selection") || stderr.is_empty() {
            return Ok(Vec::new());
        }
        return Err(PinshotError::ExternalCommand {
            cmd: "wl-paste --list-types".to_string(),
            detail: stderr.trim().to_string(),
        });
    }

    let types = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToString::to_string)
        .collect();
    Ok(types)
}

fn paste_bytes(mime: &str) -> Result<Option<Vec<u8>>> {
    let output = Command::new("wl-paste")
        .arg("--no-newline")
        .arg("--type")
        .arg(mime)
        .output()
        .map_err(|e| PinshotError::ExternalCommand {
            cmd: format!("wl-paste --type {mime}"),
            detail: e.to_string(),
        })?;

    if !output.status.success() {
        return Ok(None);
    }

    Ok(Some(output.stdout))
}

/// 按架构约定优先级生成剪贴板探测候选序列
fn select_mime_candidates(available: &[String]) -> Vec<ClipboardCandidate> {
    let mut result = Vec::new();
    let has = |target: &str| available.iter().any(|t| t.eq_ignore_ascii_case(target));

    for img_mime in ["image/png", "image/jpeg", "image/webp", "image/bmp"] {
        if has(img_mime) {
            result.push(ClipboardCandidate::ImageMime(img_mime.to_string()));
        }
    }

    if has("text/uri-list") {
        result.push(ClipboardCandidate::UriList);
    }

    for text_mime in ["text/plain;charset=utf-8", "text/plain"] {
        if has(text_mime) {
            result.push(ClipboardCandidate::TextMime(text_mime.to_string()));
        }
    }

    result
}

/// 从 `text/uri-list` 内容中提取第一个存在且为受支持格式的本地图片路径
fn first_existing_image_path(uri_list: &str) -> Option<PathBuf> {
    for line in uri_list.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let Some(path) = decode_file_uri(trimmed) else {
            continue;
        };
        if path.is_file() && is_supported_image_path(&path) {
            return Some(path);
        }
    }
    None
}

fn is_supported_image_path(path: &Path) -> bool {
    let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
        return false;
    };
    SUPPORTED_IMAGE_EXTENSIONS
        .iter()
        .any(|supported| ext.eq_ignore_ascii_case(supported))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn mime_priority_prefers_png_then_other_images_then_uri_then_text() {
        let available = vec![
            "text/plain".to_string(),
            "text/uri-list".to_string(),
            "image/jpeg".to_string(),
            "text/plain;charset=utf-8".to_string(),
            "image/png".to_string(),
        ];

        let order = select_mime_candidates(&available);
        assert_eq!(
            order,
            vec![
                ClipboardCandidate::ImageMime("image/png".to_string()),
                ClipboardCandidate::ImageMime("image/jpeg".to_string()),
                ClipboardCandidate::UriList,
                ClipboardCandidate::TextMime("text/plain;charset=utf-8".to_string()),
                ClipboardCandidate::TextMime("text/plain".to_string()),
            ]
        );
    }

    #[test]
    fn first_existing_image_path_skips_comments_and_non_images() {
        let dir = TempDir::new().expect("创建临时目录失败");
        let txt = dir.path().join("note.txt");
        let img = dir.path().join("photo 01.PNG");
        fs::write(&txt, "hello").unwrap();
        fs::write(&img, b"fake").unwrap();

        let uri_list = format!(
            "# comment\nfile://{}\nfile://{}\n",
            txt.display(),
            img.to_string_lossy().replace(' ', "%20")
        );

        let found = first_existing_image_path(&uri_list).expect("应找到本地图片路径");
        assert_eq!(found, img);
    }
}
