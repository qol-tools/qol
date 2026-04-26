import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

// Lock down which top-level views must be content-sized. Long lists
// (hotkeys, logs) and grids (plugins, store) MUST size to their content
// height — the canvas is the viewport, no inner scrollbars allowed.
//
// This is a source-text test rather than an import test because views.js
// pulls in the whole Preact view graph; we just want to assert the manifest.

const here = path.dirname(fileURLToPath(import.meta.url));
const viewsPath = path.resolve(here, '..', 'app', 'views.js');
const src = fs.readFileSync(viewsPath, 'utf8');

const MUST_BE_CONTENT_SIZED = [
    'plugins',
    'store',
    'hotkeys',
    'shortcuts',
    'task-runner',
    'profile',
    'logs',
    'dev',
];

for (const id of MUST_BE_CONTENT_SIZED) {
    test(`top-level view '${id}' is content-sized`, () => {
        const re = new RegExp(`{ id: '${id}',[^}]*contentSized: true`);
        assert.match(src, re, `${id} entry must declare contentSized: true`);
    });
}

test('world.css does not introduce overflow-y: auto on slot content', () => {
    const cssPath = path.resolve(here, '..', 'styles', 'world.css');
    const css = fs.readFileSync(cssPath, 'utf8');
    const slotBlock = css.match(/\.world-view-slot\s*\.view-body\s*\{[^}]*\}/);
    assert.ok(slotBlock, 'expected a .world-view-slot .view-body rule');
    assert.equal(
        /overflow(?:-[xy])?\s*:\s*(?:auto|scroll)/i.test(slotBlock[0]),
        false,
        `slot view-body block must not enable scrolling — got: ${slotBlock[0]}`,
    );
});
