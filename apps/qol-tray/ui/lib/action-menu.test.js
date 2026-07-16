import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
    firstEnabledActionIndex,
    lastEnabledActionIndex,
    nextEnabledActionIndex,
} from './action-menu.js';

test('action menu navigation skips disabled actions and wraps', () => {
    const actions = [
        { id: 'connect', disabled: true },
        { id: 'trust' },
        { id: 'remove', disabled: true },
        { id: 'inspect' },
    ];
    const cases = [
        ['first', firstEnabledActionIndex(actions), 1],
        ['last', lastEnabledActionIndex(actions), 3],
        ['forward skips', nextEnabledActionIndex(actions, 1, 1), 3],
        ['forward wraps', nextEnabledActionIndex(actions, 3, 1), 1],
        ['backward skips', nextEnabledActionIndex(actions, 3, -1), 1],
        ['backward wraps', nextEnabledActionIndex(actions, 1, -1), 3],
        ['empty', nextEnabledActionIndex([], 0, 1), -1],
        ['all disabled', nextEnabledActionIndex([{ disabled: true }], 0, 1), -1],
    ];
    for (const [label, actual, expected] of cases) {
        assert.equal(actual, expected, `case: ${label}`);
    }
});
