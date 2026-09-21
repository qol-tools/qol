import { test } from 'node:test';
import assert from 'node:assert/strict';
import { applyNavigateRoute } from './deeplink-navigate.js';

test('sets hash from a bare route', () => {
    const loc = { hash: '' };
    assert.equal(applyNavigateRoute('shortcuts/add?type=url', loc), true);
    assert.equal(loc.hash, '#shortcuts/add?type=url');
});

test('trims a leading hash or slash before re-prefixing', () => {
    const a = { hash: '' };
    applyNavigateRoute('#shortcuts', a);
    assert.equal(a.hash, '#shortcuts');
    const b = { hash: '' };
    applyNavigateRoute('/shortcuts', b);
    assert.equal(b.hash, '#shortcuts');
});

test('empty / whitespace / non-string is a no-op', () => {
    const loc = { hash: '#untouched' };
    assert.equal(applyNavigateRoute('', loc), false);
    assert.equal(applyNavigateRoute('   ', loc), false);
    assert.equal(applyNavigateRoute(null, loc), false);
    assert.equal(applyNavigateRoute(undefined, loc), false);
    assert.equal(loc.hash, '#untouched');
});

test('no location available is a safe no-op', () => {
    assert.equal(applyNavigateRoute('shortcuts', undefined), false);
});

test('identical hash dispatches a synthetic hashchange instead of a silent no-op', () => {
    const loc = { hash: '#shortcuts/add?type=url' };
    const dispatched = [];
    const win = {
        dispatchEvent: (e) => { dispatched.push(e.type); return true; },
        Event: class { constructor(type) { this.type = type; } },
    };
    assert.equal(applyNavigateRoute('shortcuts/add?type=url', loc, win), true);
    assert.equal(loc.hash, '#shortcuts/add?type=url');
    assert.deepEqual(dispatched, ['hashchange']);
});

test('changed hash assigns without a synthetic event (browser fires its own)', () => {
    const loc = { hash: '#plugins' };
    const dispatched = [];
    const win = {
        dispatchEvent: (e) => { dispatched.push(e.type); return true; },
        Event: class { constructor(type) { this.type = type; } },
    };
    assert.equal(applyNavigateRoute('shortcuts/add?type=url', loc, win), true);
    assert.equal(loc.hash, '#shortcuts/add?type=url');
    assert.deepEqual(dispatched, []);
});
