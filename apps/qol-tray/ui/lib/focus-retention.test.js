import { test } from 'node:test';
import assert from 'node:assert/strict';
import { pickFallbackSurface } from './focus-retention.js';

function makeSurface({ id, selected = false, connected = true, visible = true, disabled = false }) {
    return {
        id,
        isConnected: connected,
        disabled,
        getAttribute(name) {
            if (name === 'data-selected-surface') return '';
            if (name === 'data-selected') return selected ? 'true' : 'false';
            return null;
        },
        getClientRects() {
            return visible ? [{ width: 10, height: 10 }] : [];
        },
        closest(_) { return null; },
    };
}

function makeRoot({ id, surfaces = [], slots = [], connected = true, rect = null }) {
    const root = {
        id,
        isConnected: connected,
        querySelectorAll(selector) {
            if (selector === '[data-selected-surface]') return surfaces;
            if (selector === '.world-view-slot') return slots;
            throw new Error(`unexpected selector: ${selector}`);
        },
    };
    if (rect) {
        root.getBoundingClientRect = () => rect;
    }
    return root;
}

test('pickFallbackSurface picks selected surface in lost container first', () => {
    const a = makeSurface({ id: 'a' });
    const b = makeSurface({ id: 'b', selected: true });
    const c = makeSurface({ id: 'c' });
    const container = makeRoot({ id: 'container', surfaces: [a, b, c] });
    const fallback = pickFallbackSurface({ lostContainer: container, lostSlot: null, viewport: null });
    assert.equal(fallback?.id, 'b');
});

test('pickFallbackSurface falls through to first surface in container when none selected', () => {
    const a = makeSurface({ id: 'a' });
    const b = makeSurface({ id: 'b' });
    const container = makeRoot({ id: 'container', surfaces: [a, b] });
    const fallback = pickFallbackSurface({ lostContainer: container, lostSlot: null, viewport: null });
    assert.equal(fallback?.id, 'a');
});

test('pickFallbackSurface falls back to slot when container is empty', () => {
    const slotA = makeSurface({ id: 'slot-a' });
    const slotB = makeSurface({ id: 'slot-b', selected: true });
    const container = makeRoot({ id: 'container', surfaces: [] });
    const slot = makeRoot({ id: 'slot', surfaces: [slotA, slotB] });
    const fallback = pickFallbackSurface({ lostContainer: container, lostSlot: slot, viewport: null });
    assert.equal(fallback?.id, 'slot-b');
});

test('pickFallbackSurface falls back to viewport when slot is empty', () => {
    const vpA = makeSurface({ id: 'vp-a' });
    const viewport = makeRoot({ id: 'viewport', surfaces: [vpA] });
    const fallback = pickFallbackSurface({ lostContainer: null, lostSlot: null, viewport });
    assert.equal(fallback?.id, 'vp-a');
});

test('pickFallbackSurface skips disconnected surfaces', () => {
    const a = makeSurface({ id: 'a', connected: false });
    const b = makeSurface({ id: 'b' });
    const container = makeRoot({ id: 'container', surfaces: [a, b] });
    const fallback = pickFallbackSurface({ lostContainer: container, lostSlot: null, viewport: null });
    assert.equal(fallback?.id, 'b');
});

test('pickFallbackSurface skips invisible surfaces', () => {
    const a = makeSurface({ id: 'a', visible: false });
    const b = makeSurface({ id: 'b' });
    const container = makeRoot({ id: 'container', surfaces: [a, b] });
    const fallback = pickFallbackSurface({ lostContainer: container, lostSlot: null, viewport: null });
    assert.equal(fallback?.id, 'b');
});

test('pickFallbackSurface skips disabled surfaces', () => {
    const a = makeSurface({ id: 'a', disabled: true });
    const b = makeSurface({ id: 'b' });
    const container = makeRoot({ id: 'container', surfaces: [a, b] });
    const fallback = pickFallbackSurface({ lostContainer: container, lostSlot: null, viewport: null });
    assert.equal(fallback?.id, 'b');
});

test('pickFallbackSurface returns null when nothing usable anywhere', () => {
    const fallback = pickFallbackSurface({ lostContainer: null, lostSlot: null, viewport: null });
    assert.equal(fallback, null);
});

test('pickFallbackSurface returns null when all surfaces disconnected', () => {
    const a = makeSurface({ id: 'a', connected: false });
    const b = makeSurface({ id: 'b', connected: false });
    const container = makeRoot({ id: 'container', surfaces: [a, b] });
    const slot = makeRoot({ id: 'slot', surfaces: [] });
    const viewport = makeRoot({ id: 'viewport', surfaces: [] });
    const fallback = pickFallbackSurface({ lostContainer: container, lostSlot: slot, viewport });
    assert.equal(fallback, null);
});

test('pickFallbackSurface skips disconnected container/slot roots', () => {
    const a = makeSurface({ id: 'a' });
    const container = makeRoot({ id: 'container', surfaces: [a], connected: false });
    const slotA = makeSurface({ id: 'slot-a' });
    const slot = makeRoot({ id: 'slot', surfaces: [slotA] });
    const fallback = pickFallbackSurface({ lostContainer: container, lostSlot: slot, viewport: null });
    assert.equal(fallback?.id, 'slot-a');
});

test('pickFallbackSurface prefers earliest selected when multiple in container', () => {
    const a = makeSurface({ id: 'a' });
    const b = makeSurface({ id: 'b', selected: true });
    const c = makeSurface({ id: 'c', selected: true });
    const container = makeRoot({ id: 'container', surfaces: [a, b, c] });
    const fallback = pickFallbackSurface({ lostContainer: container, lostSlot: null, viewport: null });
    assert.equal(fallback?.id, 'b');
});

test('property: 200 random shapes — fallback always usable when any usable surface exists', () => {
    const seed = 0xC0FFEE;
    let s = seed;
    const rand = () => {
        s = (s * 1103515245 + 12345) & 0x7fffffff;
        return s / 0x7fffffff;
    };
    for (let i = 0; i < 200; i++) {
        const containerSize = Math.floor(rand() * 6);
        const slotSize = Math.floor(rand() * 5);
        const vpSize = Math.floor(rand() * 4);
        const make = (kind, n) => Array.from({ length: n }, (_, j) => makeSurface({
            id: `${kind}-${j}`,
            selected: rand() < 0.3,
            connected: rand() < 0.85,
            visible: rand() < 0.85,
            disabled: rand() < 0.1,
        }));
        const container = containerSize > 0 ? makeRoot({ id: 'container', surfaces: make('c', containerSize) }) : null;
        const slot = slotSize > 0 ? makeRoot({ id: 'slot', surfaces: make('s', slotSize) }) : null;
        const viewport = vpSize > 0 ? makeRoot({ id: 'viewport', surfaces: make('v', vpSize) }) : null;
        const allSurfaces = [
            ...(container?.querySelectorAll('[data-selected-surface]') || []),
            ...(slot?.querySelectorAll('[data-selected-surface]') || []),
            ...(viewport?.querySelectorAll('[data-selected-surface]') || []),
        ];
        const anyUsable = allSurfaces.some(s => s.isConnected && !s.disabled && s.getClientRects().length > 0);
        const fallback = pickFallbackSurface({ lostContainer: container, lostSlot: slot, viewport });
        if (anyUsable) {
            assert.ok(fallback, `case ${i}: expected fallback when usable surfaces exist`);
            assert.ok(fallback.isConnected, `case ${i}: fallback must be connected`);
            assert.equal(fallback.disabled, false, `case ${i}: fallback must not be disabled`);
            assert.ok(fallback.getClientRects().length > 0, `case ${i}: fallback must be visible`);
        } else {
            assert.equal(fallback, null, `case ${i}: expected null when no usable surfaces`);
        }
    }
});

function makeSurfaceWithRect({ id, selected = false, rect }) {
    const s = makeSurface({ id, selected });
    s.getBoundingClientRect = () => rect;
    return s;
}

test('pickFallbackSurface ignores off-screen world-slots in viewport fallback', () => {
    const VIEWPORT_RECT = { left: 0, top: 0, right: 800, bottom: 600, width: 800, height: 600 };
    const offscreen = makeSurfaceWithRect({
        id: 'plugin-card',
        rect: { left: -15578, top: 115, right: -15290, bottom: 277, width: 288, height: 162 },
    });
    const offscreenSlot = makeRoot({
        id: 'slot-plugins',
        surfaces: [offscreen],
        rect: { left: -15600, top: 0, right: -14320, bottom: 900, width: 1280, height: 900 },
    });
    const onscreen = makeSurfaceWithRect({
        id: 'hotkey-row',
        selected: true,
        rect: { left: 100, top: 200, right: 700, bottom: 240, width: 600, height: 40 },
    });
    const onscreenSlot = makeRoot({
        id: 'slot-hotkeys',
        surfaces: [onscreen],
        rect: { left: 80, top: 0, right: 720, bottom: 900, width: 640, height: 900 },
    });
    const viewport = makeRoot({
        id: 'viewport',
        surfaces: [offscreen, onscreen],
        slots: [offscreenSlot, onscreenSlot],
        rect: VIEWPORT_RECT,
    });
    const fallback = pickFallbackSurface({ lostContainer: null, lostSlot: null, viewport });
    assert.equal(fallback?.id, 'hotkey-row',
        'must pick a surface inside a slot whose rect intersects the viewport');
});

test('pickFallbackSurface picks first viewport-intersecting surface when no slot is selected', () => {
    const VIEWPORT_RECT = { left: 0, top: 0, right: 800, bottom: 600, width: 800, height: 600 };
    const offscreen = makeSurfaceWithRect({
        id: 'far-away',
        rect: { left: -2000, top: 0, right: -1800, bottom: 100, width: 200, height: 100 },
    });
    const onscreen = makeSurfaceWithRect({
        id: 'visible',
        rect: { left: 100, top: 100, right: 300, bottom: 200, width: 200, height: 100 },
    });
    const viewport = makeRoot({
        id: 'viewport',
        surfaces: [offscreen, onscreen],
        slots: [],
        rect: VIEWPORT_RECT,
    });
    const fallback = pickFallbackSurface({ lostContainer: null, lostSlot: null, viewport });
    assert.equal(fallback?.id, 'visible',
        'must skip surfaces whose rects fall outside the viewport');
});

test('property: 200 random shapes — selected-true in container always wins over slot/viewport', () => {
    const seed = 0xBEEFCAFE;
    let s = seed;
    const rand = () => {
        s = (s * 1103515245 + 12345) & 0x7fffffff;
        return s / 0x7fffffff;
    };
    for (let i = 0; i < 200; i++) {
        const containerSurfaces = [
            makeSurface({ id: `c-0` }),
            makeSurface({ id: `c-selected`, selected: true }),
            makeSurface({ id: `c-2` }),
        ];
        const slotSurfaces = [makeSurface({ id: 's-0', selected: rand() < 0.5 })];
        const vpSurfaces = [makeSurface({ id: 'v-0', selected: rand() < 0.5 })];
        const container = makeRoot({ id: 'container', surfaces: containerSurfaces });
        const slot = makeRoot({ id: 'slot', surfaces: slotSurfaces });
        const viewport = makeRoot({ id: 'viewport', surfaces: vpSurfaces });
        const fallback = pickFallbackSurface({ lostContainer: container, lostSlot: slot, viewport });
        assert.equal(fallback?.id, 'c-selected', `case ${i}: container's selected must win`);
    }
});
