import Clutter from 'gi://Clutter';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';

import {firePinshot, runPinshotAsync} from './coreRunner.js';
import {PinActor} from './pinActor.js';

const DEFAULT_PIN_CONFIG = {
    default_opacity: 1.0,
    min_scale: 0.2,
    max_scale: 5.0,
    zoom_step: 0.1,
    opacity_step: 0.08,
    mini_size_px: 64,
    min_visible_edge_px: 32,
    max_texture_edge_px: 4096,
    hide_on_fullscreen: true,
};

/**
 * 贴图生命周期、顶层非窗口容器、全屏避让与多屏回收管理器
 */
export class PinManager {
    constructor() {
        this._pins = new Map();
        this._config = {...DEFAULT_PIN_CONFIG};
        this._userHidden = false;
        this._systemHidden = false;
        this._destroyed = false;

        this._container = new Clutter.Actor({
            name: 'pinshot-overlay-container',
            reactive: false,
        });
        Main.layoutManager.uiGroup.add_child(this._container);

        this._fullscreenSignalId = global.display.connect('in-fullscreen-changed', () => {
            this._syncFullscreenVisibility();
        });
        this._monitorsSignalId = Main.layoutManager.connect('monitors-changed', () => {
            this._handleMonitorsChanged();
        });

        this._selectedPin = null;
        this._selectionSignalId = global.stage.connect('captured-event',
            (_stage, event) => this._handleSelectionEvent(event));
        this._loadConfigAsync();
    }

    async _loadConfigAsync() {
        const result = await runPinshotAsync(['internal', 'get-pin-config']);
        if (this._destroyed || !result.ok || !result.stdout.trim()) {
            return;
        }
        try {
            const parsed = JSON.parse(result.stdout);
            this._config = {...DEFAULT_PIN_CONFIG, ...parsed};
            this._syncFullscreenVisibility();
        } catch (err) {
            console.warn(`[pinshot] 解析前端配置 JSON 失败，继续使用默认值: ${err}`);
        }
    }

    createPin(payload) {
        if (this._destroyed || !payload || !payload.id) {
            return false;
        }

        if (this._pins.has(payload.id)) {
            return this.updatePin(payload.id, payload);
        }

        const pin = new PinActor(payload, this._config, {
            onRequestClose: id => this.closePin(id, {notifyCore: true}),
            onRaise: actorObj => this._raisePin(actorObj),
            onSelect: actorObj => this._selectPin(actorObj),
        });

        this._pins.set(payload.id, pin);
        this._container.add_child(pin.actor);
        this._syncFullscreenVisibility();
        return true;
    }

    updatePin(id, payload) {
        if (this._destroyed || !payload) {
            return false;
        }
        const pin = this._pins.get(id);
        if (!pin) {
            return this.createPin({...payload, id});
        }
        pin.updatePayload(payload);
        this._raisePin(pin);
        return true;
    }

    setPinTemporaryHidden(id, hidden) {
        const pin = this._pins.get(id);
        if (!pin) {
            return false;
        }
        if (hidden && pin === this._selectedPin) {
            this._selectPin(null);
        }
        pin.setTemporaryHidden(hidden);
        return true;
    }

    closePin(id, options = {}) {
        const {notifyCore = false} = options;
        const pin = this._pins.get(id);
        if (!pin) {
            return false;
        }

        if (pin === this._selectedPin) {
            this._selectPin(null);
        }
        this._pins.delete(id);
        pin.destroy();

        if (notifyCore) {
            firePinshot(['internal', 'mark-closed', id]);
        }

        return true;
    }

    toggleHideAll() {
        this._userHidden = !this._userHidden;
        this._syncContainerVisibility();
        return this._userHidden;
    }

    setSystemHidden(hidden) {
        this._systemHidden = hidden;
        this._syncContainerVisibility();
        return true;
    }

    unghostAll() {
        let count = 0;
        for (const pin of this._pins.values()) {
            if (pin.ghost) {
                pin.setGhost(false);
                count += 1;
            }
        }
        return count;
    }

    listActivePins() {
        return Array.from(this._pins.values(), pin => pin.toInfo());
    }

    _selectPin(pin) {
        if (this._selectedPin === pin) {
            return;
        }
        const previous = this._selectedPin;
        previous?.setSelected(false);
        this._selectedPin = pin;

        // 只设置 Shell 内部焦点，不接管 Wayland 客户端的输入
        if (pin) {
            pin.setSelected(true);
            pin.actor.grab_key_focus();
        } else if (previous && global.stage.get_key_focus() === previous.actor) {
            global.stage.set_key_focus(null);
        }
    }

    _handleSelectionEvent(event) {
        const pin = this._selectedPin;
        if (!pin) {
            return Clutter.EVENT_PROPAGATE;
        }
        if (pin.ghost || !pin.actor.visible || !this._container.visible) {
            this._selectPin(null);
            return Clutter.EVENT_PROPAGATE;
        }

        const type = event.type();
        if (type === Clutter.EventType.BUTTON_PRESS) {
            const [x, y] = event.get_coords();
            const hit = global.stage.get_actor_at_pos(Clutter.PickMode.REACTIVE, x, y);
            if (hit !== pin.actor || event.get_button() === Clutter.BUTTON_SECONDARY) {
                this._selectPin(null);
            }
            return Clutter.EVENT_PROPAGATE;
        }

        if (type !== Clutter.EventType.KEY_PRESS || global.stage.get_key_focus() !== pin.actor) {
            return Clutter.EVENT_PROPAGATE;
        }

        const key = event.get_key_symbol();
        const modifiers = event.get_state() & (
            Clutter.ModifierType.CONTROL_MASK |
            Clutter.ModifierType.SHIFT_MASK |
            Clutter.ModifierType.MOD1_MASK |
            Clutter.ModifierType.SUPER_MASK
        );
        if (key === Clutter.KEY_c && modifiers === Clutter.ModifierType.CONTROL_MASK) {
            firePinshot(['internal', 'copy-image', pin.id]);
            return Clutter.EVENT_STOP;
        }
        if ((key === Clutter.KEY_r || key === Clutter.KEY_R) &&
            modifiers === Clutter.ModifierType.SHIFT_MASK) {
            if (!(event.get_flags() & Clutter.EventFlags.FLAG_REPEATED)) {
                pin.toggleMini();
            }
            return Clutter.EVENT_STOP;
        }
        if (modifiers !== 0) {
            return Clutter.EVENT_PROPAGATE;
        }

        switch (key) {
        case Clutter.KEY_e:
            this._selectPin(null);
            firePinshot(['internal', 'edit-satty', pin.id]);
            break;
        case Clutter.KEY_t:
            this._selectPin(null);
            pin.setGhost(true);
            break;
        case Clutter.KEY_0:
            pin.resetScaleTo100();
            break;
        case Clutter.KEY_Escape:
            this.closePin(pin.id, {notifyCore: true});
            break;
        default:
            return Clutter.EVENT_PROPAGATE;
        }
        return Clutter.EVENT_STOP;
    }

    _raisePin(pin) {
        if (!this._container || !pin?.actor) {
            return;
        }
        if (pin.actor.get_parent() === this._container) {
            this._container.set_child_above_sibling(pin.actor, null);
        }
    }

    _syncContainerVisibility() {
        if (!this._container) {
            return;
        }
        const visible = !(this._userHidden || this._systemHidden);
        if (!visible) {
            this._selectPin(null);
        }
        this._container.visible = visible;
    }

    _syncFullscreenVisibility() {
        if (this._destroyed) {
            return;
        }
        const monitors = Main.layoutManager.monitors;
        for (const pin of this._pins.values()) {
            if (!this._config.hide_on_fullscreen) {
                pin.setFullscreenHidden(false);
                continue;
            }
            const idx = pin.getMonitorIndex();
            const monitor = monitors[idx];
            pin.setFullscreenHidden(Boolean(monitor?.inFullscreen));
        }
    }

    _handleMonitorsChanged() {
        if (this._destroyed) {
            return;
        }
        for (const pin of this._pins.values()) {
            pin.ensureVisibleOnMonitors();
        }
        this._syncFullscreenVisibility();
    }

    /**
     * 销毁全部贴图、断开全局监听并移除顶层容器，确保零内存残留
     */
    destroy() {
        if (this._destroyed) {
            return;
        }
        this._destroyed = true;
        global.stage.disconnect(this._selectionSignalId);
        this._selectPin(null);

        if (this._fullscreenSignalId) {
            try {
                global.display.disconnect(this._fullscreenSignalId);
            } catch (err) {
                console.warn(`[pinshot] 断开全屏监听信号失败: ${err}`);
            }
            this._fullscreenSignalId = 0;
        }

        if (this._monitorsSignalId) {
            try {
                Main.layoutManager.disconnect(this._monitorsSignalId);
            } catch (err) {
                console.warn(`[pinshot] 断开显示器热插拔信号失败: ${err}`);
            }
            this._monitorsSignalId = 0;
        }

        for (const pin of this._pins.values()) {
            pin.destroy();
        }
        this._pins.clear();

        if (this._container) {
            this._container.get_parent()?.remove_child(this._container);
            this._container.destroy();
            this._container = null;
        }
    }
}
