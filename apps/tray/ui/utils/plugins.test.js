import { test } from 'node:test';
import assert from 'node:assert/strict';
import { samePluginList, markPluginUpdated, formatPushStatus } from './plugins.js';

test('samePluginList is true for identical reference and identical content', () => {
    const list = [{ id: 'a', version: '1.0.0', update_available: false }];
    assert.equal(samePluginList(list, list), true);
    assert.equal(
        samePluginList(
            [{ id: 'a', version: '1.0.0', update_available: false }],
            [{ id: 'a', version: '1.0.0', update_available: false }]
        ),
        true
    );
});

test('samePluginList detects length, order, and field changes', () => {
    const base = [{ id: 'a', version: '1.0.0' }, { id: 'b', version: '2.0.0' }];
    assert.equal(samePluginList(base, [{ id: 'a', version: '1.0.0' }]), false, 'length');
    assert.equal(samePluginList(base, [{ id: 'b', version: '2.0.0' }, { id: 'a', version: '1.0.0' }]), false, 'order');
    assert.equal(samePluginList(base, [{ id: 'a', version: '1.0.1' }, { id: 'b', version: '2.0.0' }]), false, 'version field');
    assert.equal(
        samePluginList(
            [{ id: 'a', update_available: false }],
            [{ id: 'a', update_available: true }]
        ),
        false,
        'update_available flag'
    );
});

test('samePluginList ignores keys outside the render-relevant set', () => {
    assert.equal(
        samePluginList(
            [{ id: 'a', version: '1.0.0', _internal: 1 }],
            [{ id: 'a', version: '1.0.0', _internal: 2 }]
        ),
        true
    );
});

test('samePluginList handles empty lists and non-arrays', () => {
    assert.equal(samePluginList([], []), true);
    assert.equal(samePluginList(null, []), false);
    assert.equal(samePluginList([], null), false);
    assert.equal(samePluginList(undefined, undefined), true);
});

test('markPluginUpdated clears the update flag and bumps versions for the matching plugin only', () => {
    const before = [
        { id: 'a', update_available: true, available_version: '2.0.0', running_version: '1.0.0', installed_version: '1.0.0', version: '1.0.0' },
        { id: 'b', update_available: true, available_version: '3.0.0', installed_version: '2.5.0', version: '2.5.0' },
    ];
    const after = markPluginUpdated(before, 'a');
    const cases = [
        [after[0].update_available, false, 'a no longer updatable'],
        [after[0].running_version, '2.0.0', 'a running_version bumped to available'],
        [after[0].installed_version, '2.0.0', 'a installed_version bumped to available'],
        [after[0].version, '2.0.0', 'a version bumped to available'],
        [after[1], before[1], 'b untouched (same reference)'],
        [before[0].update_available, true, 'original list not mutated'],
    ];
    for (const [actual, expected, label] of cases) {
        assert.equal(actual, expected, label);
    }
    assert.notEqual(after, before, 'returns a new array');
});

test('markPluginUpdated falls back to the current version when available_version is absent', () => {
    const after = markPluginUpdated([{ id: 'a', update_available: true, installed_version: '1.0.0', version: '1.0.0' }], 'a');
    assert.equal(after[0].update_available, false, 'flag cleared');
    assert.equal(after[0].installed_version, '1.0.0', 'installed_version kept');
    assert.equal(after[0].version, '1.0.0', 'version kept');
});

test('markPluginUpdated is a no-op that preserves identity for an unknown id', () => {
    const before = [{ id: 'a', update_available: true }];
    assert.equal(markPluginUpdated(before, 'zzz'), before);
});

test('formatPushStatus prefers the plugin-supplied text', () => {
    assert.equal(formatPushStatus({ text: '  Recording 0:12  ', state: 'recording' }), 'Recording 0:12');
    assert.equal(formatPushStatus('Idle'), 'Idle');
    assert.equal(formatPushStatus(3), '3');
});

test('formatPushStatus renders scalar fields as readable pairs without JSON punctuation', () => {
    const label = formatPushStatus({ state: 'recording', elapsed_s: 12, sink: { id: 1 }, note: null });
    assert.equal(label, 'state: recording · elapsed s: 12');
    assert.ok(!label.includes('{'), 'no JSON punctuation reaches the card');
});

test('formatPushStatus yields an empty label for shapes with nothing to show', () => {
    for (const status of [null, undefined, {}, [], [1, 2], { nested: { a: 1 } }]) {
        assert.equal(formatPushStatus(status), '', JSON.stringify(status ?? null));
    }
});
