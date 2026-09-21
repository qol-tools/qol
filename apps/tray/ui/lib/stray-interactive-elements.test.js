import assert from 'node:assert/strict';
import { test } from 'node:test';
import { readdirSync, readFileSync } from 'node:fs';
import { join, relative, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

const uiRoot = join(fileURLToPath(new URL('.', import.meta.url)), '..');
const SCAN_DIRS = ['views', 'components', 'app'];
const RAW_INTERACTIVE = /<(button|select|textarea|input)\b/;

const SANCTIONED_RAW_INTERNALS = new Set([
    'app/views.js',
    'components/domain-rows/StoreCard.js',
    'components/shell/PeripheralPreview.js',
    'views/profile/view.js',
]);

function jsFiles(dir) {
    return readdirSync(dir, { withFileTypes: true, recursive: true })
        .filter((entry) => entry.isFile() && entry.name.endsWith('.js') && !entry.name.endsWith('.test.js'))
        .map((entry) => join(entry.parentPath, entry.name));
}

test('views compose gallery components instead of raw interactive elements', () => {
    const offenders = [];
    const cleanSanctioned = [];
    for (const dir of SCAN_DIRS) {
        for (const file of jsFiles(join(uiRoot, dir))) {
            const rel = relative(uiRoot, file).split(sep).join('/');
            const raw = RAW_INTERACTIVE.test(readFileSync(file, 'utf8'));
            if (raw && !SANCTIONED_RAW_INTERNALS.has(rel)) offenders.push(rel);
            if (!raw && SANCTIONED_RAW_INTERNALS.has(rel)) cleanSanctioned.push(rel);
        }
    }
    assert.deepEqual(
        offenders,
        [],
        `raw <button>/<select>/<input>/<textarea> outside lib/components; compose gallery primitives or extend them: ${offenders.join(', ')}`,
    );
    assert.deepEqual(
        cleanSanctioned,
        [],
        `now clean; remove from SANCTIONED_RAW_INTERNALS: ${cleanSanctioned.join(', ')}`,
    );
});
