import Adw from 'gi://Adw';
import Gdk from 'gi://Gdk?version=4.0';
import Gio from 'gi://Gio';
import Gtk from 'gi://Gtk?version=4.0';
import {ExtensionPreferences} from 'resource:///org/gnome/Shell/Extensions/js/extensions/prefs.js';

import {parseSelectionShortcut, SELECTION_ACTIONS} from './selectionShortcuts.js';
import {SHORTCUT_ACTIONS} from './shortcutActions.js';

const MEDIA_KEYS = 'org.gnome.settings-daemon.plugins.media-keys';
const SYSTEM_KEYBINDING_SCHEMAS = [
    'org.gnome.desktop.wm.keybindings',
    'org.gnome.shell.keybindings',
    'org.gnome.mutter.keybindings',
    'org.gnome.mutter.wayland.keybindings',
    MEDIA_KEYS,
];
const GLOBAL_MODIFIERS = Gdk.ModifierType.CONTROL_MASK | Gdk.ModifierType.ALT_MASK |
    Gdk.ModifierType.SUPER_MASK | Gdk.ModifierType.META_MASK | Gdk.ModifierType.HYPER_MASK;

function normalizeAccelerator(value) {
    const [valid, key, modifiers] = Gtk.accelerator_parse(value);
    if (!valid || key === 0) {
        return null;
    }
    return Gtk.accelerator_name(Gdk.keyval_to_lower(key), modifiers);
}

function includesAccelerator(values, accelerator) {
    return values.some(value => normalizeAccelerator(value) === accelerator);
}

/** 检查 Pinshot 与可读取的 GNOME 内建／自定义绑定，不修改系统配置 */
function findGlobalConflict(settings, action, accelerator) {
    for (const other of SHORTCUT_ACTIONS) {
        if (action.key !== other.key && includesAccelerator(settings.get_strv(other.key), accelerator)) {
            return other.title;
        }
    }

    const source = Gio.SettingsSchemaSource.get_default();
    for (const id of SYSTEM_KEYBINDING_SCHEMAS) {
        const schema = source?.lookup(id, true);
        if (!schema) {
            continue;
        }
        const system = new Gio.Settings({settings_schema: schema});
        for (const key of schema.list_keys()) {
            if (key === 'custom-keybindings') {
                continue;
            }
            const value = system.get_value(key);
            if (value.get_type_string() === 'as' && includesAccelerator(value.get_strv(), accelerator)) {
                return `GNOME 系统快捷键：${key}`;
            }
        }

        if (id === MEDIA_KEYS && schema.has_key('custom-keybindings')) {
            const customSchema = source.lookup(`${MEDIA_KEYS}.custom-keybinding`, true);
            if (!customSchema) {
                continue;
            }
            for (const path of system.get_strv('custom-keybindings')) {
                const custom = new Gio.Settings({settings_schema: customSchema, path});
                if (normalizeAccelerator(custom.get_string('binding')) === accelerator) {
                    return `GNOME 自定义快捷键：${custom.get_string('name') || custom.get_string('binding')}`;
                }
            }
        }
    }
    return null;
}

/** 框选已抓取输入，允许与桌面的绑定共用同一个键，只检测同组动作 */
function findSelectionConflict(settings, action, accelerator) {
    const label = parseSelectionShortcut(accelerator)?.label;
    if (!label) {
        return null;
    }
    const other = SELECTION_ACTIONS.find(({key}) => key !== action.key &&
        parseSelectionShortcut(settings.get_strv(key)[0] ?? '')?.label === label);
    return other?.label ?? null;
}

/** Pinshot 快捷键、截图输出与贴图操作速查 */
export default class PinshotPreferences extends ExtensionPreferences {
    fillPreferencesWindow(window) {
        const settings = this.getSettings();
        const page = new Adw.PreferencesPage({title: '快捷键', icon_name: 'preferences-desktop-keyboard-shortcuts-symbolic'});
        const shortcuts = new Adw.PreferencesGroup({
            title: '全局快捷键',
            description: '无需选中贴图即可使用；不会修改 GNOME 系统设置里已有的绑定。',
        });
        for (const action of SHORTCUT_ACTIONS) {
            shortcuts.add(this._createShortcutRow(window, settings, action));
        }
        page.add(shortcuts);

        const selection = new Adw.PreferencesGroup({
            title: '框选内快捷键',
            description: '只在截图选框内生效，可和全局键共用，例如 Ctrl+T。Enter 确认默认动作，Esc 取消。',
        });
        for (const {key, label} of SELECTION_ACTIONS) {
            selection.add(this._createShortcutRow(window, settings, {key, title: label}, true));
        }
        page.add(selection);
        page.add(this._createPinControls());
        window.add(page);
        window.add(this._createScreenshotPage(settings));
    }

    _createScreenshotPage(settings) {
        const page = new Adw.PreferencesPage({title: '截图', icon_name: 'camera-photo-symbolic'});
        const group = new Adw.PreferencesGroup({
            title: 'Pinshot 截图输出',
            description: '只影响 Pinshot，不改变 GNOME 自带截图。目录和文件名也用于右键「保存原图」，不影响贴图历史。',
        });
        const autoSave = new Adw.SwitchRow({
            title: '自动保存到目录',
            subtitle: '关闭时，截图贴图仍保留在 Pinshot 历史中；标注结果仍可从剪贴板粘贴。',
        });
        settings.bind('screenshot-auto-save', autoSave, 'active', Gio.SettingsBindFlags.DEFAULT);
        group.add(autoSave);

        const location = new Adw.PreferencesGroup({
            title: '保存位置和文件名',
            description: '目录可用绝对路径或 ~/，留空沿用 Pinshot 的配置。文件名自动加上本地保存时间，例如 pinshot-20261121-214530.png；同一秒重名时再加序号。',
        });
        for (const [key, title] of [
            ['screenshot-save-dir', '保存目录（默认 ~/Pictures/Pinshots）'],
            ['screenshot-filename', '文件名前缀（默认 pinshot）'],
        ]) {
            const row = new Adw.EntryRow({title, show_apply_button: true, text: settings.get_string(key)});
            row.connect('apply', () => settings.set_string(key, row.text.trim()));
            location.add(row);
        }
        const notify = new Adw.SwitchRow({
            title: '显示截图完成通知',
            subtitle: '只控制截图流程的成功通知，不影响 GNOME 自带截图。',
        });
        settings.bind('screenshot-notify', notify, 'active', Gio.SettingsBindFlags.DEFAULT);
        group.add(notify);
        const clipboard = new Adw.PreferencesGroup({title: '剪贴板图片'});
        const devicePixels = new Adw.SwitchRow({
            title: '外部图片按屏幕像素 1:1 显示',
            subtitle: '用于没有原始倍率的图片，以指针所在屏幕为准。Pinshot 复制的截图优先保留原来的大小，大图仍会自动适屏。',
        });
        settings.bind('clipboard-image-device-pixels', devicePixels, 'active', Gio.SettingsBindFlags.DEFAULT);
        clipboard.add(devicePixels);
        page.add(group);
        page.add(location);
        page.add(clipboard);
        return page;
    }

    _createShortcutRow(window, settings, action, local = false) {
        const findConflict = local ? findSelectionConflict : findGlobalConflict;
        const row = new Adw.ActionRow({title: action.title, subtitle: action.description ?? ''});
        const label = new Gtk.ShortcutLabel({disabled_text: '未设置', valign: Gtk.Align.CENTER});
        const sync = () => label.set_accelerator(settings.get_strv(action.key)[0] ?? '');
        sync();
        const changedId = settings.connect(`changed::${action.key}`, sync);
        window.connect('close-request', () => {
            settings.disconnect(changedId);
            return false;
        });

        const edit = new Gtk.Button({label: '更改', valign: Gtk.Align.CENTER});
        edit.connect('clicked', () => {
            if (local) {
                this._chooseSelectionShortcut(window, settings, action);
            } else {
                this._recordShortcut(window, settings, action);
            }
        });
        const clear = new Gtk.Button({label: '禁用', valign: Gtk.Align.CENTER});
        clear.connect('clicked', () => settings.set_strv(action.key, []));
        const reset = new Gtk.Button({label: '恢复默认', valign: Gtk.Align.CENTER});
        reset.connect('clicked', () => {
            const defaults = settings.get_default_value(action.key).get_strv();
            for (const value of defaults) {
                const conflict = findConflict(settings, action, normalizeAccelerator(value));
                if (conflict) {
                    const dialog = new Adw.AlertDialog({heading: '无法恢复默认快捷键', body: `默认快捷键已被「${conflict}」使用。`});
                    dialog.add_response('close', '关闭');
                    dialog.present(window);
                    return;
                }
            }
            settings.reset(action.key);
        });
        for (const widget of [label, edit, clear, reset]) {
            row.add_suffix(widget);
        }
        return row;
    }

    _chooseSelectionShortcut(window, settings, action) {
        const hint = '选择按键和修饰键，无需按下可能已被桌面绑定的组合键。';
        const dialog = new Adw.AlertDialog({heading: action.title, body: hint});
        dialog.add_response('cancel', '取消');
        dialog.set_close_response('cancel');

        const current = parseSelectionShortcut(settings.get_strv(action.key)[0] ?? '');
        const keys = [...'ABCDEFGHIJKLMNOPQRSTUVWXYZ', ...Array.from({length: 12}, (_, i) => `F${i + 1}`)];
        const key = Gtk.DropDown.new_from_strings(keys);
        key.set_selected(Math.max(0, keys.indexOf(current?.key.toUpperCase())));
        const control = new Gtk.CheckButton({label: 'Ctrl', active: current?.control ?? false});
        const shift = new Gtk.CheckButton({label: 'Shift', active: current?.shift ?? false});
        const modifiers = new Gtk.Box({spacing: 12, halign: Gtk.Align.CENTER});
        modifiers.append(control);
        modifiers.append(shift);
        const save = new Gtk.Button({label: '应用快捷键', css_classes: ['suggested-action']});
        save.connect('clicked', () => {
            const keyName = keys[key.get_selected()];
            const accelerator = `${control.active ? '<Control>' : ''}${shift.active ? '<Shift>' : ''}` +
                (keyName.length === 1 ? keyName.toLowerCase() : keyName);
            const conflict = findSelectionConflict(settings, action, accelerator);
            if (conflict) {
                dialog.set_body(`此快捷键已被框选内的「${conflict}」使用，请另选一组。`);
                return;
            }
            if (!settings.set_strv(action.key, [accelerator])) {
                dialog.set_body('无法写入快捷键设置。');
                return;
            }
            dialog.close();
        });
        const options = new Gtk.Box({orientation: Gtk.Orientation.VERTICAL, spacing: 12});
        options.append(key);
        options.append(modifiers);
        options.append(save);
        dialog.set_extra_child(options);
        dialog.present(window);
    }

    _recordShortcut(window, settings, action) {
        const dialog = new Adw.AlertDialog({heading: action.title, body: '按下组合键或功能键，按 Esc 取消。'});
        dialog.add_response('cancel', '取消');
        dialog.set_close_response('cancel');
        const controller = new Gtk.EventControllerKey({propagation_phase: Gtk.PropagationPhase.CAPTURE});
        controller.connect('key-pressed', (_controller, key, _code, state) => {
            const modifiers = state & Gtk.accelerator_get_default_mod_mask();
            if (key === Gdk.KEY_Escape && modifiers === 0) {
                dialog.close();
                return true;
            }
            const functionKey = key >= Gdk.KEY_F1 && key <= Gdk.KEY_F35;
            if ((!(modifiers & GLOBAL_MODIFIERS) && !functionKey) || !Gtk.accelerator_valid(key, modifiers)) {
                dialog.set_body('请使用带 Ctrl、Alt 或 Super 等修饰键的组合键，或单独使用功能键。');
                return true;
            }
            const accelerator = Gtk.accelerator_name(Gdk.keyval_to_lower(key), modifiers);
            const conflict = findGlobalConflict(settings, action, accelerator);
            if (conflict) {
                dialog.set_body(`此快捷键已被「${conflict}」使用，请另选一组。`);
                return true;
            }
            if (!settings.set_strv(action.key, [accelerator])) {
                dialog.set_body('无法写入快捷键设置。');
                return true;
            }
            dialog.close();
            return true;
        });
        dialog.add_controller(controller);
        dialog.present(window);
    }

    _createPinControls() {
        const group = new Adw.PreferencesGroup({
            title: '贴图操作速查',
            description: '局部按键只在贴图接收到键盘事件时生效；也可使用右键菜单。开启穿透后可用上面的全局快捷键，或运行 pinshot unghost-all 解除。',
        });
        for (const [title, subtitle] of [
            ['移动贴图', '左键拖动'],
            ['关闭贴图', '双击左键或单击中键；贴图接收按键时也可按 Esc'],
            ['调整大小／不透明度', '滚轮／Ctrl + 滚轮'],
            ['收起／展开迷你态', 'Shift + R'],
            ['复制图片／Satty 标注', 'Ctrl + C／E'],
            ['开启点击穿透／恢复原尺寸', 'T／0'],
        ]) {
            group.add(new Adw.ActionRow({title, subtitle}));
        }
        return group;
    }
}
