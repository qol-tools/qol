import { test } from 'node:test';
import assert from 'node:assert/strict';
import { register } from 'node:module';
import { pathToFileURL } from 'node:url';

// Stub `preact/hooks` so we can drive the hook's contract from node:test
// without pulling in a real Preact runtime. The stub records the
// useState/useEffect calls; the test runs the hook as a plain function and
// then invokes the captured effect/cleanup to simulate mount + unmount.
const stubSource = `
let bumpCount = 0;
let stateInitial = null;
let lastEffectFn = null;
let lastEffectCleanup = null;

export function useState(initial) {
    stateInitial = initial;
    const setter = (next) => {
        bumpCount += 1;
        if (typeof next === 'function') next(0);
    };
    return [initial, setter];
}

export function useEffect(fn, _deps) {
    lastEffectFn = fn;
}

export function __runEffect() {
    lastEffectCleanup = lastEffectFn ? lastEffectFn() : null;
    return lastEffectCleanup;
}

export function __runCleanup() {
    if (typeof lastEffectCleanup === 'function') lastEffectCleanup();
}

export function __bumpCount() { return bumpCount; }
export function __reset() {
    bumpCount = 0;
    stateInitial = null;
    lastEffectFn = null;
    lastEffectCleanup = null;
}
`;

const loaderSource = `
const STUB_URL = ${JSON.stringify('data:text/javascript,' + encodeURIComponent(stubSource))};
export function resolve(specifier, context, nextResolve) {
    if (specifier === 'preact/hooks') {
        return { url: STUB_URL, shortCircuit: true, format: 'module' };
    }
    return nextResolve(specifier, context);
}
`;
const loaderUrl = 'data:text/javascript,' + encodeURIComponent(loaderSource);
register(loaderUrl, pathToFileURL('./'));

const hooksStub = await import('preact/hooks');
const { useSharedSlot } = await import('./useSharedSlot.js');

function makeSlot(initial) {
    const state = { value: initial };
    const listeners = new Set();
    return {
        get: () => state.value,
        set: (v) => { state.value = v; for (const fn of listeners) fn(); },
        subscribe: (fn) => { listeners.add(fn); return () => listeners.delete(fn); },
        listenerCount: () => listeners.size,
    };
}

test('useSharedSlot returns slot.get() value', () => {
    hooksStub.__reset();
    const slot = makeSlot('alpha');
    const result = useSharedSlot(slot);
    assert.equal(result, 'alpha');
});

test('useSharedSlot subscribes on mount and unsubscribes on cleanup', () => {
    hooksStub.__reset();
    const slot = makeSlot(null);
    useSharedSlot(slot);
    assert.equal(slot.listenerCount(), 0, 'no subscription before effect runs');
    hooksStub.__runEffect();
    assert.equal(slot.listenerCount(), 1, 'subscribed after effect');
    hooksStub.__runCleanup();
    assert.equal(slot.listenerCount(), 0, 'unsubscribed on cleanup');
});

test('useSharedSlot bumps state when slot fires a change', () => {
    hooksStub.__reset();
    const slot = makeSlot({ modal: null });
    useSharedSlot(slot);
    hooksStub.__runEffect();
    assert.equal(hooksStub.__bumpCount(), 0);
    slot.set({ modal: { id: 'foo' } });
    assert.equal(hooksStub.__bumpCount(), 1, 'one bump per slot.set call');
    slot.set({ modal: { id: 'bar' } });
    assert.equal(hooksStub.__bumpCount(), 2);
    hooksStub.__runCleanup();
    slot.set({ modal: null });
    assert.equal(hooksStub.__bumpCount(), 2, 'no further bumps after cleanup');
});
