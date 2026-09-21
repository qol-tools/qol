import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
    clampOpacity,
    formatOpacityPercent,
    normalizeOpacityForServer,
    parseGpuiResponse,
    normalizeGhostColor,
    isValidGhostColor,
    GHOST_OPACITY_DEFAULT,
    GHOST_DEBUG_COLOR_DEFAULT,
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

test('parseGpuiResponse opacity table', () => {
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

test('parseGpuiResponse color table', () => {
    const cases = [
        [null, GHOST_DEBUG_COLOR_DEFAULT],
        [undefined, GHOST_DEBUG_COLOR_DEFAULT],
        [{}, GHOST_DEBUG_COLOR_DEFAULT],
        [{ ghost_debug_color: null }, GHOST_DEBUG_COLOR_DEFAULT],
        [{ ghost_debug_color: '#ff8800' }, '#ff8800'],
        [{ ghost_debug_color: 'FF8800' }, '#ff8800'],
        [{ ghost_debug_color: '#FF8800' }, '#ff8800'],
        [{ ghost_debug_color: '   #ff8800   ' }, '#ff8800'],
        [{ ghost_debug_color: '#fff' }, GHOST_DEBUG_COLOR_DEFAULT],
        [{ ghost_debug_color: 'bogus' }, GHOST_DEBUG_COLOR_DEFAULT],
        [{ ghost_debug_color: '' }, GHOST_DEBUG_COLOR_DEFAULT],
    ];
    for (const [input, expected] of cases) {
        assert.equal(parseGpuiResponse(input).ghostColor, expected, `input: ${JSON.stringify(input)}`);
    }
});

test('normalizeGhostColor table', () => {
    const cases = [
        [null, GHOST_DEBUG_COLOR_DEFAULT],
        [undefined, GHOST_DEBUG_COLOR_DEFAULT],
        ['', GHOST_DEBUG_COLOR_DEFAULT],
        ['   ', GHOST_DEBUG_COLOR_DEFAULT],
        ['#ff8800', '#ff8800'],
        ['ff8800', '#ff8800'],
        ['#FF8800', '#ff8800'],
        ['  #FFAACC  ', '#ffaacc'],
        ['#fff', GHOST_DEBUG_COLOR_DEFAULT],
        ['#ff88000', GHOST_DEBUG_COLOR_DEFAULT],
        ['#zzzzzz', GHOST_DEBUG_COLOR_DEFAULT],
        ['not-a-color', GHOST_DEBUG_COLOR_DEFAULT],
    ];
    for (const [input, expected] of cases) {
        assert.equal(normalizeGhostColor(input), expected, `input: ${String(input)}`);
    }
});

test('isValidGhostColor table', () => {
    const cases = [
        [null, false],
        [undefined, false],
        ['', false],
        ['   ', false],
        ['#ff8800', true],
        ['ff8800', true],
        ['#FF8800', true],
        ['#fff', false],
        ['#zzzzzz', false],
        ['bogus', false],
    ];
    for (const [input, expected] of cases) {
        assert.equal(isValidGhostColor(input), expected, `input: ${String(input)}`);
    }
});

test('parseGpuiResponse property: ghostColor is always either "" or "#" + 6 lowercase hex', () => {
    let rng = 31;
    const next = () => {
        rng = (rng * 1664525 + 1013904223) >>> 0;
        return rng / 0xffffffff;
    };
    const randomChar = () => {
        const all = 'abcdefABCDEF0123456789#xyzZ ';
        return all.charAt(Math.floor(next() * all.length));
    };
    for (let i = 0; i < 250; i++) {
        const len = Math.floor(next() * 10);
        let raw = '';
        for (let j = 0; j < len; j++) raw += randomChar();
        const out = parseGpuiResponse({ ghost_debug_color: raw }).ghostColor;
        const ok = out === '' || /^#[0-9a-f]{6}$/.test(out);
        assert.ok(ok, `bad ghostColor: ${JSON.stringify(out)} from ${JSON.stringify(raw)}`);
    }
});
