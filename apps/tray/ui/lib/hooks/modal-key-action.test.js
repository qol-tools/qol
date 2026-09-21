import { test } from 'node:test';
import assert from 'node:assert/strict';
import { resolveModalKeyAction } from './modal-key-action.js';

const cases = [
    ['esc on surface w/ onClose closes',
        { key: 'Escape', ctrlKey: false, isEditing: false, hasOnClose: true }, 'close'],
    ['esc on surface w/o onClose is noop (lets globalSurfaceNav.ascendLayer fire)',
        { key: 'Escape', ctrlKey: false, isEditing: false, hasOnClose: false }, 'noop'],

    ['ctrl+enter on surface saves',
        { key: 'Enter', ctrlKey: true, isEditing: false, hasOnClose: true }, 'save'],
    ['ctrl+enter while editing blurs edit and saves',
        { key: 'Enter', ctrlKey: true, isEditing: true, hasOnClose: true }, 'blur-edit-and-save'],

    ['enter on surface is noop',
        { key: 'Enter', ctrlKey: false, isEditing: false, hasOnClose: true }, 'noop'],

    ['esc while editing blurs edit',
        { key: 'Escape', ctrlKey: false, isEditing: true, hasOnClose: true }, 'blur-edit'],
    ['enter while editing blurs edit',
        { key: 'Enter', ctrlKey: false, isEditing: true, hasOnClose: true }, 'blur-edit'],

    ['letter on surface is noop',
        { key: 'a', ctrlKey: false, isEditing: false, hasOnClose: true }, 'noop'],
    ['letter while editing is noop',
        { key: 'a', ctrlKey: false, isEditing: true, hasOnClose: true }, 'noop'],
    ['arrow on surface is noop',
        { key: 'ArrowDown', ctrlKey: false, isEditing: false, hasOnClose: true }, 'noop'],
    ['arrow while editing is noop',
        { key: 'ArrowDown', ctrlKey: false, isEditing: true, hasOnClose: true }, 'noop'],
    ['tab on surface is noop (Tab cycling owned by app-level routing)',
        { key: 'Tab', ctrlKey: false, isEditing: false, hasOnClose: true }, 'noop'],

    ['esc while editing without onClose still blurs edit',
        { key: 'Escape', ctrlKey: false, isEditing: true, hasOnClose: false }, 'blur-edit'],
];

for (const [desc, input, expected] of cases) {
    test(`resolveModalKeyAction: ${desc}`, () => {
        assert.equal(resolveModalKeyAction(input), expected);
    });
}

test('regression: esc on dive-editor form-group surface closes (not noop)', () => {
    const action = resolveModalKeyAction({
        key: 'Escape',
        ctrlKey: false,
        isEditing: false,
        hasOnClose: true,
    });
    assert.equal(action, 'close',
        'Esc must call onClose so editModal clears and Tab cycling resumes after ascend');
});
