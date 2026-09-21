import { test } from 'node:test';
import assert from 'node:assert/strict';
import { register } from 'node:module';
import { pathToFileURL } from 'node:url';

const hooksStubSource = `
export function useCallback(fn) { return fn; }
`;

const dataStubSource = `
export function installStorePlugin(id) { return globalThis.__storeRuns.install(id); }
export function updateStorePlugin(id) { return globalThis.__storeRuns.update(id); }
`;

const toastStubSource = `
export function toast(type, message) { globalThis.__storeToasts.push({ type, message }); }
`;

const loaderSource = `
const HOOKS_URL = ${JSON.stringify('data:text/javascript,' + encodeURIComponent(hooksStubSource))};
const DATA_URL = ${JSON.stringify('data:text/javascript,' + encodeURIComponent(dataStubSource))};
const TOAST_URL = ${JSON.stringify('data:text/javascript,' + encodeURIComponent(toastStubSource))};
export function resolve(specifier, context, nextResolve) {
    if (specifier === 'preact/hooks') {
        return { url: HOOKS_URL, shortCircuit: true, format: 'module' };
    }
    if (specifier === './data.js' && context.parentURL?.includes('/views/store/use-install.js')) {
        return { url: DATA_URL, shortCircuit: true, format: 'module' };
    }
    if (specifier === '../../lib/toast.js' && context.parentURL?.includes('/views/store/use-install.js')) {
        return { url: TOAST_URL, shortCircuit: true, format: 'module' };
    }
    return nextResolve(specifier, context);
}
`;

register('data:text/javascript,' + encodeURIComponent(loaderSource), pathToFileURL('./'));

const { useStoreInstall } = await import(`./use-install.js?case=${Date.now()}`);

function deferred() {
    let resolve;
    const promise = new Promise(done => { resolve = done; });
    return { promise, resolve };
}

test('concurrent updates complete each plugin lifecycle independently', async () => {
    const jobs = { a: deferred(), b: deferred() };
    globalThis.__storeRuns = {
        install: () => Promise.resolve(),
        update: id => jobs[id].promise,
    };
    globalThis.__storeToasts = [];
    const active = new Set();
    const installing = {
        has: id => active.has(id),
        add: id => active.add(id),
        remove: id => active.delete(id),
    };
    const marked = [];
    let refreshes = 0;
    const controller = useStoreInstall(
        { current: [{ id: 'a', name: 'A' }, { id: 'b', name: 'B' }] },
        () => { refreshes += 1; },
        installing,
        id => marked.push(id),
    );

    const updateA = controller.updatePlugin('a');
    const updateB = controller.updatePlugin('b');
    assert.deepEqual([...active], ['a', 'b']);

    jobs.a.resolve();
    await updateA;

    assert.deepEqual(marked, ['a']);
    assert.deepEqual([...active], ['b']);
    assert.equal(refreshes, 0);

    jobs.b.resolve();
    await updateB;

    assert.deepEqual(marked, ['a', 'b']);
    assert.deepEqual([...active], []);
    assert.equal(refreshes, 0);
});

test('install refreshes after its own lifecycle and ignores duplicate starts', async () => {
    const job = deferred();
    globalThis.__storeRuns = {
        install: () => job.promise,
        update: () => Promise.resolve(),
    };
    globalThis.__storeToasts = [];
    const active = new Set();
    const installing = {
        has: id => active.has(id),
        add: id => active.add(id),
        remove: id => active.delete(id),
    };
    let refreshes = 0;
    const controller = useStoreInstall(
        { current: [{ id: 'a', name: 'A' }] },
        () => { refreshes += 1; },
        installing,
        () => {},
    );

    const install = controller.installPlugin('a');
    await controller.installPlugin('a');

    assert.deepEqual([...active], ['a']);
    assert.equal(refreshes, 0);

    job.resolve();
    await install;

    assert.deepEqual([...active], []);
    assert.equal(refreshes, 1);
});
