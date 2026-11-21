import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Shell from 'gi://Shell';
import St from 'gi://St';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';

// GNOME Shell 51 的 UIMode.SCREENSHOT_ONLY 未导出，保留原生截图界面但不提供录屏切换
const SCREENSHOT_ONLY = 2;
Gio._promisify(Gio.OutputStream.prototype, 'splice_async');

/** 一次性接管 Pinshot 发起的原生截图确认，PNG 直接写入调用方的临时流 */
export class PinshotScreenshot {
    constructor() {
        this._session = null;
    }

    async capture(stream) {
        const ui = Main.screenshotUI;
        if (this._session || ui.visible || ui._openingCoroutineInProgress || ui.screencast_in_progress) {
            throw new Error('GNOME 截图界面正忙，请先完成当前操作');
        }

        return new Promise((resolve, reject) => {
            const previousSave = Object.getOwnPropertyDescriptor(ui, '_saveScreenshot');
            const originalSave = ui._saveScreenshot;
            const session = {ui, cancellable: new Gio.Cancellable(), encoding: null};
            this._session = session;
            let closedId;
            const save = async () => {
                if (ui._currentMode !== SCREENSHOT_ONLY) {
                    return originalSave.call(ui);
                }
                session.encoding ??= this._writeSelection(ui, stream, session.cancellable)
                    .then(scale => ({scale}), error => ({error}));
                await session.encoding;
            };
            const finish = async openError => {
                if (!closedId) {
                    return;
                }
                ui.disconnect(closedId);
                closedId = 0;
                if (ui._saveScreenshot === save) {
                    if (previousSave) {
                        Object.defineProperty(ui, '_saveScreenshot', previousSave);
                    } else {
                        delete ui._saveScreenshot;
                    }
                }
                const result = await session.encoding;
                this._session = null;
                if (session.cancellable.is_cancelled()) {
                    resolve(0);
                } else if (openError || result?.error) {
                    reject(openError ?? result.error);
                } else {
                    resolve(result?.scale ?? 0);
                }
            };

            closedId = ui.connect('closed', () => finish());
            ui._saveScreenshot = save;
            ui.open(SCREENSHOT_ONLY).then(() => {
                if (!ui.visible) {
                    return finish();
                }
                if (session.cancellable.is_cancelled()) {
                    ui.close();
                }
                return undefined;
            }).catch(finish);
        });
    }

    /** 沿用 GNOME 51 对选区、屏幕和窗口的纹理／光标坐标换算，不调用系统的保存流程 */
    async _writeSelection(ui, stream, cancellable) {
        let content;
        let scale;
        let geometry = [0, 0, -1, -1];
        let cursorTexture;
        let cursorX;
        let cursorY;
        if (ui._selectionButton.checked || ui._screenButton.checked) {
            content = ui._stageScreenshot.get_content();
            scale = ui._scale;
            geometry = ui._getSelectedGeometry(true);
            cursorTexture = ui._cursor.content?.get_texture();
            cursorX = ui._cursor.x * scale;
            cursorY = ui._cursor.y * scale;
        } else {
            const window = ui._windowSelectors.flatMap(selector => selector.windows())
                .find(candidate => candidate.checked);
            if (!window) {
                throw new Error('没有选中的窗口');
            }
            content = window.windowContent;
            scale = window.bufferScale;
            cursorTexture = window.getCursorTexture()?.get_texture();
            cursorX = window.cursorPoint.x * scale;
            cursorY = window.cursorPoint.y * scale;
        }
        if (!content) {
            throw new Error('GNOME 未返回截图纹理');
        }

        const memory = Gio.MemoryOutputStream.new_resizable();
        try {
            await Shell.Screenshot.composite_to_stream(content.get_texture(), ...geometry, scale,
                ui._cursor.visible ? cursorTexture ?? null : null, cursorX, cursorY, ui._cursorScale, memory);
        } finally {
            memory.close(null);
        }
        const bytes = memory.steal_as_bytes();
        const input = Gio.MemoryInputStream.new_from_bytes(bytes);
        await stream.splice_async(input, Gio.OutputStreamSpliceFlags.CLOSE_SOURCE, GLib.PRIORITY_DEFAULT, cancellable);
        if (!cancellable.is_cancelled()) {
            St.Clipboard.get_default().set_content(St.ClipboardType.CLIPBOARD, 'image/png', bytes);
        }
        return scale;
    }

    destroy() {
        if (!this._session) {
            return;
        }
        this._session.cancellable.cancel();
        if (this._session.ui.visible) {
            this._session.ui.close();
        }
    }
}
