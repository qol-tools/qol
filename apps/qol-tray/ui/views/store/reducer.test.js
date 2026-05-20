import { test } from 'node:test';
import assert from 'node:assert/strict';
import { displayedStoreVersion, isStoreUpdateAvailable } from './reducer.js';

test('displayedStoreVersion prefers installed_version when installed', () => {
    const cases = [
        [{ installed: true, installed_version: '0.12.4', version: '0.10.25' }, '0.12.4', 'installed newer than catalog'],
        [{ installed: true, installed_version: '1.1.2', version: '1.2.1' }, '1.1.2', 'installed older than catalog (no update yet applied)'],
        [{ installed: true, installed_version: '1.0.0', version: '1.0.0' }, '1.0.0', 'installed equal to catalog'],
        [{ installed: false, installed_version: null, version: '2.3.4' }, '2.3.4', 'not installed shows catalog'],
        [{ installed: true, installed_version: null, version: '2.0.0' }, '2.0.0', 'installed bool true but no installed_version: fall back to catalog'],
        [{ installed: false, installed_version: '0.9.0', version: '1.0.0' }, '1.0.0', 'stale installed_version with installed=false: catalog wins'],
        [{}, null, 'empty plugin object'],
        [null, null, 'null plugin'],
        [undefined, null, 'undefined plugin'],
    ];
    for (const [plugin, expected, label] of cases) {
        assert.equal(displayedStoreVersion(plugin), expected, label);
    }
});

test('isStoreUpdateAvailable: only true when catalog is strictly newer', () => {
    const cases = [
        [{ installed: true, installed_version: '1.0.0', version: '1.0.1' }, true, 'patch bump'],
        [{ installed: true, installed_version: '1.0.0', version: '2.0.0' }, true, 'major bump'],
        [{ installed: true, installed_version: '1.0.0', version: '1.0.0' }, false, 'same'],
        [{ installed: true, installed_version: '1.0.1', version: '1.0.0' }, false, 'catalog older than installed'],
        [{ installed: true, installed_version: '0.12.4', version: '0.10.25' }, false, 'real-world: local build ahead of catalog'],
        [{ installed: false, installed_version: '1.0.0', version: '1.0.1' }, false, 'not installed'],
        [{ installed: true, installed_version: null, version: '1.0.1' }, false, 'no installed_version'],
        [{ installed: true, installed_version: '1.0.0', version: null }, false, 'no catalog version'],
    ];
    for (const [plugin, expected, label] of cases) {
        assert.equal(isStoreUpdateAvailable(plugin), expected, label);
    }
});
