import { test } from 'node:test';
import assert from 'node:assert/strict';
import { parseDeepRoute } from './deeplink-route.js';

test('page only', () => {
    assert.deepEqual(parseDeepRoute('#shortcuts'), { page: 'shortcuts', action: null, params: {} });
});

test('leading hash optional', () => {
    assert.deepEqual(parseDeepRoute('shortcuts'), { page: 'shortcuts', action: null, params: {} });
});

test('page + action + params', () => {
    assert.deepEqual(
        parseDeepRoute('#shortcuts/add?type=url&url=https%3A%2F%2Fx.io&name=X'),
        { page: 'shortcuts', action: 'add', params: { type: 'url', url: 'https://x.io', name: 'X' } }
    );
});

test('empty / nullish -> null page', () => {
    assert.deepEqual(parseDeepRoute(''), { page: null, action: null, params: {} });
    assert.deepEqual(parseDeepRoute(undefined), { page: null, action: null, params: {} });
});

test('trailing segments beyond action are ignored for v1', () => {
    const r = parseDeepRoute('#plugins/foo/config');
    assert.equal(r.page, 'plugins');
    assert.equal(r.action, 'foo');
});
