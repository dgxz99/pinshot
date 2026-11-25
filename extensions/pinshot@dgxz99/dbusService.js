import Gio from 'gi://Gio';
import GioUnix from 'gi://GioUnix';
import GLib from 'gi://GLib';
import St from 'gi://St';

import {LongshotController} from './longshot.js';
import {PinshotScreenshot, nativeScreenshotActive} from './screenshot.js';

export const FRONTEND_BUS_NAME = 'io.github.dgxz99.Pinshot.Frontend';
export const FRONTEND_OBJECT_PATH = '/io/github/dgxz99/Pinshot/Frontend';
export const FRONTEND_INTERFACE_NAME = 'io.github.dgxz99.Pinshot.Frontend1';
export const FRONTEND_ID = 'gnome-shell-51';

Gio._promisify(Gio.InputStream.prototype, 'read_bytes_async');

// 覆盖长图编码上限并限制 D-Bus 文件描述符读入的内存量
const MAX_CLIPBOARD_PNG_BYTES = 224 * 1024 * 1024;
const PNG_SIGNATURE = new GLib.Bytes([137, 80, 78, 71, 13, 10, 26, 10]);

const FRONTEND_IFACE_XML = `
<node>
  <interface name="${FRONTEND_INTERFACE_NAME}">
    <method name="Ping">
      <arg type="s" direction="out" name="frontend_id"/>
    </method>
    <method name="CreatePin">
      <arg type="s" direction="in" name="payload_json"/>
      <arg type="b" direction="out" name="ok"/>
    </method>
    <method name="UpdatePin">
      <arg type="s" direction="in" name="id"/>
      <arg type="s" direction="in" name="payload_json"/>
      <arg type="b" direction="out" name="ok"/>
    </method>
    <method name="SetPinTemporaryHidden">
      <arg type="s" direction="in" name="id"/>
      <arg type="b" direction="in" name="hidden"/>
      <arg type="b" direction="out" name="ok"/>
    </method>
    <method name="ClosePin">
      <arg type="s" direction="in" name="id"/>
      <arg type="b" direction="out" name="ok"/>
    </method>
    <method name="ToggleHideAll">
      <arg type="b" direction="out" name="hidden"/>
    </method>
    <method name="SetSystemHidden">
      <arg type="b" direction="in" name="hidden"/>
      <arg type="b" direction="out" name="ok"/>
    </method>
    <method name="UnghostAll">
      <arg type="u" direction="out" name="count"/>
    </method>
    <method name="ListActivePins">
      <arg type="s" direction="out" name="pins_json"/>
    </method>
    <method name="CaptureScreenshot">
      <arg type="h" direction="in" name="output_fd"/>
      <arg type="d" direction="out" name="scale"/>
    </method>
    <method name="SelectScreenshot">
      <arg type="h" direction="in" name="output_fd"/>
      <arg type="s" direction="in" name="default_action"/>
      <arg type="d" direction="out" name="scale"/>
      <arg type="s" direction="out" name="action"/>
    </method>
    <method name="ChooseCapture">
      <arg type="h" direction="in" name="output_fd"/>
      <arg type="s" direction="in" name="default_action"/>
      <arg type="as" direction="in" name="offered_actions"/>
      <arg type="d" direction="out" name="scale"/>
      <arg type="s" direction="out" name="action"/>
      <arg type="(iiii)" direction="out" name="stage_region"/>
    </method>
    <method name="ChooseScreenshot">
      <arg type="h" direction="in" name="output_fd"/>
      <arg type="s" direction="in" name="default_action"/>
      <arg type="b" direction="in" name="offer_longshot"/>
      <arg type="d" direction="out" name="scale"/>
      <arg type="s" direction="out" name="action"/>
      <arg type="(iiii)" direction="out" name="stage_region"/>
    </method>
    <method name="SelectLongshot">
      <arg type="i" direction="in" name="monitor_x"/>
      <arg type="i" direction="in" name="monitor_y"/>
      <arg type="i" direction="in" name="monitor_width"/>
      <arg type="i" direction="in" name="monitor_height"/>
      <arg type="u" direction="out" name="x"/>
      <arg type="u" direction="out" name="y"/>
      <arg type="u" direction="out" name="width"/>
      <arg type="u" direction="out" name="height"/>
    </method>
    <method name="SelectLongshotRegion">
      <arg type="i" direction="in" name="monitor_x"/>
      <arg type="i" direction="in" name="monitor_y"/>
      <arg type="i" direction="in" name="monitor_width"/>
      <arg type="i" direction="in" name="monitor_height"/>
      <arg type="(iiii)" direction="in" name="stage_region"/>
      <arg type="u" direction="out" name="x"/>
      <arg type="u" direction="out" name="y"/>
      <arg type="u" direction="out" name="width"/>
      <arg type="u" direction="out" name="height"/>
    </method>
    <method name="SelectLongshotWithAxis">
      <arg type="i" direction="in" name="monitor_x"/>
      <arg type="i" direction="in" name="monitor_y"/>
      <arg type="i" direction="in" name="monitor_width"/>
      <arg type="i" direction="in" name="monitor_height"/>
      <arg type="(iiii)" direction="in" name="stage_region"/>
      <arg type="s" direction="in" name="axis"/>
      <arg type="u" direction="out" name="x"/>
      <arg type="u" direction="out" name="y"/>
      <arg type="u" direction="out" name="width"/>
      <arg type="u" direction="out" name="height"/>
    </method>
    <method name="UpdateLongshot">
      <arg type="u" direction="in" name="length"/>
      <arg type="b" direction="in" name="warning"/>
      <arg type="b" direction="out" name="ok"/>
    </method>
    <signal name="LongshotAction">
      <arg type="s" name="action"/>
    </signal>
    <method name="SetClipboardPng">
      <arg type="h" direction="in" name="input_fd"/>
      <arg type="u" direction="in" name="length"/>
    </method>
    <method name="GetClipboardImageScale">
      <arg type="d" direction="out" name="scale"/>
    </method>
    <method name="GetScreenshotSettings">
      <arg type="b" direction="out" name="auto_save"/>
      <arg type="s" direction="out" name="save_dir"/>
      <arg type="s" direction="out" name="filename_prefix"/>
      <arg type="b" direction="out" name="notify"/>
    </method>
  </interface>
</node>
`;

/**
 * 统一前端 D-Bus 服务实现（io.github.dgxz99.Pinshot.Frontend1）
 */
export class PinshotDbusService {
    constructor(pinManager, settings) {
        this._pinManager = pinManager;
        this._settings = settings;
        this._screenshot = new PinshotScreenshot(settings);
        this._longshot = new LongshotController(pinManager, (owner, action) => {
            Gio.DBus.session.emit_signal(owner, FRONTEND_OBJECT_PATH, FRONTEND_INTERFACE_NAME,
                'LongshotAction', new GLib.Variant('(s)', [action]));
        });
        this._dbusImpl = null;
        this._busNameId = 0;
    }

    /**
     * 在 Session Bus 注册服务名并导出 Frontend1 对象
     */
    enable() {
        this._dbusImpl = Gio.DBusExportedObject.wrapJSObject(FRONTEND_IFACE_XML, this);
        this._dbusImpl.export(Gio.DBus.session, FRONTEND_OBJECT_PATH);

        this._busNameId = Gio.bus_own_name(
            Gio.BusType.SESSION,
            FRONTEND_BUS_NAME,
            Gio.BusNameOwnerFlags.ALLOW_REPLACEMENT | Gio.BusNameOwnerFlags.REPLACE,
            null,
            () => {
                console.log(`[pinshot] D-Bus 服务已注册: ${FRONTEND_BUS_NAME}`);
            },
            () => {
                this._longshot.destroy();
                this._screenshot.destroy();
                console.warn(`[pinshot] D-Bus 服务名丢失: ${FRONTEND_BUS_NAME}`);
            }
        );
    }

    Ping() {
        return FRONTEND_ID;
    }

    CreatePin(payloadJson) {
        try {
            const payload = JSON.parse(payloadJson);
            return Boolean(this._pinManager.createPin(payload));
        } catch (err) {
            console.error(`[pinshot] CreatePin 解析或执行失败: ${err}`);
            return false;
        }
    }

    UpdatePin(id, payloadJson) {
        try {
            const payload = JSON.parse(payloadJson);
            return Boolean(this._pinManager.updatePin(id, payload));
        } catch (err) {
            console.error(`[pinshot] UpdatePin 解析或执行失败: ${err}`);
            return false;
        }
    }

    SetPinTemporaryHidden(id, hidden) {
        return Boolean(this._pinManager.setPinTemporaryHidden(id, Boolean(hidden)));
    }

    ClosePin(id) {
        return Boolean(this._pinManager.closePin(id));
    }

    ToggleHideAll() {
        return Boolean(this._pinManager.toggleHideAll());
    }

    SetSystemHidden(hidden) {
        return Boolean(this._pinManager.setSystemHidden(Boolean(hidden)));
    }

    UnghostAll() {
        return Number(this._pinManager.unghostAll()) >>> 0;
    }

    ListActivePins() {
        return JSON.stringify(this._pinManager.listActivePins());
    }

    async SetClipboardPngAsync([handle, length], invocation) {
        let input;
        try {
            const headerLength = PNG_SIGNATURE.get_size();
            if (length < headerLength || length > MAX_CLIPBOARD_PNG_BYTES) {
                throw new Error('剪贴板 PNG 长度超出范围');
            }
            const fds = invocation.get_message().get_unix_fd_list();
            if (!fds || handle < 0 || handle >= fds.get_length()) {
                throw new Error('写入剪贴板缺少有效的输入文件描述符');
            }
            input = GioUnix.InputStream.new(fds.get(handle), true);
            const bytes = await input.read_bytes_async(length, GLib.PRIORITY_DEFAULT, null);
            if (bytes.get_size() !== length || !GLib.Bytes.new_from_bytes(bytes, 0, headerLength).equal(PNG_SIGNATURE)) {
                throw new Error('剪贴板 PNG 无效或未完整传输');
            }
            St.Clipboard.get_default().set_content(St.ClipboardType.CLIPBOARD, 'image/png', bytes);
            invocation.return_value(new GLib.Variant('()', []));
        } catch (error) {
            invocation.return_dbus_error(`${FRONTEND_INTERFACE_NAME}.ClipboardFailed`, error.message);
        } finally {
            input?.close(null);
        }
    }

    GetClipboardImageScale() {
        if (!this._settings.get_boolean('clipboard-image-device-pixels')) {
            return 1.0;
        }
        const scale = global.display.get_monitor_scale(global.display.get_current_monitor());
        return scale > 0 ? scale : 1.0;
    }

    GetScreenshotSettings() {
        return [this._settings.get_boolean('screenshot-auto-save'),
            this._settings.get_string('screenshot-save-dir'),
            this._settings.get_string('screenshot-filename'),
            this._settings.get_boolean('screenshot-notify')];
    }

    async SelectLongshotAsync(geometry, invocation) {
        await this._selectLongshot(geometry, null, 'vertical', invocation);
    }

    async SelectLongshotRegionAsync([x, y, width, height, initialRegion], invocation) {
        await this._selectLongshot([x, y, width, height], initialRegion, 'vertical', invocation);
    }

    async SelectLongshotWithAxisAsync([x, y, width, height, region, axis], invocation) {
        const initialRegion = region.every(value => value === 0) ? null : region;
        await this._selectLongshot([x, y, width, height], initialRegion, axis, invocation);
    }

    async _selectLongshot(geometry, initialRegion, axis, invocation) {
        try {
            if (axis !== 'vertical' && axis !== 'horizontal') {
                throw new Error('无效的长截图拼接方向');
            }
            if (this._screenshot.active || nativeScreenshotActive()) {
                throw new Error('请先完成或取消当前截图或录屏');
            }
            const rect = await this._longshot.select(invocation.get_sender(), invocation.get_connection(),
                geometry, initialRegion, axis);
            invocation.return_value(new GLib.Variant('(uuuu)', rect));
        } catch (error) {
            invocation.return_dbus_error(`${FRONTEND_INTERFACE_NAME}.LongshotFailed`, error.message);
        }
    }

    UpdateLongshotAsync([length, warning], invocation) {
        const updated = this._longshot.update(invocation.get_sender(), length, warning);
        invocation.return_value(new GLib.Variant('(b)', [updated]));
    }

    async CaptureScreenshotAsync([handle], invocation) {
        await this._captureScreenshot(handle, null, {}, '(d)', invocation);
    }

    async SelectScreenshotAsync([handle, defaultAction], invocation) {
        await this._captureScreenshot(handle, defaultAction, {}, '(ds)', invocation);
    }

    async ChooseScreenshotAsync([handle, defaultAction, offerLongshot], invocation) {
        const offeredActions = offerLongshot ? ['longshot'] : [];
        await this.ChooseCaptureAsync([handle, defaultAction, offeredActions], invocation);
    }

    async ChooseCaptureAsync([handle, defaultAction, offeredActions], invocation) {
        const options = {offeredActions, offerSave: true};
        await this._captureScreenshot(handle, defaultAction, options, '(ds(iiii))', invocation);
    }

    async _captureScreenshot(handle, defaultAction, options, replyType, invocation) {
        try {
            if (this._longshot.active) {
                throw new Error('请先完成或取消长截图');
            }
            const fds = invocation.get_message().get_unix_fd_list();
            if (!fds || handle < 0 || handle >= fds.get_length()) {
                throw new Error('截图请求缺少有效的输出文件描述符');
            }
            const stream = GioUnix.OutputStream.new(fds.get(handle), true);
            let result;
            try {
                result = await this._screenshot.capture(stream, invocation.get_sender(), invocation.get_connection(),
                    defaultAction, options);
            } finally {
                stream.close(null);
            }
            if (replyType === '(d)') {
                result = result.slice(0, 1);
            } else if (replyType === '(ds)') {
                result = result.slice(0, 2);
            }
            invocation.return_value(new GLib.Variant(replyType, result));
        } catch (error) {
            invocation.return_dbus_error(`${FRONTEND_INTERFACE_NAME}.CaptureFailed`, error.message);
        }
    }

    /**
     * 注销 D-Bus 对象并释放总线服务名
     */
    destroy() {
        this._longshot.destroy();
        this._screenshot.destroy();
        if (this._dbusImpl) {
            try {
                this._dbusImpl.unexport();
            } catch (err) {
                console.warn(`[pinshot] 取消导出 D-Bus 对象时发生异常: ${err}`);
            }
            this._dbusImpl = null;
        }

        if (this._busNameId) {
            Gio.bus_unown_name(this._busNameId);
            this._busNameId = 0;
        }

        this._pinManager = null;
    }
}
