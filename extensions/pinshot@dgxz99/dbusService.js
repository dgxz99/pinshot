import Gio from 'gi://Gio';
import GioUnix from 'gi://GioUnix';
import GLib from 'gi://GLib';

import {LongshotController} from './longshot.js';
import {PinshotScreenshot, nativeScreenshotActive} from './screenshot.js';

export const FRONTEND_BUS_NAME = 'io.github.dgxz99.Pinshot.Frontend';
export const FRONTEND_OBJECT_PATH = '/io/github/dgxz99/Pinshot/Frontend';
export const FRONTEND_INTERFACE_NAME = 'io.github.dgxz99.Pinshot.Frontend1';
export const FRONTEND_ID = 'gnome-shell-51';

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
    <method name="UpdateLongshot">
      <arg type="u" direction="in" name="height"/>
      <arg type="b" direction="in" name="warning"/>
      <arg type="b" direction="out" name="ok"/>
    </method>
    <signal name="LongshotAction">
      <arg type="s" name="action"/>
    </signal>
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
        this._screenshot = new PinshotScreenshot();
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

    GetScreenshotSettings() {
        return [this._settings.get_boolean('screenshot-auto-save'),
            this._settings.get_string('screenshot-save-dir'),
            this._settings.get_string('screenshot-filename'),
            this._settings.get_boolean('screenshot-notify')];
    }

    async SelectLongshotAsync(geometry, invocation) {
        try {
            if (this._screenshot.active || nativeScreenshotActive()) {
                throw new Error('请先完成或取消当前截图或录屏');
            }
            const rect = await this._longshot.select(invocation.get_sender(), invocation.get_connection(), geometry);
            invocation.return_value(new GLib.Variant('(uuuu)', rect));
        } catch (error) {
            invocation.return_dbus_error(`${FRONTEND_INTERFACE_NAME}.LongshotFailed`, error.message);
        }
    }

    UpdateLongshotAsync([height, warning], invocation) {
        const updated = this._longshot.update(invocation.get_sender(), height, warning);
        invocation.return_value(new GLib.Variant('(b)', [updated]));
    }

    async CaptureScreenshotAsync([handle], invocation) {
        try {
            if (this._longshot.active) {
                throw new Error('请先完成或取消长截图');
            }
            const fds = invocation.get_message().get_unix_fd_list();
            if (!fds || handle < 0 || handle >= fds.get_length()) {
                throw new Error('截图请求缺少有效的输出文件描述符');
            }
            const stream = GioUnix.OutputStream.new(fds.get(handle), true);
            let scale;
            try {
                scale = await this._screenshot.capture(stream, invocation.get_sender(), invocation.get_connection());
            } finally {
                stream.close(null);
            }
            invocation.return_value(new GLib.Variant('(d)', [scale]));
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
