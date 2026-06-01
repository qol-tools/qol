import { test } from 'node:test';
import assert from 'node:assert/strict';
import { isShortcutValid } from './validation.js';

const validUrlShortcut = () => ({
    id: 'my-shortcut', name: 'Open Example', enabled: true, export_to_launcher: true,
    action: { type: 'open_url', url: 'https://example.com' }
});

const validAppShortcut = () => ({
    id: 'launch-safari', name: 'Safari', enabled: true, export_to_launcher: true,
    action: { type: 'launch_app', app: { type: 'bundle_id', id: 'com.apple.Safari' } }
});

test('valid open_url shortcut passes', () => {
    assert.equal(isShortcutValid(validUrlShortcut()), true);
});

test('valid launch_app shortcut passes', () => {
    assert.equal(isShortcutValid(validAppShortcut()), true);
});

test('null or undefined is invalid', () => {
    assert.equal(isShortcutValid(null), false);
    assert.equal(isShortcutValid(undefined), false);
});

test('valid without an id (id is derived, not entered)', () => {
    assert.equal(isShortcutValid({
        name: 'X',
        action: { type: 'open_url', url: 'https://x.io' }
    }), true);
});

test('whitespace-only name is invalid', () => {
    const s = validUrlShortcut(); s.name = '   ';
    assert.equal(isShortcutValid(s), false);
});

test('url missing scheme is invalid', () => {
    const s = validUrlShortcut(); s.action.url = 'example.com';
    assert.equal(isShortcutValid(s), false);
});

test('url with http:// is valid', () => {
    const s = validUrlShortcut(); s.action.url = 'http://example.com';
    assert.equal(isShortcutValid(s), true);
});

test('empty url is invalid', () => {
    const s = validUrlShortcut(); s.action.url = '';
    assert.equal(isShortcutValid(s), false);
});

test('launch_app with empty path is invalid', () => {
    const s = validAppShortcut(); s.action.app = { type: 'path', path: '' };
    assert.equal(isShortcutValid(s), false);
});

test('launch_app with filled path is valid', () => {
    const s = validAppShortcut(); s.action.app = { type: 'path', path: '/Applications/Safari.app' };
    assert.equal(isShortcutValid(s), true);
});

test('launch_app with name ref requires non-blank name', () => {
    const s = validAppShortcut(); s.action.app = { type: 'name', name: '' };
    assert.equal(isShortcutValid(s), false);
    s.action.app = { type: 'name', name: 'Safari' };
    assert.equal(isShortcutValid(s), true);
});

test('browser_override must itself be a valid app ref when present', () => {
    const s = validUrlShortcut();
    s.action.browser_override = { type: 'bundle_id', id: '' };
    assert.equal(isShortcutValid(s), false);
    s.action.browser_override = { type: 'bundle_id', id: 'com.google.Chrome' };
    assert.equal(isShortcutValid(s), true);
});

test('unknown action type is invalid', () => {
    const s = validUrlShortcut(); s.action = { type: 'something_else' };
    assert.equal(isShortcutValid(s), false);
});
