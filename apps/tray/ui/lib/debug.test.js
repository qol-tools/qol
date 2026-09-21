import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createDebug, getTrace, clearTrace, formatTraceArg, setDebugEnabled } from './debug.js';

setDebugEnabled(false);

test('formatTraceArg renders primitives, rects, and truncates long strings', () => {
    const long = 'x'.repeat(250);
    const cases = [
        ['plain string', 'hello', 'hello'],
        ['number', 42, '42'],
        ['boolean', true, 'true'],
        ['null', null, 'null'],
        ['rect-like object', { left: 1, top: 2, width: 30, height: 4 }, '(1,2 30x4)'],
    ];
    for (const [label, input, expected] of cases) {
        assert.equal(formatTraceArg(input), expected, label);
    }
    assert.equal(formatTraceArg(long).length, 201, 'long string truncated to ARG_MAX + ellipsis');
    assert.ok(formatTraceArg(long).endsWith('…'), 'truncated string ends with ellipsis');
});

test('createDebug captures every log into the trace ring regardless of console state', () => {
    clearTrace();
    const log = createDebug('qol:test');
    log('arrow down', '→', 'slider');
    const rows = getTrace('qol:test');
    assert.equal(rows.length, 1, 'one entry captured');
    assert.equal(rows[0].ns, 'qol:test');
    assert.equal(rows[0].msg, 'arrow down → slider');
    assert.equal(typeof rows[0].seq, 'number');
});

test('getTrace filters by namespace or message substring', () => {
    clearTrace();
    createDebug('qol:nav')('field-move down → brightness');
    createDebug('qol:wedge')('TARGET CHANGED');
    assert.equal(getTrace('qol:nav').length, 1, 'filter by namespace');
    assert.equal(getTrace('brightness').length, 1, 'filter by message substring');
    assert.equal(getTrace().length, 2, 'no filter returns all');
});

test('trace ring is bounded and clearable', () => {
    clearTrace();
    const log = createDebug('qol:flood');
    for (let i = 0; i < 600; i += 1) log('entry', i);
    assert.equal(getTrace().length, 500, 'ring caps at TRACE_MAX');
    const newest = getTrace().at(-1);
    assert.equal(newest.msg, 'entry 599', 'newest entry retained after eviction');
    clearTrace();
    assert.equal(getTrace().length, 0, 'clearTrace empties the ring');
});
