import { test } from 'node:test';
import assert from 'node:assert/strict';
import { busyActionLabel } from './summary.js';

const TABLE = [
    { id: 'connect', idle: '', busy: 'Connecting…' },
    { id: 'pull', idle: 'Pull Now', busy: 'Pulling…' },
    { id: 'push', idle: 'Push Now', busy: 'Pushing…' },
    { id: 'disconnect', idle: 'Disconnect', busy: 'Disconnecting…' },
    { id: 'acknowledge', idle: 'Acknowledge', busy: 'Acknowledging…' },
    { id: 'export', idle: 'Export', busy: 'Exporting…' },
    { id: 'import', idle: 'Import', busy: 'Importing…' },
];

for (const { id, idle, busy } of TABLE) {
    test(`busyActionLabel ${id} idle returns idle label`, () => {
        assert.equal(busyActionLabel(id, false), idle);
    });
    test(`busyActionLabel ${id} busy returns busy label with ellipsis`, () => {
        assert.equal(busyActionLabel(id, true), busy);
    });
}

test('busyActionLabel unknown action returns empty string', () => {
    assert.equal(busyActionLabel('unknown-action', true), '');
    assert.equal(busyActionLabel('unknown-action', false), '');
    assert.equal(busyActionLabel('', true), '');
    assert.equal(busyActionLabel(null, false), '');
    assert.equal(busyActionLabel(undefined, true), '');
});
