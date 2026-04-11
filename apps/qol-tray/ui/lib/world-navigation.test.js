import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createNavigation } from './world-navigation.js';
import { contains, createWorldRegistry } from './world-registry.js';
import { createCamera } from './world-camera.js';
import { filterSurfacesByConfinement } from './spatial-nav.js';

test('filterSurfacesByConfinement returns all surfaces when confinement is null', () => {
    const surfaces = [{ tag: 'a' }, { tag: 'b' }];
    const result = filterSurfacesByConfinement(surfaces, null, null);
    assert.deepEqual(result, surfaces);
});

test('filterSurfacesByConfinement returns empty when surfaces lack closest method', () => {
    const surfaces = [{ tag: 'a' }, { tag: 'b' }];
    const confinement = { x: 0, y: 0, width: 100, height: 100, layer: -1 };
    const result = filterSurfacesByConfinement(surfaces, confinement, null);
    assert.deepEqual(result, []);
});

test('filterSurfacesByConfinement keeps surfaces whose entry is inside the confinement', () => {
    const pages = {
        a: { id: 'a', x: 0, y: 0, width: 100, height: 100, layer: -1 },
        b: { id: 'b', x: 500, y: 0, width: 100, height: 100, layer: -1 },
    };
    const registry = { getEntry: (id) => pages[id] || null };
    const confinement = { x: 0, y: 0, width: 200, height: 200, layer: -1 };
    const makeSurface = (viewId) => ({
        closest: () => ({ dataset: { viewId } }),
    });
    const surfaces = [makeSurface('a'), makeSurface('b')];
    const result = filterSurfacesByConfinement(surfaces, confinement, registry);
    assert.equal(result.length, 1);
    assert.equal(result[0].closest().dataset.viewId, 'a');
});

test('camera.panTo clamps x to bounds.x when target is left of bounds', () => {
    const cam = createCamera({ getViewportSize: () => ({ w: 400, h: 300 }) });
    cam.setBounds({ x: 100, y: 100, width: 1000, height: 1000, layer: 0 });
    cam.panTo(0, 150);
    assert.equal(cam.x, 100);
    assert.equal(cam.y, 150);
});

test('camera.panTo clamps x to right edge when target is beyond bounds', () => {
    const cam = createCamera({ getViewportSize: () => ({ w: 400, h: 300 }) });
    cam.setBounds({ x: 100, y: 100, width: 1000, height: 1000, layer: 0 });
    cam.panTo(9999, 150);
    assert.equal(cam.x, 100 + 1000 - 400);
});

test('camera.panTo does not clamp when bounds is null', () => {
    const cam = createCamera({ getViewportSize: () => ({ w: 400, h: 300 }) });
    cam.setBounds(null);
    cam.panTo(9999, 9999);
    assert.equal(cam.x, 9999);
    assert.equal(cam.y, 9999);
});

test('camera.panTo centers on bounds when viewport is larger than bounds', () => {
    const cam = createCamera({ getViewportSize: () => ({ w: 2000, h: 2000 }) });
    cam.setBounds({ x: 100, y: 100, width: 500, height: 500, layer: 0 });
    cam.panTo(200, 200);
    assert.equal(cam.x, 100 + 500 / 2 - 2000 / 2);
    assert.equal(cam.y, 100 + 500 / 2 - 2000 / 2);
});

test('camera.zoomTo clamps to minimum fit zoom when zooming out past bounds', () => {
    const cam = createCamera({ getViewportSize: () => ({ w: 1000, h: 500 }) });
    cam.setBounds({ x: 0, y: 0, width: 1000, height: 500, layer: 0 });
    cam.zoomTo(0.1);
    assert.equal(cam.zoom, 1);
});

test('camera.zoomTo allows zooming in past fit', () => {
    const cam = createCamera({ getViewportSize: () => ({ w: 1000, h: 500 }) });
    cam.setBounds({ x: 0, y: 0, width: 1000, height: 500, layer: 0 });
    cam.zoomTo(5);
    assert.equal(cam.zoom, 5);
});

test('registry stores and retrieves dive targets', () => {
    const reg = createWorldRegistry([], {});
    const claim = { x: 0, y: 0, width: 1280, height: 900, layer: -1 };
    reg.addDiveTarget({ sourceSelector: '#card-a', claim, pages: ['page-a'] });
    const targets = reg.getDiveTargets();
    assert.equal(targets.length, 1);
    assert.equal(targets[0].sourceSelector, '#card-a');
});

test('registry looks up dive target by source selector', () => {
    const reg = createWorldRegistry([], {});
    const claim = { x: 0, y: 0, width: 1280, height: 900, layer: -1 };
    reg.addDiveTarget({ sourceSelector: '#card-a', claim, pages: ['page-a'] });
    const target = reg.getDiveTargetForSource('#card-a');
    assert.equal(target?.sourceSelector, '#card-a');
});

test('registry returns null for unknown source selector', () => {
    const reg = createWorldRegistry([], {});
    assert.equal(reg.getDiveTargetForSource('#nonexistent'), null);
});

test('registry addEntry adds an entry that getEntry retrieves', () => {
    const reg = createWorldRegistry([], {});
    reg.addEntry({ id: 'custom', x: 100, y: 200, width: 50, height: 60, layer: -1 });
    const e = reg.getEntry('custom');
    assert.equal(e?.id, 'custom');
    assert.equal(e?.x, 100);
});

test('registry addEntry overwrites existing entry with same id', () => {
    const reg = createWorldRegistry([], {});
    reg.addEntry({ id: 'e', x: 0, y: 0, width: 10, height: 10, layer: 0 });
    reg.addEntry({ id: 'e', x: 999, y: 999, width: 20, height: 20, layer: 0 });
    const e = reg.getEntry('e');
    assert.equal(e.x, 999);
});

test('contains returns true when rect is null (no confinement)', () => {
    const e = { id: 'x', x: 0, y: 0, width: 100, height: 100, layer: 0 };
    assert.equal(contains(null, e), true);
});

test('contains returns true for entry fully inside rect', () => {
    const rect = { x: 0, y: 0, width: 1000, height: 1000, layer: 0 };
    const e = { id: 'x', x: 100, y: 100, width: 200, height: 200, layer: 0 };
    assert.equal(contains(rect, e), true);
});

test('contains returns true when entry exactly fills rect', () => {
    const rect = { x: 0, y: 0, width: 1000, height: 1000, layer: 0 };
    const e = { id: 'x', x: 0, y: 0, width: 1000, height: 1000, layer: 0 };
    assert.equal(contains(rect, e), true);
});

test('contains returns false for entry on a different layer', () => {
    const rect = { x: 0, y: 0, width: 1000, height: 1000, layer: 0 };
    const e = { id: 'x', x: 100, y: 100, width: 200, height: 200, layer: -1 };
    assert.equal(contains(rect, e), false);
});

test('contains returns false when entry extends past right edge', () => {
    const rect = { x: 0, y: 0, width: 1000, height: 1000, layer: 0 };
    const e = { id: 'x', x: 900, y: 0, width: 200, height: 100, layer: 0 };
    assert.equal(contains(rect, e), false);
});

test('contains returns false when entry extends past bottom edge', () => {
    const rect = { x: 0, y: 0, width: 1000, height: 1000, layer: 0 };
    const e = { id: 'x', x: 0, y: 900, width: 100, height: 200, layer: 0 };
    assert.equal(contains(rect, e), false);
});

test('contains returns false when entry starts before left edge', () => {
    const rect = { x: 100, y: 0, width: 1000, height: 1000, layer: 0 };
    const e = { id: 'x', x: 0, y: 0, width: 200, height: 100, layer: 0 };
    assert.equal(contains(rect, e), false);
});

test('contains returns false when entry starts above top edge', () => {
    const rect = { x: 0, y: 100, width: 1000, height: 1000, layer: 0 };
    const e = { id: 'x', x: 0, y: 0, width: 100, height: 200, layer: 0 };
    assert.equal(contains(rect, e), false);
});

function makeMocks() {
    const pages = {
        plugins: { id: 'plugins', x: 0, y: 0, width: 1280, height: 900, layer: 0, parent: null },
        hotkeys: { id: 'hotkeys', x: 10000, y: 0, width: 1280, height: 900, layer: 0, parent: null },
        'plugins-config': { id: 'plugins-config', x: 0, y: 0, width: 1280, height: 900, layer: -1, parent: 'plugins' },
    };
    const diveTargets = new Map();
    const registry = {
        getEntry: (id) => pages[id] || null,
        getEntriesForLayer: (n) => Object.values(pages).filter(e => e.layer === n),
        getDiveTargetForSource: (sel) => diveTargets.get(sel) || null,
        addDiveTarget: (t) => diveTargets.set(t.sourceSelector, { ...t }),
        addEntry: (e) => { pages[e.id] = { ...e }; },
    };
    const camera = {
        x: 0, y: 0, zoom: 1, layer: 0,
        panSmooth(tx, ty, dur, onComplete) { this.x = tx; this.y = ty; if (onComplete) onComplete(); },
        panTo(tx, ty) { this.x = tx; this.y = ty; },
        zoomTo(z) { this.zoom = z; },
        setLayer(l) { this.layer = l; },
        cancelSmooth() {},
        setBounds(r) { this._bounds = r; },
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

test('gotoAnchor uses page geometric center regardless of focus registry', () => {
    const { registry, camera, getSettings, domHelpers } = makeMocks();
    const nav = createNavigation({ registry, camera, getSettings, domHelpers });
    nav.setFocus('plugins', 'fake-selector');
    nav.gotoAnchor({ pageId: 'plugins' }, { respectKnob: false });
    assert.equal(camera.x, 640 - 800 / 2);
    assert.equal(camera.y, 450 - 600 / 2);
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

test('getCurrentConfinement returns null by default', () => {
    const { registry, camera, getSettings, domHelpers } = makeMocks();
    const nav = createNavigation({ registry, camera, getSettings, domHelpers });
    assert.equal(nav.getCurrentConfinement(), null);
});

test('dive into a DiveTarget sets current confinement to the target claim', () => {
    const { registry, camera, getSettings, domHelpers } = makeMocks();
    const claim = { x: 0, y: 0, width: 1280, height: 900, layer: -1 };
    registry.addDiveTarget({ sourceSelector: '#card-a', claim, pages: ['plugins-config'] });
    const nav = createNavigation({ registry, camera, getSettings, domHelpers });
    nav.setCurrentAnchor({ pageId: 'plugins' });
    nav.diveInto('#card-a');
    assert.deepEqual(nav.getCurrentConfinement(), claim);
});

test('ascend pops the confinement back to null', () => {
    const { registry, camera, getSettings, domHelpers } = makeMocks();
    const claim = { x: 0, y: 0, width: 1280, height: 900, layer: -1 };
    registry.addDiveTarget({ sourceSelector: '#card-a', claim, pages: ['plugins-config'] });
    const nav = createNavigation({ registry, camera, getSettings, domHelpers });
    nav.setCurrentAnchor({ pageId: 'plugins' });
    nav.diveInto('#card-a');
    nav.ascend();
    assert.equal(nav.getCurrentConfinement(), null);
});

test('nested dive pushes and ascend pops one frame at a time', () => {
    const { registry, camera, getSettings, domHelpers } = makeMocks();
    const claim1 = { x: 0, y: 0, width: 1280, height: 900, layer: -1 };
    const claim2 = { x: 100, y: 100, width: 400, height: 400, layer: -2 };
    registry.addDiveTarget({ sourceSelector: '#a', claim: claim1, pages: ['plugins-config'] });
    registry.addDiveTarget({ sourceSelector: '#b', claim: claim2, pages: ['plugins-config'] });
    const nav = createNavigation({ registry, camera, getSettings, domHelpers });
    nav.setCurrentAnchor({ pageId: 'plugins' });
    nav.diveInto('#a');
    nav.diveInto('#b');
    assert.deepEqual(nav.getCurrentConfinement(), claim2);
    nav.ascend();
    assert.deepEqual(nav.getCurrentConfinement(), claim1);
    nav.ascend();
    assert.equal(nav.getCurrentConfinement(), null);
});

test('diveInto on an unknown selector is a no-op', () => {
    const { registry, camera, getSettings, domHelpers } = makeMocks();
    const nav = createNavigation({ registry, camera, getSettings, domHelpers });
    nav.setCurrentAnchor({ pageId: 'plugins' });
    nav.diveInto('#nonexistent');
    assert.equal(nav.getCurrentConfinement(), null);
    assert.equal(nav.stackDepth(), 0);
});

test('dive calls camera.setBounds with the confinement rect', () => {
    const { registry, camera, getSettings, domHelpers } = makeMocks();
    const claim = { x: 0, y: 0, width: 1280, height: 900, layer: -1 };
    registry.addDiveTarget({ sourceSelector: '#card-a', claim, pages: ['plugins-config'] });
    const nav = createNavigation({ registry, camera, getSettings, domHelpers });
    nav.setCurrentAnchor({ pageId: 'plugins' });
    nav.diveInto('#card-a');
    assert.deepEqual(camera._bounds, claim);
});

test('ascend from root dive calls camera.setBounds(null)', () => {
    const { registry, camera, getSettings, domHelpers } = makeMocks();
    const claim = { x: 0, y: 0, width: 1280, height: 900, layer: -1 };
    registry.addDiveTarget({ sourceSelector: '#card-a', claim, pages: ['plugins-config'] });
    const nav = createNavigation({ registry, camera, getSettings, domHelpers });
    nav.setCurrentAnchor({ pageId: 'plugins' });
    nav.diveInto('#card-a');
    nav.ascend();
    assert.equal(camera._bounds, null);
});
