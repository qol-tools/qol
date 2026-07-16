import assert from 'node:assert/strict';
import { test } from 'node:test';
import { readdirSync, readFileSync } from 'node:fs';
import { join, relative, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

const uiRoot = join(fileURLToPath(new URL('.', import.meta.url)), '..');
const SCAN_DIRS = ['views', 'components', 'app'];
const RAW_INTERACTIVE = /<(button|select|textarea|input)\b/;

const GRANDFATHERED = new Set([
    'app/views.js',
    'components/ApiErrorToast.js',
    'components/BootHealedBanner.js',
    'components/CommandPalette.js',
    'components/domain-rows/PluginRow.js',
    'components/domain-rows/StoreCard.js',
    'components/domain-rows/SuppressedRow.js',
    'components/shell/Minimap.js',
    'components/shell/PeripheralPreview.js',
    'views/hotkeys/modal.js',
    'views/plugins/grid.js',
    'views/profile/components.js',
    'views/profile/view.js',
    'views/shortcuts/modal.js',
    'views/task-runner/panels.js',
    'views/task-runner/test-runner-subpage.js',
    'views/task-runner-view.js',
]);

function jsFiles(dir) {
    return readdirSync(dir, { withFileTypes: true, recursive: true })
        .filter((entry) => entry.isFile() && entry.name.endsWith('.js') && !entry.name.endsWith('.test.js'))
        .map((entry) => join(entry.parentPath, entry.name));
}

test('views compose gallery components instead of raw interactive elements', () => {
    const offenders = [];
    const cleanGrandfathered = [];
    for (const dir of SCAN_DIRS) {
        for (const file of jsFiles(join(uiRoot, dir))) {
            const rel = relative(uiRoot, file).split(sep).join('/');
            const raw = RAW_INTERACTIVE.test(readFileSync(file, 'utf8'));
            if (raw && !GRANDFATHERED.has(rel)) offenders.push(rel);
            if (!raw && GRANDFATHERED.has(rel)) cleanGrandfathered.push(rel);
        }
    }
    assert.deepEqual(
        offenders,
        [],
        `raw <button>/<select>/<input>/<textarea> outside lib/components; compose gallery primitives or extend them: ${offenders.join(', ')}`,
    );
    assert.deepEqual(
        cleanGrandfathered,
        [],
        `now clean; remove from GRANDFATHERED: ${cleanGrandfathered.join(', ')}`,
    );
});
