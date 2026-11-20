import Clutter from 'gi://Clutter';
import GObject from 'gi://GObject';
import St from 'gi://St';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import * as PanelMenu from 'resource:///org/gnome/shell/ui/panelMenu.js';
import * as PopupMenu from 'resource:///org/gnome/shell/ui/popupMenu.js';

import {firePinshot, runPinshotAsync} from './coreRunner.js';

/**
 * 极简顶栏管理中心（无活跃/隐藏/穿透贴图时自动隐藏，保持顶栏干净）
 */
export class PinshotPanelIndicator extends PanelMenu.Button {
    static {
        GObject.registerClass(this);
    }

    constructor(pinManager) {
        super(0.0, 'Pinshot 管理中心', false);
        this._pinManager = pinManager;
        this._destroyed = false;
        this._historyRequestSeq = 0;

        this._box = new St.BoxLayout({
            style_class: 'panel-status-menu-box pinshot-panel-box',
        });

        this._icon = new St.Icon({
            icon_name: 'view-pin-symbolic',
            style_class: 'system-status-icon',
        });
        this._badge = new St.Label({
            text: '',
            y_align: Clutter.ActorAlign.CENTER,
            style_class: 'pinshot-panel-badge',
        });

        this._box.add_child(this._icon);
        this._box.add_child(this._badge);
        this.add_child(this._box);

        this._menuOpenId = this.menu.connect('open-state-changed', (_menu, isOpen) => {
            if (isOpen && !this._destroyed) {
                this._rebuildMenu();
            }
        });

        this.syncState();
        Main.panel.addToStatusArea('pinshot', this, 1, 'right');
    }

    /**
     * 根据当前贴图总数、隐藏态与穿透数同步顶栏图标显隐与角标
     */
    syncState() {
        if (this._destroyed || !this._pinManager) {
            return;
        }

        const pins = this._pinManager.listActivePins();
        const totalCount = pins.length;
        const ghostCount = pins.filter(p => p.ghost).length;
        const userHidden = this._pinManager.isUserHidden();

        // 无活跃贴图、无隐藏贴图且无穿透贴图时彻底自动隐藏顶栏图标
        const shouldShow = totalCount > 0 || userHidden || ghostCount > 0;
        this.visible = shouldShow;

        if (!shouldShow) {
            return;
        }

        this._badge.text = userHidden ? `${totalCount}·隐` : `${totalCount}`;
        if (ghostCount > 0) {
            this._badge.add_style_class_name('pinshot-panel-badge-ghost');
        } else {
            this._badge.remove_style_class_name('pinshot-panel-badge-ghost');
        }
    }

    _rebuildMenu() {
        this.menu.removeAll();

        const pins = this._pinManager.listActivePins();
        const ghostCount = pins.filter(p => p.ghost).length;
        const userHidden = this._pinManager.isUserHidden();

        // 1. 快捷操作区
        this.menu.addAction(userHidden ? '恢复显示全部贴图' : '全部隐藏贴图', () => {
            this._pinManager.toggleHideAll();
        });

        if (ghostCount > 0) {
            this.menu.addAction(`一键解除全部穿透 (${ghostCount})`, () => {
                this._pinManager.unghostAll();
            });
        }

        if (pins.length > 0) {
            this.menu.addAction('全部收起为迷你图', () => {
                this._pinManager.collapseAllToMini();
            });
        }

        // 2. 当前活跃贴图列表
        this.menu.addMenuItem(new PopupMenu.PopupSeparatorMenuItem('当前贴图'));
        if (pins.length === 0) {
            const emptyItem = new PopupMenu.PopupMenuItem('暂无活跃贴图', {reactive: false});
            this.menu.addMenuItem(emptyItem);
        } else {
            for (const info of pins) {
                this.menu.addMenuItem(this._createActivePinItem(info));
            }
        }

        // 3. 最近历史区（异步按需加载，绝不阻塞主线程）
        this.menu.addMenuItem(new PopupMenu.PopupSeparatorMenuItem('最近历史'));
        const historySection = new PopupMenu.PopupMenuSection();
        const loadingItem = new PopupMenu.PopupMenuItem('正在加载历史记录…', {reactive: false});
        historySection.addMenuItem(loadingItem);
        this.menu.addMenuItem(historySection);

        const activeIdSet = new Set(pins.map(p => p.id));
        this._loadHistoryIntoSection(historySection, activeIdSet);
    }

    _createActivePinItem(info) {
        const item = new PopupMenu.PopupBaseMenuItem({
            style_class: 'pinshot-menu-row',
        });

        const tags = [];
        if (info.ghost) {
            tags.push('穿透');
        }
        if (info.mini) {
            tags.push('迷你');
        }
        if (info.hidden) {
            tags.push('隐藏');
        }
        const tagPrefix = tags.length > 0 ? `[${tags.join('/')}] ` : '';
        const title = info.title_preview || `${Math.round(info.width)}×${Math.round(info.height)}`;

        const label = new St.Label({
            text: `${tagPrefix}${title}`,
            x_expand: true,
            y_align: Clutter.ActorAlign.CENTER,
            style_class: 'pinshot-menu-title',
        });
        item.add_child(label);

        if (info.ghost) {
            const unghostBtn = new St.Button({
                style_class: 'pinshot-menu-icon-btn',
                child: new St.Icon({
                    icon_name: 'changes-allow-symbolic',
                    icon_size: 14,
                }),
                accessible_name: '解除穿透',
            });
            unghostBtn.connect('clicked', () => {
                this._pinManager.setPinGhost(info.id, false);
                this.menu.close();
            });
            item.add_child(unghostBtn);
        }

        const closeBtn = new St.Button({
            style_class: 'pinshot-menu-icon-btn',
            child: new St.Icon({
                icon_name: 'window-close-symbolic',
                icon_size: 14,
            }),
            accessible_name: '关闭贴图',
        });
        closeBtn.connect('clicked', () => {
            this._pinManager.closePin(info.id, {notifyCore: true});
            this.menu.close();
        });
        item.add_child(closeBtn);

        item.connect('activate', () => {
            this._pinManager.focusOrRevealPin(info.id);
        });

        return item;
    }

    async _loadHistoryIntoSection(section, activeIdSet) {
        const seq = ++this._historyRequestSeq;
        const result = await runPinshotAsync(['internal', 'list-history']);
        if (this._destroyed || seq !== this._historyRequestSeq) {
            return;
        }

        section.removeAll();
        if (!result.ok) {
            section.addMenuItem(new PopupMenu.PopupMenuItem('读取历史失败', {reactive: false}));
            return;
        }

        let items = [];
        try {
            items = JSON.parse(result.stdout || '[]');
        } catch (err) {
            console.warn(`[pinshot] 解析历史 JSON 失败: ${err}`);
            section.addMenuItem(new PopupMenu.PopupMenuItem('历史数据格式异常', {reactive: false}));
            return;
        }

        const restorable = items.filter(entry => !activeIdSet.has(entry.id));
        if (restorable.length === 0) {
            section.addMenuItem(new PopupMenu.PopupMenuItem('暂无可恢复历史', {reactive: false}));
            return;
        }

        for (const entry of restorable) {
            section.addMenuItem(this._createHistoryItem(entry));
        }
    }

    _createHistoryItem(entry) {
        const item = new PopupMenu.PopupBaseMenuItem({
            style_class: 'pinshot-menu-row',
        });

        const title = entry.title_preview || `${entry.width}×${entry.height}`;
        const label = new St.Label({
            text: `${title} (${entry.width}×${entry.height})`,
            x_expand: true,
            y_align: Clutter.ActorAlign.CENTER,
            style_class: 'pinshot-menu-title',
        });
        item.add_child(label);

        const deleteBtn = new St.Button({
            style_class: 'pinshot-menu-icon-btn',
            child: new St.Icon({
                icon_name: 'user-trash-symbolic',
                icon_size: 14,
            }),
            accessible_name: '彻底删除历史',
        });
        deleteBtn.connect('clicked', () => {
            firePinshot(['internal', 'delete-history', entry.id]);
            item.destroy();
        });
        item.add_child(deleteBtn);

        item.connect('activate', () => {
            firePinshot(['internal', 'restore-pin', entry.id]);
        });

        return item;
    }

    destroy() {
        if (this._destroyed) {
            return;
        }
        this._destroyed = true;

        if (this._menuOpenId && this.menu) {
            try {
                this.menu.disconnect(this._menuOpenId);
            } catch (err) {
                console.warn(`[pinshot] 断开顶栏菜单信号失败: ${err}`);
            }
            this._menuOpenId = 0;
        }

        this._pinManager = null;
        super.destroy();
    }
}
