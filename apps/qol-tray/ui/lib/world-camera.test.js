import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createCamera } from './world-camera.js';

test('setBounds preserves the current zoom (does not auto-bump to fit)', () => {
    const camera = createCamera({ getViewportSize: () => ({ w: 800, h: 600 }) });
    camera.setBounds({ x: 0, y: 0, width: 100, height: 100, layer: 0 });
    assert.equal(camera.zoom, 1);
});

test('createCamera honors initial zoom option (so boot uses configured defaultZoom)', () => {
    const camera = createCamera({ zoom: 0.8, getViewportSize: () => ({ w: 800, h: 600 }) });
    assert.equal(camera.zoom, 0.8);
});

test('createCamera defaults to zoom 1 when option omitted', () => {
    const camera = createCamera({ getViewportSize: () => ({ w: 800, h: 600 }) });
    assert.equal(camera.zoom, 1);
});

test('setBounds is a no-op when bounds layer does not match camera layer', () => {
    const camera = createCamera({ getViewportSize: () => ({ w: 800, h: 600 }) });
    camera.setBounds({ x: 0, y: 0, width: 100, height: 100, layer: 1 });
    assert.equal(camera.zoom, 1);
});

test('clampPanTarget centers using visibleW (not raw vp.w) at zoom != 1', () => {
    const camera = createCamera({ getViewportSize: () => ({ w: 800, h: 600 }) });
    camera.setBounds({ x: 0, y: 0, width: 100, height: 100, layer: 0 });
    camera.zoomTo(8);
    camera.panTo(9999, 9999);
    assert.equal(camera.zoom, 8);
    assert.equal(camera.x, 0);
    assert.equal(camera.y, 25);
});

test('panTo at zoom 2 clamps to bounds using visible-world width, not raw viewport width', () => {
    const camera = createCamera({ getViewportSize: () => ({ w: 800, h: 600 }) });
    camera.setBounds({ x: 0, y: 0, width: 500, height: 400, layer: 0 });
    camera.zoomTo(2);
    camera.panTo(9999, 9999);
    const vp = { w: 800, h: 600 };
    const visibleW = vp.w / camera.zoom;
    const visibleH = vp.h / camera.zoom;
    assert.equal(camera.x, 500 - visibleW);
    assert.equal(camera.y, 400 - visibleH);
});

test('zoomTo clamps to a hard MAX_ZOOM even when bounds allow higher', () => {
    const camera = createCamera({ getViewportSize: () => ({ w: 800, h: 600 }) });
    camera.setBounds({ x: 0, y: 0, width: 2000, height: 2000, layer: 0 });
    camera.zoomTo(9999);
    assert.equal(camera.zoom, 8);
});

test('zoomAround keeps the world point under the anchor fixed on screen', () => {
    const camera = createCamera({ getViewportSize: () => ({ w: 800, h: 600 }) });
    camera.setBounds({ x: 0, y: 0, width: 2000, height: 2000, layer: 0 });
    camera.panTo(100, 100);
    const anchorSx = 200;
    const anchorSy = 150;
    const anchorWorldX = camera.x + anchorSx / camera.zoom;
    const anchorWorldY = camera.y + anchorSy / camera.zoom;
    camera.zoomAround(anchorSx, anchorSy, 2);
    assert.equal(camera.zoom, 2);
    assert.equal(camera.x + anchorSx / camera.zoom, anchorWorldX);
    assert.equal(camera.y + anchorSy / camera.zoom, anchorWorldY);
});

test('zoomAround clamps to bounds min when target is too small', () => {
    const camera = createCamera({ getViewportSize: () => ({ w: 800, h: 600 }) });
    camera.setBounds({ x: 0, y: 0, width: 100, height: 100, layer: 0 });
    camera.zoomTo(20);
    camera.zoomAround(0, 0, 0.01);
    assert.equal(camera.zoom, 3);
});

test('zoomSmooth clamps target zoom and pan to bounds', () => {
    const rafCallbacks = [];
    const origRaf = globalThis.requestAnimationFrame;
    const origCancel = globalThis.cancelAnimationFrame;
    globalThis.requestAnimationFrame = (cb) => { rafCallbacks.push(cb); return rafCallbacks.length; };
    globalThis.cancelAnimationFrame = () => {};
    const origNow = performance.now.bind(performance);
    let now = 0;
    performance.now = () => now;
    try {
        const camera = createCamera({ getViewportSize: () => ({ w: 800, h: 600 }) });
        camera.setBounds({ x: 0, y: 0, width: 100, height: 100, layer: 0 });
        camera.zoomSmooth(9999, 9999, 0.5, 100);
        now = 1000;
        while (rafCallbacks.length > 0) {
            const cb = rafCallbacks.shift();
            cb(now);
        }
        assert.equal(camera.zoom, 3);
        assert.ok(Math.abs(camera.x - (50 - 800 / 3 / 2)) < 1e-9);
        assert.ok(Math.abs(camera.y - (50 - 600 / 3 / 2)) < 1e-9);
    } finally {
        if (origRaf === undefined) delete globalThis.requestAnimationFrame;
        else globalThis.requestAnimationFrame = origRaf;
        if (origCancel === undefined) delete globalThis.cancelAnimationFrame;
        else globalThis.cancelAnimationFrame = origCancel;
        performance.now = origNow;
    }
});
