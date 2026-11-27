import Meta from 'gi://Meta';
import Shell from 'gi://Shell';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';

import * as Main from 'resource:///org/gnome/shell/ui/main.js';

import {firePinshot} from './coreRunner.js';
import {PinshotDbusService} from './dbusService.js';
import {PinshotPanelIndicator} from './panelIndicator.js';
import {PinManager} from './pinManager.js';
import {SHORTCUT_ACTIONS} from './shortcutActions.js';

/**
 * Pinshot GNOME Shell 扩展入口
 */
export default class PinshotExtension extends Extension {
    enable() {
        this._settings = this.getSettings();
        this._pinManager = new PinManager();
        this._dbusService = new PinshotDbusService(this._pinManager, this._settings);
        this._dbusService.enable();

        this._registeredShortcuts = [];
        this._shortcutsDisabled = false;
        this._registerShortcuts();

        this._panelIndicator = null;
        this._syncPanelIndicator();
        this._indicatorSettingId = this._settings.connect('changed::show-panel-indicator', () => {
            this._syncPanelIndicator();
        });

        // 按 config.toml 的 restore_on_startup 决定是否自动恢复上次活跃贴图
        firePinshot(['internal', 'restore-startup']);
    }

    _syncPanelIndicator() {
        const show = this._settings.get_boolean('show-panel-indicator');
        if (show && !this._panelIndicator) {
            this._panelIndicator = new PinshotPanelIndicator(this._settings, this._pinManager, {
                isShortcutsDisabled: () => this._shortcutsDisabled,
                setShortcutsDisabled: disabled => this._setShortcutsDisabled(disabled),
                openPreferences: () => this.openPreferences(),
            });
        } else if (!show && this._panelIndicator) {
            this._panelIndicator.destroy();
            this._panelIndicator = null;
        }
    }

    _setShortcutsDisabled(disabled) {
        if (this._shortcutsDisabled === disabled) {
            return;
        }
        this._shortcutsDisabled = disabled;
        if (disabled) {
            this._unregisterShortcuts();
        } else {
            this._registerShortcuts();
        }
    }

    _registerShortcuts() {
        this._unregisterShortcuts();
        const handlers = {
            'pinshot-toggle-hide': () => this._pinManager.toggleHideAll(),
            'pinshot-unghost-all': () => this._pinManager.unghostAll(),
        };
        for (const {key, command, args = [], allowInPopup = false} of SHORTCUT_ACTIONS) {
            const handler = handlers[key] ?? (() => firePinshot([command, ...args]));
            // 区域截图在冻结桌面前不关闭菜单，其他快捷键不抢菜单输入
            const modes = allowInPopup ? Shell.ActionMode.NORMAL | Shell.ActionMode.POPUP : Shell.ActionMode.NORMAL;
            const action = Main.wm.addKeybinding(key, this._settings, Meta.KeyBindingFlags.IGNORE_AUTOREPEAT, modes, handler);
            if (action !== Meta.KeyBindingAction.NONE) {
                this._registeredShortcuts.push(key);
            } else {
                console.warn(`[pinshot] 无法注册快捷键：${key}`);
            }
        }
    }

    _unregisterShortcuts() {
        for (const key of this._registeredShortcuts ?? []) {
            Main.wm.removeKeybinding(key);
        }
        this._registeredShortcuts = [];
    }

    disable() {
        if (this._indicatorSettingId && this._settings) {
            this._settings.disconnect(this._indicatorSettingId);
            this._indicatorSettingId = 0;
        }

        if (this._panelIndicator) {
            this._panelIndicator.destroy();
            this._panelIndicator = null;
        }

        this._unregisterShortcuts();
        this._registeredShortcuts = null;
        this._shortcutsDisabled = false;

        if (this._dbusService) {
            this._dbusService.destroy();
            this._dbusService = null;
        }

        if (this._pinManager) {
            this._pinManager.destroy();
            this._pinManager = null;
        }

        this._settings = null;
    }
}
