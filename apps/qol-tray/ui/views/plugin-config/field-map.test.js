import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const root = resolve(here, '../../../../..');

test('live Preact field registry covers every Rust wire field kind', () => {
    const rust = readFileSync(resolve(root, 'libs/qol-config/src/contract/v1.rs'), 'utf8');
    const js = readFileSync(resolve(here, 'field-map.js'), 'utf8');
    const wireKinds = parseRustWireKinds(rust);
    const renderedKinds = parseFieldMapKinds(js);

    assert.deepEqual([...renderedKinds].sort(), [...wireKinds].sort());
    assert.equal(js.includes('|| StringField'), false, 'unknown kinds must not silently render as strings');
});

function parseRustWireKinds(source) {
    const body = source.match(/pub fn name\(self\)[^{]*\{([\s\S]*?)^    \}/m)?.[1];
    assert.ok(body, 'FieldKind::name must exist');
    return new Set([...body.matchAll(/Self::\w+ => "([^"]+)"/g)].map(match => match[1]));
}

function parseFieldMapKinds(source) {
    const body = source.split('const FIELD_MAP = {', 2)[1]?.split('};', 1)[0];
    assert.ok(body, 'FIELD_MAP must exist');
    return new Set([...body.matchAll(/^\s*([a-z_]+):/gm)].map(match => match[1]));
}
