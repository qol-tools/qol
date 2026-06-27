import { beforeEach, test } from 'node:test';
import assert from 'node:assert/strict';
import { register } from 'node:module';
import { pathToFileURL } from 'node:url';

const hooksStubSource = `
export function useState(initial) { return [typeof initial === 'function' ? initial() : initial, () => {}]; }
export function useEffect() {}
export function useRef(value) { return { current: value }; }
export function useCallback(fn) { return fn; }
`;

const loaderSource = `
const STUB_URL = ${JSON.stringify('data:text/javascript,' + encodeURIComponent(hooksStubSource))};
export function resolve(specifier, context, nextResolve) {
    if (specifier === 'preact/hooks') {
        return { url: STUB_URL, shortCircuit: true, format: 'module' };
    }
    return nextResolve(specifier, context);
}
`;

register('data:text/javascript,' + encodeURIComponent(loaderSource), pathToFileURL('./'));

let loadPluginConfigSession;
let preloadConfigForm;
let startPluginConfigSessionLoad;
let importId = 0;

beforeEach(async () => {
    ({
        loadPluginConfigSession,
        preloadConfigForm,
        startPluginConfigSessionLoad,
    } = await import(`./usePluginConfig.js?test=${importId++}`));
});

function jsonResponse(data, { ok = true, status = 200 } = {}) {
    return {
        ok,
        status,
        text: async () => JSON.stringify(data),
    };
}

function textResponse(text, { ok = false, status = 500 } = {}) {
    return {
        ok,
        status,
        text: async () => text,
    };
}

async function withFetch(handler, run) {
    const originalFetch = globalThis.fetch;
    globalThis.fetch = handler;
    try {
        return await run();
    } finally {
        if (originalFetch === undefined) delete globalThis.fetch;
        else globalThis.fetch = originalFetch;
    }
}

function makeForm(value = false) {
    return {
        title: 'Plugin settings',
        fields: [
            {
                id: 'enabled',
                kind: 'boolean',
                config_key: 'settings.enabled',
                value,
            },
        ],
        sections: [],
    };
}

test('loadPluginConfigSession preserves existing config when using a preloaded form', async () => {
    const form = makeForm(false);
    const urls = [];
    preloadConfigForm('plugin-a', form);
    const session = await withFetch(async (url) => {
        urls.push(url);
        assert.equal(url, '/api/plugins/plugin-a/config');
        return jsonResponse({
            settings: {
                enabled: true,
                token: 'keep-me',
            },
        });
    }, () => loadPluginConfigSession('plugin-a'));

    assert.equal(session.form, form);
    assert.deepEqual(session.fieldPaths, { enabled: 'settings.enabled' });
    assert.deepEqual(session.config, {
        settings: {
            enabled: false,
            token: 'keep-me',
        },
    });
    assert.deepEqual(urls, ['/api/plugins/plugin-a/config']);
});

test('loadPluginConfigSession fetches raw config when no config form exists', async () => {
    const urls = [];
    const session = await withFetch(async (url) => {
        urls.push(url);
        if (url === '/api/plugins/plugin-raw/config-form') return textResponse('', { status: 404 });
        if (url === '/api/plugins/plugin-raw/config') return jsonResponse({ raw: true });
        throw new Error(`unexpected url ${url}`);
    }, () => loadPluginConfigSession('plugin-raw'));

    assert.deepEqual(session, {
        form: null,
        config: { raw: true },
        fieldPaths: null,
        error: null,
    });
    assert.deepEqual(urls, [
        '/api/plugins/plugin-raw/config-form',
        '/api/plugins/plugin-raw/config',
    ]);
});

test('loadPluginConfigSession returns an empty error session when no config exists', async () => {
    const session = await withFetch(async (url) => {
        if (url === '/api/plugins/plugin-none/config-form') return textResponse('', { status: 404 });
        if (url === '/api/plugins/plugin-none/config') return textResponse('', { status: 404 });
        throw new Error(`unexpected url ${url}`);
    }, () => loadPluginConfigSession('plugin-none'));

    assert.deepEqual(session, {
        form: null,
        config: null,
        fieldPaths: null,
        error: 'No configuration found for this plugin.',
    });
});

test('startPluginConfigSessionLoad ignores a result after cleanup', async () => {
    let resolveLoad;
    const loadPromise = new Promise(resolve => { resolveLoad = resolve; });
    const applied = [];
    const errors = [];
    const cleanup = startPluginConfigSessionLoad(
        'plugin-a',
        session => applied.push(session),
        error => errors.push(error),
        () => loadPromise,
    );

    cleanup();
    resolveLoad({ form: makeForm(), config: {}, fieldPaths: {}, error: null });
    await loadPromise;
    await Promise.resolve();

    assert.deepEqual(applied, []);
    assert.deepEqual(errors, []);
});

test('startPluginConfigSessionLoad applies errors while active', async () => {
    const applied = [];
    const errors = [];
    const failure = new Error('load failed');
    startPluginConfigSessionLoad(
        'plugin-a',
        session => applied.push(session),
        error => errors.push(error),
        async () => { throw failure; },
    );

    await Promise.resolve();
    await Promise.resolve();

    assert.deepEqual(applied, []);
    assert.deepEqual(errors, [failure]);
});
