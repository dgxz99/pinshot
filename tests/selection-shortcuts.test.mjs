import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import test from 'node:test';

import {OUTPUT_ACTIONS, parseSelectionShortcut, SELECTION_ACTIONS} from '../extensions/pinshot@dgxz99/selectionShortcuts.js';

test('both toolbars share ordered actions and package their SVGs with the extension', () => {
    const actionIds = OUTPUT_ACTIONS.map(({id}) => id);
    assert.deepEqual(actionIds, ['edit', 'pin', 'save', 'cancel', 'copy']);
    assert.deepEqual(SELECTION_ACTIONS.map(({id}) => id), ['edit', 'pin', 'copy']);
    const icons = new URL('../extensions/pinshot@dgxz99/icons/', import.meta.url);
    for (const id of ['scrollshot', 'start', ...actionIds]) {
        assert.match(readFileSync(new URL(`${id}-symbolic.svg`, icons), 'utf8'), /<svg[^>]+viewBox=/);
    }
});

test('understands defaults and Ctrl+T with GTK modifier aliases', () => {
    assert.deepEqual(parseSelectionShortcut('p'), {key: 'p', control: false, shift: false, label: 'P'});
    assert.deepEqual(parseSelectionShortcut('<Control>c'), {key: 'c', control: true, shift: false, label: 'Ctrl+C'});
    assert.deepEqual(parseSelectionShortcut('<Primary>T'), {key: 't', control: true, shift: false, label: 'Ctrl+T'});
});

test('normalizes modifier order and function key case for conflict checks', () => {
    const shortcut = {key: 'F12', control: true, shift: true, label: 'Ctrl+Shift+F12'};
    assert.deepEqual(parseSelectionShortcut('<Shift><Control>F12'), shortcut);
    assert.deepEqual(parseSelectionShortcut('<Ctrl><Shift>f12'), shortcut);
});

test('rejects reserved keys, unsupported accelerators and disabled actions', () => {
    for (const accelerator of ['', 'Escape', 'Return', 'KP_Enter', '<Control>Return', '<Alt>t', '<Super>t', 'F13', 'ctrl+t']) {
        assert.equal(parseSelectionShortcut(accelerator), null, accelerator);
    }
});
