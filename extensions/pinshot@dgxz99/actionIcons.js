import Gio from 'gi://Gio';
import St from 'gi://St';

export const CANCEL_ACTION = {id: 'cancel', label: '取消', iconName: 'window-close-symbolic'};
export const CHECK_ICON = 'object-select-symbolic';
export const SCROLLSHOT_ICON = new Gio.FileIcon({file: Gio.File.new_for_uri(import.meta.url).get_parent()
    .resolve_relative_path('icons/scrollshot-symbolic.svg')});

/** 图标按钮不绘制文字；无障碍名称仍能区分确认、贴图和复制等动作 */
export function createActionButton(prefix, {id, label, iconName, gicon, primary = false}, clicked) {
    const icon = new St.Icon({icon_size: 18, ...(gicon ? {gicon} : {icon_name: iconName})});
    const button = new St.Button({name: `${prefix}-${id}`, child: icon,
        style_class: 'pinshot-action-button button', reactive: true, track_hover: true});
    button.set_accessible_name(label);
    if (primary) {
        button.add_style_class_name('pinshot-action-primary');
    }
    button.connect('clicked', clicked);
    return button;
}
