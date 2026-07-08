import { test } from 'node:test';
import assert from 'node:assert/strict';
import { visibleRowAction, firstActionableRow } from './row-action.js';

test('visibleRowAction gates on the when key', () => {
    const cases = [
        ['no row_action', null, { fixable: true }, null],
        ['missing action name', { label: 'Fix' }, { fixable: true }, null],
        ['no when renders always', { action: 'a' }, {}, { action: 'a', label: 'Run' }],
        ['when truthy renders', { action: 'a', label: 'Fix', when: 'fixable' }, { fixable: true }, { action: 'a', label: 'Fix' }],
        ['when falsy hides', { action: 'a', when: 'fixable' }, { fixable: false }, null],
        ['when key absent hides', { action: 'a', when: 'fixable' }, {}, null],
        ['null row hides gated action', { action: 'a', when: 'fixable' }, null, null],
    ];
    for (const [label, rowAction, row, expected] of cases) {
        assert.deepEqual(visibleRowAction(rowAction, row), expected, `case: ${label}`);
    }
});

test('firstActionableRow returns the first row passing the gate', () => {
    const rowAction = { action: 'apply_fixes', when: 'fixable' };
    const rows = [{ name: 'a', fixable: false }, { name: 'b', fixable: true }, { name: 'c', fixable: true }];
    assert.equal(firstActionableRow(rowAction, rows)?.name, 'b');
    assert.equal(firstActionableRow(rowAction, []), null);
    assert.equal(firstActionableRow(rowAction, null), null);
});
