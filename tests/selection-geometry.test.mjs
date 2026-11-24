import assert from 'node:assert/strict';
import test from 'node:test';

import {handleAt, handlesFor, moveRegion, regionBetween, resizeRegion, toolbarMonitor,
    toolbarPosition} from '../extensions/pinshot@dgxz99/selectionGeometry.js';

const bounds = {left: 10, top: 20, right: 300, bottom: 200};
const region = [50, 60, 100, 80];

test('creates the same selection for reverse drag', () => {
    assert.deepEqual(regionBetween([150, 60], [50, 140]), region);
});

test('hit tests eight handles in logical pixels and leaves interior free', () => {
    const handles = handlesFor(region);
    assert.equal(handles.length, 8);
    for (const handle of handles) {
        assert.deepEqual(handleAt(region, handle.slice(2), 7), handle);
    }
    assert.equal(handleAt(region, [100, 100], 7), null);
    assert.deepEqual(handleAt([10, 10, 8, 8], [17, 17], 7), [1, 1, 18, 18]);
});

test('moves without changing size, even when dragging past the boundary', () => {
    assert.deepEqual(moveRegion(region, [-200, 400], bounds), [10, 120, 100, 80]);
});

test('resize retains the opposite edge, clamps the minimum, and never crosses the bounds', () => {
    assert.deepEqual(resizeRegion(region, [1, 1], [500, 500], bounds, 32), [50, 60, 250, 140]);
    assert.deepEqual(resizeRegion(region, [-1, -1], [150, 150], bounds, 32), [118, 108, 32, 32]);
    assert.deepEqual(resizeRegion(region, [-1, 0], [500, -500], bounds, 32), [118, 60, 32, 80]);
});

const first = {x: 0, y: 0, width: 1024, height: 768};
const second = {x: 1024, y: 200, width: 1280, height: 800};

test('toolbar stays on the monitor with more of the selection', () => {
    assert.equal(toolbarMonitor([980, 250, 130, 150], [first, second]), second);
    assert.equal(toolbarMonitor([910, 350, 130, 150], [first, second]), first);
});

test('toolbar switches sides for edge selections and falls within a full display', () => {
    assert.deepEqual(toolbarPosition([50, 60, 300, 80], [170, 40], first), [180, 148]);
    assert.deepEqual(toolbarPosition([900, 720, 124, 48], [170, 40], first), [846, 672]);
    assert.deepEqual(toolbarPosition([1024, 200, 250, 800], [170, 40], second), [1282, 952]);
    assert.deepEqual(toolbarPosition([0, 0, 1024, 768], [170, 40], first), [846, 720]);
});
