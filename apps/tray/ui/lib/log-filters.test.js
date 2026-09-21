import { test } from 'node:test';
import assert from 'node:assert/strict';
import { patternsFromInput, patternsToInput } from './log-filter-patterns.js';

test('patternsFromInput returns empty array for null/undefined/empty', () => {
    const cases = [null, undefined, '', '   '];
    for (const input of cases) {
        const result = patternsFromInput(input);
        assert.deepEqual(result, [], `input: ${JSON.stringify(input)}`);
    }
});

test('patternsFromInput splits on comma and trims', () => {
    const cases = [
        ['error', ['error']],
        ['error, warn', ['error', 'warn']],
        ['  error  ,  warn  ', ['error', 'warn']],
        ['a,b,c', ['a', 'b', 'c']],
        ['a, , b', ['a', 'b']],
        [',,a,,', ['a']],
        ['error,warn,,deprecated', ['error', 'warn', 'deprecated']],
    ];
    for (const [input, expected] of cases) {
        assert.deepEqual(patternsFromInput(input), expected, `input: ${JSON.stringify(input)}`);
    }
});

test('patternsToInput joins with comma+space', () => {
    const cases = [
        [[], ''],
        [['error'], 'error'],
        [['error', 'warn'], 'error, warn'],
        [['a', 'b', 'c'], 'a, b, c'],
    ];
    for (const [input, expected] of cases) {
        assert.equal(patternsToInput(input), expected, `input: ${JSON.stringify(input)}`);
    }
});

test('patternsToInput handles non-array gracefully', () => {
    const cases = [null, undefined, 'string'];
    for (const input of cases) {
        assert.equal(patternsToInput(input), '', `input: ${JSON.stringify(input)}`);
    }
});

test('patternsFromInput then patternsToInput round-trips when canonical', () => {
    const canonicalInputs = [
        '',
        'error',
        'error, warn',
        'a, b, c',
    ];
    for (const input of canonicalInputs) {
        const patterns = patternsFromInput(input);
        const formatted = patternsToInput(patterns);
        assert.equal(formatted, input, `input: ${JSON.stringify(input)}`);
    }
});

test('patternsToInput then patternsFromInput round-trips arrays of unique non-empty strings', () => {
    const cases = [
        [],
        ['error'],
        ['error', 'warn'],
        ['a', 'b', 'c'],
    ];
    for (const patterns of cases) {
        const formatted = patternsToInput(patterns);
        const parsed = patternsFromInput(formatted);
        assert.deepEqual(parsed, patterns, `patterns: ${JSON.stringify(patterns)}`);
    }
});
