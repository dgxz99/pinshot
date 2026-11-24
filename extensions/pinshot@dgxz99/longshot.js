import Clutter from 'gi://Clutter';
import Gio from 'gi://Gio';
import Pango from 'gi://Pango';
import Shell from 'gi://Shell';
import St from 'gi://St';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';

const HUD_SPACE = 68;
const MIN_EDGE = 32;
const OUTLINE_WIDTH = 2;
// 裁剪边界会在像素空间向外取整，边框与选区之间留空避免分数缩放时串色
const OUTLINE_GAP = 2;
const OUTLINE_MARGIN = OUTLINE_WIDTH + OUTLINE_GAP;
const CANCELLED = [0, 0, 0, 0];

/** 录制边框只在采集范围外绘制，前景和四条边都不参与输入拾取 */
function showRecordingOutline(monitor, rect) {
    const [x, y, width, height] = rect;
    const left = x - OUTLINE_MARGIN;
    const top = y - OUTLINE_MARGIN;
    const outerWidth = width + 2 * OUTLINE_MARGIN;
    const outerHeight = height + 2 * OUTLINE_MARGIN;
    const outline = new St.Widget({name: 'pinshot-longshot-recording-outline',
        x: monitor.x, y: monitor.y, width: monitor.width, height: monitor.height,
        reactive: false, clip_to_allocation: true});
    const strips = [
        [left, top, outerWidth, OUTLINE_WIDTH],
        [left, top + outerHeight - OUTLINE_WIDTH, outerWidth, OUTLINE_WIDTH],
        [left, top + OUTLINE_WIDTH, OUTLINE_WIDTH, outerHeight - 2 * OUTLINE_WIDTH],
        [left + outerWidth - OUTLINE_WIDTH, top + OUTLINE_WIDTH,
            OUTLINE_WIDTH, outerHeight - 2 * OUTLINE_WIDTH],
    ];
    for (const [sx, sy, sw, sh] of strips) {
        outline.add_child(new St.Widget({x: sx, y: sy, width: sw, height: sh,
            style_class: 'pinshot-longshot-recording-edge', reactive: false}));
    }
    Main.layoutManager.uiGroup.add_child(outline);
    return outline;
}

/** 只在用户明确框选时抓取输入，框选完立即归还应用焦点 */
class RegionSelector {
    constructor(monitor, workArea) {
        this._monitor = monitor;
        this._workArea = workArea;
        const left = workArea.x - monitor.x;
        const top = workArea.y - monitor.y;
        this._bounds = {
            left: left + OUTLINE_MARGIN,
            top: top + OUTLINE_MARGIN,
            right: left + workArea.width - OUTLINE_MARGIN,
            bottom: top + workArea.height - HUD_SPACE,
        };
        this._origin = null;
        this._grab = null;
    }

    select() {
        const {x, y, width, height} = this._monitor;
        this._layer = new St.Widget({
            name: 'pinshot-longshot-selector', reactive: true, can_focus: true,
            x, y, width, height, clip_to_allocation: true,
        });
        this._shades = Array.from({length: 4}, () => new St.Widget({style_class: 'pinshot-longshot-shade'}));
        this._outline = new St.Widget({style_class: 'pinshot-longshot-outline'});
        for (const actor of [...this._shades, this._outline]) {
            this._layer.add_child(actor);
        }
        this._outline.hide();
        this._shades[0].set_size(width, height);
        for (const shade of this._shades.slice(1)) {
            shade.hide();
        }
        this._hint = new St.Label({
            style_class: 'pinshot-longshot-hint', text: '拖动框选滚动内容 · Esc 取消',
        });
        this._hint.clutter_text.ellipsize = Pango.EllipsizeMode.END;
        this._hint.set_position(this._bounds.left + 12, this._bounds.bottom + 10);
        this._hint.set_width(this._workArea.width - 24);
        this._layer.add_child(this._hint);
        Main.layoutManager.uiGroup.add_child(this._layer);
        this._grab = Main.pushModal(this._layer, {actionMode: Shell.ActionMode.SYSTEM_MODAL});
        if (this._grab.is_revoked()) {
            this.close();
            throw new Error('无法进入长截图框选，请先关闭其他 GNOME 弹层');
        }
        this._revokedId = this._grab.connect('notify::revoked', () => {
            if (this._grab?.is_revoked()) {
                this.close();
            }
        });
        this._layer.connect('captured-event', (_actor, event) => this._onEvent(event));
        this._layer.connect('destroy', () => {
            // Shell 在关闭时可能先销毁场景节点；Main 会随节点自动释放 modal grab
            this._layer = null;
            this._clearGrabHandler();
            this._grab = null;
            this._resolveSelection(CANCELLED);
        });
        return new Promise(resolve => {
            this._resolve = resolve;
        });
    }

    _onEvent(event) {
        const type = event.type();
        if (type === Clutter.EventType.KEY_PRESS && event.get_key_symbol() === Clutter.KEY_Escape) {
            this.close();
        } else if (type === Clutter.EventType.BUTTON_PRESS) {
            if (event.get_button() === Clutter.BUTTON_PRIMARY) {
                this._origin = this._pointAt(event, false);
                if (this._origin) {
                    this._draw(this._origin);
                }
            } else if (event.get_button() === Clutter.BUTTON_SECONDARY) {
                this.close();
            }
        } else if (this._origin && type === Clutter.EventType.MOTION) {
            this._draw(this._pointAt(event, true));
        } else if (this._origin && type === Clutter.EventType.BUTTON_RELEASE &&
            event.get_button() === Clutter.BUTTON_PRIMARY) {
            const rect = this._rectAt(this._pointAt(event, true));
            if (rect[2] >= MIN_EDGE && rect[3] >= MIN_EDGE) {
                this.close(rect);
            } else {
                this._draw(this._origin);
                this._origin = null;
                this._hint.text = '区域太小，请重新拖动 · Esc 取消';
            }
        }
        return Clutter.EVENT_STOP;
    }

    _pointAt(event, clamp) {
        const {left, top, right, bottom} = this._bounds;
        const [stageX, stageY] = event.get_coords();
        const x = Math.round(stageX - this._monitor.x);
        const y = Math.round(stageY - this._monitor.y);
        if (!clamp && (x < left || x >= right || y < top || y >= bottom)) {
            return null;
        }
        return [Math.min(Math.max(left, x), right), Math.min(Math.max(top, y), bottom)];
    }

    _rectAt(point) {
        const [x, y] = this._origin;
        return [Math.min(x, point[0]), Math.min(y, point[1]),
            Math.abs(point[0] - x), Math.abs(point[1] - y)];
    }

    _draw(point) {
        const [x, y, width, height] = this._rectAt(point);
        const right = x + width;
        const bottom = y + height;
        const areas = [
            [0, 0, this._monitor.width, y],
            [0, y, x, height],
            [right, y, this._monitor.width - right, height],
            [0, bottom, this._monitor.width, this._monitor.height - bottom],
        ];
        for (const [index, [sx, sy, sw, sh]] of areas.entries()) {
            this._shades[index].set_position(sx, sy);
            this._shades[index].set_size(sw, sh);
            this._shades[index].show();
        }
        this._outline.set_position(x, y);
        this._outline.set_size(width, height);
        this._outline.visible = width > 0 && height > 0;
    }

    close(rect = CANCELLED) {
        this._clearGrabHandler();
        this._resolveSelection(rect);
        this._layer?.destroy();
        this._layer = null;
        this._grab = null;
    }

    _clearGrabHandler() {
        if (this._revokedId) {
            this._grab.disconnect(this._revokedId);
            this._revokedId = 0;
        }
    }

    _resolveSelection(rect) {
        const resolve = this._resolve;
        this._resolve = null;
        resolve?.(rect);
    }
}

/** 长图 UI 由发起录制的 D-Bus 连接持有；进程退出时自动恢复贴图 */
export class LongshotController {
    constructor(pinManager, emitAction) {
        this._pinManager = pinManager;
        this._emitAction = emitAction;
        this._session = null;
    }

    get active() {
        return this._session !== null;
    }

    async select(owner, connection, geometry) {
        if (this.active) {
            throw new Error('已有长截图正在运行');
        }
        if (Main.sessionMode.isLocked) {
            return CANCELLED;
        }
        const [x, y, width, height] = geometry;
        const monitor = Main.layoutManager.monitors.find(candidate =>
            candidate.x === x && candidate.y === y && candidate.width === width && candidate.height === height);
        if (!monitor) {
            throw new Error('显示器布局已变化，无法定位共享屏幕，请重新开始');
        }
        const workArea = Main.layoutManager.getWorkAreaForMonitor(monitor.index);
        if (workArea.height < HUD_SPACE + MIN_EDGE + OUTLINE_MARGIN || workArea.width < 280) {
            throw new Error('共享显示器的可用区域太小');
        }
        const session = {owner, workArea, selector: new RegionSelector(monitor, workArea)};
        this._session = session;
        try {
            this._pinManager.setSystemHidden(true);
            session.watch = Gio.bus_watch_name_on_connection(connection, owner, Gio.BusNameWatcherFlags.NONE,
                null, () => this._close(session));
            session.monitorsChanged = Main.layoutManager.connect('monitors-changed', () => this._cancel(session));
            session.modeChanged = Main.sessionMode.connect('updated', () => {
                if (Main.sessionMode.isLocked) {
                    this._cancel(session);
                }
            });
            const rect = await session.selector.select();
            if (this._session !== session || rect[2] === 0) {
                this._close(session);
                return CANCELLED;
            }
            session.selector = null;
            session.outline = showRecordingOutline(monitor, rect);
            session.outline.connect('destroy', () => {
                session.outline = null;
            });
            this._showHud(session);
            return rect;
        } catch (error) {
            this._close(session);
            throw error;
        }
    }

    _showHud(session) {
        const {x, y, width, height} = session.workArea;
        const hudWidth = Math.min(width - 24, 440);
        const hud = new St.BoxLayout({name: 'pinshot-longshot-hud', style_class: 'pinshot-longshot-hud'});
        hud.set_position(x + Math.round((width - hudWidth) / 2), y + height - HUD_SPACE + 8);
        hud.set_size(hudWidth, 48);
        session.hud = hud;
        hud.connect('destroy', () => {
            session.hud = null;
        });
        session.status = new St.Label({text: '正在准备，暂不要滚动…', x_expand: true,
            y_align: Clutter.ActorAlign.CENTER});
        session.status.clutter_text.ellipsize = Pango.EllipsizeMode.END;
        session.finish = this._addButton(hud, '完成', () => this._act(session, 'finish'));
        session.finish.reactive = false;
        session.finish.can_focus = false;
        session.cancel = this._addButton(hud, '取消', () => this._act(session, 'cancel'));
        hud.insert_child_at_index(session.status, 0);
        Main.layoutManager.uiGroup.add_child(hud);
    }

    _addButton(hud, label, clicked) {
        const button = new St.Button({label, style_class: 'pinshot-longshot-button button',
            reactive: true, can_focus: true, track_hover: true});
        button.connect('clicked', clicked);
        hud.add_child(button);
        return button;
    }

    update(owner, height, warning) {
        const session = this._session;
        if (session?.owner !== owner || !session.hud) {
            return false;
        }
        if (!session.ending && height > 0) {
            session.status.text = warning ? `${height} px · 请回滚后慢慢向下滚` : `${height} px · 可以滚动`;
            session.finish.reactive = true;
            session.finish.can_focus = true;
        }
        return true;
    }

    _act(session, action) {
        if (this._session !== session || session.ending) {
            return;
        }
        session.ending = true;
        session.finish.reactive = false;
        session.cancel.reactive = false;
        session.finish.can_focus = false;
        session.cancel.can_focus = false;
        session.status.text = action === 'finish' ? '正在结束录制…' : '正在取消…';
        this._emitAction(session.owner, action);
    }

    _cancel(session) {
        if (this._session !== session) {
            return;
        }
        try {
            if (!session.selector) {
                this._emitAction(session.owner, 'cancel');
            }
        } finally {
            this._close(session);
        }
    }

    _close(session) {
        if (this._session !== session || !session) {
            return;
        }
        this._session = null;
        session.selector?.close();
        session.outline?.destroy();
        session.hud?.destroy();
        if (session.watch) {
            Gio.bus_unwatch_name(session.watch);
        }
        if (session.monitorsChanged) {
            Main.layoutManager.disconnect(session.monitorsChanged);
        }
        if (session.modeChanged) {
            Main.sessionMode.disconnect(session.modeChanged);
        }
        this._pinManager.setSystemHidden(false);
    }

    destroy() {
        if (this._session) {
            this._cancel(this._session);
        }
    }
}
