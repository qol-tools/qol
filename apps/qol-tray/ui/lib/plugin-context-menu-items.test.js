import { test } from 'node:test';
import assert from 'node:assert/strict';
import { pluginContextMenuItems, dispatchPluginContextAction } from './plugin-context-menu-items.js';

// ---------------------------------------------------------------------------
// Exact-output table: every combination of capability flags is an observable
// contract. The menu MUST always offer Delete, MUST offer Update only when
// plugin.update_available is truthy, and MUST offer Config only when
// plugin.has_config is truthy. Order is fixed: Update, Config, Delete.
// ---------------------------------------------------------------------------

const TABLE = [
    {
        name: 'null plugin yields empty list',
        plugin: null,
        expected: [],
    },
    {
        name: 'undefined plugin yields empty list',
        plugin: undefined,
        expected: [],
    },
    {
        name: 'plugin with no capabilities exposes only Delete',
        plugin: {},
        expected: ['delete'],
    },
    {
        name: 'plugin with update_available exposes Update and Delete',
        plugin: { update_available: true },
        expected: ['update', 'delete'],
    },
    {
        name: 'plugin with has_config exposes Config and Delete',
        plugin: { has_config: true },
        expected: ['config', 'delete'],
    },
    {
        name: 'plugin with both flags exposes Update, Config, Delete in order',
        plugin: { update_available: true, has_config: true },
        expected: ['update', 'config', 'delete'],
    },
    {
        name: 'falsy flag values (false) are treated as hidden',
        plugin: { update_available: false, has_config: false },
        expected: ['delete'],
    },
    {
        name: 'flag value of 0 is treated as hidden',
        plugin: { update_available: 0, has_config: 0 },
        expected: ['delete'],
    },
    {
        name: 'flag value of empty string is treated as hidden',
        plugin: { update_available: '', has_config: '' },
        expected: ['delete'],
    },
    {
        name: 'truthy non-boolean flags (1, "yes") reveal items',
        plugin: { update_available: 1, has_config: 'yes' },
        expected: ['update', 'config', 'delete'],
    },
    {
        name: 'extra plugin fields are ignored',
        plugin: { update_available: true, has_config: true, name: 'Foo', id: 'foo' },
        expected: ['update', 'config', 'delete'],
    },
];

for (const row of TABLE) {
    test(row.name, () => {
        const actual = pluginContextMenuItems(row.plugin);
        assert.deepEqual(actual.map(i => i.id), row.expected);
    });
}

test('every emitted item has id, label, and className', () => {
    const plugin = { update_available: true, has_config: true };
    const items = pluginContextMenuItems(plugin);
    assert.equal(items.length, 3);
    for (const item of items) {
        assert.equal(typeof item.id, 'string');
        assert.equal(typeof item.label, 'string');
        assert.equal(typeof item.className, 'string');
        assert.ok(item.id.length > 0);
        assert.ok(item.label.length > 0);
        assert.ok(item.className.length > 0);
    }
});

test('className values match the existing CSS hooks', () => {
    const items = pluginContextMenuItems({ update_available: true, has_config: true });
    const byId = Object.fromEntries(items.map(i => [i.id, i]));
    assert.equal(byId.update.className, 'context-update');
    assert.equal(byId.config.className, 'context-config');
    assert.equal(byId.delete.className, 'context-delete');
});

test('labels match the displayed text', () => {
    const items = pluginContextMenuItems({ update_available: true, has_config: true });
    const byId = Object.fromEntries(items.map(i => [i.id, i]));
    assert.equal(byId.update.label, 'Update');
    assert.equal(byId.config.label, 'Config');
    assert.equal(byId.delete.label, 'Delete');
});

test('the returned array is fresh on every call (no shared mutation risk)', () => {
    const a = pluginContextMenuItems({ update_available: true, has_config: true });
    const b = pluginContextMenuItems({ update_available: true, has_config: true });
    assert.notEqual(a, b);
    a.pop();
    assert.equal(b.length, 3);
});

// ---------------------------------------------------------------------------
// Dispatch table: every known action routes to the right handler with the
// expected side effects, and unknown ids no-op without throwing.
// ---------------------------------------------------------------------------

function makeCtx() {
    const calls = [];
    const actions = {
        updatePlugin: (id) => calls.push(['updatePlugin', id]),
        focusSelectedCard: () => calls.push(['focusSelectedCard']),
        openConfig: () => calls.push(['openConfig']),
    };
    const modal = {
        setConfirmPluginId: (id) => calls.push(['setConfirmPluginId', id]),
    };
    return { ctx: { actions, modal }, calls };
}

test('dispatch: update runs updatePlugin then focusSelectedCard', () => {
    const { ctx, calls } = makeCtx();
    const ok = dispatchPluginContextAction('update', 'plugin-a', ctx);
    assert.equal(ok, true);
    assert.deepEqual(calls, [['updatePlugin', 'plugin-a'], ['focusSelectedCard']]);
});

test('dispatch: config runs openConfig', () => {
    const { ctx, calls } = makeCtx();
    assert.equal(dispatchPluginContextAction('config', 'plugin-b', ctx), true);
    assert.deepEqual(calls, [['openConfig']]);
});

test('dispatch: delete sets confirm plugin id', () => {
    const { ctx, calls } = makeCtx();
    assert.equal(dispatchPluginContextAction('delete', 'plugin-c', ctx), true);
    assert.deepEqual(calls, [['setConfirmPluginId', 'plugin-c']]);
});

test('dispatch: unknown action id returns false and triggers no handlers', () => {
    const { ctx, calls } = makeCtx();
    assert.equal(dispatchPluginContextAction('nope', 'plugin-x', ctx), false);
    assert.deepEqual(calls, []);
});

test('dispatch: every visible menu item has a matching dispatch entry', () => {
    // Guard against the menu data and the dispatcher drifting apart.
    const plugin = { update_available: true, has_config: true };
    const items = pluginContextMenuItems(plugin);
    const { ctx } = makeCtx();
    for (const item of items) {
        assert.equal(
            dispatchPluginContextAction(item.id, 'plugin-x', ctx),
            true,
            `menu item "${item.id}" has no handler`,
        );
    }
});
