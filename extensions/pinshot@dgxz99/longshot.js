import Clutter from 'gi://Clutter';
import Gio from 'gi://Gio';
import Pango from 'gi://Pango';
import St from 'gi://St';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';

import {CANCELLED_REGION, RegionSelector} from './regionSelector.js';

const HUD_SPACE = 68;
const MIN_EDGE = 32;
const OUTLINE_WIDTH = 2;
// 裁剪边界会在像素空间向外取整，边框与选区之间留空避免分数缩放时串色
const OUTLINE_GAP = 2;
const OUTLINE_MARGIN = OUTLINE_WIDTH + OUTLINE_GAP;

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
            return CANCELLED_REGION;
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
        const left = workArea.x - monitor.x;
        const top = workArea.y - monitor.y;
        const bottom = top + workArea.height - HUD_SPACE;
        const selector = new RegionSelector(monitor, {
            name: 'pinshot-longshot-selector',
            bounds: {left: left + OUTLINE_MARGIN, top: top + OUTLINE_MARGIN,
                right: left + workArea.width - OUTLINE_MARGIN, bottom},
            minEdge: MIN_EDGE,
            hint: {x: left + 12, y: bottom + 10, width: workArea.width - 24,
                text: '拖动框选滚动内容 · Esc 取消'},
        });
        const session = {owner, workArea, selector};
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
                return CANCELLED_REGION;
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
