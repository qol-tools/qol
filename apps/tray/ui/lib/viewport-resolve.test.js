import { test } from 'node:test';
import assert from 'node:assert/strict';
import { resolveViewport } from './viewport-resolve.js';

function makeEl({ id = 'viewport', isConnected = true, clientWidth = 1752, clientHeight = 809 } = {}) {
    return { id, isConnected, clientWidth, clientHeight };
}

function makeDoc(freshEl) {
    let queried = 0;
    return {
        get queryCount() { return queried; },
        getElementById(id) {
            queried += 1;
            return id === 'viewport' ? freshEl : null;
        },
    };
}

test('returns cached element when it is connected and has non-zero width', () => {
    const cached = makeEl({ clientWidth: 1752 });
    const ref = { current: cached };
    const doc = makeDoc(makeEl({ clientWidth: 999 }));
    const result = resolveViewport(ref, doc);
    assert.equal(result, cached);
    assert.equal(ref.current, cached, 'cache must not be overwritten');
    assert.equal(doc.queryCount, 0, 'DOM must not be queried when cache is healthy');
});

test('re-resolves from DOM when cached element is detached (isConnected=false)', () => {
    const detached = makeEl({ isConnected: false, clientWidth: 0 });
    const fresh = makeEl({ isConnected: true, clientWidth: 1752 });
    const ref = { current: detached };
    const doc = makeDoc(fresh);
    const result = resolveViewport(ref, doc);
    assert.equal(result, fresh);
    assert.equal(ref.current, fresh, 'cache must be overwritten with fresh node');
    assert.equal(doc.queryCount, 1);
});

test('re-resolves from DOM when cached element has zero clientWidth', () => {
    const stale = makeEl({ isConnected: true, clientWidth: 0 });
    const fresh = makeEl({ isConnected: true, clientWidth: 1752 });
    const ref = { current: stale };
    const doc = makeDoc(fresh);
    const result = resolveViewport(ref, doc);
    assert.equal(result, fresh);
    assert.equal(ref.current, fresh);
});

test('re-resolves from DOM when cached element is null', () => {
    const fresh = makeEl({ clientWidth: 1752 });
    const ref = { current: null };
    const doc = makeDoc(fresh);
    const result = resolveViewport(ref, doc);
    assert.equal(result, fresh);
    assert.equal(ref.current, fresh);
});

test('returns null and leaves cache untouched when DOM has no #viewport either', () => {
    const detached = makeEl({ isConnected: false, clientWidth: 0 });
    const ref = { current: detached };
    const doc = makeDoc(null);
    const result = resolveViewport(ref, doc);
    assert.equal(result, null);
    assert.equal(ref.current, detached, 'should not overwrite cache with null');
});

test('handles missing ref argument', () => {
    const fresh = makeEl({ clientWidth: 1752 });
    const doc = makeDoc(fresh);
    const result = resolveViewport(undefined, doc);
    assert.equal(result, fresh);
});

test('handles missing doc (no DOM available)', () => {
    const detached = makeEl({ isConnected: false, clientWidth: 0 });
    const ref = { current: detached };
    const result = resolveViewport(ref, null);
    assert.equal(result, null);
    assert.equal(ref.current, detached);
});

test('regression: post-dive detached cache → recovers non-zero clientWidth', () => {
    const preDive = makeEl({ isConnected: true, clientWidth: 1752, clientHeight: 809 });
    const ref = { current: preDive };
    const doc = makeDoc(preDive);
    assert.equal(resolveViewport(ref, doc).clientWidth, 1752);

    preDive.isConnected = false;
    preDive.clientWidth = 0;
    preDive.clientHeight = 0;
    const postAscend = makeEl({ isConnected: true, clientWidth: 1752, clientHeight: 809 });
    const doc2 = makeDoc(postAscend);

    const recovered = resolveViewport(ref, doc2);
    assert.equal(recovered, postAscend);
    assert.equal(recovered.clientWidth, 1752);
    assert.equal(recovered.clientHeight, 809);
    assert.equal(ref.current, postAscend, 'ref must be patched so other consumers (camera, nav) recover too');
});
