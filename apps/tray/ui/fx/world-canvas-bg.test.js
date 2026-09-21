import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createWorldCanvasBg } from './world-canvas-bg.js';

function installEnv(dpr) {
    globalThis.window = { devicePixelRatio: dpr };
    globalThis.ResizeObserver = class {
        constructor(cb) { this.cb = cb; }
        observe() {}
        disconnect() {}
    };
}

// A <canvas> is a replaced element: when it carries no explicit CSS width/height,
// its layout box (clientWidth/clientHeight) resolves to its intrinsic size, i.e.
// the bitmap (canvas.width/height) - NOT the containing block. This fake reproduces
// that: reading clientWidth returns the current bitmap width.
function makeReplacedElementCanvas(parentW, parentH) {
    const ctx = {
        setTransform() {},
        clearRect() {},
        createImageData: (w, h) => ({ data: new Uint8ClampedArray(Math.max(0, w * h * 4)) }),
        putImageData() {},
        fillRect() {},
        fillStyle: '',
    };
    return {
        width: 300,
        height: 150,
        parentElement: { clientWidth: parentW, clientHeight: parentH },
        getContext: () => ctx,
        get clientWidth() { return this.width; },
        get clientHeight() { return this.height; },
    };
}

function pump(dpr, parentW, parentH, cycles) {
    installEnv(dpr);
    const canvas = makeReplacedElementCanvas(parentW, parentH);
    // zoom below MIN_SCREEN_SPACING/DOT_SPACING so draw() returns before the dot loop;
    // the resize block (where the runaway lives) still executes every call.
    const camera = { x: 0, y: 0, zoom: 0.05, subscribe(fn) { this.fn = fn; return () => {}; } };
    const bg = createWorldCanvasBg(canvas, camera);
    for (let i = 0; i < cycles; i++) camera.fn();
    bg.destroy();
    return canvas;
}

test('world-bg bitmap locks to the parent size on a Retina (dpr=2) display, never self-grows', () => {
    const canvas = pump(2, 800, 600, 15);
    assert.equal(canvas.width, 800 * 2, 'width must track parent*dpr, not double every ResizeObserver cycle');
    assert.equal(canvas.height, 600 * 2, 'height must track parent*dpr, not double every ResizeObserver cycle');
});

test('world-bg bitmap is stable at dpr=1 (the case that always worked)', () => {
    const canvas = pump(1, 1024, 768, 15);
    assert.equal(canvas.width, 1024);
    assert.equal(canvas.height, 768);
});

test('world-bg bitmap is stable at a fractional dpr', () => {
    const canvas = pump(1.5, 1000, 800, 15);
    assert.equal(canvas.width, Math.trunc(1000 * 1.5));
    assert.equal(canvas.height, Math.trunc(800 * 1.5));
});
