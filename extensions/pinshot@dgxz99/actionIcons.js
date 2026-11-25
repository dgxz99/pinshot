import Clutter from 'gi://Clutter';
import Gio from 'gi://Gio';
import St from 'gi://St';

const ICON_DIR = Gio.File.new_for_uri(import.meta.url).get_parent().get_child('icons');

export const CANCEL_ACTION = {id: 'cancel', label: '取消'};

/** 从扩展自身目录加载，不依赖宿主系统安装了哪套图标主题 */
export function createActionButton(prefix, {id, label, icon = id}, clicked, {showLabel = false} = {}) {
    const gicon = new Gio.FileIcon({file: ICON_DIR.get_child(`${icon}-symbolic.svg`)});
    const image = new St.Icon({icon_size: 16, gicon});
    let child = image;
    if (showLabel) {
        child = new St.BoxLayout({style_class: 'pinshot-action-menu-item'});
        child.add_child(image);
        child.add_child(new St.Label({text: label, y_align: Clutter.ActorAlign.CENTER}));
    }
    const button = new St.Button({name: `${prefix}-${id}`, child,
        style_class: 'pinshot-action-button', can_focus: showLabel, reactive: true, track_hover: true});
    button.set_accessible_name(label);
    button.connect('clicked', clicked);
    return button;
}
