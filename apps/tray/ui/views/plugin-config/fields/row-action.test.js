import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
    visibleRowAction,
    visibleFieldRowAction,
    visibleFieldRowActions,
    rowActionInput,
} from './row-action.js';

test('visibleRowAction gates on the when key', () => {
    const cases = [
        ['no row_action', null, { fixable: true }, null],
        ['missing action name', { label: 'Fix' }, { fixable: true }, null],
        ['no when renders always', { action: 'a' }, {}, { action: 'a', label: 'Run' }],
        [
            'when truthy preserves the complete descriptor',
            { action: 'a', input: { address: '{address}' }, key: 'Enter', label: 'Fix', when: 'fixable' },
            { address: 'AA:BB', fixable: true },
            { action: 'a', input: { address: '{address}' }, key: 'Enter', label: 'Fix', when: 'fixable' },
        ],
        ['when falsy hides', { action: 'a', when: 'fixable' }, { fixable: false }, null],
        ['when key absent hides', { action: 'a', when: 'fixable' }, {}, null],
        ['null row hides gated action', { action: 'a', when: 'fixable' }, null, null],
    ];
    for (const [label, rowAction, row, expected] of cases) {
        assert.deepEqual(visibleRowAction(rowAction, row), expected, `case: ${label}`);
    }
});

test('visibleFieldRowAction selects the action matching row state', () => {
    const field = {
        row_actions: [
            { action: 'pair_device', label: 'Pair', when: 'can_pair' },
            { action: 'connect_device', label: 'Connect', when: 'can_connect' },
            { action: 'disconnect_device', label: 'Disconnect', when: 'can_disconnect' },
        ],
    };
    const cases = [
        ['available', { can_pair: true }, 'pair_device'],
        ['paired', { can_connect: true }, 'connect_device'],
        ['connected', { can_disconnect: true }, 'disconnect_device'],
        ['no action', {}, null],
    ];
    for (const [label, row, expected] of cases) {
        assert.equal(visibleFieldRowAction(field, row)?.action ?? null, expected, `case: ${label}`);
    }
});

test('visibleFieldRowActions preserves every applicable action in contract order', () => {
    const field = {
        row_actions: [
            { action: 'connect_device', label: 'Connect', when: 'can_connect' },
            { action: 'trust_device', label: 'Trust', when: 'can_trust' },
            { action: 'remove_device', label: 'Remove', when: 'can_remove' },
        ],
    };
    assert.deepEqual(
        visibleFieldRowActions(field, { can_connect: true, can_trust: false, can_remove: true })
            .map(action => action.action),
        ['connect_device', 'remove_device'],
    );
});

test('rowActionInput materializes typed row values from templates', () => {
    const input = rowActionInput(
        { input: { address: '{address}', label: 'Pair {name}', missing: '{missing}' } },
        { address: 'AA:BB', name: 'Luna 2' },
    );
    assert.deepEqual(input, {
        address: 'AA:BB',
        label: 'Pair Luna 2',
        missing: null,
    });
});

test('selected row action retains its materialized address', () => {
    const field = {
        row_actions: [
            {
                action: 'pair_device',
                input: { address: '{address}' },
                label: 'Pair & connect',
                when: 'can_pair',
            },
        ],
    };
    const row = { address: 'AA:BB:CC:DD:EE:FF', can_pair: true };
    assert.deepEqual(rowActionInput(visibleFieldRowAction(field, row), row), {
        address: 'AA:BB:CC:DD:EE:FF',
    });
});
