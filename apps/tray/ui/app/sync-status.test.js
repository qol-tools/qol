import assert from 'node:assert/strict';
import test from 'node:test';
import { syncStatusesEqual } from './sync-status.js';

test('syncStatusesEqual accepts equivalent nested API payloads', () => {
    const left = {
        configured: true,
        incident: { message: 'restore available', files: ['one', 'two'] },
        backup_count: 2,
    };
    const right = {
        backup_count: 2,
        incident: { files: ['one', 'two'], message: 'restore available' },
        configured: true,
    };

    assert.equal(syncStatusesEqual(left, right), true);
});

test('syncStatusesEqual rejects nested status changes', () => {
    const current = { health: 'ready', incident: null, backups: ['one'] };

    assert.equal(syncStatusesEqual(current, { ...current, health: 'error' }), false);
    assert.equal(syncStatusesEqual(current, { ...current, incident: {} }), false);
    assert.equal(syncStatusesEqual(current, { ...current, backups: ['two'] }), false);
});
