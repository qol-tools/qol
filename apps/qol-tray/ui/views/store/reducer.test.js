import { test } from 'node:test';
import assert from 'node:assert/strict';
import { displayedStoreVersion, isStoreUpdateAvailable, isStoreDevLinked, resolveSelectedIndex } from './reducer.js';

test('displayedStoreVersion prefers running_version, then installed_version, then catalog', () => {
    const cases = [
        [{ installed: true, running_version: '0.13.6', installed_version: '0.12.4', version: '0.13.6' }, '0.13.6', 'running version beats stale installed dir (dev-link)'],
        [{ installed: true, installed_version: '0.12.4', version: '0.10.25' }, '0.12.4', 'installed_version when no running_version'],
        [{ installed: true, installed_version: '1.1.2', version: '1.2.1' }, '1.1.2', 'installed older than catalog'],
        [{ installed: false, installed_version: null, version: '2.3.4' }, '2.3.4', 'not installed shows catalog'],
        [{ installed: true, installed_version: null, version: '2.0.0' }, '2.0.0', 'installed but no installed/running version: catalog'],
        [{ installed: false, running_version: '9.9.9', version: '1.0.0' }, '1.0.0', 'not installed ignores running_version'],
        [{}, null, 'empty plugin object'],
        [null, null, 'null plugin'],
        [undefined, null, 'undefined plugin'],
    ];
    for (const [plugin, expected, label] of cases) {
        assert.equal(displayedStoreVersion(plugin), expected, label);
    }
});

test('isStoreUpdateAvailable trusts the backend flag and is never true for dev-linked', () => {
    const cases = [
        [{ update_available: true, source: 'installed' }, true, 'genuine update'],
        [{ update_available: true }, true, 'flag true, source absent'],
        [{ update_available: false, source: 'installed' }, false, 'no update'],
        [{ update_available: true, source: 'dev_linked' }, false, 'dev-linked is never updatable'],
        [{ source: 'dev_linked' }, false, 'dev-linked without flag'],
        [{}, false, 'no flag (registry-only / installed-state merge unavailable)'],
        [null, false, 'null plugin'],
        [undefined, false, 'undefined plugin'],
    ];
    for (const [plugin, expected, label] of cases) {
        assert.equal(isStoreUpdateAvailable(plugin), expected, label);
    }
});

test('isStoreDevLinked detects the dev_linked source', () => {
    assert.equal(isStoreDevLinked({ source: 'dev_linked' }), true);
    assert.equal(isStoreDevLinked({ source: 'installed' }), false);
    assert.equal(isStoreDevLinked({}), false);
    assert.equal(isStoreDevLinked(null), false);
    assert.equal(isStoreDevLinked(undefined), false);
});

test('resolveSelectedIndex tracks selection by id and re-anchors to the neighbor when it vanishes', () => {
    const list = [{ id: 'a' }, { id: 'b' }, { id: 'c' }];
    assert.equal(resolveSelectedIndex(list, 'b', 0), 1, 'found by id ignores fallback');
    assert.equal(resolveSelectedIndex(list, 'a', 2), 0, 'found by id at head');
    assert.equal(resolveSelectedIndex(list, 'gone', 2), 2, 'missing id falls back to clamped index');
    assert.equal(resolveSelectedIndex(list, 'gone', 9), 2, 'fallback clamped to last');
    assert.equal(resolveSelectedIndex(list, null, 1), 1, 'no id uses fallback index');
    assert.equal(resolveSelectedIndex(list, null, 0), 0, 'no id defaults to head');
    assert.equal(resolveSelectedIndex([], 'a', 3), 0, 'empty list yields 0');
    assert.equal(resolveSelectedIndex(null, 'a', 3), 0, 'non-array yields 0');
});
