import { test } from 'node:test';
import assert from 'node:assert/strict';
import { sliceMinimapRange, visibleMinimapEntries } from './minimap-filter.js';

// ---------------------------------------------------------------------------
// Parameterized table: each row is { name, input, expectedIds } so every
// branch and edge (no-dive, dive, sibling collisions, overlapping geometry,
// deep layers, empty layers, duplicate parents) is asserted as a contract.
// ---------------------------------------------------------------------------

const ground = [
    { id: 'plugins', parent: null, layer: 0 },
    { id: 'hotkeys', parent: null, layer: 0 },
    { id: 'shortcuts', parent: null, layer: 0 },
];
const pluginSections = [
    { id: 'plugin-a-config', parent: 'plugins', layer: -1 },
    { id: 'plugin-a-advanced', parent: 'plugins', layer: -1 },
];
const staticSubs = [
    { id: 'hotkeys-editor', parent: 'hotkeys', layer: -1 },
    { id: 'shortcuts-editor', parent: 'shortcuts', layer: -1 },
];
const layerMinusTwo = [
    { id: 'plugin-a-config-deep', parent: 'plugin-a-config', layer: -2 },
    { id: 'plugin-a-advanced-deep', parent: 'plugin-a-advanced', layer: -2 },
];

const TABLE = [
    {
        name: 'ground layer with no dive shows everything on the layer',
        input: { allEntries: ground, confinedPages: [], diveParent: null },
        expectedIds: ['plugins', 'hotkeys', 'shortcuts'],
    },
    {
        name: 'empty layer yields empty result regardless of dive state',
        input: { allEntries: [], confinedPages: ['whatever'], diveParent: 'plugins' },
        expectedIds: [],
    },
    {
        name: 'dive into plugins restricts to plugin sections only, ignoring sibling static subs on same layer',
        input: {
            allEntries: [...pluginSections, ...staticSubs],
            confinedPages: ['plugin-a-config', 'plugin-a-advanced'],
            diveParent: 'plugins',
        },
        expectedIds: ['plugin-a-config', 'plugin-a-advanced'],
    },
    {
        name: 'dive into hotkeys-editor restricts to hotkeys editor only, not plugin configs',
        input: {
            allEntries: [...pluginSections, ...staticSubs],
            confinedPages: ['hotkeys-editor'],
            diveParent: 'hotkeys',
        },
        expectedIds: ['hotkeys-editor'],
    },
    {
        name: 'confinedPages takes precedence over diveParent when both present',
        input: {
            allEntries: [...pluginSections, ...staticSubs],
            confinedPages: ['plugin-a-config'],
            diveParent: 'hotkeys',
        },
        expectedIds: ['plugin-a-config'],
    },
    {
        name: 'falls back to diveParent match when confinedPages is empty',
        input: {
            allEntries: [...pluginSections, ...staticSubs],
            confinedPages: [],
            diveParent: 'plugins',
        },
        expectedIds: ['plugin-a-config', 'plugin-a-advanced'],
    },
    {
        name: 'falls back to diveParent match when confinedPages is undefined',
        input: {
            allEntries: [...pluginSections, ...staticSubs],
            confinedPages: undefined,
            diveParent: 'hotkeys',
        },
        expectedIds: ['hotkeys-editor'],
    },
    {
        name: 'confinedPages with ids not present in allEntries yields empty (stale registry)',
        input: {
            allEntries: ground,
            confinedPages: ['ghost-page', 'also-ghost'],
            diveParent: null,
        },
        expectedIds: [],
    },
    {
        name: 'partial overlap: only matching ids survive',
        input: {
            allEntries: [...pluginSections, ...staticSubs],
            confinedPages: ['plugin-a-config', 'ghost', 'hotkeys-editor'],
            diveParent: null,
        },
        expectedIds: ['plugin-a-config', 'hotkeys-editor'],
    },
    {
        name: 'deep dive (layer -2) filters correctly when caller supplies that layer only',
        input: {
            allEntries: layerMinusTwo,
            confinedPages: ['plugin-a-config-deep'],
            diveParent: 'plugin-a-config',
        },
        expectedIds: ['plugin-a-config-deep'],
    },
    {
        name: 'ground fallback with null diveParent returns input untouched',
        input: { allEntries: ground, confinedPages: null, diveParent: null },
        expectedIds: ['plugins', 'hotkeys', 'shortcuts'],
    },
    {
        name: 'preserves input order',
        input: {
            allEntries: [
                { id: 'c', parent: 'p' }, { id: 'a', parent: 'p' }, { id: 'b', parent: 'p' },
            ],
            confinedPages: ['a', 'b', 'c'],
            diveParent: null,
        },
        expectedIds: ['c', 'a', 'b'],
    },
];

for (const row of TABLE) {
    test(`visibleMinimapEntries: ${row.name}`, () => {
        const result = visibleMinimapEntries(row.input);
        assert.deepEqual(result.map(e => e.id), row.expectedIds);
    });
}

// ---------------------------------------------------------------------------
// Property tests — 200 generated worlds per property. Invariants must hold
// at any depth, any layer, with arbitrary numbers of entries and siblings.
// ---------------------------------------------------------------------------

function makeRng(seed) {
    let s = seed >>> 0;
    return () => {
        s = (s * 1664525 + 1013904223) >>> 0;
        return s / 2 ** 32;
    };
}

function randInt(rng, min, max) {
    return min + Math.floor(rng() * (max - min + 1));
}

function genWorld(rng) {
    const numLayers = randInt(rng, 1, 5);
    const entries = [];
    const diveTargets = [];
    for (let layer = 0; layer > -numLayers; layer--) {
        const entriesOnLayer = randInt(rng, 1, 8);
        for (let i = 0; i < entriesOnLayer; i++) {
            const parent = layer === 0
                ? null
                : entries.find(e => e.layer === layer + 1)?.id ?? null;
            entries.push({
                id: `L${layer}-e${i}-${randInt(rng, 0, 1_000_000)}`,
                parent,
                layer,
            });
        }
        const parentsOnThisLayer = [...new Set(entries.filter(e => e.layer === layer).map(e => e.parent).filter(Boolean))];
        for (const p of parentsOnThisLayer) {
            const children = entries.filter(e => e.parent === p).map(e => e.id);
            if (children.length > 0) diveTargets.push({ parent: p, pages: children });
        }
    }
    return { entries, diveTargets };
}

function runProperty(name, check, cases = 200) {
    test(name, () => {
        for (let i = 0; i < cases; i++) {
            const rng = makeRng(0xC0FFEE + i);
            const world = genWorld(rng);
            check(world, i, rng);
        }
    });
}

runProperty('property: result is always a subset of the input entries', (world) => {
    for (const target of world.diveTargets) {
        const layer = world.entries.find(e => e.id === target.pages[0])?.layer;
        const allEntries = world.entries.filter(e => e.layer === layer);
        const result = visibleMinimapEntries({
            allEntries,
            confinedPages: target.pages,
            diveParent: target.parent,
        });
        const inputIds = new Set(allEntries.map(e => e.id));
        for (const e of result) assert.ok(inputIds.has(e.id), `leak: ${e.id} not in input`);
    }
});

runProperty('property: when confinedPages is provided, result ids are exactly (confinedPages ∩ allEntries)', (world) => {
    for (const target of world.diveTargets) {
        const layer = world.entries.find(e => e.id === target.pages[0])?.layer;
        const allEntries = world.entries.filter(e => e.layer === layer);
        const result = visibleMinimapEntries({
            allEntries,
            confinedPages: target.pages,
            diveParent: target.parent,
        });
        const expected = allEntries
            .filter(e => target.pages.includes(e.id))
            .map(e => e.id)
            .sort();
        const actual = result.map(e => e.id).sort();
        assert.deepEqual(actual, expected);
    }
});

runProperty('property: sibling dives on the same layer never leak into each other', (world) => {
    const byLayer = new Map();
    for (const t of world.diveTargets) {
        const layer = world.entries.find(e => e.id === t.pages[0])?.layer;
        if (layer == null) continue;
        if (!byLayer.has(layer)) byLayer.set(layer, []);
        byLayer.get(layer).push(t);
    }
    for (const [layer, targets] of byLayer) {
        if (targets.length < 2) continue;
        const allEntries = world.entries.filter(e => e.layer === layer);
        for (let i = 0; i < targets.length; i++) {
            for (let j = i + 1; j < targets.length; j++) {
                const a = visibleMinimapEntries({ allEntries, confinedPages: targets[i].pages, diveParent: targets[i].parent });
                const b = visibleMinimapEntries({ allEntries, confinedPages: targets[j].pages, diveParent: targets[j].parent });
                const aIds = new Set(a.map(e => e.id));
                for (const e of b) {
                    if (targets[i].pages.includes(e.id) && targets[j].pages.includes(e.id)) continue;
                    assert.ok(!aIds.has(e.id), `sibling leak: ${e.id} visible in both dives`);
                }
            }
        }
    }
});

runProperty('property: at any layer, with no dive, result equals allEntries of that layer', (world) => {
    const layers = [...new Set(world.entries.map(e => e.layer))];
    for (const layer of layers) {
        const allEntries = world.entries.filter(e => e.layer === layer);
        const result = visibleMinimapEntries({ allEntries, confinedPages: [], diveParent: null });
        assert.deepEqual(result.map(e => e.id), allEntries.map(e => e.id));
    }
});

runProperty('property: diveParent fallback yields only children of that parent on the given layer', (world) => {
    for (const target of world.diveTargets) {
        const layer = world.entries.find(e => e.id === target.pages[0])?.layer;
        const allEntries = world.entries.filter(e => e.layer === layer);
        const result = visibleMinimapEntries({ allEntries, confinedPages: [], diveParent: target.parent });
        for (const e of result) {
            assert.equal(e.parent, target.parent, `diveParent leak: ${e.id} has parent=${e.parent}, expected ${target.parent}`);
        }
    }
});

runProperty('property: empty confinedPages with null diveParent is identity', (world) => {
    const allEntries = world.entries;
    const result = visibleMinimapEntries({ allEntries, confinedPages: [], diveParent: null });
    assert.equal(result, allEntries);
});

// ---------------------------------------------------------------------------
// sliceMinimapRange contract tests.
// ---------------------------------------------------------------------------

const E = (id, x, width = 1000) => ({ id, x, width });

const RANGE_TABLE = [
    {
        name: 'empty list short-circuits',
        input: { entries: [], worldStart: 0, worldEnd: 100 },
        expected: [],
    },
    {
        name: 'invalid range (NaN start) returns identity',
        input: { entries: [E('a', 0), E('b', 100)], worldStart: NaN, worldEnd: 100 },
        expected: ['a', 'b'],
    },
    {
        name: 'invalid range (end <= start) returns identity',
        input: { entries: [E('a', 0), E('b', 100)], worldStart: 50, worldEnd: 50 },
        expected: ['a', 'b'],
    },
    {
        name: 'range covering everything yields everything',
        input: { entries: [E('a', 0), E('b', 5000), E('c', 10000)], worldStart: -1e9, worldEnd: 1e9 },
        expected: ['a', 'b', 'c'],
    },
    {
        name: 'range strictly between two entries yields empty',
        input: { entries: [E('a', 0, 100), E('b', 5000, 100)], worldStart: 200, worldEnd: 4900 },
        expected: [],
    },
    {
        name: 'partial left-overlap: entry trailing edge intersects range',
        input: { entries: [E('a', 0, 1000), E('b', 5000, 1000)], worldStart: 800, worldEnd: 1200 },
        expected: ['a'],
    },
    {
        name: 'partial right-overlap: entry leading edge intersects range',
        input: { entries: [E('a', 0, 1000), E('b', 5000, 1000)], worldStart: 4500, worldEnd: 5500 },
        expected: ['b'],
    },
    {
        name: 'tangent on right edge (entry.x === worldEnd) excluded',
        input: { entries: [E('a', 0, 100), E('b', 200, 100)], worldStart: 0, worldEnd: 200 },
        expected: ['a'],
    },
    {
        name: 'tangent on left edge (entry.x+width === worldStart) excluded',
        input: { entries: [E('a', 0, 100), E('b', 200, 100)], worldStart: 100, worldEnd: 300 },
        expected: ['b'],
    },
    {
        name: 'preserves input order',
        input: { entries: [E('c', 200, 50), E('a', 0, 50), E('b', 100, 50)], worldStart: -10, worldEnd: 1000 },
        expected: ['c', 'a', 'b'],
    },
    {
        name: 'wide entry: range fully inside one entry yields that one',
        input: { entries: [E('a', 0, 10000)], worldStart: 4000, worldEnd: 6000 },
        expected: ['a'],
    },
];

for (const row of RANGE_TABLE) {
    test(`sliceMinimapRange: ${row.name}`, () => {
        const result = sliceMinimapRange({
            entries: row.input.entries,
            worldStart: row.input.worldStart,
            worldEnd: row.input.worldEnd,
        });
        assert.deepEqual(result.map(e => e.id), row.expected);
    });
}

test('sliceMinimapRange: factor 1 with viewport on a page keeps that page', () => {
    const entries = Array.from({ length: 10 }, (_, i) => E(`e${i}`, i * 10000, 1280));
    // simulate: viewport zoom 1, viewport width 1280, camera parked on entry 4
    const cameraX = entries[4].x;
    const viewportRange = 1280;
    const center = cameraX + viewportRange / 2;
    const result = sliceMinimapRange({
        entries,
        worldStart: center - viewportRange / 2,
        worldEnd: center + viewportRange / 2,
    });
    assert.deepEqual(result.map(e => e.id), ['e4']);
});

test('sliceMinimapRange: as factor grows from 1 to "all", visible count is monotonic', () => {
    const entries = Array.from({ length: 10 }, (_, i) => E(`e${i}`, i * 10000, 1280));
    const cameraX = entries[4].x;
    const viewportRange = 1280;
    const center = cameraX + viewportRange / 2;
    let prev = 0;
    for (const factor of [1, 2, 4, 8, 16, 32, 64, 128]) {
        const half = (viewportRange * factor) / 2;
        const result = sliceMinimapRange({
            entries,
            worldStart: center - half,
            worldEnd: center + half,
        });
        assert.ok(result.length >= prev, `count regressed at factor=${factor}`);
        prev = result.length;
    }
    assert.equal(prev, entries.length);
});

runProperty('property: n-deep chain of dives — each level only sees its registered children', (world) => {
    let chain = world.entries.filter(e => e.layer === 0);
    let depth = 0;
    while (chain.length > 0 && depth < 6) {
        const anchor = chain[0];
        const nextLayer = anchor.layer - 1;
        const children = world.entries.filter(e => e.parent === anchor.id && e.layer === nextLayer);
        if (children.length === 0) break;
        const allEntries = world.entries.filter(e => e.layer === nextLayer);
        const result = visibleMinimapEntries({
            allEntries,
            confinedPages: children.map(c => c.id),
            diveParent: anchor.id,
        });
        assert.deepEqual(
            result.map(e => e.id).sort(),
            children.map(e => e.id).sort(),
            `depth=${depth} anchor=${anchor.id}`,
        );
        chain = children;
        depth++;
    }
});
