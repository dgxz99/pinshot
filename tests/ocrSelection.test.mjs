import assert from 'node:assert/strict';
import test from 'node:test';

import {
    computeSelectionRects,
    extractSelectedText,
    hitTestText,
    isInDragBorder,
    locateCaret,
    normalizeSelection,
    prepareOcrDocument,
    selectAll,
    selectWordAt,
    toImageCoords,
} from '../extensions/pinshot@dgxz99/ocrSelection.js';

function sampleDoc() {
    return prepareOcrDocument({
        version: 1,
        model_id: 'ppocrv5-mobile',
        image_width: 800,
        image_height: 400,
        lines: [
            {
                bbox: [40, 40, 200, 30],
                text: 'Hello 贴图',
                confidence: 0.99,
                char_x: [0, 0.12, 0.22, 0.29, 0.36, 0.48, 0.56, 0.78, 1.0],
                same_visual_line_next: true,
            },
            {
                bbox: [280, 42, 160, 28],
                text: 'World',
                confidence: 0.98,
                char_x: [0, 0.25, 0.45, 0.62, 0.78, 1.0],
                same_visual_line_next: false,
            },
            {
                bbox: [40, 110, 300, 30],
                text: 'fn main() -> Result',
                confidence: 0.97,
                same_visual_line_next: false,
            },
        ],
    });
}

test('hitTestText locates caret inside text lines and returns null on blank area', () => {
    const doc = sampleDoc();
    assert.equal(hitTestText(doc, 10, 10), null);

    const hit = hitTestText(doc, 40 + 200 * 0.78, 55);
    assert.deepEqual(hit, {lineIndex: 0, charIndex: 7});
});

test('isInDragBorder reserves edge grip while shrinking safely on tiny pins', () => {
    assert.equal(isInDragBorder(4, 100, 400, 300), true);
    assert.equal(isInDragBorder(396, 100, 400, 300), true);
    assert.equal(isInDragBorder(50, 50, 400, 300), false);
    assert.equal(isInDragBorder(6, 6, 30, 30, 8), false);
});

test('cross-line selection handles reverse drag, same visual row spacing, and scaling', () => {
    const doc = sampleDoc();
    const anchor = locateCaret(doc, 200, 125); // line 2
    const focus = locateCaret(doc, 40 + 200 * 0.56, 55); // line 0 after "Hello "
    const sel = normalizeSelection(anchor, focus);
    assert.ok(sel);
    assert.equal(sel.startLine, 0);
    assert.equal(sel.endLine, 2);

    const text = extractSelectedText(doc, sel);
    assert.match(text, /^贴图 World\nfn /);

    // 2x display shrink -> 同一视觉行的 line 0 与 line 1 自动桥接为 1 条连续高亮带，加上 line 2 共 2 条
    const rects = computeSelectionRects(doc, sel, 400, 200);
    assert.equal(rects.length, 2);
    assert.ok(Math.abs(rects[0].y - 20) < 0.01);
    assert.ok(Math.abs(rects[0].height - 15) < 0.01);
    assert.ok(rects[0].x + rects[0].width >= 219.9);

    const imgCoords = toImageCoords(100, 50, 400, 200, doc);
    assert.deepEqual(imgCoords, {ix: 200, iy: 100});
});

test('double-click selects contiguous ASCII or CJK token without closing pin', () => {
    const doc = sampleDoc();
    const cjk = selectWordAt(doc, 0, 6);
    assert.equal(extractSelectedText(doc, cjk), '贴图');

    const ascii = selectWordAt(doc, 2, 4);
    assert.equal(extractSelectedText(doc, ascii), 'main');

    const full = selectAll(doc);
    assert.equal(extractSelectedText(doc, full), 'Hello 贴图 World\nfn main() -> Result');
});
