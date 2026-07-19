import { test } from 'node:test';
import assert from 'node:assert/strict';
import { extractPath, queryFlag, runtimeActivityLabel } from './query-data.js';

test('extractPath resolves nested query values safely', () => {
    const value = { search: { active: true }, zero: 0 };
    const cases = [
        ['nested', 'search.active', true],
        ['zero', 'zero', 0],
        ['missing', 'search.missing', null],
        ['past primitive', 'zero.value', null],
        ['empty path', '', null],
    ];
    for (const [label, path, expected] of cases) {
        assert.equal(extractPath(value, path), expected, `case: ${label}`);
    }
});

test('queryFlag accepts only explicit wire truth values', () => {
    const cases = [
        [true, true],
        [1, true],
        ['true', true],
        [false, false],
        [0, false],
        ['false', false],
        [null, false],
    ];
    for (const [value, expected] of cases) {
        assert.equal(queryFlag({ value }, 'value'), expected, `value: ${value}`);
    }
});

test('runtimeActivityLabel exposes labels only while active', () => {
    const cases = [
        [{ active_value_from: 'searching', active_label: 'LIVE' }, { searching: true }, 'LIVE'],
        [{ active_value_from: 'searching' }, { searching: 'true' }, 'Live'],
        [{ active_value_from: 'searching', active_label: 'LIVE' }, { searching: false }, null],
        [{}, { searching: true }, null],
    ];
    for (const [field, value, expected] of cases) {
        assert.equal(runtimeActivityLabel(field, value), expected);
    }
});
