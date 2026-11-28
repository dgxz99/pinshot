/**
 * 贴图离线 OCR 划词选字纯几何与文本计算模块（零 GNOME Shell 依赖，可直接由 Node 单测）
 */

const clamp = (value, min, max) => Math.min(Math.max(value, min), max);

const ASCII_WORD_RE = /^[A-Za-z0-9_$-]$/;
const SPACE_RE = /^\s$/;
const CJK_RE = /^[\u2E80-\u9FFF\uAC00-\uD7AF\uF900-\uFAFF]$/;

function normalizeCharX(rawCharX, charCount) {
    if (Array.isArray(rawCharX) && rawCharX.length === charCount + 1) {
        const out = new Array(charCount + 1);
        let prev = 0;
        for (let i = 0; i <= charCount; i += 1) {
            const v = Number(rawCharX[i]);
            const next = Number.isFinite(v) ? clamp(v, prev, 1) : prev;
            out[i] = next;
            prev = next;
        }
        out[0] = 0;
        out[charCount] = 1;
        return out;
    }
    const out = new Array(charCount + 1);
    for (let i = 0; i <= charCount; i += 1) {
        out[i] = charCount > 0 ? i / charCount : 0;
    }
    return out;
}

/**
 * 规范化后端返回的 OCR 文档并预拆 Unicode 字符数组
 */
export function prepareOcrDocument(rawDoc) {
    if (!rawDoc || !Array.isArray(rawDoc.lines)) {
        return null;
    }
    const imageWidth = Math.max(1, Number(rawDoc.image_width) || 1);
    const imageHeight = Math.max(1, Number(rawDoc.image_height) || 1);
    const lines = [];

    for (const item of rawDoc.lines) {
        if (!item || typeof item.text !== 'string' || !Array.isArray(item.bbox) || item.bbox.length < 4) {
            continue;
        }
        const chars = Array.from(item.text);
        if (chars.length === 0) {
            continue;
        }
        const [x, y, w, h] = item.bbox.map(Number);
        if (![x, y, w, h].every(Number.isFinite) || w <= 0 || h <= 0) {
            continue;
        }
        lines.push({
            bbox: [x, y, w, h],
            text: item.text,
            chars,
            charX: normalizeCharX(item.char_x, chars.length),
            sameVisualLineNext: Boolean(item.same_visual_line_next),
        });
    }

    return {
        version: Number(rawDoc.version) || 1,
        modelId: String(rawDoc.model_id ?? ''),
        imageWidth,
        imageHeight,
        lines,
    };
}

/**
 * 判断局部坐标是否位于贴图四周拖拽热区（保证满屏文字贴图也能从边缘拖动）
 */
export function isInDragBorder(localX, localY, displayWidth, displayHeight, borderPx = 8) {
    const w = Math.max(1, displayWidth);
    const h = Math.max(1, displayHeight);
    const effective = Math.min(borderPx, Math.floor(Math.min(w, h) / 6));
    if (effective <= 0) {
        return false;
    }
    return localX < effective || localY < effective || localX > w - effective || localY > h - effective;
}

/**
 * 贴图局部显示坐标 -> 原图像素坐标
 */
export function toImageCoords(localX, localY, displayWidth, displayHeight, doc) {
    const w = Math.max(1, displayWidth);
    const h = Math.max(1, displayHeight);
    return {
        ix: (localX / w) * doc.imageWidth,
        iy: (localY / h) * doc.imageHeight,
    };
}

function caretIndexInLine(line, ix) {
    const [x, , w] = line.bbox;
    const rel = clamp((ix - x) / Math.max(1e-6, w), 0, 1);
    const charX = line.charX;
    let best = 0;
    let bestDist = Infinity;
    for (let i = 0; i < charX.length; i += 1) {
        const dist = Math.abs(rel - charX[i]);
        if (dist < bestDist) {
            bestDist = dist;
            best = i;
        }
    }
    return best;
}

/**
 * 命中测试：判断原图像素坐标是否落在任意 OCR 文本框内
 */
export function hitTestText(doc, ix, iy, padPx = 4) {
    if (!doc || doc.lines.length === 0) {
        return null;
    }
    let bestIndex = -1;
    let bestDist = Infinity;

    for (let i = 0; i < doc.lines.length; i += 1) {
        const [x, y, w, h] = doc.lines[i].bbox;
        if (ix >= x - padPx && ix <= x + w + padPx && iy >= y - padPx && iy <= y + h + padPx) {
            const cx = x + w / 2;
            const cy = y + h / 2;
            const d = Math.hypot(ix - cx, iy - cy);
            if (d < bestDist) {
                bestDist = d;
                bestIndex = i;
            }
        }
    }

    if (bestIndex < 0) {
        return null;
    }
    return {
        lineIndex: bestIndex,
        charIndex: caretIndexInLine(doc.lines[bestIndex], ix),
    };
}

/**
 * 划词拖拽过程中定位最近的文本光标（允许指针跨过行间隙或略微拖出文本行外）
 */
export function locateCaret(doc, ix, iy) {
    if (!doc || doc.lines.length === 0) {
        return null;
    }
    const direct = hitTestText(doc, ix, iy, 4);
    if (direct) {
        return direct;
    }

    let bestIndex = 0;
    let bestScore = Infinity;
    for (let i = 0; i < doc.lines.length; i += 1) {
        const [x, y, w, h] = doc.lines[i].bbox;
        const dy = iy < y ? (y - iy) : (iy > y + h ? (iy - (y + h)) : 0);
        const dx = ix < x ? (x - ix) : (ix > x + w ? (ix - (x + w)) : 0);
        // 垂直距离权重更高，确保跨行拖选稳定贴近当前视觉行
        const score = dy * 4 + dx;
        if (score < bestScore) {
            bestScore = score;
            bestIndex = i;
        }
    }

    return {
        lineIndex: bestIndex,
        charIndex: caretIndexInLine(doc.lines[bestIndex], ix),
    };
}

/**
 * 将起止锚点归一化为按阅读顺序排列的非空选区；若起止相同返回 null
 */
export function normalizeSelection(anchor, focus) {
    if (!anchor || !focus) {
        return null;
    }
    const forward = anchor.lineIndex < focus.lineIndex ||
        (anchor.lineIndex === focus.lineIndex && anchor.charIndex <= focus.charIndex);
    const start = forward ? anchor : focus;
    const end = forward ? focus : anchor;
    if (start.lineIndex === end.lineIndex && start.charIndex === end.charIndex) {
        return null;
    }
    return {
        startLine: start.lineIndex,
        startChar: start.charIndex,
        endLine: end.lineIndex,
        endChar: end.charIndex,
    };
}

function charCategory(ch) {
    if (SPACE_RE.test(ch)) {
        return 'space';
    }
    if (ASCII_WORD_RE.test(ch)) {
        return 'ascii';
    }
    if (CJK_RE.test(ch)) {
        return 'cjk';
    }
    return 'punct';
}

/**
 * 文字区双击：选中命中的连续词块（英文单词/标识符、中文短语或整行）
 */
export function selectWordAt(doc, lineIndex, charIndex) {
    if (!doc || lineIndex < 0 || lineIndex >= doc.lines.length) {
        return null;
    }
    const chars = doc.lines[lineIndex].chars;
    if (chars.length === 0) {
        return null;
    }
    const idx = clamp(charIndex >= chars.length ? chars.length - 1 : charIndex, 0, chars.length - 1);
    const cat = charCategory(chars[idx]);
    if (cat === 'space') {
        return {
            startLine: lineIndex,
            startChar: 0,
            endLine: lineIndex,
            endChar: chars.length,
        };
    }

    let start = idx;
    let end = idx + 1;
    while (start > 0 && charCategory(chars[start - 1]) === cat) {
        start -= 1;
    }
    while (end < chars.length && charCategory(chars[end]) === cat) {
        end += 1;
    }
    return {
        startLine: lineIndex,
        startChar: start,
        endLine: lineIndex,
        endChar: end,
    };
}

/**
 * 全选当前贴图所有识别文字
 */
export function selectAll(doc) {
    if (!doc || doc.lines.length === 0) {
        return null;
    }
    const lastLine = doc.lines.length - 1;
    return {
        startLine: 0,
        startChar: 0,
        endLine: lastLine,
        endChar: doc.lines[lastLine].chars.length,
    };
}

/**
 * 提取选区对应的纯文本字符串
 */
export function extractSelectedText(doc, selection) {
    if (!doc || !selection) {
        return '';
    }
    const startLine = clamp(selection.startLine, 0, doc.lines.length - 1);
    const endLine = clamp(selection.endLine, startLine, doc.lines.length - 1);
    const parts = [];

    for (let i = startLine; i <= endLine; i += 1) {
        const line = doc.lines[i];
        const c0 = i === startLine ? clamp(selection.startChar, 0, line.chars.length) : 0;
        const c1 = i === endLine ? clamp(selection.endChar, 0, line.chars.length) : line.chars.length;
        const slice = line.chars.slice(c0, c1).join('');
        if (slice.length > 0) {
            if (parts.length > 0) {
                const prevLine = doc.lines[i - 1];
                parts.push(prevLine?.sameVisualLineNext ? ' ' : '\n');
            }
            parts.push(slice);
        }
    }
    return parts.join('');
}

/**
 * 计算选区在当前贴图显示尺寸下的高亮矩形列表（同一视觉行相邻选区自动桥接为连续平滑选区条）
 */
export function computeSelectionRects(doc, selection, displayWidth, displayHeight) {
    if (!doc || !selection || doc.lines.length === 0) {
        return [];
    }
    const scaleX = Math.max(1, displayWidth) / doc.imageWidth;
    const scaleY = Math.max(1, displayHeight) / doc.imageHeight;
    const startLine = clamp(selection.startLine, 0, doc.lines.length - 1);
    const endLine = clamp(selection.endLine, startLine, doc.lines.length - 1);
    const rects = [];
    let prevMeta = null;

    for (let i = startLine; i <= endLine; i += 1) {
        const line = doc.lines[i];
        const c0 = i === startLine ? clamp(selection.startChar, 0, line.chars.length) : 0;
        const c1 = i === endLine ? clamp(selection.endChar, 0, line.chars.length) : line.chars.length;
        if (c1 <= c0) {
            prevMeta = null;
            continue;
        }
        const [bx, by, bw, bh] = line.bbox;
        const x0 = bx + bw * line.charX[c0];
        const x1 = bx + bw * line.charX[c1];
        const rect = {
            x: x0 * scaleX,
            y: by * scaleY,
            width: Math.max(2, (x1 - x0) * scaleX),
            height: Math.max(2, bh * scaleY),
        };

        // 同视觉行且连续跨越边界、间距较小时，合并为同一条无缝高亮带
        if (prevMeta && prevMeta.sameVisualLineNext && prevMeta.toLineEnd && c0 === 0) {
            const last = rects[rects.length - 1];
            const gap = rect.x - (last.x + last.width);
            const maxBridge = Math.max(last.height, rect.height) * 2.5;
            if (gap <= maxBridge) {
                const top = Math.min(last.y, rect.y);
                const bottom = Math.max(last.y + last.height, rect.y + rect.height);
                const right = Math.max(last.x + last.width, rect.x + rect.width);
                last.y = top;
                last.height = bottom - top;
                last.width = right - last.x;
                prevMeta = {
                    sameVisualLineNext: line.sameVisualLineNext,
                    toLineEnd: c1 === line.chars.length,
                };
                continue;
            }
        }

        rects.push(rect);
        prevMeta = {
            sameVisualLineNext: line.sameVisualLineNext,
            toLineEnd: c1 === line.chars.length,
        };
    }
    return rects;
}
