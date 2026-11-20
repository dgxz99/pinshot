import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';

import {firePinshot} from './coreRunner.js';
import {PinshotDbusService} from './dbusService.js';
import {PinshotPanelIndicator} from './panelIndicator.js';
import {PinManager} from './pinManager.js';

/**
 * Pinshot GNOME Shell 51 扩展入口
 */
export default class PinshotExtension extends Extension {
    enable() {
        this._pinManager = new PinManager();
        this._panelIndicator = new PinshotPanelIndicator(this._pinManager);
        this._pinManager.setStateChangedHandler(() => {
            if (this._panelIndicator) {
                this._panelIndicator.syncState();
            }
        });

        this._dbusService = new PinshotDbusService(this._pinManager);
        this._dbusService.enable();

        // 按 config.toml 的 restore_on_startup 决定是否自动恢复上次活跃贴图
        firePinshot(['internal', 'restore-startup']);
    }

    disable() {
        if (this._dbusService) {
            this._dbusService.destroy();
            this._dbusService = null;
        }

        if (this._panelIndicator) {
            this._panelIndicator.destroy();
            this._panelIndicator = null;
        }

        if (this._pinManager) {
            this._pinManager.destroy();
            this._pinManager = null;
        }
    }
}
