use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::OnceLock;

use image::{DynamicImage, GenericImageView, ImageFormat};

use crate::config::{TextCardConfig, TextCardTheme};
use crate::desktop::{ColorScheme, detect_color_scheme, detect_display_scale};
use crate::error::{PinshotError, Result};
use crate::rich_text;

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

/// 将纯文本渲染为高分屏清晰 PNG 卡片（动态贴合内容宽度，超出上限自动折行）
pub fn render_text_card(raw_text: &str, cfg: &TextCardConfig) -> Result<RenderedTextCard> {
    render_clipboard_text_card(raw_text, None, cfg)
}

/// 将剪贴板文本（含可选 `text/html` 富文本）渲染为动态尺寸 PNG 卡片：
/// - 优先还原 IDEA / VS Code / 浏览器 `text/html` 的配色与样式，异常时自动回退纯文本
/// - 短文本按实际内容紧凑排版，仅超出宽度上限时自动折行
pub fn render_clipboard_text_card(
    raw_text: &str,
    html: Option<&str>,
    cfg: &TextCardConfig,
) -> Result<RenderedTextCard> {
    let source_scale = detect_display_scale();
    let scheme = match cfg.theme {
        TextCardTheme::Auto => detect_color_scheme(),
        TextCardTheme::Dark => ColorScheme::Dark,
        TextCardTheme::Light => ColorScheme::Light,
    };
    render_text_card_with_env(raw_text, html, cfg, scheme, source_scale)
}

/// 可注入主题与缩放倍率的渲染入口（便于测试与外部指定环境参数）
fn render_text_card_with_env(
    raw_text: &str,
    html: Option<&str>,
    cfg: &TextCardConfig,
    scheme: ColorScheme,
    source_scale: f64,
) -> Result<RenderedTextCard> {
    let normalized = normalize_text(raw_text);
    let title_preview = build_title_preview(&normalized);
    let palette = match scheme {
        ColorScheme::Dark => DARK_PALETTE,
        ColorScheme::Light => LIGHT_PALETTE,
    };

    let rich = html.and_then(|h| rich_text::parse_clipboard_html(h, &normalized));
    let is_code = rich.as_ref().is_some_and(|d| d.is_code) || detect_code(&normalized);

    let (bg_color, fg_color, markup) = if let Some(ref doc) = rich {
        let bg = doc.root_bg.as_deref().unwrap_or(palette.background);
        let fg = doc.root_fg.as_deref().unwrap_or(palette.foreground);
        let markup =
            rich_text::render_rich_to_pango_markup(doc, cfg.max_lines, bg, fg, palette.muted);
        (bg.to_string(), fg.to_string(), markup)
    } else {
        let display_text = if is_code {
            dedent_plain_code(&normalized)
        } else {
            normalized.clone()
        };
        let (body, truncated_total) = truncate_lines(&display_text, cfg.max_lines);
        let markup = build_pango_markup(&body, truncated_total, palette.muted);
        (
            palette.background.to_string(),
            palette.foreground.to_string(),
            markup,
        )
    };

    let scale = if source_scale.is_finite() && source_scale > 0.0 {
        source_scale
    } else {
        1.0
    };
    let dpi = (96.0 * scale).round().max(72.0) as u32;
    // 代码卡片适当放宽折行上限，避免 80 列 IDE 代码被过早折断
    let max_logical_width = if is_code {
        cfg.max_width_px.max(960)
    } else {
        cfg.max_width_px
    };
    let max_pixel_width = ((max_logical_width as f64) * scale).round().max(160.0) as u32;
    let width_pt = ((max_logical_width as f64) * 72.0 / 96.0)
        .round()
        .max(120.0) as u32;
    let margin_px = ((cfg.padding_px as f64) * scale).round().max(8.0) as u32;
    let resolved_font = if is_code {
        resolve_code_font(&cfg.code_font)
    } else {
        cfg.prose_font.clone()
    };
    let font = resolved_font.as_str();

    let temp = tempfile::tempdir().map_err(|e| PinshotError::io(std::env::temp_dir(), e))?;
    let input_path = temp.path().join("card.pango");
    let output_path = temp.path().join("card.png");
    fs::write(&input_path, markup.as_bytes()).map_err(|e| PinshotError::io(&input_path, e))?;

    // 第一遍：按内容自然宽度紧凑渲染（不传 --width），短文本不再留大片右侧空白
    // 仅当首遍成功生成有效 PNG 且未超出宽度上限时直接复用；若超宽或触及 Cairo 单边上限产出空文件则走限宽折行路径
    let first_fit = run_pango_view(
        &input_path,
        &output_path,
        None,
        margin_px,
        dpi,
        &bg_color,
        &fg_color,
        font,
    )
    .ok()
    .and_then(|()| fs::read(&output_path).ok())
    .and_then(|bytes| {
        let width = image::load_from_memory(&bytes)
            .ok()
            .map(|img| img.dimensions().0)?;
        (width > 0 && width <= max_pixel_width + margin_px * 2).then_some(bytes)
    });

    let png_bytes = match first_fit {
        Some(bytes) => bytes,
        None => {
            run_pango_view(
                &input_path,
                &output_path,
                Some(width_pt),
                margin_px,
                dpi,
                &bg_color,
                &fg_color,
                font,
            )?;
            let wrapped = fs::read(&output_path).map_err(|e| PinshotError::io(&output_path, e))?;
            let img = image::load_from_memory(&wrapped)?;
            trim_wrapped_right_margin(&img, &bg_color, margin_px)?.unwrap_or(wrapped)
        }
    };

    Ok(RenderedTextCard {
        png_bytes,
        title_preview,
        source_scale: scale,
        is_code,
    })
}

#[allow(clippy::too_many_arguments)]
fn run_pango_view(
    input_path: &Path,
    output_path: &Path,
    width_pt: Option<u32>,
    margin_px: u32,
    dpi: u32,
    background: &str,
    foreground: &str,
    font: &str,
) -> Result<()> {
    let mut cmd = Command::new("pango-view");
    cmd.arg("--no-display").arg("--markup");
    if let Some(w) = width_pt {
        cmd.arg("--wrap=word-char").arg(format!("--width={w}"));
    }
    let output = cmd
        .arg(format!("--margin={margin_px}"))
        .arg(format!("--dpi={dpi}"))
        .arg(format!("--background={background}"))
        .arg(format!("--foreground={foreground}"))
        .arg(format!("--font={font}"))
        .arg("--line-spacing=1.22")
        .arg("-o")
        .arg(output_path)
        .arg(input_path)
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
    Ok(())
}

/// `pango-view --width=...` 折行后会把画布固定撑满整个折行上限宽度；
/// 当按词折行导致实际每行明显更短时（例如长 token 被整体折到下一行），
/// 裁掉右侧多余纯底色留白，仅保留对称的 `margin_px` 右边距。
fn trim_wrapped_right_margin(
    img: &DynamicImage,
    bg_hex: &str,
    margin_px: u32,
) -> Result<Option<Vec<u8>>> {
    let Some(bg) = parse_hex_rgb(bg_hex) else {
        return Ok(None);
    };
    let rgb = img.to_rgb8();
    let (width, height) = rgb.dimensions();
    if width == 0 || height == 0 {
        return Ok(None);
    }

    let mut max_ink_x: Option<u32> = None;
    'outer: for x in (0..width).rev() {
        for y in 0..height {
            if rgb.get_pixel(x, y).0 != bg {
                max_ink_x = Some(x);
                break 'outer;
            }
        }
    }

    let Some(rightmost) = max_ink_x else {
        return Ok(None);
    };
    let tight_width = rightmost.saturating_add(1).saturating_add(margin_px);
    if tight_width >= width {
        return Ok(None);
    }

    let cropped = img.crop_imm(0, 0, tight_width, height);
    let mut cursor = std::io::Cursor::new(Vec::new());
    cropped.write_to(&mut cursor, ImageFormat::Png)?;
    Ok(Some(cursor.into_inner()))
}

fn parse_hex_rgb(hex: &str) -> Option<[u8; 3]> {
    let h = hex.strip_prefix('#')?;
    if h.len() != 6 {
        return None;
    }
    Some([
        u8::from_str_radix(&h[0..2], 16).ok()?,
        u8::from_str_radix(&h[2..4], 16).ok()?,
        u8::from_str_radix(&h[4..6], 16).ok()?,
    ])
}

/// 统一换行符、将 Tab 转为 4 空格并剔除尾部多余空行
fn normalize_text(raw: &str) -> String {
    let unified = raw.replace("\r\n", "\n").replace('\r', "\n");
    let expanded = unified.replace('\t', "    ");
    expanded.trim_end_matches('\n').to_string()
}

/// 去除多行纯文本代码块的公共左侧冗余缩进（仅用于渲染，不改动保存的原始纯文本）
fn dedent_plain_code(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let non_blank: Vec<&&str> = lines.iter().filter(|l| !l.trim().is_empty()).collect();
    if non_blank.len() < 2 {
        return text.to_string();
    }
    let min_indent = non_blank
        .iter()
        .map(|l| l.bytes().take_while(|&b| b == b' ').count())
        .min()
        .unwrap_or(0);
    if min_indent == 0 {
        return text.to_string();
    }
    lines
        .into_iter()
        .map(|l| {
            if l.trim().is_empty() {
                ""
            } else {
                &l[min_indent.min(l.len())..]
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// 在中文 locale（如 `zh_CN.UTF-8`）下，fontconfig 往往把通用 `Monospace` 优先映射到新宋体（SimSun），
/// 导致仅含标点（如 `}`）或中文注释的整行缩进宽度与字形与英文代码行错位。
/// 当配置使用通用 `Monospace` 时，优先把系统英文等宽字体（`monospace:lang=en`）置前，
/// 让所有 ASCII 空格与符号统一使用等宽字体，同时保留 `Monospace` 作为中文字体回退。
fn resolve_code_font(configured: &str) -> String {
    let trimmed = configured.trim();
    let lower = trimmed.to_ascii_lowercase();
    let is_generic_mono =
        lower == "monospace" || lower.starts_with("monospace ") || lower.starts_with("monospace,");
    if !is_generic_mono {
        return trimmed.to_string();
    }
    let Some(latin_mono) = detect_latin_monospace_family() else {
        return trimmed.to_string();
    };
    if let Some(cjk_sans) = detect_cjk_sans_family() {
        format!("{latin_mono}, {cjk_sans}, {trimmed}")
    } else {
        format!("{latin_mono}, {trimmed}")
    }
}

fn detect_latin_monospace_family() -> Option<&'static str> {
    static CACHE: OnceLock<Option<String>> = OnceLock::new();
    CACHE
        .get_or_init(|| fc_match_primary_family("monospace:lang=en", "monospace"))
        .as_deref()
}

fn detect_cjk_sans_family() -> Option<&'static str> {
    static CACHE: OnceLock<Option<String>> = OnceLock::new();
    CACHE
        .get_or_init(|| fc_match_primary_family("sans:lang=zh-cn", "sans"))
        .as_deref()
}

fn fc_match_primary_family(pattern: &str, generic_name: &str) -> Option<String> {
    let output = Command::new("fc-match")
        .args(["--format=%{family[0]}\n", pattern])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let family = String::from_utf8_lossy(&output.stdout)
        .lines()
        .next()?
        .split(',')
        .next()?
        .trim()
        .to_string();
    if family.is_empty() || family.eq_ignore_ascii_case(generic_name) {
        None
    } else {
        Some(family)
    }
}

/// 基于缩进比例、代码关键字与符号密度自动判断文本是否为代码
fn detect_code(text: &str) -> bool {
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
        "public ",
        "private ",
        "protected ",
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
        if CODE_PREFIXES
            .iter()
            .any(|prefix| trimmed.starts_with(prefix))
        {
            keyword_hits += 1;
        }
        if trimmed.ends_with(';') || trimmed.ends_with('{') || trimmed.ends_with('}') {
            semicolon_or_brace_ends += 1;
        }
    }

    let token_hits: usize = CODE_TOKENS
        .iter()
        .map(|tok| text.matches(tok).count())
        .sum();
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
fn truncate_lines(text: &str, max_lines: usize) -> (String, Option<usize>) {
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() <= max_lines {
        return (text.to_string(), None);
    }

    let kept = lines[..max_lines].join("\n");
    (kept, Some(lines.len()))
}

/// 转义 Pango XML 特殊字符并剔除非换行控制字符，防止 `pango-view --markup` 报错
fn escape_pango_markup(text: &str) -> String {
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
fn build_title_preview(text: &str) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

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
        let long =
            "这是一段非常非常长的中文标题摘要内容用于验证按字符截断而不是按字节截断的安全性检查";
        let preview = build_title_preview(long);
        assert!(preview.ends_with('…'));
        assert_eq!(preview.chars().count(), 41);
    }

    #[test]
    fn pango_view_renders_valid_png_for_code_and_chinese() {
        let cfg = TextCardConfig::default();
        let text = "fn greet() {\n    println!(\"你好 Pinshot <Wayland> & Rust\");\n}";
        let rendered = render_text_card_with_env(text, None, &cfg, ColorScheme::Dark, 1.5)
            .expect("调用 pango-view 渲染文字卡片失败");

        assert!(rendered.is_code);
        assert_eq!(rendered.source_scale, 1.5);
        assert_eq!(rendered.title_preview, "fn greet() {");

        let img = image::load_from_memory(&rendered.png_bytes).expect("渲染结果应为有效 PNG");
        let (w, h) = img.dimensions();
        assert!(w > 100 && h > 30, "渲染尺寸异常: {w}x{h}");
    }

    #[test]
    fn short_text_renders_snug_width_and_rich_html_preserves_colors() {
        let cfg = TextCardConfig::default();
        let short = render_text_card_with_env("Hello", None, &cfg, ColorScheme::Dark, 1.0)
            .expect("短文本渲染失败");
        let (short_w, _) = image::load_from_memory(&short.png_bytes)
            .unwrap()
            .dimensions();
        assert!(
            short_w < 200,
            "短文本应自适应紧凑宽度，而不是固定 680px，实际: {short_w}"
        );

        let idea_html = r#"<pre style="background-color:#2b2b2b;color:#a9b7c6;font-family:'JetBrains Mono',monospace;"><span style="color:#cc7832;">fn</span> <span style="color:#ffc66d;">main</span>() {}</pre>"#;
        let rich = render_text_card_with_env(
            "fn main() {}",
            Some(idea_html),
            &cfg,
            ColorScheme::Light,
            1.0,
        )
        .expect("富文本代码卡片渲染失败");
        assert!(rich.is_code);
        let rich_img = image::load_from_memory(&rich.png_bytes).unwrap().to_rgb8();
        // 验证左上角背景像素保留了 IDEA 的 #2b2b2b 暗色背景，而非 Light 主题白底
        let corner = rich_img.get_pixel(2, 2);
        assert_eq!(corner.0, [0x2b, 0x2b, 0x2b]);
    }

    #[test]
    fn code_card_keeps_brace_and_cjk_comment_indentation_aligned() {
        let cfg = TextCardConfig::default();
        let java = "    public static void main(String[] args) {\n        // 中文注释测试\n        System.out.println(\"ok\");\n    }";
        let rendered = render_text_card_with_env(java, None, &cfg, ColorScheme::Dark, 1.0)
            .expect("Java 代码卡片渲染失败");
        assert!(rendered.is_code);

        let img = image::load_from_memory(&rendered.png_bytes)
            .unwrap()
            .to_rgb8();
        let (w, h) = img.dimensions();
        let bg = [0x1e_u8, 0x1f, 0x29];
        let band_h = (h as f64 - 32.0) / 4.0;
        let leftmost_x = |row: usize| -> u32 {
            let y0 = (16.0 + row as f64 * band_h + 2.0) as u32;
            let y1 = ((16.0 + (row + 1) as f64 * band_h - 2.0) as u32).min(h);
            let mut min_x = w;
            for y in y0..y1 {
                for x in 0..w {
                    let p = img.get_pixel(x, y).0;
                    let diff = (i32::from(p[0]) - i32::from(bg[0])).abs()
                        + (i32::from(p[1]) - i32::from(bg[1])).abs()
                        + (i32::from(p[2]) - i32::from(bg[2])).abs();
                    if diff > 30 {
                        min_x = min_x.min(x);
                        break;
                    }
                }
            }
            min_x
        };

        let x_public = leftmost_x(0);
        let x_comment = leftmost_x(1);
        let x_system = leftmost_x(2);
        let x_brace = leftmost_x(3);
        assert!(
            x_public.abs_diff(x_brace) <= 2,
            "`public` ({x_public}) 与 `}}` ({x_brace}) 应左对齐"
        );
        assert!(
            x_comment.abs_diff(x_system) <= 2,
            "中文注释 ({x_comment}) 与 `System` ({x_system}) 应缩进对齐"
        );
    }

    #[test]
    fn long_single_line_falls_back_to_wrapped_valid_png() {
        let cfg = TextCardConfig::default();
        let long_line = "a".repeat(6_000);
        let rendered = render_text_card_with_env(&long_line, None, &cfg, ColorScheme::Dark, 1.0)
            .expect("超长单行文本应通过限宽折行渲染出有效 PNG");
        let img = image::load_from_memory(&rendered.png_bytes).expect("结果应为有效 PNG");
        let (w, h) = img.dimensions();
        assert!(w > 100 && w <= cfg.max_width_px + 128, "折行宽度异常: {w}");
        assert!(h > 200, "折行后应产生多行高度，实际: {h}");
    }

    #[test]
    fn wrapped_word_break_trims_excess_right_whitespace() {
        let cfg = TextCardConfig::default();
        // 第一词组 + 空格 + 超长连续数字：总长超 960px 触发按词折行，但折行后两行实际宽度都远小于 960px
        let ones = "1".repeat(55);
        let text = format!("SpringApplication.run(KeepAccountV2Application.class, args); {ones}");
        let html = format!(
            r#"<pre style="background-color:#282a36;color:#f8f8f2;">SpringApplication.<span style="color:#50fa7b;">run</span>(KeepAccountV2Application.class, args); {ones}</pre>"#
        );
        let rendered = render_text_card_with_env(&text, Some(&html), &cfg, ColorScheme::Dark, 1.0)
            .expect("折行代码渲染失败");
        assert!(rendered.is_code);
        let img = image::load_from_memory(&rendered.png_bytes).unwrap();
        let (w, h) = img.dimensions();
        assert!(h > 55, "应已折成两行，实际高度: {h}");
        assert!(
            w < 700,
            "按词折行后应裁掉右侧多余空白而不是固定撑满 960px，实际宽度: {w}"
        );
    }
}
