import { test } from 'node:test';
import assert from 'node:assert/strict';
import { resolvePluginVersion, formatPluginVersionLabel } from './plugin-version.js';

test('resolvePluginVersion prefers running_version, then installed_version, then catalog', () => {
    const cases = [
        [{ installed: true, running_version: '0.13.6', installed_version: '0.12.4', version: '0.10.0' }, '0.13.6', 'running over installed when installed=true'],
        [{ installed: true, installed_version: '0.12.4', version: '0.10.25' }, '0.12.4', 'installed_version when no running_version'],
        [{ installed: true, installed_version: '1.1.2', version: '1.2.1' }, '1.1.2', 'installed older than catalog'],
        [{ installed: false, installed_version: null, version: '2.3.4' }, '2.3.4', 'not installed shows catalog'],
        [{ installed: false, version: '1.0.0' }, '1.0.0', 'not installed: catalog only'],
        [{ installed: true, installed_version: null, version: '2.0.0' }, '2.0.0', 'installed but no installed/running: catalog'],
        [{ installed: false, running_version: '9.9.9', version: '1.0.0' }, '1.0.0', 'not installed ignores running_version'],
        [{ version: '0.5.0' }, '0.5.0', 'no installed flag: catalog as fallback'],
        [{}, null, 'empty plugin object'],
        [null, null, 'null plugin'],
        [undefined, null, 'undefined plugin'],
        [{ installed: true }, null, 'installed but no versions at all'],
    ];
    for (const [plugin, expected, label] of cases) {
        assert.equal(resolvePluginVersion(plugin), expected, label);
    }
});

test('formatPluginVersionLabel renders v-prefixed string, range, or empty', () => {
    const cases = [
        ['1.2.3', false, 'v1.2.3', 'plain string'],
        ['0.0.1', false, 'v0.0.1', 'leading zero'],
        ['', false, '', 'empty string yields empty'],
        [null, false, '', 'null yields empty'],
        [undefined, false, '', 'undefined yields empty'],
        ['1.2.3', true, 'v1.2.3', 'hasUpdate ignored when version is plain string (no range info)'],
        [{ from: '1.0.0', to: '2.0.0' }, true, 'v1.0.0 -> v2.0.0', 'update range from object'],
        [{ from: '1.0.0', to: '2.0.0' }, false, 'v1.0.0', 'object without hasUpdate uses from'],
        [{ current: '1.2.3' }, false, 'v1.2.3', 'object with current uses current'],
        [{ current: '1.2.3' }, true, 'v1.2.3', 'object with current and hasUpdate but no to'],
        [{}, false, '', 'empty object yields empty'],
        [{ from: '', to: '' }, true, '', 'empty range strings yield empty'],
    ];
    for (const [version, hasUpdate, expected, label] of cases) {
        assert.equal(formatPluginVersionLabel(version, hasUpdate), expected, label);
    }
});

test('formatPluginVersionLabel composed with resolvePluginVersion handles all real plugin shapes', () => {
    const cases = [
        [{ installed: true, running_version: '0.13.6' }, false, 'v0.13.6', 'installed plugin with running version'],
        [{ installed: false, version: '1.0.0' }, false, 'v1.0.0', 'store entry, not installed'],
        [{ installed: true, installed_version: '1.0.0', available_version: '2.0.0' }, false, 'v1.0.0', 'installed plugin with update available, hasUpdate=false'],
        [{}, false, '', 'empty plugin yields empty label'],
        [null, false, '', 'null plugin yields empty label'],
    ];
    for (const [plugin, hasUpdate, expected, label] of cases) {
        assert.equal(formatPluginVersionLabel(resolvePluginVersion(plugin), hasUpdate), expected, label);
    }
});
