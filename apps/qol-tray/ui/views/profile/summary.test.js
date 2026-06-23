import { test } from 'node:test';
import assert from 'node:assert/strict';
import { busyActionLabel } from './summary.js';

const TABLE = [
    { id: 'connect', idle: '', busy: 'Connecting…' },
    { id: 'pull', idle: 'Pull Now', busy: 'Pulling…' },
    { id: 'push', idle: 'Push Now', busy: 'Pushing…' },
    { id: 'disconnect', idle: 'Disconnect', busy: 'Disconnecting…' },
    { id: 'export', idle: 'Export', busy: 'Exporting…' },
    { id: 'import', idle: 'Import', busy: 'Importing…' },
];

test('busyActionLabel acknowledge returns empty (removed action)', () => {
    assert.equal(busyActionLabel('acknowledge', true), '');
    assert.equal(busyActionLabel('acknowledge', false), '');
});

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
