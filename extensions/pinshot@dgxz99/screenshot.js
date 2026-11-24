import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Shell from 'gi://Shell';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';

import {RegionSelector} from './regionSelector.js';
import {parseSelectionShortcut, SELECTION_ACTIONS} from './selectionShortcuts.js';

Gio._promisify(Gio.OutputStream.prototype, 'splice_async');

const LEGACY_ACTIONS = [{id: 'confirm', label: '确认'}];
const CANCELLED = [0, ''];

/** 避免在系统截图或录屏已经打开时叠加 Pinshot 框选 */
export function nativeScreenshotActive() {
    const ui = Main.screenshotUI;
    return ui.visible || ui._openingCoroutineInProgress || ui.screencast_in_progress;
}

/** 用单张冻结画面框选并写入调用方的临时流，不接管 GNOME 原生截图 */
export class PinshotScreenshot {
    constructor(settings) {
        this._settings = settings;
        this._session = null;
    }

    get active() {
        return this._session !== null;
    }

    async capture(stream, owner, connection, defaultAction) {
        const actions = defaultAction === null ? LEGACY_ACTIONS : SELECTION_ACTIONS.map(({id, key, label}) => ({
            id, label, shortcut: parseSelectionShortcut(this._settings.get_strv(key)[0] ?? ''),
        }));
        const initialAction = defaultAction ?? 'confirm';
        if (!actions.some(({id}) => id === initialAction)) {
            throw new Error('无效的截图默认动作');
        }
        if (this.active || nativeScreenshotActive()) {
            throw new Error('请先完成或取消当前截图');
        }
        if (Main.sessionMode.isLocked) {
            return CANCELLED;
        }
        const session = {selector: null, cancellable: new Gio.Cancellable()};
        this._session = session;
        try {
            session.watch = Gio.bus_watch_name_on_connection(connection, owner, Gio.BusNameWatcherFlags.NONE,
                null, () => this._cancel(session));
            session.monitorsChanged = Main.layoutManager.connect('monitors-changed', () => this._cancel(session));
            session.modeChanged = Main.sessionMode.connect('updated', () => {
                if (Main.sessionMode.isLocked) {
                    this._cancel(session);
                }
            });

            const [width, height] = global.stage.get_size();
            const [content, scale] = await new Shell.Screenshot().screenshot_stage_to_content();
            if (session.cancellable.is_cancelled() || nativeScreenshotActive()) {
                return CANCELLED;
            }
            session.selector = new RegionSelector({x: 0, y: 0, width, height}, {
                name: 'pinshot-screenshot-selector', background: content, actions, defaultAction: initialAction,
            });
            const selection = await session.selector.select();
            session.selector = null;
            if (!selection || session.cancellable.is_cancelled()) {
                return CANCELLED;
            }
            const {rect, action} = selection;
            await this._writeSelection(content.get_texture(), scale, rect, stream, session.cancellable);
            return session.cancellable.is_cancelled() ? CANCELLED : [scale, action];
        } catch (error) {
            if (session.cancellable.is_cancelled()) {
                return CANCELLED;
            }
            throw error;
        } finally {
            this._cancel(session);
            if (session.watch) {
                Gio.bus_unwatch_name(session.watch);
            }
            if (session.monitorsChanged) {
                Main.layoutManager.disconnect(session.monitorsChanged);
            }
            if (session.modeChanged) {
                Main.sessionMode.disconnect(session.modeChanged);
            }
            this._session = null;
        }
    }

    /** Surface 与 stage 共用原点，裁剪边界向外取整并限制在纹理内 */
    async _writeSelection(texture, scale, [x, y, width, height], stream, cancellable) {
        const left = Math.floor(x * scale);
        const top = Math.floor(y * scale);
        const right = Math.min(texture.get_width(), Math.ceil((x + width) * scale));
        const bottom = Math.min(texture.get_height(), Math.ceil((y + height) * scale));
        if (right <= left || bottom <= top) {
            throw new Error('截图选区超出冻结画面');
        }
        const memory = Gio.MemoryOutputStream.new_resizable();
        try {
            await Shell.Screenshot.composite_to_stream(texture, left, top, right - left, bottom - top,
                scale, null, 0, 0, 1, memory);
        } finally {
            memory.close(null);
        }
        const input = Gio.MemoryInputStream.new_from_bytes(memory.steal_as_bytes());
        await stream.splice_async(input, Gio.OutputStreamSpliceFlags.CLOSE_SOURCE, GLib.PRIORITY_DEFAULT, cancellable);
    }

    _cancel(session) {
        session.cancellable.cancel();
        session.selector?.close();
    }

    destroy() {
        if (this._session) {
            this._cancel(this._session);
        }
    }
}
