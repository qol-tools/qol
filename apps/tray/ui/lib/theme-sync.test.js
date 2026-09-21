import assert from 'node:assert/strict';
import { test } from 'node:test';

function installBrowserGlobals(fetchImpl) {
    const originalWindow = globalThis.window;
    const originalFetch = globalThis.fetch;
    const originalDispatchEvent = globalThis.dispatchEvent;
    const originalDocument = globalThis.document;
    const originalLocation = globalThis.location;
    const originalCustomEvent = globalThis.CustomEvent;
    const events = [];
    const attributes = new Map();
    const styles = new Map();

    globalThis.fetch = fetchImpl;
    globalThis.dispatchEvent = (event) => {
        events.push(event);
        return true;
    };
    globalThis.window = globalThis;
    globalThis.document = {
        documentElement: {
            setAttribute(name, value) {
                attributes.set(name, value);
            },
            style: {
                setProperty(name, value) {
                    styles.set(name, value);
                },
            },
        },
    };
    globalThis.location = { origin: 'http://localhost' };
    globalThis.CustomEvent = class CustomEvent {
        constructor(type, init = {}) {
            this.type = type;
            this.detail = init.detail;
        }
    };

    return {
        events,
        attributes,
        styles,
        restore() {
            if (originalWindow === undefined) delete globalThis.window;
            else globalThis.window = originalWindow;
            if (originalFetch === undefined) delete globalThis.fetch;
            else globalThis.fetch = originalFetch;
            if (originalDispatchEvent === undefined) delete globalThis.dispatchEvent;
            else globalThis.dispatchEvent = originalDispatchEvent;
            if (originalDocument === undefined) delete globalThis.document;
            else globalThis.document = originalDocument;
            if (originalLocation === undefined) delete globalThis.location;
            else globalThis.location = originalLocation;
            if (originalCustomEvent === undefined) delete globalThis.CustomEvent;
            else globalThis.CustomEvent = originalCustomEvent;
        },
    };
}

function freshModuleUrl() {
    return new URL(`./theme-sync.js?case=${Date.now()}-${Math.random()}`, import.meta.url).href;
}

test('setTheme persists via /api/theme and applies the attribute', async () => {
    const calls = [];
    const harness = installBrowserGlobals(async (url, options) => {
        calls.push({ url, options });
        return new Response(JSON.stringify({ key: 'midnight', selectedKey: 'midnight' }), {
            status: 200,
            headers: { 'content-type': 'application/json' },
        });
    });
    try {
        const themeSync = await import(freshModuleUrl());
        const seen = [];
        themeSync.subscribeTheme((key) => seen.push(key));

        const selected = await themeSync.setTheme('midnight');

        assert.equal(selected, 'midnight');
        assert.equal(themeSync.getTheme(), 'midnight');
        assert.equal(calls.length, 1);
        assert.equal(calls[0].url, '/api/theme');
        assert.equal(harness.attributes.get('data-qol-theme'), 'midnight');
        assert.deepEqual(seen, ['midnight']);
    } finally {
        harness.restore();
    }
});

test('unknown response keys resolve to the default theme', async () => {
    const harness = installBrowserGlobals(async () => {
        return new Response(JSON.stringify({ key: 'nope', selectedKey: 'nope' }), {
            status: 200,
            headers: { 'content-type': 'application/json' },
        });
    });
    try {
        const themeSync = await import(freshModuleUrl());

        const selected = await themeSync.setTheme('nope');

        assert.equal(selected, null);
        assert.equal(harness.attributes.get('data-qol-theme'), 'slate');
    } finally {
        harness.restore();
    }
});

test('theme save failures preserve local state and defer feedback to caller', async () => {
    const harness = installBrowserGlobals(async () => {
        return new Response('unknown theme', { status: 400 });
    });
    try {
        const themeSync = await import(freshModuleUrl());
        const seen = [];
        themeSync.subscribeTheme((key) => seen.push(key));

        assert.equal(themeSync.getTheme(), null);
        await assert.rejects(() => themeSync.setTheme('midnight'), /unknown theme/);

        assert.equal(themeSync.getTheme(), null);
        assert.deepEqual(seen, []);
        assert.deepEqual(harness.events, []);
    } finally {
        harness.restore();
    }
});

test('theme switch re-tints an auto accent to the theme accent', async () => {
    const harness = installBrowserGlobals(async () => {
        return new Response(JSON.stringify({ key: 'midnight', selectedKey: 'midnight' }), {
            status: 200,
            headers: { 'content-type': 'application/json' },
        });
    });
    try {
        const themeSync = await import(freshModuleUrl());

        await themeSync.setTheme('midnight');

        assert.equal(harness.attributes.get('data-qol-theme'), 'midnight');
        assert.equal(harness.attributes.get('data-qol-identity'), 'modern');
        assert.equal(harness.styles.get('--accent-rgb'), '138, 147, 247');
        assert.equal(harness.styles.get('--accent-hover'), '#a5afff');
    } finally {
        harness.restore();
    }
});

test('applyThemeSelection stamps the effective theme on startup', async () => {
    const harness = installBrowserGlobals(async () => {
        throw new Error('no network expected');
    });
    try {
        const themeSync = await import(freshModuleUrl());

        const selected = themeSync.applyThemeSelection();

        assert.equal(selected, null);
        assert.equal(harness.attributes.get('data-qol-theme'), 'slate');
    } finally {
        harness.restore();
    }
});
