import { test } from 'node:test';
import assert from 'node:assert/strict';
import { getViewLabel, resolveViewLabel } from './view-labels.js';

test('getViewLabel returns declared text for known top-level id', () => {
    assert.deepEqual(getViewLabel('plugins'), { text: 'Plugins', animation: null });
});

test('getViewLabel preserves animation field for object entries', () => {
    assert.deepEqual(getViewLabel('dev'), { text: 'Developer', animation: 'scramble' });
});

test('getViewLabel falls back to id for unknown view', () => {
    assert.deepEqual(getViewLabel('hotkeys-editor'), { text: 'hotkeys-editor', animation: null });
});

test('getViewLabel returns declared text for dev-gpui subpage', () => {
    assert.deepEqual(getViewLabel('dev-gpui'), { text: 'GPUI', animation: null });
});

test('resolveViewLabel prefers VIEW_LABELS entry for known top-level id', () => {
    const entry = { id: 'plugins', label: 'Override Should Lose' };
    assert.deepEqual(resolveViewLabel(entry), { text: 'Plugins', animation: null });
});

test('resolveViewLabel preserves animation for declared entries', () => {
    const entry = { id: 'dev' };
    assert.deepEqual(resolveViewLabel(entry), { text: 'Developer', animation: 'scramble' });
});

test('resolveViewLabel returns entry.label when id is unknown but label provided', () => {
    const entry = { id: 'plugin-lights-zigbee', label: 'Zigbee' };
    assert.deepEqual(resolveViewLabel(entry), { text: 'Zigbee', animation: null });
});

test('resolveViewLabel returns entry.label for static subpages registered with a label', () => {
    const entry = { id: 'hotkeys-editor', label: 'Hotkey Editor' };
    assert.deepEqual(resolveViewLabel(entry), { text: 'Hotkey Editor', animation: null });
});

test('resolveViewLabel does NOT return kebab id when entry has a label', () => {
    const cases = [
        { id: 'hotkeys-editor', label: 'Hotkey Editor' },
        { id: 'shortcuts-editor', label: 'Shortcut Editor' },
        { id: 'logs-detail', label: 'Log Detail' },
        { id: 'plugins-uninstall-confirm', label: 'Confirm Uninstall' },
        { id: 'plugins-actions', label: 'Plugin Actions' },
        { id: 'dev-log-filters', label: 'Edit Log Filters' },
        { id: 'dev-plugin-actions', label: 'Plugin Actions' },
        { id: 'task-runner-test-runner', label: 'Test Runner' },
        { id: 'task-runner-editor', label: 'Action Editor' },
        { id: 'profile-backup-detail', label: 'Backup Detail' },
        { id: 'plugin-lights-zigbee', label: 'Zigbee' },
        { id: 'plugin-foo-config', label: 'Settings' },
    ];
    for (const entry of cases) {
        const result = resolveViewLabel(entry);
        assert.equal(result.text, entry.label, `entry.id=${entry.id}`);
        assert.notEqual(result.text, entry.id, `kebab id leaked for ${entry.id}`);
    }
});

test('resolveViewLabel falls back to id when no declared entry and no entry.label', () => {
    const entry = { id: 'mystery-view' };
    assert.deepEqual(resolveViewLabel(entry), { text: 'mystery-view', animation: null });
});

test('resolveViewLabel handles null entry safely', () => {
    assert.deepEqual(resolveViewLabel(null), { text: '', animation: null });
});

test('resolveViewLabel: declared text wins, declared animation wins, entry.label is ignored when declared exists', () => {
    const entry = { id: 'plugins', label: 'Should Be Ignored' };
    const result = resolveViewLabel(entry);
    assert.equal(result.text, 'Plugins');
    assert.equal(result.animation, null);
});
