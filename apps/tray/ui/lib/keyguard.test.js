import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
    BROWSER_RESERVED_CHORDS,
    isReservedChord,
    isKeyguardActive,
    pauseKeyguard,
    resumeKeyguard,
    createKeyguardPause,
} from './keyguard.js';

function ev({ key, ctrl = false, meta = false, shift = false, alt = false }) {
    return { key, ctrlKey: ctrl, metaKey: meta, shiftKey: shift, altKey: alt };
}

test('reserves Ctrl+R and Cmd+R', () => {
    assert.equal(isReservedChord(ev({ key: 'r', ctrl: true })), true);
    assert.equal(isReservedChord(ev({ key: 'R', ctrl: true })), true);
    assert.equal(isReservedChord(ev({ key: 'r', meta: true })), true);
});

test('reserves Ctrl+Shift+R (hard reload)', () => {
    assert.equal(isReservedChord(ev({ key: 'R', ctrl: true, shift: true })), true);
    assert.equal(isReservedChord(ev({ key: 'r', meta: true, shift: true })), true);
});

test('reserves F5 alone, Ctrl+F5 (hard reload), Shift+F5, and F12 alone', () => {
    assert.equal(isReservedChord(ev({ key: 'F5' })), true);
    assert.equal(isReservedChord(ev({ key: 'F5', ctrl: true })), true);
    assert.equal(isReservedChord(ev({ key: 'F5', meta: true })), true);
    assert.equal(isReservedChord(ev({ key: 'F5', shift: true })), true);
    assert.equal(isReservedChord(ev({ key: 'F12' })), true);
});

test('reserves devtools chords', () => {
    assert.equal(isReservedChord(ev({ key: 'I', ctrl: true, shift: true })), true);
    assert.equal(isReservedChord(ev({ key: 'J', meta: true, shift: true })), true);
    assert.equal(isReservedChord(ev({ key: 'C', ctrl: true, shift: true })), true);
});

test('reserves zoom chords (Ctrl+0, Ctrl+=, Ctrl++, Ctrl+-)', () => {
    assert.equal(isReservedChord(ev({ key: '0', ctrl: true })), true);
    assert.equal(isReservedChord(ev({ key: '=', ctrl: true })), true);
    assert.equal(isReservedChord(ev({ key: '+', ctrl: true, shift: true })), true);
    assert.equal(isReservedChord(ev({ key: '-', meta: true })), true);
});

test('reserves Ctrl+F, Ctrl+W, Ctrl+P', () => {
    assert.equal(isReservedChord(ev({ key: 'f', ctrl: true })), true);
    assert.equal(isReservedChord(ev({ key: 'w', meta: true })), true);
    assert.equal(isReservedChord(ev({ key: 'p', ctrl: true })), true);
});

test('does NOT reserve plain r (no modifier)', () => {
    assert.equal(isReservedChord(ev({ key: 'r' })), false);
    assert.equal(isReservedChord(ev({ key: 'R' })), false);
});

test('does NOT reserve Ctrl+Alt+F / Ctrl+Shift+F / Ctrl+Alt+P (extra modifiers)', () => {
    assert.equal(isReservedChord(ev({ key: 'f', ctrl: true, alt: true })), false);
    assert.equal(isReservedChord(ev({ key: 'f', ctrl: true, shift: true })), false);
    assert.equal(isReservedChord(ev({ key: 'p', ctrl: true, alt: true })), false);
});

test('does NOT reserve Ctrl+Alt+R (Ctrl+R requires shift-or-bare meta, not alt)', () => {
    assert.equal(isReservedChord(ev({ key: 'r', ctrl: true, alt: true })), false);
});

test('does NOT reserve Alt+F5 (F5 specs are no-mod / meta-only / shift-only)', () => {
    assert.equal(isReservedChord(ev({ key: 'F5', alt: true })), false);
});

test('does NOT reserve Ctrl+Shift+0 (zoom-reset is meta-only, no shift)', () => {
    assert.equal(isReservedChord(ev({ key: '0', ctrl: true, shift: true })), false);
});

test('does NOT reserve Ctrl+E (palette), Ctrl+Enter (modal save), Tab, Esc, arrows', () => {
    assert.equal(isReservedChord(ev({ key: 'e', ctrl: true })), false);
    assert.equal(isReservedChord(ev({ key: 'Enter', ctrl: true })), false);
    assert.equal(isReservedChord(ev({ key: 'Tab' })), false);
    assert.equal(isReservedChord(ev({ key: 'Escape' })), false);
    assert.equal(isReservedChord(ev({ key: 'ArrowDown' })), false);
});

test('does NOT reserve Shift+! (fit-all binding)', () => {
    assert.equal(isReservedChord(ev({ key: '!', shift: true })), false);
    assert.equal(isReservedChord(ev({ key: '1', shift: true })), false);
});

test('reserved set is frozen', () => {
    assert.throws(() => { BROWSER_RESERVED_CHORDS[0] = { key: 'x' }; });
    assert.throws(() => { BROWSER_RESERVED_CHORDS[0].key = 'x'; });
});

test('handles missing event.key gracefully', () => {
    assert.equal(isReservedChord({ ctrlKey: true }), false);
    assert.equal(isReservedChord({}), false);
});

test('pauseKeyguard deactivates guard until resumeKeyguard', () => {
    assert.equal(isKeyguardActive(), true);
    pauseKeyguard();
    assert.equal(isKeyguardActive(), false);
    resumeKeyguard();
    assert.equal(isKeyguardActive(), true);
});

test('pauseKeyguard nests; equal resumes restore', () => {
    pauseKeyguard();
    pauseKeyguard();
    resumeKeyguard();
    assert.equal(isKeyguardActive(), false);
    resumeKeyguard();
    assert.equal(isKeyguardActive(), true);
});

test('resumeKeyguard never underflows', () => {
    resumeKeyguard();
    resumeKeyguard();
    assert.equal(isKeyguardActive(), true);
    pauseKeyguard();
    assert.equal(isKeyguardActive(), false);
    resumeKeyguard();
    assert.equal(isKeyguardActive(), true);
});

test('createKeyguardPause: repeated pause counts as one (idempotent)', () => {
    const p = createKeyguardPause();
    p.pause();
    p.pause();
    p.pause();
    assert.equal(isKeyguardActive(), false);
    p.resume();
    assert.equal(isKeyguardActive(), true);
});

test('createKeyguardPause: resume before any pause is a no-op', () => {
    const p = createKeyguardPause();
    p.resume();
    p.resume();
    assert.equal(isKeyguardActive(), true);
});

test('createKeyguardPause: double-resume on single pause does not underflow other instances', () => {
    const p = createKeyguardPause();
    p.pause();
    p.resume();
    p.resume();
    assert.equal(isKeyguardActive(), true);
    p.pause();
    assert.equal(isKeyguardActive(), false);
    p.resume();
    assert.equal(isKeyguardActive(), true);
});

test('createKeyguardPause: independent instances tracked separately', () => {
    const a = createKeyguardPause();
    const b = createKeyguardPause();
    a.pause();
    b.pause();
    assert.equal(isKeyguardActive(), false);
    a.resume();
    assert.equal(isKeyguardActive(), false);
    b.resume();
    assert.equal(isKeyguardActive(), true);
});

test('createKeyguardPause: lifecycle - start->capture->dispose path leaves depth at zero', () => {
    const p = createKeyguardPause();
    p.pause();
    assert.equal(isKeyguardActive(), false);
    p.resume();
    p.resume();
    assert.equal(isKeyguardActive(), true);
});

test('createKeyguardPause: lifecycle - start->unmount-without-capture path leaves depth at zero', () => {
    const p = createKeyguardPause();
    p.pause();
    assert.equal(isKeyguardActive(), false);
    p.resume();
    assert.equal(isKeyguardActive(), true);
});

test('createKeyguardPause: lifecycle - never-started instance disposed is a no-op', () => {
    const p = createKeyguardPause();
    p.resume();
    assert.equal(isKeyguardActive(), true);
});
