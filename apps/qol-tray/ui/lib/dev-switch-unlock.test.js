import { test, beforeEach } from 'node:test';
import assert from 'node:assert/strict';
import { createUnlockTracker, STORAGE_KEY } from './dev-switch-unlock.js';

function makeStorage(initial = null) {
    let value = initial;
    return {
        getItem: (k) => (k === STORAGE_KEY ? value : null),
        setItem: (k, v) => { if (k === STORAGE_KEY) value = String(v); },
        removeItem: (k) => { if (k === STORAGE_KEY) value = null; },
    };
}

let now = 0;
const next = (delta = 0) => { now += delta; return now; };

beforeEach(() => { now = 1_000_000; });

test('reveal: 7 clicks within 2s flips the flag', () => {
    const t = createUnlockTracker({ now: () => now, storage: makeStorage() });
    for (let i = 0; i < 6; i++) { t.bumpClick(); next(100); }
    assert.equal(t.isRevealed(), false);
    t.bumpClick();
    assert.equal(t.isRevealed(), true);
});

test('reveal: 6 clicks within 2s does not flip', () => {
    const t = createUnlockTracker({ now: () => now, storage: makeStorage() });
    for (let i = 0; i < 6; i++) { t.bumpClick(); next(100); }
    assert.equal(t.isRevealed(), false);
});

test('reveal: clicks outside the 2s window are pruned', () => {
    const t = createUnlockTracker({ now: () => now, storage: makeStorage() });
    for (let i = 0; i < 6; i++) { t.bumpClick(); next(100); }
    next(3000);
    t.bumpClick();
    assert.equal(t.isRevealed(), false);
});

test('reveal: 7 clicks across exactly the window edge - the oldest is pruned', () => {
    const t = createUnlockTracker({ now: () => now, storage: makeStorage() });
    t.bumpClick();
    next(2100);
    for (let i = 0; i < 6; i++) { t.bumpClick(); next(100); }
    assert.equal(t.isRevealed(), false);
});

test('reveal: typing d-e-v flips the flag', () => {
    const t = createUnlockTracker({ now: () => now, storage: makeStorage() });
    t.feedKey('d');
    t.feedKey('e');
    t.feedKey('v');
    assert.equal(t.isRevealed(), true);
});

test('reveal: typing D-E-V (uppercase) flips the flag', () => {
    const t = createUnlockTracker({ now: () => now, storage: makeStorage() });
    t.feedKey('D');
    t.feedKey('E');
    t.feedKey('V');
    assert.equal(t.isRevealed(), true);
});

test('reveal: typing d-e-x resets and does not flip', () => {
    const t = createUnlockTracker({ now: () => now, storage: makeStorage() });
    t.feedKey('d');
    t.feedKey('e');
    t.feedKey('x');
    assert.equal(t.isRevealed(), false);
});

test('reveal: typing d-d resets buffer to start over with the second d', () => {
    const t = createUnlockTracker({ now: () => now, storage: makeStorage() });
    t.feedKey('d');
    t.feedKey('d');
    t.feedKey('e');
    t.feedKey('v');
    assert.equal(t.isRevealed(), true);
});

test('reveal: gap > 1.5s between strokes resets the buffer', () => {
    const t = createUnlockTracker({ now: () => now, storage: makeStorage() });
    t.feedKey('d');
    t.feedKey('e');
    next(2000);
    t.feedKey('v');
    assert.equal(t.isRevealed(), false);
});

test('reveal: noise keys (modifiers, arrows, multi-char keys) are ignored', () => {
    const t = createUnlockTracker({ now: () => now, storage: makeStorage() });
    t.feedKey('Shift');
    t.feedKey('ArrowDown');
    t.feedKey('Tab');
    assert.equal(t.isRevealed(), false);
    t.feedKey('d');
    t.feedKey('e');
    t.feedKey('v');
    assert.equal(t.isRevealed(), true);
});

test('reveal: non-string and empty inputs are ignored', () => {
    const t = createUnlockTracker({ now: () => now, storage: makeStorage() });
    t.feedKey(null);
    t.feedKey(undefined);
    t.feedKey('');
    t.feedKey(42);
    assert.equal(t.isRevealed(), false);
});

test('persisted: tracker reads localStorage on init', () => {
    const t = createUnlockTracker({ now: () => now, storage: makeStorage('1') });
    assert.equal(t.isRevealed(), true);
});

test('persisted: reveal writes to storage', () => {
    const storage = makeStorage();
    const t = createUnlockTracker({ now: () => now, storage });
    t.feedKey('d'); t.feedKey('e'); t.feedKey('v');
    assert.equal(storage.getItem(STORAGE_KEY), '1');
});

test('subscribe: listener fires on reveal', () => {
    const t = createUnlockTracker({ now: () => now, storage: makeStorage() });
    let observed = null;
    t.subscribe(v => { observed = v; });
    t.feedKey('d'); t.feedKey('e'); t.feedKey('v');
    assert.equal(observed, true);
});

test('subscribe: listener does NOT fire on noisy input', () => {
    const t = createUnlockTracker({ now: () => now, storage: makeStorage() });
    let calls = 0;
    t.subscribe(() => { calls++; });
    t.bumpClick();
    t.feedKey('d');
    t.feedKey('x');
    assert.equal(calls, 0);
});

test('once revealed, further input is a no-op', () => {
    const t = createUnlockTracker({ now: () => now, storage: makeStorage('1') });
    let calls = 0;
    t.subscribe(() => { calls++; });
    t.feedKey('d');
    t.bumpClick();
    assert.equal(calls, 0);
    assert.equal(t.isRevealed(), true);
});
