import { test } from 'node:test';
import assert from 'node:assert/strict';
import { applyRecordingKey } from './recorder.js';

// ---------------------------------------------------------------------------
// Pure recorder logic: applyRecordingKey takes the in-progress modal state plus
// a synthetic KeyboardEvent and returns { modal, advance }. These tests lock
// the canonical shortcut format used in profile/core/hotkeys.json — modifier
// order Ctrl → Alt → Shift → Super → key, joined with `+`.
// ---------------------------------------------------------------------------

const baseModal = { key: '', recording: true };

function ev(overrides = {}) {
    return {
        ctrlKey: false,
        altKey: false,
        shiftKey: false,
        metaKey: false,
        ...overrides,
    };
}

// ---------------------------------------------------------------------------
// Special keys must be recordable, including the keys browsers steal.
// ---------------------------------------------------------------------------

const RECORDABLE_SPECIAL_KEYS = [
    { name: 'Tab',          event: ev({ key: 'Tab', code: 'Tab' }),                                       expected: 'Tab' },
    { name: 'Alt+Shift+Tab', event: ev({ key: 'Tab', code: 'Tab', altKey: true, shiftKey: true }),        expected: 'Alt+Shift+Tab' },
    { name: 'Ctrl+Tab',     event: ev({ key: 'Tab', code: 'Tab', ctrlKey: true }),                       expected: 'Ctrl+Tab' },
    { name: 'Backspace',    event: ev({ key: 'Backspace', code: 'Backspace' }),                          expected: 'Backspace' },
    { name: 'Delete',       event: ev({ key: 'Delete', code: 'Delete' }),                                expected: 'Delete' },
    { name: 'Enter',        event: ev({ key: 'Enter', code: 'Enter' }),                                  expected: 'Enter' },
    { name: 'Space',        event: ev({ key: ' ', code: 'Space' }),                                      expected: 'Space' },
    { name: 'ArrowDown',    event: ev({ key: 'ArrowDown', code: 'ArrowDown' }),                          expected: 'Down' },
    { name: 'F5',           event: ev({ key: 'F5', code: 'F5' }),                                        expected: 'F5' },
    { name: 'Letter',       event: ev({ key: 'a', code: 'KeyA' }),                                       expected: 'A' },
    { name: 'Digit',        event: ev({ key: '5', code: 'Digit5' }),                                     expected: '5' },
];

for (const row of RECORDABLE_SPECIAL_KEYS) {
    test(`applyRecordingKey records ${row.name} as ${row.expected}`, () => {
        const result = applyRecordingKey(baseModal, row.event);
        assert.equal(result.modal.key, row.expected, `expected ${row.expected} for ${row.name}`);
        assert.equal(result.modal.recording, false, 'should exit recording on a complete capture');
        assert.equal(result.advance, true, 'should advance to next field after capture');
    });
}

// ---------------------------------------------------------------------------
// Escape must cancel recording without writing a key — it is the cancel signal,
// not a recordable shortcut.
// ---------------------------------------------------------------------------

test('applyRecordingKey: Escape cancels recording without changing key', () => {
    const seeded = { key: 'Alt+F1', recording: true };
    const result = applyRecordingKey(seeded, ev({ key: 'Escape', code: 'Escape' }));
    assert.equal(result.modal.recording, false);
    assert.equal(result.modal.key, 'Alt+F1', 'Escape must not overwrite a previously recorded key');
    assert.equal(result.advance, false);
});

// ---------------------------------------------------------------------------
// Pressing only a modifier shows the partial chord but does not advance.
// ---------------------------------------------------------------------------

test('applyRecordingKey: lone Alt shows partial chord, stays recording', () => {
    const result = applyRecordingKey(baseModal, ev({ key: 'Alt', code: 'AltLeft', altKey: true }));
    assert.equal(result.modal.key, 'Alt');
    assert.equal(result.modal.recording, true);
    assert.equal(result.advance, false);
});

test('applyRecordingKey: lone Shift+Ctrl shows partial chord, stays recording', () => {
    const result = applyRecordingKey(baseModal, ev({
        key: 'Shift', code: 'ShiftLeft', ctrlKey: true, shiftKey: true,
    }));
    assert.equal(result.modal.key, 'Ctrl+Shift');
    assert.equal(result.modal.recording, true);
    assert.equal(result.advance, false);
});

// ---------------------------------------------------------------------------
// Modifier-only shortcuts (no concrete key) must not commit.
// ---------------------------------------------------------------------------

test('applyRecordingKey: unknown code with only modifiers does not advance', () => {
    const result = applyRecordingKey(baseModal, ev({
        key: 'Unidentified', code: 'Unidentified', altKey: true,
    }));
    assert.equal(result.modal, baseModal, 'state must be unchanged on a non-recordable event');
    assert.equal(result.advance, false);
});

// ---------------------------------------------------------------------------
// Property: across every combination of modifiers + a recordable key, the
// emitted shortcut must (a) preserve canonical modifier order Ctrl→Alt→Shift→
// Super→key, (b) join with `+`, (c) end in the key, (d) drop disabled mods.
// ---------------------------------------------------------------------------

const MODIFIER_ORDER = ['Ctrl', 'Alt', 'Shift', 'Super'];
const TERMINAL_CASES = [
    { key: 'a',         code: 'KeyA',     expected: 'A' },
    { key: 'Tab',       code: 'Tab',      expected: 'Tab' },
    { key: 'Backspace', code: 'Backspace', expected: 'Backspace' },
    { key: 'F12',       code: 'F12',      expected: 'F12' },
    { key: 'ArrowLeft', code: 'ArrowLeft', expected: 'Left' },
];

test('property: all 16 modifier combos × 5 keys produce canonical ordering', () => {
    for (let mask = 0; mask < 16; mask++) {
        const ctrlKey = Boolean(mask & 1);
        const altKey = Boolean(mask & 2);
        const shiftKey = Boolean(mask & 4);
        const metaKey = Boolean(mask & 8);
        for (const term of TERMINAL_CASES) {
            const result = applyRecordingKey(baseModal, ev({
                key: term.key, code: term.code, ctrlKey, altKey, shiftKey, metaKey,
            }));
            const parts = result.modal.key.split('+');
            const tail = parts.pop();
            assert.equal(tail, term.expected,
                `key suffix mismatch for mask=${mask} key=${term.key}: ${result.modal.key}`);

            const expectedMods = MODIFIER_ORDER.filter((_, i) => Boolean(mask & (1 << i)));
            assert.deepEqual(parts, expectedMods,
                `modifier order mismatch for mask=${mask} key=${term.key}: ${result.modal.key}`);

            assert.equal(result.modal.recording, false);
            assert.equal(result.advance, true);
        }
    }
});

// ---------------------------------------------------------------------------
// macOS dead-keys: Option (Alt) + letter on a US keyboard layout produces a
// composed character in `event.key` (e.g. Alt+Q → Œ, Alt+E → é). The recorder
// must derive the terminal key from `event.code` (the physical key, e.g.
// 'KeyQ') so the saved shortcut is `Alt+Shift+Q`, not `Alt+Shift+Œ`.
// ---------------------------------------------------------------------------

const MACOS_DEAD_KEYS = [
    {
        name: 'Alt+Shift+Q on macOS produces Œ in event.key but KeyQ in event.code',
        event: ev({ key: 'Œ', code: 'KeyQ', altKey: true, shiftKey: true }),
        expected: 'Alt+Shift+Q',
    },
    {
        name: 'Alt+E on macOS produces é in event.key but KeyE in event.code',
        event: ev({ key: 'é', code: 'KeyE', altKey: true }),
        expected: 'Alt+E',
    },
    {
        name: 'Alt+N on macOS produces ˜ in event.key but KeyN in event.code',
        event: ev({ key: '˜', code: 'KeyN', altKey: true }),
        expected: 'Alt+N',
    },
    {
        name: 'Ctrl+Alt+Shift+5 commits with all modifiers and digit',
        event: ev({ key: '%', code: 'Digit5', ctrlKey: true, altKey: true, shiftKey: true }),
        expected: 'Ctrl+Alt+Shift+5',
    },
    {
        name: 'Alt+Shift+Tab still records as Alt+Shift+Tab on any layout',
        event: ev({ key: 'Tab', code: 'Tab', altKey: true, shiftKey: true }),
        expected: 'Alt+Shift+Tab',
    },
];

for (const row of MACOS_DEAD_KEYS) {
    test(`applyRecordingKey: ${row.name}`, () => {
        const result = applyRecordingKey(baseModal, row.event);
        assert.equal(result.modal.key, row.expected,
            `expected ${row.expected}, got ${result.modal.key}`);
        assert.equal(result.modal.recording, false);
        assert.equal(result.advance, true);
    });
}
