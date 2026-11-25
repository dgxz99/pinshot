import Clutter from 'gi://Clutter';
import Gio from 'gi://Gio';
import Pango from 'gi://Pango';
import St from 'gi://St';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';

import {CANCEL_ACTION, createActionButton} from './actionIcons.js';
import {CANCELLED_REGION, RegionSelector} from './regionSelector.js';
import {initialRegionOnMonitor, toolbarPosition} from './selectionGeometry.js';
import {OUTPUT_ACTIONS} from './selectionShortcuts.js';

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

    async select(owner, connection, geometry, initialRegion, axis) {
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
        const direction = axis === 'horizontal' ? '横向' : '纵向';
        const left = workArea.x - monitor.x;
        const top = workArea.y - monitor.y;
        const bottom = top + workArea.height - HUD_SPACE;
        let rect = null;
        let selector = null;
        if (initialRegion) {
            const screenBounds = {left: 0, top: 0, right: monitor.width, bottom: monitor.height};
            rect = initialRegionOnMonitor(initialRegion, monitor, screenBounds, MIN_EDGE);
            if (!rect) {
                throw new Error('选区必须完整位于已授权的屏幕内，且至少为 32×32；换屏时请重新选择共享屏幕');
            }
        } else {
            const bounds = {left: left + OUTLINE_MARGIN, top: top + OUTLINE_MARGIN,
                right: left + workArea.width - OUTLINE_MARGIN, bottom};
            selector = new RegionSelector(monitor, {
                name: 'pinshot-longshot-selector', bounds, minEdge: MIN_EDGE,
                actions: [CANCEL_ACTION, {id: 'start', label: '开始录制'}], defaultAction: 'start',
                hint: {x: left + 12, y: bottom + 10, width: workArea.width - 24, text: `拖动框选${direction}滚动的内容`},
            });
        }
        const session = {owner, workArea, selector, direction};
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
            if (selector) {
                const selection = await selector.select();
                if (this._session !== session || !selection) {
                    this._close(session);
                    return CANCELLED_REGION;
                }
                rect = selection.rect;
                session.selector = null;
            }
            session.outline = showRecordingOutline(monitor, rect);
            session.outline.connect('destroy', () => {
                session.outline = null;
            });
            this._showHud(session, [monitor.x + rect[0], monitor.y + rect[1], rect[2], rect[3]]);
            return rect;
        } catch (error) {
            this._close(session);
            throw error;
        }
    }

    _showHud(session, absoluteRect) {
        const hud = new St.BoxLayout({name: 'pinshot-longshot-hud', style_class: 'pinshot-longshot-hud'});
        session.hud = hud;
        hud.connect('destroy', () => {
            session.hud = null;
        });
        session.status = new St.Label({text: '正在准备…', style_class: 'pinshot-longshot-status',
            width: 132, y_align: Clutter.ActorAlign.CENTER});
        session.status.clutter_text.ellipsize = Pango.EllipsizeMode.END;
        hud.add_child(session.status);
        session.buttons = OUTPUT_ACTIONS.map(action => {
            const button = createActionButton('pinshot-longshot-action', action, () => this._act(session, action.id));
            this._setButtonEnabled(button, action.id === 'cancel');
            hud.add_child(button);
            return button;
        });
        Main.layoutManager.uiGroup.add_child(hud);
        const [, width] = hud.get_preferred_width(-1);
        const [, height] = hud.get_preferred_height(width);
        const position = toolbarPosition(absoluteRect, [width, height], session.workArea, {allowOverlap: false});
        if (!position) {
            throw new Error('选区外没有足够空间放录制工具条，请框小一点再试');
        }
        hud.set_position(...position);
        hud.set_size(width, height);
    }

    _setButtonEnabled(button, enabled) {
        button.reactive = enabled;
        button.can_focus = enabled;
        button.opacity = enabled ? 255 : 90;
    }

    update(owner, length, warning) {
        const session = this._session;
        if (session?.owner !== owner || !session.hud) {
            return false;
        }
        if (!session.ending && length > 0) {
            session.status.text = warning ? `${length} px · 回滚后慢滚` : `${length} px · ${session.direction}滚动`;
            for (const button of session.buttons) {
                this._setButtonEnabled(button, true);
            }
        }
        return true;
    }

    _act(session, action) {
        if (this._session !== session || session.ending) {
            return;
        }
        session.ending = true;
        for (const button of session.buttons) {
            this._setButtonEnabled(button, false);
        }
        session.status.text = action === 'cancel' ? '正在取消…' : '正在结束录制…';
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
