import { test } from 'node:test';
import assert from 'node:assert/strict';
import { editorParentViewId, resolveViewKeyboard } from './view-keyboard-fallback.js';

test('editorParentViewId strips the -editor suffix to recover the parent id', () => {
    assert.equal(editorParentViewId('hotkeys-editor'), 'hotkeys');
    assert.equal(editorParentViewId('shortcuts-editor'), 'shortcuts');
    assert.equal(editorParentViewId('task-runner-editor'), 'task-runner');
});

test('editorParentViewId returns null for ids without the -editor suffix', () => {
    assert.equal(editorParentViewId('hotkeys'), null);
    assert.equal(editorParentViewId('plugins'), null);
    assert.equal(editorParentViewId(''), null);
    assert.equal(editorParentViewId(undefined), null);
    assert.equal(editorParentViewId(null), null);
});

test('editorParentViewId is intentionally narrow: non-editor dive sub-pages return null', () => {
    assert.equal(editorParentViewId('logs-detail'), null);
    assert.equal(editorParentViewId('profile-backup-detail'), null);
    assert.equal(editorParentViewId('dev-log-filters'), null);
    assert.equal(editorParentViewId('dev-plugin-actions'), null);
    assert.equal(editorParentViewId('plugins-uninstall-confirm'), null);
    assert.equal(editorParentViewId('plugins-actions'), null);
    assert.equal(editorParentViewId('task-runner-test-runner'), null);
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
    assert.equal(resolveViewKeyboard('logs-detail', get), null);
});

test('resolveViewKeyboard prefers anchor handler over activeViewId during dive', () => {
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

test('resolveViewKeyboard for non-editor dive sub-page returns null (caller falls back to activeViewId)', () => {
    const logsHandler = { handleKey: () => {}, isBlocking: () => false };
    const registry = new Map([['logs', logsHandler]]);
    const get = (id) => registry.get(id) || null;

    assert.equal(resolveViewKeyboard('logs', get, 'logs-detail'), logsHandler);
});
