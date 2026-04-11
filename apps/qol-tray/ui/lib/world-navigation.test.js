import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createNavigation } from './world-navigation.js';

function makeMocks() {
    const pages = {
        plugins: { id: 'plugins', x: 0, y: 0, width: 1280, height: 900, layer: 0, parent: null },
        hotkeys: { id: 'hotkeys', x: 10000, y: 0, width: 1280, height: 900, layer: 0, parent: null },
        'plugins-config': { id: 'plugins-config', x: 0, y: 0, width: 1280, height: 900, layer: -1, parent: 'plugins' },
    };
    const registry = {
        getEntry: (id) => pages[id] || null,
        getEntriesForLayer: (n) => Object.values(pages).filter(e => e.layer === n),
    };
    const camera = {
        x: 0, y: 0, zoom: 1, layer: 0,
        panSmooth(tx, ty) { this.x = tx; this.y = ty; },
        panTo(tx, ty) { this.x = tx; this.y = ty; },
        zoomTo(z) { this.zoom = z; },
        setLayer(l) { this.layer = l; },
        cancelSmooth() {},
    };
    const settings = { anchorToPages: true };
    const domHelpers = {
        resolveSelector: () => null,
        getViewportSize: () => ({ w: 800, h: 600 }),
    };
    return { registry, camera, getSettings: () => settings, domHelpers, settings };
}

test('getCurrentAnchor returns the current anchor', () => {
    const { registry, camera, getSettings, domHelpers } = makeMocks();
    const nav = createNavigation({ registry, camera, getSettings, domHelpers });
    nav.setCurrentAnchor({ pageId: 'plugins' });
    assert.equal(nav.getCurrentAnchor().pageId, 'plugins');
});

test('dive pushes current anchor and sets new', () => {
    const { registry, camera, getSettings, domHelpers } = makeMocks();
    const nav = createNavigation({ registry, camera, getSettings, domHelpers });
    nav.setCurrentAnchor({ pageId: 'plugins' });
    nav.dive('plugins-config');
    assert.equal(nav.getCurrentAnchor().pageId, 'plugins-config');
    assert.equal(nav.stackDepth(), 1);
});

test('ascend pops and restores previous anchor', () => {
    const { registry, camera, getSettings, domHelpers } = makeMocks();
    const nav = createNavigation({ registry, camera, getSettings, domHelpers });
    nav.setCurrentAnchor({ pageId: 'plugins' });
    nav.dive('plugins-config');
    const ok = nav.ascend();
    assert.equal(ok, true);
    assert.equal(nav.getCurrentAnchor().pageId, 'plugins');
    assert.equal(nav.stackDepth(), 0);
});

test('ascend returns false when stack is empty', () => {
    const { registry, camera, getSettings, domHelpers } = makeMocks();
    const nav = createNavigation({ registry, camera, getSettings, domHelpers });
    nav.setCurrentAnchor({ pageId: 'plugins' });
    assert.equal(nav.ascend(), false);
    assert.equal(nav.getCurrentAnchor().pageId, 'plugins');
});

test('dive/ascend invariant holds across all layer-0 pages', () => {
    const { registry, camera, getSettings, domHelpers } = makeMocks();
    const nav = createNavigation({ registry, camera, getSettings, domHelpers });
    const pages = ['plugins', 'hotkeys'];
    for (const start of pages) {
        nav.setCurrentAnchor({ pageId: start });
        nav.dive('plugins-config');
        nav.ascend();
        assert.equal(nav.getCurrentAnchor().pageId, start, `restoring from ${start}`);
    }
});

test('ascend restores zoom captured at dive time', () => {
    const { registry, camera, getSettings, domHelpers } = makeMocks();
    const nav = createNavigation({ registry, camera, getSettings, domHelpers });
    nav.setCurrentAnchor({ pageId: 'plugins' });
    camera.zoom = 1.5;
    nav.dive('plugins-config');
    camera.zoom = 2.0;
    nav.ascend();
    assert.equal(camera.zoom, 1.5);
});

function spyCamera() {
    const calls = [];
    return {
        x: 0, y: 0, zoom: 1, layer: 0,
        panSmooth(tx, ty, dur, onComplete) { calls.push(['panSmooth', tx, ty, dur]); this.x = tx; this.y = ty; if (onComplete) onComplete(); },
        panTo(tx, ty) { calls.push(['panTo', tx, ty]); this.x = tx; this.y = ty; },
        zoomTo(z) { calls.push(['zoomTo', z]); this.zoom = z; },
        setLayer(l) { calls.push(['setLayer', l]); this.layer = l; },
        cancelSmooth() { calls.push(['cancelSmooth']); },
        _calls: calls,
    };
}

test('gotoAnchor centers on page geometric center when focus registry is empty', () => {
    const { registry, getSettings, domHelpers } = makeMocks();
    const camera = spyCamera();
    const nav = createNavigation({ registry, camera, getSettings, domHelpers });
    nav.gotoAnchor({ pageId: 'plugins' }, { respectKnob: false });
    const pan = camera._calls.find(c => c[0] === 'panSmooth');
    assert.ok(pan, 'panSmooth was called');
    assert.equal(pan[1], 640 - 800 / 2);
    assert.equal(pan[2], 450 - 600 / 2);
});

test('gotoAnchor honors respectKnob when knob is off', () => {
    const { registry, domHelpers, settings } = makeMocks();
    const camera = spyCamera();
    settings.anchorToPages = false;
    const nav = createNavigation({ registry, camera, getSettings: () => settings, domHelpers });
    nav.gotoAnchor({ pageId: 'plugins' }, { respectKnob: true });
    assert.equal(camera._calls.filter(c => c[0] === 'panSmooth').length, 0);
});

test('gotoAnchor with respectKnob=false bypasses the knob', () => {
    const { registry, domHelpers, settings } = makeMocks();
    const camera = spyCamera();
    settings.anchorToPages = false;
    const nav = createNavigation({ registry, camera, getSettings: () => settings, domHelpers });
    nav.gotoAnchor({ pageId: 'plugins' }, { respectKnob: false });
    assert.equal(camera._calls.filter(c => c[0] === 'panSmooth').length, 1);
});

test('gotoAnchor falls back to first layer-0 page on unknown pageId', () => {
    const { registry, getSettings, domHelpers } = makeMocks();
    const camera = spyCamera();
    const nav = createNavigation({ registry, camera, getSettings, domHelpers });
    nav.gotoAnchor({ pageId: 'nonexistent' }, { respectKnob: false });
    const pan = camera._calls.find(c => c[0] === 'panSmooth');
    assert.ok(pan, 'panSmooth was called with fallback');
    assert.equal(pan[1], 640 - 800 / 2);
});

test('gotoAnchor centers on resolved surface world center when focus registry has it', () => {
    const { registry, getSettings } = makeMocks();
    const camera = spyCamera();
    const domHelpers = {
        resolveSelector: (sel) => sel === 'fake-selector' ? { x: 500, y: 300 } : null,
        getViewportSize: () => ({ w: 800, h: 600 }),
    };
    const nav = createNavigation({ registry, camera, getSettings, domHelpers });
    nav.setFocus('plugins', 'fake-selector');
    nav.gotoAnchor({ pageId: 'plugins' }, { respectKnob: false });
    const pan = camera._calls.find(c => c[0] === 'panSmooth');
    assert.ok(pan);
    assert.equal(pan[1], 500 - 800 / 2);
    assert.equal(pan[2], 300 - 600 / 2);
});

test('gotoAnchor falls back to page center when focus selector is stale', () => {
    const { registry, getSettings } = makeMocks();
    const camera = spyCamera();
    const domHelpers = {
        resolveSelector: () => null,
        getViewportSize: () => ({ w: 800, h: 600 }),
    };
    const nav = createNavigation({ registry, camera, getSettings, domHelpers });
    nav.setFocus('plugins', '#gone');
    nav.gotoAnchor({ pageId: 'plugins' }, { respectKnob: false });
    const pan = camera._calls.find(c => c[0] === 'panSmooth');
    assert.equal(pan[1], 640 - 800 / 2);
});

test('gotoAnchor triggers setLayer when page layer differs', () => {
    const { registry, getSettings, domHelpers } = makeMocks();
    const camera = spyCamera();
    camera.layer = 0;
    const nav = createNavigation({ registry, camera, getSettings, domHelpers });
    nav.gotoAnchor({ pageId: 'plugins-config' }, { respectKnob: false });
    const setLayer = camera._calls.find(c => c[0] === 'setLayer');
    assert.ok(setLayer);
    assert.equal(setLayer[1], -1);
});

test('dive/ascend invariant holds under passive noise', () => {
    const { registry, camera, getSettings, domHelpers } = makeMocks();
    const nav = createNavigation({ registry, camera, getSettings, domHelpers });
    const pages = ['plugins', 'hotkeys'];
    const noise = [
        () => nav.setFocus('plugins', '[data-index="1"]'),
        () => nav.setFocus('hotkeys', '[data-index="2"]'),
        () => nav.gotoAnchor({ pageId: 'hotkeys' }, { respectKnob: true }),
    ];
    for (const start of pages) {
        nav.setCurrentAnchor({ pageId: start });
        for (const n of noise) n();
        nav.dive('plugins-config');
        for (const n of noise) n();
        nav.ascend();
        assert.equal(nav.getCurrentAnchor().pageId, start, `restoring from ${start} under noise`);
    }
});
