import Clutter from 'gi://Clutter';
import Gio from 'gi://Gio';
import Pango from 'gi://Pango';
import St from 'gi://St';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import * as PopupMenu from 'resource:///org/gnome/shell/ui/popupMenu.js';

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
const GIF_START_ACTION = {id: 'start', label: '开始录制 GIF', icon: 'record'};
const GIF_STOP_ACTION = {id: 'save', label: '停止并保存 GIF', icon: 'stop'};

function screenBounds(monitor) {
    return {left: 0, top: 0, right: monitor.width, bottom: monitor.height};
}

function monitorFor(geometry) {
    const [x, y, width, height] = geometry;
    return Main.layoutManager.monitors.find(candidate =>
        candidate.x === x && candidate.y === y && candidate.width === width && candidate.height === height);
}

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

/** 长图与 GIF 共用框选、录制边框和工具条；发起连接退出时恢复贴图 */
export class LongshotController {
    constructor(pinManager, emitAction) {
        this._pinManager = pinManager;
        this._emitAction = emitAction;
        this._session = null;
    }

    get active() {
        return this._session !== null;
    }

    async select(owner, connection, geometry, initialRegion, mode) {
        if (this.active) {
            throw new Error('已有长截图或 GIF 录制正在运行');
        }
        if (Main.sessionMode.isLocked) {
            return CANCELLED_REGION;
        }
        const monitor = monitorFor(geometry);
        if (!monitor) {
            throw new Error('显示器布局已变化，无法定位共享屏幕，请重新开始');
        }
        const workArea = Main.layoutManager.getWorkAreaForMonitor(monitor.index);
        if (workArea.height < HUD_SPACE + MIN_EDGE + OUTLINE_MARGIN || workArea.width < 280) {
            throw new Error('共享显示器的可用区域太小');
        }
        const direction = mode === 'horizontal' ? '横向' : '纵向';
        const left = workArea.x - monitor.x;
        const top = workArea.y - monitor.y;
        const bottom = top + workArea.height - HUD_SPACE;
        let rect = null;
        let selector = null;
        if (initialRegion) {
            rect = initialRegionOnMonitor(initialRegion, monitor, screenBounds(monitor), MIN_EDGE);
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
        try {
            this._open(session, connection);
            if (selector) {
                const selection = await selector.select();
                if (this._session !== session || !selection) {
                    this._close(session);
                    return CANCELLED_REGION;
                }
                rect = selection.rect;
                session.selector = null;
            }
            this._showOutline(session, monitor, rect);
            this._showHud(session, [monitor.x + rect[0], monitor.y + rect[1], rect[2], rect[3]]);
            return rect;
        } catch (error) {
            this._close(session);
            throw error;
        }
    }

    /** 选区和帧率均确定后才允许 Core 打开共享请求 */
    async prepareGif(owner, connection, stageRegion, frameRates, defaultRate) {
        if (this.active) {
            throw new Error('已有长截图或 GIF 录制正在运行');
        }
        if (Main.sessionMode.isLocked) {
            return 0;
        }
        const session = {owner, gif: true, frameRates, frameRate: defaultRate};
        try {
            this._open(session, connection);
            if (stageRegion.every(value => value === 0)) {
                const [width, height] = global.stage.get_size();
                session.selector = new RegionSelector({x: 0, y: 0, width, height}, {
                    name: 'pinshot-longshot-selector', minEdge: MIN_EDGE,
                    actions: [CANCEL_ACTION, {id: 'confirm', label: '确认 GIF 选区', icon: 'copy'}], defaultAction: 'confirm',
                });
                const selection = await session.selector.select();
                if (this._session !== session || !selection) {
                    this._close(session);
                    return 0;
                }
                session.selector = null;
                stageRegion = selection.rect;
            }
            const monitor = Main.layoutManager.monitors.find(candidate =>
                initialRegionOnMonitor(stageRegion, candidate, screenBounds(candidate), MIN_EDGE));
            if (!monitor) {
                throw new Error('GIF 选区必须完整位于同一显示器内，且至少为 32×32');
            }
            session.stageRegion = stageRegion;
            session.workArea = Main.layoutManager.getWorkAreaForMonitor(monitor.index);
            session.phase = 'ready';
            this._showOutline(session, monitor, initialRegionOnMonitor(stageRegion, monitor, screenBounds(monitor), MIN_EDGE));
            this._showHud(session, stageRegion);
            return await new Promise(resolve => {
                session.confirmGif = resolve;
            });
        } catch (error) {
            this._close(session);
            throw error;
        }
    }

    /** 授权后复核原选区，不允许 Portal 换屏时截到其他内容 */
    beginGif(owner, geometry) {
        const session = this._session;
        if (session?.owner !== owner || session.phase !== 'authorizing') {
            return CANCELLED_REGION;
        }
        const monitor = monitorFor(geometry);
        const rect = monitor && initialRegionOnMonitor(session.stageRegion, monitor, screenBounds(monitor), MIN_EDGE);
        if (!rect) {
            throw new Error('GIF 选区不在已授权的屏幕内；请用 pinshot gif --choose-screen 重新选择');
        }
        session.phase = 'recording';
        session.status.text = '正在准备…';
        session.startButton.hide();
        session.stopButton.show();
        this._setButtonEnabled(session.cancelButton, true);
        for (const edge of session.outline.get_children()) {
            edge.set_style_class_name('pinshot-gif-recording-edge');
        }
        session.outline.show();
        session.hud.show();
        return rect;
    }

    _open(session, connection) {
        this._session = session;
        this._pinManager.setSystemHidden(true);
        session.watch = Gio.bus_watch_name_on_connection(connection, session.owner, Gio.BusNameWatcherFlags.NONE,
            null, () => this._close(session));
        session.monitorsChanged = Main.layoutManager.connect('monitors-changed', () => this._cancel(session));
        session.modeChanged = Main.sessionMode.connect('updated', () => {
            if (Main.sessionMode.isLocked) {
                this._cancel(session);
            }
        });
    }

    _showOutline(session, monitor, rect) {
        session.outline = showRecordingOutline(monitor, rect);
        session.outline.connect('destroy', () => {
            session.outline = null;
        });
    }

    _showHud(session, absoluteRect) {
        const hud = new St.BoxLayout({name: 'pinshot-longshot-hud', style_class: 'pinshot-longshot-hud'});
        session.hud = hud;
        hud.connect('destroy', () => {
            session.hud = null;
        });
        session.status = new St.Label({text: session.gif ? '00:00' : '正在准备…', style_class: 'pinshot-longshot-status',
            width: session.gif ? 96 : 132, y_align: Clutter.ActorAlign.CENTER});
        session.status.clutter_text.ellipsize = Pango.EllipsizeMode.END;
        hud.add_child(session.status);
        if (session.gif) {
            this._populateGifHud(session);
        } else {
            session.buttons = OUTPUT_ACTIONS.map(action => {
                const button = createActionButton('pinshot-longshot-action', action, () => this._act(session, action.id));
                this._setButtonEnabled(button, action.id === 'cancel');
                hud.add_child(button);
                return button;
            });
        }
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

    _populateGifHud(session) {
        const hud = session.hud;
        const frameRate = new St.Button({name: 'pinshot-gif-frame-rate', label: `${session.frameRate} FPS ▾`,
            style_class: 'pinshot-action-button pinshot-gif-rate', can_focus: true, reactive: true});
        frameRate.set_accessible_name('选择 GIF 帧率');
        hud.add_child(frameRate);
        session.frameRateButton = frameRate;
        session.frameRateMenu = new PopupMenu.PopupMenu(frameRate, 0.5, St.Side.TOP);
        session.frameRateMenu.actor.name = 'pinshot-gif-frame-rate-menu';
        Main.layoutManager.uiGroup.add_child(session.frameRateMenu.actor);
        session.frameRateMenu.actor.hide();
        session.menuManager = new PopupMenu.PopupMenuManager(frameRate);
        session.menuManager.addMenu(session.frameRateMenu);
        const items = session.frameRates.map(rate => session.frameRateMenu.addAction(`${rate} FPS`, () => {
            session.frameRate = rate;
            frameRate.label = `${rate} FPS ▾`;
            for (const [index, item] of items.entries()) {
                const ornament = session.frameRates[index] === rate ? PopupMenu.Ornament.CHECK : PopupMenu.Ornament.NONE;
                item.setOrnament(ornament);
            }
        }));
        items[session.frameRates.indexOf(session.frameRate)].setOrnament(PopupMenu.Ornament.CHECK);
        frameRate.connect('clicked', () => session.frameRateMenu.toggle());
        session.startButton = createActionButton('pinshot-longshot-action', GIF_START_ACTION, () => this._startGif(session));
        session.startButton.add_style_class_name('pinshot-gif-record-button');
        this._setButtonEnabled(session.startButton, true);
        session.stopButton = createActionButton('pinshot-longshot-action', GIF_STOP_ACTION, () => this._act(session, 'save'));
        this._setButtonEnabled(session.stopButton, false);
        session.stopButton.hide();
        session.cancelButton = createActionButton('pinshot-longshot-action', CANCEL_ACTION, () => {
            if (session.phase === 'ready') {
                this._close(session);
            } else {
                this._act(session, 'cancel');
            }
        });
        this._setButtonEnabled(session.cancelButton, true);
        hud.add_child(session.startButton);
        hud.add_child(session.stopButton);
        hud.add_child(session.cancelButton);
        session.buttons = [session.stopButton, session.cancelButton];
    }

    _startGif(session) {
        if (this._session !== session || session.phase !== 'ready' || !session.confirmGif) {
            return;
        }
        session.phase = 'authorizing';
        session.frameRateMenu.close(false);
        session.frameRateButton.label = `${session.frameRate} FPS`;
        this._setButtonEnabled(session.frameRateButton, false);
        session.outline.hide();
        session.hud.hide();
        const confirm = session.confirmGif;
        session.confirmGif = null;
        confirm(session.frameRate);
    }

    _setButtonEnabled(button, enabled) {
        button.reactive = enabled;
        button.can_focus = enabled;
        button.opacity = enabled ? 255 : 90;
    }

    update(owner, length, warning, gif = false) {
        const session = this._session;
        if (session?.owner !== owner || !session.hud || Boolean(session.gif) !== gif ||
            (gif && session.phase !== 'recording')) {
            return false;
        }
        if (!session.ending && (gif || length > 0)) {
            if (gif) {
                const minutes = String(Math.floor(length / 60)).padStart(2, '0');
                const seconds = String(length % 60).padStart(2, '0');
                const duration = `${minutes}:${seconds}`;
                session.status.text = warning ? `${duration} · 快满了` : duration;
            } else {
                session.status.text = warning ? `${length} px · 回滚后慢滚` : `${length} px · ${session.direction}滚动`;
            }
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
            if (!session.selector && (!session.gif || session.phase === 'recording')) {
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
        session.confirmGif?.(0);
        session.selector?.close();
        if (session.frameRateMenu) {
            session.menuManager.removeMenu(session.frameRateMenu);
            session.frameRateMenu.destroy();
        }
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
