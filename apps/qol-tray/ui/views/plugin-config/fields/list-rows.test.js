import { test } from 'node:test';
import assert from 'node:assert/strict';
import { interpolate, listItem, rowsFrom } from './list-rows.js';

const rows = [
    { name: 'QuietComfort 45', status: 'Connected', address: 'AA:BB:CC:DD:EE:01', trusted: true },
    { name: 'MX Keys', status: 'Disconnected', address: 'AA:BB:CC:DD:EE:02', trusted: false },
];

test('list row helpers preserve the query payload contract', () => {
    assert.equal(rowsFrom(rows), rows);
    assert.equal(rowsFrom({ items: rows }), rows);
    assert.deepEqual(rowsFrom({ rows }), []);
    assert.equal(interpolate('{name} / {missing}', rows[0]), 'QuietComfort 45 / ');
});

test('pending query rows retain their primary label while activation is disabled', () => {
    const field = {
        row_label: '{name}',
        row_subtitle: '{status}',
        row_actions: [
            { action: 'connect_device', label: 'Connect', when: 'can_connect' },
            { action: 'pair_device', label: 'Pair', when: 'can_pair' },
        ],
    };
    const item = listItem(field, {
        address: '46:68:59:7F:5F:E9',
        name: 'Luna 2',
        status: 'Connecting...',
        action_pending: true,
        can_connect: true,
        can_pair: true,
    }, 0);

    assert.equal(item.actionLabel, 'Connect');
    assert.equal(item.pending, true);
    assert.equal(item.disabled, true);
    assert.deepEqual(item.actions.map(action => action.disabled), [true, true]);
});
