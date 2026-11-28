use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::capture::ensure_png_bytes;
use crate::clipboard_png;
use crate::dbus_client::FrontendClient;
use crate::desktop::decode_file_uri;
use crate::error::{PinshotError, Result};

/// 支持从 `text/uri-list` 直接贴出的本地图片扩展名
const SUPPORTED_IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "webp", "bmp"];

/// 剪贴板解析结果
#[derive(Debug, Clone, PartialEq)]
pub enum ClipboardContent {
    /// 已统一转码为标准 PNG 的图片
    Image {
        png_bytes: Vec<u8>,
        title_preview: String,
        /// Pinshot 复制时记录的像素与逻辑尺寸之比；外部图片通常没有此值
        source_scale: Option<f64>,
    },
    /// 非空 UTF-8 文本（附带可选的 `text/html` 富文本，保留 IDE / 浏览器代码样式与颜色）
    Text { plain: String, html: Option<String> },
    /// 剪贴板为空或无支持的内容类型
    Empty,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ClipboardCandidate {
    ImageMime(String),
    UriList(String),
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
                    let source_scale = clipboard_png::source_scale(&raw);
                    let png_bytes = ensure_png_bytes(&raw)?;
                    return Ok(ClipboardContent::Image {
                        png_bytes,
                        title_preview: "剪贴板图片".to_string(),
                        source_scale,
                    });
                }
            }
            ClipboardCandidate::UriList(mime) => {
                if let Some(raw) = paste_bytes(&mime)? {
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
                            source_scale: None,
                        });
                    }
                }
            }
            ClipboardCandidate::TextMime(mime) => {
                if let Some(raw) = paste_bytes(&mime)? {
                    let plain = String::from_utf8_lossy(&raw).into_owned();
                    if !plain.trim().is_empty() {
                        let html = read_optional_html(&types, &plain)?;
                        return Ok(ClipboardContent::Text { plain, html });
                    }
                }
            }
        }
    }

    Ok(ClipboardContent::Empty)
}

/// 将 PNG 写入 Wayland 剪贴板，并保留重新贴图所需的原始倍率
pub fn copy_png_bytes(frontend: &FrontendClient, bytes: &[u8], source_scale: f64) -> Result<()> {
    let tagged = clipboard_png::with_source_scale(bytes, source_scale);
    let png = tagged.as_deref().unwrap_or(bytes);
    if frontend.set_clipboard_png(png)? {
        return Ok(());
    }
    run_wl_copy("image/png", png)
}

/// 读取本地 PNG 文件并带上倍率写入 Wayland 剪贴板
pub fn copy_png_file(frontend: &FrontendClient, path: &Path, source_scale: f64) -> Result<()> {
    let bytes = fs::read(path).map_err(|e| PinshotError::io(path, e))?;
    copy_png_bytes(frontend, &bytes, source_scale)
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
        if stderr.contains("Nothing is copied")
            || stderr.contains("No selection")
            || stderr.is_empty()
        {
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

fn read_optional_html(available: &[String], plain: &str) -> Result<Option<String>> {
    // 必须使用 `available` 中的原始 MIME 字符串（wl-paste --type 区分大小写与空格）
    // 优先选择显式声明 UTF-8 / UTF-16 / Unicode 的 HTML 变体：
    // IntelliJ IDEA / Java AWT 的裸 `text/html` 默认按 ISO-8859-1 编码，会把中文变成 `?`。
    for mime in select_html_mimes(available) {
        let Some(raw) = paste_bytes(mime)? else {
            continue;
        };
        if raw.is_empty() {
            continue;
        }
        let html = decode_html_bytes(&raw, mime);
        if html.trim().is_empty() {
            continue;
        }
        // 若纯文本含中文等非 ASCII 字符，但当前 HTML 变体把它们全损毁成 `?`，则跳过继续尝试下一个 Unicode 变体
        if html_lost_non_ascii(&html, plain) {
            continue;
        }
        return Ok(Some(html));
    }
    Ok(None)
}

/// 从 `wl-paste --list-types` 原始列表中筛选所有 `text/html*` MIME，保留原始大小写并按 Unicode 保真度排序
fn select_html_mimes(available: &[String]) -> Vec<&str> {
    let mut ranked: Vec<(u8, usize, &str)> = available
        .iter()
        .enumerate()
        .filter_map(|(idx, raw)| {
            let trimmed = raw.trim();
            let compact: String = trimmed
                .chars()
                .filter(|c| !c.is_ascii_whitespace())
                .map(|c| c.to_ascii_lowercase())
                .collect();
            let has_utf8 = compact.contains("charset=utf-8") || compact.contains("charset=utf8");
            let has_utf16 = compact.contains("charset=utf-16")
                || compact.contains("charset=utf16")
                || compact.contains("charset=unicode");
            let priority =
                if compact == "text/html;charset=utf-8" || compact == "text/html;charset=utf8" {
                    0
                } else if compact.starts_with("text/html;") && has_utf8 {
                    1
                } else if compact.starts_with("text/html;") && has_utf16 {
                    2
                } else if compact == "text/html" {
                    3
                } else if compact.starts_with("text/html;") {
                    4
                } else {
                    return None;
                };
            Some((priority, idx, raw.as_str()))
        })
        .collect();
    ranked.sort_by_key(|&(prio, idx, _)| (prio, idx));
    ranked.into_iter().map(|(_, _, mime)| mime).collect()
}

/// 检测 HTML 变体是否因 ISO-8859-1 / US-ASCII 编码导致中文等非 ASCII 字符丢失为 `?`
fn html_lost_non_ascii(html: &str, plain: &str) -> bool {
    let plain_non_ascii = plain.chars().filter(|c| !c.is_ascii()).count();
    if plain_non_ascii == 0 {
        return false;
    }
    if !html.is_ascii() || html.contains("&#") {
        return false;
    }
    let plain_q = plain.bytes().filter(|&b| b == b'?').count();
    let html_q = html.bytes().filter(|&b| b == b'?').count();
    html_q >= plain_q + plain_non_ascii.div_ceil(2)
}

/// 解码剪贴板 HTML 字节（支持 UTF-8、UTF-16LE/BE 有无 BOM 及 Windows CF_HTML 头部）
fn decode_html_bytes(raw: &[u8], mime: &str) -> String {
    let text = if raw.starts_with(&[0xFF, 0xFE]) && raw.len() >= 4 {
        decode_utf16_le(&raw[2..])
    } else if raw.starts_with(&[0xFE, 0xFF]) && raw.len() >= 4 {
        decode_utf16_be(&raw[2..])
    } else if raw.len() >= 4 && raw.len().is_multiple_of(2) {
        let lower_mime = mime.to_ascii_lowercase();
        let even_zeros = raw.iter().step_by(2).filter(|&&b| b == 0).count();
        let odd_zeros = raw[1..].iter().step_by(2).filter(|&&b| b == 0).count();
        let hinted_utf16 = lower_mime.contains("utf-16")
            || lower_mime.contains("utf16")
            || lower_mime.contains("unicode");
        if (hinted_utf16 && lower_mime.contains("le"))
            || (odd_zeros * 4 >= raw.len() && odd_zeros > even_zeros * 2)
        {
            decode_utf16_le(raw)
        } else if (hinted_utf16 && lower_mime.contains("be"))
            || (even_zeros * 4 >= raw.len() && even_zeros > odd_zeros * 2)
        {
            decode_utf16_be(raw)
        } else {
            String::from_utf8_lossy(raw).into_owned()
        }
    } else {
        String::from_utf8_lossy(raw).into_owned()
    };

    let cleaned = text.trim_matches('\0');
    // 剥离部分应用（如 Electron / Wine / CF_HTML）自带的 Version:0.9 头部
    if cleaned.starts_with("Version:")
        && let Some(pos) = cleaned.find('<')
    {
        return cleaned[pos..].to_string();
    }
    cleaned.to_string()
}

fn decode_utf16_le(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|&c| u16::from_le_bytes(c))
        .collect();
    String::from_utf16_lossy(&units)
}

fn decode_utf16_be(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|&c| u16::from_be_bytes(c))
        .collect();
    String::from_utf16_lossy(&units)
}

/// 按架构约定优先级生成剪贴板探测候选序列（保留 `available` 中的原始 MIME 大小写）
fn select_mime_candidates(available: &[String]) -> Vec<ClipboardCandidate> {
    let mut result = Vec::new();
    let find_exact = |target: &str| {
        available
            .iter()
            .find(|t| t.eq_ignore_ascii_case(target))
            .cloned()
    };

    for img_mime in ["image/png", "image/jpeg", "image/webp", "image/bmp"] {
        if let Some(actual) = find_exact(img_mime) {
            result.push(ClipboardCandidate::ImageMime(actual));
        }
    }

    if let Some(actual) = find_exact("text/uri-list") {
        result.push(ClipboardCandidate::UriList(actual));
    }

    for text_mime in ["text/plain;charset=utf-8", "text/plain"] {
        if let Some(actual) = find_exact(text_mime) {
            result.push(ClipboardCandidate::TextMime(actual));
        }
    }
    // 兜底匹配带空格的 `text/plain; charset=utf-8` 等变体
    for raw in available {
        let compact: String = raw
            .chars()
            .filter(|c| !c.is_ascii_whitespace())
            .map(|c| c.to_ascii_lowercase())
            .collect();
        if compact.starts_with("text/plain;")
            && (compact.contains("charset=utf-8") || compact.contains("charset=utf8"))
            && !result
                .iter()
                .any(|c| matches!(c, ClipboardCandidate::TextMime(m) if m == raw))
        {
            result.push(ClipboardCandidate::TextMime(raw.clone()));
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
                ClipboardCandidate::UriList("text/uri-list".to_string()),
                ClipboardCandidate::TextMime("text/plain;charset=utf-8".to_string()),
                ClipboardCandidate::TextMime("text/plain".to_string()),
            ]
        );
    }

    #[test]
    fn select_html_mimes_preserves_exact_case_and_ranks_idea_java_flavors() {
        let available = vec![
            "text/plain;charset=UTF-8".to_string(),
            "text/html;document=selection;class=java.lang.String;charset=utf-8".to_string(),
            "text/html;charset=UTF-8".to_string(),
            "text/html".to_string(),
            "text/html;charset=unicode".to_string(),
        ];

        assert_eq!(
            select_html_mimes(&available),
            vec![
                "text/html;charset=UTF-8",
                "text/html;document=selection;class=java.lang.String;charset=utf-8",
                "text/html;charset=unicode",
                "text/html",
            ]
        );

        let spaced = vec!["text/html; charset=UTF-8".to_string()];
        assert_eq!(select_html_mimes(&spaced), vec!["text/html; charset=UTF-8"]);
    }

    #[test]
    fn detects_iso8859_chinese_loss_and_decodes_utf16_be_le_html() {
        let plain = "// 禁用 Jakarta Mail 生成 Message-ID";
        let corrupted = "<pre>// ?? Jakarta Mail ?? Message-ID</pre>";
        assert!(html_lost_non_ascii(corrupted, plain));

        let valid_utf8 = "<pre>// 禁用 Jakarta Mail 生成 Message-ID</pre>";
        assert!(!html_lost_non_ascii(valid_utf8, plain));

        let valid_entities = "<pre>// &#31105;&#29992; Jakarta Mail</pre>";
        assert!(!html_lost_non_ascii(valid_entities, plain));

        // 验证无 BOM 的 UTF-16BE / UTF-16LE 中文 HTML 均可无损解码
        let be_bytes: Vec<u8> = valid_utf8
            .encode_utf16()
            .flat_map(u16::to_be_bytes)
            .collect();
        assert_eq!(
            decode_html_bytes(&be_bytes, "text/html;charset=utf-16be"),
            valid_utf8
        );
        let le_bytes: Vec<u8> = valid_utf8
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        assert_eq!(
            decode_html_bytes(&le_bytes, "text/html;charset=unicode"),
            valid_utf8
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
