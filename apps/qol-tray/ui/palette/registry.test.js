import { test } from 'node:test';
import assert from 'node:assert/strict';

import {
    registerCommands,
    unregisterCommands,
    getContextualCommands,
    getRegistryVersion,
    subscribeRegistry,
    GLOBAL_ID,
} from './registry.js';

function clearKnown(triples) {
    for (const [viewId, scope] of triples) unregisterCommands(viewId, scope);
}

test('multiple scopes under the same viewId all surface', () => {
    const a = Symbol('a');
    const b = Symbol('b');
    registerCommands('hotkeys', a, [{ id: 'h:add', label: 'Add', run: () => {} }]);
    registerCommands('hotkeys', b, [{ id: 'h:edit', label: 'Edit', run: () => {} }]);
    const ids = getContextualCommands('hotkeys').map(c => c.id).sort();
    assert.deepEqual(ids, ['h:add', 'h:edit']);
    clearKnown([['hotkeys', a], ['hotkeys', b]]);
});

test('unregister of one scope leaves other scopes intact', () => {
    const a = Symbol('a');
    const b = Symbol('b');
    registerCommands('store', a, [{ id: 's:install', label: 'Install', run: () => {} }]);
    registerCommands('store', b, [{ id: 's:refresh', label: 'Refresh', run: () => {} }]);
    unregisterCommands('store', a);
    const ids = getContextualCommands('store').map(c => c.id);
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
