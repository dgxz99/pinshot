import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import St from 'gi://St';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import * as PanelMenu from 'resource:///org/gnome/shell/ui/panelMenu.js';
import * as PopupMenu from 'resource:///org/gnome/shell/ui/popupMenu.js';

import {firePinshot} from './coreRunner.js';

const ICON_DIR = Gio.File.new_for_uri(import.meta.url).get_parent().get_child('icons');

/**
 * 原生 GNOME 顶栏图标与快捷菜单（无额外托盘扩展依赖，支持彩色与单色图标切换）
 */
export class PinshotPanelIndicator {
    constructor(settings, pinManager, callbacks) {
        this._settings = settings;
        this._pinManager = pinManager;
        this._callbacks = callbacks;
        this._pendingTimerIds = new Set();

        this._button = new PanelMenu.Button(0.0, 'Pinshot', false);
        this._button.add_style_class_name('pinshot-panel-button');
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
        menu.addAction('截取当前全屏', () => this._launchAfterClose(['capture-fullscreen']));
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

    /** 立即中止菜单淡出动画并等待合成器重绘数帧，避免下拉菜单被冻进截图画面 */
    _launchAfterClose(args) {
        const menuActor = this._button.menu.actor;
        menuActor.remove_all_transitions();
        menuActor.hide();
        menuActor.opacity = 255;
        this._button.menu.close(false);

        const timerId = GLib.timeout_add(GLib.PRIORITY_DEFAULT, 80, () => {
            this._pendingTimerIds.delete(timerId);
            if (this._button) {
                firePinshot(args);
            }
            return GLib.SOURCE_REMOVE;
        });
        this._pendingTimerIds.add(timerId);
    }

    _refreshDynamicItems() {
        const hidden = this._pinManager.userHidden;
        this._toggleHideItem.label.text = hidden ? '显示全部贴图' : '隐藏全部贴图';

        const ghostCount = this._pinManager.listActivePins().filter(pin => pin.ghost).length;
        this._unghostItem.label.text = ghostCount > 0
            ? `解除全部点击穿透 (${ghostCount})`
            : '解除全部点击穿透';
        this._unghostItem.setSensitive(ghostCount > 0);

        this._disableShortcutsItem.setToggleState(Boolean(this._callbacks.isShortcutsDisabled()));
    }

    destroy() {
        for (const timerId of this._pendingTimerIds) {
            GLib.Source.remove(timerId);
        }
        this._pendingTimerIds.clear();

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
