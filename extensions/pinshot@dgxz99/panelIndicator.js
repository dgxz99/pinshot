import Clutter from 'gi://Clutter';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Pango from 'gi://Pango';
import St from 'gi://St';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import * as PanelMenu from 'resource:///org/gnome/shell/ui/panelMenu.js';
import * as PopupMenu from 'resource:///org/gnome/shell/ui/popupMenu.js';

import {firePinshot} from './coreRunner.js';

const ICON_DIR = Gio.File.new_for_uri(import.meta.url).get_parent().get_child('icons');
const MAX_PIN_LABEL_CHARS = 26;

function truncateLabel(text) {
    const clean = (text || '贴图').trim() || '贴图';
    const chars = Array.from(clean);
    if (chars.length <= MAX_PIN_LABEL_CHARS) {
        return clean;
    }
    return `${chars.slice(0, MAX_PIN_LABEL_CHARS).join('')}…`;
}

/**
 * 原生 GNOME 顶栏图标与快捷菜单（无额外托盘扩展依赖，支持彩色与单色图标切换）
 */
export class PinshotPanelIndicator {
    constructor(settings, pinManager, callbacks) {
        this._settings = settings;
        this._pinManager = pinManager;
        this._callbacks = callbacks;
        this._pendingIdleIds = new Set();

        this._button = new PanelMenu.Button(0.0, 'Pinshot', false);
        this._icon = new St.Icon({
            style_class: 'system-status-icon',
            icon_size: 16,
        });
        this._button.add_child(this._icon);
        this._syncIcon();

        this._buildMenu();

        this._openStateId = this._button.menu.connect('open-state-changed', (_menu, isOpen) => {
            if (isOpen) {
                this._refreshDynamicItems();
            }
        });
        this._iconStyleId = this._settings.connect('changed::panel-icon-style', () => {
            this._syncIcon();
        });

        Main.panel.addToStatusArea('pinshot-indicator', this._button, 0, 'right');
    }

    _syncIcon() {
        const style = this._settings.get_string('panel-icon-style');
        const filename = style === 'symbolic' ? 'pinshot-symbolic.svg' : 'pinshot.svg';
        this._icon.gicon = new Gio.FileIcon({file: ICON_DIR.get_child(filename)});
    }

    _buildMenu() {
        const menu = this._button.menu;

        menu.addAction('区域截图', () => this._launchAfterClose(['capture']));
        menu.addAction('纵向长截图', () => this._launchAfterClose(['longshot']));
        menu.addAction('横向长截图', () => this._launchAfterClose(['longshot', '--horizontal']));
        menu.addAction('录制 GIF', () => this._launchAfterClose(['gif']));
        menu.addAction('剪贴板贴图', () => firePinshot(['pin-clipboard']));

        menu.addMenuItem(new PopupMenu.PopupSeparatorMenuItem());

        this._toggleHideItem = menu.addAction('隐藏全部贴图', () => {
            this._pinManager.toggleHideAll();
        });
        menu.addAction('恢复最近贴图', () => firePinshot(['restore-last']));
        this._unghostItem = menu.addAction('解除全部点击穿透', () => {
            this._pinManager.unghostAll();
        });

        this._activePinsMenu = new PopupMenu.PopupSubMenuMenuItem('活跃贴图（无）');
        menu.addMenuItem(this._activePinsMenu);

        menu.addAction('清空历史缓存', () => firePinshot(['clear-history']));

        menu.addMenuItem(new PopupMenu.PopupSeparatorMenuItem());

        this._disableShortcutsItem = new PopupMenu.PopupSwitchMenuItem(
            '禁用全局快捷键',
            Boolean(this._callbacks.isShortcutsDisabled())
        );
        this._disableShortcutsItem.connect('toggled', (_item, state) => {
            this._callbacks.setShortcutsDisabled(state);
        });
        menu.addMenuItem(this._disableShortcutsItem);

        menu.addAction('首选项…', () => {
            this._callbacks.openPreferences();
        });
    }

    _scheduleIdle(fn) {
        const idleId = GLib.idle_add(GLib.PRIORITY_DEFAULT_IDLE, () => {
            this._pendingIdleIds.delete(idleId);
            if (this._button) {
                fn();
            }
            return GLib.SOURCE_REMOVE;
        });
        this._pendingIdleIds.add(idleId);
    }

    /** 截图/录制类命令先关闭菜单并等待事件循环空闲，避免下拉菜单被冻进截图或抢占模态输入 */
    _launchAfterClose(args) {
        this._button.menu.close(false);
        this._scheduleIdle(() => firePinshot(args));
    }

    _refreshDynamicItems() {
        const hidden = this._pinManager.userHidden;
        this._toggleHideItem.label.text = hidden ? '显示全部贴图' : '隐藏全部贴图';

        const activePins = this._pinManager.listActivePins();
        const ghostCount = activePins.filter(pin => pin.ghost).length;
        this._unghostItem.label.text = ghostCount > 0
            ? `解除全部点击穿透 (${ghostCount})`
            : '解除全部点击穿透';
        this._unghostItem.setSensitive(ghostCount > 0);

        this._disableShortcutsItem.setToggleState(Boolean(this._callbacks.isShortcutsDisabled()));

        this._activePinsMenu.label.text = activePins.length > 0
            ? `活跃贴图 (${activePins.length})`
            : '活跃贴图（无）';
        this._activePinsMenu.menu.removeAll();

        if (activePins.length === 0) {
            const emptyItem = new PopupMenu.PopupMenuItem('当前没有活跃贴图', {reactive: false});
            emptyItem.setSensitive(false);
            this._activePinsMenu.menu.addMenuItem(emptyItem);
            return;
        }

        for (const pin of activePins) {
            const tags = [];
            if (pin.ghost) {
                tags.push('穿透');
            }
            if (pin.mini) {
                tags.push('迷你');
            }
            const suffix = tags.length > 0 ? ` [${tags.join('·')}]` : '';
            const item = new PopupMenu.PopupMenuItem(`${truncateLabel(pin.title_preview)}${suffix}`);
            item.label.clutter_text.ellipsize = Pango.EllipsizeMode.END;
            item.label.x_expand = true;

            const closeButton = new St.Button({
                style_class: 'pinshot-panel-pin-close',
                child: new St.Icon({icon_name: 'window-close-symbolic', icon_size: 14}),
                reactive: true,
                can_focus: true,
                y_align: Clutter.ActorAlign.CENTER,
            });
            closeButton.set_accessible_name('关闭贴图');
            closeButton.connect('clicked', () => {
                this._pinManager.closePin(pin.id, {notifyCore: true});
                this._scheduleIdle(() => this._refreshDynamicItems());
            });
            item.add_child(closeButton);

            item.connect('activate', () => {
                this._pinManager.focusPin(pin.id);
            });
            this._activePinsMenu.menu.addMenuItem(item);
        }
    }

    destroy() {
        for (const idleId of this._pendingIdleIds) {
            GLib.Source.remove(idleId);
        }
        this._pendingIdleIds.clear();

        if (this._iconStyleId) {
            this._settings.disconnect(this._iconStyleId);
            this._iconStyleId = 0;
        }
        if (this._button) {
            if (this._openStateId) {
                this._button.menu.disconnect(this._openStateId);
                this._openStateId = 0;
            }
            this._button.destroy();
            this._button = null;
        }
        this._settings = null;
        this._pinManager = null;
        this._callbacks = null;
    }
}
