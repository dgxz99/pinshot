import assert from 'node:assert/strict';
import test from 'node:test';

import {parseSelectionShortcut} from '../extensions/pinshot@dgxz99/selectionShortcuts.js';

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
