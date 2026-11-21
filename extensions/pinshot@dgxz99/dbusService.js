import Gio from 'gi://Gio';
import GioUnix from 'gi://GioUnix';
import GLib from 'gi://GLib';

import {PinshotScreenshot} from './screenshot.js';

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
    <method name="GetScreenshotSettings">
      <arg type="b" direction="out" name="auto_save"/>
      <arg type="s" direction="out" name="save_dir"/>
      <arg type="s" direction="out" name="filename_template"/>
      <arg type="b" direction="out" name="notify"/>
    </method>
    <method name="SelectRegion">
      <arg type="s" direction="out" name="region_json"/>
    </method>
    <method name="UpdateLongshotHud">
      <arg type="s" direction="in" name="state_json"/>
      <arg type="b" direction="out" name="ok"/>
    </method>
    <signal name="LongshotAction">
      <arg type="s" name="action"/>
    </signal>
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
                console.warn(`[pinshot] D-Bus 服务名丢失: ${FRONTEND_BUS_NAME}`);
            }
        );
    }

    /**
     * 向正在运行的长图进程广播用户操作（finish / cancel）
     */
    emitLongshotAction(action) {
        if (!this._dbusImpl) {
            return;
        }
        this._dbusImpl.emit_signal('LongshotAction', new GLib.Variant('(s)', [String(action)]));
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

    async CaptureScreenshotAsync([handle], invocation) {
        try {
            const fds = invocation.get_message().get_unix_fd_list();
            if (!fds || handle < 0 || handle >= fds.get_length()) {
                throw new Error('截图请求缺少有效的输出文件描述符');
            }
            const stream = GioUnix.OutputStream.new(fds.get(handle), true);
            let scale;
            try {
                scale = await this._screenshot.capture(stream);
            } finally {
                stream.close(null);
            }
            invocation.return_value(new GLib.Variant('(d)', [scale]));
        } catch (error) {
            invocation.return_dbus_error(`${FRONTEND_INTERFACE_NAME}.CaptureFailed`, error.message);
        }
    }

    SelectRegionAsync(_params, invocation) {
        Promise.resolve(this._pinManager.selectRegion())
            .then(result => {
                invocation.return_value(new GLib.Variant('(s)', [JSON.stringify(result)]));
            })
            .catch(err => {
                console.error(`[pinshot] SelectRegion 执行失败: ${err}`);
                const fallback = {
                    cancelled: true,
                    monitor_index: 0,
                    x: 0,
                    y: 0,
                    width: 0,
                    height: 0,
                    scale: 1.0,
                };
                invocation.return_value(new GLib.Variant('(s)', [JSON.stringify(fallback)]));
            });
    }

    UpdateLongshotHud(stateJson) {
        try {
            const state = JSON.parse(stateJson);
            return Boolean(this._pinManager.updateLongshotHud(state));
        } catch (err) {
            console.error(`[pinshot] UpdateLongshotHud 解析或执行失败: ${err}`);
            return false;
        }
    }

    /**
     * 注销 D-Bus 对象并释放总线服务名
     */
    destroy() {
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
