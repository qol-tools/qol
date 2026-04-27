import { test } from 'node:test';
import assert from 'node:assert/strict';
import { resolveModalKeyAction } from './modal-key-action.js';

// Parameterized contract for the modal-key dispatch decision.
// Each row: [description, input, expectedAction]
const cases = [
    // Escape on a surface (not editing) MUST close — this is the regression
    // that broke Tab cycling: dive ascend without onClose left editModal set
    // and the parent view's isBlocking() stayed true forever.
    ['esc on surface w/ onClose closes',
        { key: 'Escape', ctrlKey: false, isEditing: false, hasOnClose: true }, 'close'],
    ['esc on surface w/o onClose is noop (lets globalSurfaceNav.ascendLayer fire)',
        { key: 'Escape', ctrlKey: false, isEditing: false, hasOnClose: false }, 'noop'],

    // Ctrl+Enter saves both inside and outside an editing input
    ['ctrl+enter on surface saves',
        { key: 'Enter', ctrlKey: true, isEditing: false, hasOnClose: true }, 'save'],
    ['ctrl+enter while editing blurs edit and saves',
        { key: 'Enter', ctrlKey: true, isEditing: true, hasOnClose: true }, 'blur-edit-and-save'],

    // Bare Enter on a surface does nothing — the underlying surface activates
    // it via globalSurfaceNav, not the modal hook.
    ['enter on surface is noop',
        { key: 'Enter', ctrlKey: false, isEditing: false, hasOnClose: true }, 'noop'],

    // Editing-input cases: Esc and bare Enter blur back to the field surface
    ['esc while editing blurs edit',
        { key: 'Escape', ctrlKey: false, isEditing: true, hasOnClose: true }, 'blur-edit'],
    ['enter while editing blurs edit',
        { key: 'Enter', ctrlKey: false, isEditing: true, hasOnClose: true }, 'blur-edit'],

    // Random other keys are always noop in either state
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

    // hasOnClose=false Escape while editing still blurs edit (no close path
    // through the editor, but caret should still leave the input)
    ['esc while editing without onClose still blurs edit',
        { key: 'Escape', ctrlKey: false, isEditing: true, hasOnClose: false }, 'blur-edit'],
];

for (const [desc, input, expected] of cases) {
    test(`resolveModalKeyAction: ${desc}`, () => {
        assert.equal(resolveModalKeyAction(input), expected);
    });
}

// Regression lock: the very specific path the user hit. Esc on a form-group
// surface in a dive-mounted editor MUST resolve to 'close' so the parent
// view's editModalRef gets cleared, isBlocking() returns false, and Tab
// cycling at layer 0 keeps working after ascend.
test('regression: esc on dive-editor form-group surface closes (not noop)', () => {
    const action = resolveModalKeyAction({
        key: 'Escape',
        ctrlKey: false,
        isEditing: false,    // div.form-group is the focused surface, not an input
        hasOnClose: true,    // useHotkeys passes closeAndExit as onClose
    });
    assert.equal(action, 'close',
        'Esc must call onClose so editModal clears and Tab cycling resumes after ascend');
});
