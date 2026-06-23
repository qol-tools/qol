import { test } from 'node:test';
import assert from 'node:assert/strict';
import { composeListEditorHandler } from './list-editor-dispatch.js';

function run({ modalOpen, pre, list }) {
    const calls = { modal: 0, list: 0, pre: 0, listIntercept: 0 };
    composeListEditorHandler({
        modalRef: { current: modalOpen ? {} : null },
        onModal: () => { calls.modal++; },
        onList: () => { calls.list++; },
        preIntercept: pre === undefined ? undefined : () => { calls.pre++; return pre; },
        listIntercept: list === undefined ? undefined : () => { calls.listIntercept++; return list; },
    })({});
    return calls;
}

const cases = [
    ['modal open, no intercepts routes to onModal', { modalOpen: true }, { modal: 1, list: 0 }],
    ['modal closed, no intercepts routes to onList', { modalOpen: false }, { modal: 0, list: 1 }],
    ['preIntercept true stops before modal', { modalOpen: true, pre: true }, { modal: 0, list: 0, pre: 1 }],
    ['preIntercept false continues to modal', { modalOpen: true, pre: false }, { modal: 1, list: 0, pre: 1 }],
    ['listIntercept true stops before onList', { modalOpen: false, list: true }, { modal: 0, list: 0, listIntercept: 1 }],
    ['listIntercept false falls through to onList', { modalOpen: false, list: false }, { modal: 0, list: 1, listIntercept: 1 }],
    ['listIntercept skipped while modal open', { modalOpen: true, list: true }, { modal: 1, list: 0, listIntercept: 0 }],
];

for (const [desc, input, expected] of cases) {
    test(`composeListEditorHandler: ${desc}`, () => {
        const calls = run(input);
        for (const [sink, want] of Object.entries(expected)) {
            assert.equal(calls[sink], want, `${desc} (${sink})`);
        }
    });
}

test('composeListEditorHandler: reads modalRef at call time, not compose time', () => {
    const modalRef = { current: null };
    const calls = { modal: 0, list: 0 };
    const handler = composeListEditorHandler({
        modalRef,
        onModal: () => { calls.modal++; },
        onList: () => { calls.list++; },
    });
    handler({});
    assert.equal(calls.list, 1, 'closed routes to list');
    modalRef.current = {};
    handler({});
    assert.equal(calls.modal, 1, 'reopened routes to modal');
    assert.equal(calls.list, 1, 'list not called again');
});

test('composeListEditorHandler: dispatch order is pre then listIntercept then onList', () => {
    const order = [];
    composeListEditorHandler({
        modalRef: { current: null },
        onModal: () => order.push('modal'),
        onList: () => order.push('list'),
        preIntercept: () => { order.push('pre'); return false; },
        listIntercept: () => { order.push('list-intercept'); return false; },
    })({});
    assert.deepEqual(order, ['pre', 'list-intercept', 'list']);
});
