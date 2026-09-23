import { test } from 'node:test';
import assert from 'node:assert/strict';
import { buildShortcutPrefill } from './prefill.js';

test('url prefill (default type)', () => {
    const m = buildShortcutPrefill({ url: 'https://x.io', name: 'X' });
    assert.equal(m.editing, false);
    assert.equal(m.shortcut.name, 'X');
    assert.deepEqual(m.shortcut.action, { type: 'open_url', url: 'https://x.io' });
    assert.equal(m.shortcut.enabled, true);
    assert.equal(m.shortcut.export_to_launcher, true);
    assert.ok(!('id' in m.shortcut));
});

test('app prefill with explicit ref type', () => {
    const m = buildShortcutPrefill({ type: 'app', app_type: 'bundle_id', app: 'com.apple.Safari', name: 'Safari' });
    assert.deepEqual(m.shortcut.action, { type: 'launch_app', app: { type: 'bundle_id', id: 'com.apple.Safari' } });
});

test('app prefill defaults ref type to name', () => {
    const m = buildShortcutPrefill({ type: 'app', app: 'Safari' });
    assert.deepEqual(m.shortcut.action, { type: 'launch_app', app: { type: 'name', name: 'Safari' } });
});

test('missing params yield empty fields, never throw', () => {
    const m = buildShortcutPrefill({});
    assert.deepEqual(m.shortcut.action, { type: 'open_url', url: '' });
    assert.equal(m.shortcut.name, '');
});
