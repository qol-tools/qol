import { test } from 'node:test';
import assert from 'node:assert/strict';
import { resolveInitialBranch } from './worktree-selection.js';

test('returns the server active branch verbatim', () => {
    assert.equal(resolveInitialBranch({ serverActive: 'feat/x' }), 'feat/x');
});

test('returns null when server has no active branch', () => {
    assert.equal(resolveInitialBranch({ serverActive: null }), null);
});

test('returns null when server active is empty string', () => {
    assert.equal(resolveInitialBranch({ serverActive: '' }), null);
});

test('does not consult localStorage / persisted', () => {
    // Even if the caller passes a persisted value, server is canonical.
    assert.equal(
        resolveInitialBranch({ serverActive: null, persisted: 'stale-branch' }),
        null,
    );
});
