import Meta from 'gi://Meta';
import Shell from 'gi://Shell';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';

import * as Main from 'resource:///org/gnome/shell/ui/main.js';

import {firePinshot} from './coreRunner.js';
import {PinshotDbusService} from './dbusService.js';
import {PinManager} from './pinManager.js';
import {SHORTCUT_ACTIONS} from './shortcutActions.js';

/**
 * Pinshot GNOME Shell 51 扩展入口
 */
export default class PinshotExtension extends Extension {
    enable() {
        this._pinManager = new PinManager();
        this._dbusService = new PinshotDbusService(this._pinManager);
        this._dbusService.enable();

        this._registerShortcuts();

        // 按 config.toml 的 restore_on_startup 决定是否自动恢复上次活跃贴图
        firePinshot(['internal', 'restore-startup']);
    }

    _registerShortcuts() {
        const settings = this.getSettings();
        const handlers = {
            'pinshot-toggle-hide': () => this._pinManager.toggleHideAll(),
            'pinshot-unghost-all': () => this._pinManager.unghostAll(),
        };
        this._registeredShortcuts = [];
        for (const {key, command} of SHORTCUT_ACTIONS) {
            const handler = handlers[key] ?? (() => firePinshot([command]));
            const action = Main.wm.addKeybinding(key, settings, Meta.KeyBindingFlags.IGNORE_AUTOREPEAT,
                Shell.ActionMode.NORMAL, handler);
            if (action !== Meta.KeyBindingAction.NONE) {
                this._registeredShortcuts.push(key);
            } else {
                console.warn(`[pinshot] 无法注册快捷键：${key}`);
            }
        }
    }

    disable() {
        for (const key of this._registeredShortcuts ?? []) {
            Main.wm.removeKeybinding(key);
        }
        this._registeredShortcuts = null;

        if (this._dbusService) {
            this._dbusService.destroy();
            this._dbusService = null;
        }

        if (this._pinManager) {
            this._pinManager.destroy();
            this._pinManager = null;
        }
    }
}
