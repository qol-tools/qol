import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

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

const NO_CLIP_CSS_TARGETS = [
    { file: 'common-controls.css', selector: '.code-block' },
    { file: 'app-shell.css', selector: '.view-body' },
    { file: 'app-shell.css', selector: '.plugin-config-detail' },
];

for (const { file, selector } of NO_CLIP_CSS_TARGETS) {
    test(`${file}: ${selector} does not enable scrolling or fixed max-height clipping`, () => {
        const cssPath = path.resolve(here, '..', 'styles', file);
        const css = fs.readFileSync(cssPath, 'utf8');
        const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
        const re = new RegExp(`${escaped}\\s*\\{[^}]*\\}`);
        const block = css.match(re);
        assert.ok(block, `expected a ${selector} rule in ${file}`);
        assert.equal(
            /overflow(?:-[xy])?\s*:\s*(?:auto|scroll)/i.test(block[0]),
            false,
            `${selector} must not enable scrolling — got: ${block[0]}`,
        );
        assert.equal(
            /max-height\s*:/i.test(block[0]),
            false,
            `${selector} must not pin max-height (no clipping) — got: ${block[0]}`,
        );
    });
}

test('App.js:registerStaticDiveTargets entries declare contentSized: true', () => {
    const appPath = path.resolve(here, '..', 'components', 'App.js');
    const src = fs.readFileSync(appPath, 'utf8');
    const fnMatch = src.match(/function registerStaticDiveTargets[\s\S]*?\n\}\n/);
    assert.ok(fnMatch, 'expected registerStaticDiveTargets in App.js');
    assert.match(
        fnMatch[0],
        /registry\.addEntry\(\{[\s\S]*?contentSized:\s*true[\s\S]*?\}\)/,
        'static dive targets must register entries with contentSized: true',
    );
});

test('App.js:registerPluginDiveTarget plugin section pages declare contentSized: true', () => {
    const appPath = path.resolve(here, '..', 'components', 'App.js');
    const src = fs.readFileSync(appPath, 'utf8');
    const fnMatch = src.match(/function registerPluginDiveTarget[\s\S]*?\n\}\n/);
    assert.ok(fnMatch, 'expected registerPluginDiveTarget in App.js');
    assert.match(
        fnMatch[0],
        /registry\.addEntry\(\{[\s\S]*?contentSized:\s*true[\s\S]*?\}\)/,
        'plugin section pages must register entries with contentSized: true',
    );
});
