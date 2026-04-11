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
