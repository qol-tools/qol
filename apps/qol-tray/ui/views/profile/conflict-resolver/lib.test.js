import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
    allPicked,
    buildPicks,
    formatValue,
    leafKey,
    nextIndex,
    relativeTime,
    summarize,
    toChoices,
} from './lib.js';

test('summarize counts mine/remote and ignores nulls', () => {
    const cases = [
        { picks: [], expected: { keptMine: 0, tookRemote: 0 } },
        { picks: ['mine'], expected: { keptMine: 1, tookRemote: 0 } },
        { picks: ['remote'], expected: { keptMine: 0, tookRemote: 1 } },
        { picks: [null, 'mine', 'remote', null, 'mine'], expected: { keptMine: 2, tookRemote: 1 } },
        { picks: ['mine', 'mine', 'remote'], expected: { keptMine: 2, tookRemote: 1 } },
    ];
    for (const { picks, expected } of cases) {
        assert.deepEqual(summarize(picks), expected, `picks=${JSON.stringify(picks)}`);
    }
});

test('allPicked returns false when empty', () => {
    assert.equal(allPicked([]), false);
});

test('allPicked is true only when every entry is mine or remote', () => {
    const cases = [
        { picks: ['mine'], expected: true },
        { picks: ['remote'], expected: true },
        { picks: ['mine', 'remote'], expected: true },
        { picks: ['mine', null], expected: false },
        { picks: [null, null], expected: false },
        { picks: ['mine', 'other'], expected: false },
    ];
    for (const { picks, expected } of cases) {
        assert.equal(allPicked(picks), expected, `picks=${JSON.stringify(picks)}`);
    }
});

test('formatValue renders primitives and objects safely', () => {
    const cases = [
        [null, 'null'],
        [undefined, 'null'],
        ['hello', '"hello"'],
        [42, '42'],
        [true, 'true'],
        [false, 'false'],
        [[1, 2, 3], '[1,2,3]'],
        [{ a: 1 }, '{"a":1}'],
    ];
    for (const [input, expected] of cases) {
        assert.equal(formatValue(input), expected, `value=${JSON.stringify(input)}`);
    }
});

test('relativeTime returns sensible labels at boundary points', () => {
    const now = Date.parse('2026-06-23T12:00:00Z');
    const cases = [
        [null, 'unknown time'],
        [undefined, 'unknown time'],
        ['not-a-date', 'unknown time'],
        ['2026-06-23T11:59:30Z', 'just now'],
        ['2026-06-23T11:30:00Z', 'edited 30 minutes ago'],
        ['2026-06-23T10:00:00Z', 'edited 2 hours ago'],
        ['2026-06-22T12:00:00Z', 'edited 1 day ago'],
        ['2026-06-21T12:00:00Z', 'edited 2 days ago'],
        ['2026-06-23T13:00:00Z', 'in the future'],
    ];
    for (const [input, expected] of cases) {
        assert.equal(relativeTime(input, now), expected, `input=${input}`);
    }
});

test('buildPicks returns null per conflict', () => {
    assert.deepEqual(buildPicks([]), []);
    assert.deepEqual(buildPicks([{}, {}, {}]), [null, null, null]);
});

test('nextIndex clamps to [0, total - 1]', () => {
    const cases = [
        { index: 0, total: 3, dir: -1, expected: 0 },
        { index: 0, total: 3, dir: 1, expected: 1 },
        { index: 2, total: 3, dir: 1, expected: 2 },
        { index: 1, total: 3, dir: -1, expected: 0 },
        { index: 0, total: 0, dir: 1, expected: 0 },
    ];
    for (const { index, total, dir, expected } of cases) {
        assert.equal(nextIndex(index, total, dir), expected, `i=${index} total=${total} dir=${dir}`);
    }
});

test('toChoices skips null picks and yields snake_case keys', () => {
    const conflicts = [
        { file: 'a.json', key_path: 'opacity', local: 0.8, remote: 0.5 },
        { file: 'b.json', key_path: 'theme', local: 'warm', remote: 'cool' },
        { file: 'c.json', key_path: 'x', local: 1, remote: 2 },
    ];
    const picks = ['mine', null, 'remote'];
    assert.deepEqual(toChoices(conflicts, picks), [
        { file: 'a.json', key_path: 'opacity', side: 'mine' },
        { file: 'c.json', key_path: 'x', side: 'remote' },
    ]);
});

test('leafKey returns the dotted-path tail', () => {
    assert.equal(leafKey('opacity'), 'opacity');
    assert.equal(leafKey('win.w'), 'w');
    assert.equal(leafKey('a.b.c'), 'c');
    assert.equal(leafKey(''), '');
    assert.equal(leafKey(undefined), '');
});
