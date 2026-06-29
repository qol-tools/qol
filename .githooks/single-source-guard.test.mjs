import { test } from 'node:test';
import assert from 'node:assert/strict';
import { daemonEnvDrift } from './single-source-guard-lib.mjs';

const rust = (name) => `pub const ENV_DAEMON_REPLACE_EXISTING: &str = "${name}";`;
const py = (name, quote = "'") => `REPLACE_EXISTING_ENV = ${quote}${name}${quote}`;

test('agreeing values pass', () => {
    assert.equal(daemonEnvDrift(rust('QOL_X'), py('QOL_X')), null);
});

test('double-quoted python still compares, not a silent pass', () => {
    assert.equal(daemonEnvDrift(rust('QOL_X'), py('QOL_X', '"')), null);
    assert.match(daemonEnvDrift(rust('QOL_X'), py('QOL_Y', '"')), /drift/);
});

test('mismatching values are rejected', () => {
    assert.match(daemonEnvDrift(rust('QOL_X'), py('QOL_Y')), /drift/);
});

test('missing rust value fails closed', () => {
    assert.match(daemonEnvDrift('// no const here', py('QOL_X')), /cannot verify/);
});

test('missing python value fails closed', () => {
    assert.match(daemonEnvDrift(rust('QOL_X'), '# no assignment here'), /cannot verify/);
});

test('unreadable source (null) fails closed', () => {
    assert.match(daemonEnvDrift(null, py('QOL_X')), /cannot verify/);
    assert.match(daemonEnvDrift(rust('QOL_X'), null), /cannot verify/);
});
