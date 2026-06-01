import { test } from 'node:test';
import assert from 'node:assert/strict';
import { setPendingShortcutPrefill, takePendingShortcutPrefill, subscribeShortcutPrefill } from './deeplink-intent.js';

test('take returns null when nothing pending', () => {
    assert.equal(takePendingShortcutPrefill(), null);
});

test('set then take returns the value once', () => {
    const v = { editing: false, shortcut: { name: 'X' } };
    setPendingShortcutPrefill(v);
    assert.equal(takePendingShortcutPrefill(), v);
    assert.equal(takePendingShortcutPrefill(), null); // cleared
});

test('set notifies subscribers so a late-arriving prefill is consumed', () => {
    let notified = 0;
    const unsub = subscribeShortcutPrefill(() => { notified++; });
    setPendingShortcutPrefill({ editing: false, shortcut: { name: 'Y' } });
    assert.equal(notified, 1, 'subscriber fires on set');
    assert.equal(takePendingShortcutPrefill().shortcut.name, 'Y');
    unsub();
    setPendingShortcutPrefill({ editing: false, shortcut: { name: 'Z' } });
    assert.equal(notified, 1, 'no notify after unsubscribe');
    takePendingShortcutPrefill();
});
