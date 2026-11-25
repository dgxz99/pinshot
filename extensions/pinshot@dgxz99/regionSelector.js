import Clutter from 'gi://Clutter';
import Pango from 'gi://Pango';
import Shell from 'gi://Shell';
import St from 'gi://St';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';

import {createActionButton} from './actionIcons.js';
import {containsPoint, handleAt, handlesFor, moveRegion, regionBetween, resizeRegion,
    toolbarMonitor, toolbarPosition} from './selectionGeometry.js';

const HANDLE_SIZE = 8;
const HANDLE_RADIUS = 7;
const SHORTCUT_MODIFIERS = Clutter.ModifierType.SHIFT_MASK | Clutter.ModifierType.CONTROL_MASK |
    Clutter.ModifierType.MOD1_MASK | Clutter.ModifierType.SUPER_MASK | Clutter.ModifierType.META_MASK |
    Clutter.ModifierType.HYPER_MASK;
const CURSORS = [
    [Clutter.CursorType.NW_RESIZE, Clutter.CursorType.N_RESIZE, Clutter.CursorType.NE_RESIZE],
    [Clutter.CursorType.W_RESIZE, Clutter.CursorType.MOVE, Clutter.CursorType.E_RESIZE],
    [Clutter.CursorType.SW_RESIZE, Clutter.CursorType.S_RESIZE, Clutter.CursorType.SE_RESIZE],
];

export const CANCELLED_REGION = [0, 0, 0, 0];

/** 保留可调整的选区，只有确认或取消才结束模态并归还输入 */
export class RegionSelector {
    constructor(area, {name, bounds, minEdge = 8, hint = null, background = null, actions, defaultAction,
        initialRect = null}) {
        this._area = area;
        this._name = name;
        this._bounds = bounds ?? {left: 0, top: 0, right: area.width, bottom: area.height};
        this._minEdge = minEdge;
        this._hintPlacement = hint;
        this._background = background;
        this._actions = actions;
        this._defaultAction = defaultAction;
        this._rect = initialRect;
        this._drag = null;
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
        this._outline = new St.Widget({name: 'pinshot-selection-rect', style_class: 'pinshot-selection-outline'});
        this._handles = Array.from({length: 8}, () => new St.Widget({
            style_class: 'pinshot-selection-handle', width: HANDLE_SIZE, height: HANDLE_SIZE, visible: false,
        }));
        for (const actor of [...this._shades, this._outline, ...this._handles]) {
            this._layer.add_child(actor);
        }
        if (this._hintPlacement) {
            const {x: hintX, y: hintY, width: hintWidth, text} = this._hintPlacement;
            this._hint = new St.Label({style_class: 'pinshot-selection-hint', text});
            this._hint.clutter_text.ellipsize = Pango.EllipsizeMode.END;
            this._hint.set_position(hintX, hintY);
            this._hint.set_width(hintWidth);
            this._layer.add_child(this._hint);
        }
        this._toolbar = this._createToolbar();
        this._layer.add_child(this._toolbar);
        this._draw(this._rect);
        Main.layoutManager.uiGroup.add_child(this._layer);
        this._showToolbar();
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
            this._resolveSelection(null);
        });
        return new Promise(resolve => {
            this._resolve = resolve;
        });
    }

    _createToolbar() {
        const toolbar = new St.BoxLayout({name: 'pinshot-selection-toolbar',
            style_class: 'pinshot-selection-toolbar', reactive: true, visible: false});
        toolbar.set_cursor_type(Clutter.CursorType.DEFAULT);
        for (const action of this._actions) {
            const button = createActionButton('pinshot-selection-action', action, () => {
                if (action.id === 'cancel') {
                    this.close();
                } else {
                    this._confirm(action.id === 'confirm' ? this._defaultAction : action.id);
                }
            });
            toolbar.add_child(button);
        }
        return toolbar;
    }

    _onEvent(event) {
        const type = event.type();
        if (!this._layer || type === Clutter.EventType.ENTER || type === Clutter.EventType.LEAVE) {
            return Clutter.EVENT_PROPAGATE;
        }
        if (type === Clutter.EventType.KEY_PRESS) {
            this._onKey(event);
        } else if (type === Clutter.EventType.BUTTON_PRESS && event.get_button() === Clutter.BUTTON_SECONDARY) {
            this.close();
        } else if (type === Clutter.EventType.MOTION || type === Clutter.EventType.BUTTON_PRESS ||
            type === Clutter.EventType.BUTTON_RELEASE) {
            if (!this._drag && this._toolbar.visible) {
                const target = global.stage.get_event_actor(event);
                if (target && this._toolbar.contains(target)) {
                    return Clutter.EVENT_PROPAGATE;
                }
            }
            this._onPointer(type, event);
        }
        return Clutter.EVENT_STOP;
    }

    _onKey(event) {
        const key = event.get_key_symbol();
        if (key === Clutter.KEY_Escape) {
            this.close();
            return;
        }
        if (!this._rect || this._drag) {
            return;
        }
        const modifiers = event.get_state() & SHORTCUT_MODIFIERS;
        if (modifiers === 0 && (key === Clutter.KEY_Return || key === Clutter.KEY_KP_Enter)) {
            this._confirm(this._defaultAction);
            return;
        }
        const character = String.fromCodePoint(Clutter.keysym_to_unicode(key)).toLowerCase();
        const action = this._actions.find(({shortcut}) => {
            if (!shortcut) {
                return false;
            }
            const shortcutModifiers = (shortcut.control ? Clutter.ModifierType.CONTROL_MASK : 0) |
                (shortcut.shift ? Clutter.ModifierType.SHIFT_MASK : 0);
            return modifiers === shortcutModifiers &&
                (shortcut.key.length === 1 ? shortcut.key === character : key === Clutter[`KEY_${shortcut.key}`]);
        });
        if (action) {
            this._confirm(action.id);
        }
    }

    _onPointer(type, event) {
        if (type === Clutter.EventType.MOTION) {
            const point = this._pointAt(event);
            if (this._drag) {
                this._draw(this._dragRegion(point));
            } else {
                this._updateCursor(point);
            }
        } else if (type === Clutter.EventType.BUTTON_PRESS && event.get_button() === Clutter.BUTTON_PRIMARY) {
            this._startDrag(this._pointAt(event));
        } else if (this._drag && type === Clutter.EventType.BUTTON_RELEASE &&
            event.get_button() === Clutter.BUTTON_PRIMARY) {
            this._endDrag(this._pointAt(event));
        }
    }

    _pointAt(event) {
        const [x, y] = event.get_coords();
        return [Math.round(x - this._area.x), Math.round(y - this._area.y)];
    }

    _clampPoint([x, y]) {
        const {left, top, right, bottom} = this._bounds;
        return [Math.min(Math.max(left, x), right), Math.min(Math.max(top, y), bottom)];
    }

    _operationAt(point) {
        if (!this._rect) {
            return null;
        }
        return handleAt(this._rect, point, HANDLE_RADIUS) ??
            (containsPoint(this._rect, point) ? [0, 0] : null);
    }

    _updateCursor(point) {
        const operation = this._operationAt(point);
        let cursor = Clutter.CursorType.CROSSHAIR;
        if (operation) {
            const [hx, hy] = operation;
            cursor = CURSORS[hy + 1][hx + 1];
        }
        this._layer.set_cursor_type(cursor);
    }

    _startDrag(point) {
        const [x, y] = point;
        const operation = this._operationAt(point);
        const {left, top, right, bottom} = this._bounds;
        if (!operation && (x < left || x >= right || y < top || y >= bottom)) {
            return;
        }
        this._updateCursor(point);
        this._drag = {origin: point, operation};
        this._toolbar.hide();
        this._draw(this._dragRegion(point));
    }

    _dragRegion(point) {
        const {origin, operation} = this._drag;
        if (!operation) {
            return regionBetween(origin, this._clampPoint(point));
        }
        const [hx, hy] = operation;
        const delta = [point[0] - origin[0], point[1] - origin[1]];
        if (hx === 0 && hy === 0) {
            return moveRegion(this._rect, delta, this._bounds);
        }
        return resizeRegion(this._rect, operation, delta, this._bounds, this._minEdge);
    }

    _endDrag(point) {
        const rect = this._dragRegion(point);
        this._drag = null;
        if (rect[2] >= this._minEdge && rect[3] >= this._minEdge) {
            this._rect = rect;
        } else if (this._hint) {
            this._hint.text = '区域太小，请重新拖动 · Esc 取消';
        }
        this._draw(this._rect);
        this._updateCursor(point);
        this._showToolbar();
    }

    _draw(rect) {
        const [x, y, width, height] = rect ?? CANCELLED_REGION;
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
            this._shades[index].visible = sw > 0 && sh > 0;
        }
        this._outline.set_position(x, y);
        this._outline.set_size(width, height);
        this._outline.visible = width > 0 && height > 0;
        for (const [index, [, , hx, hy]] of handlesFor(this._outline.visible ? rect : CANCELLED_REGION).entries()) {
            const handle = this._handles[index];
            handle.set_position(hx - HANDLE_SIZE / 2, hy - HANDLE_SIZE / 2);
            handle.visible = Boolean(rect && !this._drag);
        }
        if (this._hint) {
            this._hint.visible = !rect && !this._drag;
        }
    }

    _showToolbar() {
        if (!this._rect) {
            return;
        }
        const [x, y, width, height] = this._rect;
        const absolute = [x + this._area.x, y + this._area.y, width, height];
        const monitor = toolbarMonitor(absolute, Main.layoutManager.monitors);
        if (!monitor) {
            return;
        }
        this._toolbar.show();
        const [, tw] = this._toolbar.get_preferred_width(-1);
        const [, th] = this._toolbar.get_preferred_height(tw);
        const [tx, ty] = toolbarPosition(absolute, [tw, th], monitor);
        this._toolbar.set_position(tx - this._area.x, ty - this._area.y);
        this._toolbar.set_size(tw, th);
    }

    _confirm(action) {
        if (this._rect && !this._drag) {
            this.close({rect: this._rect, action});
        }
    }

    close(result = null) {
        this._clearGrabHandler();
        this._resolveSelection(result);
        const layer = this._layer;
        this._layer = null;
        layer?.destroy();
        this._grab = null;
    }

    _clearGrabHandler() {
        if (this._revokedId) {
            this._grab.disconnect(this._revokedId);
            this._revokedId = 0;
        }
    }

    _resolveSelection(result) {
        const resolve = this._resolve;
        this._resolve = null;
        resolve?.(result);
    }
}
