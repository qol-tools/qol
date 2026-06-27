import { test } from 'node:test';
import assert from 'node:assert/strict';
import { statusImpliesRestart, setHostRestarting, isHostRestarting } from './host-restart.js';

test('statusImpliesRestart is true only while a host restart is pending', () => {
    const cases = [
        ['downloading', true],
        ['compiling', true],
        ['done', true],
        ['recompile_done', true],
        ['idle', false],
        ['checking', false],
        ['available', false],
        ['up-to-date', false],
        ['error', false],
    ];
    for (const [status, expected] of cases) {
        assert.equal(statusImpliesRestart(status), expected, `status: ${status}`);
    }
});

test('host restarting flag round-trips', () => {
    setHostRestarting(true);
    assert.equal(isHostRestarting(), true);
    setHostRestarting(false);
    assert.equal(isHostRestarting(), false);
});
