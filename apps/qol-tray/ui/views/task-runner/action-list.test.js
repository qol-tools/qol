import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, resolve } from 'node:path';

const here = dirname(fileURLToPath(import.meta.url));
const source = readFileSync(resolve(here, 'action-list.js'), 'utf8');

test('empty task-runner list renders a selected surface', () => {
    const emptyBranch = source.split('if (data.actionIds.length === 0)', 2)[1]?.split('return html`<div class="actions-list">', 2)[1]?.split('</div>`;', 1)[0] || '';
    assert.match(emptyBranch, /<\$\{Surface\}/);
    assert.match(emptyBranch, /selected=\$\{true\}/);
    assert.match(emptyBranch, /onActivate=\$\{\(\) => edit\.openEditModal\(\)\}/);
});
