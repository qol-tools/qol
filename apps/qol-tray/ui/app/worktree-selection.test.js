import { test } from 'node:test';
import assert from 'node:assert/strict';
import { resolveInitialWorktree, parentDir } from './worktree-selection.js';

const WORKTREES = [
    { branch: 'main', path: '/repo/main' },
    { branch: 'feat/x', path: '/repo/worktrees/feat-x/qol-tray' },
    { branch: 'fix/y', path: '/repo/worktrees/fix-y/plugin-alt-tab' },
];

test('server active path wins over persisted when both valid', () => {
    const result = resolveInitialWorktree({
        persisted: '/repo/worktrees/feat-x/qol-tray',
        serverActive: '/repo/worktrees/fix-y/plugin-alt-tab',
        worktrees: WORKTREES,
    });
    assert.equal(result, '/repo/worktrees/fix-y/plugin-alt-tab');
});

test('falls back to persisted when server has no active path', () => {
    const result = resolveInitialWorktree({
        persisted: '/repo/worktrees/feat-x/qol-tray',
        serverActive: null,
        worktrees: WORKTREES,
    });
    assert.equal(result, '/repo/worktrees/feat-x/qol-tray');
});

test('ignores server active path not present in worktrees', () => {
    const result = resolveInitialWorktree({
        persisted: '/repo/worktrees/feat-x/qol-tray',
        serverActive: '/gone/from/disk',
        worktrees: WORKTREES,
    });
    assert.equal(result, '/repo/worktrees/feat-x/qol-tray');
});

test('returns null when neither server nor persisted match', () => {
    const result = resolveInitialWorktree({
        persisted: '/nope',
        serverActive: '/also-nope',
        worktrees: WORKTREES,
    });
    assert.equal(result, null);
});

test('returns null when both inputs are empty', () => {
    const result = resolveInitialWorktree({
        persisted: null,
        serverActive: null,
        worktrees: WORKTREES,
    });
    assert.equal(result, null);
});

test('resolves persisted parent-dir selection to a worktree path', () => {
    const result = resolveInitialWorktree({
        persisted: '/repo/worktrees/feat-x',
        serverActive: null,
        worktrees: WORKTREES,
    });
    assert.equal(result, '/repo/worktrees/feat-x/qol-tray');
});

test('treats empty worktrees list as empty array', () => {
    const result = resolveInitialWorktree({
        persisted: '/repo/worktrees/feat-x/qol-tray',
        serverActive: '/repo/worktrees/feat-x/qol-tray',
        worktrees: null,
    });
    assert.equal(result, null);
});

test('server active match is case- and boundary-sensitive', () => {
    const result = resolveInitialWorktree({
        persisted: null,
        serverActive: '/repo/main/',
        worktrees: WORKTREES,
    });
    assert.equal(result, null, 'trailing slash should not match /repo/main');
});

test('parentDir strips final segment', () => {
    assert.equal(parentDir('/a/b/c'), '/a/b');
    assert.equal(parentDir('/only'), null);
    assert.equal(parentDir(''), null);
    assert.equal(parentDir(null), null);
});

test('switching selection after recompile: server now points at new path', () => {
    const result = resolveInitialWorktree({
        persisted: '/repo/main',
        serverActive: '/repo/worktrees/fix-y/plugin-alt-tab',
        worktrees: WORKTREES,
    });
    assert.equal(result, '/repo/worktrees/fix-y/plugin-alt-tab');
});
