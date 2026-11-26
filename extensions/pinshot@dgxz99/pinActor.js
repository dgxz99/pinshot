import Clutter from 'gi://Clutter';
import Cogl from 'gi://Cogl';
import GdkPixbuf from 'gi://GdkPixbuf';
import GLib from 'gi://GLib';
import St from 'gi://St';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';

import {PinContextMenu} from './contextMenu.js';

const DOUBLE_CLICK_MS = 320;
const DOUBLE_CLICK_MAX_DELTA = 8;

const clamp = (value, min, max) => Math.min(Math.max(value, min), max);

/**
 * 单张「真·非窗口」悬浮贴图（挂载在 uiGroup 顶层，严格遵守 4 条内存防泄漏红线）
 */
export class PinActor {
    constructor(payload, config, callbacks) {
        this.id = payload.id;
        this._config = config;
        this._callbacks = callbacks;

        this._imagePath = payload.image_path;
        this._thumbPath = payload.thumb_path;
        this._pixelWidth = Math.max(1, Number(payload.width) || 1);
        this._pixelHeight = Math.max(1, Number(payload.height) || 1);
        this._sourceScale = Number(payload.source_scale) > 0 ? Number(payload.source_scale) : 1.0;
        this.kind = payload.kind ?? 'screenshot';
        this.hasText = Boolean(payload.has_text);
        this.titlePreview = payload.title_preview ?? '';

        // 逻辑 1:1 基准尺寸（分数缩放严格还原）
        this._baseWidth = Math.max(24, Math.round(this._pixelWidth / this._sourceScale));
        this._baseHeight = Math.max(24, Math.round(this._pixelHeight / this._sourceScale));

        this._scale = 1.0;
        this._opacity = clamp(this._config.default_opacity ?? 1.0, 0.15, 1.0);
        this._opacityBeforeGhost = this._opacity;
        this._mini = false;
        this._ghost = false;
        this._selected = false;
        this._tempHidden = false;
        this._fullscreenHidden = false;
        this._savedNormalState = null;

        this._dragging = false;
        this._dragStartX = 0;
        this._dragStartY = 0;
        this._dragOriginX = 0;
        this._dragOriginY = 0;
        this._lastClickTimeUs = 0;
        this._lastClickX = 0;
        this._lastClickY = 0;

        this._actorSignalIds = [];
        this._stageDragSignalId = 0;
        this._destroyed = false;

        this.actor = new St.Widget({
            name: `pinshot-pin-${this.id}`,
            style_class: 'pinshot-pin',
            reactive: true,
            can_focus: true,
            layout_manager: new Clutter.BinLayout(),
        });

        this._imageWidget = new St.Widget({
            x_expand: true,
            y_expand: true,
            reactive: false,
            content_gravity: Clutter.ContentGravity.RESIZE_FILL,
        });
        this.actor.add_child(this._imageWidget);

        this._contextMenu = new PinContextMenu(this, callbacks.onRequestClose);

        this._initGeometry(payload);
        this._loadFullTexture();
        this._applyOpacity(this._opacity);
        this._actorSignalIds = [
            this.actor.connect('button-press-event', (_actor, event) => this._onButtonPress(event)),
            this.actor.connect('scroll-event', (_actor, event) => this._onScroll(event)),
        ];
    }

    get mini() {
        return this._mini;
    }

    get ghost() {
        return this._ghost;
    }

    _scaledSize() {
        return [
            Math.max(24, Math.round(this._baseWidth * this._scale)),
            Math.max(24, Math.round(this._baseHeight * this._scale)),
        ];
    }

    /**
     * 计算初始缩放与弹出位置：
     * - 区域截图带原始选区坐标时，按 1:1 尺寸原位贴出；
     * - 其他来源（剪贴板、长图、历史恢复）超工作区 75% 时等比收敛并围绕指针居中。
     */
    _initGeometry(payload) {
        if (Number.isFinite(payload.initial_x) && Number.isFinite(payload.initial_y)) {
            this._zoomMin = this._config.min_scale;
            this._scale = 1.0;
            const [w, h] = this._scaledSize();
            const clamped = this._clampPositionToMonitors(payload.initial_x, payload.initial_y, w, h);
            this.actor.set_size(w, h);
            this.actor.set_position(clamped.x, clamped.y);
            return;
        }

        const [ptrX, ptrY] = global.get_pointer();
        const monitorIndex = this._findMonitorIndexAt(ptrX, ptrY);
        const workArea = Main.layoutManager.getWorkAreaForMonitor(monitorIndex);

        const maxInitW = Math.max(120, Math.floor(workArea.width * 0.75));
        const maxInitH = Math.max(120, Math.floor(workArea.height * 0.75));

        let initScale = 1.0;
        if (this._baseWidth > maxInitW || this._baseHeight > maxInitH) {
            initScale = Math.min(maxInitW / this._baseWidth, maxInitH / this._baseHeight);
        }
        // 长图的适屏比例可能低于普通贴图下限，滚轮也要能回到该比例
        this._zoomMin = this.kind === 'longshot' ? Math.min(this._config.min_scale, initScale) : this._config.min_scale;
        this._scale = Math.max(this._zoomMin, initScale);

        const [w, h] = this._scaledSize();

        // 初始弹出尽量完整留在当前显示器工作区内
        const maxX = workArea.x + Math.max(0, workArea.width - w);
        const maxY = workArea.y + Math.max(0, workArea.height - h);
        const x = clamp(Math.round(ptrX - w / 2), workArea.x, maxX);
        const y = clamp(Math.round(ptrY - h / 2), workArea.y, maxY);

        this.actor.set_size(w, h);
        this.actor.set_position(x, y);
    }

    _onButtonPress(event) {
        if (this._destroyed || this._ghost) {
            return Clutter.EVENT_PROPAGATE;
        }

        const button = event.get_button();
        const [stageX, stageY] = event.get_coords();

        if (button === Clutter.BUTTON_MIDDLE) {
            this._callbacks?.onRequestClose?.(this.id);
            return Clutter.EVENT_STOP;
        }

        if (button === Clutter.BUTTON_SECONDARY) {
            this._lastClickTimeUs = 0;
            this._callbacks?.onRaise?.(this);
            this._contextMenu.openAt(stageX, stageY);
            return Clutter.EVENT_STOP;
        }

        if (button === Clutter.BUTTON_PRIMARY) {
            this._callbacks?.onRaise?.(this);
            this._callbacks?.onSelect?.(this);

            const nowUs = GLib.get_monotonic_time();
            const dtMs = (nowUs - this._lastClickTimeUs) / 1000;
            const dist = Math.hypot(stageX - this._lastClickX, stageY - this._lastClickY);

            if (this._lastClickTimeUs !== 0 && dtMs > 0 &&
                dtMs <= DOUBLE_CLICK_MS && dist <= DOUBLE_CLICK_MAX_DELTA) {
                this._lastClickTimeUs = 0;
                this._callbacks?.onRequestClose?.(this.id);
                return Clutter.EVENT_STOP;
            }

            this._lastClickTimeUs = nowUs;
            this._lastClickX = stageX;
            this._lastClickY = stageY;

            this._startDrag(stageX, stageY);
            return Clutter.EVENT_STOP;
        }

        return Clutter.EVENT_PROPAGATE;
    }

    _startDrag(stageX, stageY) {
        this._stopDrag();
        this._dragging = true;
        this._dragStartX = stageX;
        this._dragStartY = stageY;
        this._dragOriginX = this.actor.x;
        this._dragOriginY = this.actor.y;

        // 仅拖拽期间挂在 stage 上捕获位移与抬起事件，避免 grab 抢占底层编辑器焦点
        this._stageDragSignalId = global.stage.connect('captured-event', (_stage, event) => {
            if (!this._dragging || this._destroyed) {
                return Clutter.EVENT_PROPAGATE;
            }
            const type = event.type();
            if (type === Clutter.EventType.MOTION) {
                const [curX, curY] = event.get_coords();
                const dx = curX - this._dragStartX;
                const dy = curY - this._dragStartY;
                // 拖动后立即再点击不应被识别为关闭手势
                if (Math.hypot(dx, dy) > DOUBLE_CLICK_MAX_DELTA) {
                    this._lastClickTimeUs = 0;
                }
                const nextX = this._dragOriginX + dx;
                const nextY = this._dragOriginY + dy;
                const clamped = this._clampPositionToMonitors(nextX, nextY, this.actor.width, this.actor.height);
                this.actor.set_position(clamped.x, clamped.y);
                return Clutter.EVENT_STOP;
            }
            if (type === Clutter.EventType.BUTTON_RELEASE &&
                event.get_button() === Clutter.BUTTON_PRIMARY) {
                this._stopDrag();
                return Clutter.EVENT_STOP;
            }
            return Clutter.EVENT_PROPAGATE;
        });
    }

    _stopDrag() {
        this._dragging = false;
        if (this._stageDragSignalId) {
            try {
                global.stage.disconnect(this._stageDragSignalId);
            } catch (err) {
                console.warn(`[pinshot] 断开拖拽信号失败: ${err}`);
            }
            this._stageDragSignalId = 0;
        }
    }

    _onScroll(event) {
        if (this._destroyed || this._ghost) {
            return Clutter.EVENT_PROPAGATE;
        }

        const direction = event.get_scroll_direction();
        let stepSign = 0;
        if (direction === Clutter.ScrollDirection.UP) {
            stepSign = 1;
        } else if (direction === Clutter.ScrollDirection.DOWN) {
            stepSign = -1;
        } else if (direction === Clutter.ScrollDirection.SMOOTH) {
            const [, dy] = event.get_scroll_delta();
            if (Math.abs(dy) < 0.01) {
                return Clutter.EVENT_STOP;
            }
            stepSign = dy < 0 ? 1 : -1;
        } else {
            return Clutter.EVENT_PROPAGATE;
        }

        const state = event.get_state();
        const isCtrl = Boolean(state & Clutter.ModifierType.CONTROL_MASK);

        if (isCtrl) {
            const delta = stepSign * this._config.opacity_step;
            this._applyOpacity(this._opacity + delta);
            this._opacityBeforeGhost = this._opacity;
            return Clutter.EVENT_STOP;
        }

        if (this._mini) {
            return Clutter.EVENT_STOP;
        }

        const [stageX, stageY] = event.get_coords();
        const factor = stepSign > 0 ? (1 + this._config.zoom_step) : (1 / (1 + this._config.zoom_step));
        this._zoomAroundPoint(this._scale * factor, stageX, stageY);
        return Clutter.EVENT_STOP;
    }

    _zoomAroundPoint(targetScale, anchorStageX, anchorStageY) {
        const nextScale = clamp(targetScale, this._zoomMin, this._config.max_scale);
        if (Math.abs(nextScale - this._scale) < 0.001) {
            return;
        }

        const oldW = Math.max(1, this.actor.width);
        const oldH = Math.max(1, this.actor.height);
        const relX = (anchorStageX - this.actor.x) / oldW;
        const relY = (anchorStageY - this.actor.y) / oldH;

        this._scale = nextScale;
        const [newW, newH] = this._scaledSize();

        const rawX = Math.round(anchorStageX - relX * newW);
        const rawY = Math.round(anchorStageY - relY * newH);
        const clamped = this._clampPositionToMonitors(rawX, rawY, newW, newH);

        this.actor.set_size(newW, newH);
        this.actor.set_position(clamped.x, clamped.y);
    }

    /**
     * 恢复 100% 原始 1:1 逻辑尺寸（若当前在迷你态会先展开）
     */
    resetScaleTo100() {
        if (this._mini) {
            this.toggleMini();
        }
        const centerX = this.actor.x + this.actor.width / 2;
        const centerY = this.actor.y + this.actor.height / 2;
        this._zoomAroundPoint(1.0, centerX, centerY);
    }

    /**
     * 切换迷你缩略图态：切入时立即销毁高清纹理并加载缩略图，展开时从磁盘重新载入
     */
    toggleMini() {
        if (this._destroyed) {
            return;
        }

        if (!this._mini) {
            this._savedNormalState = {
                x: this.actor.x,
                y: this.actor.y,
                scale: this._scale,
            };
            this._mini = true;
            this.actor.add_style_class_name('pinshot-pin-mini');

            const miniSize = Math.max(32, this._config.mini_size_px ?? 64);
            this._releaseTexture();
            this._loadThumbTexture(miniSize);

            this.actor.set_size(miniSize, miniSize);
            const clamped = this._clampPositionToMonitors(this.actor.x, this.actor.y, miniSize, miniSize);
            this.actor.set_position(clamped.x, clamped.y);
        } else {
            this._mini = false;
            this.actor.remove_style_class_name('pinshot-pin-mini');

            const prev = this._savedNormalState;
            if (prev) {
                this._scale = prev.scale;
            }
            const [w, h] = this._scaledSize();

            this._releaseTexture();
            this._loadFullTexture();

            this.actor.set_size(w, h);
            const targetX = prev ? prev.x : this.actor.x;
            const targetY = prev ? prev.y : this.actor.y;
            const clamped = this._clampPositionToMonitors(targetX, targetY, w, h);
            this.actor.set_position(clamped.x, clamped.y);
        }
    }

    /**
     * 切换点击穿透模式（Ghost Mode）并自动压低不透明度提示状态
     */
    setGhost(enabled) {
        if (this._destroyed || this._ghost === enabled) {
            return;
        }
        this._ghost = enabled;
        this.actor.reactive = !enabled;

        if (enabled) {
            this._stopDrag();
            this._contextMenu.close();
            this.setSelected(false);
            this._opacityBeforeGhost = this._opacity;
            this.actor.add_style_class_name('pinshot-pin-ghost');
            if (this._opacity > 0.75) {
                this._applyOpacity(0.65);
            }
        } else {
            this.actor.remove_style_class_name('pinshot-pin-ghost');
            this._applyOpacity(this._opacityBeforeGhost);
        }
    }

    setSelected(selected) {
        if (this._destroyed || this._selected === selected) {
            return;
        }
        this._selected = selected;
        if (selected) {
            this.actor.add_style_class_name('pinshot-pin-selected');
        } else {
            this.actor.remove_style_class_name('pinshot-pin-selected');
        }
    }

    setTemporaryHidden(hidden) {
        this._tempHidden = hidden;
        this._syncVisibility();
    }

    setFullscreenHidden(hidden) {
        this._fullscreenHidden = hidden;
        this._syncVisibility();
    }

    _syncVisibility() {
        if (!this.actor) {
            return;
        }
        this.actor.visible = !(this._tempHidden || this._fullscreenHidden);
    }

    /**
     * 原地替换贴图图像内容（用于 satty 标注回流等场景）
     */
    updatePayload(payload) {
        if (this._destroyed) {
            return;
        }
        this._imagePath = payload.image_path;
        this._thumbPath = payload.thumb_path;
        this._pixelWidth = Math.max(1, Number(payload.width) || this._pixelWidth);
        this._pixelHeight = Math.max(1, Number(payload.height) || this._pixelHeight);
        this._sourceScale = Number(payload.source_scale) > 0 ? Number(payload.source_scale) : this._sourceScale;
        this.hasText = Boolean(payload.has_text);
        this.titlePreview = payload.title_preview ?? this.titlePreview;

        this._baseWidth = Math.max(24, Math.round(this._pixelWidth / this._sourceScale));
        this._baseHeight = Math.max(24, Math.round(this._pixelHeight / this._sourceScale));
        this._tempHidden = false;
        this._syncVisibility();

        this._releaseTexture();
        if (this._mini) {
            const miniSize = Math.max(32, this._config.mini_size_px ?? 64);
            this._loadThumbTexture(miniSize);
        } else {
            this._loadFullTexture();
            const [w, h] = this._scaledSize();
            this.actor.set_size(w, h);
        }
    }

    /**
     * 显示器热插拔后检查贴图是否落到无效区域外，若越界则拉回主屏可见区
     */
    ensureVisibleOnMonitors() {
        if (this._destroyed || !this.actor) {
            return;
        }
        const clamped = this._clampPositionToMonitors(
            this.actor.x,
            this.actor.y,
            this.actor.width,
            this.actor.height
        );
        this.actor.set_position(clamped.x, clamped.y);
    }

    /**
     * 返回当前贴图中心点所在的显示器索引
     */
    getMonitorIndex() {
        const cx = this.actor.x + this.actor.width / 2;
        const cy = this.actor.y + this.actor.height / 2;
        return this._findMonitorIndexAt(cx, cy);
    }

    /**
     * 导出供 D-Bus `ListActivePins` 与顶栏菜单使用的状态快照
     */
    toInfo() {
        return {
            id: this.id,
            x: this.actor.x,
            y: this.actor.y,
            width: this.actor.width,
            height: this.actor.height,
            scale: this._scale,
            opacity: this._opacity,
            mini: this._mini,
            ghost: this._ghost,
            hidden: this._tempHidden || this._fullscreenHidden,
            title_preview: this.titlePreview,
            kind: this.kind,
            has_text: this.hasText,
        };
    }

    /**
     * 加载高清纹理（红线 2：超大图与超长图按 max_texture_edge_px 及 2x 屏分辨率上限降采样）
     */
    _loadFullTexture() {
        const maxEdge = this._computeMaxTextureEdge();
        this._loadTextureFromPath(this._imagePath, maxEdge);
    }

    _loadThumbTexture(miniSize) {
        const path = GLib.file_test(this._thumbPath, GLib.FileTest.EXISTS)
            ? this._thumbPath
            : this._imagePath;
        const targetEdge = Math.max(128, miniSize * 2);
        this._loadTextureFromPath(path, targetEdge);
    }

    _computeMaxTextureEdge() {
        const cfgLimit = Math.max(512, this._config.max_texture_edge_px ?? 4096);
        let maxMonitorEdge = 1920;
        for (const monitor of Main.layoutManager.monitors) {
            const physEdge = Math.max(monitor.width, monitor.height) * (monitor.scale || 1);
            if (physEdge > maxMonitorEdge) {
                maxMonitorEdge = physEdge;
            }
        }
        return Math.round(Math.min(cfgLimit, maxMonitorEdge * 2));
    }

    _loadTextureFromPath(filePath, maxEdge) {
        try {
            const [, imgW, imgH] = GdkPixbuf.Pixbuf.get_file_info(filePath);
            let pixbuf;
            if (imgW > maxEdge || imgH > maxEdge) {
                pixbuf = GdkPixbuf.Pixbuf.new_from_file_at_scale(filePath, maxEdge, maxEdge, true);
            } else {
                pixbuf = GdkPixbuf.Pixbuf.new_from_file(filePath);
            }

            const width = pixbuf.get_width();
            const height = pixbuf.get_height();
            const rowstride = pixbuf.get_rowstride();
            const hasAlpha = pixbuf.get_has_alpha();
            const pixelBytes = pixbuf.read_pixel_bytes();
            const pixelFormat = hasAlpha ? Cogl.PixelFormat.RGBA_8888 : Cogl.PixelFormat.RGB_888;

            const coglContext = global.stage.get_context().get_backend().get_cogl_context();
            const content = St.ImageContent.new_with_preferred_size(width, height);
            content.set_bytes(coglContext, pixelBytes, pixelFormat, width, height, rowstride);

            this._imageWidget.content = content;
        } catch (err) {
            console.error(`[pinshot] 加载贴图纹理失败 (${filePath}): ${err}`);
        }
    }

    _releaseTexture() {
        if (this._imageWidget) {
            this._imageWidget.content = null;
        }
    }

    _applyOpacity(val) {
        this._opacity = clamp(val, 0.15, 1.0);
        if (this.actor) {
            this.actor.opacity = Math.round(this._opacity * 255);
        }
    }

    _findMonitorIndexAt(x, y) {
        const monitors = Main.layoutManager.monitors;
        for (let i = 0; i < monitors.length; i += 1) {
            const m = monitors[i];
            if (x >= m.x && x < m.x + m.width && y >= m.y && y < m.y + m.height) {
                return i;
            }
        }
        return Main.layoutManager.primaryIndex;
    }

    /**
     * 钳制拖拽坐标，确保贴图至少保留 min_visible_edge_px 在有效显示器合并边界内
     */
    _clampPositionToMonitors(x, y, width, height) {
        const monitors = Main.layoutManager.monitors;
        if (!monitors || monitors.length === 0) {
            return {x: Math.round(x), y: Math.round(y)};
        }

        const edge = Math.min(
            Math.max(16, this._config.min_visible_edge_px ?? 32),
            Math.floor(Math.min(width, height) / 2)
        );

        let minX = Infinity;
        let minY = Infinity;
        let maxX = -Infinity;
        let maxY = -Infinity;
        for (const m of monitors) {
            minX = Math.min(minX, m.x);
            minY = Math.min(minY, m.y);
            maxX = Math.max(maxX, m.x + m.width);
            maxY = Math.max(maxY, m.y + m.height);
        }

        const clampedX = clamp(x, minX - width + edge, maxX - edge);
        const clampedY = clamp(y, minY - height + edge, maxY - edge);

        // 检查钳制后是否仍与任意显示器相交（处理非矩形多屏排列的死角）
        const overlapsAny = monitors.some(m => {
            return clampedX + width > m.x + edge &&
                clampedX < m.x + m.width - edge &&
                clampedY + height > m.y + edge &&
                clampedY < m.y + m.height - edge;
        });

        if (overlapsAny) {
            return {x: Math.round(clampedX), y: Math.round(clampedY)};
        }

        // 回退拉回主屏可见区域
        const primary = Main.layoutManager.primaryMonitor ?? monitors[0];
        const safeX = clamp(clampedX, primary.x, primary.x + Math.max(0, primary.width - edge));
        const safeY = clamp(clampedY, primary.y, primary.y + Math.max(0, primary.height - edge));
        return {x: Math.round(safeX), y: Math.round(safeY)};
    }

    /**
     * 红线 4：严格断开全部信号、关闭菜单、释放纹理引用并销毁 actor
     */
    destroy() {
        if (this._destroyed) {
            return;
        }
        this._destroyed = true;
        this._stopDrag();

        if (this._contextMenu) {
            this._contextMenu.destroy();
            this._contextMenu = null;
        }

        if (this.actor) {
            for (const sigId of this._actorSignalIds) {
                try {
                    this.actor.disconnect(sigId);
                } catch (err) {
                    console.warn(`[pinshot] 断开贴图信号失败: ${err}`);
                }
            }
        }
        this._actorSignalIds = [];

        this._releaseTexture();
        if (this._imageWidget) {
            this._imageWidget.destroy();
            this._imageWidget = null;
        }

        if (this.actor) {
            this.actor.get_parent()?.remove_child(this.actor);
            this.actor.destroy();
            this.actor = null;
        }

        this._callbacks = null;
    }
}
