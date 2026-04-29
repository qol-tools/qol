import { test } from 'node:test';
import assert from 'node:assert/strict';
import { sliceMinimapWindow, visibleMinimapEntries } from './minimap-filter.js';

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
// sliceMinimapWindow contract tests.
// ---------------------------------------------------------------------------

const SLICE_TABLE = [
    {
        name: 'radius 0 returns input untouched',
        input: { entries: ['a', 'b', 'c', 'd', 'e'], activeId: 'c', radius: 0 },
        expected: ['a', 'b', 'c', 'd', 'e'],
    },
    {
        name: 'negative radius returns input untouched',
        input: { entries: ['a', 'b', 'c'], activeId: 'b', radius: -2 },
        expected: ['a', 'b', 'c'],
    },
    {
        name: 'window larger than entries returns all',
        input: { entries: ['a', 'b', 'c'], activeId: 'b', radius: 5 },
        expected: ['a', 'b', 'c'],
    },
    {
        name: 'centred window: active in middle of long list',
        input: { entries: ['a', 'b', 'c', 'd', 'e', 'f', 'g'], activeId: 'd', radius: 2 },
        expected: ['b', 'c', 'd', 'e', 'f'],
    },
    {
        name: 'left-edge slide: active at index 0 produces leading window',
        input: { entries: ['a', 'b', 'c', 'd', 'e', 'f', 'g'], activeId: 'a', radius: 2 },
        expected: ['a', 'b', 'c', 'd', 'e'],
    },
    {
        name: 'right-edge slide: active at last index produces trailing window',
        input: { entries: ['a', 'b', 'c', 'd', 'e', 'f', 'g'], activeId: 'g', radius: 2 },
        expected: ['c', 'd', 'e', 'f', 'g'],
    },
    {
        name: 'one off the right edge still produces 5 entries',
        input: { entries: ['a', 'b', 'c', 'd', 'e', 'f', 'g'], activeId: 'f', radius: 2 },
        expected: ['c', 'd', 'e', 'f', 'g'],
    },
    {
        name: 'unknown activeId returns leading window',
        input: { entries: ['a', 'b', 'c', 'd', 'e'], activeId: 'ghost', radius: 1 },
        expected: ['a', 'b', 'c'],
    },
    {
        name: 'null activeId returns leading window',
        input: { entries: ['a', 'b', 'c', 'd', 'e'], activeId: null, radius: 1 },
        expected: ['a', 'b', 'c'],
    },
    {
        name: 'empty list short-circuits',
        input: { entries: [], activeId: 'x', radius: 2 },
        expected: [],
    },
    {
        name: 'fractional radius floors',
        input: { entries: ['a', 'b', 'c', 'd', 'e'], activeId: 'c', radius: 1.9 },
        expected: ['b', 'c', 'd'],
    },
];

for (const row of SLICE_TABLE) {
    test(`sliceMinimapWindow: ${row.name}`, () => {
        const result = sliceMinimapWindow({
            entries: row.input.entries.map(id => ({ id })),
            activeId: row.input.activeId,
            radius: row.input.radius,
        });
        assert.deepEqual(result.map(e => e.id), row.expected);
    });
}

test('sliceMinimapWindow: result always contains activeId when found', () => {
    const entries = Array.from({ length: 10 }, (_, i) => ({ id: `e${i}` }));
    for (let i = 0; i < entries.length; i++) {
        for (let r = 1; r <= 4; r++) {
            const result = sliceMinimapWindow({ entries, activeId: `e${i}`, radius: r });
            assert.ok(result.some(e => e.id === `e${i}`), `lost active e${i} at radius=${r}`);
            const expectedSize = Math.min(r * 2 + 1, entries.length);
            assert.equal(result.length, expectedSize, `wrong size at i=${i} r=${r}`);
        }
    }
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
