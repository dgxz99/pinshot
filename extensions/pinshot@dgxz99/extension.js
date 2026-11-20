import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';

import {PinshotDbusService} from './dbusService.js';
import {PinManager} from './pinManager.js';

/**
 * Pinshot GNOME Shell 51 扩展入口
 */
export default class PinshotExtension extends Extension {
    enable() {
        this._pinManager = new PinManager();
        this._dbusService = new PinshotDbusService(this._pinManager);
        this._dbusService.enable();
    }

    disable() {
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
