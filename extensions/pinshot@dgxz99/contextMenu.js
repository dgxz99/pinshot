import St from 'gi://St';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import * as PopupMenu from 'resource:///org/gnome/shell/ui/popupMenu.js';

import {firePinshot} from './coreRunner.js';

/**
 * 单张贴图右键上下文菜单（挂入 uiGroup，关闭后立即释放锚点）
 */
export class PinContextMenu {
    constructor(pinActor, callbacks) {
        this._pin = pinActor;
        this._callbacks = callbacks;
        this._anchor = null;
        this._menu = null;
        this._menuManager = null;
        this._openStateId = 0;
    }

    /**
     * 在指定全局坐标打开右键菜单
     */
    openAt(x, y) {
        this.close();

        this._anchor = new St.Widget({
            x: Math.round(x),
            y: Math.round(y),
            width: 1,
            height: 1,
            reactive: false,
        });
        Main.layoutManager.uiGroup.add_child(this._anchor);

        this._menu = new PopupMenu.PopupMenu(this._anchor, 0.0, St.Side.TOP);
        this._menu.actor.add_style_class_name('pinshot-context-menu');
        Main.layoutManager.uiGroup.add_child(this._menu.actor);
        this._menu.actor.hide();

        this._menuManager = new PopupMenu.PopupMenuManager(this._anchor);
        this._menuManager.addMenu(this._menu);

        this._populateItems();

        this._openStateId = this._menu.connect('open-state-changed', (_menu, isOpen) => {
            if (!isOpen) {
                this.close();
            }
        });

        this._menu.open(true);
    }

    _populateItems() {
        const pin = this._pin;

        this._menu.addAction('复制图片', () => {
            firePinshot(['internal', 'copy-image', pin.id]);
        });

        if (pin.hasText) {
            this._menu.addAction('复制原始文本', () => {
                firePinshot(['internal', 'copy-text', pin.id]);
            });
        }

        this._menu.addAction('标注编辑 (satty)', () => {
            firePinshot(['internal', 'edit-satty', pin.id]);
        });

        this._menu.addAction('保存原图', () => {
            firePinshot(['internal', 'save-image', pin.id]);
        });

        this._menu.addMenuItem(new PopupMenu.PopupSeparatorMenuItem());

        this._menu.addAction(pin.mini ? '展开原始贴图' : '收起为迷你图', () => {
            pin.toggleMini();
        });

        this._menu.addAction('恢复 100% 原始尺寸', () => {
            pin.resetScaleTo100();
        });

        this._menu.addAction('开启点击穿透（可从顶栏解除）', () => {
            pin.setGhost(true);
        });

        this._menu.addMenuItem(new PopupMenu.PopupSeparatorMenuItem());

        this._menu.addAction('关闭贴图', () => {
            if (this._callbacks?.onRequestClose) {
                this._callbacks.onRequestClose(pin.id);
            }
        });
    }

    /**
     * 关闭并彻底销毁菜单与定位锚点
     */
    close() {
        if (this._menu && this._openStateId) {
            try {
                this._menu.disconnect(this._openStateId);
            } catch (err) {
                console.warn(`[pinshot] 断开右键菜单信号失败: ${err}`);
            }
            this._openStateId = 0;
        }

        if (this._menuManager && this._menu) {
            try {
                this._menuManager.removeMenu(this._menu);
            } catch (err) {
                console.warn(`[pinshot] 从菜单管理器移除菜单失败: ${err}`);
            }
        }
        this._menuManager = null;

        if (this._menu) {
            this._menu.destroy();
            this._menu = null;
        }

        if (this._anchor) {
            const parent = this._anchor.get_parent();
            if (parent) {
                parent.remove_child(this._anchor);
            }
            this._anchor.destroy();
            this._anchor = null;
        }
    }

    destroy() {
        this.close();
        this._pin = null;
        this._callbacks = null;
    }
}
