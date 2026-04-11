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
