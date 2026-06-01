import { test } from 'node:test';
import assert from 'node:assert/strict';
import { resolveDeepLink } from './deeplink-resolve.js';

function fakeDeps() {
    const calls = { setPending: [] };
    return { calls, setPendingShortcutPrefill: (p) => calls.setPending.push(p) };
}

test('page-only route stashes nothing (view router handles the page)', () => {
    const d = fakeDeps();
    const handled = resolveDeepLink({ page: 'hotkeys', action: null, params: {} }, d);
    assert.equal(handled, false);
    assert.equal(d.calls.setPending.length, 0);
});

test('shortcuts/add stashes a prefill', () => {
    const d = fakeDeps();
    const handled = resolveDeepLink({ page: 'shortcuts', action: 'add', params: { url: 'https://x.io', name: 'X' } }, d);
    assert.equal(handled, true);
    assert.equal(d.calls.setPending.length, 1);
    assert.equal(d.calls.setPending[0].shortcut.action.url, 'https://x.io');
});

test('null page does nothing', () => {
    const d = fakeDeps();
    assert.equal(resolveDeepLink({ page: null, action: null, params: {} }, d), false);
    assert.equal(d.calls.setPending.length, 0);
});

test('unknown action on a real page does nothing', () => {
    const d = fakeDeps();
    assert.equal(resolveDeepLink({ page: 'shortcuts', action: 'frobnicate', params: {} }, d), false);
    assert.equal(d.calls.setPending.length, 0);
});
