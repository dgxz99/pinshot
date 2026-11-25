const clamp = (value, min, max) => Math.min(Math.max(value, min), max);

/** 将任意方向拖出的两点转换为同一坐标系下的矩形 */
export function regionBetween([x, y], [dx, dy]) {
    return [Math.min(x, dx), Math.min(y, dy), Math.abs(dx - x), Math.abs(dy - y)];
}

/** 八个控制点及其拖动方向，坐标为选区所在的逻辑坐标 */
export function handlesFor([x, y, width, height]) {
    const right = x + width;
    const bottom = y + height;
    return [
        [-1, -1, x, y], [1, -1, right, y], [-1, 1, x, bottom], [1, 1, right, bottom],
        [0, -1, x + width / 2, y], [0, 1, x + width / 2, bottom],
        [-1, 0, x, y + height / 2], [1, 0, right, y + height / 2],
    ];
}

/** 手柄命中优先于移动；细小选区按距离消解手柄重叠 */
export function handleAt(rect, [x, y], radius) {
    let nearest = null;
    let distance = Infinity;
    for (const handle of handlesFor(rect)) {
        const ox = x - handle[2];
        const oy = y - handle[3];
        const squared = ox * ox + oy * oy;
        if (Math.abs(ox) <= radius && Math.abs(oy) <= radius && squared < distance) {
            nearest = handle;
            distance = squared;
        }
    }
    return nearest;
}

export function containsPoint([x, y, width, height], [px, py]) {
    return px >= x && px <= x + width && py >= y && py <= y + height;
}

/** 移动到边界时保留原选区尺寸 */
export function moveRegion([x, y, width, height], [dx, dy], {left, top, right, bottom}) {
    return [clamp(x + dx, left, right - width), clamp(y + dy, top, bottom - height), width, height];
}

/** 只拖动命中的边，固定对侧；既不翻转也不低于此模式的最小选区尺寸 */
export function resizeRegion([x, y, width, height], [hx, hy], [dx, dy], bounds, minEdge) {
    let right = x + width;
    let bottom = y + height;
    if (hx < 0) {
        x = clamp(x + dx, bounds.left, right - minEdge);
    } else if (hx > 0) {
        right = clamp(right + dx, x + minEdge, bounds.right);
    }
    if (hy < 0) {
        y = clamp(y + dy, bounds.top, bottom - minEdge);
    } else if (hy > 0) {
        bottom = clamp(bottom + dy, y + minEdge, bounds.bottom);
    }
    return [x, y, right - x, bottom - y];
}

/** 只复用完全落在共享屏幕可录制范围内的原选区，不静默裁掉一部分 */
export function initialRegionOnMonitor(region, monitor, bounds, minEdge) {
    if (!region) {
        return null;
    }
    const [stageX, stageY, width, height] = region;
    const x = stageX - monitor.x;
    const y = stageY - monitor.y;
    if (width < minEdge || height < minEdge || x < bounds.left || y < bounds.top ||
        x + width > bounds.right || y + height > bounds.bottom) {
        return null;
    }
    return [x, y, width, height];
}

function overlapArea([x, y, width, height], {x: ax, y: ay, width: aw, height: ah}) {
    const horizontal = Math.max(0, Math.min(x + width, ax + aw) - Math.max(x, ax));
    const vertical = Math.max(0, Math.min(y + height, ay + ah) - Math.max(y, ay));
    return horizontal * vertical;
}

/** 跨屏时让工具条留在选区面积最大的屏幕，不落到虚拟桌面的空隙 */
export function toolbarMonitor(rect, monitors) {
    let best = null;
    let area = -1;
    for (const monitor of monitors) {
        const overlap = overlapArea(rect, monitor);
        if (overlap > area) {
            best = monitor;
            area = overlap;
        }
    }
    return best;
}

/** 优先贴下缘、上缘和两侧；录制时不允许工具条盖住采集区域 */
export function toolbarPosition([x, y, width, height], [tw, th], screen, {allowOverlap = true} = {}) {
    const gap = 8;
    if (!allowOverlap && (tw + 2 * gap > screen.width || th + 2 * gap > screen.height)) {
        return null;
    }
    const left = screen.x + gap;
    const top = screen.y + gap;
    const right = Math.max(left, screen.x + screen.width - tw - gap);
    const bottom = Math.max(top, screen.y + screen.height - th - gap);
    const alignX = clamp(x + width - tw, left, right);
    const alignY = clamp(y + height - th, top, bottom);
    const candidates = [[alignX, y + height + gap], [alignX, y - th - gap],
        [x + width + gap, alignY], [x - tw - gap, alignY]];
    return candidates.find(([tx, ty]) => tx >= left && tx <= right && ty >= top && ty <= bottom) ??
        (allowOverlap ? [alignX, bottom] : null);
}
