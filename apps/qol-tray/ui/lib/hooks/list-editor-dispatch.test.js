import { test } from 'node:test';
import assert from 'node:assert/strict';
import { composeListEditorHandler } from './list-editor-dispatch.js';

function fakeEvent(k, extra = {}) {
    return {
        key: k,
        ctrlKey: false, metaKey: false, altKey: false, shiftKey: false,
        ...extra,
        preventDefault: () => {},
    };
}

function makeHandler(overrides = {}) {
    const calls = { modal: 0, list: 0 };
    const h = composeListEditorHandler({
        modalRef: { current: null },
        onModal: () => { calls.modal++; },
        onList: () => { calls.list++; },
        ...overrides,
    });
    return { h, calls };
}

test('modal open: routes to onModal, not onList', () => {
    const { h, calls } = makeHandler({ modalRef: { current: {} } });
    h(fakeEvent('Enter'));
    assert.equal(calls.modal, 1);
    assert.equal(calls.list, 0);
});

test('modal closed: routes to onList', () => {
    const { h, calls } = makeHandler();
    h(fakeEvent('a'));
    assert.equal(calls.list, 1);
    assert.equal(calls.modal, 0);
});

test('preIntercept returning true stops before modal check', () => {
    const { h, calls } = makeHandler({
        modalRef: { current: {} },
        preIntercept: () => true,
    });
    h(fakeEvent('x'));
    assert.equal(calls.modal, 0, 'preIntercept should stop before modal');
    assert.equal(calls.list, 0);
});

test('preIntercept returning false continues to modal', () => {
    const { h, calls } = makeHandler({
        modalRef: { current: {} },
        preIntercept: () => false,
    });
    h(fakeEvent('x'));
    assert.equal(calls.modal, 1);
});

test('listIntercept returning true stops before onList', () => {
    let intercepted = false;
    const { h, calls } = makeHandler({
        listIntercept: () => { intercepted = true; return true; },
    });
    h(fakeEvent('r'));
    assert.ok(intercepted);
    assert.equal(calls.list, 0);
});

test('listIntercept returning false falls through to onList', () => {
    const { h, calls } = makeHandler({ listIntercept: () => false });
    h(fakeEvent('a'));
    assert.equal(calls.list, 1);
});

test('listIntercept is not called when modal is open', () => {
    let intercepted = false;
    const { h, calls } = makeHandler({
        modalRef: { current: {} },
        listIntercept: () => { intercepted = true; return true; },
    });
    h(fakeEvent('r'));
    assert.ok(!intercepted, 'listIntercept must not run when modal is open');
    assert.equal(calls.modal, 1);
});

test('modal ref flips: closed then open', () => {
    const modalRef = { current: null };
    const calls = { modal: 0, list: 0 };
    const h = composeListEditorHandler({
        modalRef,
        onModal: () => { calls.modal++; },
        onList: () => { calls.list++; },
    });
    h(fakeEvent('a'));
    assert.equal(calls.list, 1);
    modalRef.current = {};
    h(fakeEvent('Escape'));
    assert.equal(calls.modal, 1);
    assert.equal(calls.list, 1);
});

test('preIntercept is called before listIntercept and modal', () => {
    const order = [];
    const modalRef = { current: null };
    const h = composeListEditorHandler({
        modalRef,
        onModal: () => { order.push('modal'); },
        onList: () => { order.push('list'); },
        preIntercept: (e) => { order.push('pre'); return false; },
        listIntercept: (e) => { order.push('list-intercept'); return false; },
    });
    h(fakeEvent('a'));
    assert.deepEqual(order, ['pre', 'list-intercept', 'list']);
});
