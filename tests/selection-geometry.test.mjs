import assert from 'node:assert/strict';
import test from 'node:test';

import {handleAt, handlesFor, initialRegionOnMonitor, moveRegion, regionBetween, resizeRegion, toolbarMonitor,
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

test('reuses absolute selection only if the entire region fits the shared recording bounds', () => {
    const area = {x: -1024, y: 200, width: 1024, height: 768};
    const workArea = {left: 4, top: 42, right: 1020, bottom: 700};
    assert.deepEqual(initialRegionOnMonitor([-924, 311, 400, 252], area, workArea, 32), [100, 111, 400, 252]);
    assert.equal(initialRegionOnMonitor([300, 311, 400, 252], area, workArea, 32), null);
    assert.equal(initialRegionOnMonitor([-40, 311, 400, 252], area, workArea, 32), null);
    assert.equal(initialRegionOnMonitor([-924, 800, 400, 252], area, workArea, 32), null);
    assert.equal(initialRegionOnMonitor([-924, 311, 16, 252], area, workArea, 32), null);
    assert.equal(initialRegionOnMonitor(null, area, workArea, 32), null);
});

test('toolbar stays on the monitor with more of the selection', () => {
    assert.equal(toolbarMonitor([980, 250, 130, 150], [first, second]), second);
    assert.equal(toolbarMonitor([910, 350, 130, 150], [first, second]), first);
});

test('recording toolbar stays entirely outside the captured region or reports no room', () => {
    const options = {allowOverlap: false};
    assert.deepEqual(toolbarPosition([100, 80, 500, 300], [270, 34], first, options), [330, 388]);
    assert.deepEqual(toolbarPosition([200, 700, 600, 68], [270, 34], first, options), [530, 658]);
    assert.deepEqual(toolbarPosition([0, 0, 500, 768], [270, 34], first, options), [508, 726]);
    assert.equal(toolbarPosition([0, 0, 1024, 768], [270, 34], first, options), null);
    assert.equal(toolbarPosition([300, 300, 500, 100], [1020, 34], first, options), null);
});

test('toolbar switches sides for edge selections and falls within a full display', () => {
    assert.deepEqual(toolbarPosition([50, 60, 300, 80], [170, 40], first), [180, 148]);
    assert.deepEqual(toolbarPosition([900, 720, 124, 48], [170, 40], first), [846, 672]);
    assert.deepEqual(toolbarPosition([1024, 200, 250, 800], [170, 40], second), [1282, 952]);
    assert.deepEqual(toolbarPosition([0, 0, 1024, 768], [170, 40], first), [846, 720]);
});
