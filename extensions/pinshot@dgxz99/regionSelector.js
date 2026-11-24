import Clutter from 'gi://Clutter';
import Pango from 'gi://Pango';
import Shell from 'gi://Shell';
import St from 'gi://St';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';

export const CANCELLED_REGION = [0, 0, 0, 0];

/** 只在用户明确框选时抓取输入，框选完立即归还应用焦点 */
export class RegionSelector {
    constructor(area, {name, bounds, minEdge = 1, hint = null, background = null}) {
        this._area = area;
        this._name = name;
        this._bounds = bounds ?? {left: 0, top: 0, right: area.width, bottom: area.height};
        this._minEdge = minEdge;
        this._hintPlacement = hint;
        this._background = background;
        this._origin = null;
        this._grab = null;
    }

    select() {
        const {x, y, width, height} = this._area;
        this._layer = new St.Widget({
            name: this._name, reactive: true, can_focus: true, content: this._background,
            x, y, width, height, clip_to_allocation: true,
        });
        this._layer.set_cursor_type(Clutter.CursorType.CROSSHAIR);
        this._shades = Array.from({length: 4}, () => new St.Widget({style_class: 'pinshot-selection-shade'}));
        this._outline = new St.Widget({style_class: 'pinshot-selection-outline'});
        for (const actor of [...this._shades, this._outline]) {
            this._layer.add_child(actor);
        }
        this._outline.hide();
        this._shades[0].set_size(width, height);
        for (const shade of this._shades.slice(1)) {
            shade.hide();
        }
        if (this._hintPlacement) {
            const {x: hintX, y: hintY, width: hintWidth, text} = this._hintPlacement;
            this._hint = new St.Label({style_class: 'pinshot-selection-hint', text});
            this._hint.clutter_text.ellipsize = Pango.EllipsizeMode.END;
            this._hint.set_position(hintX, hintY);
            this._hint.set_width(hintWidth);
            this._layer.add_child(this._hint);
        }
        Main.layoutManager.uiGroup.add_child(this._layer);
        this._grab = Main.pushModal(this._layer, {actionMode: Shell.ActionMode.SYSTEM_MODAL});
        if (this._grab.is_revoked()) {
            this.close();
            throw new Error('无法进入截图框选，请先关闭其他 GNOME 弹层');
        }
        this._revokedId = this._grab.connect('notify::revoked', () => {
            if (this._grab?.is_revoked()) {
                this.close();
            }
        });
        this._layer.connect('captured-event', (_actor, event) => this._onEvent(event));
        this._layer.connect('destroy', () => {
            // Shell 在关闭时可能先销毁场景节点；Main 会随节点自动释放 modal grab
            this._layer = null;
            this._clearGrabHandler();
            this._grab = null;
            this._resolveSelection(CANCELLED_REGION);
        });
        return new Promise(resolve => {
            this._resolve = resolve;
        });
    }

    _onEvent(event) {
        const type = event.type();
        if (type === Clutter.EventType.KEY_PRESS && event.get_key_symbol() === Clutter.KEY_Escape) {
            this.close();
        } else if (type === Clutter.EventType.BUTTON_PRESS) {
            if (event.get_button() === Clutter.BUTTON_PRIMARY) {
                this._origin = this._pointAt(event, false);
                if (this._origin) {
                    this._draw(this._origin);
                }
            } else if (event.get_button() === Clutter.BUTTON_SECONDARY) {
                this.close();
            }
        } else if (this._origin && type === Clutter.EventType.MOTION) {
            this._draw(this._pointAt(event, true));
        } else if (this._origin && type === Clutter.EventType.BUTTON_RELEASE &&
            event.get_button() === Clutter.BUTTON_PRIMARY) {
            const rect = this._rectAt(this._pointAt(event, true));
            if (rect[2] >= this._minEdge && rect[3] >= this._minEdge) {
                this.close(rect);
            } else {
                this._draw(this._origin);
                this._origin = null;
                if (this._hint) {
                    this._hint.text = '区域太小，请重新拖动 · Esc 取消';
                }
            }
        }
        return Clutter.EVENT_STOP;
    }

    _pointAt(event, clamp) {
        const {left, top, right, bottom} = this._bounds;
        const [stageX, stageY] = event.get_coords();
        const x = Math.round(stageX - this._area.x);
        const y = Math.round(stageY - this._area.y);
        if (!clamp && (x < left || x >= right || y < top || y >= bottom)) {
            return null;
        }
        return [Math.min(Math.max(left, x), right), Math.min(Math.max(top, y), bottom)];
    }

    _rectAt(point) {
        const [x, y] = this._origin;
        return [Math.min(x, point[0]), Math.min(y, point[1]),
            Math.abs(point[0] - x), Math.abs(point[1] - y)];
    }

    _draw(point) {
        const [x, y, width, height] = this._rectAt(point);
        const right = x + width;
        const bottom = y + height;
        const areas = [
            [0, 0, this._area.width, y],
            [0, y, x, height],
            [right, y, this._area.width - right, height],
            [0, bottom, this._area.width, this._area.height - bottom],
        ];
        for (const [index, [sx, sy, sw, sh]] of areas.entries()) {
            this._shades[index].set_position(sx, sy);
            this._shades[index].set_size(sw, sh);
            this._shades[index].show();
        }
        this._outline.set_position(x, y);
        this._outline.set_size(width, height);
        this._outline.visible = width > 0 && height > 0;
    }

    close(rect = CANCELLED_REGION) {
        this._clearGrabHandler();
        this._resolveSelection(rect);
        this._layer?.destroy();
        this._layer = null;
        this._grab = null;
    }

    _clearGrabHandler() {
        if (this._revokedId) {
            this._grab.disconnect(this._revokedId);
            this._revokedId = 0;
        }
    }

    _resolveSelection(rect) {
        const resolve = this._resolve;
        this._resolve = null;
        resolve?.(rect);
    }
}
