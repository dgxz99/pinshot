import Clutter from 'gi://Clutter';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';

/**
 * 贴图生命周期与顶层非窗口容器管理器（M0 骨架，M1 接入 PinActor 完整渲染）
 */
export class PinManager {
    constructor() {
        this._pins = new Map();
        this._userHidden = false;
        this._systemHidden = false;
        this._container = new Clutter.Actor({
            name: 'pinshot-overlay-container',
            reactive: false,
        });
        Main.layoutManager.uiGroup.add_child(this._container);
    }

    createPin(payload) {
        if (!payload || !payload.id) {
            return false;
        }
        // M0 阶段记录状态占位，M1 将替换为真实 PinActor 非窗口渲染
        this._pins.set(payload.id, {
            id: payload.id,
            x: payload.initial_pos?.x ?? 0,
            y: payload.initial_pos?.y ?? 0,
            width: Math.round(payload.width / (payload.source_scale || 1.0)),
            height: Math.round(payload.height / (payload.source_scale || 1.0)),
            scale: 1.0,
            opacity: 1.0,
            mini: false,
            ghost: false,
            hidden: false,
            title_preview: payload.title_preview ?? '',
            kind: payload.kind ?? 'screenshot',
            has_text: Boolean(payload.has_text),
        });
        return true;
    }

    updatePin(id, payload) {
        if (!this._pins.has(id)) {
            return this.createPin(payload);
        }
        const existing = this._pins.get(id);
        this._pins.set(id, {
            ...existing,
            width: Math.round(payload.width / (payload.source_scale || 1.0)),
            height: Math.round(payload.height / (payload.source_scale || 1.0)),
            title_preview: payload.title_preview ?? existing.title_preview,
            has_text: Boolean(payload.has_text),
            hidden: false,
        });
        return true;
    }

    setPinTemporaryHidden(id, hidden) {
        const entry = this._pins.get(id);
        if (!entry) {
            return false;
        }
        entry.hidden = hidden;
        return true;
    }

    closePin(id) {
        return this._pins.delete(id);
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
                pin.ghost = false;
                count += 1;
            }
        }
        return count;
    }

    listActivePins() {
        return Array.from(this._pins.values());
    }

    selectRegion() {
        // M3 阶段由 regionSelector.js 提供真实交互框选
        return {
            cancelled: true,
            monitor_index: 0,
            x: 0,
            y: 0,
            width: 0,
            height: 0,
            scale: 1.0,
        };
    }

    updateLongshotHud(_state) {
        // M3 阶段由 longshotHud.js 实现悬浮控制条
        return true;
    }

    _syncContainerVisibility() {
        if (!this._container) {
            return;
        }
        this._container.visible = !(this._userHidden || this._systemHidden);
    }

    /**
     * 销毁全部贴图与顶层容器，确保禁用扩展后零残留
     */
    destroy() {
        this._pins.clear();
        if (this._container) {
            const parent = this._container.get_parent();
            if (parent) {
                parent.remove_child(this._container);
            }
            this._container.destroy();
            this._container = null;
        }
    }
}
