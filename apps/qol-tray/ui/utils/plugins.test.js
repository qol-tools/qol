import { test } from 'node:test';
import assert from 'node:assert/strict';
import { samePluginList } from './plugins.js';

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
