//! 轻量级剪贴板 `text/html` → Pango Markup 转换器（零外部依赖）
//!
//! 面向 IntelliJ IDEA、VS Code、浏览器等应用复制出的富文本与代码片段，
//! 保留前景色、背景色、粗体、斜体、下划线、删除线与等宽格式；
//! 忽略 `<script>`/`<style>`/图片等无关元素，解析异常时自动回退纯文本。

use std::collections::HashMap;
use std::fmt::Write as _;

/// 单个带样式文本片段
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StyledSpan {
    pub text: String,
    pub style: SpanStyle,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SpanStyle {
    pub fg: Option<String>,
    pub bg: Option<String>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    pub monospace: bool,
}

/// 解析后的富文本文档
#[derive(Debug, Clone)]
pub struct RichDocument {
    pub lines: Vec<Vec<StyledSpan>>,
    /// 若源 HTML（如 IDEA / VS Code `<pre>`/`<div>`）自带明确背景色与前景色，则用于卡片底色
    pub root_bg: Option<String>,
    pub root_fg: Option<String>,
    pub is_code: bool,
}

#[derive(Debug, Clone)]
struct StyleFrame {
    tag: String,
    style: SpanStyle,
    preserve_ws: bool,
    is_block: bool,
    skip_content: bool,
}

/// 将剪贴板 `text/html` 解析为可渲染的富文本行；若为空或与 `plain` 偏差过大返回 `None` 触发纯文本回退
pub fn parse_clipboard_html(html: &str, plain: &str) -> Option<RichDocument> {
    let html = html.trim_start_matches('\u{FEFF}');
    if html.trim().is_empty() {
        return None;
    }

    let mut stack: Vec<StyleFrame> = Vec::new();
    let mut lines: Vec<Vec<StyledSpan>> = vec![Vec::new()];
    let mut class_styles: HashMap<String, String> = HashMap::new();
    let mut style_buf = String::new();
    let mut root_bg: Option<String> = None;
    let mut root_fg: Option<String> = None;
    let mut saw_styled_span = false;
    let mut saw_pre_or_mono = false;
    let mut total_visible_chars = 0usize;
    let mut last_was_space = false;

    let bytes = html.as_bytes();
    let len = bytes.len();
    let mut i = 0usize;

    while i < len {
        if bytes[i] == b'<' {
            // 注释 <!-- ... -->
            if html[i..].starts_with("<!--") {
                if let Some(end) = html[i + 4..].find("-->") {
                    i = i + 4 + end + 3;
                } else {
                    break;
                }
                continue;
            }
            // DOCTYPE / XML 处理指令
            if html[i..].starts_with("<!") || html[i..].starts_with("<?") {
                if let Some(end) = html[i + 2..].find('>') {
                    i = i + 2 + end + 1;
                } else {
                    break;
                }
                continue;
            }

            let Some(close_rel) = find_tag_end(&html[i + 1..]) else {
                // 未闭合的 '<'，当作普通字符处理
                push_text(
                    "<",
                    &stack,
                    &mut lines,
                    &mut total_visible_chars,
                    &mut last_was_space,
                );
                i += 1;
                continue;
            };
            let tag_inner = &html[i + 1..i + 1 + close_rel];
            i = i + 1 + close_rel + 1;

            let trimmed = tag_inner.trim();
            if trimmed.is_empty() {
                continue;
            }

            if let Some(rest) = trimmed.strip_prefix('/') {
                let close_name = rest
                    .split(|c: char| c.is_ascii_whitespace() || c == '/')
                    .next()
                    .unwrap_or("")
                    .to_ascii_lowercase();
                if let Some(pos) = stack.iter().rposition(|f| f.tag == close_name) {
                    let popped = stack.remove(pos);
                    if popped.tag == "style" && !style_buf.is_empty() {
                        parse_css_class_rules(&style_buf, &mut class_styles);
                        style_buf.clear();
                    }
                    if popped.is_block && !popped.skip_content && !current_line_empty(&lines) {
                        if !popped.preserve_ws {
                            trim_trailing_line_spaces(lines.last_mut().unwrap());
                        }
                        lines.push(Vec::new());
                        last_was_space = false;
                    }
                }
                continue;
            }

            let self_closing = trimmed.ends_with('/');
            let body = trimmed.trim_end_matches('/').trim();
            let (tag_name, attrs) = split_tag_and_attrs(body);
            let tag = tag_name.to_ascii_lowercase();

            if tag == "br" {
                if !is_skipping(&stack) {
                    if !stack.last().is_some_and(|f| f.preserve_ws)
                        && let Some(last_line) = lines.last_mut()
                    {
                        trim_trailing_line_spaces(last_line);
                    }
                    lines.push(Vec::new());
                    last_was_space = false;
                }
                continue;
            }

            let is_void = self_closing
                || matches!(
                    tag.as_str(),
                    "meta" | "link" | "img" | "hr" | "input" | "col" | "area" | "base"
                );

            let parent_style = stack.last().map(|f| f.style.clone()).unwrap_or_default();
            let parent_pre = stack.last().is_some_and(|f| f.preserve_ws);
            let parent_skip = is_skipping(&stack);

            let skip_content = parent_skip
                || matches!(
                    tag.as_str(),
                    "head" | "style" | "script" | "title" | "noscript" | "svg"
                );
            let is_block = matches!(
                tag.as_str(),
                "div"
                    | "p"
                    | "pre"
                    | "li"
                    | "tr"
                    | "blockquote"
                    | "h1"
                    | "h2"
                    | "h3"
                    | "h4"
                    | "h5"
                    | "h6"
            );

            let mut style = parent_style;
            let mut preserve_ws = parent_pre;
            let mut tag_bg: Option<String> = None;
            let mut tag_fg: Option<String> = None;

            match tag.as_str() {
                "b" | "strong" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                    style.bold = true;
                    saw_styled_span = true;
                }
                "i" | "em" => {
                    style.italic = true;
                    saw_styled_span = true;
                }
                "u" => {
                    style.underline = true;
                    saw_styled_span = true;
                }
                "s" | "del" | "strike" => {
                    style.strike = true;
                    saw_styled_span = true;
                }
                "pre" => {
                    style.monospace = true;
                    preserve_ws = true;
                    saw_pre_or_mono = true;
                }
                "code" | "kbd" | "tt" => {
                    style.monospace = true;
                    saw_pre_or_mono = true;
                }
                _ => {}
            }

            if !skip_content {
                apply_html_attrs(
                    attrs,
                    &class_styles,
                    &mut style,
                    &mut preserve_ws,
                    &mut tag_bg,
                    &mut tag_fg,
                    &mut saw_styled_span,
                    &mut saw_pre_or_mono,
                );
                // 首个正文出现前的块级容器（body / pre / div）的 background-color / color 作为卡片整体主题
                // 内层容器（如 <pre>）优先覆盖外层（如 <body>）的默认背景
                if total_visible_chars == 0 && (is_block || tag == "body") {
                    if let Some(bg) = tag_bg {
                        root_bg = Some(bg);
                        // 根背景已用作整卡底色，避免每个 span 重复画背景
                        style.bg = None;
                    }
                    if let Some(fg) = tag_fg {
                        root_fg = Some(fg);
                    }
                }
            }

            if is_block && !skip_content && !current_line_empty(&lines) {
                lines.push(Vec::new());
                last_was_space = false;
            }
            if tag == "li" && !skip_content {
                push_text(
                    "• ",
                    &stack,
                    &mut lines,
                    &mut total_visible_chars,
                    &mut last_was_space,
                );
            }

            if !is_void {
                stack.push(StyleFrame {
                    tag,
                    style,
                    preserve_ws,
                    is_block,
                    skip_content,
                });
            }
        } else {
            let next_lt = html[i..].find('<').map_or(len, |rel| i + rel);
            let raw_segment = &html[i..next_lt];
            i = next_lt;
            if stack.last().is_some_and(|f| f.tag == "style") {
                style_buf.push_str(raw_segment);
                continue;
            }
            if is_skipping(&stack) {
                continue;
            }
            let decoded = decode_html_entities(raw_segment);
            push_text(
                &decoded,
                &stack,
                &mut lines,
                &mut total_visible_chars,
                &mut last_was_space,
            );
        }
    }

    // 清理首尾空行
    while lines.first().is_some_and(|l| line_is_blank(l)) {
        lines.remove(0);
    }
    while lines.last().is_some_and(|l| line_is_blank(l)) {
        lines.pop();
    }

    if lines.is_empty() || total_visible_chars == 0 {
        return None;
    }

    // 校验提取出的可见字符量与纯文本一致度，防异常 HTML 丢正文
    let plain_non_ws = plain.chars().filter(|c| !c.is_whitespace()).count();
    if plain_non_ws > 0 && total_visible_chars * 100 < plain_non_ws * 40 {
        return None;
    }

    // 若 HTML 里完全没有任何颜色、背景、粗斜体等样式信息，直接交回纯文本渲染更简洁
    let has_any_style = saw_styled_span || root_bg.is_some() || root_fg.is_some();
    if !has_any_style {
        return None;
    }

    if saw_pre_or_mono {
        dedent_rich_lines(&mut lines);
    }

    Some(RichDocument {
        lines,
        root_bg,
        root_fg,
        is_code: saw_pre_or_mono,
    })
}

/// 将解析出的 `RichDocument` 渲染为转义安全的 Pango Markup
pub fn render_rich_to_pango_markup(
    doc: &RichDocument,
    max_lines: usize,
    card_bg: &str,
    default_fg: &str,
    muted_color: &str,
) -> String {
    let total_lines = doc.lines.len();
    let keep = total_lines.min(max_lines);
    let mut out = String::new();

    for (line_idx, spans) in doc.lines.iter().take(keep).enumerate() {
        if line_idx > 0 {
            out.push('\n');
        }
        for span in spans {
            if span.text.is_empty() {
                continue;
            }
            let escaped = escape_pango(&span.text);
            let fg = span.style.fg.as_deref().filter(|c| {
                !c.eq_ignore_ascii_case(default_fg) && colors_have_contrast(c, card_bg)
            });
            let bg = span
                .style
                .bg
                .as_deref()
                .filter(|c| !c.eq_ignore_ascii_case(card_bg));

            let has_attrs = fg.is_some()
                || bg.is_some()
                || span.style.bold
                || span.style.italic
                || span.style.underline
                || span.style.strike
                || (span.style.monospace && !doc.is_code);

            if !has_attrs {
                out.push_str(&escaped);
                continue;
            }

            out.push_str("<span");
            if let Some(color) = fg {
                let _ = write!(&mut out, " foreground=\"{color}\"");
            }
            if let Some(color) = bg {
                let _ = write!(&mut out, " background=\"{color}\"");
            }
            if span.style.bold {
                out.push_str(" weight=\"bold\"");
            }
            if span.style.italic {
                out.push_str(" style=\"italic\"");
            }
            if span.style.underline {
                out.push_str(" underline=\"single\"");
            }
            if span.style.strike {
                out.push_str(" strikethrough=\"true\"");
            }
            if span.style.monospace && !doc.is_code {
                out.push_str(" font_family=\"monospace\"");
            }
            out.push('>');
            out.push_str(&escaped);
            out.push_str("</span>");
        }
    }

    if total_lines > max_lines {
        let _ = write!(
            &mut out,
            "\n\n<span foreground=\"{muted_color}\" size=\"smaller\">… 已截断，共 {total_lines} 行（右键可复制完整原文）</span>"
        );
    }

    out
}

fn is_skipping(stack: &[StyleFrame]) -> bool {
    stack.iter().any(|f| f.skip_content)
}

fn current_line_empty(lines: &[Vec<StyledSpan>]) -> bool {
    lines.last().is_none_or(|l| l.is_empty())
}

fn line_is_blank(line: &[StyledSpan]) -> bool {
    line.iter()
        .all(|s| s.text.chars().all(|c| c.is_whitespace()))
}

fn push_text(
    text: &str,
    stack: &[StyleFrame],
    lines: &mut Vec<Vec<StyledSpan>>,
    total_visible: &mut usize,
    last_was_space: &mut bool,
) {
    let preserve_ws = stack.last().is_some_and(|f| f.preserve_ws);
    let style = stack.last().map(|f| f.style.clone()).unwrap_or_default();

    if preserve_ws {
        let unified = text
            .replace("\r\n", "\n")
            .replace('\r', "\n")
            .replace('\t', "    ")
            .replace('\u{00A0}', " ");
        for (idx, segment) in unified.split('\n').enumerate() {
            if idx > 0 {
                lines.push(Vec::new());
            }
            if !segment.is_empty() {
                *total_visible += segment.chars().filter(|c| !c.is_whitespace()).count();
                append_span(lines.last_mut().unwrap(), segment, &style);
            }
        }
        *last_was_space = false;
        return;
    }

    let mut collapsed = String::with_capacity(text.len());
    for ch in text.chars() {
        if ch == '\u{00A0}' {
            // &nbsp; 保留为普通空格且不参与折叠
            collapsed.push(' ');
            *last_was_space = false;
        } else if ch.is_whitespace() {
            if !*last_was_space && (!current_line_empty(lines) || !collapsed.is_empty()) {
                collapsed.push(' ');
                *last_was_space = true;
            }
        } else {
            collapsed.push(ch);
            *last_was_space = false;
            *total_visible += 1;
        }
    }

    if !collapsed.is_empty() {
        append_span(lines.last_mut().unwrap(), &collapsed, &style);
    }
}

fn trim_trailing_line_spaces(line: &mut Vec<StyledSpan>) {
    while let Some(last) = line.last_mut() {
        let trimmed_len = last.text.trim_end_matches(' ').len();
        if trimmed_len == 0 {
            line.pop();
        } else {
            last.text.truncate(trimmed_len);
            break;
        }
    }
}

fn append_span(line: &mut Vec<StyledSpan>, text: &str, style: &SpanStyle) {
    if let Some(last) = line.last_mut()
        && &last.style == style
    {
        last.text.push_str(text);
        return;
    }
    line.push(StyledSpan {
        text: text.to_string(),
        style: style.clone(),
    });
}

fn find_tag_end(s: &str) -> Option<usize> {
    let mut quote: Option<u8> = None;
    for (idx, b) in s.bytes().enumerate() {
        match quote {
            Some(q) if b == q => quote = None,
            Some(_) => {}
            None if b == b'"' || b == b'\'' => quote = Some(b),
            None if b == b'>' => return Some(idx),
            None => {}
        }
    }
    None
}

fn split_tag_and_attrs(body: &str) -> (&str, &str) {
    match body.find(|c: char| c.is_ascii_whitespace()) {
        Some(pos) => (&body[..pos], body[pos..].trim()),
        None => (body, ""),
    }
}

#[allow(clippy::too_many_arguments)]
fn apply_html_attrs(
    attrs: &str,
    class_styles: &HashMap<String, String>,
    style: &mut SpanStyle,
    preserve_ws: &mut bool,
    tag_bg: &mut Option<String>,
    tag_fg: &mut Option<String>,
    saw_styled: &mut bool,
    saw_mono: &mut bool,
) {
    if attrs.is_empty() {
        return;
    }
    let parsed = parse_attrs(attrs);
    // 先应用 class 规则，再应用内联 style，确保内联样式优先级更高
    if !class_styles.is_empty() {
        for (name, value) in &parsed {
            if name.eq_ignore_ascii_case("class") {
                for cls in value.split_whitespace() {
                    if let Some(decls) = class_styles.get(cls) {
                        apply_css_declarations(
                            decls,
                            style,
                            preserve_ws,
                            tag_bg,
                            tag_fg,
                            saw_styled,
                            saw_mono,
                        );
                    }
                }
            }
        }
    }
    for (name, value) in parsed {
        if name.eq_ignore_ascii_case("color") {
            if let Some(hex) = parse_css_color(&value) {
                style.fg = Some(hex.clone());
                *tag_fg = Some(hex);
                *saw_styled = true;
            }
        } else if name.eq_ignore_ascii_case("style") {
            apply_css_declarations(
                &value,
                style,
                preserve_ws,
                tag_bg,
                tag_fg,
                saw_styled,
                saw_mono,
            );
        }
    }
}

fn apply_css_declarations(
    decls: &str,
    style: &mut SpanStyle,
    preserve_ws: &mut bool,
    tag_bg: &mut Option<String>,
    tag_fg: &mut Option<String>,
    saw_styled: &mut bool,
    saw_mono: &mut bool,
) {
    for decl in decls.split(';') {
        let Some((k, v)) = decl.split_once(':') else {
            continue;
        };
        let key = k.trim().to_ascii_lowercase();
        let val = v.trim();
        match key.as_str() {
            "color" => {
                if let Some(hex) = parse_css_color(val) {
                    style.fg = Some(hex.clone());
                    *tag_fg = Some(hex);
                    *saw_styled = true;
                }
            }
            "background-color" | "background" => {
                if let Some(hex) = parse_css_color(val) {
                    style.bg = Some(hex.clone());
                    *tag_bg = Some(hex);
                    *saw_styled = true;
                }
            }
            "font-weight" => {
                let lower = val.to_ascii_lowercase();
                if lower == "bold" || lower == "bolder" {
                    style.bold = true;
                    *saw_styled = true;
                } else if lower == "normal" {
                    style.bold = false;
                } else if let Ok(num) = lower.parse::<u32>() {
                    style.bold = num >= 600;
                    if style.bold {
                        *saw_styled = true;
                    }
                }
            }
            "font-style" => {
                let lower = val.to_ascii_lowercase();
                if lower == "italic" || lower == "oblique" {
                    style.italic = true;
                    *saw_styled = true;
                } else if lower == "normal" {
                    style.italic = false;
                }
            }
            "text-decoration" | "text-decoration-line" => {
                let lower = val.to_ascii_lowercase();
                if lower.contains("underline") {
                    style.underline = true;
                    *saw_styled = true;
                }
                if lower.contains("line-through") {
                    style.strike = true;
                    *saw_styled = true;
                }
                if lower == "none" {
                    style.underline = false;
                    style.strike = false;
                }
            }
            "white-space" => {
                let lower = val.to_ascii_lowercase();
                if lower == "pre" || lower == "pre-wrap" || lower == "break-spaces" {
                    *preserve_ws = true;
                }
            }
            "font-family" => {
                let lower = val.to_ascii_lowercase();
                if [
                    "mono",
                    "consolas",
                    "courier",
                    "menlo",
                    "fira",
                    "jetbrains",
                    "hack",
                    "cascadia",
                    "source code",
                    "iosevka",
                    "meslo",
                ]
                .iter()
                .any(|kw| lower.contains(kw))
                {
                    style.monospace = true;
                    *saw_mono = true;
                }
            }
            _ => {}
        }
    }
}

/// 从 `<style>` 块提取简单的 `.classname { ... }` 样式映射，支持基于 class 的语法高亮
fn parse_css_class_rules(css: &str, out: &mut HashMap<String, String>) {
    // 剔除 /* ... */ 注释
    let mut clean = String::with_capacity(css.len());
    let mut rest = css;
    while let Some(start) = rest.find("/*") {
        clean.push_str(&rest[..start]);
        if let Some(end) = rest[start + 2..].find("*/") {
            rest = &rest[start + 2 + end + 2..];
        } else {
            rest = "";
            break;
        }
    }
    clean.push_str(rest);

    let mut pos = 0usize;
    let bytes = clean.as_bytes();
    while pos < bytes.len() {
        let Some(open_rel) = clean[pos..].find('{') else {
            break;
        };
        let open = pos + open_rel;
        let selector_list = clean[pos..open].trim();
        let Some(close_rel) = clean[open + 1..].find('}') else {
            break;
        };
        let close = open + 1 + close_rel;
        let body = clean[open + 1..close].trim();
        pos = close + 1;

        if body.is_empty() {
            continue;
        }
        for sel in selector_list.split(',') {
            let trimmed = sel.trim();
            if let Some(cls) = trimmed.strip_prefix('.')
                && !cls.is_empty()
                && cls
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
            {
                let entry = out.entry(cls.to_string()).or_default();
                if !entry.is_empty() && !entry.ends_with(';') {
                    entry.push(';');
                }
                entry.push_str(body);
            }
        }
    }
}

/// 去除多行代码块的公共左侧冗余缩进（保留所有相对缩进）
fn dedent_rich_lines(lines: &mut [Vec<StyledSpan>]) {
    let non_blank_count = lines.iter().filter(|l| !line_is_blank(l)).count();
    if non_blank_count < 2 {
        return;
    }
    let min_indent = lines
        .iter()
        .filter(|l| !line_is_blank(l))
        .map(|l| leading_spaces_in_line(l))
        .min()
        .unwrap_or(0);
    if min_indent == 0 {
        return;
    }
    for line in lines.iter_mut() {
        strip_leading_spaces_from_line(line, min_indent);
    }
}

fn leading_spaces_in_line(line: &[StyledSpan]) -> usize {
    let mut count = 0usize;
    for span in line {
        for ch in span.text.chars() {
            if ch == ' ' {
                count += 1;
            } else {
                return count;
            }
        }
    }
    count
}

fn strip_leading_spaces_from_line(line: &mut Vec<StyledSpan>, mut to_strip: usize) {
    while !line.is_empty() && to_strip > 0 {
        let span = &mut line[0];
        let mut cut = 0usize;
        for ch in span.text.chars() {
            if ch == ' ' && to_strip > 0 {
                cut += 1;
                to_strip -= 1;
            } else {
                break;
            }
        }
        if cut >= span.text.len() {
            line.remove(0);
        } else {
            if cut > 0 {
                span.text.drain(..cut);
            }
            break;
        }
    }
}

fn parse_attrs(attrs: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let bytes = attrs.as_bytes();
    let len = bytes.len();
    let mut i = 0usize;

    while i < len {
        while i < len && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= len {
            break;
        }
        let name_start = i;
        while i < len && !bytes[i].is_ascii_whitespace() && bytes[i] != b'=' {
            i += 1;
        }
        let name = attrs[name_start..i].to_string();
        while i < len && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= len || bytes[i] != b'=' {
            continue;
        }
        i += 1;
        while i < len && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= len {
            break;
        }
        let value = if bytes[i] == b'"' || bytes[i] == b'\'' {
            let q = bytes[i];
            i += 1;
            let start = i;
            while i < len && bytes[i] != q {
                i += 1;
            }
            let val = &attrs[start..i];
            if i < len {
                i += 1;
            }
            decode_html_entities(val)
        } else {
            let start = i;
            while i < len && !bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            decode_html_entities(&attrs[start..i])
        };
        out.push((name, value));
    }
    out
}

/// 解析常见 CSS 颜色值（`#rgb`、`#rrggbb`、`rgb(...)`、`rgba(...)` 及常用命名色）
pub fn parse_css_color(raw: &str) -> Option<String> {
    let token = raw
        .split_whitespace()
        .find(|part| !part.eq_ignore_ascii_case("!important"))?
        .trim();
    let lower = token.to_ascii_lowercase();
    if lower == "transparent" || lower == "inherit" || lower == "initial" || lower == "none" {
        return None;
    }

    if let Some(hex) = lower.strip_prefix('#') {
        return match hex.len() {
            3 if hex.bytes().all(|b| b.is_ascii_hexdigit()) => {
                let b = hex.as_bytes();
                Some(format!(
                    "#{0}{0}{1}{1}{2}{2}",
                    b[0] as char, b[1] as char, b[2] as char
                ))
            }
            6 if hex.bytes().all(|b| b.is_ascii_hexdigit()) => Some(format!("#{hex}")),
            8 if hex.bytes().all(|b| b.is_ascii_hexdigit()) => {
                let alpha = u8::from_str_radix(&hex[6..8], 16).ok()?;
                (alpha >= 20).then(|| format!("#{}", &hex[..6]))
            }
            _ => None,
        };
    }

    let trimmed_raw = raw.trim().to_ascii_lowercase();
    let trimmed = trimmed_raw
        .strip_suffix("!important")
        .unwrap_or(&trimmed_raw)
        .trim();
    if let Some(inner) = trimmed
        .strip_prefix("rgb(")
        .or_else(|| trimmed.strip_prefix("rgba("))
        .and_then(|s| s.strip_suffix(')'))
    {
        let parts: Vec<&str> = inner
            .split(|c: char| c == ',' || c == '/' || c.is_ascii_whitespace())
            .filter(|p| !p.is_empty())
            .collect();
        if parts.len() >= 3 {
            let r = parts[0].parse::<f64>().ok()?.round().clamp(0.0, 255.0) as u8;
            let g = parts[1].parse::<f64>().ok()?.round().clamp(0.0, 255.0) as u8;
            let b = parts[2].parse::<f64>().ok()?.round().clamp(0.0, 255.0) as u8;
            if parts.len() >= 4 {
                let a = parts[3].parse::<f64>().ok().unwrap_or(1.0);
                if a < 0.08 {
                    return None;
                }
            }
            return Some(format!("#{r:02x}{g:02x}{b:02x}"));
        }
    }

    let named = match lower.as_str() {
        "black" => "#000000",
        "white" => "#ffffff",
        "red" => "#d93025",
        "green" => "#188038",
        "blue" => "#1a73e8",
        "yellow" => "#f9ab00",
        "orange" => "#e37400",
        "purple" => "#9334e6",
        "gray" | "grey" => "#808080",
        "cyan" | "aqua" => "#00acc1",
        "magenta" => "#c2185b",
        _ => return None,
    };
    Some(named.to_string())
}

fn parse_hex_rgb(hex: &str) -> Option<(u8, u8, u8)> {
    let h = hex.strip_prefix('#')?;
    if h.len() != 6 {
        return None;
    }
    Some((
        u8::from_str_radix(&h[0..2], 16).ok()?,
        u8::from_str_radix(&h[2..4], 16).ok()?,
        u8::from_str_radix(&h[4..6], 16).ok()?,
    ))
}

fn colors_have_contrast(a: &str, b: &str) -> bool {
    let (Some((r1, g1, b1)), Some((r2, g2, b2))) = (parse_hex_rgb(a), parse_hex_rgb(b)) else {
        return true;
    };
    let l1 = 0.299 * f64::from(r1) + 0.587 * f64::from(g1) + 0.114 * f64::from(b1);
    let l2 = 0.299 * f64::from(r2) + 0.587 * f64::from(g2) + 0.114 * f64::from(b2);
    let diff = (l1 - l2).abs();
    let chroma = (i32::from(r1) - i32::from(r2)).abs()
        + (i32::from(g1) - i32::from(g2)).abs()
        + (i32::from(b1) - i32::from(b2)).abs();
    diff >= 22.0 || chroma >= 60
}

fn decode_html_entities(input: &str) -> String {
    if !input.contains('&') {
        return input.to_string();
    }
    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let len = bytes.len();
    let mut i = 0usize;
    while i < len {
        if bytes[i] == b'&'
            && let Some(semi) = input[i + 1..].find(';')
            && semi <= 10
        {
            let entity = &input[i + 1..i + 1 + semi];
            if let Some(ch) = decode_single_entity(entity) {
                out.push(ch);
                i = i + 1 + semi + 1;
                continue;
            }
        }
        let ch = input[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn decode_single_entity(entity: &str) -> Option<char> {
    match entity {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        "nbsp" => Some('\u{00A0}'),
        _ if entity.starts_with("#x") || entity.starts_with("#X") => {
            u32::from_str_radix(&entity[2..], 16)
                .ok()
                .and_then(char::from_u32)
        }
        _ if entity.starts_with('#') => entity[1..].parse::<u32>().ok().and_then(char::from_u32),
        _ => None,
    }
}

fn escape_pango(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 16);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_intellij_idea_code_html_with_theme_and_colors() {
        let html = r#"<html><head><meta charset="UTF-8"></head><body>
<pre style="background-color:#2b2b2b;color:#a9b7c6;font-family:'JetBrains Mono',monospace;">
<span style="color:#cc7832;font-weight:bold;">public class</span> Main {
    <span style="color:#6a8759;">&quot;hello&quot;</span>;
}</pre></body></html>"#;
        let plain = "public class Main {\n    \"hello\";\n}";
        let doc = parse_clipboard_html(html, plain).expect("应成功解析 IDEA HTML");

        assert_eq!(doc.root_bg.as_deref(), Some("#2b2b2b"));
        assert_eq!(doc.root_fg.as_deref(), Some("#a9b7c6"));
        assert!(doc.is_code);
        assert_eq!(doc.lines.len(), 3);
        assert_eq!(doc.lines[0][0].text, "public class");
        assert_eq!(doc.lines[0][0].style.fg.as_deref(), Some("#cc7832"));
        assert!(doc.lines[0][0].style.bold);
        assert_eq!(doc.lines[1][1].text, "\"hello\"");
        assert_eq!(doc.lines[1][1].style.fg.as_deref(), Some("#6a8759"));

        let markup = render_rich_to_pango_markup(&doc, 80, "#2b2b2b", "#a9b7c6", "#8b94a8");
        assert!(markup.contains("foreground=\"#cc7832\""));
        assert!(markup.contains("weight=\"bold\""));
        assert!(markup.contains("&quot;hello&quot;"));
    }

    #[test]
    fn parses_vscode_multiline_div_html() {
        let html = r#"<meta charset='utf-8'><div style="color: rgb(212, 212, 212); background-color: #1e1e1e; font-family: 'Fira Code', monospace; white-space: pre;"><div><span style="color: #569cd6;">fn</span> <span>main()</span></div><div><br></div><div><span style="color: #ce9178;">"ok"</span></div></div>"#;
        let plain = "fn main()\n\n\"ok\"";
        let doc = parse_clipboard_html(html, plain).expect("应成功解析 VS Code HTML");

        assert_eq!(doc.root_bg.as_deref(), Some("#1e1e1e"));
        assert_eq!(doc.root_fg.as_deref(), Some("#d4d4d4"));
        assert!(doc.is_code);
        assert_eq!(doc.lines.len(), 3);
        assert!(doc.lines[1].is_empty());
        assert_eq!(doc.lines[2][0].text, "\"ok\"");
    }

    #[test]
    fn skips_script_style_and_falls_back_when_unstyled() {
        let unstyled = "<div><span>plain text only</span></div>";
        assert!(parse_clipboard_html(unstyled, "plain text only").is_none());

        let with_script =
            "<style>.a{color:red}</style><script>alert(1)</script><p><b>Hello</b> <i>World</i></p>";
        let doc = parse_clipboard_html(with_script, "Hello World").expect("应解析富文本");
        assert_eq!(doc.lines.len(), 1);
        assert_eq!(doc.lines[0][0].text, "Hello");
        assert!(doc.lines[0][0].style.bold);
        assert!(doc.lines[0][2].style.italic);
    }

    #[test]
    fn applies_style_class_rules_and_dedents_multiline_code() {
        let html = r#"<html><head><style>.kw { color: #cf8e6d; font-weight: bold; } .cm { color: #7a7e85; }</style></head><body style="background-color:#ffffff;"><pre style="background-color:#1e1f22;color:#bcbec4;">    <span class="kw">public</span> void run() {
        <span class="cm">// comment</span>
    }</pre></body></html>"#;
        let plain = "    public void run() {\n        // comment\n    }";
        let doc = parse_clipboard_html(html, plain).expect("应解析 class 高亮与内层 pre 背景");
        assert_eq!(doc.root_bg.as_deref(), Some("#1e1f22"));
        assert_eq!(doc.lines.len(), 3);
        assert_eq!(doc.lines[0][0].text, "public");
        assert_eq!(doc.lines[0][0].style.fg.as_deref(), Some("#cf8e6d"));
        assert!(doc.lines[0][0].style.bold);
        // 公共 4 空格缩进已去除，第二行保留 4 空格相对缩进，第三行 '}' 顶格对齐
        assert_eq!(doc.lines[1][0].text, "    ");
        assert_eq!(doc.lines[2][0].text, "}");
    }

    #[test]
    fn preserves_word_spaces_in_paragraphs_across_tags_and_collapses_runs() {
        let html = "<p>   Hello   world\n\tfrom <b>Pinshot</b>  <i>rich </i> <u> text</u>   </p>";
        let plain = "Hello world from Pinshot rich text";
        let doc = parse_clipboard_html(html, plain).expect("应解析带样式段落");
        assert_eq!(doc.lines.len(), 1);
        let joined: String = doc.lines[0].iter().map(|s| s.text.as_str()).collect();
        assert_eq!(joined, "Hello world from Pinshot rich text");
    }
}
