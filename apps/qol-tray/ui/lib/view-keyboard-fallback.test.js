import { test } from 'node:test';
import assert from 'node:assert/strict';
import { parentViewIdFor, resolveViewKeyboard } from './view-keyboard-fallback.js';

test('parentViewIdFor strips the -editor suffix to recover the parent id', () => {
    assert.equal(parentViewIdFor('hotkeys-editor'), 'hotkeys');
    assert.equal(parentViewIdFor('shortcuts-editor'), 'shortcuts');
    assert.equal(parentViewIdFor('task-runner-editor'), 'task-runner');
});

test('parentViewIdFor returns null for ids without the -editor suffix', () => {
    assert.equal(parentViewIdFor('hotkeys'), null);
    assert.equal(parentViewIdFor('plugins'), null);
    assert.equal(parentViewIdFor(''), null);
    assert.equal(parentViewIdFor(undefined), null);
    assert.equal(parentViewIdFor(null), null);
    assert.equal(parentViewIdFor('logs-detail'), null);
    assert.equal(parentViewIdFor('profile-backup-detail'), null);
});

test('resolveViewKeyboard returns the direct registration when present', () => {
    const hotkeysHandler = { handleKey: () => {}, isBlocking: () => false };
    const editorHandler = { handleKey: () => {}, isBlocking: () => true };
    const registry = new Map([
        ['hotkeys', hotkeysHandler],
        ['hotkeys-editor', editorHandler],
    ]);
    const get = (id) => registry.get(id) || null;

    assert.equal(resolveViewKeyboard('hotkeys', get), hotkeysHandler);
    assert.equal(resolveViewKeyboard('hotkeys-editor', get), editorHandler);
});

test('resolveViewKeyboard falls back to parent for known editor sub-pages', () => {
    const hotkeysHandler = { handleKey: () => {}, isBlocking: () => true };
    const shortcutsHandler = { handleKey: () => {}, isBlocking: () => true };
    const taskRunnerHandler = { handleKey: () => {}, isBlocking: () => true };
    const registry = new Map([
        ['hotkeys', hotkeysHandler],
        ['shortcuts', shortcutsHandler],
        ['task-runner', taskRunnerHandler],
    ]);
    const get = (id) => registry.get(id) || null;

    assert.equal(resolveViewKeyboard('hotkeys-editor', get), hotkeysHandler);
    assert.equal(resolveViewKeyboard('shortcuts-editor', get), shortcutsHandler);
    assert.equal(resolveViewKeyboard('task-runner-editor', get), taskRunnerHandler);
});

test('resolveViewKeyboard returns null when neither direct nor parent is registered', () => {
    const get = () => null;
    assert.equal(resolveViewKeyboard('hotkeys-editor', get), null);
    assert.equal(resolveViewKeyboard('unknown-view', get), null);
    assert.equal(resolveViewKeyboard('', get), null);
});

test('resolveViewKeyboard does not fall back for non-editor sub-pages', () => {
    const logsHandler = { handleKey: () => {}, isBlocking: () => false };
    const get = (id) => (id === 'logs' ? logsHandler : null);
    // logs-detail is not in EDITOR_PARENT_VIEW, so no fallback
    assert.equal(resolveViewKeyboard('logs-detail', get), null);
});

test('resolveViewKeyboard prefers anchor handler over activeViewId during dive', () => {
    // During dive, activeViewId stays on parent (e.g. 'hotkeys') but the
    // anchor moves to the editor sub-page (e.g. 'hotkeys-editor').
    const editorHandler = { handleKey: () => {}, isBlocking: () => true };
    const parentHandler = { handleKey: () => {}, isBlocking: () => false };
    const registry = new Map([
        ['hotkeys', parentHandler],
        ['hotkeys-editor', editorHandler],
    ]);
    const get = (id) => registry.get(id) || null;

    assert.equal(resolveViewKeyboard('hotkeys', get, 'hotkeys-editor'), editorHandler);
});

test('resolveViewKeyboard falls back to anchor parent when anchor unregistered', () => {
    // If the editor sub-page hasn't registered yet (e.g. modal not open),
    // the anchor's parent registration must still be reachable.
    const parentHandler = { handleKey: () => {}, isBlocking: () => true };
    const registry = new Map([['hotkeys', parentHandler]]);
    const get = (id) => registry.get(id) || null;

    assert.equal(resolveViewKeyboard('hotkeys', get, 'hotkeys-editor'), parentHandler);
});

test('resolveViewKeyboard with same anchor and viewId behaves like no anchor', () => {
    const handler = { handleKey: () => {}, isBlocking: () => false };
    const get = (id) => (id === 'hotkeys' ? handler : null);

    assert.equal(resolveViewKeyboard('hotkeys', get, 'hotkeys'), handler);
});

test('resolveViewKeyboard with null anchor falls back to viewId-only resolution', () => {
    const handler = { handleKey: () => {}, isBlocking: () => false };
    const get = (id) => (id === 'hotkeys' ? handler : null);

    assert.equal(resolveViewKeyboard('hotkeys', get, null), handler);
    assert.equal(resolveViewKeyboard('hotkeys-editor', get, null), handler);
});
