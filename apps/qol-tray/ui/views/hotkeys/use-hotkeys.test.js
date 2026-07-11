import { test } from 'node:test';
import assert from 'node:assert/strict';
import { register } from 'node:module';
import { pathToFileURL } from 'node:url';

const hooksStubSource = `
export function useState(initial) { return [typeof initial === 'function' ? initial() : initial, () => {}]; }
export function useRef(value) { return { current: value }; }
export function useCallback(fn) { return fn; }
export function useMemo(fn) { return fn(); }
export function useEffect() {}
export function useLayoutEffect() {}
`;

const modalStubSource = `
export function createEditModalState() { return null; }
export function changeEditModalPlugin(previous) { return previous; }
`;

const listEditorStubSource = `
export function useListEditorKeyboard() { return { handleKey() {}, isBlocking() {}, modalNav: {} }; }
`;

const recorderStubSource = `
export function useRecorder() { return {}; }
`;

const apiStubSource = `
export function apiJson() { return Promise.resolve({}); }
export function apiText(...args) { return globalThis.__hotkeyApiText(...args); }
export function jsonRequest(method, payload) { return { method, body: JSON.stringify(payload) }; }
`;

const loaderSource = `
const HOOKS_URL = ${JSON.stringify('data:text/javascript,' + encodeURIComponent(hooksStubSource))};
const MODAL_URL = ${JSON.stringify('data:text/javascript,' + encodeURIComponent(modalStubSource))};
const LIST_EDITOR_URL = ${JSON.stringify('data:text/javascript,' + encodeURIComponent(listEditorStubSource))};
const RECORDER_URL = ${JSON.stringify('data:text/javascript,' + encodeURIComponent(recorderStubSource))};
const API_URL = ${JSON.stringify('data:text/javascript,' + encodeURIComponent(apiStubSource))};
export function resolve(specifier, context, nextResolve) {
    if (specifier === 'preact/hooks') {
        return { url: HOOKS_URL, shortCircuit: true, format: 'module' };
    }
    if (specifier === './modal.js' && context.parentURL?.includes('/views/hotkeys/use-hotkeys.js')) {
        return { url: MODAL_URL, shortCircuit: true, format: 'module' };
    }
    if (specifier === '../../lib/hooks/useListEditorKeyboard.js' && context.parentURL?.includes('/views/hotkeys/use-hotkeys.js')) {
        return { url: LIST_EDITOR_URL, shortCircuit: true, format: 'module' };
    }
    if (specifier === './useRecorder.js' && context.parentURL?.includes('/views/hotkeys/use-hotkeys.js')) {
        return { url: RECORDER_URL, shortCircuit: true, format: 'module' };
    }
    if (specifier === '../../api/client.js' && context.parentURL?.includes('/views/hotkeys/data.js')) {
        return { url: API_URL, shortCircuit: true, format: 'module' };
    }
    return nextResolve(specifier, context);
}
`;

register('data:text/javascript,' + encodeURIComponent(loaderSource), pathToFileURL('./'));

globalThis.__hotkeyApiText = async () => 'Hotkeys saved';
const { executeDelete, executeSave } = await import(`./use-hotkeys.js?test=${Date.now()}`);
const { persistHotkeys } = await import('./data.js');

function deferred() {
    let resolve;
    let reject;
    const promise = new Promise((done, fail) => { resolve = done; reject = fail; });
    return { promise, resolve, reject };
}

function hotkey(id, key) {
    return { id, key, plugin_uid: 'plugin-a', action: 'run', enabled: true };
}

function editModal(entry, key = entry.key) {
    return {
        hotkey: entry,
        key,
        pluginUid: entry.plugin_uid,
        action: entry.action,
        enabled: entry.enabled,
    };
}

function dataState(hotkeys, modal, selectedIndex = 0) {
    const calls = [];
    const data = {
        hotkeysRef: { current: hotkeys },
        editModalRef: { current: modal },
        selectedIndexRef: { current: selectedIndex },
        mutationQueueRef: { current: Promise.resolve() },
        setHotkeys: value => calls.push(['hotkeys', value]),
        setEditModal: value => calls.push(['modal', value]),
        setSelectedIndex: value => calls.push(['index', value]),
        refreshErrors: () => {},
    };
    return { calls, data };
}

test('persistHotkeys rejects a non-success response', async () => {
    const originalApiText = globalThis.__hotkeyApiText;
    globalThis.__hotkeyApiText = async () => { throw new Error('write failed'); };
    try {
        await assert.rejects(() => persistHotkeys([]), /write failed/);
    } finally {
        globalThis.__hotkeyApiText = originalApiText;
    }
});

test('save commits UI state only after persistence succeeds', async () => {
    const first = hotkey('a', 'Ctrl+A');
    const { calls, data } = dataState([first], editModal(first, 'Ctrl+B'));
    const pending = deferred();
    const recorder = { cancel() {} };

    const save = executeSave(data, recorder, () => pending.promise);
    await Promise.resolve();

    assert.deepEqual(calls, []);
    assert.deepEqual(data.hotkeysRef.current, [first]);

    pending.resolve();
    assert.equal(await save, true);
    assert.equal(data.hotkeysRef.current[0].key, 'Ctrl+B');
    assert.deepEqual(calls.map(([kind]) => kind), ['hotkeys', 'modal']);
});

test('failed save keeps the prior state and editor open', async () => {
    const first = hotkey('a', 'Ctrl+A');
    const modal = editModal(first, 'Ctrl+B');
    const { calls, data } = dataState([first], modal);
    const recorder = { cancel() {} };

    const saved = await executeSave(data, recorder, async () => { throw new Error('write failed'); });

    assert.equal(saved, false);
    assert.deepEqual(calls, []);
    assert.deepEqual(data.hotkeysRef.current, [first]);
    assert.equal(data.editModalRef.current, modal);
});

test('queued saves keep a newer editor open and commit the newest payload last', async () => {
    const first = hotkey('a', 'Ctrl+A');
    const modalA = editModal(first, 'Ctrl+B');
    const modalB = editModal(first, 'Ctrl+C');
    const { calls, data } = dataState([first], modalA);
    const requests = [deferred(), deferred()];
    const payloads = [];
    const persist = next => {
        payloads.push(next[0].key);
        return requests[payloads.length - 1].promise;
    };
    const recorder = { cancel() {} };

    const saveA = executeSave(data, recorder, persist);
    data.editModalRef.current = modalB;
    const saveB = executeSave(data, recorder, persist);
    await Promise.resolve();

    assert.deepEqual(payloads, ['Ctrl+B']);
    requests[0].resolve();
    assert.equal(await saveA, false);
    await Promise.resolve();
    assert.deepEqual(payloads, ['Ctrl+B', 'Ctrl+C']);
    assert.equal(calls.some(([kind]) => kind === 'modal'), false);

    requests[1].resolve();
    assert.equal(await saveB, true);
    assert.equal(data.hotkeysRef.current[0].key, 'Ctrl+C');
    assert.equal(calls.filter(([kind]) => kind === 'modal').length, 1);
});

test('failed delete keeps the prior list and selection', async () => {
    const entries = [hotkey('a', 'Ctrl+A'), hotkey('b', 'Ctrl+B')];
    const { calls, data } = dataState(entries, null, 1);

    const deleted = await executeDelete(data, async () => { throw new Error('write failed'); });

    assert.equal(deleted, false);
    assert.deepEqual(calls, []);
    assert.equal(data.hotkeysRef.current, entries);
    assert.equal(data.selectedIndexRef.current, 1);
});

test('concurrent deletes serialize against the committed refs', async () => {
    const entries = [
        hotkey('a', 'Ctrl+A'),
        hotkey('b', 'Ctrl+B'),
        hotkey('c', 'Ctrl+C'),
    ];
    const { data } = dataState(entries, null, 1);
    const requests = [deferred(), deferred()];
    const persisted = [];
    const persist = next => {
        persisted.push(next.map(entry => entry.id));
        return requests[persisted.length - 1].promise;
    };

    const deleteB = executeDelete(data, persist);
    const deleteC = executeDelete(data, persist);
    await Promise.resolve();
    assert.deepEqual(persisted, [['a', 'c']]);

    requests[0].resolve();
    assert.equal(await deleteB, true);
    await Promise.resolve();
    assert.deepEqual(persisted, [['a', 'c'], ['a']]);

    requests[1].resolve();
    assert.equal(await deleteC, true);

    assert.deepEqual(data.hotkeysRef.current.map(entry => entry.id), ['a']);
    assert.equal(data.selectedIndexRef.current, 0);
});
