import { test } from 'node:test';
import assert from 'node:assert/strict';
import { deriveShortcutId } from './derive-id.js';

test('slugifies a name', () => {
    assert.equal(deriveShortcutId('My Shortcut', []), 'my-shortcut');
});

test('strips non-charset and trims dashes', () => {
    assert.equal(deriveShortcutId('  Hello, World!! ', []), 'hello-world');
});

test('dedupes against existing ids', () => {
    assert.equal(deriveShortcutId('Docs', ['docs']), 'docs-2');
    assert.equal(deriveShortcutId('Docs', ['docs', 'docs-2']), 'docs-3');
});

test('falls back to host when name empty', () => {
    assert.equal(deriveShortcutId('', [], 'github.com'), 'github-com');
});

test('falls back to "shortcut" when nothing usable', () => {
    assert.equal(deriveShortcutId('***', [], ''), 'shortcut');
});

test('caps length at 64', () => {
    const long = 'a'.repeat(100);
    assert.equal(deriveShortcutId(long, []).length, 64);
});
