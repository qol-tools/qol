import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
    clampOpacity,
    formatOpacityPercent,
    normalizeOpacityForServer,
    parseGpuiResponse,
    GHOST_OPACITY_DEFAULT,
} from './runtime-gpui-opacity.js';

test('clampOpacity table', () => {
    const cases = [
        [0, 0],
        [0.5, 0.5],
        [1, 1],
        [-0.1, 0],
        [1.5, 1],
        [Number.NaN, GHOST_OPACITY_DEFAULT],
        [Number.POSITIVE_INFINITY, GHOST_OPACITY_DEFAULT],
        [Number.NEGATIVE_INFINITY, GHOST_OPACITY_DEFAULT],
        ['0.42', 0.42],
        [null, GHOST_OPACITY_DEFAULT],
        [undefined, GHOST_OPACITY_DEFAULT],
        ['nope', GHOST_OPACITY_DEFAULT],
    ];
    for (const [input, expected] of cases) {
        assert.equal(clampOpacity(input), expected, `input: ${String(input)}`);
    }
});

test('clampOpacity property: result is always within [0,1]', () => {
    let rng = 17;
    const next = () => {
        rng = (rng * 1664525 + 1013904223) >>> 0;
        return rng / 0xffffffff;
    };
    for (let i = 0; i < 250; i++) {
        const raw = (next() * 4) - 2;
        const out = clampOpacity(raw);
        assert.ok(out >= 0 && out <= 1, `out of range: ${out} from ${raw}`);
    }
});

test('formatOpacityPercent table', () => {
    const cases = [
        [0, '0%'],
        [0.5, '50%'],
        [1, '100%'],
        [0.234, '23%'],
        [-0.1, '0%'],
        [1.5, '100%'],
        [Number.NaN, '0%'],
    ];
    for (const [input, expected] of cases) {
        assert.equal(formatOpacityPercent(input), expected, `input: ${input}`);
    }
});

test('normalizeOpacityForServer table', () => {
    const cases = [
        [null, null],
        [undefined, null],
        [Number.NaN, null],
        ['oops', null],
        [0, 0],
        [0.7, 0.7],
        [2, 1],
        [-1, 0],
    ];
    for (const [input, expected] of cases) {
        assert.equal(normalizeOpacityForServer(input), expected, `input: ${String(input)}`);
    }
});

test('parseGpuiResponse table', () => {
    const cases = [
        [null, GHOST_OPACITY_DEFAULT],
        [undefined, GHOST_OPACITY_DEFAULT],
        [{}, GHOST_OPACITY_DEFAULT],
        [{ ghost_opacity: null }, GHOST_OPACITY_DEFAULT],
        [{ ghost_opacity: 0.5 }, 0.5],
        [{ ghost_opacity: 1.5 }, 1],
        [{ ghost_opacity: -0.5 }, 0],
        [{ ghost_opacity: 'oops' }, GHOST_OPACITY_DEFAULT],
    ];
    for (const [input, expected] of cases) {
        assert.equal(parseGpuiResponse(input).ghostOpacity, expected, `input: ${JSON.stringify(input)}`);
    }
});
