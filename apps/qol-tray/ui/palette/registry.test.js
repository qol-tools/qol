// Locks down the palette command registry: per-view registrations must surface
// alongside global commands when the palette queries the active view's bucket.
// Regression hook for the world-canvas overhaul, where per-view actions vanished
// from the Ctrl+E palette while globals still appeared.

import { test } from 'node:test';
import assert from 'node:assert/strict';

import {
    registerCommands,
    unregisterCommands,
    getCommands,
    getContextualCommands,
    getRegistryVersion,
    subscribeRegistry,
    GLOBAL_ID,
} from './registry.js';

function clearKnown(triples) {
    for (const [viewId, scope] of triples) unregisterCommands(viewId, scope);
}

test('per-view registration appears in getCommands(viewId)', () => {
    const scope = Symbol('s1');
    registerCommands('plugins', scope, [{ id: 'a', label: 'A', run: () => {} }]);
    const out = getCommands('plugins');
    assert.equal(out.length, 1);
    assert.equal(out[0].id, 'a');
    unregisterCommands('plugins', scope);
});

test('per-view commands precede globals in getCommands result', () => {
    const sP = Symbol('p');
    const sG = Symbol('g');
    registerCommands('plugins', sP, [{ id: 'p:1', label: 'P1', run: () => {} }]);
    registerCommands(GLOBAL_ID, sG, [{ id: 'g:1', label: 'G1', run: () => {} }]);
    const ids = getCommands('plugins').map(c => c.id);
    assert.deepEqual(ids, ['p:1', 'g:1']);
    clearKnown([['plugins', sP], [GLOBAL_ID, sG]]);
});

test('querying a view that has no bucket returns only globals', () => {
    const sP = Symbol('p');
    const sG = Symbol('g');
    registerCommands('plugins', sP, [{ id: 'p:1', label: 'P1', run: () => {} }]);
    registerCommands(GLOBAL_ID, sG, [{ id: 'g:1', label: 'G1', run: () => {} }]);
    const ids = getCommands('logs').map(c => c.id);
    assert.deepEqual(ids, ['g:1']);
    clearKnown([['plugins', sP], [GLOBAL_ID, sG]]);
});

test('multiple scopes under the same viewId all surface', () => {
    const a = Symbol('a');
    const b = Symbol('b');
    registerCommands('hotkeys', a, [{ id: 'h:add', label: 'Add', run: () => {} }]);
    registerCommands('hotkeys', b, [{ id: 'h:edit', label: 'Edit', run: () => {} }]);
    const ids = getCommands('hotkeys').map(c => c.id).sort();
    assert.deepEqual(ids, ['h:add', 'h:edit']);
    clearKnown([['hotkeys', a], ['hotkeys', b]]);
});

test('unregister of one scope leaves other scopes intact', () => {
    const a = Symbol('a');
    const b = Symbol('b');
    registerCommands('store', a, [{ id: 's:install', label: 'Install', run: () => {} }]);
    registerCommands('store', b, [{ id: 's:refresh', label: 'Refresh', run: () => {} }]);
    unregisterCommands('store', a);
    const ids = getCommands('store').map(c => c.id);
    assert.deepEqual(ids, ['s:refresh']);
    clearKnown([['store', b]]);
});

test('subscribeRegistry fires on register and unregister', () => {
    const events = [];
    const unsub = subscribeRegistry(v => events.push(v));
    const before = getRegistryVersion();
    const scope = Symbol('s');
    registerCommands('logs', scope, [{ id: 'l:r', label: 'Refresh', run: () => {} }]);
    unregisterCommands('logs', scope);
    unsub();
    assert.equal(events.length, 2);
    assert.equal(events[0], before + 1);
    assert.equal(events[1], before + 2);
});

test('getContextualCommands returns only the active view bucket when populated', () => {
    const sP = Symbol('p');
    const sG = Symbol('g');
    registerCommands('hotkeys', sP, [
        { id: 'hk:add', label: 'Add hotkey', run: () => {} },
        { id: 'hk:edit', label: 'Edit hotkey', run: () => {} },
        { id: 'hk:delete', label: 'Delete hotkey', run: () => {} },
    ]);
    registerCommands(GLOBAL_ID, sG, [
        { id: 'g:nav:plugins', label: 'Go to Plugins', run: () => {} },
        { id: 'g:nav:logs', label: 'Go to Logs', run: () => {} },
        { id: 'g:cfg:export', label: 'Export configuration', run: () => {} },
    ]);
    const ids = getContextualCommands('hotkeys').map(c => c.id);
    assert.deepEqual(ids, ['hk:add', 'hk:edit', 'hk:delete']);
    clearKnown([['hotkeys', sP], [GLOBAL_ID, sG]]);
});

test('getContextualCommands falls back to globals when active view has no commands', () => {
    const sG = Symbol('g');
    registerCommands(GLOBAL_ID, sG, [
        { id: 'g:nav:plugins', label: 'Go to Plugins', run: () => {} },
    ]);
    const ids = getContextualCommands('view-with-no-commands').map(c => c.id);
    assert.deepEqual(ids, ['g:nav:plugins']);
    clearKnown([[GLOBAL_ID, sG]]);
});

test('getContextualCommands returns empty when neither view nor globals registered', () => {
    assert.deepEqual(getContextualCommands('nope'), []);
});

// Property test: simulated full mount flow where N views each register K commands
// always yields per-view commands first followed by globals. Locks the bucket
// ordering invariant the palette depends on.
function rng(seed) {
    let s = seed >>> 0;
    return () => {
        s = (s * 1664525 + 1013904223) >>> 0;
        return s / 0x100000000;
    };
}

test('property: per-view commands precede globals across randomized mount orders', () => {
    const r = rng(0xc0ffee);
    for (let trial = 0; trial < 200; trial++) {
        const viewIds = [`v${trial}a`, `v${trial}b`, `v${trial}c`];
        const scopes = viewIds.map(() => Symbol());
        const globalScope = Symbol('g');
        const numCommands = Math.max(1, Math.floor(r() * 4));
        const numGlobals = Math.max(1, Math.floor(r() * 4));
        const order = viewIds.map((_, i) => i);
        for (let i = order.length - 1; i > 0; i--) {
            const j = Math.floor(r() * (i + 1));
            [order[i], order[j]] = [order[j], order[i]];
        }
        for (const i of order) {
            const cmds = Array.from({ length: numCommands }, (_, k) => ({
                id: `${viewIds[i]}:${k}`, label: `L${k}`, run: () => {},
            }));
            registerCommands(viewIds[i], scopes[i], cmds);
        }
        const globals = Array.from({ length: numGlobals }, (_, k) => ({
            id: `g:${trial}:${k}`, label: `G${k}`, run: () => {},
        }));
        registerCommands(GLOBAL_ID, globalScope, globals);

        const target = viewIds[Math.floor(r() * viewIds.length)];
        const out = getCommands(target);
        assert.equal(out.length, numCommands + numGlobals,
            `trial ${trial}: expected ${numCommands + numGlobals}, got ${out.length}`);
        for (let k = 0; k < numCommands; k++) {
            assert.ok(out[k].id.startsWith(`${target}:`),
                `trial ${trial}: per-view cmd ${k} not first`);
        }
        for (let k = 0; k < numGlobals; k++) {
            assert.ok(out[numCommands + k].id.startsWith('g:'),
                `trial ${trial}: global cmd ${k} not after per-view`);
        }

        for (let i = 0; i < viewIds.length; i++) unregisterCommands(viewIds[i], scopes[i]);
        unregisterCommands(GLOBAL_ID, globalScope);
    }
});
