import { test } from 'node:test';
import assert from 'node:assert/strict';

const handlers = {};
const attrs = new Map();
const body = {
    dataset: {},
    setAttribute: (key, value) => attrs.set(key, value),
    removeAttribute: (key) => attrs.delete(key),
    hasAttribute: (key) => attrs.has(key),
};

globalThis.document = { body };
globalThis.window = {
    addEventListener: (type, fn) => { (handlers[type] ||= []).push(fn); },
};

const { getModifierState, subscribeModifiers, isCtrlHeld, subscribeCtrl } =
    await import('./modifier-state.js');

function fire(type, key) {
    for (const fn of handlers[type] || []) fn({ key });
}

function reset() {
    for (const fn of handlers.blur || []) fn();
}

const KEY_CASES = [
    ['Control', 'ctrl'],
    ['Shift', 'shift'],
    ['Alt', 'alt'],
    ['Meta', 'meta'],
];

for (const [key, field] of KEY_CASES) {
    test(`${key} keydown/keyup toggles ${field}`, () => {
        reset();
        fire('keydown', key);
        assert.equal(getModifierState()[field], true, `${field} after keydown`);
        fire('keyup', key);
        assert.equal(getModifierState()[field], false, `${field} after keyup`);
    });
}

test('ctrl writes body.dataset.ctrlHeld, shift writes data-shift-held', () => {
    reset();
    fire('keydown', 'Control');
    assert.equal(body.dataset.ctrlHeld, '', 'ctrlHeld attr present while held');
    fire('keydown', 'Shift');
    assert.equal(body.hasAttribute('data-shift-held'), true, 'shift attr present while held');
    fire('keyup', 'Control');
    assert.equal('ctrlHeld' in body.dataset, false, 'ctrlHeld attr removed on release');
    fire('keyup', 'Shift');
    assert.equal(body.hasAttribute('data-shift-held'), false, 'shift attr removed on release');
});

test('blur clears every modifier and its body attrs', () => {
    reset();
    fire('keydown', 'Control');
    fire('keydown', 'Shift');
    fire('blur');
    const state = getModifierState();
    assert.deepEqual(state, { ctrl: false, shift: false, alt: false, meta: false });
    assert.equal('ctrlHeld' in body.dataset, false, 'ctrlHeld cleared on blur');
    assert.equal(body.hasAttribute('data-shift-held'), false, 'shift cleared on blur');
});

test('subscribeModifiers notifies with a fresh snapshot reference on change only', () => {
    reset();
    const seen = [];
    const unsubscribe = subscribeModifiers(() => seen.push(getModifierState()));
    const before = getModifierState();
    fire('keydown', 'Control');
    fire('keydown', 'Control');
    unsubscribe();
    fire('keydown', 'Shift');
    assert.equal(seen.length, 1, 'one notification for one real change, none for the repeat');
    assert.notEqual(seen[0], before, 'snapshot reference changes on mutation');
    assert.equal(seen[0].ctrl, true);
});

test('subscribeCtrl emits the bool on ctrl flips only, not other modifiers', () => {
    reset();
    const flips = [];
    const unsubscribe = subscribeCtrl((held) => flips.push(held));
    fire('keydown', 'Shift');
    fire('keydown', 'Control');
    fire('keyup', 'Control');
    unsubscribe();
    assert.deepEqual(flips, [true, false], 'only ctrl down/up, shift ignored');
    assert.equal(isCtrlHeld(), false, 'isCtrlHeld mirrors the snapshot');
});
