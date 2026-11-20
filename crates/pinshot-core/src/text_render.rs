use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::config::{TextCardConfig, TextCardTheme};
use crate::desktop::{detect_color_scheme, detect_display_scale, ColorScheme};
use crate::error::{PinshotError, Result};

/// 渲染完成后的文字卡片结果
#[derive(Debug, Clone)]
pub struct RenderedTextCard {
    pub png_bytes: Vec<u8>,
    pub title_preview: String,
    pub source_scale: f64,
    pub is_code: bool,
}

#[derive(Debug, Clone, Copy)]
struct CardPalette {
    background: &'static str,
    foreground: &'static str,
    muted: &'static str,
}

const DARK_PALETTE: CardPalette = CardPalette {
    background: "#1e1f29",
    foreground: "#e6eaf2",
    muted: "#8b94a8",
};

const LIGHT_PALETTE: CardPalette = CardPalette {
    background: "#fafbfc",
    foreground: "#1f2328",
    muted: "#656d76",
};

/// 将纯文本渲染为高分屏清晰 PNG 卡片（自动检测代码特征、转义与超长截断）
pub fn render_text_card(raw_text: &str, cfg: &TextCardConfig) -> Result<RenderedTextCard> {
    let source_scale = detect_display_scale();
    let scheme = match cfg.theme {
        TextCardTheme::Auto => detect_color_scheme(),
        TextCardTheme::Dark => ColorScheme::Dark,
        TextCardTheme::Light => ColorScheme::Light,
    };
    render_text_card_with_env(raw_text, cfg, scheme, source_scale)
}

/// 可注入主题与缩放倍率的渲染入口（便于测试与外部指定环境参数）
pub fn render_text_card_with_env(
    raw_text: &str,
    cfg: &TextCardConfig,
    scheme: ColorScheme,
    source_scale: f64,
) -> Result<RenderedTextCard> {
    let normalized = normalize_text(raw_text);
    let is_code = detect_code(&normalized);
    let title_preview = build_title_preview(&normalized);

    let (body, truncated_total) = truncate_lines(&normalized, cfg.max_lines);
    let palette = match scheme {
        ColorScheme::Dark => DARK_PALETTE,
        ColorScheme::Light => LIGHT_PALETTE,
    };
    let markup = build_pango_markup(&body, truncated_total, palette.muted);

    let scale = if source_scale.is_finite() && source_scale > 0.0 {
        source_scale
    } else {
        1.0
    };
    let dpi = (96.0 * scale).round().max(72.0) as u32;
    // pango-view 的 --width 单位为 points：points * dpi / 72 == max_width_px * scale
    let width_pt = ((cfg.max_width_px as f64) * 72.0 / 96.0).round().max(120.0) as u32;
    let margin_px = ((cfg.padding_px as f64) * scale).round().max(8.0) as u32;
    let font = if is_code {
        cfg.code_font.as_str()
    } else {
        cfg.prose_font.as_str()
    };

    let guard = TempRenderFiles::create()?;
    fs::write(&guard.input_path, markup.as_bytes())
        .map_err(|e| PinshotError::io(&guard.input_path, e))?;

    let output = Command::new("pango-view")
        .arg("--no-display")
        .arg("--markup")
        .arg("--wrap=word-char")
        .arg(format!("--width={width_pt}"))
        .arg(format!("--margin={margin_px}"))
        .arg(format!("--dpi={dpi}"))
        .arg(format!("--background={}", palette.background))
        .arg(format!("--foreground={}", palette.foreground))
        .arg(format!("--font={font}"))
        .arg("--line-spacing=1.22")
        .arg("-o")
        .arg(&guard.output_path)
        .arg(&guard.input_path)
        .output()
        .map_err(|e| PinshotError::ExternalCommand {
            cmd: "pango-view".to_string(),
            detail: e.to_string(),
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(PinshotError::ExternalCommand {
            cmd: "pango-view".to_string(),
            detail: stderr,
        });
    }

    let png_bytes =
        fs::read(&guard.output_path).map_err(|e| PinshotError::io(&guard.output_path, e))?;

    Ok(RenderedTextCard {
        png_bytes,
        title_preview,
        source_scale: scale,
        is_code,
    })
}

/// 统一换行符、将 Tab 转为 4 空格并剔除尾部多余空行
pub fn normalize_text(raw: &str) -> String {
    let unified = raw.replace("\r\n", "\n").replace('\r', "\n");
    let expanded = unified.replace('\t', "    ");
    expanded.trim_end_matches('\n').to_string()
}

/// 基于缩进比例、代码关键字与符号密度自动判断文本是否为代码
pub fn detect_code(text: &str) -> bool {
    let non_empty_lines: Vec<&str> = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    if non_empty_lines.is_empty() {
        return false;
    }

    const CODE_PREFIXES: &[&str] = &[
        "fn ",
        "pub ",
        "let ",
        "const ",
        "var ",
        "func ",
        "function ",
        "async ",
        "def ",
        "class ",
        "import ",
        "export ",
        "from ",
        "use ",
        "struct ",
        "enum ",
        "impl ",
        "trait ",
        "return ",
        "SELECT ",
        "INSERT ",
        "UPDATE ",
        "DELETE ",
        "#include",
        "package ",
        "#!/",
        "curl ",
        "git ",
        "cargo ",
        "sudo ",
    ];

    const CODE_TOKENS: &[&str] = &[
        "{", "}", "();", "=>", "::", "->", "!=", "==", "&&", "||", "</", "[]",
    ];

    let mut indented_lines = 0usize;
    let mut keyword_hits = 0usize;
    let mut semicolon_or_brace_ends = 0usize;

    for line in &non_empty_lines {
        if line.starts_with("  ") {
            indented_lines += 1;
        }
        let trimmed = line.trim();
        if CODE_PREFIXES.iter().any(|prefix| trimmed.starts_with(prefix)) {
            keyword_hits += 1;
        }
        if trimmed.ends_with(';') || trimmed.ends_with('{') || trimmed.ends_with('}') {
            semicolon_or_brace_ends += 1;
        }
    }

    let token_hits: usize = CODE_TOKENS.iter().map(|tok| text.matches(tok).count()).sum();
    let total_lines = non_empty_lines.len();

    if keyword_hits >= 1 && (token_hits >= 1 || total_lines >= 2) {
        return true;
    }
    if total_lines >= 3 && indented_lines * 100 / total_lines >= 35 && token_hits >= 2 {
        return true;
    }
    if semicolon_or_brace_ends >= 2 && token_hits >= 2 {
        return true;
    }
    if token_hits >= 4 {
        return true;
    }

    false
}

/// 按最大行数截断展示内容；超出时返回总行数以便追加截断提示
pub fn truncate_lines(text: &str, max_lines: usize) -> (String, Option<usize>) {
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() <= max_lines {
        return (text.to_string(), None);
    }

    let kept = lines[..max_lines].join("\n");
    (kept, Some(lines.len()))
}

/// 转义 Pango XML 特殊字符并剔除非换行控制字符，防止 `pango-view --markup` 报错
pub fn escape_pango_markup(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 32);
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            '\n' => out.push('\n'),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

fn build_pango_markup(body: &str, truncated_total: Option<usize>, muted_color: &str) -> String {
    let mut markup = escape_pango_markup(body);
    if let Some(total) = truncated_total {
        markup.push_str(&format!(
            "\n\n<span foreground=\"{muted_color}\" size=\"smaller\">… 已截断，共 {total} 行（右键可复制完整原文）</span>"
        ));
    }
    markup
}

/// 提取首行非空文本作为顶栏与历史摘要（最多 40 个字符）
pub fn build_title_preview(text: &str) -> String {
    let first = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("文本卡片");

    let max_chars = 40;
    let mut chars = first.chars();
    let prefix: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{prefix}…")
    } else {
        prefix
    }
}

struct TempRenderFiles {
    input_path: PathBuf,
    output_path: PathBuf,
}

impl TempRenderFiles {
    fn create() -> Result<Self> {
        let dir = std::env::temp_dir();
        let pid = std::process::id();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let input_path = dir.join(format!("pinshot-text-{pid}-{nanos}.pango"));
        let output_path = dir.join(format!("pinshot-text-{pid}-{nanos}.png"));
        Ok(Self {
            input_path,
            output_path,
        })
    }
}

impl Drop for TempRenderFiles {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.input_path);
        let _ = fs::remove_file(&self.output_path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::GenericImageView;

    #[test]
    fn detect_code_distinguishes_code_and_prose() {
        let rust_code = "fn main() {\n    let x = 42;\n    println!(\"{x}\");\n}";
        assert!(detect_code(rust_code));

        let json_code = "{\n  \"name\": \"pinshot\",\n  \"scale\": 1.5\n}";
        assert!(detect_code(json_code));

        let prose_zh = "这是一段普通的中文笔记。\n今天下午三点开会，需要准备架构演示材料。";
        assert!(!detect_code(prose_zh));

        let prose_en = "Meeting notes for Friday.\nPlease review the updated roadmap before noon.";
        assert!(!detect_code(prose_en));
    }

    #[test]
    fn escape_pango_markup_escapes_xml_entities_and_strips_control_chars() {
        let raw = "a < b && c > d \"ok\" 'x'\x00\x07\nline2";
        assert_eq!(
            escape_pango_markup(raw),
            "a &lt; b &amp;&amp; c &gt; d &quot;ok&quot; &apos;x&apos;\nline2"
        );
    }

    #[test]
    fn truncate_lines_caps_and_reports_total_count() {
        let content = (1..=100)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let (body, total) = truncate_lines(&content, 80);
        assert_eq!(body.lines().count(), 80);
        assert_eq!(total, Some(100));
    }

    #[test]
    fn build_title_preview_handles_multibyte_chinese_safely() {
        let long = "这是一段非常非常长的中文标题摘要内容用于验证按字符截断而不是按字节截断的安全性检查";
        let preview = build_title_preview(long);
        assert!(preview.ends_with('…'));
        assert_eq!(preview.chars().count(), 41);
    }

    #[test]
    fn pango_view_renders_valid_png_for_code_and_chinese() {
        let cfg = TextCardConfig::default();
        let text = "fn greet() {\n    println!(\"你好 Pinshot <Wayland> & Rust\");\n}";
        let rendered = render_text_card_with_env(text, &cfg, ColorScheme::Dark, 1.5)
            .expect("调用 pango-view 渲染文字卡片失败");

        assert!(rendered.is_code);
        assert_eq!(rendered.source_scale, 1.5);
        assert_eq!(rendered.title_preview, "fn greet() {");

        let img = image::load_from_memory(&rendered.png_bytes).expect("渲染结果应为有效 PNG");
        let (w, h) = img.dimensions();
        assert!(w > 100 && h > 30, "渲染尺寸异常: {w}x{h}");
    }
}
