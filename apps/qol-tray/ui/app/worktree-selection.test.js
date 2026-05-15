import { test } from 'node:test';
import assert from 'node:assert/strict';
import { resolveInitialBranch } from './worktree-selection.js';

const BRANCHES = ['main', 'feat/x', 'fix/y'];

test('server active branch wins over persisted when both valid', () => {
    const result = resolveInitialBranch({
        persisted: 'feat/x',
        serverActive: 'fix/y',
        branches: BRANCHES,
    });
    assert.equal(result, 'fix/y');
});

test('falls back to persisted when server has no active branch', () => {
    const result = resolveInitialBranch({
        persisted: 'feat/x',
        serverActive: null,
        branches: BRANCHES,
    });
    assert.equal(result, 'feat/x');
});

test('ignores server active branch not present in list', () => {
    const result = resolveInitialBranch({
        persisted: 'feat/x',
        serverActive: 'gone',
        branches: BRANCHES,
    });
    assert.equal(result, 'feat/x');
});

test('returns null when neither server nor persisted match', () => {
    const result = resolveInitialBranch({
        persisted: 'nope',
        serverActive: 'also-nope',
        branches: BRANCHES,
    });
    assert.equal(result, null);
});

test('returns null when both inputs are empty', () => {
    const result = resolveInitialBranch({
        persisted: null,
        serverActive: null,
        branches: BRANCHES,
    });
    assert.equal(result, null);
});

test('treats non-array branches as empty', () => {
    const result = resolveInitialBranch({
        persisted: 'feat/x',
        serverActive: 'feat/x',
        branches: null,
    });
    assert.equal(result, null);
});

test('branch match is exact, not substring', () => {
    const result = resolveInitialBranch({
        persisted: null,
        serverActive: 'feat/x-suffix',
        branches: ['feat/x', 'feat/x-suffix'],
    });
    assert.equal(result, 'feat/x-suffix');
});
